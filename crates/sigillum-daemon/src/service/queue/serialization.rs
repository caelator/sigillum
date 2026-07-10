//! W7.4 per-source serialization: at most one in-flight
//! (broadcast-but-unconfirmed) `PlanStepExecution` job per (chain id,
//! lowercased source address) may broadcast at a time. Split out of
//! `processing.rs` to keep the drain loop itself readable (house
//! architecture cap).

use std::collections::HashMap;

use sigillum_api::{QueueJob, QueueJobPayload};

use super::state::normalize_queue_state;
use super::{QUEUE_STATE_PREPARED, QUEUE_STATE_SENT, QUEUE_STATE_SUBMITTED_UNKNOWN};

pub(super) type SourceKey = (u64, String);

/// Only `PlanStepExecution` jobs carry an explicit `chain_id` +
/// `source_address` pair — legacy `EthSeed*`/`EthStealth*` families are
/// unaffected by construction (`None`).
fn plan_step_source_key(payload: &QueueJobPayload) -> Option<SourceKey> {
    match payload {
        QueueJobPayload::PlanStepExecution(step) => {
            Some((step.chain_id, step.source_address.to_ascii_lowercase()))
        }
        _ => None,
    }
}

/// A job whose `prerequisite_job_ids` explicitly names `occupant_job_id` is
/// already safely sequenced AFTER it (W6.4 dependency ordering) — nonce
/// fetch-at-broadcast-time means it naturally gets the next nonce once its
/// prerequisite has broadcast. Per-source serialization exists to guard
/// INDEPENDENT jobs sharing a source, not the same-batch
/// sweep→revoke→fund_gas chain W7.3 already resolves in one drain call.
fn plan_step_depends_on(payload: &QueueJobPayload, occupant_job_id: &str) -> bool {
    match payload {
        QueueJobPayload::PlanStepExecution(step) => step
            .prerequisite_job_ids
            .iter()
            .any(|id| id == occupant_job_id),
        _ => false,
    }
}

/// Initial (chain, source) -> occupying job id snapshot, built once before
/// the drain loop from every job currently `sent`.
pub(super) fn build_in_flight_sources(jobs: &[QueueJob]) -> HashMap<SourceKey, String> {
    jobs.iter()
        .filter_map(|job| {
            if matches!(
                normalize_queue_state(&job.state),
                QUEUE_STATE_PREPARED | QUEUE_STATE_SUBMITTED_UNKNOWN | QUEUE_STATE_SENT
            ) {
                plan_step_source_key(&job.payload).map(|key| (key, job.id.clone()))
            } else {
                None
            }
        })
        .collect()
}

/// `Some(reason)` when `job` must be skipped THIS drain cycle: its source
/// is occupied by another in-flight job it is not dependency-ordered
/// after. E1-consistent representation (see the W7.4 report /
/// `plan_steps.rs` module doc): the job's persisted state is UNCHANGED
/// (stays `queued`/`blocked`/`retrying`) — this is a transient,
/// visible-via-`last_error` skip, never a new persisted state and never the
/// legacy `deferred` wire string.
pub(super) fn skip_reason(
    job: &QueueJob,
    in_flight_sources: &HashMap<SourceKey, String>,
) -> Option<String> {
    if matches!(
        normalize_queue_state(&job.state),
        QUEUE_STATE_PREPARED | QUEUE_STATE_SUBMITTED_UNKNOWN | QUEUE_STATE_SENT
    ) {
        return None;
    }
    let key = plan_step_source_key(&job.payload)?;
    let occupant_job_id = in_flight_sources.get(&key)?;
    if occupant_job_id == &job.id || plan_step_depends_on(&job.payload, occupant_job_id) {
        return None;
    }
    Some(format!(
        "source_serialization: waiting for in-flight job {occupant_job_id} on source {} \
         (chain {}) to confirm before this job may broadcast",
        key.1, key.0
    ))
}

/// Refresh the snapshot for `job` immediately after its outcome is known,
/// so a source that just freed up (left `sent`) or was newly occupied
/// (just reached `sent`) is visible to a same-batch sibling right away.
pub(super) fn refresh(map: &mut HashMap<SourceKey, String>, job: &QueueJob) {
    let Some(key) = plan_step_source_key(&job.payload) else {
        return;
    };
    if matches!(
        normalize_queue_state(&job.state),
        QUEUE_STATE_PREPARED | QUEUE_STATE_SUBMITTED_UNKNOWN | QUEUE_STATE_SENT
    ) {
        map.insert(key, job.id.clone());
    } else if map.get(&key) == Some(&job.id) {
        map.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::PlanStepExecutionPayload;

    use super::*;

    fn plan_step_job(id: &str, state: &str, chain_id: u64, source: &str) -> QueueJob {
        QueueJob {
            id: id.into(),
            state: state.into(),
            attempts: 0,
            created_at_unix: 1,
            updated_at_unix: 1,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::PlanStepExecution(Box::new(PlanStepExecutionPayload {
                plan_id: "plan_1".into(),
                step_id: format!("step_{id}"),
                chain_id,
                source_address: source.into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                wallet_family: "eth-seed".into(),
                wallet_profile: "seed-a".into(),
                provider_profile: "mainnet".into(),
                action: "sweep_native".into(),
                asset_kind: "native".into(),
                asset_address: None,
                amount_hex: "0x1".into(),
                destination_address: None,
                call_label: "native.transfer(value)".into(),
                call_target_address: "0x0000000000000000000000000000000000000002".into(),
                call_data_hex: "0x".into(),
                call_value_wei_hex: None,
                simulation_evidence_hash_hex: "ab".repeat(32),
                fee_basis: None,
                max_priority_fee_per_gas_hex: None,
                max_fee_per_gas_hex: None,
                prerequisite_job_ids: Vec::new(),
            })),
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        }
    }

    #[test]
    fn independent_same_source_job_is_skipped_while_occupant_is_in_flight() {
        let occupant = plan_step_job(
            "job_a",
            "sent",
            1,
            "0xAAAA000000000000000000000000000000AAAA",
        );
        let waiting = plan_step_job(
            "job_b",
            "queued",
            1,
            "0xaaaa000000000000000000000000000000aaaa",
        );
        let map = build_in_flight_sources(std::slice::from_ref(&occupant));
        let reason = skip_reason(&waiting, &map).expect("should be skipped");
        assert!(reason.starts_with("source_serialization:"), "{reason}");
        assert!(reason.contains("job_a"), "{reason}");
    }

    #[test]
    fn dependency_ordered_job_is_exempt_from_serialization() {
        let occupant = plan_step_job(
            "job_a",
            "sent",
            1,
            "0xaaaa000000000000000000000000000000aaaa",
        );
        let mut dependent = plan_step_job(
            "job_b",
            "queued",
            1,
            "0xaaaa000000000000000000000000000000aaaa",
        );
        if let QueueJobPayload::PlanStepExecution(step) = &mut dependent.payload {
            step.prerequisite_job_ids.push("job_a".into());
        }
        let map = build_in_flight_sources(std::slice::from_ref(&occupant));
        assert_eq!(skip_reason(&dependent, &map), None);
    }

    #[test]
    fn different_chain_or_source_is_never_serialized() {
        let occupant = plan_step_job(
            "job_a",
            "sent",
            1,
            "0xaaaa000000000000000000000000000000aaaa",
        );
        let other_chain = plan_step_job(
            "job_b",
            "queued",
            8453,
            "0xaaaa000000000000000000000000000000aaaa",
        );
        let other_source = plan_step_job(
            "job_c",
            "queued",
            1,
            "0xbbbb000000000000000000000000000000bbbb",
        );
        let map = build_in_flight_sources(std::slice::from_ref(&occupant));
        assert_eq!(skip_reason(&other_chain, &map), None);
        assert_eq!(skip_reason(&other_source, &map), None);
    }

    #[test]
    fn refresh_frees_the_source_once_the_occupant_leaves_sent() {
        let mut map = HashMap::new();
        let mut job = plan_step_job(
            "job_a",
            "sent",
            1,
            "0xaaaa000000000000000000000000000000aaaa",
        );
        refresh(&mut map, &job);
        assert_eq!(map.len(), 1);
        job.state = "confirmed".into();
        refresh(&mut map, &job);
        assert!(map.is_empty());
    }
}
