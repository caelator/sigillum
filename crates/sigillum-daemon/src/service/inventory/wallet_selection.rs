use secrecy::ExposeSecret;
use serde::Deserialize;
use sigillum_api::{EthSeedWalletProfile, EthXpubWalletProfile};
use sigillum_core::{
    SecretStore, VaultLifecycle, derive_ethereum_xpub_receive_branch_from_mnemonic,
    derive_sigillum_ethereum_xpub_receive_branch,
};
use zeroize::Zeroize;

use crate::service::helpers::map_xpub_error;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB};

pub(super) const DERIVATION_PATTERN_PROJECT: &str = "project";
pub(super) const DERIVATION_PATTERN_STANDARD: &str = "standard";
pub(super) const DERIVATION_PATTERN_LEDGER_LIVE: &str = "ledger_live";

const DEFAULT_ACCOUNT_LIMIT: u32 = 3;
const MAX_ACCOUNT_LIMIT: u32 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SeedDerivationPattern {
    Project,
    Standard,
    LedgerLive,
}

impl SeedDerivationPattern {
    pub(super) fn parse(value: Option<&str>) -> ServiceResult<Self> {
        match value.unwrap_or(DERIVATION_PATTERN_PROJECT).trim() {
            "" | DERIVATION_PATTERN_PROJECT => Ok(Self::Project),
            DERIVATION_PATTERN_STANDARD => Ok(Self::Standard),
            DERIVATION_PATTERN_LEDGER_LIVE => Ok(Self::LedgerLive),
            _ => Err(ServiceError::bad_request(
                "derivation_pattern must be project, standard, or ledger_live.",
            )),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Project => DERIVATION_PATTERN_PROJECT,
            Self::Standard => DERIVATION_PATTERN_STANDARD,
            Self::LedgerLive => DERIVATION_PATTERN_LEDGER_LIVE,
        }
    }

    fn scans_seed_accounts(self) -> bool {
        !matches!(self, Self::Project)
    }
}

pub(super) fn scan_account_limit(value: Option<u32>) -> ServiceResult<u32> {
    let limit = value.unwrap_or(DEFAULT_ACCOUNT_LIMIT);
    if limit == 0 {
        return Err(ServiceError::bad_request(
            "account_limit must be greater than zero.",
        ));
    }
    if limit > MAX_ACCOUNT_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "account_limit must be <= {MAX_ACCOUNT_LIMIT}."
        )));
    }
    Ok(limit)
}

#[derive(Clone, Debug)]
pub(in crate::service::inventory) struct DiscoveryWallet {
    pub(in crate::service::inventory) family: String,
    pub(in crate::service::inventory) profile: String,
    pub(in crate::service::inventory) receive_path: String,
    pub(in crate::service::inventory) receive_xpub: String,
    pub(in crate::service::inventory) derivation_pattern: String,
    pub(in crate::service::inventory) account_index: u32,
}

#[derive(Debug, Deserialize)]
struct StoredSeedWalletSecret {
    mnemonic: String,
    #[serde(default)]
    mnemonic_passphrase: Option<String>,
}

pub(super) fn select_discovery_wallets(
    service: &SigillumService,
    seed_profiles: &[EthSeedWalletProfile],
    xpub_profiles: &[EthXpubWalletProfile],
    requested_family: Option<&str>,
    requested_profile: Option<&str>,
    seed_derivation_pattern: SeedDerivationPattern,
    account_limit: u32,
) -> ServiceResult<Vec<DiscoveryWallet>> {
    let mut wallets = Vec::new();

    if requested_family.is_none() || requested_family == Some(WALLET_FAMILY_ETH_SEED) {
        for profile in seed_profiles {
            if requested_profile.is_some_and(|name| name != profile.name) {
                continue;
            }
            if seed_derivation_pattern.scans_seed_accounts() {
                wallets.extend(seed_account_discovery_wallets(
                    service,
                    profile,
                    seed_derivation_pattern,
                    account_limit,
                )?);
            } else {
                wallets.push(DiscoveryWallet {
                    family: WALLET_FAMILY_ETH_SEED.into(),
                    profile: profile.name.clone(),
                    receive_path: profile.receive_path.clone(),
                    receive_xpub: profile.receive_xpub.clone(),
                    derivation_pattern: DERIVATION_PATTERN_PROJECT.into(),
                    account_index: profile.project_account,
                });
            }
        }
    }

    if requested_family.is_none() || requested_family == Some(WALLET_FAMILY_ETH_XPUB) {
        for profile in xpub_profiles {
            if requested_profile.is_some_and(|name| name != profile.name) {
                continue;
            }
            let export = service.with_vault(profile.compartment_id, |vault| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::forbidden("Wallet compartment is locked."))?;
                derive_sigillum_ethereum_xpub_receive_branch(
                    master_key.as_ref(),
                    profile.project_account,
                )
                .map_err(map_xpub_error)
            })?;
            wallets.push(DiscoveryWallet {
                family: WALLET_FAMILY_ETH_XPUB.into(),
                profile: profile.name.clone(),
                receive_path: export.receive_path,
                receive_xpub: export.receive_xpub,
                derivation_pattern: DERIVATION_PATTERN_PROJECT.into(),
                account_index: profile.project_account,
            });
        }
    }

    Ok(wallets)
}

fn seed_account_discovery_wallets(
    service: &SigillumService,
    profile: &EthSeedWalletProfile,
    seed_derivation_pattern: SeedDerivationPattern,
    account_limit: u32,
) -> ServiceResult<Vec<DiscoveryWallet>> {
    service.with_vault(profile.compartment_id, |vault| {
        if !vault.is_unlocked() {
            return Err(ServiceError::forbidden("Wallet compartment is locked."));
        }
        let secret = vault
            .read_secret(&profile.mnemonic_secret_key)?
            .ok_or_else(|| ServiceError::internal("Seed wallet secret is missing."))?;
        let mut stored: StoredSeedWalletSecret = serde_json::from_str(secret.expose_secret())
            .map_err(|error| {
                ServiceError::internal(format!("Failed to parse seed wallet secret: {error}"))
            })?;
        let mut wallets = Vec::new();
        for account_index in 0..account_limit {
            let export = derive_ethereum_xpub_receive_branch_from_mnemonic(
                &stored.mnemonic,
                stored.mnemonic_passphrase.as_deref(),
                account_index,
            )
            .map_err(map_xpub_error)?;
            wallets.push(DiscoveryWallet {
                family: WALLET_FAMILY_ETH_SEED.into(),
                profile: profile.name.clone(),
                receive_path: export.receive_path,
                receive_xpub: export.receive_xpub,
                derivation_pattern: seed_derivation_pattern.label().into(),
                account_index,
            });
        }
        stored.mnemonic.zeroize();
        if let Some(passphrase) = stored.mnemonic_passphrase.as_mut() {
            passphrase.zeroize();
        }
        Ok(wallets)
    })
}
