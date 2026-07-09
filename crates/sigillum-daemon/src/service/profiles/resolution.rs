//! Profile lookup helpers shared by deposits, queue execution, and wallets.

use sigillum_api::{
    EthSeedWalletProfile, EthStealthWalletProfile, EthXpubWalletProfile, EvmProviderProfile,
};

use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(in crate::service) fn resolve_wallet_profile(
        &self,
        name: &str,
    ) -> ServiceResult<(EvmProviderProfile, EthStealthWalletProfile)> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        resolve_wallet_profile_in_registry(&registry, name)
    }

    pub(in crate::service) fn resolve_xpub_wallet_profile(
        &self,
        name: &str,
    ) -> ServiceResult<(EvmProviderProfile, EthXpubWalletProfile)> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        resolve_xpub_wallet_profile_in_registry(&registry, name)
    }

    /// Resolve an `eth-seed` wallet profile and its provider profile by name
    /// (W7.3 signer resolution and EthSeed* queue execution).
    pub(in crate::service) fn resolve_eth_seed_wallet_profile(
        &self,
        name: &str,
    ) -> ServiceResult<(EvmProviderProfile, EthSeedWalletProfile)> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let wallet = registry
            .eth_seed_wallets
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
}

pub(in crate::service) fn resolve_wallet_profile_in_registry(
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

fn resolve_xpub_wallet_profile_in_registry(
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
