use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sigillum_api::{
    EthSeedWalletCreateRequest, EthSeedWalletCreateResponse, EthSeedWalletProfile,
    EthSeedWalletProfileListResponse, EthSeedWalletProfileMutationResponse,
    EthSeedWalletProfileUpsertRequest, EvmProfileDeleteRequest,
};
use sigillum_core::{
    SecretStore, derive_ethereum_address_from_control_xpub, derive_ethereum_address_from_xpub,
    derive_ethereum_private_key_from_mnemonic, derive_ethereum_xpub_control_branch_from_mnemonic,
    derive_ethereum_xpub_receive_branch_from_mnemonic, ethereum_mnemonic_word_count,
    generate_ethereum_mnemonic,
};
use zeroize::{Zeroize, Zeroizing};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::map_xpub_error;

use super::{ServiceError, ServiceResult, SigillumService};
use super::{remove_named, upsert_named, validate_profile_name};

#[derive(Debug, Serialize, Deserialize)]
struct StoredSeedWalletSecret {
    mnemonic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mnemonic_passphrase: Option<String>,
}

/// Default BIP-39 word count for daemon-generated seed wallets.
const DEFAULT_SEED_WALLET_WORD_COUNT: usize = 24;

/// How [`SigillumService::persist_eth_seed_wallet_profile`] treats an existing
/// profile with the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeedWalletWriteMode {
    /// Replace an existing profile in place (import/upsert semantics).
    Upsert,
    /// Reject with 409 Conflict when the name already exists (create semantics).
    CreateOnly,
}

/// Validated and resolved inputs for storing a seed wallet profile.
///
/// `mnemonic` is the normalized phrase inside a [`Zeroizing`] buffer so it is
/// wiped from memory once the profile has been persisted (or on any error).
struct SeedWalletProfileMaterial {
    name: String,
    label: Option<String>,
    mnemonic: Zeroizing<String>,
    mnemonic_passphrase: Option<String>,
    project_account: u32,
    provider_profile: String,
    compartment_id: usize,
    chain_id: Option<u64>,
    default_destination_address: Option<String>,
    execution_enabled: bool,
}

impl SigillumService {
    pub(crate) fn list_eth_seed_wallet_profiles(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<EthSeedWalletProfileListResponse> {
        let _ = self.require_session(token)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        Ok(EthSeedWalletProfileListResponse {
            profiles: registry.eth_seed_wallets,
        })
    }

    pub(crate) async fn upsert_eth_seed_wallet_profile(
        &self,
        token: Option<&str>,
        mut body: EthSeedWalletProfileUpsertRequest,
    ) -> ServiceResult<EthSeedWalletProfileMutationResponse> {
        let token = self.require_session(token)?;
        validate_profile_name(&body.name)?;
        let compartment_id = body
            .compartment_id
            .or_else(|| self.state.active_compartment_id_for(token))
            .ok_or_else(|| ServiceError::vault_locked("No active compartment."))?;

        let mnemonic = Zeroizing::new(normalize_mnemonic_phrase(&body.mnemonic));
        body.mnemonic.zeroize();

        let profile = self
            .persist_eth_seed_wallet_profile(
                token,
                SeedWalletProfileMaterial {
                    name: body.name,
                    label: body.label,
                    mnemonic,
                    mnemonic_passphrase: body.mnemonic_passphrase,
                    project_account: body.project_account,
                    provider_profile: body.provider_profile,
                    compartment_id,
                    chain_id: body.chain_id,
                    default_destination_address: body.default_destination_address,
                    execution_enabled: body.execution_enabled.unwrap_or(false),
                },
                SeedWalletWriteMode::Upsert,
            )
            .await?;

        Ok(EthSeedWalletProfileMutationResponse {
            status: "ok".into(),
            profile,
            pruned_inventory: None,
        })
    }

    /// Create a brand-new seed wallet profile from a daemon-generated BIP-39
    /// mnemonic and return the phrase exactly once for operator backup.
    ///
    /// The phrase lives in [`Zeroizing`] buffers end to end; the only copy
    /// that leaves this function is the one embedded in the response. It is
    /// stored solely as an encrypted vault secret (the same path the upsert
    /// flow uses) and never written to the audit log. Unlike upsert, creating
    /// never overwrites an existing profile of the same name.
    pub(crate) async fn create_eth_seed_wallet_profile(
        &self,
        token: Option<&str>,
        body: EthSeedWalletCreateRequest,
    ) -> ServiceResult<EthSeedWalletCreateResponse> {
        let token = self.require_session(token)?;
        validate_profile_name(&body.name)?;
        let compartment_id = body
            .compartment_id
            .or_else(|| self.state.active_compartment_id_for(token))
            .ok_or_else(|| ServiceError::vault_locked("No active compartment."))?;

        let word_count = body.word_count.unwrap_or(DEFAULT_SEED_WALLET_WORD_COUNT);
        let mut mnemonic = generate_ethereum_mnemonic(word_count).map_err(map_xpub_error)?;

        let profile = self
            .persist_eth_seed_wallet_profile(
                token,
                SeedWalletProfileMaterial {
                    name: body.name,
                    label: body.label,
                    mnemonic: mnemonic.clone(),
                    mnemonic_passphrase: body.mnemonic_passphrase,
                    project_account: body.project_account,
                    provider_profile: body.provider_profile,
                    compartment_id,
                    chain_id: body.chain_id,
                    default_destination_address: body.default_destination_address,
                    execution_enabled: body.execution_enabled.unwrap_or(false),
                },
                SeedWalletWriteMode::CreateOnly,
            )
            .await?;

        Ok(EthSeedWalletCreateResponse {
            status: "created".into(),
            mnemonic: std::mem::take(&mut *mnemonic),
            profile,
        })
    }

    /// Shared core of seed-wallet import (`upsert`) and creation (`create`).
    ///
    /// Validates the phrase, derives the public receive/control material,
    /// stores the phrase (and optional BIP-39 passphrase) as an encrypted
    /// vault secret, persists the profile in the registry, and records the
    /// matching audit event. Audit events carry profile metadata only — never
    /// any mnemonic material.
    async fn persist_eth_seed_wallet_profile(
        &self,
        token: &str,
        mut material: SeedWalletProfileMaterial,
        mode: SeedWalletWriteMode,
    ) -> ServiceResult<EthSeedWalletProfile> {
        let session_context = self.capture_session_operation_context(Some(token))?;
        let word_count =
            ethereum_mnemonic_word_count(&material.mnemonic).map_err(map_xpub_error)?;
        if word_count != 12 && word_count != 24 {
            return Err(ServiceError::bad_request(
                "Seed phrase must contain exactly 12 or 24 words.",
            ));
        }

        let export = derive_ethereum_xpub_receive_branch_from_mnemonic(
            &material.mnemonic,
            material.mnemonic_passphrase.as_deref(),
            material.project_account,
        )
        .map_err(map_xpub_error)?;
        let first_receive_address = derive_ethereum_address_from_xpub(&export.receive_xpub, 0)
            .map_err(map_xpub_error)?
            .address;

        let control_export = derive_ethereum_xpub_control_branch_from_mnemonic(
            &material.mnemonic,
            material.mnemonic_passphrase.as_deref(),
            material.project_account,
        )
        .map_err(map_xpub_error)?;
        let sponsor_address =
            derive_ethereum_address_from_control_xpub(&control_export.receive_xpub, 0)
                .map_err(map_xpub_error)?
                .address;
        let hot_address =
            derive_ethereum_address_from_control_xpub(&control_export.receive_xpub, 1)
                .map_err(map_xpub_error)?
                .address;
        let treasury_address =
            derive_ethereum_address_from_control_xpub(&control_export.receive_xpub, 2)
                .map_err(map_xpub_error)?
                .address;

        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        if !registry
            .evm_providers
            .iter()
            .any(|profile| profile.name == material.provider_profile)
        {
            return Err(ServiceError::not_found("Provider profile not found."));
        }
        if mode == SeedWalletWriteMode::CreateOnly
            && registry
                .eth_seed_wallets
                .iter()
                .any(|profile| profile.name == material.name)
        {
            return Err(ServiceError::conflict(
                "Seed wallet profile already exists. Use upsert to replace it.",
            ));
        }

        let mnemonic_secret_key = seed_wallet_secret_key(&material.name);
        let mut stored_secret = StoredSeedWalletSecret {
            mnemonic: std::mem::take(&mut *material.mnemonic),
            mnemonic_passphrase: material.mnemonic_passphrase,
        };
        let secret_payload =
            Zeroizing::new(serde_json::to_string(&stored_secret).map_err(|error| {
                ServiceError::internal(format!("Failed to serialize seed wallet secret: {error}"))
            })?);
        stored_secret.mnemonic.zeroize();
        if let Some(passphrase) = stored_secret.mnemonic_passphrase.as_mut() {
            passphrase.zeroize();
        }
        self.with_vault(material.compartment_id, |vault| {
            if !vault.is_unlocked() {
                return Err(ServiceError::vault_locked("Wallet compartment is locked."));
            }
            Ok(vault.set_secret(&mnemonic_secret_key, secret_payload.as_str())?)
        })?;

        let profile = EthSeedWalletProfile {
            name: material.name,
            label: material.label,
            project_account: export.project_account,
            provider_profile: material.provider_profile,
            compartment_id: material.compartment_id,
            chain_id: material.chain_id,
            word_count,
            mnemonic_secret_key,
            account_path: export.account_path,
            receive_path: export.receive_path,
            receive_xpub: export.receive_xpub,
            first_receive_address,
            default_destination_address: material.default_destination_address,
            control_xpub: Some(control_export.receive_xpub),
            sponsor_address: Some(sponsor_address),
            hot_address: Some(hot_address),
            treasury_address: Some(treasury_address),
            execution_enabled: material.execution_enabled,
        };

        upsert_named(&mut registry.eth_seed_wallets, profile.clone(), |item| {
            &item.name
        });
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        let audit_event = match mode {
            SeedWalletWriteMode::Upsert => AuditEventSpec::ProfilesEthSeedWalletUpsert {
                name: profile.name.clone(),
                provider_profile: profile.provider_profile.clone(),
                word_count: profile.word_count,
            },
            SeedWalletWriteMode::CreateOnly => AuditEventSpec::ProfilesEthSeedWalletCreate {
                name: profile.name.clone(),
                provider_profile: profile.provider_profile.clone(),
                word_count: profile.word_count,
            },
        };
        self.record_audit(Some(profile.compartment_id), audit_event)?;

        Ok(profile)
    }

    pub(crate) async fn delete_eth_seed_wallet_profile(
        &self,
        token: Option<&str>,
        body: EvmProfileDeleteRequest,
    ) -> ServiceResult<EthSeedWalletProfileMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        let profile = registry
            .eth_seed_wallets
            .iter()
            .find(|profile| profile.name == body.name)
            .cloned()
            .ok_or_else(|| ServiceError::not_found("Seed wallet profile not found."))?;

        // Fail fast before any mutation: with the compartment locked the
        // secret delete below would fail anyway, but a requested inventory
        // cascade must not run ahead of that failure.
        if body.prune_inventory == Some(true) {
            self.with_vault(profile.compartment_id, |vault| {
                if !vault.is_unlocked() {
                    return Err(ServiceError::vault_locked("Wallet compartment is locked."));
                }
                Ok(())
            })?;
        }

        // Forget cascade (plan task 3.2): the profile's scanned-address rows,
        // holdings, scan state, receive allocations (active ones are
        // retire-then-purged in the same operation), and the counterparty
        // bindings those allocations carried — before the profile itself goes.
        let pruned_inventory = if body.prune_inventory == Some(true) {
            Some(
                self.prune_inventory_for_deleted_profile(
                    token,
                    "eth-seed",
                    crate::service::inventory::prune::InventoryPruneScope::WalletProfile {
                        family: "eth-seed",
                        name: &body.name,
                    },
                )
                .await?,
            )
        } else {
            None
        };

        self.with_vault(profile.compartment_id, |vault| {
            if !vault.is_unlocked() {
                return Err(ServiceError::vault_locked("Wallet compartment is locked."));
            }
            Ok(vault.delete_secret(&profile.mnemonic_secret_key)?)
        })?;

        let profile = remove_named(&mut registry.eth_seed_wallets, &body.name, |item| {
            &item.name
        })
        .ok_or_else(|| ServiceError::not_found("Seed wallet profile not found."))?;
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEthSeedWalletDelete {
                name: profile.name.clone(),
            },
        )?;

        Ok(EthSeedWalletProfileMutationResponse {
            status: "deleted".into(),
            profile,
            pruned_inventory,
        })
    }

    /// Derive the signing key for `derivation_path` from a seed wallet
    /// profile's vault-stored mnemonic, inside the unlocked compartment
    /// (W7.3 plan-step execution).
    ///
    /// Locked compartment or a missing/corrupt secret both fail closed with
    /// a named `ServiceError` (never panics). The mnemonic (and BIP-39
    /// passphrase, if any) are held in `Zeroizing` buffers that are wiped
    /// when this function returns; the returned [`k256::ecdsa::SigningKey`]
    /// zeroizes itself on drop (house style, cf. `ethereum_xpub.rs`).
    pub(in crate::service) fn derive_eth_seed_signing_key(
        &self,
        profile: &EthSeedWalletProfile,
        derivation_path: &str,
    ) -> ServiceResult<k256::ecdsa::SigningKey> {
        self.with_vault(profile.compartment_id, |vault| {
            if !vault.is_unlocked() {
                return Err(ServiceError::vault_locked("Wallet compartment is locked."));
            }
            let secret = vault
                .read_secret(&profile.mnemonic_secret_key)
                .map_err(|error| ServiceError::internal(error.to_string()))?
                .ok_or_else(|| ServiceError::not_found("Seed wallet mnemonic secret not found."))?;
            let mut stored: StoredSeedWalletSecret = serde_json::from_str(secret.expose_secret())
                .map_err(|error| {
                ServiceError::internal(format!("Failed to parse seed wallet secret: {error}"))
            })?;
            let mnemonic = Zeroizing::new(std::mem::take(&mut stored.mnemonic));
            let passphrase = stored.mnemonic_passphrase.take().map(Zeroizing::new);
            derive_ethereum_private_key_from_mnemonic(
                &mnemonic,
                passphrase.as_ref().map(|value| value.as_str()),
                derivation_path,
            )
            .map_err(map_xpub_error)
        })
    }
}

fn normalize_mnemonic_phrase(phrase: &str) -> String {
    phrase.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn seed_wallet_secret_key(name: &str) -> String {
    format!("wallet.seed.{name}.mnemonic")
}
