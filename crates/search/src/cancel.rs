use crate::SearchError;
use std::{collections::HashMap, sync::{Arc, Mutex}, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

const CAPACITY: usize = 4096;
const RETENTION: Duration = Duration::from_secs(15 * 60);
#[derive(Default,Clone)]
pub(crate) struct Registry { inner: Arc<Mutex<HashMap<String, Entry>>> }
struct Entry { state: State, at: Instant }
enum State { Active(CancellationToken), Cancelled, Complete }
pub(crate) struct Guard { registry: Registry, id: String, pub token: CancellationToken }
impl Registry {
    pub fn register(&self, id: &str) -> Result<Guard,SearchError> {
        let mut entries = self.inner.lock().map_err(|_| SearchError::new(502,"request_registry_unavailable"))?;
        prune(&mut entries);
        if let Some(entry) = entries.get(id) {
            return Err(SearchError::new(409,match entry.state { State::Cancelled => "request_cancelled", _ => "duplicate_request_id" }));
        }
        if entries.len() >= CAPACITY { return Err(SearchError::new(429,"request_registry_full")); }
        let token = CancellationToken::new();
        entries.insert(id.to_owned(),Entry { state: State::Active(token.clone()), at: Instant::now() });
        Ok(Guard { registry: self.clone(), id: id.to_owned(), token })
    }
    pub fn cancel(&self, id: &str) -> Result<(),SearchError> {
        let mut entries = self.inner.lock().map_err(|_| SearchError::new(502,"request_registry_unavailable"))?;
        prune(&mut entries);
        if let Some(entry) = entries.get_mut(id) {
            if let State::Active(token) = &entry.state { token.cancel(); }
            entry.state = State::Cancelled;
            entry.at = Instant::now();
        } else {
            if entries.len() >= CAPACITY { return Err(SearchError::new(429,"request_registry_full")); }
            entries.insert(id.to_owned(),Entry { state: State::Cancelled, at: Instant::now() });
        }
        Ok(())
    }
}
fn prune(entries: &mut HashMap<String,Entry>) { entries.retain(|_, entry| matches!(entry.state,State::Active(_)) || entry.at.elapsed() < RETENTION); }
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.registry.inner.lock() {
            if let Some(entry) = entries.get_mut(&self.id) {
                if matches!(entry.state,State::Active(_)) { entry.state = State::Complete; entry.at = Instant::now(); }
            }
        }
    }
}
#[cfg(test)]
#[path = "cancel_tests.rs"]
mod tests;
