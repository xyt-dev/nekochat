//! Bounded, account-scoped invalidations; token streaming remains on the turn observer.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use axum_extra::extract::CookieJar;
use futures_util::{StreamExt, stream, stream::BoxStream};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::accounts::{CurrentUser, http_observer};
use crate::api::ApiError;
use crate::state::AppState;

type Sender = broadcast::Sender<Arc<str>>;

#[derive(Clone, Default)]
pub(crate) struct AccountEvents(Arc<Mutex<HashMap<String, Weak<Sender>>>>);

impl AccountEvents {
    pub(crate) fn publish(&self, owner: &str, conversation_id: &str) {
        let sender = self.0.lock().unwrap_or_else(PoisonError::into_inner)
            .get(owner).and_then(Weak::upgrade);
        if let Some(sender) = sender {
            let _ = sender.send(Arc::from(conversation_id));
        }
    }

    fn observe(&self, owner: &str, cancellation: Arc<CancellationToken>)
        -> BoxStream<'static, Option<Arc<str>>>
    {
        let (sender, receiver) = {
            let mut accounts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            accounts.retain(|_, sender| sender.strong_count() != 0);
            let sender = accounts.get(owner).and_then(Weak::upgrade).unwrap_or_else(|| {
                let (sender, _) = broadcast::channel(64);
                let sender = Arc::new(sender);
                accounts.insert(owner.to_owned(), Arc::downgrade(&sender));
                sender
            });
            // Subscribe before exposing the initial authoritative resync.
            let receiver = sender.subscribe();
            (sender, receiver)
        };
        stream::unfold((sender, receiver, cancellation, true),
            |(sender, mut receiver, cancellation, initial)| async move {
                if cancellation.is_cancelled() {
                    return None;
                }
                let event = if initial {
                    None
                } else {
                    tokio::select! {
                        biased;
                        () = cancellation.cancelled() => return None,
                        update = receiver.recv() => match update {
                            Ok(id) => Some(id),
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                // A full resync supersedes the retained stale notifications.
                                receiver = sender.subscribe();
                                None
                            }
                            Err(broadcast::error::RecvError::Closed) => return None,
                        },
                    }
                };
                Some((event, (sender, receiver, cancellation, false)))
            },
        ).boxed()
    }
}

#[derive(Serialize)]
struct SyncEvent<'a> {
    conversation_id: Option<&'a str>,
}

pub(crate) async fn events(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    jar: CookieJar,
) -> Result<Response, ApiError> {
    let cancellation = http_observer(&state, &user, &jar).await?;
    // Each channel is selected by the authenticated owner before anything reaches SSE.
    let events = state.account_events.observe(&user.id, cancellation).map(|id| {
        Ok::<_, Infallible>(Event::default().event("sync").json_data(SyncEvent {
            conversation_id: id.as_deref(),
        }).expect("conversation IDs serialize as JSON strings"))
    });
    Ok(Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(1)))
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscription_precedes_initial_resync_and_filters_other_owners() {
        let events = AccountEvents::default();
        let mut alice = events.observe("alice", Arc::new(CancellationToken::new()));
        let mut bob = events.observe("bob", Arc::new(CancellationToken::new()));
        // Publish before polling the initial event: bootstrap must not open a loss window.
        events.publish("alice", "alice-conversation");
        events.publish("bob", "bob-conversation");
        assert_eq!(alice.next().await, Some(None));
        assert_eq!(alice.next().await.unwrap().as_deref(), Some("alice-conversation"));
        assert_eq!(bob.next().await, Some(None));
        assert_eq!(bob.next().await.unwrap().as_deref(), Some("bob-conversation"));
    }

    #[tokio::test]
    async fn a_lagging_observer_resyncs_and_resumes_without_replaying_stale_ids() {
        let events = AccountEvents::default();
        let mut observer = events.observe("alice", Arc::new(CancellationToken::new()));
        assert_eq!(observer.next().await, Some(None));
        for _ in 0..1024 {
            events.publish("alice", "old-conversation");
        }
        assert_eq!(observer.next().await, Some(None));
        events.publish("alice", "latest-conversation");
        assert_eq!(observer.next().await.unwrap().as_deref(), Some("latest-conversation"));
    }

    #[tokio::test]
    async fn revocation_wins_over_initial_and_queued_notifications() {
        let events = AccountEvents::default();
        let cancellation = Arc::new(CancellationToken::new());
        let mut observer = events.observe("alice", cancellation.clone());
        events.publish("alice", "private-conversation");
        cancellation.cancel();
        assert!(observer.next().await.is_none());
    }
}
