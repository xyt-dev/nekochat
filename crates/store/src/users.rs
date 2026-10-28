//! Local identities, password modes, opaque-session fingerprints and avatar blobs.
use crate::{Result, Store, StoreError, new_id, now_ms};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PasswordStorage {
    Plain,
    Hash,
}
impl PasswordStorage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Hash => "hash",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserPublic {
    pub id: String,
    pub username: String,
    pub avatar: Option<String>,
    pub last_character_id: Option<String>,
    pub last_conversation_id: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct User {
    #[serde(flatten)]
    pub public: UserPublic,
    pub disabled: bool,
    pub password_storage: PasswordStorage,
    pub created_at: i64,
    #[serde(skip)]
    pub credential_revision: i64,
}
impl std::ops::Deref for User {
    type Target = UserPublic;
    fn deref(&self) -> &UserPublic {
        &self.public
    }
}
const USER_COLS: &str = "id,username,avatar,last_character_id,last_conversation_id,disabled,password_storage,created_at,credential_revision";
fn user_row(r: &Row) -> rusqlite::Result<User> {
    let mode: String = r.get(6)?;
    Ok(User {
        public: UserPublic {
            id: r.get(0)?,
            username: r.get(1)?,
            avatar: r.get(2)?,
            last_character_id: r.get(3)?,
            last_conversation_id: r.get(4)?,
        },
        disabled: r.get(5)?,
        password_storage: if mode == "hash" {
            PasswordStorage::Hash
        } else {
            PasswordStorage::Plain
        },
        created_at: r.get(7)?,
        credential_revision: r.get(8)?,
    })
}
fn encode_password(password: &str, mode: PasswordStorage) -> Result<String> {
    if mode == PasswordStorage::Plain {
        return Ok(password.to_owned());
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| StoreError::Conflict("random source unavailable".into()))?;
    let salt = SaltString::encode_b64(&random)
        .map_err(|_| StoreError::Conflict("salt encoding failed".into()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| StoreError::Conflict("password encoding failed".into()))
}
fn verify_password(password: &str, encoded: &str, mode: PasswordStorage) -> bool {
    match mode {
        PasswordStorage::Plain => bool::from(password.as_bytes().ct_eq(encoded.as_bytes())),
        PasswordStorage::Hash => PasswordHash::new(encoded).ok().is_some_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        }),
    }
}
fn mode_in(conn: &rusqlite::Connection) -> Result<PasswordStorage> {
    let value: String = conn.query_row(
        "SELECT value FROM settings WHERE key='password_storage'",
        [],
        |r| r.get(0),
    )?;
    Ok(if value == "hash" {
        PasswordStorage::Hash
    } else {
        PasswordStorage::Plain
    })
}
impl Store {
    pub fn password_storage(&self) -> Result<PasswordStorage> {
        mode_in(&self.conn())
    }
    pub fn set_password_storage(&self, mode: PasswordStorage) -> Result<()> {
        self.conn().execute(
            "UPDATE settings SET value=?1 WHERE key='password_storage'",
            [mode.as_str()],
        )?;
        Ok(())
    }
    pub fn create_user(&self, username: &str, password: &str) -> Result<User> {
        let conn = self.conn();
        let mode = mode_in(&conn)?;
        let id = new_id();
        conn.execute("INSERT INTO users(id,username,password,password_storage,created_at) VALUES(?1,?2,?3,?4,?5)",params![id,username,encode_password(password,mode)?,mode.as_str(),now_ms()]).map_err(|e|match &e {rusqlite::Error::SqliteFailure(f,_) if f.code==rusqlite::ErrorCode::ConstraintViolation=>StoreError::Conflict("username already exists".into()),_=>e.into()})?;
        Ok(conn.query_row(
            &format!("SELECT {USER_COLS} FROM users WHERE id=?1"),
            [id],
            user_row,
        )?)
    }
    pub fn get_user(&self, id: &str) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {USER_COLS} FROM users WHERE id=?1"),
                [id],
                user_row,
            )
            .optional()?)
    }
    pub fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {USER_COLS} FROM users ORDER BY created_at,id"
        ))?;
        Ok(stmt
            .query_map([], user_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn authenticate_user(&self, username: &str, password: &str) -> Result<Option<User>> {
        let conn = self.conn();
        let found = conn
            .query_row(
                &format!("SELECT {USER_COLS},password FROM users WHERE username=?1 AND disabled=0"),
                [username],
                |r| Ok((user_row(r)?, r.get::<_, String>(9)?)),
            )
            .optional()?;
        let Some((user, encoded)) = found else {
            return Ok(None);
        };
        if !verify_password(password, &encoded, user.password_storage) {
            return Ok(None);
        }
        Ok(Some(user))
    }
    pub fn user_password(&self, id: &str) -> Result<(Option<String>, PasswordStorage)> {
        let conn = self.conn();
        let (password, mode): (String, String) = conn
            .query_row(
                "SELECT password,password_storage FROM users WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        Ok(if mode == "plain" {
            (Some(password), PasswordStorage::Plain)
        } else {
            (None, PasswordStorage::Hash)
        })
    }
    pub fn verified_password_revision(&self, id: &str, password: &str) -> Result<Option<i64>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT password,password_storage,credential_revision FROM users WHERE id=?1 AND disabled=0",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(encoded, mode, revision)| {
            verify_password(
                password,
                &encoded,
                if mode == "hash" {
                    PasswordStorage::Hash
                } else {
                    PasswordStorage::Plain
                },
            )
            .then_some(revision)
        }))
    }
    pub fn update_user(
        &self,
        id: &str,
        expected_revision: Option<i64>,
        username: Option<&str>,
        password: Option<&str>,
        avatar: Option<Option<&str>>,
        disabled: Option<bool>,
    ) -> Result<User> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mode = tx
            .query_row(
                "SELECT password_storage FROM users WHERE id=?1 AND (?2 IS NULL OR (disabled=0 AND credential_revision=?2))",
                params![id, expected_revision],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        if let Some(name) = username {
            tx.execute(
                "UPDATE users SET username=?2 WHERE id=?1",
                params![id, name],
            )
            .map_err(|e| match &e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::Conflict("username already exists".into())
                }
                _ => e.into(),
            })?;
        }
        if let Some(password) = password {
            let mode = if mode == "hash" {
                PasswordStorage::Hash
            } else {
                PasswordStorage::Plain
            };
            tx.execute(
                "UPDATE users SET password=?2 WHERE id=?1",
                params![id, encode_password(password, mode)?],
            )?;
        }
        if let Some(avatar) = avatar {
            tx.execute(
                "UPDATE users SET avatar=?2 WHERE id=?1",
                params![id, avatar],
            )?;
        }
        if let Some(disabled) = disabled {
            tx.execute(
                "UPDATE users SET disabled=?2 WHERE id=?1",
                params![id, disabled],
            )?;
        }
        if password.is_some() || disabled == Some(true) {
            tx.execute(
                "UPDATE users SET credential_revision=credential_revision+1 WHERE id=?1",
                [id],
            )?;
            tx.execute("DELETE FROM user_sessions WHERE user_id=?1", [id])?;
        }
        let user = tx.query_row(
            &format!("SELECT {USER_COLS} FROM users WHERE id=?1"),
            [id],
            user_row,
        )?;
        tx.commit()?;
        Ok(user)
    }
    pub fn delete_user(&self, id: &str) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let conversations = {
            let mut query = tx.prepare("SELECT id FROM conversations WHERE owner_user_id=?1")?;
            query
                .query_map([id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        };
        tx.execute("UPDATE nodes SET parent_id=NULL WHERE conversation_id IN (SELECT id FROM conversations WHERE owner_user_id=?1)",[id])?;
        tx.execute("DELETE FROM conversations WHERE owner_user_id=?1", [id])?;
        if tx.execute("DELETE FROM users WHERE id=?1", [id])? == 0 {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        Ok(conversations)
    }

    /// Erase old database/WAL pages after callers have cancelled deleted conversations.
    pub fn purge_deleted_pages(&self) -> Result<()> {
        self.conn()
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
    pub fn create_user_session(
        &self,
        id: &str,
        credential_revision: i64,
        fingerprint: &str,
        expires_at: i64,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM user_sessions WHERE expires_at<=?1", [now_ms()])?;
        if conn.execute("INSERT INTO user_sessions(fingerprint,user_id,expires_at) SELECT ?2,id,?3 FROM users WHERE id=?1 AND disabled=0 AND credential_revision=?4",params![id,fingerprint,expires_at,credential_revision])?==0{return Err(StoreError::NotFound)}
        Ok(())
    }
    pub fn user_for_session(&self, fingerprint: &str) -> Result<Option<User>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("SELECT {USER_COLS} FROM users WHERE disabled=0 AND id=(SELECT user_id FROM user_sessions WHERE fingerprint=?1 AND expires_at>?2)"),params![fingerprint,now_ms()],user_row).optional()?)
    }
    pub fn revoke_user_session(&self, fingerprint: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM user_sessions WHERE fingerprint=?1",
            [fingerprint],
        )?;
        Ok(())
    }
    pub fn revoke_user_sessions(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM user_sessions WHERE user_id=?1", [id])?;
        Ok(())
    }
    pub fn set_user_preferences(
        &self,
        id: &str,
        character: Option<&str>,
        conversation: Option<&str>,
    ) -> Result<UserPublic> {
        let conn = self.conn();
        if let Some(character) = character {
            if conn
                .query_row(
                    "SELECT 1 FROM characters WHERE id=?1 AND enabled=1",
                    [character],
                    |_| Ok(()),
                )
                .optional()?
                .is_none()
            {
                return Err(StoreError::NotFound);
            }
        }
        if let Some(conversation) = conversation {
            if conn
                .query_row(
                    "SELECT 1 FROM conversations WHERE id=?1 AND owner_user_id=?2",
                    params![conversation, id],
                    |_| Ok(()),
                )
                .optional()?
                .is_none()
            {
                return Err(StoreError::NotFound);
            }
        }
        if conn.execute("UPDATE users SET last_character_id=?2,last_conversation_id=?3 WHERE id=?1 AND disabled=0",params![id,character,conversation])?==0{return Err(StoreError::NotFound)}
        Ok(conn
            .query_row(
                &format!("SELECT {USER_COLS} FROM users WHERE id=?1"),
                [id],
                user_row,
            )?
            .public)
    }
    pub fn legacy_conversation_count(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM conversations WHERE owner_user_id IS NULL",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn assign_legacy_conversations(
        &self,
        user_id: &str,
        ids: Option<&[String]>,
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if tx
            .query_row("SELECT 1 FROM users WHERE id=?1", [user_id], |_| Ok(()))
            .optional()?
            .is_none()
        {
            return Err(StoreError::NotFound);
        }
        let count = if let Some(ids) = ids {
            let mut count = 0;
            for id in ids {
                if tx.execute("UPDATE conversations SET owner_user_id=?2 WHERE id=?1 AND owner_user_id IS NULL",params![id,user_id])?==0{return Err(StoreError::NotFound)}
                count += 1;
            }
            count
        } else {
            tx.execute(
                "UPDATE conversations SET owner_user_id=?1 WHERE owner_user_id IS NULL",
                [user_id],
            )?
        };
        tx.commit()?;
        Ok(count)
    }
    pub fn insert_avatar(&self, media_type: &str, bytes: &[u8]) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO avatars(id,media_type,bytes) VALUES(?1,?2,?3)",
            params![id, media_type, bytes],
        )?;
        Ok(format!("/api/avatars/{id}"))
    }
    pub fn avatar_exists(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT 1 FROM avatars WHERE id=?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }
    pub fn get_avatar(&self, id: &str) -> Result<Option<(String, Vec<u8>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT media_type,bytes FROM avatars WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}
