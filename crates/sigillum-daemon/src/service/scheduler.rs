//! Background scheduler (plan task 1.6): advances queue retries, receipt
//! confirmations, and stealth-deposit refreshes while no client is driving
//! the request surface.
//!
//! One periodic loop wakes every `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS`
//! (default 60 s) and runs a bounded cycle through the SAME code paths the
//! request-driven endpoints use:
//!
//! 1. treasury automation — only when the persisted treasury policy has
//!    `enabled && allow_treasury_automation` (both default off), exactly
//!    like the maintenance cycle's first stage;
//! 2. stealth-deposit balance refresh — bounded (the runtime policy's
//!    `deposit_default_refresh_limit`), at most once per
//!    `SIGILLUM_SCHEDULER_REFRESH_SECS` (default 5 min), mirroring the
//!    maintenance refresh path including its auto-enqueue default;
//!    one-time receive lifecycle (plan task 3.3) rides the same cadence —
//!    settle confirmed one-time sweeps (retire + optional purge), observe
//!    one-time allocation balances (auto-watch), and enqueue due one-time
//!    sweeps (dedupe + Sweep-family gates + destination policy +
//!    cross-party linkage, mirroring the stealth deposit sweep rules);
//! 3. queue drain — a bounded batch of [`DRAIN_JOB_LIMIT`] jobs via
//!    [`SigillumService::process_queue_state`], so the durable
//!    `prepared`/`submitted_unknown` barriers, the never-re-sign rule, the
//!    execution gates, and per-(source, chain) serialization all apply
//!    exactly as they do to `POST /api/queue/process`.
//!
//! Fail-closed invariants are preserved by construction:
//!
//! - the vault is never touched while locked: the cycle skips outright
//!   before creating its session or reading any vault state;
//! - `execution_paused` skips the drain stage, and the drain loop itself
//!   re-checks the kill switch between jobs exactly as today;
//! - a cycle runs under `operation_guard` like every other mutating path.
//!
//! Coordination: a cycle that cannot acquire the guard within
//! [`GUARD_WAIT`] is SKIPPED (no queueing storms behind an operator-driven
//! scan/drain/maintenance run). Each cycle has a bounded time budget
//! ([`CYCLE_BUDGET`]); abandoning an over-budget cycle is crash-equivalent
//! because the queue's durable barriers bracket every dangerous region.
//! Repeated consecutive failures back off exponentially
//! ([`MAX_CYCLE_BACKOFF_SECS`]) and log a daemon warning.
//!
//! Observability without spam: ticks are NOT registered as operations (the
//! registry retains 50 records). A cycle registers a `scheduler_cycle`
//! operation only when it actually advanced work (processed > 0 queue jobs
//! or refreshed > 0 deposits), and the status snapshot (last tick time,
//! last outcome, consecutive-failure count, due-work counters) surfaces in
//! `GET /api/diagnostics` under the `scheduler` block.
//!
//! Session discipline: a cycle mints an ephemeral full session to drive the
//! request-time code paths and revokes it on every exit (RAII). The internal
//! session neither consumes nor evicts operator-session capacity, and the
//! scheduler cannot defeat the idle auto-lock — between cycles no scheduler
//! session exists, while an in-flight cycle only ever refreshes its own
//! session's activity clock.

use std::sync::Arc;
use std::time::Duration;

use sigillum_api::{
    EthStealthDepositRefreshRequest, OPERATION_KIND_SCHEDULER_CYCLE, OPERATION_STATE_COMPLETED,
    QueueProcessRequest, SchedulerStatusResponse,
};

use crate::AppState;
use crate::audit_log::AuditEventSpec;

use super::helpers::now_unix;
use super::{ServiceResult, SigillumService};

/// Default queue-drain cadence: one cycle per minute.
const DEFAULT_QUEUE_TICK_SECS: u64 = 60;
/// Default stealth-deposit refresh cadence: every five minutes.
const DEFAULT_REFRESH_SECS: u64 = 300;
/// Upper bound for the exponential failure backoff between cycles.
const MAX_CYCLE_BACKOFF_SECS: u64 = 30 * 60;
/// How long a cycle waits for the operation guard before skipping. Short on
/// purpose: the scheduler is opportunistic background progress and must
/// never queue up behind operator-driven work.
const GUARD_WAIT: Duration = Duration::from_millis(500);
/// Hard time budget for one cycle. An abandoned cycle is crash-equivalent:
/// the queue's durable prepared/submitted_unknown barriers make a drop at
/// any await point recoverable on the next cycle or restart.
const CYCLE_BUDGET: Duration = Duration::from_secs(120);
/// Bounded queue-drain batch per cycle.
const DRAIN_JOB_LIMIT: usize = 25;

const OUTCOME_ADVANCED: &str = "advanced";
const OUTCOME_IDLE: &str = "idle";
const OUTCOME_SKIPPED_LOCKED: &str = "skipped_locked";
const OUTCOME_SKIPPED_GUARD_BUSY: &str = "skipped_guard_busy";
const OUTCOME_FAILED: &str = "failed";

/// Scheduler configuration, loaded once at startup from the environment
/// (same `SIGILLUM_*` override discipline as [`crate::policy::RuntimePolicy`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SchedulerConfig {
    /// `SIGILLUM_SCHEDULER_DISABLE=1` (also `true`/`yes`) turns the loop
    /// off; enabled by default.
    pub enabled: bool,
    /// `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS`, clamped to >= 1.
    pub queue_tick_secs: u64,
    /// `SIGILLUM_SCHEDULER_REFRESH_SECS`, clamped to >= 1.
    pub refresh_secs: u64,
}

impl SchedulerConfig {
    pub(crate) fn from_env() -> Self {
        Self::from_pairs(std::env::vars())
    }

    fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut disabled = false;
        let mut queue_tick_secs = None;
        let mut refresh_secs = None;
        for (key, value) in pairs {
            let key = key.as_ref();
            let value = value.as_ref().trim();
            match key {
                "SIGILLUM_SCHEDULER_DISABLE" => {
                    disabled = matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
                }
                "SIGILLUM_SCHEDULER_QUEUE_TICK_SECS" => {
                    queue_tick_secs = value.parse().ok();
                }
                "SIGILLUM_SCHEDULER_REFRESH_SECS" => {
                    refresh_secs = value.parse().ok();
                }
                _ => {}
            }
        }
        Self {
            enabled: !disabled,
            queue_tick_secs: queue_tick_secs.unwrap_or(DEFAULT_QUEUE_TICK_SECS).max(1),
            refresh_secs: refresh_secs.unwrap_or(DEFAULT_REFRESH_SECS).max(1),
        }
    }

    /// The static half of the diagnostics snapshot, present even before the
    /// first tick (and when the loop is disabled).
    pub(crate) fn status_baseline(&self) -> SchedulerStatusResponse {
        SchedulerStatusResponse {
            enabled: self.enabled,
            queue_tick_secs: self.queue_tick_secs,
            refresh_secs: self.refresh_secs,
            ..Default::default()
        }
    }
}

/// Spawn the scheduler loop unless disabled. Called once by the daemon's
/// run entry point (next to the idle-lock task) and by integration tests
/// opting into background activity. Tolerates being called without a tokio
/// runtime in scope; the production entry points always have one.
pub fn spawn_scheduler(state: Arc<AppState>) {
    let config = SchedulerConfig::from_env();
    if !config.enabled {
        tracing::info!("background scheduler disabled via SIGILLUM_SCHEDULER_DISABLE");
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::warn!("background scheduler not spawned: no tokio runtime in scope");
        return;
    };
    tracing::info!(
        queue_tick_secs = config.queue_tick_secs,
        refresh_secs = config.refresh_secs,
        "background scheduler started"
    );
    runtime.spawn(run_loop(state, config));
}

async fn run_loop(state: Arc<AppState>, config: SchedulerConfig) {
    let service = SigillumService::new(state.clone());
    let mut last_refresh_at_unix: Option<u64> = None;
    loop {
        // First cycle only after one full tick, and consecutive failures
        // back off exponentially (capped) to avoid hammering a sick
        // provider or store.
        let shift = state.scheduler_status().consecutive_failures.min(5);
        let delay_secs = config
            .queue_tick_secs
            .saturating_mul(1u64 << shift)
            .min(MAX_CYCLE_BACKOFF_SECS);
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;

        let refresh_due = last_refresh_at_unix
            .is_none_or(|last| now_unix().saturating_sub(last) >= config.refresh_secs);
        match tokio::time::timeout(CYCLE_BUDGET, run_cycle(&service, &state, refresh_due)).await {
            Ok(Ok(report)) => {
                if report.refresh_ran {
                    last_refresh_at_unix = Some(now_unix());
                }
                state.scheduler_note_cycle(report.outcome, false);
            }
            Ok(Err(error)) => {
                let consecutive_failures = state.scheduler_status().consecutive_failures + 1;
                tracing::warn!(
                    error = %error.message(),
                    consecutive_failures,
                    "background scheduler cycle failed"
                );
                state.scheduler_note_cycle(OUTCOME_FAILED, true);
            }
            Err(_) => {
                let consecutive_failures = state.scheduler_status().consecutive_failures + 1;
                tracing::warn!(
                    budget_secs = CYCLE_BUDGET.as_secs(),
                    consecutive_failures,
                    "background scheduler cycle exceeded its time budget; abandoned crash-safely"
                );
                state.scheduler_note_cycle(OUTCOME_FAILED, true);
            }
        }
    }
}

struct CycleReport {
    outcome: &'static str,
    refresh_ran: bool,
}

/// One bounded background cycle. All stages reuse the request-driven code
/// paths; see the module docs for the invariant list.
async fn run_cycle(
    service: &SigillumService,
    state: &Arc<AppState>,
    refresh_due: bool,
) -> ServiceResult<CycleReport> {
    // Vault lock state first: no vault access without unlock.
    if !state.is_unlocked() || state.is_locking() {
        return Ok(CycleReport {
            outcome: OUTCOME_SKIPPED_LOCKED,
            refresh_ran: false,
        });
    }

    // Ephemeral full session, revoked on every exit path (including a
    // budget-timeout drop) so the idle auto-lock is never defeated.
    let session = InternalSession::create(state);

    // Stage 1 — treasury automation (mirrors the maintenance cycle: before
    // the guard, self-gated on `enabled && allow_treasury_automation`,
    // both default off).
    let automation = service.run_treasury_automation(session.token()).await?;

    // Read-only due-work pre-check, before touching the guard.
    let paused = service.queue_execution_paused()?;
    let queue = crate::queue_store::load_queue(&state.base_dir)
        .map_err(|error| super::ServiceError::internal(format!("Failed to load queue: {error}")))?;
    let drain_due = !paused && super::queue::queue_due_stats(&queue, now_unix()).due_now > 0;
    // Plan task 3.3: active one-time allocations are due work on every tick
    // (settle/enqueue evaluation); their balance observation rides the
    // refresh cadence exactly like the stealth-deposit refresh.
    let one_time_tracked = crate::inventory::load_wallet_inventory(&state.base_dir)
        .map_err(|error| {
            super::ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
        })?
        .receive_allocations
        .iter()
        .any(|allocation| allocation.one_time && allocation.status == "active");
    let refresh_wanted = refresh_due && {
        let deposits_nonempty = !crate::deposits::load_deposits(&state.base_dir)
            .map_err(|error| {
                super::ServiceError::internal(format!("Failed to load deposits: {error}"))
            })?
            .eth_stealth
            .is_empty();
        deposits_nonempty || one_time_tracked
    };
    if !drain_due && !refresh_wanted && !one_time_tracked {
        return Ok(CycleReport {
            outcome: OUTCOME_IDLE,
            refresh_ran: false,
        });
    }

    // A cycle that cannot start promptly SKIPS rather than queueing up
    // behind operator-driven work; per-(source, chain) serialization stays
    // intact because all processing goes through the single guard.
    let _guard = match tokio::time::timeout(GUARD_WAIT, state.operation_guard()).await {
        Ok(guard) => guard,
        Err(_) => {
            return Ok(CycleReport {
                outcome: OUTCOME_SKIPPED_GUARD_BUSY,
                refresh_ran: false,
            });
        }
    };

    // Authoritative reload under the guard, exactly like the maintenance
    // path.
    let mut deposits = crate::deposits::load_deposits(&state.base_dir).map_err(|error| {
        super::ServiceError::internal(format!("Failed to load deposits: {error}"))
    })?;
    let mut queue = crate::queue_store::load_queue(&state.base_dir)
        .map_err(|error| super::ServiceError::internal(format!("Failed to load queue: {error}")))?;

    let mut stages = Vec::new();
    if automation.is_some() {
        stages.push("stage:treasury_automation".to_string());
    }

    // Stage 2 — deposit refresh (bounded; mirrors the maintenance refresh,
    // including its auto-enqueue default).
    let mut refreshed = 0;
    let mut detected = 0;
    let mut queued = 0;
    if refresh_wanted {
        let refresh = service
            .refresh_eth_stealth_deposits_state(
                session.token(),
                &mut deposits,
                &mut queue,
                EthStealthDepositRefreshRequest {
                    id: None,
                    limit: None,
                    auto_enqueue: None,
                },
            )
            .await?;
        refreshed = refresh.processed;
        detected = refresh.detected;
        queued = refresh.queued;
        stages.push("stage:deposit_refresh".to_string());
    }

    // Stage 2.5 — one-time receive lifecycle (plan task 3.3): settle
    // confirmed sweeps (retire + optional purge), observe balances on the
    // refresh cadence (auto-watch), enqueue due sweeps. Everything it
    // enqueues drains under the same gates and barriers in stage 3.
    let one_time = service
        .advance_one_time_receive_allocations_state(session.token(), &mut queue, refresh_due)
        .await?;
    if one_time.tracked {
        stages.push("stage:one_time_receive".to_string());
    }

    // Stage 3 — queue drain (bounded batch). The kill switch is re-checked
    // here under the guard and again by the drain loop between jobs, and
    // the execution gates gate at drain time exactly as today.
    let mut drain = None;
    if !service.queue_execution_paused()? {
        let processed = service
            .process_queue_state(
                session.token(),
                &mut queue,
                QueueProcessRequest {
                    id: None,
                    limit: Some(DRAIN_JOB_LIMIT),
                    run_async: None,
                },
                None,
            )
            .await?;
        stages.push("stage:queue_drain".to_string());
        drain = Some(processed);
    }

    let _ = super::deposits::sync_eth_stealth_deposits_with_queue(&mut deposits, &queue);
    crate::queue_store::save_queue(&state.base_dir, &queue)
        .map_err(|error| super::ServiceError::internal(format!("Failed to save queue: {error}")))?;
    crate::deposits::save_deposits(&state.base_dir, &deposits).map_err(|error| {
        super::ServiceError::internal(format!("Failed to save deposits: {error}"))
    })?;

    let processed_jobs = drain.as_ref().map_or(0, |drain| drain.processed);
    let advanced = refreshed + processed_jobs + one_time.advanced_work();
    if advanced == 0 {
        return Ok(CycleReport {
            outcome: OUTCOME_IDLE,
            refresh_ran: refresh_wanted,
        });
    }

    // Audit the cycle in the same shape the maintenance endpoint records —
    // background value movement must be accountable.
    service.record_audit(
        None,
        AuditEventSpec::MaintenanceRun {
            refreshed,
            detected,
            queued,
            processed: processed_jobs,
            succeeded: drain.as_ref().map_or(0, |drain| drain.succeeded),
            blocked: drain.as_ref().map_or(0, |drain| drain.blocked),
            retrying: drain.as_ref().map_or(0, |drain| drain.retrying),
            failed: drain.as_ref().map_or(0, |drain| drain.failed),
        },
    )?;

    // Register the cycle as a completed summary operation — only cycles
    // that advanced work, so routine ticks never churn the bounded registry
    // (SSE operation events come free via the registry).
    let operation = state.start_operation(OPERATION_KIND_SCHEDULER_CYCLE, stages);
    state.operation_set_progress_total(operation.id(), advanced as u64);
    state.operation_set_progress(operation.id(), advanced as u64);
    state.finish_operation(operation.id(), OPERATION_STATE_COMPLETED, None);

    Ok(CycleReport {
        outcome: OUTCOME_ADVANCED,
        refresh_ran: refresh_wanted,
    })
}

/// Ephemeral full session for one scheduler cycle. Revocation on drop
/// (including when an over-budget cycle future is dropped) guarantees the
/// scheduler leaves no session behind that could defer the idle auto-lock.
struct InternalSession<'a> {
    state: &'a AppState,
    token: String,
}

impl<'a> InternalSession<'a> {
    fn create(state: &'a AppState) -> Self {
        Self {
            state,
            token: state.create_internal_session(None),
        }
    }

    fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for InternalSession<'_> {
    fn drop(&mut self) {
        self.state.revoke_session(&self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_config_defaults_match_documented_baseline() {
        let config = SchedulerConfig::from_pairs([] as [(&str, &str); 0]);

        assert!(config.enabled);
        assert_eq!(config.queue_tick_secs, 60);
        assert_eq!(config.refresh_secs, 300);

        let baseline = config.status_baseline();
        assert!(baseline.enabled);
        assert_eq!(baseline.queue_tick_secs, 60);
        assert_eq!(baseline.refresh_secs, 300);
        assert_eq!(baseline.last_tick_at_unix, None);
        assert_eq!(baseline.consecutive_failures, 0);
    }

    #[test]
    fn scheduler_config_parses_overrides_and_disable() {
        let config = SchedulerConfig::from_pairs([
            ("SIGILLUM_SCHEDULER_QUEUE_TICK_SECS", "5"),
            ("SIGILLUM_SCHEDULER_REFRESH_SECS", "30"),
        ]);
        assert!(config.enabled);
        assert_eq!(config.queue_tick_secs, 5);
        assert_eq!(config.refresh_secs, 30);

        for value in ["1", "true", "TRUE", "yes"] {
            let config = SchedulerConfig::from_pairs([("SIGILLUM_SCHEDULER_DISABLE", value)]);
            assert!(
                !config.enabled,
                "disable value {value:?} must turn the loop off"
            );
            assert!(!config.status_baseline().enabled);
        }

        let config = SchedulerConfig::from_pairs([("SIGILLUM_SCHEDULER_DISABLE", "0")]);
        assert!(config.enabled);
    }

    #[test]
    fn scheduler_config_clamps_invalid_intervals() {
        let config = SchedulerConfig::from_pairs([
            ("SIGILLUM_SCHEDULER_QUEUE_TICK_SECS", "0"),
            ("SIGILLUM_SCHEDULER_REFRESH_SECS", "not-a-number"),
        ]);

        assert_eq!(config.queue_tick_secs, 1);
        assert_eq!(config.refresh_secs, DEFAULT_REFRESH_SECS);
    }

    #[test]
    fn internal_session_is_full_normal_auth_and_raii_revoked() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
        state.unlock_compartment(
            0,
            [1u8; 32],
            sigillum_fido2::config::CompartmentMeta {
                id: 0,
                label: "daily".into(),
                threshold: 1,
                passphrase_mode: None,
            },
        );

        let token;
        {
            let session = InternalSession::create(&state);
            token = session.token().to_string();
            assert!(state.verify_token_passive(&token));
            assert!(state.session_is_full(&token));
            assert_eq!(
                state.session_count(),
                0,
                "internal credentials are not operator sessions"
            );
        }

        assert!(!state.verify_token_passive(&token));
    }
}
