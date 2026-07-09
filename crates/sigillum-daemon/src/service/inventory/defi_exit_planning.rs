use sha3::{Digest, Keccak256};
use sigillum_api::{
    ChainProfile, ConsolidationPlanStep, EvmProviderProfile, WalletPlanStepAction,
    WalletPlanStepStatus, WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::service::SigillumService;
use crate::service::evm::EvmContractCallPreflight;
use crate::service::evm::normalize_address;
use crate::service::helpers::{now_unix, random_id};

use super::defi_adapters::DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY;

const UNISWAP_V2_REMOVE_LIQUIDITY_DEADLINE_SECS: u64 = 1_800;
const UNISWAP_V2_AMOUNT_MIN_NUMERATOR: u128 = 995;
const UNISWAP_V2_AMOUNT_MIN_DENOMINATOR: u128 = 1_000;

impl SigillumService {
    pub(super) async fn expand_defi_exit_steps(
        &self,
        providers: &[EvmProviderProfile],
        chain_profiles: &[ChainProfile],
        steps: Vec<ConsolidationPlanStep>,
    ) -> Vec<ConsolidationPlanStep> {
        let mut expanded = Vec::with_capacity(steps.len());
        for step in steps {
            if should_expand_uniswap_v2_step(&step) {
                expanded.extend(
                    self.expand_uniswap_v2_remove_liquidity_step(providers, chain_profiles, step)
                        .await,
                );
            } else {
                expanded.push(step);
            }
        }
        expanded
    }

    async fn expand_uniswap_v2_remove_liquidity_step(
        &self,
        providers: &[EvmProviderProfile],
        chain_profiles: &[ChainProfile],
        step: ConsolidationPlanStep,
    ) -> Vec<ConsolidationPlanStep> {
        let Some(router) = uniswap_v2_router_for_chain(chain_profiles, step.chain_id) else {
            return vec![blocked_uniswap_v2_step(step, "missing_uniswap_v2_router")];
        };
        let Some(provider) = providers.iter().find(|provider| {
            provider.name == step.provider_profile && provider.chain_id == step.chain_id
        }) else {
            return vec![blocked_uniswap_v2_step(step, "missing_provider_profile")];
        };

        let Some(pair) = step
            .asset_address
            .as_deref()
            .and_then(|address| normalize_address(address).ok())
        else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_pool_state_unreadable",
            )];
        };

        let Some(pool_state) = self
            .read_uniswap_v2_pool_state(provider, &step, &pair)
            .await
        else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_pool_state_unreadable",
            )];
        };

        let Some(liquidity) = quantity_u128_from_hex(&step.amount_hex) else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_amounts_unsupported",
            )];
        };
        if pool_state.total_supply == 0 || liquidity > pool_state.total_supply {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_amounts_unsupported",
            )];
        }
        let Some(expected0) = mul_div_u128(liquidity, pool_state.reserve0, pool_state.total_supply)
        else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_amounts_unsupported",
            )];
        };
        let Some(expected1) = mul_div_u128(liquidity, pool_state.reserve1, pool_state.total_supply)
        else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_amounts_unsupported",
            )];
        };
        let Some(amount0_min) = mul_div_u128(
            expected0,
            UNISWAP_V2_AMOUNT_MIN_NUMERATOR,
            UNISWAP_V2_AMOUNT_MIN_DENOMINATOR,
        ) else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_amounts_unsupported",
            )];
        };
        let Some(amount1_min) = mul_div_u128(
            expected1,
            UNISWAP_V2_AMOUNT_MIN_NUMERATOR,
            UNISWAP_V2_AMOUNT_MIN_DENOMINATOR,
        ) else {
            return vec![blocked_uniswap_v2_step(
                step,
                "uniswap_v2_amounts_unsupported",
            )];
        };

        let deadline_unix = now_unix().saturating_add(UNISWAP_V2_REMOVE_LIQUIDITY_DEADLINE_SECS);
        let mut approve_step = step.clone();
        approve_step.id = random_id();
        approve_step.action = WalletPlanStepAction::ApproveErc20;
        approve_step.status = WalletPlanStepStatus::ReviewRequired;
        approve_step.simulation_status = WalletSimulationStatus::Required;
        approve_step.blockers.clear();
        approve_step.counterparty_address = Some(router.clone());
        approve_step.protocol_address = Some(pair);
        approve_step.destination_address = None;
        approve_step.exit_token0_address = None;
        approve_step.exit_token1_address = None;
        approve_step.exit_amount0_min_hex = None;
        approve_step.exit_amount1_min_hex = None;
        approve_step.exit_deadline_unix = None;
        approve_step.depends_on.clear();

        let approve_step_id = approve_step.id.clone();
        let mut remove_step = step;
        remove_step.action = WalletPlanStepAction::ExitDefiPosition;
        remove_step.status = WalletPlanStepStatus::ReviewRequired;
        remove_step.simulation_status = WalletSimulationStatus::Required;
        remove_step.blockers.clear();
        remove_step.protocol_address = Some(router);
        remove_step.counterparty_address = None;
        remove_step.exit_token0_address = Some(pool_state.token0);
        remove_step.exit_token1_address = Some(pool_state.token1);
        remove_step.exit_amount0_min_hex = Some(canonical_quantity_hex_u128(amount0_min));
        remove_step.exit_amount1_min_hex = Some(canonical_quantity_hex_u128(amount1_min));
        remove_step.exit_deadline_unix = Some(deadline_unix);
        remove_step.depends_on = vec![approve_step_id];

        vec![approve_step, remove_step]
    }

    async fn read_uniswap_v2_pool_state(
        &self,
        provider: &EvmProviderProfile,
        step: &ConsolidationPlanStep,
        pair: &str,
    ) -> Option<UniswapV2PoolState> {
        let token0 = self
            .uniswap_v2_pair_call_word(provider, step, pair, "token0()")
            .await
            .and_then(|word| address_from_abi_word(&word))?;
        let token1 = self
            .uniswap_v2_pair_call_word(provider, step, pair, "token1()")
            .await
            .and_then(|word| address_from_abi_word(&word))?;
        let reserves = self
            .uniswap_v2_pair_call_words(provider, step, pair, "getReserves()", 3)
            .await?;
        let reserve0 = word_u128(&reserves[0])?;
        let reserve1 = word_u128(&reserves[1])?;
        let total_supply = self
            .uniswap_v2_pair_call_word(provider, step, pair, "totalSupply()")
            .await
            .and_then(|word| word_u128(&word))?;

        Some(UniswapV2PoolState {
            token0,
            token1,
            reserve0,
            reserve1,
            total_supply,
        })
    }

    async fn uniswap_v2_pair_call_word(
        &self,
        provider: &EvmProviderProfile,
        step: &ConsolidationPlanStep,
        pair: &str,
        signature: &str,
    ) -> Option<String> {
        self.uniswap_v2_pair_call_words(provider, step, pair, signature, 1)
            .await
            .and_then(|mut words| words.pop())
    }

    async fn uniswap_v2_pair_call_words(
        &self,
        provider: &EvmProviderProfile,
        step: &ConsolidationPlanStep,
        pair: &str,
        signature: &str,
        expected_words: usize,
    ) -> Option<Vec<String>> {
        let data_hex = format!("0x{}", function_selector_hex(signature));
        let result = self
            .evm_contract_call_preflight_for_provider(
                provider.compartment_id,
                provider,
                EvmContractCallPreflight {
                    from_address: &step.address,
                    target_address: pair,
                    data_hex: &data_hex,
                    value_hex: None,
                    block_tag: "latest",
                },
            )
            .await
            .ok()?;
        strict_words_hex(&result, expected_words)
    }
}

#[derive(Clone, Debug)]
struct UniswapV2PoolState {
    token0: String,
    token1: String,
    reserve0: u128,
    reserve1: u128,
    total_supply: u128,
}

fn should_expand_uniswap_v2_step(step: &ConsolidationPlanStep) -> bool {
    step.action == WalletPlanStepAction::ExitDefiPosition
        && step.claim_adapter.as_deref() == Some(DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY)
        && step.blockers.is_empty()
}

fn uniswap_v2_router_for_chain(chain_profiles: &[ChainProfile], chain_id: u64) -> Option<String> {
    chain_profiles
        .iter()
        .find(|profile| profile.chain_id == Some(chain_id))
        .and_then(|profile| profile.uniswap_v2_router_address.as_deref())
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .and_then(|address| normalize_address(address).ok())
}

fn blocked_uniswap_v2_step(
    mut step: ConsolidationPlanStep,
    blocker: &'static str,
) -> ConsolidationPlanStep {
    if !step.blockers.iter().any(|existing| existing == blocker) {
        step.blockers.push(blocker.into());
    }
    step.status = WalletPlanStepStatus::Blocked;
    step.simulation_status = WalletSimulationStatus::NotRun;
    step.risk_level = "blocked".into();
    step
}

fn strict_words_hex(value: &str, expected_words: usize) -> Option<Vec<String>> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.len() != expected_words.checked_mul(64)?
        || !raw.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    raw.as_bytes()
        .chunks_exact(64)
        .map(|chunk| Some(std::str::from_utf8(chunk).ok()?.to_ascii_lowercase()))
        .collect::<Option<Vec<_>>>()
}

fn address_from_abi_word(word: &str) -> Option<String> {
    if word.len() != 64 || !word[..24].bytes().all(|byte| byte == b'0') {
        return None;
    }
    normalize_address(&format!("0x{}", &word[24..])).ok()
}

fn word_u128(word: &str) -> Option<u128> {
    quantity_u128_from_hex(&format!("0x{word}"))
}

fn quantity_u128_from_hex(value: &str) -> Option<u128> {
    let bytes = decode_quantity_hex(value).ok()?;
    if bytes[..16].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(u128::from_be_bytes(bytes[16..].try_into().ok()?))
}

fn canonical_quantity_hex_u128(value: u128) -> String {
    if value == 0 {
        "0x0".into()
    } else {
        format!("0x{value:x}")
    }
}

fn function_selector_hex(signature: &str) -> String {
    let digest = Keccak256::digest(signature.as_bytes());
    hex::encode(&digest[..4])
}

fn mul_div_u128(a: u128, b: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    let product = mul_u128_to_u256(a, b);
    let denominator = [(d as u64), (d >> 64) as u64, 0, 0];
    let mut quotient = [0u64; 4];
    let mut remainder = [0u64; 4];

    for bit in (0..256).rev() {
        shl1_u256(&mut remainder);
        if bit_is_set(&product, bit) {
            remainder[0] |= 1;
        }
        if cmp_u256(&remainder, &denominator).is_ge() {
            sub_assign_u256(&mut remainder, &denominator);
            set_bit(&mut quotient, bit);
        }
    }

    if quotient[2] != 0 || quotient[3] != 0 {
        return None;
    }
    Some(((quotient[1] as u128) << 64) | quotient[0] as u128)
}

fn mul_u128_to_u256(a: u128, b: u128) -> [u64; 4] {
    let a_limbs = [a as u64, (a >> 64) as u64];
    let b_limbs = [b as u64, (b >> 64) as u64];
    let mut out = [0u64; 4];
    for (a_index, a_limb) in a_limbs.iter().copied().enumerate() {
        for (b_index, b_limb) in b_limbs.iter().copied().enumerate() {
            add_product_at(&mut out, a_index + b_index, a_limb, b_limb);
        }
    }
    out
}

fn add_product_at(out: &mut [u64; 4], offset: usize, a: u64, b: u64) {
    let product = (a as u128) * (b as u128);
    let lo = product as u64;
    let hi = (product >> 64) as u64;
    let carry = add_limb(out, offset, lo);
    let carry = add_limb(out, offset + 1, hi.wrapping_add(carry));
    if hi == u64::MAX && carry > 0 {
        let _ = add_limb(out, offset + 2, 1);
    } else if carry > 0 {
        let _ = add_limb(out, offset + 2, carry);
    }
}

fn add_limb(out: &mut [u64; 4], index: usize, value: u64) -> u64 {
    if value == 0 || index >= out.len() {
        return 0;
    }
    let (sum, carry) = out[index].overflowing_add(value);
    out[index] = sum;
    if carry { 1 } else { 0 }
}

fn bit_is_set(value: &[u64; 4], bit: usize) -> bool {
    let limb = bit / 64;
    let offset = bit % 64;
    (value[limb] & (1u64 << offset)) != 0
}

fn set_bit(value: &mut [u64; 4], bit: usize) {
    let limb = bit / 64;
    let offset = bit % 64;
    value[limb] |= 1u64 << offset;
}

fn shl1_u256(value: &mut [u64; 4]) {
    let mut carry = 0u64;
    for limb in value.iter_mut() {
        let next_carry = *limb >> 63;
        *limb = (*limb << 1) | carry;
        carry = next_carry;
    }
}

fn cmp_u256(left: &[u64; 4], right: &[u64; 4]) -> std::cmp::Ordering {
    for index in (0..4).rev() {
        match left[index].cmp(&right[index]) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

fn sub_assign_u256(left: &mut [u64; 4], right: &[u64; 4]) {
    let mut borrow = 0u64;
    for index in 0..4 {
        let (diff, borrowed_a) = left[index].overflowing_sub(right[index]);
        let (diff, borrowed_b) = diff.overflowing_sub(borrow);
        left[index] = diff;
        borrow = if borrowed_a || borrowed_b { 1 } else { 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defi_mul_div_u128_handles_product_overflow() {
        let value = mul_div_u128(1u128 << 96, 1u128 << 64, 1u128 << 80).unwrap();

        assert_eq!(value, 1u128 << 80);
    }

    #[test]
    fn defi_mul_div_u128_floors_exactly() {
        assert_eq!(
            mul_div_u128(1_000_000, 50_000_000, 32_000_000),
            Some(1_562_500)
        );
        assert_eq!(
            mul_div_u128(1_562_500, UNISWAP_V2_AMOUNT_MIN_NUMERATOR, 1_000),
            Some(1_554_687)
        );
    }

    #[test]
    fn defi_mul_div_u128_rejects_zero_denominator_and_wide_quotient() {
        assert_eq!(mul_div_u128(1, 1, 0), None);
        assert_eq!(mul_div_u128(u128::MAX, u128::MAX, 1), None);
    }

    #[test]
    fn defi_strict_words_and_address_parse_fail_closed() {
        let token = address_from_abi_word(
            "000000000000000000000000dead70c0000000000000000000000000000000aa",
        )
        .unwrap();
        assert_eq!(token, "0xdead70c0000000000000000000000000000000aa");
        assert!(
            address_from_abi_word(
                "100000000000000000000000dead70c0000000000000000000000000000000aa",
            )
            .is_none()
        );
        assert!(strict_words_hex("0x0", 1).is_none());
    }
}
