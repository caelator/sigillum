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
//! All mutable interior state is wrapped in [`ResilientMutex`], which transparently
//! recovers from mutex poisoning so the daemon remains available even if a thread
//! panics while holding a lock.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::RngCore;
use rand::rngs::OsRng;
use sigillum_api::AuditEvent;
use sigillum_core::{FileVault, VaultConfig, VaultLifecycle, recover_snapshot_restore};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::CompartmentMeta;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;

use crate::audit_db::AuditQuery;
use crate::audit_log::{AuditEventSpec, StoredAuditEvent};
use crate::operations::{OperationGuard, PendingOperationSpec, list_pending_operations};
use crate::policy::RuntimePolicy;

/// A resilient wrapper around `std::sync::Mutex<T>` that automatically recovers from poisoning.
///
/// # Why This Exists
///
/// In Sigillum's design, the mutex-protected data structures (vaults, unlocked compartments,
/// sessions, throttle state, and recovery summaries) are always structurally valid and
/// logically recoverable. A panic within a critical section should not permanently brick
/// the daemon by poisoning the mutex.
///
/// This wrapper implements the pattern `lock().unwrap_or_else(|e| e.into_inner())` at the
/// type level, ensuring that:
/// - All lock acquisitions transparently recover from poisoning
/// - Callers simply use `.lock()` without error handling
/// - The daemon remains operational even if a thread panics while holding the lock
///
/// The protected data remains accessible because:
/// - If a panic occurred, the data was left in some state (not corrupted)
/// - The calling code can proceed with the current state or implement recovery logic
/// - Permanently blocking access would be worse than continuing with the protected value
///
/// This is a conscious design choice for a security daemon where availability matters.
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

    /// Acquire the lock, returning the guarded value.
    ///
    /// If the previous holder panicked and poisoned the lock, this automatically
    /// recovers by calling `into_inner()` on the poison error, making the data
    /// accessible again.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<T: fmt::Debug> fmt::Debug for ResilientMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.lock() {
            Ok(guard) => f
                .debug_struct("ResilientMutex")
                .field("value", &*guard)
                .finish(),
            Err(e) => f
                .debug_struct("ResilientMutex")
                .field("value", e.get_ref())
                .finish(),
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
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug)]
struct SessionState {
    active_compartment_id: Option<usize>,
    created_at: Instant,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            active_compartment_id: None,
            created_at: Instant::now(),
        }
    }
}

/// Tracks failed authentication attempts for rate limiting.
#[derive(Debug, Default)]
struct UnlockThrottle {
    consecutive_failures: u32,
    last_failure: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// Rate limiter for failed unlock attempts.
    unlock_throttle: ResilientMutex<UnlockThrottle>,
    /// Startup-time reconciliation summary for observability.
    startup_recovery: ResilientMutex<StartupRecoverySummary>,
}

impl AppState {
    pub fn new(base_dir: PathBuf) -> Self {
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

        Self {
            fido2,
            base_dir,
            started_at_unix,
            http_client: reqwest::Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .timeout(HTTP_REQUEST_TIMEOUT)
                .build()
                .expect("daemon HTTP client should build"),
            runtime_policy: RuntimePolicy::from_env(),
            vaults: ResilientMutex::new(HashMap::new()),
            unlocked: ResilientMutex::new(HashMap::new()),
            sessions: ResilientMutex::new(HashMap::new()),
            operation_lock: AsyncMutex::new(()),
            unlock_throttle: ResilientMutex::new(UnlockThrottle::default()),
            startup_recovery: ResilientMutex::new(StartupRecoverySummary::default()),
        }
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

    // ── Session management ──────────────────────────────────────────

    /// Create a new session token bound to the current unlocked state.
    ///
    /// Enforces a maximum session count ([`MAX_SESSIONS`]) and evicts
    /// expired sessions before allocating a new one.
    pub fn create_session(&self, preferred_active: Option<usize>) -> String {
        let active = preferred_active.or_else(|| self.default_active_compartment_id());

        loop {
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let token = hex::encode(bytes);

            let mut sessions = self.sessions.lock();

            // Evict expired sessions.
            sessions.retain(|_, s| s.created_at.elapsed() < SESSION_TTL);

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
                        created_at: Instant::now(),
                    },
                );
                return token;
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
    }

    /// Verify a candidate token against the stored session token.
    pub fn verify_token(&self, candidate: &str) -> bool {
        let sessions = self.sessions.lock();
        sessions.iter().any(|(stored, state)| {
            state.created_at.elapsed() < SESSION_TTL && Self::token_matches(stored, candidate)
        })
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
        crate::audit_db::append_event(&path, &event)
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
}

fn stash_snapshot_placeholder_ops(base_dir: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    let rollback = snapshot_temp_path(base_dir, "rollback");
    if !rollback.exists() || !snapshot_placeholder_dir(base_dir)? {
        return Ok(None);
    }

    let ops_dir = base_dir.join(".ops");
    if !ops_dir.exists() {
        return Ok(None);
    }

    let preserved_ops = snapshot_temp_path(base_dir, "ops-preserved");
    if preserved_ops.exists() {
        std::fs::remove_dir_all(&preserved_ops)?;
    }
    std::fs::rename(&ops_dir, &preserved_ops)?;
    std::fs::remove_dir_all(base_dir)?;
    Ok(Some(preserved_ops))
}

fn restore_stashed_ops_dir(base_dir: &Path, preserved_ops: &Path) -> Result<(), std::io::Error> {
    if !preserved_ops.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(base_dir)?;
    let target = base_dir.join(".ops");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::rename(preserved_ops, target)
}

fn snapshot_placeholder_dir(base_dir: &Path) -> Result<bool, std::io::Error> {
    let mut entries = match std::fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };

    entries.try_fold(true, |is_placeholder, entry| {
        let entry = entry?;
        Ok(is_placeholder && entry.file_name() == ".ops")
    })
}

fn snapshot_temp_path(base_dir: &Path, suffix: &str) -> PathBuf {
    let parent = base_dir.parent().unwrap_or(Path::new("."));
    let name = base_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sigillum".into());
    parent.join(format!(".{name}.{suffix}"))
}

pub(crate) fn recover_compartment_replacements(base_dir: &Path) -> Result<(), std::io::Error> {
    let compartments_dir = base_dir.join("compartments");
    let entries = match std::fs::read_dir(&compartments_dir) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    let mut compartment_ids = std::collections::BTreeSet::new();
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let candidate = name
            .split('.')
            .next()
            .and_then(|prefix| prefix.parse::<usize>().ok());
        if let Some(id) = candidate {
            compartment_ids.insert(id);
        }
    }

    for id in compartment_ids {
        let live = compartments_dir.join(id.to_string());
        let replacement = live.with_extension("replacing");
        let rollback = live.with_extension("rollback");
        if live.exists() {
            if rollback.exists() {
                std::fs::remove_dir_all(&rollback)?;
            }
            if replacement.exists() {
                std::fs::remove_dir_all(&replacement)?;
            }
            continue;
        }
        if rollback.exists() {
            if replacement.exists() {
                let _ = std::fs::remove_dir_all(&replacement);
            }
            std::fs::rename(&rollback, &live)?;
            continue;
        }
        if replacement.exists() {
            std::fs::rename(&replacement, &live)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn meta(id: usize, threshold: usize, label: &str) -> CompartmentMeta {
        CompartmentMeta {
            id,
            label: label.into(),
            threshold,
            passphrase_mode: None,
        }
    }

    #[test]
    fn sessions_track_active_compartments_independently() {
        let dir = TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf());

        state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

        let session_a = state.create_session(Some(0));
        let session_b = state.create_session(Some(0));

        state.switch_active_for(&session_a, 1).unwrap();

        assert_eq!(state.active_compartment_id_for(&session_a), Some(1));
        assert_eq!(state.active_compartment_id_for(&session_b), Some(0));
    }

    #[test]
    fn removing_active_compartment_repoints_sessions() {
        let dir = TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf());

        state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

        let session = state.create_session(Some(1));
        state.remove_compartment(1);

        assert_eq!(state.active_compartment_id_for(&session), Some(0));
    }

    #[test]
    fn lock_all_clears_sessions_and_vault_instances() {
        let dir = TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf());

        state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
        let session = state.create_session(Some(0));

        assert!(state.verify_token(&session));
        assert!(state.with_vault(0, |_| true).is_some());

        state.lock_all();

        assert!(!state.verify_token(&session));
        assert!(state.with_vault(0, |_| true).is_none());
        assert!(!state.is_unlocked());
    }

    #[test]
    fn audit_log_roundtrip_and_limit() {
        let dir = TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf());

        state
            .record_audit_event(
                Some(0),
                AuditEventSpec::UnlockPassphrase {
                    compartment_ids: vec![0],
                    count: 1,
                },
            )
            .unwrap();
        state
            .record_audit_event(
                Some(0),
                AuditEventSpec::SecretSet {
                    key: "db_pass".into(),
                },
            )
            .unwrap();
        state
            .record_audit_event(
                None,
                AuditEventSpec::SnapshotExport {
                    file_count: 4,
                    total_bytes: 128,
                },
            )
            .unwrap();

        let events = state
            .read_audit_events(AuditQuery {
                tail: 2,
                kind: None,
                since: None,
                key: None,
            })
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "snapshot.export");
        assert_eq!(events[1].kind, "secret.set");
        assert_eq!(events[0].details["total_bytes"], serde_json::json!(128));
    }

    #[test]
    fn startup_recovery_summary_defaults_to_zero() {
        let dir = TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf());

        assert_eq!(
            state.startup_recovery_summary(),
            StartupRecoverySummary::default()
        );
    }
}
