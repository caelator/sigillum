//! `PlanStepExecution` queue job execution (W7.3/W7.4).
//!
//! Mirrors the `sweeps.rs` split: this module owns everything specific to
//! draining a `PlanStepExecution` job, while `service/queue/processing.rs`
//! keeps the generic drain loop. `plan_steps/signing.rs` owns the actual
//! sign + broadcast crypto boundary (including W7.4's nonce-retry/fee-bump
//! loop); `plan_steps/receipts.rs` owns post-broadcast semantics (broadcast
//! error classification, fee bump math, receipt-confirmation polling). This
//! file owns the pre-signing guards (dependency ordering, evidence-hash
//! verification, signer resolution, fee cap) that must all pass BEFORE any
//! key material is touched, PLUS the W7.4 dispatch: a job already in `sent`
//! (broadcast in a PRIOR drain, possibly before a daemon restart — E2) skips
//! straight to receipt polling and NEVER re-signs or re-broadcasts.
//!
//! Execution order per FRESH job (never reordered — each guard fails
//! closed):
//! 1. Dependency ordering (W6.4 / E1): an unmet prerequisite defers this job
//!    (`blocked`, re-tried next drain); a failed prerequisite halts it the
//!    same way, naming the prerequisite.
//! 2. Evidence-hash re-verification (W7.2's `simulation_evidence_hash_hex`)
//!    against the step's CURRENT live state — a mismatch (including a
//!    missing plan/step, which cannot be told apart from tampering) means
//!    `operator_action_required` and the job is NEVER signed.
//! 3. Signer resolution: the wallet family must be `eth-seed`; watch-only or
//!    unknown families are unreachable by enqueue-validation construction
//!    but are re-checked here anyway and blocked via
//!    `TransactionPolicyAction::BlockWatchOnlySigner`. A locked compartment
//!    or missing profile also blocks (never panics).
//! 4. Fee cap: `max_fee_per_gas_cap_hex`, when the policy sets one, is
//!    enforced against the job's recorded fee basis before any signing.
//! 5. Sign + broadcast (`signing.rs`), reusing the payload's prepared call
//!    fields verbatim. On a "nonce too low"/underpriced broadcast rejection,
//!    `signing.rs` retries EXACTLY once (fresh nonce, or one fee bump within
//!    the policy cap) before parking to `operator_action_required`. A
//!    broadcast-time revert rejection parks the same way, generalizing the
//!    W7.3 claim-only rule. Claim (`ClaimReward`) failures at this stage are
//!    NEVER retried at all — the Merkle proof may already be consumed by a
//!    single broadcast attempt — and convert straight to
//!    `operator_action_required`.
//! 6. A job that JUST reached `sent` is NOT polled for a receipt within the
//!    same drain call (so `sent` truthfully means "broadcast, awaiting
//!    confirmation" the instant it is set) — confirmation happens on a
//!    LATER drain/maintenance cycle via the resume path above.

pub(super) mod receipts;
mod signing;

use std::collections::HashMap;

use sigillum_api::{PlanStepExecutionPayload, WalletPlanStepAction};
use sigillum_core::decode_quantity_hex;

use crate::service::helpers::{compare_u256, map_wallet_error, session_fingerprint_hex};
use crate::service::transaction_policy::TransactionPolicyAction;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::gates::plan_action_execution_family;
use super::state::normalize_queue_state;
use super::{
    QUEUE_STATE_CONFIRMED, QUEUE_STATE_FAILED_TERMINAL, QUEUE_STATE_LEGACY_FAILED, QUEUE_STATE_SENT,
};
use super::{QUEUE_STATE_OPERATOR_ACTION_REQUIRED, QueueExecution};

const WALLET_FAMILY_ETH_SEED: &str = "eth-seed";

impl SigillumService {
    /// Entry point called from the drain loop for a `PlanStepExecution` job
    /// that has already passed its W7.1 execution-family gate check.
    ///
    /// `job_states` is a snapshot of every OTHER job's id -> state in this
    /// drain batch, refreshed by the caller after each job so a prerequisite
    /// that reaches `confirmed` while being polled can unblock a later
    /// dependent in that same drain. A freshly broadcast prerequisite remains
    /// `sent`, so its dependents wait for a later confirmation cycle.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::queue) async fn process_plan_step_execution(
        &self,
        token: &str,
        job_id: &str,
        job_state: &str,
        job_transaction_hash_hex: Option<&str>,
        job_broadcast_at_unix: Option<u64>,
        payload: &PlanStepExecutionPayload,
        job_states: &HashMap<String, String>,
    ) -> ServiceResult<QueueExecution> {
        // W7.4: a job already in `sent` broadcast in a PRIOR drain call —
        // possibly before a daemon restart (E2 crash-resumption). NEVER
        // re-sign or re-broadcast; only continue receipt polling using the
        // persisted transaction hash.
        if normalize_queue_state(job_state) == QUEUE_STATE_SENT {
            return self
                .resume_plan_step_confirmation(
                    payload,
                    job_transaction_hash_hex,
                    job_broadcast_at_unix,
                )
                .await;
        }

        // 1. Dependency ordering (E1 semantics) — cheap, no vault access.
        if let Some(reason) = dependency_block_reason(payload, job_states) {
            return Ok(QueueExecution::Blocked(reason));
        }

        // 2. Evidence-hash re-verification BEFORE anything else. A mismatch
        //    (or a plan/step that no longer exists) is treated identically:
        //    fail closed, never sign.
        let inventory_state = crate::inventory::load_wallet_inventory(&self.state.base_dir)
            .map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        if let Err(reason) = crate::service::inventory::verify_plan_step_execution_evidence(
            &inventory_state,
            payload,
        ) {
            return Ok(QueueExecution::OperatorActionRequired(reason));
        }
        let Some(step) = inventory_state
            .consolidation_plans
            .iter()
            .find(|plan| plan.id == payload.plan_id)
            .and_then(|plan| plan.steps.iter().find(|step| step.id == payload.step_id))
        else {
            // Unreachable in practice: evidence verification above already
            // proved the plan/step exists. Fail closed anyway rather than
            // unwrap, per "never panic".
            return Ok(QueueExecution::OperatorActionRequired(format!(
                "evidence_hash_tamper: step {} vanished from plan {} between verification and \
                 execution",
                payload.step_id, payload.plan_id
            )));
        };

        // 3. Signer resolution: family re-check, then profile/vault lookup.
        if payload.wallet_family != WALLET_FAMILY_ETH_SEED {
            return Ok(QueueExecution::Blocked(format!(
                "{}: wallet family '{}' is not a seed-derived signer",
                TransactionPolicyAction::BlockWatchOnlySigner.as_str(),
                payload.wallet_family
            )));
        }
        let (provider, profile) =
            match self.resolve_eth_seed_wallet_profile(&payload.wallet_profile) {
                Ok(pair) => pair,
                Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => {
                    return Ok(QueueExecution::Blocked(format!(
                        "wallet_profile_not_found: {}",
                        error.message()
                    )));
                }
                Err(error) => return Err(error),
            };

        // 4. Fee cap, checked before any crypto work. The resolved cap
        //    bytes (if any) are also threaded into signing so the W7.4
        //    single fee-bump retry stays within the same policy ceiling.
        let fee_cap = self.resolve_fee_cap_bytes()?;
        if let Some(reason) = self.plan_step_fee_cap_block_reason(payload, fee_cap)? {
            return Ok(QueueExecution::Blocked(reason));
        }

        // 5. Derive the signing key. Locked compartment / missing secret /
        //    corrupt secret all fail closed to `blocked` (never panic); the
        //    operator resolves by unlocking or repairing the profile.
        let signing_key = match self.derive_eth_seed_signing_key(&profile, &payload.derivation_path)
        {
            Ok(key) => key,
            Err(error) => {
                return Ok(QueueExecution::Blocked(format!(
                    "signer_unavailable: {}",
                    error.message()
                )));
            }
        };
        if !sigillum_core::ethereum_address_from_signing_key(&signing_key)
            .eq_ignore_ascii_case(&payload.source_address)
        {
            drop(signing_key);
            return Ok(QueueExecution::Blocked(format!(
                "{}: derived signing key does not match source address {}",
                TransactionPolicyAction::BlockWatchOnlySigner.as_str(),
                payload.source_address
            )));
        }

        // 6. Sign only. The drain persists these exact bytes and their hash
        //    before a separate broadcast phase; once prepared, this job is
        //    never signed again.
        let action_family = plan_action_execution_family(&payload.action)
            .map(|family| family.as_str())
            .unwrap_or("unknown");
        let outcome = self
            .sign_plan_step(
                job_id,
                payload,
                action_family,
                &provider,
                profile.compartment_id,
                step,
                signing_key,
                &session_fingerprint_hex(token),
            )
            .await;
        match outcome {
            Ok(execution) => Ok(execution),
            Err(error) if payload.action == WalletPlanStepAction::ClaimReward => {
                Ok(QueueExecution::OperatorActionRequired(format!(
                    "claim_execution_failed: {}; claim proofs are never auto-retried after a \
                     failed broadcast attempt",
                    error.message()
                )))
            }
            Err(error) => Err(error),
        }
    }

    /// Resolve the policy's `max_fee_per_gas_cap_hex` (if any) to decoded
    /// bytes, reused for BOTH the pre-signing block check and the W7.4
    /// single fee-bump retry ceiling.
    fn resolve_fee_cap_bytes(&self) -> ServiceResult<Option<[u8; 32]>> {
        let Some(cap_hex) = self
            .current_treasury_policy()?
            .and_then(|policy| policy.max_fee_per_gas_cap_hex)
        else {
            return Ok(None);
        };
        Ok(Some(
            decode_quantity_hex(&cap_hex).map_err(map_wallet_error)?,
        ))
    }

    fn plan_step_fee_cap_block_reason(
        &self,
        payload: &PlanStepExecutionPayload,
        fee_cap: Option<[u8; 32]>,
    ) -> ServiceResult<Option<String>> {
        let Some(cap) = fee_cap else {
            return Ok(None);
        };
        let Some(fee_hex) = payload.max_fee_per_gas_hex.as_deref() else {
            return Ok(None);
        };
        let fee = decode_quantity_hex(fee_hex).map_err(map_wallet_error)?;
        if compare_u256(&fee, &cap).is_gt() {
            return Ok(Some(format!(
                "max_fee_per_gas_cap_exceeded: recorded fee {fee_hex} exceeds policy cap"
            )));
        }
        Ok(None)
    }
}

/// A job with prerequisite ids that have not reached receipt-confirmed success
/// defers per E1 semantics; a failed (or missing) prerequisite halts dependents
/// to `blocked`, naming the prerequisite.
pub(super) fn dependency_block_reason(
    payload: &PlanStepExecutionPayload,
    job_states: &HashMap<String, String>,
) -> Option<String> {
    for prerequisite_id in &payload.prerequisite_job_ids {
        match job_states.get(prerequisite_id).map(String::as_str) {
            Some(state) if normalize_queue_state(state) == QUEUE_STATE_CONFIRMED => continue,
            Some(state)
                if matches!(
                    normalize_queue_state(state),
                    QUEUE_STATE_FAILED_TERMINAL
                        | QUEUE_STATE_LEGACY_FAILED
                        | QUEUE_STATE_OPERATOR_ACTION_REQUIRED
                ) =>
            {
                return Some(format!(
                    "dependency_failed: prerequisite job {prerequisite_id} is in state \
                     '{state}'; this step cannot proceed until the prerequisite is resolved"
                ));
            }
            Some(state) => {
                return Some(format!(
                    "dependency_pending: prerequisite job {prerequisite_id} has not yet \
                     succeeded (state='{state}')"
                ));
            }
            None => {
                return Some(format!(
                    "dependency_missing: prerequisite job {prerequisite_id} was not found in \
                     the queue"
                ));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with_prerequisites(prerequisite_job_ids: Vec<String>) -> PlanStepExecutionPayload {
        PlanStepExecutionPayload {
            plan_id: "plan_1".into(),
            step_id: "step_2".into(),
            chain_id: 1,
            source_address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            action: "sweep_native".into(),
            asset_kind: "native".into(),
            asset_address: None,
            amount_hex: "0x1".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            call_label: "native.transfer(value)".into(),
            call_target_address: "0x9999999999999999999999999999999999999999".into(),
            call_data_hex: "0x".into(),
            call_value_wei_hex: Some("0x1".into()),
            simulation_evidence_hash_hex: "ab".repeat(32),
            fee_basis: None,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            prerequisite_job_ids,
        }
    }

    #[test]
    fn dependency_defers_on_pending_prerequisite() {
        let payload = payload_with_prerequisites(vec!["job_a".into()]);
        let mut states = HashMap::new();
        states.insert("job_a".into(), "queued".into());
        let reason = dependency_block_reason(&payload, &states).unwrap();
        assert!(reason.starts_with("dependency_pending:"), "{reason}");
    }

    #[test]
    fn dependency_halts_on_failed_prerequisite_naming_it() {
        let payload = payload_with_prerequisites(vec!["job_a".into()]);
        let mut states = HashMap::new();
        states.insert("job_a".into(), "failed_terminal".into());
        let reason = dependency_block_reason(&payload, &states).unwrap();
        assert!(reason.starts_with("dependency_failed:"), "{reason}");
        assert!(reason.contains("job_a"), "{reason}");
    }

    #[test]
    fn dependency_halts_on_operator_action_required_prerequisite() {
        let payload = payload_with_prerequisites(vec!["job_a".into()]);
        let mut states = HashMap::new();
        states.insert("job_a".into(), "operator_action_required".into());
        let reason = dependency_block_reason(&payload, &states).unwrap();
        assert!(reason.starts_with("dependency_failed:"), "{reason}");
    }

    #[test]
    fn dependency_blocks_on_missing_prerequisite() {
        let payload = payload_with_prerequisites(vec!["ghost_job".into()]);
        let reason = dependency_block_reason(&payload, &HashMap::new()).unwrap();
        assert!(reason.starts_with("dependency_missing:"), "{reason}");
    }

    #[test]
    fn dependency_defers_while_prerequisite_is_broadcast_but_unconfirmed() {
        let payload = payload_with_prerequisites(vec!["job_a".into()]);
        let mut states = HashMap::new();
        states.insert("job_a".into(), "sent".into());
        let reason = dependency_block_reason(&payload, &states).unwrap();
        assert!(reason.starts_with("dependency_pending:"), "{reason}");
    }

    #[test]
    fn dependency_defers_while_prerequisite_is_prepared_or_submission_unknown() {
        let payload = payload_with_prerequisites(vec!["job_a".into()]);
        for state in ["prepared", "submitted_unknown"] {
            let mut states = HashMap::new();
            states.insert("job_a".into(), state.into());
            let reason = dependency_block_reason(&payload, &states).unwrap();
            assert!(reason.starts_with("dependency_pending:"), "{reason}");
        }
    }

    #[test]
    fn dependency_clears_once_prerequisite_is_confirmed() {
        let payload = payload_with_prerequisites(vec!["job_a".into()]);
        let mut states = HashMap::new();
        states.insert("job_a".into(), "confirmed".into());
        assert_eq!(dependency_block_reason(&payload, &states), None);
    }

    #[test]
    fn dependency_none_for_a_step_with_no_prerequisites() {
        let payload = payload_with_prerequisites(Vec::new());
        assert_eq!(dependency_block_reason(&payload, &HashMap::new()), None);
    }
}
