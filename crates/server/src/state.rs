//! Shared application state.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use nekochat_gateway::Provider;
use nekochat_store::Store;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

/// Conversation id → cancellation token of its in-flight turn.
pub type RunningTurns = Arc<Mutex<HashMap<String, CancellationToken>>>;

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
}

impl AppState {
    pub fn new(config: Config, providers: Vec<Provider>, store: Store) -> AppState {
        AppState {
            store: Arc::new(store),
            http: reqwest::Client::new(),
            providers: Arc::new(providers),
            config: Arc::new(config),
            sessions: Sessions::default(),
            running: RunningTurns::default(),
        }
    }

    pub fn provider(&self, key: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.key == key)
    }
}
