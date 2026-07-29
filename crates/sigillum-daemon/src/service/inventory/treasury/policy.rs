use sigillum_api::{
    TreasuryAllowedDestination, TreasuryPolicy, TreasuryPolicyMutationResponse,
    TreasuryPolicyResponse, TreasuryPolicyUpdateRequest,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::service::evm::normalize_address;
use crate::service::helpers::{compare_u256, now_unix, session_fingerprint_hex};
use crate::service::transaction_policy::{
    TransactionPolicyCheck, TransactionPolicyKind, transaction_policy_actions,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::support::{load_inventory_state, save_inventory_state, trimmed_optional};

const DEFAULT_HOT_FLOOR_WEI_HEX: &str = "0xde0b6b3a7640000";
const DEFAULT_HOT_TARGET_WEI_HEX: &str = "0xde0b6b3a7640000";

impl SigillumService {
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
        if body.simulation_freshness_secs == Some(0) {
            return Err(ServiceError::bad_request(
                "simulation_freshness_secs must be greater than 0",
            ));
        }
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let previous_policy = state.treasury_policy.clone();
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

        let hot_floor_wei_hex = validated_required_quantity_hex(
            "hot_floor_wei_hex",
            body.hot_floor_wei_hex,
            DEFAULT_HOT_FLOOR_WEI_HEX,
        )?;
        let hot_target_wei_hex = validated_required_quantity_hex(
            "hot_target_wei_hex",
            body.hot_target_wei_hex,
            DEFAULT_HOT_TARGET_WEI_HEX,
        )?;
        let hot_floor = decode_quantity_hex(&hot_floor_wei_hex).map_err(|_| {
            ServiceError::bad_request("hot_floor_wei_hex must be a hex uint256 quantity")
        })?;
        let hot_target = decode_quantity_hex(&hot_target_wei_hex).map_err(|_| {
            ServiceError::bad_request("hot_target_wei_hex must be a hex uint256 quantity")
        })?;
        if compare_u256(&hot_floor, &hot_target).is_gt() {
            return Err(ServiceError::bad_request(
                "hot_floor_wei_hex must be less than or equal to hot_target_wei_hex",
            ));
        }
        let hot_overflow_wei_hex =
            validated_cap_hex("hot_overflow_wei_hex", body.hot_overflow_wei_hex)?;
        if let Some(hot_overflow_wei_hex) = hot_overflow_wei_hex.as_ref() {
            let hot_overflow = decode_quantity_hex(hot_overflow_wei_hex).map_err(|_| {
                ServiceError::bad_request("hot_overflow_wei_hex must be a hex uint256 quantity")
            })?;
            if compare_u256(&hot_target, &hot_overflow).is_gt() {
                return Err(ServiceError::bad_request(
                    "hot_target_wei_hex must be less than or equal to hot_overflow_wei_hex",
                ));
            }
        }
        let previous_execution_paused = previous_policy
            .as_ref()
            .map(|policy| policy.execution_paused)
            .unwrap_or(false);
        // Policy edits that omit the kill switch must not silently resume execution.
        let execution_paused = body.execution_paused.unwrap_or(previous_execution_paused);

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
            block_cross_party_linkage: body.block_cross_party_linkage.unwrap_or(false),
            allow_claim_execution: body.allow_claim_execution.unwrap_or(false),
            allow_gas_topups: body.allow_gas_topups.unwrap_or(false),
            max_gas_topup_wei_hex: validated_cap_hex(
                "max_gas_topup_wei_hex",
                body.max_gas_topup_wei_hex,
            )?,
            allow_plan_execution: body.allow_plan_execution.unwrap_or(false),
            allow_sweep_execution: body.allow_sweep_execution.unwrap_or(false),
            allow_revoke_execution: body.allow_revoke_execution.unwrap_or(false),
            allow_exit_execution: body.allow_exit_execution.unwrap_or(false),
            execution_paused,
            max_fee_per_gas_cap_hex: validated_cap_hex(
                "max_fee_per_gas_cap_hex",
                body.max_fee_per_gas_cap_hex,
            )?,
            simulation_freshness_secs: body.simulation_freshness_secs.unwrap_or(900),
            hot_floor_wei_hex,
            hot_target_wei_hex,
            hot_overflow_wei_hex,
            allow_treasury_automation: body.allow_treasury_automation.unwrap_or(false),
            created_at_unix: previous_policy
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
        let fingerprint_hex = session_fingerprint_hex(token);
        for (gate, old_value, new_value) in [
            (
                "allow_plan_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_plan_execution)
                    .unwrap_or(false),
                policy.allow_plan_execution,
            ),
            (
                "allow_sweep_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_sweep_execution)
                    .unwrap_or(false),
                policy.allow_sweep_execution,
            ),
            (
                "allow_revoke_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_revoke_execution)
                    .unwrap_or(false),
                policy.allow_revoke_execution,
            ),
            (
                "allow_exit_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_exit_execution)
                    .unwrap_or(false),
                policy.allow_exit_execution,
            ),
            (
                "allow_claim_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_claim_execution)
                    .unwrap_or(false),
                policy.allow_claim_execution,
            ),
            (
                "allow_gas_topups",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_gas_topups)
                    .unwrap_or(false),
                policy.allow_gas_topups,
            ),
            (
                "allow_treasury_automation",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_treasury_automation)
                    .unwrap_or(false),
                policy.allow_treasury_automation,
            ),
            (
                "execution_paused",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.execution_paused)
                    .unwrap_or(false),
                policy.execution_paused,
            ),
        ] {
            if old_value != new_value {
                self.record_audit(
                    self.state.active_compartment_id_for(token),
                    AuditEventSpec::TreasuryExecutionGateUpdate {
                        gate: gate.into(),
                        old_value,
                        new_value,
                        session_fingerprint_hex: fingerprint_hex.clone(),
                    },
                )?;
            }
        }

        Ok(TreasuryPolicyMutationResponse {
            status: "updated".into(),
            policy,
        })
    }
}

/// Reject caps that would not decode during enforcement: a cap that silently
/// fails to parse later would be a guardrail that never fires.
fn validated_cap_hex(field: &str, value: Option<String>) -> ServiceResult<Option<String>> {
    let Some(value) = value.and_then(trimmed_optional) else {
        return Ok(None);
    };
    if !has_hex_quantity_prefix(&value) {
        return Err(ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        )));
    }
    decode_quantity_hex(&value).map_err(|_| {
        ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        ))
    })?;
    Ok(Some(value))
}

fn validated_required_quantity_hex(
    field: &str,
    value: Option<String>,
    default_value: &str,
) -> ServiceResult<String> {
    let value = value
        .and_then(trimmed_optional)
        .unwrap_or_else(|| default_value.to_string());
    if !has_hex_quantity_prefix(&value) {
        return Err(ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        )));
    }
    decode_quantity_hex(&value).map_err(|_| {
        ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        ))
    })?;
    Ok(value)
}

fn has_hex_quantity_prefix(value: &str) -> bool {
    value.starts_with("0x") || value.starts_with("0X")
}

/// Treasury policy blockers for a single consolidation plan step.
///
/// Returned markers extend the step's planner blockers; policy violations
/// block a step rather than rewriting it, so the operator always sees which
/// guardrail fired. Only sweep actions are destination-routed, and only
/// native amounts are comparable in wei, so other actions pass untouched.
/// A sweep step with no destination is already a planner blocker
/// (`missing_destination`) and is not duplicated here.
pub(in crate::service) fn policy_blockers_for_step(
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
mod tests;
