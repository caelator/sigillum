use std::collections::{BTreeMap, BTreeSet};

use sigillum_api::{
    ConsolidationPlanStep, EthSeedWalletProfile, EvmProviderProfile, TreasuryPolicy,
    WalletAssetKind, WalletInventoryAddress, WalletPlanStepAction, WalletPlanStepStatus,
    WalletSignerStatus, WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::inventory::WalletInventoryState;
use crate::profiles::ProfileRegistry;
use crate::service::SigillumService;
use crate::service::helpers::{compare_u256, multiply_u256_u64, random_id};

use super::WALLET_FAMILY_ETH_SEED;
use super::simulation::{
    FeeBasisResolution, inventory_native_balance_hex_for_step, zero_value_transaction_gas_limit,
};
use super::treasury::{add_u256, encode_quantity_hex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResolvedGasFees {
    max_fee_per_gas_hex: String,
}

pub(super) fn apply_gas_topups(
    policy: Option<&TreasuryPolicy>,
    seed_profiles: &[EthSeedWalletProfile],
    inventory_addresses: &[WalletInventoryAddress],
    fee_basis_for: &dyn Fn(&str, u64) -> Option<ResolvedGasFees>,
    providers: &[EvmProviderProfile],
    steps: Vec<ConsolidationPlanStep>,
) -> Vec<ConsolidationPlanStep> {
    if !gas_topup_policy_enabled(policy) {
        return steps;
    }
    let policy = policy.expect("policy gate checked");

    let mut expanded = Vec::with_capacity(steps.len());
    for mut step in steps {
        if !gas_topup_candidate_step(&step) {
            expanded.push(step);
            continue;
        }
        let Some(provider) = providers.iter().find(|provider| {
            provider.name == step.provider_profile && provider.chain_id == step.chain_id
        }) else {
            expanded.push(step);
            continue;
        };
        let Some(fees) = fee_basis_for(&step.provider_profile, step.chain_id) else {
            expanded.push(step);
            continue;
        };
        let Ok(max_fee) = decode_quantity_hex(&fees.max_fee_per_gas_hex) else {
            expanded.push(step);
            continue;
        };

        let gas_limit = gas_limit_for_step(provider, &step);
        let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
        let Some(shortfall) = step_has_gas_shortfall(inventory_addresses, &step, &gas_cost) else {
            expanded.push(step);
            continue;
        };
        if !shortfall {
            expanded.push(step);
            continue;
        }

        let topup = add_u256(&gas_cost, &shr1_u256(&gas_cost));
        if topup_exceeds_cap(&topup, policy.max_gas_topup_wei_hex.as_deref()) {
            push_unique(
                &mut step.blockers,
                "gas_topup_exceeds_cap:max_gas_topup_wei_hex",
            );
            step.status = WalletPlanStepStatus::Blocked;
            step.risk_level = "blocked".into();
            step.simulation_status = WalletSimulationStatus::NotRun;
            expanded.push(step);
            continue;
        }

        let Some(seed_profile) = seed_profiles
            .iter()
            .find(|profile| profile.name == step.wallet_profile)
        else {
            expanded.push(step);
            continue;
        };
        let Some(sponsor_address) = seed_profile
            .sponsor_address
            .as_deref()
            .map(str::trim)
            .filter(|address| !address.is_empty())
        else {
            expanded.push(step);
            continue;
        };
        if sponsor_address.eq_ignore_ascii_case(&step.address) {
            expanded.push(step);
            continue;
        }

        let Some(sponsor_balance) =
            sponsor_balance_for_step(inventory_addresses, &step, sponsor_address)
        else {
            expanded.push(step);
            continue;
        };
        let sponsor_gas_cost =
            multiply_u256_u64(&max_fee, provider.native_gas_limit.unwrap_or(21_000));
        let required = add_u256(&topup, &sponsor_gas_cost);
        if compare_u256(&sponsor_balance, &required).is_lt() {
            expanded.push(step);
            continue;
        }

        let topup_step = fund_gas_step(seed_profile, sponsor_address, &step, &topup);
        push_unique(&mut step.depends_on, &topup_step.id);
        expanded.push(topup_step);
        expanded.push(step);
    }

    expanded
}

impl SigillumService {
    pub(super) async fn expand_gas_topup_steps(
        &self,
        providers: &[EvmProviderProfile],
        registry: &ProfileRegistry,
        state: &WalletInventoryState,
        steps: Vec<ConsolidationPlanStep>,
    ) -> Vec<ConsolidationPlanStep> {
        if !gas_topup_policy_enabled(state.treasury_policy.as_ref()) {
            return steps;
        }

        let mut provider_keys = BTreeSet::<(String, u64)>::new();
        for step in &steps {
            provider_keys.insert((step.provider_profile.clone(), step.chain_id));
        }

        let mut fee_lookup = BTreeMap::<(String, u64), ResolvedGasFees>::new();
        for (provider_name, chain_id) in provider_keys {
            let Some(provider) = providers
                .iter()
                .find(|provider| provider.name == provider_name && provider.chain_id == chain_id)
            else {
                continue;
            };
            let gas_limit = provider.native_gas_limit.unwrap_or(21_000);
            match self
                .resolve_fee_basis_for_provider_profile(provider, gas_limit)
                .await
            {
                Ok(FeeBasisResolution::Resolved(basis)) => {
                    fee_lookup.insert(
                        (provider.name.clone(), provider.chain_id),
                        ResolvedGasFees {
                            max_fee_per_gas_hex: basis.max_fee_per_gas_hex,
                        },
                    );
                }
                Ok(FeeBasisResolution::MissingStatic { .. }) | Err(_) => {}
            }
        }

        apply_gas_topups(
            state.treasury_policy.as_ref(),
            &registry.eth_seed_wallets,
            &state.addresses,
            &|provider_profile, chain_id| {
                fee_lookup
                    .get(&(provider_profile.to_string(), chain_id))
                    .cloned()
            },
            providers,
            steps,
        )
    }
}

fn gas_topup_policy_enabled(policy: Option<&TreasuryPolicy>) -> bool {
    policy
        .map(|policy| policy.enabled && policy.allow_gas_topups)
        .unwrap_or(false)
}

fn gas_topup_candidate_step(step: &ConsolidationPlanStep) -> bool {
    step.blockers.is_empty()
        && step.wallet_family == WALLET_FAMILY_ETH_SEED
        && step.action != WalletPlanStepAction::FundGas
        && step.action != WalletPlanStepAction::ReviewAsset
}

fn gas_limit_for_step(provider: &EvmProviderProfile, step: &ConsolidationPlanStep) -> u64 {
    if step.action == WalletPlanStepAction::SweepNative {
        provider.native_gas_limit.unwrap_or(21_000)
    } else {
        zero_value_transaction_gas_limit(provider, step)
    }
}

fn step_has_gas_shortfall(
    inventory_addresses: &[WalletInventoryAddress],
    step: &ConsolidationPlanStep,
    gas_cost: &[u8; 32],
) -> Option<bool> {
    if step.action == WalletPlanStepAction::SweepNative {
        let amount = decode_quantity_hex(&step.amount_hex).ok()?;
        return Some(compare_u256(&amount, gas_cost).is_le());
    }

    let balance_hex = inventory_native_balance_hex_for_step(inventory_addresses, step)?;
    let balance = decode_quantity_hex(balance_hex).ok()?;
    Some(compare_u256(&balance, gas_cost).is_lt())
}

fn topup_exceeds_cap(topup: &[u8; 32], cap_hex: Option<&str>) -> bool {
    let Some(cap_hex) = cap_hex else {
        return false;
    };
    let Ok(cap) = decode_quantity_hex(cap_hex) else {
        return false;
    };
    compare_u256(topup, &cap).is_gt()
}

fn sponsor_balance_for_step(
    inventory_addresses: &[WalletInventoryAddress],
    step: &ConsolidationPlanStep,
    sponsor_address: &str,
) -> Option<[u8; 32]> {
    inventory_addresses
        .iter()
        .find(|address| {
            address.wallet_family == step.wallet_family
                && address.wallet_profile == step.wallet_profile
                && address.provider_profile == step.provider_profile
                && address.chain_id == step.chain_id
                && address.address.eq_ignore_ascii_case(sponsor_address)
        })
        .and_then(|address| decode_quantity_hex(&address.native_balance_wei_hex).ok())
}

fn fund_gas_step(
    seed_profile: &EthSeedWalletProfile,
    sponsor_address: &str,
    dependent: &ConsolidationPlanStep,
    topup: &[u8; 32],
) -> ConsolidationPlanStep {
    ConsolidationPlanStep {
        id: random_id(),
        sequence: 0,
        depends_on: Vec::new(),
        action: WalletPlanStepAction::FundGas,
        status: WalletPlanStepStatus::ReviewRequired,
        wallet_family: dependent.wallet_family.clone(),
        wallet_profile: dependent.wallet_profile.clone(),
        provider_profile: dependent.provider_profile.clone(),
        chain_id: dependent.chain_id,
        address: sponsor_address.to_string(),
        derivation_path: format!("m/44'/60'/{}'/1/0", seed_profile.project_account),
        asset_kind: WalletAssetKind::Native,
        asset_address: None,
        token_id_hex: None,
        counterparty_address: None,
        protocol_address: None,
        claim_adapter: None,
        claim_index_hex: None,
        claim_proof: Vec::new(),
        exit_token0_address: None,
        exit_token1_address: None,
        exit_amount0_min_hex: None,
        exit_amount1_min_hex: None,
        exit_deadline_unix: None,
        amount_hex: encode_quantity_hex(topup),
        destination_address: Some(dependent.address.clone()),
        signer_status: WalletSignerStatus::Available,
        simulation_status: WalletSimulationStatus::Required,
        simulation_evidence: Vec::new(),
        risk_level: "low".into(),
        blockers: Vec::new(),
        linkage_warnings: Vec::new(),
        auto_eligible: false,
        approved: false,
    }
}

fn shr1_u256(value: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u8;
    for (index, byte) in value.iter().enumerate() {
        out[index] = (byte >> 1) | carry;
        carry = (byte & 1) << 7;
    }
    out
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "0x1111111111111111111111111111111111111111";
    const SPONSOR: &str = "0x4444444444444444444444444444444444444444";

    fn sample_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: Vec::new(),
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
            allow_claim_execution: false,
            allow_gas_topups: true,
            max_gas_topup_wei_hex: None,
            simulation_freshness_secs: 900,
            hot_floor_wei_hex: "0xde0b6b3a7640000".into(),
            hot_target_wei_hex: "0xde0b6b3a7640000".into(),
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }

    fn sample_provider() -> EvmProviderProfile {
        EvmProviderProfile {
            name: "mainnet".into(),
            rpc_url: "http://127.0.0.1:8545".into(),
            auth_token_key: None,
            compartment_id: 0,
            chain_id: 1,
            max_priority_fee_per_gas_hex: Some("0x1".into()),
            max_fee_per_gas_hex: Some("0x2".into()),
            native_gas_limit: Some(21_000),
            erc20_gas_limit: Some(65_000),
            fee_estimation_enabled: false,
        }
    }

    fn sample_seed_profile(sponsor_address: Option<&str>) -> EthSeedWalletProfile {
        EthSeedWalletProfile {
            name: "seed-main".into(),
            label: Some("Seed main".into()),
            project_account: 0,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            word_count: 12,
            mnemonic_secret_key: "wallet.seed.seed-main.mnemonic".into(),
            account_path: "m/44'/60'/0'".into(),
            receive_path: "m/44'/60'/0'/0".into(),
            receive_xpub: "xpub661MyMwAqRbcFexample".into(),
            first_receive_address: SOURCE.into(),
            default_destination_address: None,
            control_xpub: Some("xpub661MyMwAqRbcControl".into()),
            sponsor_address: sponsor_address.map(str::to_string),
            hot_address: None,
            treasury_address: None,
            execution_enabled: false,
        }
    }

    fn inventory_address(address: &str, native_balance_wei_hex: &str) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: format!("addr_{}", &address[2..6]),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            address_index: 0,
            activity_state: "funded".into(),
            native_balance_wei_hex: native_balance_wei_hex.into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "test".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_step(action: WalletPlanStepAction, amount_hex: &str) -> ConsolidationPlanStep {
        let asset_kind = if action == WalletPlanStepAction::SweepNative {
            WalletAssetKind::Native
        } else {
            WalletAssetKind::Erc20
        };
        ConsolidationPlanStep {
            id: "step_1".into(),
            sequence: 0,
            depends_on: Vec::new(),
            action,
            status: WalletPlanStepStatus::ReviewRequired,
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: SOURCE.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind,
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            exit_token0_address: None,
            exit_token1_address: None,
            exit_amount0_min_hex: None,
            exit_amount1_min_hex: None,
            exit_deadline_unix: None,
            amount_hex: amount_hex.into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: WalletSignerStatus::Available,
            simulation_status: WalletSimulationStatus::Required,
            simulation_evidence: Vec::new(),
            risk_level: "low".into(),
            blockers: Vec::new(),
            linkage_warnings: Vec::new(),
            auto_eligible: false,
            approved: false,
        }
    }

    fn run_apply(
        policy: Option<&TreasuryPolicy>,
        seed_profiles: Vec<EthSeedWalletProfile>,
        inventory_addresses: Vec<WalletInventoryAddress>,
        steps: Vec<ConsolidationPlanStep>,
    ) -> Vec<ConsolidationPlanStep> {
        let providers = vec![sample_provider()];
        apply_gas_topups(
            policy,
            &seed_profiles,
            &inventory_addresses,
            &|provider_profile, chain_id| {
                if provider_profile == "mainnet" && chain_id == 1 {
                    Some(ResolvedGasFees {
                        max_fee_per_gas_hex: "0x2".into(),
                    })
                } else {
                    None
                }
            },
            &providers,
            steps,
        )
    }

    #[test]
    fn gas_topup_shortfall_with_sponsor_emits_capped_fund_gas_step() {
        let policy = sample_policy();
        let dependent = sample_step(WalletPlanStepAction::SweepErc20, "0xf4240");
        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x0"),
                inventory_address(SPONSOR, "0xde0b6b3a7640000"),
            ],
            vec![dependent],
        );

        assert_eq!(output.len(), 2);
        assert_eq!(output[0].action, WalletPlanStepAction::FundGas);
        assert_eq!(output[0].address, SPONSOR);
        assert_eq!(output[0].destination_address.as_deref(), Some(SOURCE));
        assert_eq!(output[0].amount_hex, "0x2f9b8");
        assert_eq!(output[0].status, WalletPlanStepStatus::ReviewRequired);
        assert_eq!(
            output[0].simulation_status,
            WalletSimulationStatus::Required
        );
        assert_eq!(output[0].derivation_path, "m/44'/60'/0'/1/0");
        assert_eq!(output[0].asset_kind, WalletAssetKind::Native);
        assert_eq!(output[1].id, "step_1");
        assert_eq!(output[1].depends_on, vec![output[0].id.clone()]);
    }

    #[test]
    fn gas_topup_amount_respects_cap() {
        let mut policy = sample_policy();
        policy.max_gas_topup_wei_hex = Some("0x2f9b8".into());
        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x0"),
                inventory_address(SPONSOR, "0x100000"),
            ],
            vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")],
        );

        assert_eq!(output.len(), 2);
        assert_eq!(output[0].action, WalletPlanStepAction::FundGas);
        assert_eq!(output[0].amount_hex, "0x2f9b8");
    }

    #[test]
    fn gas_topup_over_cap_blocks_dependent_naming_cap() {
        let mut policy = sample_policy();
        policy.max_gas_topup_wei_hex = Some("0x2f9b7".into());
        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x0"),
                inventory_address(SPONSOR, "0x100000"),
            ],
            vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")],
        );

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].action, WalletPlanStepAction::SweepErc20);
        assert_eq!(output[0].status, WalletPlanStepStatus::Blocked);
        assert_eq!(output[0].risk_level, "blocked");
        assert_eq!(output[0].simulation_status, WalletSimulationStatus::NotRun);
        assert!(
            output[0]
                .blockers
                .contains(&"gas_topup_exceeds_cap:max_gas_topup_wei_hex".into())
        );
    }

    #[test]
    fn gas_topup_policy_off_leaves_steps_byte_identical() {
        let mut policy = sample_policy();
        policy.allow_gas_topups = false;
        let input = vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![inventory_address(SOURCE, "0x0")],
            input.clone(),
        );
        assert_eq!(output, input);

        let output = run_apply(
            None,
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![inventory_address(SOURCE, "0x0")],
            input.clone(),
        );
        assert_eq!(output, input);
    }

    #[test]
    fn gas_topup_disabled_policy_leaves_steps_byte_identical() {
        let mut policy = sample_policy();
        policy.enabled = false;
        let input = vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![inventory_address(SOURCE, "0x0")],
            input.clone(),
        );

        assert_eq!(output, input);
    }

    #[test]
    fn gas_topup_without_sponsor_leaves_existing_gas_blocker_path_untouched() {
        let policy = sample_policy();
        let input = vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(None)],
            vec![inventory_address(SOURCE, "0x0")],
            input.clone(),
        );

        assert_eq!(output, input);
    }

    #[test]
    fn gas_topup_insufficient_sponsor_balance_emits_nothing() {
        let policy = sample_policy();
        let input = vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x0"),
                inventory_address(SPONSOR, "0x39dc7"),
            ],
            input.clone(),
        );

        assert_eq!(output, input);

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x0"),
                inventory_address(SPONSOR, "0x39dc8"),
            ],
            input.clone(),
        );

        assert_eq!(output.len(), 2);
        assert_eq!(output[0].action, WalletPlanStepAction::FundGas);
        assert_eq!(output[0].amount_hex, "0x2f9b8");
    }

    #[test]
    fn gas_topup_unknown_sponsor_balance_emits_nothing() {
        let policy = sample_policy();
        let input = vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![inventory_address(SOURCE, "0x0")],
            input.clone(),
        );

        assert_eq!(output, input);
    }

    #[test]
    fn gas_topup_not_emitted_when_balance_covers_gas() {
        let policy = sample_policy();
        let input = vec![sample_step(WalletPlanStepAction::SweepErc20, "0xf4240")];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x1fbd0"),
                inventory_address(SPONSOR, "0x100000"),
            ],
            input.clone(),
        );

        assert_eq!(output, input);
    }

    #[test]
    fn gas_topup_native_sweep_shortfall_gets_topup() {
        let policy = sample_policy();
        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![inventory_address(SPONSOR, "0x100000")],
            vec![sample_step(WalletPlanStepAction::SweepNative, "0xa410")],
        );

        assert_eq!(output.len(), 2);
        assert_eq!(output[0].action, WalletPlanStepAction::FundGas);
        assert_eq!(output[0].amount_hex, "0xf618");
        assert_eq!(output[1].depends_on, vec![output[0].id.clone()]);
    }

    #[test]
    fn gas_topup_skips_already_blocked_steps() {
        let policy = sample_policy();
        let mut step = sample_step(WalletPlanStepAction::SweepErc20, "0xf4240");
        step.blockers.push("watch_only".into());
        step.status = WalletPlanStepStatus::Blocked;
        let input = vec![step];

        let output = run_apply(
            Some(&policy),
            vec![sample_seed_profile(Some(SPONSOR))],
            vec![
                inventory_address(SOURCE, "0x0"),
                inventory_address(SPONSOR, "0x100000"),
            ],
            input.clone(),
        );

        assert_eq!(output, input);
    }
}
