//! Admin sessions: a server-side store keyed by an opaque random token.
//!
//! There is no signed/stateless token here on purpose. A stateless token would
//! need a signing key, and a scheduler with no `SPKY_AUTH_SECRET` would have to
//! invent one per process — at which point the token is already only valid for
//! the lifetime of this process, exactly like a `HashMap` entry, but with more
//! machinery and no revocation. So: 256 bits of randomness, looked up in a map.
//!
//! The consequence, which is documented rather than worked around: sessions do
//! not survive a scheduler restart. Persisting operator sessions into the
//! tenant database would be the only alternative, and putting an auth artifact
//! into the very database whose outage you might be debugging is a bad trade.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

/// How a session was established. Surfaced to the UI so a break-glass session
/// can be visibly flagged: it means the roster was bypassed, which the person
/// looking at the screen should know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    /// Signed in as a user on the `_00_admin` roster.
    Roster,
    /// Signed in with `SPKY_ADMIN_PASSWORD`.
    Breakglass,
}

#[derive(Debug, Clone)]
pub struct Session {
    /// The `_00_admin` user record id, or `"breakglass"`.
    pub subject: String,
    /// Display name for the UI.
    pub label: String,
    pub mode: SessionMode,
    pub expires_at: Instant,
}

pub struct SessionStore {
    sessions: Mutex<HashMap<String, Session>>,
    ttl: Duration,
}

impl SessionStore {
    pub fn new(ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            ttl,
        })
    }

    /// Mint a session and return its bearer token.
    pub fn create(&self, subject: String, label: String, mode: SessionMode) -> String {
        // Two v4 UUIDs: ~242 bits of entropy from the same CSPRNG `rand`
        // would give us, without pulling in another dependency for it.
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let session = Session {
            subject,
            label,
            mode,
            expires_at: Instant::now() + self.ttl,
        };
        let mut map = self.sessions.lock().expect("session store poisoned");
        // Sweep here rather than on a timer: sessions are few, and this is the
        // only place the map grows.
        let now = Instant::now();
        map.retain(|_, s| s.expires_at > now);
        map.insert(token.clone(), session);
        token
    }

    /// Look up a live session, or `None` if unknown or expired.
    pub fn get(&self, token: &str) -> Option<Session> {
        let mut map = self.sessions.lock().expect("session store poisoned");
        match map.get(token) {
            Some(s) if s.expires_at > Instant::now() => Some(s.clone()),
            Some(_) => {
                map.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn revoke(&self, token: &str) {
        self.sessions
            .lock()
            .expect("session store poisoned")
            .remove(token);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sessions.lock().expect("session store poisoned").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_token_resolves_to_its_session() {
        let store = SessionStore::new(Duration::from_secs(60));
        let token = store.create("user:a".into(), "alice".into(), SessionMode::Roster);
        let s = store.get(&token).expect("session should be live");
        assert_eq!(s.subject, "user:a");
        assert_eq!(s.mode, SessionMode::Roster);
    }

    #[test]
    fn tokens_are_unique_per_session() {
        let store = SessionStore::new(Duration::from_secs(60));
        let a = store.create("user:a".into(), "a".into(), SessionMode::Roster);
        let b = store.create("user:a".into(), "a".into(), SessionMode::Roster);
        assert_ne!(a, b);
    }

    #[test]
    fn an_expired_session_is_rejected_and_dropped() {
        let store = SessionStore::new(Duration::from_millis(0));
        let token = store.create("user:a".into(), "a".into(), SessionMode::Roster);
        std::thread::sleep(Duration::from_millis(5));
        assert!(store.get(&token).is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn revoke_invalidates_immediately() {
        let store = SessionStore::new(Duration::from_secs(60));
        let token = store.create("user:a".into(), "a".into(), SessionMode::Roster);
        store.revoke(&token);
        assert!(store.get(&token).is_none());
    }

    #[test]
    fn an_unknown_token_is_rejected() {
        let store = SessionStore::new(Duration::from_secs(60));
        assert!(store.get("not-a-token").is_none());
    }
}
