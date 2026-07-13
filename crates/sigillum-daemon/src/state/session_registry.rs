//! Active daemon sessions and the narrowly-scoped predecessor used for Lock.
//!
//! A compartment switch rotates the bearer token. The immediately preceding
//! full-session token remains usable only for process-global Lock while the
//! response carrying its successor may still be in flight. Active sessions and
//! predecessor tombstones share one mutex-owned registry so rotation, cleanup,
//! and namespace uniqueness are atomic.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::RngCore;
use rand::rngs::OsRng;
use subtle::ConstantTimeEq;

pub(super) const MAX_SESSIONS: usize = 64;
const MAX_RETIRED_LOCK_TOKENS: usize = MAX_SESSIONS;
/// Maximum time a rotated full-session predecessor remains usable for Lock.
///
/// The browser allows a compartment-switch mutation 120 seconds, then gives
/// its fail-closed Lock fallback another 10 seconds. The extra 50-second
/// margin covers scheduling and transport delay without turning the retired
/// token into a general-purpose session. The effective deadline is still
/// capped by the source session's original TTL and idle deadline.
const RETIRED_LOCK_GRACE: Duration = Duration::from_secs(180);

#[derive(Clone, Debug)]
pub(super) struct SessionState {
    pub(super) active_compartment_id: Option<usize>,
    pub(super) created_at: Instant,
    pub(super) expires_at: Instant,
    pub(super) last_activity: Instant,
    pub(super) scopes: Option<Vec<String>>,
    lineage_id: [u8; 32],
}

#[derive(Clone, Debug)]
struct RetiredLockToken {
    token: String,
    lineage_id: [u8; 32],
    expires_at: Instant,
}

#[derive(Debug, Default)]
pub(super) struct SessionRegistry {
    pub(super) active: HashMap<String, SessionState>,
    retired_for_lock: VecDeque<RetiredLockToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LockTokenAuthorization {
    FullOrRetired,
    Capability,
    Invalid,
}

impl SessionRegistry {
    fn token_matches(stored: &str, candidate: &str) -> bool {
        let a = stored.as_bytes();
        let b = candidate.as_bytes();
        a.len() == b.len() && a.ct_eq(b).into()
    }

    fn active_key_for(&self, candidate: &str) -> Option<String> {
        self.active
            .keys()
            .find(|stored| Self::token_matches(stored, candidate))
            .cloned()
    }

    fn random_token() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    }

    fn random_lineage_id() -> [u8; 32] {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        bytes
    }

    fn prune(&mut self, now: Instant, idle_timeout: Duration, remove_idle_sessions: bool) {
        self.active.retain(|_, session| {
            now < session.expires_at
                && (!remove_idle_sessions
                    || now.saturating_duration_since(session.last_activity) < idle_timeout)
        });

        // A tombstone is useful only while a non-idle full-session successor
        // in the same lineage is still live. This also cascades expiry, idle,
        // capacity, revoke, and explicit invalidation cleanup.
        let live_full_lineages = self
            .active
            .values()
            .filter(|session| {
                session.scopes.is_none()
                    && now < session.expires_at
                    && now.saturating_duration_since(session.last_activity) < idle_timeout
            })
            .map(|session| session.lineage_id)
            .collect::<HashSet<_>>();
        self.retired_for_lock.retain(|entry| {
            now < entry.expires_at && live_full_lineages.contains(&entry.lineage_id)
        });
    }

    pub(super) fn active_compartment_id_for(&self, token: &str) -> Option<Option<usize>> {
        self.active
            .iter()
            .find(|(stored, _)| Self::token_matches(stored, token))
            .map(|(_, session)| session.active_compartment_id)
    }

    pub(super) fn create(
        &mut self,
        preferred_active: Option<usize>,
        scopes: Option<Vec<String>>,
        ttl: Duration,
        idle_timeout: Duration,
    ) -> (String, u64) {
        let expires_at_unix = SystemTime::now()
            .checked_add(ttl)
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .unwrap_or_default()
            .as_secs();
        let lineage_id = Self::random_lineage_id();

        loop {
            let now = Instant::now();
            self.prune(now, idle_timeout, true);
            if self.active.len() >= MAX_SESSIONS {
                if let Some(oldest_key) = self
                    .active
                    .iter()
                    .min_by_key(|(_, session)| session.created_at)
                    .map(|(token, _)| token.clone())
                {
                    self.active.remove(&oldest_key);
                    self.prune(now, idle_timeout, false);
                }
            }

            let token = Self::random_token();
            if self.active.contains_key(&token)
                || self
                    .retired_for_lock
                    .iter()
                    .any(|entry| entry.token == token)
            {
                continue;
            }
            self.active.insert(
                token.clone(),
                SessionState {
                    active_compartment_id: preferred_active,
                    created_at: now,
                    expires_at: now + ttl,
                    last_activity: now,
                    scopes,
                    lineage_id,
                },
            );
            return (token, expires_at_unix);
        }
    }

    pub(super) fn switch_active_for(&mut self, token: &str, id: usize) -> Result<(), &'static str> {
        let session_key = self.active_key_for(token).ok_or("invalid session")?;
        self.active
            .get_mut(&session_key)
            .ok_or("invalid session")?
            .active_compartment_id = Some(id);
        Ok(())
    }

    pub(super) fn rotate_active_for(
        &mut self,
        token: &str,
        id: usize,
        idle_timeout: Duration,
    ) -> Result<String, &'static str> {
        let now = Instant::now();
        self.prune(now, idle_timeout, true);
        let session_key = self.active_key_for(token).ok_or("invalid session")?;
        let mut session = self.active.remove(&session_key).ok_or("invalid session")?;
        session.active_compartment_id = Some(id);

        let idle_deadline = session
            .last_activity
            .checked_add(idle_timeout)
            .unwrap_or(session.expires_at);
        let grace_deadline = now
            .checked_add(RETIRED_LOCK_GRACE)
            .unwrap_or(session.expires_at);
        let predecessor_expiry = session.expires_at.min(idle_deadline).min(grace_deadline);

        // Keep exactly the immediate predecessor for a lineage. Capability
        // sessions never gain Lock authority, even if this state primitive is
        // reused outside the full-session switch endpoint.
        self.retired_for_lock
            .retain(|entry| entry.lineage_id != session.lineage_id);

        loop {
            let replacement = Self::random_token();
            if Self::token_matches(&session_key, &replacement)
                || self.active.contains_key(&replacement)
                || self
                    .retired_for_lock
                    .iter()
                    .any(|entry| entry.token == replacement)
            {
                continue;
            }
            let lineage_id = session.lineage_id;
            let was_full_session = session.scopes.is_none();
            self.active.insert(replacement.clone(), session);
            if was_full_session && predecessor_expiry > now {
                while self.retired_for_lock.len() >= MAX_RETIRED_LOCK_TOKENS {
                    self.retired_for_lock.pop_front();
                }
                self.retired_for_lock.push_back(RetiredLockToken {
                    token: session_key,
                    lineage_id,
                    expires_at: predecessor_expiry,
                });
            }
            return Ok(replacement);
        }
    }

    pub(super) fn repoint_compartment(&mut self, removed: usize, replacement: Option<usize>) {
        for session in self.active.values_mut() {
            if session.active_compartment_id == Some(removed) {
                session.active_compartment_id = replacement;
            }
        }
    }

    pub(super) fn verify(&mut self, candidate: &str, idle_timeout: Duration) -> bool {
        let now = Instant::now();
        self.prune(now, idle_timeout, true);
        let Some(session_key) = self.active_key_for(candidate) else {
            return false;
        };
        let Some(session) = self.active.get_mut(&session_key) else {
            return false;
        };
        session.last_activity = now;
        true
    }

    pub(super) fn authorize_for_lock(
        &mut self,
        candidate: &str,
        idle_timeout: Duration,
    ) -> LockTokenAuthorization {
        let now = Instant::now();
        self.prune(now, idle_timeout, true);
        if let Some(session_key) = self.active_key_for(candidate) {
            let Some(session) = self.active.get_mut(&session_key) else {
                return LockTokenAuthorization::Invalid;
            };
            if session.scopes.is_some() {
                return LockTokenAuthorization::Capability;
            }
            session.last_activity = now;
            return LockTokenAuthorization::FullOrRetired;
        }

        // Scan the complete bounded tombstone set. Every comparison is
        // constant-time for equal-length bearer tokens, and no match returns
        // early based on its position.
        let mut matched = false;
        for entry in &self.retired_for_lock {
            matched |= Self::token_matches(&entry.token, candidate);
        }
        if matched {
            LockTokenAuthorization::FullOrRetired
        } else {
            LockTokenAuthorization::Invalid
        }
    }

    #[cfg(test)]
    pub(super) fn verify_for_lock(&mut self, candidate: &str, idle_timeout: Duration) -> bool {
        self.authorize_for_lock(candidate, idle_timeout) == LockTokenAuthorization::FullOrRetired
    }

    pub(super) fn idle_lock_due(&mut self, idle_timeout: Duration) -> bool {
        let now = Instant::now();
        self.prune(now, idle_timeout, false);
        self.active.is_empty()
            || self
                .active
                .values()
                .all(|session| now.saturating_duration_since(session.last_activity) >= idle_timeout)
    }

    pub(super) fn has_scope(&self, candidate: &str, scope: &str) -> bool {
        self.active
            .iter()
            .find(|(stored, _)| Self::token_matches(stored, candidate))
            .is_some_and(|(_, session)| {
                session
                    .scopes
                    .as_ref()
                    .is_none_or(|scopes| scopes.iter().any(|candidate| candidate == scope))
            })
    }

    pub(super) fn is_full(&self, candidate: &str) -> bool {
        self.active
            .iter()
            .find(|(stored, _)| Self::token_matches(stored, candidate))
            .is_some_and(|(_, session)| session.scopes.is_none())
    }

    pub(super) fn revoke(&mut self, candidate: &str) {
        let Some(session_key) = self.active_key_for(candidate) else {
            return;
        };
        if let Some(session) = self.active.remove(&session_key) {
            self.retired_for_lock
                .retain(|entry| entry.lineage_id != session.lineage_id);
        }
    }

    pub(super) fn clear(&mut self) {
        self.active.clear();
        self.retired_for_lock.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.active.len()
    }

    #[cfg(test)]
    pub(super) fn retired_len(&self) -> usize {
        self.retired_for_lock.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_lock_grace_is_bounded_and_enforced_while_successor_is_live() {
        assert_eq!(RETIRED_LOCK_GRACE, Duration::from_secs(180));

        let idle_timeout = Duration::from_secs(60 * 60);
        let mut registry = SessionRegistry::default();
        let (predecessor, _) =
            registry.create(Some(0), None, Duration::from_secs(60 * 60), idle_timeout);
        let successor = registry
            .rotate_active_for(&predecessor, 1, idle_timeout)
            .unwrap();
        assert!(registry.verify_for_lock(&predecessor, idle_timeout));
        assert!(registry.verify(&successor, idle_timeout));

        registry.retired_for_lock.front_mut().unwrap().expires_at =
            Instant::now() - Duration::from_secs(1);

        assert!(
            !registry.verify_for_lock(&predecessor, idle_timeout),
            "expired predecessor must fail even while its successor remains live"
        );
        assert!(registry.verify(&successor, idle_timeout));
        assert_eq!(registry.retired_len(), 0);
    }
}
