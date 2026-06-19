use secrecy::ExposeSecret;
use serde::Deserialize;
use sigillum_api::{EthSeedWalletProfile, EthXpubWalletProfile};
use sigillum_core::{
    EthereumXpubError, EthereumXpubReceiveAddress, SecretStore, VaultLifecycle,
    derive_ethereum_address_from_imported_xpub, derive_ethereum_address_from_xpub,
    derive_ethereum_receive_branch_from_account_xpub,
    derive_ethereum_receive_branch_from_account_xpub_with_path,
    derive_ethereum_xpub_receive_branch_from_mnemonic,
    derive_sigillum_ethereum_xpub_receive_branch,
};
use zeroize::Zeroize;

use crate::service::helpers::map_xpub_error;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB};

pub(super) const DERIVATION_PATTERN_PROJECT: &str = "project";
pub(super) const DERIVATION_PATTERN_STANDARD: &str = "standard";
pub(super) const DERIVATION_PATTERN_LEDGER_LIVE: &str = "ledger_live";
pub(in crate::service::inventory) const DERIVATION_PATTERN_IMPORTED_XPUB: &str = "imported_xpub";

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

pub(in crate::service::inventory) fn derive_discovery_wallet_address(
    wallet: &DiscoveryWallet,
    index: u32,
) -> Result<EthereumXpubReceiveAddress, EthereumXpubError> {
    if wallet.derivation_pattern == DERIVATION_PATTERN_IMPORTED_XPUB {
        derive_ethereum_address_from_imported_xpub(&wallet.receive_xpub, index)
    } else {
        derive_ethereum_address_from_xpub(&wallet.receive_xpub, index)
    }
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
            let wallet = if let Some(receive_xpub) = profile
                .external_receive_xpub
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                DiscoveryWallet {
                    family: WALLET_FAMILY_ETH_XPUB.into(),
                    profile: profile.name.clone(),
                    receive_path: profile
                        .external_receive_path
                        .clone()
                        .unwrap_or_else(|| eth_receive_path(profile.project_account)),
                    receive_xpub: receive_xpub.to_string(),
                    derivation_pattern: DERIVATION_PATTERN_IMPORTED_XPUB.into(),
                    account_index: profile.project_account,
                }
            } else if let Some(account_xpub) = profile
                .external_account_xpub
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let export = if let Some(path) = profile
                    .external_account_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    derive_ethereum_receive_branch_from_account_xpub_with_path(
                        account_xpub,
                        path,
                        profile.project_account,
                    )
                } else {
                    derive_ethereum_receive_branch_from_account_xpub(
                        account_xpub,
                        profile.project_account,
                    )
                }
                .map_err(map_xpub_error)?;
                DiscoveryWallet {
                    family: WALLET_FAMILY_ETH_XPUB.into(),
                    profile: profile.name.clone(),
                    receive_path: export.receive_path,
                    receive_xpub: export.receive_xpub,
                    derivation_pattern: DERIVATION_PATTERN_IMPORTED_XPUB.into(),
                    account_index: profile.project_account,
                }
            } else {
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
                DiscoveryWallet {
                    family: WALLET_FAMILY_ETH_XPUB.into(),
                    profile: profile.name.clone(),
                    receive_path: export.receive_path,
                    receive_xpub: export.receive_xpub,
                    derivation_pattern: DERIVATION_PATTERN_PROJECT.into(),
                    account_index: profile.project_account,
                }
            };
            wallets.push(wallet);
        }
    }

    Ok(wallets)
}

pub(in crate::service::inventory) fn eth_account_path(project_account: u32) -> String {
    format!("m/44'/60'/{project_account}'")
}

pub(in crate::service::inventory) fn eth_receive_path(project_account: u32) -> String {
    format!("{}/0", eth_account_path(project_account))
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_core::{
        derive_ethereum_account_xpub_from_mnemonic,
        derive_ethereum_xpub_control_branch_from_mnemonic,
        derive_ethereum_xpub_receive_branch_from_mnemonic,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::AppState;

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn imported_xpub_profile_selects_receive_branch_without_vault() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state);
        let imported =
            derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let profile = EthXpubWalletProfile {
            name: "external-ledger".into(),
            project_account: 0,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            external_receive_xpub: Some(imported.receive_xpub.clone()),
            external_receive_path: None,
            external_account_xpub: None,
            external_account_path: None,
            default_destination_address: None,
            execution_enabled: false,
        };

        let wallets = select_discovery_wallets(
            &service,
            &[],
            &[profile],
            Some(WALLET_FAMILY_ETH_XPUB),
            Some("external-ledger"),
            SeedDerivationPattern::Project,
            1,
        )
        .unwrap();

        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].receive_xpub, imported.receive_xpub);
        assert_eq!(wallets[0].receive_path, imported.receive_path);
        assert_eq!(
            wallets[0].derivation_pattern,
            DERIVATION_PATTERN_IMPORTED_XPUB
        );
    }

    #[test]
    fn imported_account_xpub_profile_selects_normalized_receive_branch() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state);
        let account_xpub =
            derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let expected =
            derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let profile = EthXpubWalletProfile {
            name: "external-ledger".into(),
            project_account: 0,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            external_receive_xpub: None,
            external_receive_path: None,
            external_account_xpub: Some(account_xpub),
            external_account_path: None,
            default_destination_address: None,
            execution_enabled: false,
        };

        let wallets = select_discovery_wallets(
            &service,
            &[],
            &[profile],
            Some(WALLET_FAMILY_ETH_XPUB),
            Some("external-ledger"),
            SeedDerivationPattern::Project,
            1,
        )
        .unwrap();

        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].receive_xpub, expected.receive_xpub);
        assert_eq!(wallets[0].receive_path, expected.receive_path);
        assert_eq!(
            wallets[0].derivation_pattern,
            DERIVATION_PATTERN_IMPORTED_XPUB
        );
    }

    #[test]
    fn imported_account_xpub_custom_path_profile_selects_operator_path() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state);
        let account_xpub =
            derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let profile = EthXpubWalletProfile {
            name: "external-ledger".into(),
            project_account: 99,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            external_receive_xpub: None,
            external_receive_path: None,
            external_account_xpub: Some(account_xpub),
            external_account_path: Some("m/44'/60'/99'".into()),
            default_destination_address: None,
            execution_enabled: false,
        };

        let wallets = select_discovery_wallets(
            &service,
            &[],
            &[profile],
            Some(WALLET_FAMILY_ETH_XPUB),
            Some("external-ledger"),
            SeedDerivationPattern::Project,
            1,
        )
        .unwrap();

        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].receive_path, "m/44'/60'/99'/0");
        assert_eq!(wallets[0].account_index, 99);
        assert_eq!(
            wallets[0].derivation_pattern,
            DERIVATION_PATTERN_IMPORTED_XPUB
        );
    }

    #[test]
    fn imported_custom_path_xpub_profile_selects_operator_path() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state);
        let imported =
            derive_ethereum_xpub_control_branch_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
        let profile = EthXpubWalletProfile {
            name: "external-control".into(),
            project_account: 99,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            external_receive_xpub: Some(imported.receive_xpub.clone()),
            external_receive_path: Some(imported.receive_path.clone()),
            external_account_xpub: None,
            external_account_path: None,
            default_destination_address: None,
            execution_enabled: false,
        };

        let wallets = select_discovery_wallets(
            &service,
            &[],
            &[profile],
            Some(WALLET_FAMILY_ETH_XPUB),
            Some("external-control"),
            SeedDerivationPattern::Project,
            1,
        )
        .unwrap();

        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].receive_xpub, imported.receive_xpub);
        assert_eq!(wallets[0].receive_path, imported.receive_path);
        assert_eq!(wallets[0].account_index, 99);
        assert_eq!(
            derive_discovery_wallet_address(&wallets[0], 3).unwrap(),
            derive_ethereum_address_from_imported_xpub(&imported.receive_xpub, 3).unwrap()
        );
    }
}
