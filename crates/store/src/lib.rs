//! SQLite persistence for nekochat.

mod attachments;
mod catalog;
mod tree;
mod search_usage;
mod usage;
mod usage_stats;
mod users;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub use attachments::Attachment;
pub use catalog::{Character, CharacterSpec, ListedModel, ModelSpec, SettingKey};
pub use tree::{Conversation, DEFAULT_TITLE, Node, NodeRole, NodeStatus};
pub use usage::{UsagePurpose, UsageRecord};
pub use search_usage::SearchUsageRecord;
pub use usage_stats::{AllowanceWindow, ModelAllowance, ModelUsageStats, UsagePoint, UsageTotals};
pub use users::{PasswordStorage, User, UserPublic};

const SCHEMA: &str = include_str!("schema.sql");
/// The schema version this code writes and understands.
const SCHEMA_VERSION: i64 = 6;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("WAL checkpoint could not complete (database busy)")]
    CheckpointBusy,
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedSchema { found: i64, supported: i64 },
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens (creating if needed) the database file at `path`, its parent directory included.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| {
                StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?;
        }
        let conn = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
            )?;
        }
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        conn.pragma_update(None, "secure_delete", "ON")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Keep the schema-1 bootstrap so migration also covers unversioned legacy files.
        conn.execute_batch(SCHEMA)?;
        if version < 2 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE characters ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
                 ALTER TABLE characters ADD COLUMN model_id TEXT;
                 ALTER TABLE characters ADD COLUMN parameters TEXT NOT NULL DEFAULT '{}';
                 ALTER TABLE characters ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
                 ALTER TABLE characters ADD COLUMN sort INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE characters ADD COLUMN background TEXT;
                 ALTER TABLE characters ADD COLUMN resource_packs TEXT NOT NULL DEFAULT '[]';
                 ALTER TABLE characters ADD COLUMN sticker_identity TEXT NOT NULL DEFAULT 'neutral';
                 ALTER TABLE characters ADD COLUMN sticker_frequency TEXT NOT NULL DEFAULT 'often';
                 UPDATE characters SET display_name = name, model_id =
                   (SELECT value FROM settings WHERE key = 'default_model_id'
                    AND value IN (SELECT id FROM models));
                 ALTER TABLE nodes ADD COLUMN character_id TEXT;
                 ALTER TABLE nodes ADD COLUMN character_name TEXT;
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }
        if version < 3 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE attachments (
                   id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                   media_type TEXT NOT NULL, bytes BLOB NOT NULL, created_at INTEGER NOT NULL);
                 CREATE INDEX attachments_conversation ON attachments(conversation_id);
                 CREATE TABLE node_attachments (
                   node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                   attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                   position INTEGER NOT NULL, PRIMARY KEY(node_id,attachment_id), UNIQUE(node_id,position));
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?;
        }
        if version < 4 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE avatars (id TEXT PRIMARY KEY, media_type TEXT NOT NULL, bytes BLOB NOT NULL);
                 CREATE TABLE users (
                   id TEXT PRIMARY KEY, username TEXT NOT NULL COLLATE NOCASE UNIQUE,
                   password TEXT NOT NULL, password_storage TEXT NOT NULL CHECK(password_storage IN ('plain','hash')),
                   avatar TEXT, disabled INTEGER NOT NULL DEFAULT 0,
                   last_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
                   last_conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
                   created_at INTEGER NOT NULL);
                 CREATE TABLE user_sessions (
                   fingerprint TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                   expires_at INTEGER NOT NULL);
                 CREATE INDEX user_sessions_user ON user_sessions(user_id);
                 ALTER TABLE conversations ADD COLUMN owner_user_id TEXT REFERENCES users(id) ON DELETE SET NULL;
                 CREATE INDEX conversations_owner ON conversations(owner_user_id,updated_at);
                 ALTER TABLE characters ADD COLUMN avatar TEXT;
                 ALTER TABLE nodes ADD COLUMN character_avatar TEXT;
                 ALTER TABLE attachments ADD COLUMN name TEXT NOT NULL DEFAULT 'image';
                 ALTER TABLE attachments ADD COLUMN kind TEXT NOT NULL DEFAULT 'image' CHECK(kind IN ('image','document'));
                 ALTER TABLE attachments ADD COLUMN extracted_text TEXT;
                 UPDATE attachments SET name = CASE media_type WHEN 'image/png' THEN 'image.png' WHEN 'image/jpeg' THEN 'image.jpg' WHEN 'image/webp' THEN 'image.webp' WHEN 'image/gif' THEN 'image.gif' ELSE 'image' END;
                 DELETE FROM settings WHERE key='default_model_id';
                 INSERT INTO settings(key,value) VALUES('password_storage','plain');
                 PRAGMA user_version = 4;
                 COMMIT;",
            )?;
        }
        if version < 5 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE users ADD COLUMN credential_revision INTEGER NOT NULL DEFAULT 0;
                 PRAGMA user_version = 5;
                 COMMIT;",
            )?;
        }
        if version < 6 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE characters ADD COLUMN search_service_id TEXT;
                 CREATE TABLE search_usage (
                   id TEXT PRIMARY KEY, at_ms INTEGER NOT NULL, service_id TEXT NOT NULL,
                   status TEXT NOT NULL, usage TEXT);
                 CREATE INDEX search_usage_at ON search_usage(at_ms,id);
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for suffix in ["-wal", "-shm"] {
                let mut filename = path.as_os_str().to_owned();
                filename.push(suffix);
                let sidecar = std::path::PathBuf::from(filename);
                if sidecar.exists() {
                    std::fs::set_permissions(sidecar, std::fs::Permissions::from_mode(0o600))
                        .map_err(|e| {
                            StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
                        })?;
                }
            }
        }
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave SQLite itself inconsistent.
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Store>();
};

/// Current Unix time in milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A fresh UUIDv7 id string.
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
