//! Asynchronous job queue for transaction and sweep operations.
//!
//! Queues, processes, and tracks Ethereum stealth transfers and deposit
//! sweeps with deferred execution and retry logic.

mod authorization;
mod broadcast;
mod dispatch;
mod enqueue;
#[cfg(test)]
mod enqueue_tests;
mod execution;
mod failpoints;
mod failure;
mod gates;
mod in_flight;
mod outcomes;
mod pause;
mod payloads;
mod plan_steps;
mod processing;
mod replay;
mod seed_sends;
mod selection;
mod serialization;
mod state;
mod status;
mod stealth_gas_topup;
mod sweeps;
mod tally;

use sigillum_api::{QueueJobListResponse, QueueJobPayload};

pub(super) use execution::QueueExecution;

pub(super) use state::{
    is_active_or_completed_queue_state, is_active_queue_state, mark_job_operator_action_required,
    queue_job_failed_state, queue_job_operator_action_required, queue_job_sweep_settled_state,
    recover_queue_job,
};
pub(super) use status::{count_queue_states, queue_status};

// W7.2 plan-step enqueue (service/inventory/plan_execution_enqueue.rs) reuses
// the queue domain's gate evaluation and job construction.
pub(in crate::service) use gates::{
    ExecutionFamily, execution_gate_denial, plan_action_execution_family,
};
pub(in crate::service) use payloads::queued_job;
pub(in crate::service) use state::queue_due_stats;

use super::list_query;
use super::{ServiceError, ServiceResult, SigillumService};

// ── Queue State Constants ──────────────────────────────────────────────────

const QUEUE_STATE_QUEUED: &str = "queued";
const QUEUE_STATE_BLOCKED: &str = "blocked";
const QUEUE_STATE_RETRYING: &str = "retrying";
/// Signed bytes and their locally-derived hash are durably persisted. No
/// network submission has started yet, and this job must never be re-signed.
const QUEUE_STATE_PREPARED: &str = "prepared";
/// A pre-broadcast marker was durably persisted before the RPC call. The
/// network outcome may be unknown after a crash; recovery polls by hash or
/// resubmits the exact prepared bytes.
const QUEUE_STATE_SUBMITTED_UNKNOWN: &str = "submitted_unknown";
const QUEUE_STATE_SENT: &str = "sent";
/// W7.4: new first-class state (schema v4). `sent` means "broadcast,
/// awaiting confirmation"; `confirmed` means the receipt reached the
/// chain's configured `finality_blocks` depth with a SUCCESS status.
/// Genuinely terminal — never revisited by the drain loop.
const QUEUE_STATE_CONFIRMED: &str = "confirmed";
const QUEUE_STATE_FAILED_TERMINAL: &str = "failed_terminal";
const QUEUE_STATE_OPERATOR_ACTION_REQUIRED: &str = "operator_action_required";
const QUEUE_STATE_LEGACY_DEFERRED: &str = "deferred";
const QUEUE_STATE_LEGACY_FAILED: &str = "failed";

// ── Queue Operations: Listing ──────────────────────────────────────────────

impl SigillumService {
    pub(crate) fn list_queue_jobs(
        &self,
        token: Option<&str>,
        query: list_query::QueueJobListQuery,
    ) -> ServiceResult<QueueJobListResponse> {
        let _ = self.require_session(token)?;
        let state = query
            .state
            .map(|value| list_query::validated_value("state", value, &list_query::QUEUE_JOB_STATES))
            .transpose()?;
        let kind = query
            .kind
            .map(|value| list_query::validated_value("kind", value, &list_query::QUEUE_JOB_KINDS))
            .transpose()?;
        let queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let mut jobs = queue.jobs;
        if let Some(state) = state.as_deref() {
            jobs.retain(|job| job.state == state);
        }
        if let Some(kind) = kind.as_deref() {
            jobs.retain(|job| queue_job_kind(job) == kind);
        }
        if let Some(chain_id) = query.chain_id {
            jobs.retain(|job| queue_job_chain_id(job) == Some(chain_id));
        }
        if let Some(sort) = query.sort {
            let order = list_query::effective_order(query.sort.as_ref(), query.order);
            let key = |job: &sigillum_api::QueueJob| match sort {
                list_query::CreatedUpdatedSort::Created => job.created_at_unix,
                list_query::CreatedUpdatedSort::Updated => job.updated_at_unix,
            };
            match order {
                list_query::SortOrder::Asc => jobs.sort_by_key(&key),
                list_query::SortOrder::Desc => jobs.sort_by_key(|job| std::cmp::Reverse(key(job))),
            }
        }
        let (jobs, pagination) = list_query::paginate(jobs, query.page);
        Ok(QueueJobListResponse {
            jobs: jobs.into_iter().map(queue_job_for_response).collect(),
            pagination,
        })
    }
}

fn queue_job_for_response(mut job: sigillum_api::QueueJob) -> sigillum_api::QueueJob {
    // Signed bytes are intentionally persisted for crash-safe replay, but
    // returning them over list/process/maintenance APIs would grant any API
    // consumer an unnecessary transaction-submission capability.
    job.receipt.signed_raw_transaction_hex = None;
    job
}

/// Wire `kind` tag of a job payload, for the `kind` list filter.
fn queue_job_kind(job: &sigillum_api::QueueJob) -> &'static str {
    match &job.payload {
        QueueJobPayload::EthStealthTransfer { .. } => "eth_stealth_transfer",
        QueueJobPayload::EthStealthErc20Transfer { .. } => "eth_stealth_erc20_transfer",
        QueueJobPayload::EthStealthNativeSweep { .. } => "eth_stealth_native_sweep",
        QueueJobPayload::EthStealthErc20Sweep { .. } => "eth_stealth_erc20_sweep",
        QueueJobPayload::EthStealthGasTopup { .. } => "eth_stealth_gas_topup",
        QueueJobPayload::EthSeedTransfer { .. } => "eth_seed_transfer",
        QueueJobPayload::EthSeedNativeSweep { .. } => "eth_seed_native_sweep",
        QueueJobPayload::EthSeedErc20Sweep { .. } => "eth_seed_erc20_sweep",
        QueueJobPayload::PlanStepExecution(_) => "plan_step_execution",
    }
}

/// Chain id carried by the job payload, when it carries one: only
/// `plan_step_execution` jobs record a chain today, so a `chain_id` filter
/// matches those jobs exclusively.
fn queue_job_chain_id(job: &sigillum_api::QueueJob) -> Option<u64> {
    match &job.payload {
        QueueJobPayload::PlanStepExecution(payload) => Some(payload.chain_id),
        _ => None,
    }
}
