//! Core-owned reply progress. Observers never own or cancel the upstream task.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use futures_util::{StreamExt, stream};
use nekochat_core::{Metrics, Usage};
use nekochat_resources::ResourceChunk;
use nekochat_store::{Node, NodeStatus, new_id};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::chat::{TurnEvent, TurnStream, sticker_markdown};

const OBSERVER_BUFFER: usize = 64;
type Update = Option<(u64, Arc<TurnEvent>)>;

#[derive(Default)]
struct Progress {
    usage: Option<Usage>,
    first: Option<Instant>,
    last: Option<Instant>,
    first_reasoning: Option<Instant>,
    first_text: Option<Instant>,
}

struct LiveData {
    node: Option<Node>,
    finalizing: Option<Arc<Node>>,
    start: Option<Arc<TurnEvent>>,
    terminal: Option<Arc<TurnEvent>>,
    progress: Progress,
    started: Instant,
    revision: u64,
    closed: bool,
}

pub struct Generation {
    pub id: String,
    cancel: CancellationToken,
    discard: AtomicBool,
    data: Mutex<LiveData>,
    updates: broadcast::Sender<Update>,
}

impl Generation {
    pub(crate) fn new() -> Self {
        let (updates, _) = broadcast::channel(OBSERVER_BUFFER);
        Self {
            id: new_id(),
            cancel: CancellationToken::new(),
            discard: AtomicBool::new(false),
            data: Mutex::new(LiveData {
                node: None,
                finalizing: None,
                start: None,
                terminal: None,
                progress: Progress::default(),
                started: Instant::now(),
                revision: 0,
                closed: false,
            }),
            updates,
        }
    }

    fn lock(&self) -> MutexGuard<'_, LiveData> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub(crate) fn stop(&self) {
        self.cancel.cancel();
    }

    pub(crate) fn discard(&self) {
        self.discard.store(true, Ordering::Release);
        self.cancel.cancel();
    }

    pub(crate) fn discarded(&self) -> bool {
        self.discard.load(Ordering::Acquire)
    }

    pub(crate) async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    pub(crate) fn start(&self, node: Node, event: TurnEvent) {
        let mut data = self.lock();
        data.started = Instant::now();
        data.node = Some(node);
        let event = Arc::new(event);
        data.start = Some(event.clone());
        self.publish(&mut data, event);
    }

    fn publish(&self, data: &mut LiveData, event: Arc<TurnEvent>) {
        data.revision += 1;
        // Zero subscribers is normal; it does not affect generation or persistence.
        let _ = self.updates.send(Some((data.revision, event)));
    }

    pub(crate) fn reasoning(&self, text: String) {
        let mut data = self.lock();
        let now = Instant::now();
        data.progress.first_reasoning.get_or_insert(now);
        data.progress.first.get_or_insert(now);
        data.progress.last = Some(now);
        if let Some(node) = &mut data.node {
            node.reasoning
                .get_or_insert_with(String::new)
                .push_str(&text);
        }
        self.publish(&mut data, Arc::new(TurnEvent::Reasoning(text)));
    }

    pub(crate) fn text_arrived(&self) {
        let mut data = self.lock();
        let now = Instant::now();
        data.progress.first_text.get_or_insert(now);
        data.progress.first.get_or_insert(now);
        data.progress.last = Some(now);
    }

    pub(crate) fn chunk(&self, chunk: ResourceChunk) {
        let mut data = self.lock();
        if let Some(node) = &mut data.node {
            match &chunk {
                ResourceChunk::Text(text) => node.content.push_str(text),
                ResourceChunk::Sticker(resource) => {
                    node.content.push_str(&sticker_markdown(resource))
                }
            }
        }
        self.publish(&mut data, Arc::new(TurnEvent::Chunk(chunk)));
    }

    pub(crate) fn usage(&self, usage: Usage) {
        self.lock().progress.usage = Some(usage);
    }

    /// Edits the accumulated reply once, before immutable finalization and persistence.
    pub(crate) fn finalize_content(&self, finalize: impl FnOnce(&mut String)) {
        if let Some(node) = &mut self.lock().node {
            finalize(&mut node.content);
        }
    }

    /// Moves the accumulated strings into a shared immutable node, also visible while saving.
    pub(crate) fn finalize(
        &self,
        status: NodeStatus,
        model_id: String,
        model_label: String,
        effort: Option<String>,
    ) -> (Option<Arc<Node>>, Option<Usage>) {
        let mut data = self.lock();
        let end = Instant::now();
        let metrics = data.progress.metrics(data.started, end);
        let node = data.node.take().map(|mut node| {
            node.status = status;
            node.metrics = Some(metrics);
            node.reasoning = node.reasoning.filter(|text| !text.is_empty());
            node.model_id = Some(model_id);
            node.model_label = Some(model_label);
            node.effort = effort;
            Arc::new(node)
        });
        let keep = node.as_ref().is_some_and(|node| {
            node.status != NodeStatus::Error || !node.content.is_empty() || node.reasoning.is_some()
        });
        data.finalizing = node.clone();
        (if keep { node } else { None }, data.progress.usage.take())
    }

    pub(crate) fn finish(&self, event: TurnEvent) {
        let mut data = self.lock();
        let event = Arc::new(event);
        data.terminal = Some(event.clone());
        self.publish(&mut data, event);
        data.closed = true;
        let _ = self.updates.send(None);
    }

    pub(crate) fn close(&self) {
        let mut data = self.lock();
        if !data.closed {
            data.closed = true;
            let _ = self.updates.send(None);
        }
    }

    fn snapshot(data: &LiveData) -> Option<Arc<TurnEvent>> {
        let node = data
            .finalizing
            .clone()
            .or_else(|| data.node.as_ref().map(|node| Arc::new(node.clone())))?;
        Some(Arc::new(TurnEvent::Snapshot {
            node,
            start: data.start.clone(),
            elapsed_ms: millis(data.started.elapsed()),
            reasoning_start_ms: data
                .progress
                .first_reasoning
                .map(|at| millis(at.saturating_duration_since(data.started))),
            reasoning_end_ms: data
                .progress
                .first_text
                .map(|at| millis(at.saturating_duration_since(data.started))),
        }))
    }

    fn cursor(self: &Arc<Self>, resume: bool) -> Observer {
        // A fresh receiver and snapshot share the publication lock, including after lag.
        let data = self.lock();
        let receiver = self.updates.subscribe();
        let pending = (resume && !self.discarded())
            .then(|| Self::snapshot(&data))
            .flatten();
        Observer {
            generation: self.clone(),
            receiver,
            revision: if pending.is_some() { data.revision } else { 0 },
            pending,
            terminal: data.terminal.clone(),
            resume,
            done: data.closed,
        }
    }

    pub(crate) fn observe(self: &Arc<Self>, resume: bool) -> TurnStream {
        let cursor = self.cursor(resume);
        stream::unfold(cursor, |mut cursor| async move {
            if let Some(event) = cursor.pending.take() {
                return Some((event, cursor));
            }
            if let Some(event) = cursor.terminal.take() {
                return Some((event, cursor));
            }
            if cursor.done {
                return None;
            }
            loop {
                match cursor.receiver.recv().await {
                    Ok(Some((revision, event))) => {
                        if revision <= cursor.revision {
                            continue;
                        }
                        if cursor.resume && matches!(event.as_ref(), TurnEvent::Start { .. }) {
                            cursor = cursor.generation.cursor(true);
                            if let Some(snapshot) = cursor.pending.take() {
                                return Some((snapshot, cursor));
                            }
                            if let Some(terminal) = cursor.terminal.take() {
                                return Some((terminal, cursor));
                            }
                            if cursor.done {
                                return None;
                            }
                            continue;
                        }
                        cursor.revision = revision;
                        cursor.done =
                            matches!(event.as_ref(), TurnEvent::Done(_) | TurnEvent::Error { .. });
                        return Some((event, cursor));
                    }
                    Ok(None) | Err(broadcast::error::RecvError::Closed) => return None,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        cursor = cursor.generation.cursor(true);
                        if let Some(snapshot) = cursor.pending.take() {
                            return Some((snapshot, cursor));
                        }
                        if let Some(terminal) = cursor.terminal.take() {
                            return Some((terminal, cursor));
                        }
                        if cursor.done {
                            return None;
                        }
                    }
                }
            }
        })
        .boxed()
    }
}

struct Observer {
    generation: Arc<Generation>,
    receiver: broadcast::Receiver<Update>,
    revision: u64,
    pending: Option<Arc<TurnEvent>>,
    terminal: Option<Arc<TurnEvent>>,
    resume: bool,
    done: bool,
}

impl Progress {
    fn metrics(&self, started: Instant, end: Instant) -> Metrics {
        let tokens_per_sec = match (&self.usage, self.first, self.last) {
            (Some(usage), Some(first), Some(last)) => {
                let span = last - first;
                (span.as_millis() > 0)
                    .then(|| (usage.output + usage.reasoning) as f64 / span.as_secs_f64())
            }
            _ => None,
        };
        Metrics {
            ttft_ms: self.first.map(|first| millis(first - started)),
            total_ms: millis(end - started),
            tokens_per_sec,
            usage: self.usage.clone().unwrap_or_default(),
            reasoning_ms: self.first_reasoning.and_then(|first| {
                self.first_text
                    .or(self.last)
                    .map(|end| millis(end.saturating_duration_since(first)))
            }),
        }
    }
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekochat_store::NodeRole;

    #[tokio::test]
    async fn lagging_observers_resynchronize_without_replaying_old_deltas() {
        let generation = Arc::new(Generation::new());
        let mut completion_lag = generation.observe(false);
        let reply = Node {
            id: generation.id.clone(),
            conversation_id: "conversation".into(),
            parent_id: Some("user".into()),
            role: NodeRole::Assistant,
            content: String::new(),
            attachments: Vec::new(),
            reasoning: None,
            model_id: None,
            model_label: None,
            effort: None,
            character_id: Some("role".into()),
            character_name: Some("Role".into()),
            character_avatar: None,
            status: NodeStatus::Interrupted,
            metrics: None,
            created_at: 1,
        };
        let user = Node {
            id: "user".into(),
            parent_id: None,
            role: NodeRole::User,
            content: "question".into(),
            character_id: None,
            character_name: None,
            character_avatar: None,
            status: NodeStatus::Complete,
            ..reply.clone()
        };
        generation.start(
            reply,
            TurnEvent::Start {
                user_node: Some(user),
                assistant_node_id: generation.id.clone(),
                character_id: "role".into(),
                character_name: "Role".into(),
                character_avatar: None,
            },
        );
        let mut observer = generation.observe(true);
        assert!(matches!(
            observer.next().await.unwrap().as_ref(),
            TurnEvent::Snapshot { .. }
        ));
        let count = OBSERVER_BUFFER * 2;
        for _ in 0..count {
            generation.chunk(ResourceChunk::Text("x".into()));
        }
        let snapshot = observer.next().await.unwrap();
        let TurnEvent::Snapshot { node, .. } = snapshot.as_ref() else {
            panic!("lag must resynchronize");
        };
        assert_eq!(node.content, "x".repeat(count));
        generation.chunk(ResourceChunk::Text("tail".into()));
        let next = tokio::time::timeout(std::time::Duration::from_secs(1), observer.next())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(next.as_ref(), TurnEvent::Chunk(ResourceChunk::Text(text)) if text == "tail")
        );
        generation.finalize_content(|content| {
            content.push_str("\n\n**参考来源**\n\n[1] [Source](<https://example.com/>)");
        });
        let (node, _) = generation.finalize(
            NodeStatus::Complete,
            "model".into(),
            "private-label".into(),
            None,
        );
        let node = node.unwrap();
        assert_eq!(node.content, format!("{}tail\n\n**参考来源**\n\n[1] [Source](<https://example.com/>)", "x".repeat(count)));
        let mut saving_observer = generation.observe(true);
        let snapshot = saving_observer.next().await.unwrap();
        let TurnEvent::Snapshot { node: saving_node, .. } = snapshot.as_ref() else {
            panic!("saving must expose the finalized source content");
        };
        assert!(Arc::ptr_eq(saving_node, &node));
        generation.finish(TurnEvent::Done(node));
        assert!(matches!(
            observer.next().await.unwrap().as_ref(),
            TurnEvent::Done(_)
        ));
        assert!(observer.next().await.is_none());
        let recovered = completion_lag.next().await.unwrap();
        let TurnEvent::Snapshot {
            node,
            start: Some(start),
            ..
        } = recovered.as_ref()
        else {
            panic!("terminal lag must first recover its conversation context");
        };
        let TurnEvent::Start {
            user_node: Some(user),
            ..
        } = start.as_ref()
        else {
            panic!("missing user parent");
        };
        assert_eq!(node.parent_id.as_deref(), Some(user.id.as_str()));
        assert_eq!(user.content, "question");
        assert!(matches!(
            completion_lag.next().await.unwrap().as_ref(),
            TurnEvent::Done(_)
        ));
        assert!(completion_lag.next().await.is_none());
    }
}
