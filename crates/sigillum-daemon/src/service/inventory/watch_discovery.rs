use std::collections::HashSet;

use sigillum_api::WatchAddressProbe;

use crate::service::evm::normalize_address;
use crate::service::{ServiceError, ServiceResult};

use super::WALLET_FAMILY_ETH_WATCH;
use super::wallet_selection::DiscoveryWallet;

#[derive(Clone, Debug)]
pub(super) struct WatchDiscoveryAddress {
    pub(super) wallet: DiscoveryWallet,
    pub(super) address: String,
    pub(super) address_index: u32,
}

pub(super) fn select_watch_addresses(
    probes: &[WatchAddressProbe],
    requested_family: Option<&str>,
    requested_profile: Option<&str>,
) -> ServiceResult<Vec<WatchDiscoveryAddress>> {
    if requested_family.is_some_and(|family| family != WALLET_FAMILY_ETH_WATCH) {
        return Ok(Vec::new());
    }

    let mut watches = Vec::new();
    let mut seen = HashSet::new();
    for (index, probe) in probes.iter().enumerate() {
        let address = normalize_address(&probe.address)?;
        let profile = watch_profile_name(probe, &address);
        if requested_profile.is_some_and(|name| name != profile) {
            continue;
        }
        if !seen.insert((profile.clone(), address.clone())) {
            continue;
        }
        let address_index = u32::try_from(index).map_err(|_| {
            ServiceError::bad_request("watch_addresses exceeds supported index range")
        })?;
        watches.push(WatchDiscoveryAddress {
            wallet: DiscoveryWallet {
                family: WALLET_FAMILY_ETH_WATCH.into(),
                profile: profile.clone(),
                receive_path: format!("m/watch/{profile}"),
                receive_xpub: String::new(),
            },
            address,
            address_index,
        });
    }
    Ok(watches)
}

fn watch_profile_name(probe: &WatchAddressProbe, normalized_address: &str) -> String {
    probe
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(|label| format!("watch:{label}"))
        .unwrap_or_else(|| {
            let suffix = normalized_address
                .get(normalized_address.len().saturating_sub(8)..)
                .unwrap_or(normalized_address);
            format!("watch:{suffix}")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_profile_names_use_labels_or_address_suffix() {
        let labeled = WatchAddressProbe {
            address: "0x7777777777777777777777777777777777777777".into(),
            label: Some("old-ledger".into()),
        };
        let unlabeled = WatchAddressProbe {
            address: "0x888888888888888888888888888888888888abcd".into(),
            label: None,
        };

        assert_eq!(
            watch_profile_name(&labeled, &labeled.address),
            "watch:old-ledger"
        );
        assert_eq!(
            watch_profile_name(&unlabeled, &unlabeled.address),
            "watch:8888abcd"
        );
    }

    #[test]
    fn select_watch_addresses_dedupes_profile_address_pairs() {
        let probes = vec![
            WatchAddressProbe {
                address: "0x7777777777777777777777777777777777777777".into(),
                label: Some("old-ledger".into()),
            },
            WatchAddressProbe {
                address: "0x7777777777777777777777777777777777777777".into(),
                label: Some("old-ledger".into()),
            },
            WatchAddressProbe {
                address: "0x7777777777777777777777777777777777777777".into(),
                label: Some("client".into()),
            },
        ];

        let watches = select_watch_addresses(&probes, Some(WALLET_FAMILY_ETH_WATCH), None).unwrap();

        assert_eq!(watches.len(), 2);
        assert_eq!(watches[0].wallet.profile, "watch:old-ledger");
        assert_eq!(watches[1].wallet.profile, "watch:client");
    }
}
