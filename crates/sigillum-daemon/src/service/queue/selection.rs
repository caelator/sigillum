//! Operation-progress job selection for the drain loop.
//!
//! A drain's work extent is knowable up front — unlike a discovery scan's
//! gap-limited crawl — so its operation reports `progress.total`. The
//! selection mirrors the drain loop's own admission decisions (target-id
//! filter, runnability, per-source serialization, batch limit) against the
//! drain-start snapshots. Split out of `processing.rs` to keep the drain
//! loop itself readable (house architecture cap).

use sigillum_api::QueueProcessRequest;

use super::serialization;
use super::state::queue_job_is_runnable;

/// Which jobs a drain will attempt, tracked for operation progress.
///
/// Computed once at drain start, then adjusted live as the loop re-checks
/// admission against evolving snapshots: a selected job parked mid-drain by
/// per-source serialization shrinks the total ([`Self::deselect`]), and an
/// initially-skipped job admitted mid-drain (its backoff expired or its
/// source freed) grows it ([`Self::select`]). The total therefore always
/// equals the attempted-job count when the drain finishes a full pass, and
/// `total - processed` is exactly the remaining selected jobs when the
/// drain stops early (pause latch or operator cancel).
pub(super) struct QueueDrainSelection {
    /// Parallel to `QueueState::jobs`: whether the job at that index is
    /// currently selected for this drain.
    selected: Vec<bool>,
    /// Current selected count — reported as the operation's
    /// `progress.total`.
    total: usize,
}

impl QueueDrainSelection {
    /// Compute the drain-start selection with the same decisions the loop
    /// makes, in the same order, against the initial in-flight-source
    /// snapshot. `now` is the drain-start timestamp (the loop re-reads the
    /// clock per job; a backoff that expires mid-drain is admitted later
    /// via [`Self::select`]).
    pub(super) fn new(
        queue: &crate::queue_store::QueueState,
        body: &QueueProcessRequest,
        limit: usize,
        now: u64,
    ) -> Self {
        let force_target = body.id.is_some();
        let in_flight_sources = serialization::build_in_flight_sources(&queue.jobs);
        let mut selected = vec![false; queue.jobs.len()];
        let mut total = 0usize;
        for (index, job) in queue.jobs.iter().enumerate() {
            if total >= limit {
                break;
            }
            if let Some(target_id) = body.id.as_deref() {
                if job.id != target_id {
                    continue;
                }
            }
            if !queue_job_is_runnable(job, force_target, now) {
                if body.id.is_some() {
                    break;
                }
                continue;
            }
            if serialization::skip_reason(job, &in_flight_sources).is_some() {
                if body.id.is_some() {
                    break;
                }
                continue;
            }
            selected[index] = true;
            total += 1;
        }
        Self { selected, total }
    }

    /// The current selected-job count (operation `progress.total`).
    pub(super) fn total(&self) -> usize {
        self.total
    }

    /// Mark the job at `index` selected (mid-drain admission). Returns the
    /// new total only when the selection actually changed.
    pub(super) fn select(&mut self, index: usize) -> Option<usize> {
        if self.selected[index] {
            return None;
        }
        self.selected[index] = true;
        self.total += 1;
        Some(self.total)
    }

    /// Mark the job at `index` unselected (a drain-start-selected job the
    /// loop just parked, e.g. per-source serialization). Returns the new
    /// total only when the selection actually changed.
    pub(super) fn deselect(&mut self, index: usize) -> Option<usize> {
        if !self.selected[index] {
            return None;
        }
        self.selected[index] = false;
        self.total = self.total.saturating_sub(1);
        Some(self.total)
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::{QueueJob, QueueJobPayload};

    use super::*;

    fn stealth_job(id: &str, state: &str) -> QueueJob {
        QueueJob {
            id: id.into(),
            state: state.into(),
            attempts: 0,
            created_at_unix: 1,
            updated_at_unix: 1,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthTransfer {
                wallet_profile: "payments".into(),
                stealth_address: "0x0000000000000000000000000000000000000001".into(),
                ephemeral_public_key_hex: "0x02".into(),
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        }
    }

    fn queue(states: &[&str]) -> crate::queue_store::QueueState {
        crate::queue_store::QueueState {
            jobs: states
                .iter()
                .enumerate()
                .map(|(index, state)| stealth_job(&format!("job-{index}"), state))
                .collect(),
        }
    }

    fn body(id: Option<&str>) -> QueueProcessRequest {
        QueueProcessRequest {
            id: id.map(str::to_string),
            limit: None,
            run_async: None,
        }
    }

    #[test]
    fn selection_counts_runnable_jobs_up_to_limit() {
        let queue = queue(&["queued", "sent", "queued", "failed"]);
        // `sent` legacy stealth jobs are terminal for the drain (only
        // PlanStepExecution jobs are revisited from `sent`); legacy
        // `failed` normalizes to `failed_terminal` — neither is runnable.
        let selection = QueueDrainSelection::new(&queue, &body(None), 10, 100);
        assert_eq!(selection.total(), 2);

        let limited = QueueDrainSelection::new(&queue, &body(None), 1, 100);
        assert_eq!(limited.total(), 1);
    }

    #[test]
    fn selection_honors_target_id_break_semantics() {
        let queue = queue(&["queued", "sent", "queued"]);
        let missing = QueueDrainSelection::new(&queue, &body(Some("nope")), 10, 100);
        assert_eq!(missing.total(), 0);
        let targeted = QueueDrainSelection::new(&queue, &body(Some("job-2")), 10, 100);
        assert_eq!(targeted.total(), 1);
        // A non-runnable target selects nothing (the loop breaks on it).
        let terminal = QueueDrainSelection::new(&queue, &body(Some("job-1")), 10, 100);
        assert_eq!(terminal.total(), 0);
    }

    #[test]
    fn select_and_deselect_adjust_the_total_once() {
        let queue = queue(&["queued", "queued"]);
        let mut selection = QueueDrainSelection::new(&queue, &body(None), 10, 100);
        assert_eq!(selection.total(), 2);
        assert_eq!(selection.deselect(1), Some(1));
        assert_eq!(selection.deselect(1), None, "second deselect is a no-op");
        assert_eq!(selection.select(1), Some(2));
        assert_eq!(selection.select(1), None, "second select is a no-op");
        assert_eq!(selection.total(), 2);
    }
}
