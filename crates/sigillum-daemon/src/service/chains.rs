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
