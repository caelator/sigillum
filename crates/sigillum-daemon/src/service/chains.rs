use sigillum_api::{ChainProfile, DEFAULT_DORMANCY_BLOCK_WINDOW};

pub(crate) const BUILTIN_CHAIN_SOURCE: &str = "builtin";
pub(crate) const CHAIN_FAMILY_EVM: &str = "evm";

#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinChainSpec {
    pub(crate) name: &'static str,
    pub(crate) chain_id: u64,
    pub(crate) native_symbol: &'static str,
}

pub(crate) const BUILTIN_CHAIN_SPECS: &[BuiltinChainSpec] = &[
    BuiltinChainSpec {
        name: "ethereum",
        chain_id: 1,
        native_symbol: "ETH",
    },
    BuiltinChainSpec {
        name: "base",
        chain_id: 8453,
        native_symbol: "ETH",
    },
    BuiltinChainSpec {
        name: "arbitrum-one",
        chain_id: 42161,
        native_symbol: "ETH",
    },
    BuiltinChainSpec {
        name: "op-mainnet",
        chain_id: 10,
        native_symbol: "ETH",
    },
    BuiltinChainSpec {
        name: "polygon-pos",
        chain_id: 137,
        native_symbol: "POL",
    },
];

pub(crate) fn builtin_chain_profile(spec: BuiltinChainSpec) -> ChainProfile {
    ChainProfile {
        name: spec.name.into(),
        chain_family: CHAIN_FAMILY_EVM.into(),
        chain_id: Some(spec.chain_id),
        provider_profile: None,
        native_symbol: spec.native_symbol.into(),
        native_decimals: 18,
        finality_blocks: 0,
        dormancy_block_window: DEFAULT_DORMANCY_BLOCK_WINDOW,
        permit2_address: None,
        uniswap_v2_router_address: None,
        explorer_url: None,
        capabilities: Vec::new(),
        enabled: true,
        source: BUILTIN_CHAIN_SOURCE.into(),
        builtin: true,
        created_at_unix: 0,
        updated_at_unix: 0,
    }
}

pub(crate) fn ensure_builtin_chain_profiles(profiles: &mut Vec<ChainProfile>) {
    for profile in profiles.iter_mut() {
        if profile.dormancy_block_window == 0 {
            profile.dormancy_block_window = DEFAULT_DORMANCY_BLOCK_WINDOW;
        }
    }
    for spec in BUILTIN_CHAIN_SPECS {
        if let Some(existing) = profiles
            .iter_mut()
            .find(|profile| profile.chain_id == Some(spec.chain_id))
        {
            promote_existing_builtin(existing, *spec);
        } else {
            profiles.push(builtin_chain_profile(*spec));
        }
    }
    profiles.sort_by_key(|profile| (profile.chain_id.unwrap_or(u64::MAX), profile.name.clone()));
}

pub(crate) fn chain_profile_for_id(
    profiles: &[ChainProfile],
    chain_id: u64,
) -> Option<&ChainProfile> {
    profiles
        .iter()
        .find(|profile| profile.chain_id == Some(chain_id) && profile.enabled)
}

/// W7.4: conservative confirmation depth assumed for a `chain_id` with no
/// registered (or disabled) `ChainProfile`. A plan step's execution payload
/// always carries a `chain_id`, but nothing guarantees an operator has
/// registered a profile for it (custom/unlisted chains); fail toward MORE
/// confirmations rather than fewer. 12 blocks matches the conservative
/// depth commonly used for pre-finality EVM confirmation (roughly
/// mainnet's historical "safe" reorg-resistance heuristic).
pub(crate) const DEFAULT_FINALITY_BLOCKS_WHEN_UNREGISTERED: u64 = 12;

/// Registry-driven confirmation depth for a chain (W1.1's `finality_blocks`,
/// consumed by W7.4's receipt-confirmation polling). A REGISTERED profile's
/// `finality_blocks` is honored exactly as configured, including an explicit
/// `0` (confirm on any mined receipt) — `0` is only a documented DEFAULT for
/// an unregistered chain when there is no profile to consult at all.
pub(crate) fn finality_blocks_for_chain(profiles: &[ChainProfile], chain_id: u64) -> u64 {
    chain_profile_for_id(profiles, chain_id)
        .map(|profile| profile.finality_blocks)
        .unwrap_or(DEFAULT_FINALITY_BLOCKS_WHEN_UNREGISTERED)
}

fn promote_existing_builtin(profile: &mut ChainProfile, spec: BuiltinChainSpec) {
    if profile.name.trim().is_empty() {
        profile.name = spec.name.into();
    }
    profile.chain_family = CHAIN_FAMILY_EVM.into();
    profile.native_symbol = spec.native_symbol.into();
    if profile.native_decimals == 0 {
        profile.native_decimals = 18;
    }
    if profile.dormancy_block_window == 0 {
        profile.dormancy_block_window = DEFAULT_DORMANCY_BLOCK_WINDOW;
    }
    profile.source = BUILTIN_CHAIN_SOURCE.into();
    profile.builtin = true;
    profile.enabled = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(chain_id: u64, finality_blocks: u64, enabled: bool) -> ChainProfile {
        let mut profile = builtin_chain_profile(BuiltinChainSpec {
            name: "test-chain",
            chain_id,
            native_symbol: "ETH",
        });
        profile.finality_blocks = finality_blocks;
        profile.enabled = enabled;
        profile
    }

    #[test]
    fn finality_blocks_uses_registered_profile_value_including_zero() {
        let profiles = vec![profile(1, 0, true), profile(8453, 25, true)];
        assert_eq!(finality_blocks_for_chain(&profiles, 1), 0);
        assert_eq!(finality_blocks_for_chain(&profiles, 8453), 25);
    }

    #[test]
    fn finality_blocks_falls_back_to_conservative_default_when_unregistered() {
        let profiles = vec![profile(1, 0, true)];
        assert_eq!(
            finality_blocks_for_chain(&profiles, 999_999),
            DEFAULT_FINALITY_BLOCKS_WHEN_UNREGISTERED
        );
    }

    #[test]
    fn finality_blocks_falls_back_to_conservative_default_when_disabled() {
        let profiles = vec![profile(1, 3, false)];
        assert_eq!(
            finality_blocks_for_chain(&profiles, 1),
            DEFAULT_FINALITY_BLOCKS_WHEN_UNREGISTERED
        );
    }
}
