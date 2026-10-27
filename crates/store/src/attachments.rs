//! Conversation-owned image bytes and reusable node relationships.
use crate::{Result, Store, StoreError, new_id, now_ms};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub media_type: String,
    pub size: usize,
    pub url: String,
}

fn metadata(id: String, conv: &str, media_type: String, size: usize) -> Attachment {
    Attachment {
        url: format!("/api/conversations/{conv}/attachments/{id}"),
        id,
        media_type,
        size,
    }
}

pub(crate) fn node_attachments(
    conn: &Connection,
    conv: &str,
    node: &str,
) -> Result<Vec<Attachment>> {
    let mut stmt = conn.prepare("SELECT a.id, a.media_type, length(a.bytes) FROM node_attachments n JOIN attachments a ON a.id=n.attachment_id WHERE n.node_id=?1 AND a.conversation_id=?2 ORDER BY n.position")?;
    let rows = stmt.query_map(params![node, conv], |r| {
        Ok(metadata(
            r.get(0)?,
            conv,
            r.get(1)?,
            r.get::<_, i64>(2)? as usize,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl Store {
    pub fn insert_attachment(
        &self,
        conv: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<Attachment> {
        let conn = self.conn();
        if conn
            .query_row(
                "SELECT 1 FROM conversations WHERE id=?1",
                [conv],
                |_| Ok(()),
            )
            .optional()?
            .is_none()
        {
            return Err(StoreError::NotFound);
        }
        let id = new_id();
        conn.execute("INSERT INTO attachments(id,conversation_id,media_type,bytes,created_at) VALUES(?1,?2,?3,?4,?5)", params![id,conv,media_type,bytes,now_ms()])?;
        Ok(metadata(id, conv, media_type.into(), bytes.len()))
    }

    pub fn get_attachment(&self, conv: &str, id: &str) -> Result<Option<(Attachment, Vec<u8>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT media_type, bytes FROM attachments WHERE id=?1 AND conversation_id=?2",
                params![id, conv],
                |r| {
                    let media_type = r.get(0)?;
                    let bytes: Vec<u8> = r.get(1)?;
                    Ok((metadata(id.into(), conv, media_type, bytes.len()), bytes))
                },
            )
            .optional()?)
    }

    pub fn attachment_metadata(&self, conv: &str, ids: &[String]) -> Result<Vec<Attachment>> {
        let conn = self.conn();
        ids.iter().map(|id| conn.query_row("SELECT media_type,length(bytes) FROM attachments WHERE id=?1 AND conversation_id=?2",params![id,conv],|r|Ok(metadata(id.clone(),conv,r.get(0)?,r.get::<_, i64>(1)? as usize))).optional()?.ok_or(StoreError::NotFound)).collect()
    }
}
