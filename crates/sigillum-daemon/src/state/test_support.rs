//! Test-only controls for deterministic session-idle assertions.

use std::time::{Duration, Instant};

use super::AppState;

impl AppState {
    pub(crate) fn backdate_session_activity(&self, candidate: &str, age: Duration) {
        let mut sessions = self.sessions.lock();
        let session_key =
            Self::session_key_for(&sessions, candidate).expect("test session should exist");
        sessions
            .get_mut(&session_key)
            .expect("test session should exist")
            .last_activity = Instant::now() - age;
    }

    pub(crate) fn session_activity_elapsed(&self, candidate: &str) -> Option<Duration> {
        let sessions = self.sessions.lock();
        let session_key = Self::session_key_for(&sessions, candidate)?;
        sessions
            .get(&session_key)
            .map(|session| session.last_activity.elapsed())
    }
}
