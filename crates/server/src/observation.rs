//! Revoking a login closes its HTTP observers, never the core generation.
use std::sync::{Arc, Mutex, PoisonError, Weak};
use tokio_util::sync::CancellationToken;

struct Observer {
    user_id: String,
    fingerprint: String,
    cancellation: Weak<CancellationToken>,
}

#[derive(Clone, Default)]
pub struct Observations(Arc<Mutex<Vec<Observer>>>);

impl Observations {
    pub(crate) fn register(&self, user_id: &str, fingerprint: String) -> Arc<CancellationToken> {
        let cancellation = Arc::new(CancellationToken::new());
        let mut observers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        observers.retain(|observer| observer.cancellation.strong_count() != 0);
        observers.push(Observer {
            user_id: user_id.to_owned(),
            fingerprint,
            cancellation: Arc::downgrade(&cancellation),
        });
        cancellation
    }

    pub(crate) fn revoke_session(&self, fingerprint: &str) {
        self.revoke(|observer| observer.fingerprint == fingerprint);
    }

    pub(crate) fn revoke_user(&self, user_id: &str) {
        self.revoke(|observer| observer.user_id == user_id);
    }

    fn revoke(&self, matches: impl Fn(&Observer) -> bool) {
        let mut observers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        observers.retain(|observer| {
            let Some(cancellation) = observer.cancellation.upgrade() else {
                return false;
            };
            if matches(observer) {
                cancellation.cancel();
                false
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_and_user_revocation_close_only_the_affected_observers() {
        let observations = Observations::default();
        let first = observations.register("alice", "first-session".into());
        let same_session = observations.register("alice", "first-session".into());
        let other_session = observations.register("alice", "other-session".into());
        let bob = observations.register("bob", "bob-session".into());
        observations.revoke_session("first-session");
        assert!(first.is_cancelled());
        assert!(same_session.is_cancelled());
        assert!(!other_session.is_cancelled());
        assert!(!bob.is_cancelled());
        observations.revoke_user("alice");
        assert!(other_session.is_cancelled());
        assert!(!bob.is_cancelled());
    }
}
