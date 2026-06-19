use sigillum_api::{EthSeedWalletProfile, EthXpubWalletProfile};
use sigillum_core::{VaultLifecycle, derive_sigillum_ethereum_xpub_receive_branch};

use crate::service::helpers::map_xpub_error;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB};

#[derive(Clone, Debug)]
pub(in crate::service::inventory) struct DiscoveryWallet {
    pub(in crate::service::inventory) family: String,
    pub(in crate::service::inventory) profile: String,
    pub(in crate::service::inventory) receive_path: String,
    pub(in crate::service::inventory) receive_xpub: String,
}

pub(super) fn select_discovery_wallets(
    service: &SigillumService,
    seed_profiles: &[EthSeedWalletProfile],
    xpub_profiles: &[EthXpubWalletProfile],
    requested_family: Option<&str>,
    requested_profile: Option<&str>,
) -> ServiceResult<Vec<DiscoveryWallet>> {
    let mut wallets = Vec::new();

    if requested_family.is_none() || requested_family == Some(WALLET_FAMILY_ETH_SEED) {
        for profile in seed_profiles {
            if requested_profile.is_some_and(|name| name != profile.name) {
                continue;
            }
            wallets.push(DiscoveryWallet {
                family: WALLET_FAMILY_ETH_SEED.into(),
                profile: profile.name.clone(),
                receive_path: profile.receive_path.clone(),
                receive_xpub: profile.receive_xpub.clone(),
            });
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
            });
        }
    }

    Ok(wallets)
}
