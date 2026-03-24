//! Profile management for EVM providers and stealth wallets.
//!
//! Manages creation, updating, and deletion of reusable provider and wallet
//! profiles with chain configuration and fee parameters.

use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendErc20WithProfileRequest,
    EthStealthSendResponse, EthStealthSendTransferRequest, EthStealthSendWithProfileRequest,
    EthStealthWalletProfile, EthStealthWalletProfileListResponse,
    EthStealthWalletProfileMutationResponse, EthStealthWalletProfileUpsertRequest,
    EthXpubWalletProfile, EthXpubWalletProfileListResponse, EthXpubWalletProfileMutationResponse,
    EthXpubWalletProfileUpsertRequest, EvmProfileDeleteRequest, EvmProviderProfile,
    EvmProviderProfileListResponse, EvmProviderProfileMutationResponse,
    EvmProviderProfileUpsertRequest,
};

use crate::audit_log::AuditEventSpec;

use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) fn list_evm_provider_profiles(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<EvmProviderProfileListResponse> {
        let _ = self.require_session(token)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        Ok(EvmProviderProfileListResponse {
            profiles: registry.evm_providers,
        })
    }

    pub(crate) async fn upsert_evm_provider_profile(
        &self,
        token: Option<&str>,
        body: EvmProviderProfileUpsertRequest,
    ) -> ServiceResult<EvmProviderProfileMutationResponse> {
        let token = self.require_session(token)?;
        validate_profile_name(&body.name)?;
        if body.provider.rpc_url.trim().is_empty() {
            return Err(ServiceError::bad_request("rpc_url is required"));
        }
        if body.chain_id == 0 {
            return Err(ServiceError::bad_request("chain_id must be >= 1"));
        }
        let compartment_id = body
            .provider
            .compartment_id
            .or_else(|| self.state.active_compartment_id_for(token))
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;

        let profile = EvmProviderProfile {
            name: body.name,
            rpc_url: body.provider.rpc_url,
            auth_token_key: body.provider.auth_token_key,
            compartment_id,
            chain_id: body.chain_id,
            max_priority_fee_per_gas_hex: body.max_priority_fee_per_gas_hex,
            max_fee_per_gas_hex: body.max_fee_per_gas_hex,
            native_gas_limit: body.native_gas_limit,
            erc20_gas_limit: body.erc20_gas_limit,
        };

        let _guard = self.state.operation_guard().await;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        upsert_named(&mut registry.evm_providers, profile.clone(), |item| {
            &item.name
        });
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEvmProviderUpsert {
                name: profile.name.clone(),
                chain_id: profile.chain_id,
            },
        )?;

        Ok(EvmProviderProfileMutationResponse {
            status: "ok".into(),
            profile,
        })
    }

    pub(crate) async fn delete_evm_provider_profile(
        &self,
        token: Option<&str>,
        body: EvmProfileDeleteRequest,
    ) -> ServiceResult<EvmProviderProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        if registry
            .eth_stealth_wallets
            .iter()
            .any(|profile| profile.provider_profile == body.name)
            || registry
                .eth_xpub_wallets
                .iter()
                .any(|profile| profile.provider_profile == body.name)
        {
            return Err(ServiceError::conflict(
                "Provider profile is still referenced by a wallet profile.",
            ));
        }
        let profile = remove_named(&mut registry.evm_providers, &body.name, |item| &item.name)
            .ok_or_else(|| ServiceError::not_found("Provider profile not found."))?;
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEvmProviderDelete {
                name: profile.name.clone(),
            },
        )?;

        Ok(EvmProviderProfileMutationResponse {
            status: "deleted".into(),
            profile,
        })
    }

    pub(crate) fn list_eth_stealth_wallet_profiles(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<EthStealthWalletProfileListResponse> {
        let _ = self.require_session(token)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        Ok(EthStealthWalletProfileListResponse {
            profiles: registry.eth_stealth_wallets,
        })
    }

    pub(crate) async fn upsert_eth_stealth_wallet_profile(
        &self,
        token: Option<&str>,
        body: EthStealthWalletProfileUpsertRequest,
    ) -> ServiceResult<EthStealthWalletProfileMutationResponse> {
        let token = self.require_session(token)?;
        validate_profile_name(&body.name)?;
        if body.wallet.trim().is_empty() {
            return Err(ServiceError::bad_request("wallet is required"));
        }
        let compartment_id = body
            .compartment_id
            .or_else(|| self.state.active_compartment_id_for(token))
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;

        let _guard = self.state.operation_guard().await;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        if !registry
            .evm_providers
            .iter()
            .any(|profile| profile.name == body.provider_profile)
        {
            return Err(ServiceError::not_found("Provider profile not found."));
        }

        let profile = EthStealthWalletProfile {
            name: body.name,
            wallet: body.wallet,
            short_name: body.short_name.unwrap_or_else(|| "eth".into()),
            provider_profile: body.provider_profile,
            compartment_id,
            chain_id: body.chain_id,
            default_destination_address: body.default_destination_address,
        };

        upsert_named(&mut registry.eth_stealth_wallets, profile.clone(), |item| {
            &item.name
        });
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEthStealthWalletUpsert {
                name: profile.name.clone(),
                provider_profile: profile.provider_profile.clone(),
            },
        )?;

        Ok(EthStealthWalletProfileMutationResponse {
            status: "ok".into(),
            profile,
        })
    }

    pub(crate) async fn delete_eth_stealth_wallet_profile(
        &self,
        token: Option<&str>,
        body: EvmProfileDeleteRequest,
    ) -> ServiceResult<EthStealthWalletProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        let profile = remove_named(&mut registry.eth_stealth_wallets, &body.name, |item| {
            &item.name
        })
        .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEthStealthWalletDelete {
                name: profile.name.clone(),
            },
        )?;

        Ok(EthStealthWalletProfileMutationResponse {
            status: "deleted".into(),
            profile,
        })
    }

    pub(crate) fn list_eth_xpub_wallet_profiles(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<EthXpubWalletProfileListResponse> {
        let _ = self.require_session(token)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        Ok(EthXpubWalletProfileListResponse {
            profiles: registry.eth_xpub_wallets,
        })
    }

    pub(crate) async fn upsert_eth_xpub_wallet_profile(
        &self,
        token: Option<&str>,
        body: EthXpubWalletProfileUpsertRequest,
    ) -> ServiceResult<EthXpubWalletProfileMutationResponse> {
        let token = self.require_session(token)?;
        validate_profile_name(&body.name)?;
        let compartment_id = body
            .compartment_id
            .or_else(|| self.state.active_compartment_id_for(token))
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;

        let _guard = self.state.operation_guard().await;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        if !registry
            .evm_providers
            .iter()
            .any(|profile| profile.name == body.provider_profile)
        {
            return Err(ServiceError::not_found("Provider profile not found."));
        }

        let profile = EthXpubWalletProfile {
            name: body.name,
            project_account: body.project_account,
            provider_profile: body.provider_profile,
            compartment_id,
            chain_id: body.chain_id,
            default_destination_address: body.default_destination_address,
        };

        upsert_named(&mut registry.eth_xpub_wallets, profile.clone(), |item| {
            &item.name
        });
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEthXpubWalletUpsert {
                name: profile.name.clone(),
                provider_profile: profile.provider_profile.clone(),
            },
        )?;

        Ok(EthXpubWalletProfileMutationResponse {
            status: "ok".into(),
            profile,
        })
    }

    pub(crate) async fn delete_eth_xpub_wallet_profile(
        &self,
        token: Option<&str>,
        body: EvmProfileDeleteRequest,
    ) -> ServiceResult<EthXpubWalletProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut registry =
            crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load profile registry: {error}"))
            })?;
        let profile = remove_named(&mut registry.eth_xpub_wallets, &body.name, |item| {
            &item.name
        })
        .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;
        crate::profiles::save_profiles(&self.state.base_dir, &registry).map_err(|error| {
            ServiceError::internal(format!("Failed to save profile registry: {error}"))
        })?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ProfilesEthXpubWalletDelete {
                name: profile.name.clone(),
            },
        )?;

        Ok(EthXpubWalletProfileMutationResponse {
            status: "deleted".into(),
            profile,
        })
    }

    pub(crate) async fn eth_stealth_send_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendWithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let _ = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        let destination_address = body
            .destination_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "destination_address is required when the wallet profile has no default.",
                )
            })?;

        self.eth_stealth_send_transfer(
            token,
            EthStealthSendTransferRequest {
                rpc_url: provider.rpc_url,
                wallet: wallet.wallet,
                stealth: body.stealth.clone(),
                fees: sigillum_api::Eip1559Fees {
                    chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                    max_priority_fee_per_gas_hex: provider
                        .max_priority_fee_per_gas_hex
                        .clone()
                        .ok_or_else(|| {
                            ServiceError::bad_request(
                                "provider profile is missing max_priority_fee_per_gas_hex",
                            )
                        })?,
                    max_fee_per_gas_hex: provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
                        ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
                    })?,
                },
                destination_address,
                value_wei_hex: body.value_wei_hex,
                auth_token_key: provider.auth_token_key,
                provider_compartment_id: Some(provider.compartment_id),
                wallet_compartment_id: Some(wallet.compartment_id),
                nonce: body.nonce,
                gas_limit: body.gas_limit.or(provider.native_gas_limit),
                broadcast: body.broadcast,
            },
        )
        .await
    }

    pub(crate) async fn eth_stealth_send_erc20_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendErc20WithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let _ = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;

        self.eth_stealth_send_erc20_transfer(
            token,
            EthStealthSendErc20TransferRequest {
                rpc_url: provider.rpc_url,
                wallet: wallet.wallet,
                stealth: body.stealth.clone(),
                fees: sigillum_api::Eip1559Fees {
                    chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                    max_priority_fee_per_gas_hex: provider
                        .max_priority_fee_per_gas_hex
                        .clone()
                        .ok_or_else(|| {
                            ServiceError::bad_request(
                                "provider profile is missing max_priority_fee_per_gas_hex",
                            )
                        })?,
                    max_fee_per_gas_hex: provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
                        ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
                    })?,
                },
                token_address: body.token_address,
                recipient_address: body.recipient_address,
                amount_hex: body.amount_hex,
                auth_token_key: provider.auth_token_key,
                provider_compartment_id: Some(provider.compartment_id),
                wallet_compartment_id: Some(wallet.compartment_id),
                nonce: body.nonce,
                gas_limit: body.gas_limit.or(provider.erc20_gas_limit),
                broadcast: body.broadcast,
            },
        )
        .await
    }

    pub(super) fn resolve_wallet_profile(
        &self,
        name: &str,
    ) -> ServiceResult<(EvmProviderProfile, EthStealthWalletProfile)> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        resolve_wallet_profile_in_registry(&registry, name)
    }

    pub(super) fn resolve_xpub_wallet_profile(
        &self,
        name: &str,
    ) -> ServiceResult<(EvmProviderProfile, EthXpubWalletProfile)> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        resolve_xpub_wallet_profile_in_registry(&registry, name)
    }
}

pub(super) fn resolve_wallet_profile_in_registry(
    registry: &crate::profiles::ProfileRegistry,
    name: &str,
) -> ServiceResult<(EvmProviderProfile, EthStealthWalletProfile)> {
    let wallet = registry
        .eth_stealth_wallets
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;
    let provider = registry
        .evm_providers
        .iter()
        .find(|profile| profile.name == wallet.provider_profile)
        .cloned()
        .ok_or_else(|| ServiceError::not_found("Provider profile not found."))?;
    Ok((provider, wallet))
}

pub(super) fn resolve_xpub_wallet_profile_in_registry(
    registry: &crate::profiles::ProfileRegistry,
    name: &str,
) -> ServiceResult<(EvmProviderProfile, EthXpubWalletProfile)> {
    let wallet = registry
        .eth_xpub_wallets
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;
    let provider = registry
        .evm_providers
        .iter()
        .find(|profile| profile.name == wallet.provider_profile)
        .cloned()
        .ok_or_else(|| ServiceError::not_found("Provider profile not found."))?;
    Ok((provider, wallet))
}

fn validate_profile_name(name: &str) -> ServiceResult<()> {
    if name.trim().is_empty() {
        return Err(ServiceError::bad_request("name is required"));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ServiceError::bad_request(
            "name may only contain letters, numbers, '-' and '_'",
        ));
    }
    Ok(())
}

fn upsert_named<T, F>(items: &mut Vec<T>, item: T, name: F)
where
    F: Fn(&T) -> &str,
{
    if let Some(existing) = items
        .iter_mut()
        .find(|existing| name(existing) == name(&item))
    {
        *existing = item;
    } else {
        items.push(item);
    }
    items.sort_by(|left, right| name(left).cmp(name(right)));
}

fn remove_named<T, F>(items: &mut Vec<T>, target: &str, name: F) -> Option<T>
where
    F: Fn(&T) -> &str,
{
    let index = items.iter().position(|item| name(item) == target)?;
    Some(items.remove(index))
}
