//! Shared application state.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nekochat_gateway::Provider;
use nekochat_store::Store;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

/// Conversation id → cancellation token of its in-flight turn.
pub type RunningTurns = Arc<Mutex<HashMap<String, CancellationToken>>>;

fn lock(running: &RunningTurns) -> MutexGuard<'_, HashMap<String, CancellationToken>> {
    // The map stays consistent even if a holder panicked.
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks a conversation as generating; dropping it (any exit path, panics included) unmarks it.
pub struct TurnGuard {
    running: RunningTurns,
    conv_id: String,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        lock(&self.running).remove(&self.conv_id);
    }
}

/// In-memory session tokens (hex of 32 random bytes) for the two password scopes.
#[derive(Clone, Default)]
pub struct Sessions {
    access: Arc<Mutex<HashSet<String>>>,
    admin: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Access,
    Admin,
}

impl Sessions {
    fn set(&self, scope: Scope) -> &Mutex<HashSet<String>> {
        match scope {
            Scope::Access => &self.access,
            Scope::Admin => &self.admin,
        }
    }

    /// Creates and remembers a new token for `scope`.
    pub fn issue(&self, scope: Scope) -> String {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("OS random source unavailable");
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        self.set(scope).lock().unwrap().insert(token.clone());
        token
    }

    pub fn contains(&self, scope: Scope, token: &str) -> bool {
        self.set(scope).lock().unwrap().contains(token)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub http: reqwest::Client,
    pub providers: Arc<Vec<Provider>>,
    pub config: Arc<Config>,
    pub sessions: Sessions,
    pub running: RunningTurns,
    pub resources: Arc<nekochat_resources::ResourceCatalog>,
    pub codex: Arc<crate::codex::CodexAccount>,
}

/// Upper bound for establishing an upstream connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Upper bound for silence on an upstream connection, between body chunks included.
pub const READ_TIMEOUT: Duration = Duration::from_secs(120);

impl AppState {
    pub fn new(config: Config, providers: Vec<Provider>, store: Store) -> AppState {
        AppState::with_timeouts(config, providers, store, CONNECT_TIMEOUT, READ_TIMEOUT)
    }

    /// Like `new`, with explicit upstream timeouts. No overall timeout: replies stream for long.
    pub fn with_timeouts(
        config: Config,
        mut providers: Vec<Provider>,
        store: Store,
        connect: Duration,
        read: Duration,
    ) -> AppState {
        let http = reqwest::Client::builder()
            .connect_timeout(connect)
            .read_timeout(read)
            .build()
            .expect("HTTP client with timeouts only");
        let resources = nekochat_resources::ResourceCatalog::load(&config.data_dir.join("skills"))
            .expect("loading validated local resource packs");
        let codex = Arc::new(crate::codex::CodexAccount::new(&config.data_dir));
        providers.push(crate::codex::provider());
        AppState {
            store: Arc::new(store),
            http,
            providers: Arc::new(providers),
            config: Arc::new(config),
            sessions: Sessions::default(),
            running: RunningTurns::default(),
            resources: Arc::new(resources),
            codex,
        }
    }

    pub fn provider(&self, key: &str) -> Option<&Provider> {
        if key == crate::codex::PROVIDER_KEY && !self.codex.connected() {
            return None;
        }
        self.providers.iter().find(|p| p.key == key)
    }

    /// Resolve a request-local credential; access tokens never enter persistent Provider metadata.
    pub async fn authenticated_provider(
        &self,
        key: &str,
    ) -> Result<Provider, crate::api::ApiError> {
        let mut provider = self
            .providers
            .iter()
            .find(|provider| provider.key == key)
            .cloned()
            .ok_or_else(crate::api::ApiError::not_found)?;
        if key == crate::codex::PROVIDER_KEY {
            provider.api_key = self
                .codex
                .access_token(&self.http)
                .await
                .map_err(|message| {
                    crate::api::ApiError::new(axum::http::StatusCode::BAD_GATEWAY, message)
                })?;
        }
        Ok(provider)
    }

    /// Claims `conv_id` for a new turn; `None` while another turn there is running.
    pub fn begin_turn(&self, conv_id: &str) -> Option<(TurnGuard, CancellationToken)> {
        let mut running = lock(&self.running);
        let Entry::Vacant(slot) = running.entry(conv_id.to_owned()) else {
            return None;
        };
        let token = slot.insert(CancellationToken::new()).clone();
        let guard = TurnGuard {
            running: self.running.clone(),
            conv_id: conv_id.to_owned(),
        };
        Some((guard, token))
    }

    /// Stops the conversation's running turn, if any; a cancelled turn persists nothing.
    pub fn cancel_turn(&self, conv_id: &str) {
        if let Some(token) = lock(&self.running).get(conv_id) {
            token.cancel();
        }
    }

    /// Whether a turn is generating in the conversation right now.
    pub fn turn_running(&self, conv_id: &str) -> bool {
        lock(&self.running).contains_key(conv_id)
    }
}
