//! Conversations, message nodes and the branch tree.

use nekochat_core::Metrics;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError, new_id, now_ms};

/// Title of a conversation that has not been named from its first message yet.
pub const DEFAULT_TITLE: &str = "新对话";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub character_id: Option<String>,
    pub leaf_id: Option<String>,
    pub session_key: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    User,
    Assistant,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Complete,
    Interrupted,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub conversation_id: String,
    pub parent_id: Option<String>,
    pub role: NodeRole,
    pub content: String,
    pub reasoning: Option<String>,
    pub model_id: Option<String>,
    pub model_label: Option<String>,
    pub effort: Option<String>,
    pub character_id: Option<String>,
    pub character_name: Option<String>,
    pub status: NodeStatus,
    pub metrics: Option<Metrics>,
    pub created_at: i64,
}

impl NodeRole {
    fn as_str(self) -> &'static str {
        match self {
            NodeRole::User => "user",
            NodeRole::Assistant => "assistant",
        }
    }
}

impl NodeStatus {
    fn as_str(self) -> &'static str {
        match self {
            NodeStatus::Complete => "complete",
            NodeStatus::Interrupted => "interrupted",
            NodeStatus::Error => "error",
        }
    }
}

impl ToSql for NodeRole {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for NodeRole {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "user" => Ok(NodeRole::User),
            "assistant" => Ok(NodeRole::Assistant),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for NodeStatus {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for NodeStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "complete" => Ok(NodeStatus::Complete),
            "interrupted" => Ok(NodeStatus::Interrupted),
            "error" => Ok(NodeStatus::Error),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

const CONV_COLS: &str = "id, title, character_id, leaf_id, session_key, created_at, updated_at";
const NODE_COLS: &str = "id, conversation_id, parent_id, role, content, reasoning, model_id, model_label, effort, status, metrics, created_at, character_id, character_name";

fn conversation_from_row(r: &Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: r.get(0)?,
        title: r.get(1)?,
        character_id: r.get(2)?,
        leaf_id: r.get(3)?,
        session_key: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

/// Reads a node row; `metrics` stays raw JSON for the caller to decode.
fn node_from_row(r: &Row) -> rusqlite::Result<(Node, Option<String>)> {
    Ok((
        Node {
            id: r.get(0)?,
            conversation_id: r.get(1)?,
            parent_id: r.get(2)?,
            role: r.get(3)?,
            content: r.get(4)?,
            reasoning: r.get(5)?,
            model_id: r.get(6)?,
            model_label: r.get(7)?,
            effort: r.get(8)?,
            status: r.get(9)?,
            metrics: None,
            created_at: r.get(11)?,
            character_id: r.get(12)?,
            character_name: r.get(13)?,
        },
        r.get(10)?,
    ))
}

fn decode_node((mut node, metrics): (Node, Option<String>)) -> Result<Node> {
    node.metrics = metrics.as_deref().map(serde_json::from_str).transpose()?;
    Ok(node)
}

fn node_in(conn: &Connection, conv_id: &str, node_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM nodes WHERE id = ?1 AND conversation_id = ?2",
            params![node_id, conv_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Rewrites the database so no freed or stale bytes survive, then empties the WAL.
fn scrub(conn: &Connection) -> Result<()> {
    conn.execute_batch("VACUUM")?;
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        return Err(StoreError::CheckpointBusy);
    }
    Ok(())
}

impl Store {
    pub fn create_conversation(&self, character_id: Option<&str>) -> Result<Conversation> {
        let now = now_ms();
        let conv = Conversation {
            id: new_id(),
            title: DEFAULT_TITLE.into(),
            character_id: character_id.map(Into::into),
            leaf_id: None,
            session_key: uuid::Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
        };
        self.conn().execute(
            &format!("INSERT INTO conversations ({CONV_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
            params![
                conv.id,
                conv.title,
                conv.character_id,
                conv.leaf_id,
                conv.session_key,
                conv.created_at,
                conv.updated_at
            ],
        )?;
        Ok(conv)
    }

    pub fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CONV_COLS} FROM conversations ORDER BY updated_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], conversation_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_conversation(&self, id: &str) -> Result<Option<Conversation>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {CONV_COLS} FROM conversations WHERE id = ?1"),
                [id],
                conversation_from_row,
            )
            .optional()?)
    }

    pub fn set_title(&self, id: &str, title: &str) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE conversations SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, now_ms()],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    pub fn set_character(&self, id: &str, character_id: &str) -> Result<()> {
        if self.conn().execute(
            "UPDATE conversations SET character_id=?2, updated_at=?3 WHERE id=?1",
            params![id, character_id, now_ms()],
        )? == 0
        {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// Inserts `node` and makes it the conversation's leaf, atomically.
    pub fn insert_node(&self, node: &Node) -> Result<()> {
        let metrics = node
            .metrics
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM conversations WHERE id = ?1",
                [&node.conversation_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(StoreError::NotFound);
        }
        if let Some(parent) = &node.parent_id
            && !node_in(&tx, &node.conversation_id, parent)?
        {
            return Err(StoreError::Conflict(
                "parent belongs to another conversation".into(),
            ));
        }
        tx.execute(
            &format!(
                "INSERT INTO nodes ({NODE_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
            ),
            params![
                node.id,
                node.conversation_id,
                node.parent_id,
                node.role,
                node.content,
                node.reasoning,
                node.model_id,
                node.model_label,
                node.effort,
                node.status,
                metrics,
                node.created_at,
                node.character_id,
                node.character_name
            ],
        )?;
        tx.execute(
            "UPDATE conversations SET leaf_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![node.conversation_id, node.id, now_ms()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_node(&self, conv_id: &str, node_id: &str) -> Result<Option<Node>> {
        self.conn()
            .query_row(
                &format!("SELECT {NODE_COLS} FROM nodes WHERE id = ?1 AND conversation_id = ?2"),
                params![node_id, conv_id],
                node_from_row,
            )
            .optional()?
            .map(decode_node)
            .transpose()
    }

    pub fn set_leaf(&self, conv_id: &str, node_id: Option<&str>) -> Result<()> {
        let conn = self.conn();
        if let Some(node_id) = node_id
            && !node_in(&conn, conv_id, node_id)?
        {
            return Err(StoreError::NotFound);
        }
        let n = conn.execute(
            "UPDATE conversations SET leaf_id = ?2 WHERE id = ?1",
            params![conv_id, node_id],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// Follows the most recently created child from `node_id` down to a leaf.
    pub fn newest_leaf_under(&self, conv_id: &str, node_id: &str) -> Result<String> {
        let conn = self.conn();
        if !node_in(&conn, conv_id, node_id)? {
            return Err(StoreError::NotFound);
        }
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes WHERE conversation_id = ?1 AND parent_id = ?2
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )?;
        let mut current = node_id.to_owned();
        while let Some(child) = stmt
            .query_row(params![conv_id, current], |r| r.get::<_, String>(0))
            .optional()?
        {
            current = child;
        }
        Ok(current)
    }

    /// Nodes from the root down to `node_id`; empty for `None`.
    pub fn path_to(&self, conv_id: &str, node_id: Option<&str>) -> Result<Vec<Node>> {
        let Some(node_id) = node_id else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "WITH RECURSIVE up(id, depth) AS (
                 SELECT id, 0 FROM nodes WHERE id = ?1 AND conversation_id = ?2
                 UNION ALL
                 SELECT n.parent_id, up.depth + 1 FROM nodes n JOIN up ON n.id = up.id
                 WHERE n.parent_id IS NOT NULL
             )
             SELECT {cols} FROM up JOIN nodes n ON n.id = up.id ORDER BY up.depth DESC",
            cols = NODE_COLS
                .split(", ")
                .map(|c| format!("n.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let rows = stmt.query_map(params![node_id, conv_id], node_from_row)?;
        let path = rows.map(|r| decode_node(r?)).collect::<Result<Vec<_>>>()?;
        if path.is_empty() {
            return Err(StoreError::NotFound);
        }
        Ok(path)
    }

    pub fn nodes(&self, conv_id: &str) -> Result<Vec<Node>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {NODE_COLS} FROM nodes WHERE conversation_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map([conv_id], node_from_row)?;
        rows.map(|r| decode_node(r?)).collect()
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        // Detach parents first: a deep cascade through `parent_id` exceeds SQLite's trigger depth.
        tx.execute(
            "UPDATE nodes SET parent_id = NULL WHERE conversation_id = ?1",
            [id],
        )?;
        let n = tx.execute("DELETE FROM conversations WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        scrub(&conn)
    }

    /// Removes every node and resets the title, keeping the conversation itself.
    pub fn clear_conversation(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE conversations SET leaf_id = NULL, title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, DEFAULT_TITLE, now_ms()],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        tx.execute(
            "UPDATE nodes SET parent_id = NULL WHERE conversation_id = ?1",
            [id],
        )?;
        tx.execute("DELETE FROM nodes WHERE conversation_id = ?1", [id])?;
        tx.commit()?;
        scrub(&conn)
    }
}
