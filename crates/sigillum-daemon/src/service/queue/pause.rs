//! Operator kill switch for queue execution.

use sigillum_api::{QueueExecutionPauseResponse, TreasuryPolicy};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::{now_unix, session_fingerprint_hex};
use crate::service::{ServiceError, ServiceResult, SigillumService};

const DEFAULT_HOT_BALANCE_WEI_HEX: &str = "0xde0b6b3a7640000";

impl SigillumService {
    pub(crate) async fn set_queue_execution_paused(
        &self,
        token: Option<&str>,
        paused: bool,
    ) -> ServiceResult<QueueExecutionPauseResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        let now = now_unix();
        let mut policy = state
            .treasury_policy
            .take()
            .unwrap_or_else(|| inert_treasury_policy(now));
        let old = policy.execution_paused;
        policy.execution_paused = paused;
        policy.updated_at_unix = now;
        state.treasury_policy = Some(policy);
        crate::inventory::save_wallet_inventory(&self.state.base_dir, &state).map_err(|error| {
            ServiceError::internal(format!("Failed to save wallet inventory: {error}"))
        })?;

        if old != paused {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryExecutionGateUpdate {
                    gate: "execution_paused".into(),
                    old_value: old,
                    new_value: paused,
                    session_fingerprint_hex: session_fingerprint_hex(token),
                },
            )?;
        }

        Ok(QueueExecutionPauseResponse {
            status: if paused { "paused" } else { "resumed" }.into(),
            execution_paused: paused,
        })
    }
}

fn inert_treasury_policy(now: u64) -> TreasuryPolicy {
    // A disabled policy enforces no treasury constraints, but the kill switch it
    // carries is honored unconditionally by queue drain.
    TreasuryPolicy {
        enabled: false,
        allowed_destinations: Vec::new(),
        max_step_native_wei_hex: None,
        max_plan_native_wei_hex: None,
        require_simulation: true,
        allow_raw_digest_signing: false,
        block_cross_party_linkage: false,
        allow_claim_execution: false,
        allow_gas_topups: false,
        max_gas_topup_wei_hex: None,
        allow_plan_execution: false,
        allow_sweep_execution: false,
        allow_revoke_execution: false,
        allow_exit_execution: false,
        execution_paused: false,
        max_fee_per_gas_cap_hex: None,
        simulation_freshness_secs: 900,
        hot_floor_wei_hex: DEFAULT_HOT_BALANCE_WEI_HEX.into(),
        hot_target_wei_hex: DEFAULT_HOT_BALANCE_WEI_HEX.into(),
        created_at_unix: now,
        updated_at_unix: now,
    }
}
