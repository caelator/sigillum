//! Shared daemon state: vault registry, session management, and compartment tracking.
//!
//! [`AppState`] is the single `Arc`-shared struct that Axum handlers and the service
//! layer access concurrently. It owns:
//!
//! - **Vault registry** — up to [`SHARD_SLOTS`](sigillum_fido2::config::SHARD_SLOTS)
//!   [`FileVault`] instances, one per potential compartment.
//! - **Session tokens** — random 256-bit bearer tokens with per-session active
//!   compartment tracking. Tokens are compared in constant time via `subtle::ConstantTimeEq`.
//! - **Unlocked compartment metadata** — cached [`CompartmentMeta`] for each
//!   compartment that has been successfully unlocked in this process lifetime.
//! - **Unlock throttle** — exponential backoff on failed unlock attempts to
//!   resist online brute-force.
//! - **Operation mutex** — an async `tokio::sync::Mutex` that serializes mutating
//!   operations (writes, key registration, queue processing) for crash safety.
//!
//! All mutable interior state is wrapped in [`ResilientMutex`]. Because this
//! state contains vault, session, throttle, and lock invariants, poisoning is
//! treated as unrecoverable: the process aborts instead of using potentially
//! half-mutated security state. The service supervisor is responsible for a
//! clean restart.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::RngCore;
use rand::rngs::OsRng;
use serde::Serialize;
use sigillum_api::AuditEvent;
use sigillum_core::{FileVault, VaultConfig, VaultLifecycle, recover_snapshot_restore};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::CompartmentMeta;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;

mod audit_keys;
mod recovery_files;
#[cfg(test)]
mod tests;

pub(crate) use recovery_files::recover_compartment_replacements;
use recovery_files::{restore_stashed_ops_dir, stash_snapshot_placeholder_ops};

use crate::DaemonInitError;
use crate::audit_db::AuditQuery;
use crate::audit_log::{AuditEventSpec, StoredAuditEvent};
use crate::operations::{OperationGuard, PendingOperationSpec, list_pending_operations};
use crate::policy::RuntimePolicy;

/// A fail-closed wrapper around `std::sync::Mutex<T>`.
///
/// A panic while holding security-sensitive state can leave logical invariants
/// half-applied even when the value remains memory-safe. There is no generic
/// validator for `T`, so a poisoned lock aborts the process. This preserves the
/// existing infallible `lock()` API while ensuring no caller silently consumes
/// unvalidated state after a panic.
pub struct ResilientMutex<T> {
    inner: Mutex<T>,
}

impl<T> ResilientMutex<T> {
    /// Create a new `ResilientMutex` wrapping a value.
    pub fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
        }
    }

    /// Acquire the lock, aborting the process if a previous holder poisoned it.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => abort_on_security_state_poison(),
        }
    }
}

fn abort_on_security_state_poison() -> ! {
    tracing::error!(
        "security state mutex poisoned; aborting rather than using potentially inconsistent state"
    );
    std::process::abort()
}

impl<T: fmt::Debug> fmt::Debug for ResilientMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.lock() {
            Ok(guard) => f
                .debug_struct("ResilientMutex")
                .field("value", &*guard)
                .finish(),
            Err(_) => abort_on_security_state_poison(),
        }
    }
}

/// Maximum number of concurrent sessions before the oldest is evicted.
const MAX_SESSIONS: usize = 64;

/// Session time-to-live. Sessions older than this are considered expired.
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Maximum failed unlock attempts before a cooldown is enforced.
const MAX_UNLOCK_ATTEMPTS: u32 = 5;

/// Base cooldown after exceeding the failed-attempt threshold.
/// Actual delay doubles with each subsequent failure (exponential backoff).
const UNLOCK_COOLDOWN_BASE: Duration = Duration::from_secs(5);
const BIOMETRIC_CHALLENGE_TTL: Duration = Duration::from_secs(60);
const MAX_BIOMETRIC_CHALLENGES: usize = 64;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug)]
struct BiometricChallengeState {
    id: [u8; 16],
    nonce: [u8; 32],
    issued_at: Instant,
}

#[derive(Clone, Debug)]
struct SessionState {
    active_compartment_id: Option<usize>,
    created_at: Instant,
    expires_at: Instant,
    last_activity: Instant,
    scopes: Option<Vec<String>>,
}

impl Default for SessionState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            active_compartment_id: None,
            created_at: now,
            expires_at: now + SESSION_TTL,
            last_activity: now,
            scopes: None,
        }
    }
}

/// Tracks failed authentication attempts for rate limiting.
#[derive(Debug, Default)]
struct UnlockThrottle {
    consecutive_failures: u32,
    last_failure: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct StartupRecoverySummary {
    pub interrupted_operation_count: usize,
    pub recovered_operation_count: usize,
    pub unresolved_operation_count: usize,
    pub recovered_queue_job_count: usize,
    pub reconciled_deposit_count: usize,
}

/// Shared daemon state with multi-compartment vault support and deniability.
///
/// Compartment metadata is only known after unlock (discovered from encrypted
/// `meta.enc` files). The `unlocked` map tracks discovered compartments.
pub struct AppState {
    pub fido2: Fido2Manager,
    pub base_dir: PathBuf,
    started_at_unix: u64,
    http_client: reqwest::Client,
    runtime_policy: RuntimePolicy,
    vaults: ResilientMutex<HashMap<usize, FileVault>>,
    /// Compartments that have been unlocked and verified — comp_id → meta.
    unlocked: ResilientMutex<HashMap<usize, CompartmentMeta>>,
    /// Per-session state keyed by bearer token.
    sessions: ResilientMutex<HashMap<String, SessionState>>,
    /// Serializes state-changing operations that touch on-disk data.
    operation_lock: AsyncMutex<()>,
    /// Preemptive queue kill-switch latch. This deliberately lives outside
    /// `operation_lock` so a pause request can stop a drain that currently
    /// holds the disk-operation mutex. The persisted treasury policy remains
    /// the source of truth across restarts; startup recovery synchronizes it.
    queue_execution_paused: AtomicBool,
    /// Rate limiter for failed unlock attempts.
    unlock_throttle: ResilientMutex<UnlockThrottle>,
    /// Startup-time reconciliation summary for observability.
    startup_recovery: ResilientMutex<StartupRecoverySummary>,
    startup_ready: ResilientMutex<bool>,
    startup_error: ResilientMutex<Option<String>>,
    lock_state: ResilientMutex<LockState>,
    biometric_challenges: ResilientMutex<VecDeque<BiometricChallengeState>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockState {
    Ready,
    Locking,
}

impl AppState {
    pub fn new(base_dir: PathBuf) -> Result<Self, DaemonInitError> {
        let preserved_ops = stash_snapshot_placeholder_ops(&base_dir).ok().flatten();
        let _ = recover_snapshot_restore(&base_dir);
        if let Some(preserved_ops) = preserved_ops.as_deref() {
            let _ = restore_stashed_ops_dir(&base_dir, preserved_ops);
        }
        let _ = recover_compartment_replacements(&base_dir);
        let fido2_config_path = base_dir.join("fido2_keys.json");
        let fido2 = Fido2Manager::new(fido2_config_path);
        let started_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Err(error) = crate::audit::migration::migrate_jsonl_to_sqlite(&base_dir) {
            tracing::warn!(error = %error, "failed to initialize audit database");
        }

        Ok(Self {
            fido2,
            base_dir,
            started_at_unix,
            http_client: reqwest::Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .timeout(HTTP_REQUEST_TIMEOUT)
                .build()?,
            runtime_policy: RuntimePolicy::from_env(),
            vaults: ResilientMutex::new(HashMap::new()),
            unlocked: ResilientMutex::new(HashMap::new()),
            sessions: ResilientMutex::new(HashMap::new()),
            operation_lock: AsyncMutex::new(()),
            queue_execution_paused: AtomicBool::new(false),
            unlock_throttle: ResilientMutex::new(UnlockThrottle::default()),
            startup_recovery: ResilientMutex::new(StartupRecoverySummary::default()),
            startup_ready: ResilientMutex::new(false),
            startup_error: ResilientMutex::new(None),
            lock_state: ResilientMutex::new(LockState::Ready),
            biometric_challenges: ResilientMutex::new(VecDeque::new()),
        })
    }

    fn token_matches(stored: &str, candidate: &str) -> bool {
        let a = stored.as_bytes();
        let b = candidate.as_bytes();
        a.len() == b.len() && a.ct_eq(b).into()
    }

    #[must_use]
    pub fn started_at_unix(&self) -> u64 {
        self.started_at_unix
    }

    #[must_use]
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    #[must_use]
    pub fn runtime_policy(&self) -> RuntimePolicy {
        self.runtime_policy
    }

    #[must_use]
    pub fn startup_ready(&self) -> bool {
        *self.startup_ready.lock()
    }

    #[must_use]
    pub fn startup_error(&self) -> Option<String> {
        self.startup_error.lock().clone()
    }

    pub fn mark_startup_ready(&self) {
        *self.startup_ready.lock() = true;
        *self.startup_error.lock() = None;
    }

    pub fn mark_startup_failed(&self, error: impl Into<String>) {
        *self.startup_ready.lock() = false;
        *self.startup_error.lock() = Some(error.into());
    }

    fn session_key_for(
        sessions: &HashMap<String, SessionState>,
        candidate: &str,
    ) -> Option<String> {
        sessions
            .keys()
            .find(|stored| Self::token_matches(stored, candidate))
            .cloned()
    }

    #[must_use]
    pub fn default_active_compartment_id(&self) -> Option<usize> {
        self.unlocked
            .lock()
            .values()
            .min_by_key(|m| m.threshold)
            .map(|m| m.id)
    }

    /// Currently active compartment id for a session.
    #[must_use]
    pub fn active_compartment_id_for(&self, token: &str) -> Option<usize> {
        let active = {
            let sessions = self.sessions.lock();
            sessions
                .iter()
                .find(|(stored, _)| Self::token_matches(stored, token))
                .and_then(|(_, session)| session.active_compartment_id)
        };
        active.or_else(|| self.default_active_compartment_id())
    }

    /// Path to a compartment's data directory.
    pub fn compartment_dir(&self, id: usize) -> PathBuf {
        self.base_dir.join("compartments").join(id.to_string())
    }

    pub async fn operation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.operation_lock.lock().await
    }

    #[must_use]
    pub fn queue_execution_pause_latched(&self) -> bool {
        self.queue_execution_paused.load(Ordering::Acquire)
    }

    pub fn set_queue_execution_pause_latch(&self, paused: bool) {
        self.queue_execution_paused.store(paused, Ordering::Release);
    }

    #[must_use]
    pub fn is_locking(&self) -> bool {
        *self.lock_state.lock() == LockState::Locking
    }

    #[must_use]
    pub fn begin_locking(&self) -> bool {
        if !self.is_unlocked() {
            return false;
        }
        let mut state = self.lock_state.lock();
        if *state == LockState::Locking {
            return false;
        }
        *state = LockState::Locking;
        true
    }

    pub fn finish_locking(&self) {
        *self.lock_state.lock() = LockState::Ready;
    }

    /// Lock and zeroize all unlocked compartments while leaving the daemon running.
    pub async fn lock_now(&self) -> bool {
        if !self.begin_locking() {
            return false;
        }
        let _guard = self.operation_guard().await;
        self.lock_all();
        true
    }

    pub fn salt_path(&self, id: usize) -> PathBuf {
        self.compartment_dir(id).join("passphrase.salt")
    }

    pub fn wrapped_key_path(&self, id: usize) -> PathBuf {
        self.compartment_dir(id).join("passphrase_wrapped_key.enc")
    }

    pub fn audit_log_path(&self) -> PathBuf {
        self.base_dir.join("audit.log")
    }

    pub fn audit_db_path(&self) -> PathBuf {
        self.base_dir.join("audit.db")
    }

    pub fn audit_key_path(&self) -> PathBuf {
        self.base_dir.join("audit.key")
    }

    pub fn biometric_enrollment_path(&self) -> PathBuf {
        self.base_dir.join("biometric_enrollment.json")
    }

    pub fn begin_operation(
        &self,
        spec: PendingOperationSpec,
        subject: Option<String>,
    ) -> Result<OperationGuard, std::io::Error> {
        crate::operations::begin_operation(&self.base_dir, spec, subject)
    }

    /// Ensure a FileVault exists for the given compartment, creating if needed.
    pub fn ensure_vault(&self, id: usize) {
        let mut vaults = self.vaults.lock();
        vaults.entry(id).or_insert_with(|| {
            let vault_dir = self.compartment_dir(id);
            let config = VaultConfig {
                base_dir: vault_dir,
                tier1_file: "api_keys.json".into(),
                tier2_file: "vault.enc".into(),
            };
            FileVault::new(config)
        });
    }

    /// Execute a closure with the active compartment's vault for a session.
    #[must_use]
    pub fn with_active_vault_for<F, R>(&self, token: &str, f: F) -> Option<R>
    where
        F: FnOnce(&FileVault) -> R,
    {
        let id = self.active_compartment_id_for(token)?;
        let vaults = self.vaults.lock();
        let vault = vaults.get(&id)?;
        Some(f(vault))
    }

    /// Execute a closure with a specific compartment's vault.
    #[must_use]
    pub fn with_vault<F, R>(&self, id: usize, f: F) -> Option<R>
    where
        F: FnOnce(&FileVault) -> R,
    {
        let vaults = self.vaults.lock();
        let vault = vaults.get(&id)?;
        Some(f(vault))
    }

    /// Unlock a single compartment: load master key and register metadata.
    pub fn unlock_compartment(&self, id: usize, master_key: [u8; 32], meta: CompartmentMeta) {
        self.ensure_vault(id);
        let vaults = self.vaults.lock();
        if let Some(vault) = vaults.get(&id) {
            vault.load_master_key(master_key);
        }
        drop(vaults);
        self.unlocked.lock().insert(id, meta);
    }

    /// Unlock multiple compartments at once (cascading FIDO2).
    pub fn unlock_multiple(&self, compartments: &[(CompartmentMeta, [u8; 32])]) {
        for (meta, master_key) in compartments {
            self.ensure_vault(meta.id);
            let vaults = self.vaults.lock();
            if let Some(vault) = vaults.get(&meta.id) {
                vault.load_master_key(*master_key);
            }
            drop(vaults);
            self.unlocked.lock().insert(meta.id, meta.clone());
        }
    }

    // ── Unlock throttle ────────────────────────────────────────────

    /// Check whether an unlock attempt is currently allowed.
    /// Returns `Err(seconds_remaining)` if the caller must wait.
    pub fn check_unlock_throttle(&self) -> Result<(), u64> {
        let throttle = self.unlock_throttle.lock();
        if throttle.consecutive_failures < MAX_UNLOCK_ATTEMPTS {
            return Ok(());
        }
        if let Some(last) = throttle.last_failure {
            let exponent = (throttle.consecutive_failures - MAX_UNLOCK_ATTEMPTS).min(6);
            let cooldown = UNLOCK_COOLDOWN_BASE * 2u32.pow(exponent);
            let elapsed = last.elapsed();
            if elapsed < cooldown {
                return Err((cooldown - elapsed).as_secs() + 1);
            }
        }
        Ok(())
    }

    /// Record a failed unlock attempt for rate-limiting purposes.
    pub fn record_unlock_failure(&self) {
        let mut throttle = self.unlock_throttle.lock();
        throttle.consecutive_failures += 1;
        throttle.last_failure = Some(Instant::now());
    }

    /// Reset the unlock throttle after a successful authentication.
    pub fn reset_unlock_throttle(&self) {
        let mut throttle = self.unlock_throttle.lock();
        throttle.consecutive_failures = 0;
        throttle.last_failure = None;
    }

    pub fn issue_biometric_challenge(&self) -> ([u8; 16], [u8; 32], u64) {
        let mut id = [0u8; 16];
        let mut nonce = [0u8; 32];
        OsRng.fill_bytes(&mut id);
        OsRng.fill_bytes(&mut nonce);
        let expires_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + BIOMETRIC_CHALLENGE_TTL.as_secs();

        let mut challenges = self.biometric_challenges.lock();
        challenges.retain(|entry| entry.issued_at.elapsed() < BIOMETRIC_CHALLENGE_TTL);
        while challenges.len() >= MAX_BIOMETRIC_CHALLENGES {
            challenges.pop_front();
        }
        challenges.push_back(BiometricChallengeState {
            id,
            nonce,
            issued_at: Instant::now(),
        });
        (id, nonce, expires_at_unix)
    }

    pub fn consume_biometric_challenge(&self, id: &[u8; 16]) -> Option<[u8; 32]> {
        let mut challenges = self.biometric_challenges.lock();
        challenges.retain(|entry| entry.issued_at.elapsed() < BIOMETRIC_CHALLENGE_TTL);
        let index = challenges.iter().position(|entry| &entry.id == id)?;
        let entry = challenges.remove(index)?;
        Some(entry.nonce)
    }

    #[must_use]
    pub fn biometric_challenge_count(&self) -> usize {
        self.biometric_challenges.lock().len()
    }

    // ── Session management ──────────────────────────────────────────

    /// Create a new session token bound to the current unlocked state.
    ///
    /// Enforces a maximum session count ([`MAX_SESSIONS`]) and evicts
    /// expired sessions before allocating a new one.
    pub fn create_session(&self, preferred_active: Option<usize>) -> String {
        self.create_session_inner(preferred_active, None, SESSION_TTL)
            .0
    }

    pub fn create_capability_session(
        &self,
        preferred_active: Option<usize>,
        scopes: Vec<String>,
        ttl: Duration,
    ) -> (String, u64) {
        self.create_session_inner(preferred_active, Some(scopes), ttl)
    }

    fn create_session_inner(
        &self,
        preferred_active: Option<usize>,
        scopes: Option<Vec<String>>,
        ttl: Duration,
    ) -> (String, u64) {
        let active = preferred_active.or_else(|| self.default_active_compartment_id());
        let expires_at_unix = SystemTime::now()
            .checked_add(ttl)
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .unwrap_or_default()
            .as_secs();

        loop {
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let token = hex::encode(bytes);

            let mut sessions = self.sessions.lock();

            // Evict expired sessions.
            let now = Instant::now();
            sessions.retain(|_, s| now < s.expires_at);

            // If still at capacity, evict the oldest session.
            if sessions.len() >= MAX_SESSIONS {
                if let Some(oldest_key) = sessions
                    .iter()
                    .min_by_key(|(_, s)| s.created_at)
                    .map(|(k, _)| k.clone())
                {
                    sessions.remove(&oldest_key);
                }
            }

            if !sessions.contains_key(&token) {
                sessions.insert(
                    token.clone(),
                    SessionState {
                        active_compartment_id: active,
                        created_at: now,
                        expires_at: now + ttl,
                        last_activity: now,
                        scopes: scopes.clone(),
                    },
                );
                return (token, expires_at_unix);
            }
        }
    }

    /// Switch the active compartment for a session. Must be in the unlocked set.
    #[must_use = "check the Result for compartment switch errors"]
    pub fn switch_active_for(&self, token: &str, id: usize) -> Result<(), &'static str> {
        if !self.unlocked.lock().contains_key(&id) {
            return Err("compartment not unlocked");
        }

        let mut sessions = self.sessions.lock();
        let session_key = Self::session_key_for(&sessions, token).ok_or("invalid session")?;
        if let Some(session) = sessions.get_mut(&session_key) {
            session.active_compartment_id = Some(id);
        }
        Ok(())
    }

    /// List all currently unlocked compartments, sorted by threshold.
    pub fn unlocked_compartments(&self) -> Vec<CompartmentMeta> {
        let mut metas: Vec<CompartmentMeta> = self.unlocked.lock().values().cloned().collect();
        metas.sort_by_key(|m| m.threshold);
        metas
    }

    /// Highest threshold among currently unlocked compartments.
    #[must_use]
    pub fn max_unlocked_threshold(&self) -> Option<usize> {
        self.unlocked.lock().values().map(|m| m.threshold).max()
    }

    /// True if the vault is currently unlocked (any compartment).
    pub fn is_unlocked(&self) -> bool {
        !self.unlocked.lock().is_empty()
    }

    /// Remove a single compartment: zeroize its master key, remove from unlocked set,
    /// and switch active if it was the active one.
    pub fn remove_compartment(&self, id: usize) {
        // Zeroize master key in the vault
        let mut vaults = self.vaults.lock();
        if let Some(vault) = vaults.get(&id) {
            vault.zeroize_master_key();
        }
        vaults.remove(&id);
        drop(vaults);

        // Remove from unlocked set
        self.unlocked.lock().remove(&id);

        // Repoint any session that had this compartment selected.
        let new_active = self.default_active_compartment_id();
        let mut sessions = self.sessions.lock();
        for session in sessions.values_mut() {
            if session.active_compartment_id == Some(id) {
                session.active_compartment_id = new_active;
            }
        }
    }

    /// Lock all compartments and clear state.
    pub fn lock_all(&self) {
        let mut vaults = self.vaults.lock();
        for vault in vaults.values() {
            vault.zeroize_master_key();
        }
        vaults.clear();
        drop(vaults);
        self.unlocked.lock().clear();
        self.sessions.lock().clear();
        self.finish_locking();
    }

    /// Verify a candidate token against the stored session token.
    pub fn verify_token(&self, candidate: &str) -> bool {
        if self.is_locking() {
            return false;
        }
        let idle_lock_secs = self.runtime_policy.idle_lock_secs;
        let mut sessions = self.sessions.lock();
        let now = Instant::now();
        sessions.retain(|_, state| now < state.expires_at);
        let Some(session_key) = Self::session_key_for(&sessions, candidate) else {
            return false;
        };
        let Some(session) = sessions.get_mut(&session_key) else {
            return false;
        };
        if session.last_activity.elapsed() >= Duration::from_secs(idle_lock_secs) {
            sessions.remove(&session_key);
            return false;
        }
        session.last_activity = Instant::now();
        true
    }

    #[must_use]
    pub fn idle_lock_due(&self) -> bool {
        if self.is_locking() {
            return false;
        }
        self.idle_lock_due_after_drain()
    }

    #[must_use]
    pub fn idle_lock_due_after_drain(&self) -> bool {
        if !self.is_unlocked() {
            return false;
        }
        let idle_lock_secs = self.runtime_policy.idle_lock_secs;
        let mut sessions = self.sessions.lock();
        let now = Instant::now();
        sessions.retain(|_, state| now < state.expires_at);
        if sessions.is_empty() {
            return true;
        }
        sessions
            .values()
            .all(|state| state.last_activity.elapsed() >= Duration::from_secs(idle_lock_secs))
    }

    #[must_use]
    pub fn session_has_scope(&self, candidate: &str, scope: &str) -> bool {
        let sessions = self.sessions.lock();
        sessions
            .iter()
            .find(|(stored, _)| Self::token_matches(stored, candidate))
            .is_some_and(|(_, state)| {
                state
                    .scopes
                    .as_ref()
                    .is_none_or(|scopes| scopes.iter().any(|candidate| candidate == scope))
            })
    }

    #[must_use]
    pub fn session_is_full(&self, candidate: &str) -> bool {
        let sessions = self.sessions.lock();
        sessions
            .iter()
            .find(|(stored, _)| Self::token_matches(stored, candidate))
            .is_some_and(|(_, state)| state.scopes.is_none())
    }

    /// Invalidate all active sessions (e.g. after credential rotation).
    /// The vault remains unlocked — callers must re-authenticate to get a
    /// new session token.
    pub fn invalidate_all_sessions(&self) {
        self.sessions.lock().clear();
    }

    /// Revoke a session token.
    pub fn revoke_session(&self, candidate: &str) {
        let mut sessions = self.sessions.lock();
        if let Some(session_key) = Self::session_key_for(&sessions, candidate) {
            sessions.remove(&session_key);
        }
    }

    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.lock().len()
    }

    #[must_use]
    pub fn pending_operation_count(&self) -> usize {
        list_pending_operations(&self.base_dir)
            .map(|pending| pending.len())
            .unwrap_or(0)
    }

    #[must_use]
    pub fn startup_recovery_summary(&self) -> StartupRecoverySummary {
        *self.startup_recovery.lock()
    }

    pub fn set_startup_recovery_summary(&self, summary: StartupRecoverySummary) {
        *self.startup_recovery.lock() = summary;
    }

    /// Check if Sigillum has been initialized (marker file exists).
    pub fn is_initialized(&self) -> bool {
        self.base_dir.join(".initialized").exists()
    }

    /// Extract master keys from all unlocked compartment vaults.
    pub fn extract_all_master_keys(&self) -> Vec<(usize, Zeroizing<[u8; 32]>)> {
        let vaults = self.vaults.lock();
        let unlocked = self.unlocked.lock();
        unlocked
            .keys()
            .filter_map(|id| {
                vaults
                    .get(id)
                    .and_then(|vault| vault.extract_master_key().map(|mk| (*id, mk)))
            })
            .collect()
    }

    /// Extract master keys with their CompartmentMeta for all unlocked vaults.
    pub fn extract_all_master_keys_with_meta(&self) -> Vec<(CompartmentMeta, Zeroizing<[u8; 32]>)> {
        let vaults = self.vaults.lock();
        let unlocked = self.unlocked.lock();
        unlocked
            .iter()
            .filter_map(|(id, meta)| {
                vaults
                    .get(id)
                    .and_then(|vault| vault.extract_master_key().map(|mk| (meta.clone(), mk)))
            })
            .collect()
    }

    pub(crate) fn record_audit_event(
        &self,
        compartment_id: Option<usize>,
        spec: AuditEventSpec,
    ) -> Result<(), std::io::Error> {
        let created_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let event = StoredAuditEvent {
            created_at_unix,
            compartment_id,
            spec,
        };
        let path = self.audit_db_path();
        let (scope, key) = self.audit_chain_scope_and_key(compartment_id)?;
        crate::audit_db::append_event_chained(&path, &event, &scope, &key)
    }

    pub(crate) fn read_audit_events(
        &self,
        query: AuditQuery,
    ) -> Result<Vec<AuditEvent>, std::io::Error> {
        let path = self.audit_db_path();
        crate::audit_db::query_events(
            &path,
            &AuditQuery {
                tail: self.runtime_policy().audit_limit(Some(query.tail.max(1))),
                ..query
            },
        )
    }

    pub(crate) fn verify_audit_chain(
        &self,
        scope: &str,
    ) -> Result<sigillum_api::AuditVerifyReport, std::io::Error> {
        let key = self.audit_key_for_scope(scope)?;
        crate::audit_db::verify_chain(&self.audit_db_path(), scope, &key)
    }
}
