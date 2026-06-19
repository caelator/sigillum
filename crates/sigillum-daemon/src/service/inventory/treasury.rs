//! Treasury console aggregation over inventory, routing, risk, and planning,
//! plus purpose-labeled receive-address allocation.

use std::collections::{BTreeMap, BTreeSet};

use sigillum_api::{
    TreasuryAllowedDestination, TreasuryChainSummary, TreasuryGroupSummary,
    TreasuryOverviewResponse, TreasuryPlanSummary, TreasuryPolicy, TreasuryPolicyMutationResponse,
    TreasuryPolicyResponse, TreasuryPolicyUpdateRequest, TreasuryReceiveAllocateRequest,
    TreasuryReceiveAllocation, TreasuryReceiveAllocationListResponse,
    TreasuryReceiveAllocationMutationResponse, TreasuryReceiveRotateRequest,
    TreasuryReceiveSummary, TreasuryRiskSummary, TreasuryRoutingStatus, WalletAssetHolding,
    WalletInventoryAddress,
};
use sigillum_core::{decode_quantity_hex, derive_ethereum_address_from_xpub};

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;
use crate::service::evm::normalize_address;
use crate::service::helpers::{map_xpub_error, now_unix, random_id};
use crate::service::transaction_policy::{
    TransactionPolicyCheck, TransactionPolicyKind, transaction_policy_actions,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::risk::derive_inventory_risk_findings;
use super::support::{
    load_inventory_state, save_inventory_state, trimmed_optional, trimmed_required,
};
use super::wallet_selection::select_discovery_wallets;
use super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_WATCH, WALLET_FAMILY_ETH_XPUB};

const DEFAULT_NATIVE_SYMBOL: &str = "ETH";
const RECEIVE_STATUS_ACTIVE: &str = "active";
const RECEIVE_STATUS_RETIRED: &str = "retired";
/// Absurdly large but bounded: receive indices beyond this point indicate a
/// runaway caller, not a treasury that genuinely needs a million addresses.
const MAX_RECEIVE_INDEX: u32 = 1_000_000;

/// Saturating big-endian addition of two 256-bit quantities.
pub(super) fn add_u256(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for index in (0..32).rev() {
        let sum = left[index] as u16 + right[index] as u16 + carry;
        out[index] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    if carry > 0 {
        // Saturate rather than wrap: an overflowing treasury total is already
        // far outside plausible balances, and wrapping would understate value.
        return [0xff; 32];
    }
    out
}

pub(super) fn encode_quantity_hex(value: &[u8; 32]) -> String {
    let first_nonzero = value.iter().position(|byte| *byte != 0);
    match first_nonzero {
        None => "0x0".to_string(),
        Some(start) => {
            let mut encoded = String::with_capacity(2 + (32 - start) * 2);
            encoded.push_str("0x");
            let mut rendered = false;
            for byte in &value[start..] {
                if rendered {
                    encoded.push_str(&format!("{byte:02x}"));
                } else {
                    encoded.push_str(&format!("{byte:x}"));
                    rendered = true;
                }
            }
            encoded
        }
    }
}

fn decoded_balance(hex: &str) -> [u8; 32] {
    decode_quantity_hex(hex).unwrap_or([0u8; 32])
}

fn balance_is_nonzero(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

fn has_classification(address: &WalletInventoryAddress, classification: &str) -> bool {
    address
        .classifications
        .iter()
        .any(|entry| entry == classification)
}

#[derive(Default)]
struct ChainAccumulator {
    address_count: usize,
    funded_address_count: usize,
    native_total: [u8; 32],
}

#[derive(Default)]
struct GroupAccumulator {
    address_count: usize,
    funded_address_count: usize,
    native_total: [u8; 32],
    signer_address_count: usize,
    watch_only_address_count: usize,
    erc20_holding_count: usize,
    nft_holding_count: usize,
    defi_holding_count: usize,
    claimable_holding_count: usize,
    approval_exposure_count: usize,
    dormant_candidate_count: usize,
}

fn classify_holding(group: &mut GroupAccumulator, holding: &WalletAssetHolding) {
    match holding.asset_kind.as_str() {
        "erc20" => group.erc20_holding_count += 1,
        "erc721" | "erc1155" | "nft" => group.nft_holding_count += 1,
        "defi" => group.defi_holding_count += 1,
        "airdrop" | "reward" => group.claimable_holding_count += 1,
        "approval" => group.approval_exposure_count += 1,
        _ => {}
    }
}

impl SigillumService {
    pub(crate) fn treasury_overview(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryOverviewResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            crate::service::ServiceError::internal(format!("Failed to load profiles: {error}"))
        })?;

        let mut chains: BTreeMap<u64, ChainAccumulator> = BTreeMap::new();
        let mut groups: BTreeMap<(String, String, u64), GroupAccumulator> = BTreeMap::new();
        let mut tracked_address_count = 0usize;
        let mut funded_address_count = 0usize;
        let mut watch_only_address_count = 0usize;
        let mut signer_address_count = 0usize;

        for address in &state.addresses {
            let balance = decoded_balance(&address.native_balance_wei_hex);
            let funded = balance_is_nonzero(&balance);
            tracked_address_count += 1;
            if funded {
                funded_address_count += 1;
            }
            let watch_only = matches!(
                address.wallet_family.as_str(),
                WALLET_FAMILY_ETH_XPUB | WALLET_FAMILY_ETH_WATCH
            ) || has_classification(address, "watch_only");
            if watch_only {
                watch_only_address_count += 1;
            }
            if address.wallet_family == WALLET_FAMILY_ETH_SEED
                || has_classification(address, "signer_available")
            {
                signer_address_count += 1;
            }

            let chain = chains.entry(address.chain_id).or_default();
            chain.address_count += 1;
            if funded {
                chain.funded_address_count += 1;
            }
            chain.native_total = add_u256(&chain.native_total, &balance);

            let group = groups
                .entry((
                    address.wallet_family.clone(),
                    address.wallet_profile.clone(),
                    address.chain_id,
                ))
                .or_default();
            group.address_count += 1;
            if funded {
                group.funded_address_count += 1;
            }
            group.native_total = add_u256(&group.native_total, &balance);
            if watch_only {
                group.watch_only_address_count += 1;
            } else if address.wallet_family == WALLET_FAMILY_ETH_SEED
                || has_classification(address, "signer_available")
            {
                group.signer_address_count += 1;
            }
            if has_classification(address, "dormant_candidate") {
                group.dormant_candidate_count += 1;
            }
        }

        for holding in &state.holdings {
            let group = groups
                .entry((
                    holding.wallet_family.clone(),
                    holding.wallet_profile.clone(),
                    holding.chain_id,
                ))
                .or_default();
            classify_holding(group, holding);
        }

        let native_symbol_for_chain = |chain_id: u64| -> String {
            state
                .chain_profiles
                .iter()
                .find(|profile| profile.chain_id == Some(chain_id))
                .map(|profile| profile.native_symbol.clone())
                .unwrap_or_else(|| DEFAULT_NATIVE_SYMBOL.to_string())
        };

        let chains = chains
            .into_iter()
            .map(|(chain_id, accumulator)| TreasuryChainSummary {
                chain_id,
                native_symbol: native_symbol_for_chain(chain_id),
                address_count: accumulator.address_count,
                funded_address_count: accumulator.funded_address_count,
                native_total_wei_hex: encode_quantity_hex(&accumulator.native_total),
            })
            .collect();

        let groups = groups
            .into_iter()
            .map(
                |((wallet_family, wallet_profile, chain_id), accumulator)| TreasuryGroupSummary {
                    wallet_family,
                    wallet_profile,
                    chain_id,
                    address_count: accumulator.address_count,
                    funded_address_count: accumulator.funded_address_count,
                    native_total_wei_hex: encode_quantity_hex(&accumulator.native_total),
                    signer_address_count: accumulator.signer_address_count,
                    watch_only_address_count: accumulator.watch_only_address_count,
                    erc20_holding_count: accumulator.erc20_holding_count,
                    nft_holding_count: accumulator.nft_holding_count,
                    defi_holding_count: accumulator.defi_holding_count,
                    claimable_holding_count: accumulator.claimable_holding_count,
                    approval_exposure_count: accumulator.approval_exposure_count,
                    dormant_candidate_count: accumulator.dormant_candidate_count,
                },
            )
            .collect();

        let balance_for_address = |target: &str| -> Option<String> {
            let target = target.to_ascii_lowercase();
            let mut total = [0u8; 32];
            let mut seen = false;
            for address in &state.addresses {
                if address.address.to_ascii_lowercase() == target {
                    total = add_u256(&total, &decoded_balance(&address.native_balance_wei_hex));
                    seen = true;
                }
            }
            seen.then(|| encode_quantity_hex(&total))
        };

        let routing = registry
            .eth_seed_wallets
            .iter()
            .map(|profile| TreasuryRoutingStatus {
                wallet_profile: profile.name.clone(),
                hot_address: profile.hot_address.clone(),
                treasury_address: profile.treasury_address.clone(),
                default_destination_address: profile.default_destination_address.clone(),
                hot_native_balance_wei_hex: profile
                    .hot_address
                    .as_deref()
                    .and_then(balance_for_address),
                treasury_native_balance_wei_hex: profile
                    .treasury_address
                    .as_deref()
                    .and_then(balance_for_address),
                routing_ready: profile.treasury_address.is_some()
                    || profile.default_destination_address.is_some(),
            })
            .collect();

        let mut risk = TreasuryRiskSummary::default();
        let mut findings = state.risk_findings.clone();
        findings.extend(derive_inventory_risk_findings(
            &state.addresses,
            &state.holdings,
            &state.risk_catalog,
        ));
        for finding in &findings {
            risk.total_findings += 1;
            match finding.risk_level.as_str() {
                "critical" => risk.critical_findings += 1,
                "high" => risk.high_findings += 1,
                "medium" => risk.medium_findings += 1,
                _ => risk.low_findings += 1,
            }
        }

        let latest_plan = state
            .consolidation_plans
            .iter()
            .max_by_key(|plan| plan.created_at_unix);
        let plans = TreasuryPlanSummary {
            total_plans: state.consolidation_plans.len(),
            latest_plan_id: latest_plan.map(|plan| plan.id.clone()),
            latest_plan_status: latest_plan.map(|plan| plan.status.clone()),
            latest_review_required_steps: latest_plan
                .map(|plan| plan.summary.review_required_steps)
                .unwrap_or(0),
            latest_approved_steps: latest_plan
                .map(|plan| plan.summary.approved_steps)
                .unwrap_or(0),
            latest_executable_steps: latest_plan
                .map(|plan| plan.summary.executable_steps)
                .unwrap_or(0),
            latest_blocked_steps: latest_plan
                .map(|plan| plan.summary.blocked_steps)
                .unwrap_or(0),
            latest_policy_violations: latest_plan
                .map(|plan| plan.policy_violations.clone())
                .unwrap_or_default(),
        };

        let receive = receive_summary(&state.receive_allocations);

        Ok(TreasuryOverviewResponse {
            generated_at_unix: now_unix(),
            tracked_address_count,
            funded_address_count,
            watch_only_address_count,
            signer_address_count,
            chains,
            groups,
            routing,
            risk,
            plans,
            receive,
        })
    }

    pub(crate) fn get_treasury_policy(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryPolicyResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(TreasuryPolicyResponse {
            policy: state.treasury_policy,
        })
    }

    pub(crate) async fn update_treasury_policy(
        &self,
        token: Option<&str>,
        body: TreasuryPolicyUpdateRequest,
    ) -> ServiceResult<TreasuryPolicyMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let now = now_unix();

        let mut allowed_destinations: Vec<TreasuryAllowedDestination> = Vec::new();
        for destination in body.allowed_destinations {
            let address = normalize_address(&destination.address)?;
            // Dedupe case-insensitively and keep the first label, so repeated
            // operator input cannot silently relabel an approved destination.
            if allowed_destinations
                .iter()
                .any(|existing| existing.address.eq_ignore_ascii_case(&address))
            {
                continue;
            }
            allowed_destinations.push(TreasuryAllowedDestination {
                address,
                label: destination.label.and_then(trimmed_optional),
            });
        }

        let policy = TreasuryPolicy {
            enabled: body.enabled,
            allowed_destinations,
            max_step_native_wei_hex: validated_cap_hex(
                "max_step_native_wei_hex",
                body.max_step_native_wei_hex,
            )?,
            max_plan_native_wei_hex: validated_cap_hex(
                "max_plan_native_wei_hex",
                body.max_plan_native_wei_hex,
            )?,
            require_simulation: body.require_simulation.unwrap_or(true),
            allow_raw_digest_signing: body.allow_raw_digest_signing.unwrap_or(false),
            created_at_unix: state
                .treasury_policy
                .as_ref()
                .map(|existing| existing.created_at_unix)
                .unwrap_or(now),
            updated_at_unix: now,
        };
        state.treasury_policy = Some(policy.clone());
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryPolicyUpdate {
                enabled: policy.enabled,
                destinations: policy.allowed_destinations.len(),
            },
        )?;

        Ok(TreasuryPolicyMutationResponse {
            status: "updated".into(),
            policy,
        })
    }

    /// All receive allocations, active and retired, in allocation order.
    pub(crate) fn list_treasury_receive_allocations(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryReceiveAllocationListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(TreasuryReceiveAllocationListResponse {
            allocations: state.receive_allocations,
        })
    }

    /// Allocate a fresh purpose-labeled receive address for a wallet profile.
    ///
    /// Privacy: derivation is pure local xpub math — no provider or network
    /// I/O — and every allocation takes the next unused receive index, so a
    /// counterparty only ever sees an address that has never been handed out
    /// for another purpose. Fresh-per-purpose addresses keep unrelated
    /// payments unlinkable on-chain.
    pub(crate) async fn allocate_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceiveAllocateRequest,
    ) -> ServiceResult<TreasuryReceiveAllocationMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let wallet_profile = trimmed_required("wallet_profile", &body.wallet_profile)?;
        let purpose = trimmed_required("purpose", &body.purpose)?;
        let label = body.label.and_then(trimmed_optional);

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let allocation =
            self.issue_receive_allocation(&mut state, &wallet_profile, purpose, label)?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveAllocate {
                wallet_profile: allocation.wallet_profile.clone(),
                purpose: allocation.purpose.clone(),
            },
        )?;

        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "allocated".into(),
            allocation,
        })
    }

    /// Retire an active allocation and issue the next index for the same
    /// wallet profile, carrying over its purpose and label.
    ///
    /// Both mutations land in a single state save: either the old allocation
    /// is retired AND its replacement exists, or neither change persists.
    /// Like allocation, the replacement address is derived locally from the
    /// profile xpub with no network I/O.
    pub(crate) async fn rotate_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceiveRotateRequest,
    ) -> ServiceResult<TreasuryReceiveAllocationMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let allocation_id = body.allocation_id.trim().to_string();

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(existing) = state
            .receive_allocations
            .iter_mut()
            .find(|allocation| allocation.id == allocation_id)
        else {
            return Err(ServiceError::not_found("Receive allocation not found."));
        };
        if existing.status != RECEIVE_STATUS_ACTIVE {
            return Err(ServiceError::bad_request(
                "Receive allocation is not active.",
            ));
        }
        existing.status = RECEIVE_STATUS_RETIRED.into();
        existing.retired_at_unix = Some(now_unix());
        let wallet_profile = existing.wallet_profile.clone();
        let purpose = existing.purpose.clone();
        let label = existing.label.clone();

        let allocation =
            self.issue_receive_allocation(&mut state, &wallet_profile, purpose, label)?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveRotate { id: allocation_id },
        )?;

        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "rotated".into(),
            allocation,
        })
    }

    /// Derive the next receive allocation for `wallet_profile` and append it
    /// to `state` (callers persist the state afterwards).
    ///
    /// Profile resolution prefers seed profiles over xpub profiles with the
    /// same name, matching discovery-wallet selection. The next index is one
    /// past the highest index either previously allocated or already observed
    /// in scanned inventory, so fresh allocations never collide with
    /// addresses the treasury has used before.
    fn issue_receive_allocation(
        &self,
        state: &mut WalletInventoryState,
        wallet_profile: &str,
        purpose: String,
        label: Option<String>,
    ) -> ServiceResult<TreasuryReceiveAllocation> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let wallets = select_discovery_wallets(
            self,
            &registry.eth_seed_wallets,
            &registry.eth_xpub_wallets,
            None,
            Some(wallet_profile),
        )?;
        let Some(wallet) = wallets.into_iter().next() else {
            return Err(ServiceError::not_found("Wallet profile not found."));
        };

        let next_index = next_receive_index(
            &state.receive_allocations,
            &state.addresses,
            &wallet.family,
            &wallet.profile,
        );
        if next_index > MAX_RECEIVE_INDEX {
            return Err(ServiceError::bad_request("Receive index space exhausted."));
        }

        let derived = derive_ethereum_address_from_xpub(&wallet.receive_xpub, next_index)
            .map_err(map_xpub_error)?;
        let allocation = TreasuryReceiveAllocation {
            id: random_id(),
            wallet_family: wallet.family.clone(),
            wallet_profile: wallet.profile.clone(),
            address: derived.address,
            derivation_path: format!("{}/{}", wallet.receive_path, next_index),
            address_index: next_index,
            purpose,
            label,
            status: RECEIVE_STATUS_ACTIVE.into(),
            created_at_unix: now_unix(),
            retired_at_unix: None,
        };
        state.receive_allocations.push(allocation.clone());
        Ok(allocation)
    }
}

/// Console rollup of allocation counts and distinct active purposes.
fn receive_summary(allocations: &[TreasuryReceiveAllocation]) -> TreasuryReceiveSummary {
    let mut summary = TreasuryReceiveSummary::default();
    let mut purposes = BTreeSet::new();
    for allocation in allocations {
        if allocation.status == RECEIVE_STATUS_ACTIVE {
            summary.active_allocations += 1;
            purposes.insert(allocation.purpose.as_str());
        } else {
            summary.retired_allocations += 1;
        }
    }
    summary.purposes = purposes.len();
    summary
}

/// Next unused receive index for one wallet profile.
///
/// Considers both prior allocations (including retired ones, which must never
/// be re-issued) and inventory rows discovered by scans, so the allocator
/// stays ahead of any index the wallet has already exposed on-chain. Returns
/// 0 only when neither source knows the profile.
fn next_receive_index(
    allocations: &[TreasuryReceiveAllocation],
    addresses: &[WalletInventoryAddress],
    wallet_family: &str,
    wallet_profile: &str,
) -> u32 {
    let allocated_max = allocations
        .iter()
        .filter(|allocation| allocation.wallet_profile == wallet_profile)
        .map(|allocation| allocation.address_index)
        .max();
    let observed_max = addresses
        .iter()
        .filter(|address| {
            address.wallet_family == wallet_family && address.wallet_profile == wallet_profile
        })
        .map(|address| address.address_index)
        .max();
    match allocated_max.into_iter().chain(observed_max).max() {
        Some(max_index) => max_index.saturating_add(1),
        None => 0,
    }
}

/// Reject caps that would not decode during enforcement: a cap that silently
/// fails to parse later would be a guardrail that never fires.
fn validated_cap_hex(field: &str, value: Option<String>) -> ServiceResult<Option<String>> {
    let Some(value) = value.and_then(trimmed_optional) else {
        return Ok(None);
    };
    decode_quantity_hex(&value).map_err(|_| {
        ServiceError::bad_request(format!("{field} must be a hex uint256 quantity"))
    })?;
    Ok(Some(value))
}

/// Treasury policy blockers for a single consolidation plan step.
///
/// Returned markers extend the step's planner blockers; policy violations
/// block a step rather than rewriting it, so the operator always sees which
/// guardrail fired. Only sweep actions are destination-routed, and only
/// native amounts are comparable in wei, so other actions pass untouched.
/// A sweep step with no destination is already a planner blocker
/// (`missing_destination`) and is not duplicated here.
pub(super) fn policy_blockers_for_step(
    policy: &TreasuryPolicy,
    action: &str,
    destination_address: Option<&str>,
    asset_kind: &str,
    amount_hex: &str,
) -> Vec<String> {
    if !action.starts_with("sweep") && action != "raw_digest" {
        return Vec::new();
    }
    let kind = if action == "raw_digest" {
        TransactionPolicyKind::RawDigest
    } else {
        TransactionPolicyKind::RoutedTransfer
    };
    transaction_policy_actions(
        policy,
        TransactionPolicyCheck {
            kind,
            destination_address,
            asset_kind,
            amount_hex,
        },
    )
    .into_iter()
    .map(|action| action.as_str().to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_u256_carries_and_saturates() {
        let one = decode_quantity_hex("0x1").unwrap();
        let max_byte = decode_quantity_hex("0xff").unwrap();
        let sum = add_u256(&one, &max_byte);
        assert_eq!(encode_quantity_hex(&sum), "0x100");

        let max = [0xffu8; 32];
        let saturated = add_u256(&max, &one);
        assert_eq!(saturated, [0xffu8; 32]);
    }

    #[test]
    fn encode_quantity_hex_trims_leading_zeroes() {
        assert_eq!(encode_quantity_hex(&[0u8; 32]), "0x0");
        let value = decode_quantity_hex("0xde0b6b3a7640000").unwrap();
        assert_eq!(encode_quantity_hex(&value), "0xde0b6b3a7640000");
    }

    fn sample_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![TreasuryAllowedDestination {
                address: "0x9999999999999999999999999999999999999999".into(),
                label: Some("cold-treasury".into()),
            }],
            max_step_native_wei_hex: Some("0xde0b6b3a7640000".into()),
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    #[test]
    fn policy_allows_allowlisted_sweep_within_cap() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some("0x9999999999999999999999999999999999999999"),
            "native",
            "0xde0b6b3a7640000",
        );
        assert!(blockers.is_empty(), "unexpected blockers: {blockers:?}");
    }

    #[test]
    fn policy_allowlist_match_is_case_insensitive() {
        let destination = "0x9999999999999999999999999999999999999999".to_ascii_uppercase();
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some(destination.as_str()),
            "native",
            "0x1",
        );
        assert!(blockers.is_empty(), "unexpected blockers: {blockers:?}");
    }

    #[test]
    fn policy_blocks_non_allowlisted_sweep_destination() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_erc20",
            Some("0x8888888888888888888888888888888888888888"),
            "erc20",
            "0x1",
        );
        assert_eq!(blockers, vec!["block_destination".to_string()]);
    }

    #[test]
    fn policy_blocks_native_amount_above_step_cap() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some("0x9999999999999999999999999999999999999999"),
            "native",
            "0xde0b6b3a7640001",
        );
        assert_eq!(blockers, vec!["block_step_cap".to_string()]);
    }

    #[test]
    fn policy_reports_destination_and_cap_violations_together() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some("0x8888888888888888888888888888888888888888"),
            "native",
            "0x1bc16d674ec80000",
        );
        assert_eq!(
            blockers,
            vec![
                "block_destination".to_string(),
                "block_step_cap".to_string(),
            ]
        );
    }

    #[test]
    fn disabled_policy_blocks_nothing() {
        let mut policy = sample_policy();
        policy.enabled = false;
        let blockers = policy_blockers_for_step(
            &policy,
            "sweep_native",
            Some("0x8888888888888888888888888888888888888888"),
            "native",
            "0xffffffffffffffffffffffff",
        );
        assert!(blockers.is_empty());
    }

    #[test]
    fn policy_ignores_non_sweep_actions_and_missing_destinations() {
        // Revokes and claims are not destination-routed value moves.
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "revoke_erc20_approval",
            Some("0x8888888888888888888888888888888888888888"),
            "approval",
            "0x1",
        );
        assert!(blockers.is_empty());

        // A sweep with no destination is already blocked by the planner.
        let blockers =
            policy_blockers_for_step(&sample_policy(), "sweep_erc20", None, "erc20", "0x1");
        assert!(blockers.is_empty());
    }

    #[test]
    fn empty_allowlist_blocks_routed_sweeps_when_enabled() {
        let mut policy = sample_policy();
        policy.allowed_destinations.clear();
        let blockers = policy_blockers_for_step(
            &policy,
            "sweep_native",
            Some("0x9999999999999999999999999999999999999999"),
            "native",
            "0x1",
        );
        assert_eq!(blockers, vec!["block_destination".to_string()]);
    }

    fn sample_allocation(
        wallet_profile: &str,
        address_index: u32,
        status: &str,
        purpose: &str,
    ) -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: format!("alloc_{wallet_profile}_{address_index}"),
            wallet_family: "eth-seed".into(),
            wallet_profile: wallet_profile.into(),
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: format!("m/44'/60'/0'/0/{address_index}"),
            address_index,
            purpose: purpose.into(),
            label: None,
            status: status.into(),
            created_at_unix: 1,
            retired_at_unix: None,
        }
    }

    fn sample_inventory_address(
        wallet_family: &str,
        wallet_profile: &str,
        address_index: u32,
    ) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: format!("addr_{wallet_profile}_{address_index}"),
            wallet_family: wallet_family.into(),
            wallet_profile: wallet_profile.into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x2222222222222222222222222222222222222222".into(),
            derivation_path: format!("m/44'/60'/0'/0/{address_index}"),
            address_index,
            activity_state: "funded".into(),
            native_balance_wei_hex: "0x1".into(),
            transaction_count: 0,
            classifications: Vec::new(),
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    #[test]
    fn next_receive_index_starts_at_zero_when_nothing_is_known() {
        assert_eq!(next_receive_index(&[], &[], "eth-seed", "seed-main"), 0);
    }

    #[test]
    fn next_receive_index_advances_past_allocations_only() {
        let allocations = vec![
            sample_allocation("seed-main", 0, "retired", "acme"),
            sample_allocation("seed-main", 4, "active", "beta"),
        ];
        assert_eq!(
            next_receive_index(&allocations, &[], "eth-seed", "seed-main"),
            5
        );
    }

    #[test]
    fn next_receive_index_advances_past_inventory_only() {
        let addresses = vec![
            sample_inventory_address("eth-seed", "seed-main", 2),
            sample_inventory_address("eth-seed", "seed-main", 7),
        ];
        assert_eq!(
            next_receive_index(&[], &addresses, "eth-seed", "seed-main"),
            8
        );
    }

    #[test]
    fn next_receive_index_takes_the_max_of_both_sources() {
        let allocations = vec![sample_allocation("seed-main", 9, "active", "acme")];
        let addresses = vec![sample_inventory_address("eth-seed", "seed-main", 3)];
        assert_eq!(
            next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
            10
        );

        let allocations = vec![sample_allocation("seed-main", 1, "active", "acme")];
        let addresses = vec![sample_inventory_address("eth-seed", "seed-main", 6)];
        assert_eq!(
            next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
            7
        );
    }

    #[test]
    fn next_receive_index_ignores_other_profiles_and_families() {
        let allocations = vec![sample_allocation("seed-other", 40, "active", "acme")];
        let addresses = vec![
            sample_inventory_address("eth-seed", "seed-other", 50),
            // Same profile name under a different family does not count for
            // the inventory source.
            sample_inventory_address("eth-xpub", "seed-main", 60),
        ];
        assert_eq!(
            next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
            0
        );
    }

    #[test]
    fn receive_summary_counts_active_retired_and_distinct_purposes() {
        let allocations = vec![
            sample_allocation("seed-main", 0, "retired", "acme"),
            sample_allocation("seed-main", 1, "active", "acme"),
            sample_allocation("seed-main", 2, "active", "acme"),
            sample_allocation("seed-main", 3, "active", "beta"),
        ];
        let summary = receive_summary(&allocations);
        assert_eq!(summary.active_allocations, 3);
        assert_eq!(summary.retired_allocations, 1);
        // Retired purposes do not count; duplicates collapse.
        assert_eq!(summary.purposes, 2);

        assert_eq!(receive_summary(&[]), TreasuryReceiveSummary::default());
    }
}
