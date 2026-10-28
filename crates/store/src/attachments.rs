//! Conversation-owned image/document bytes, extracted text and reusable node relationships.
use crate::{Result, Store, StoreError, new_id, now_ms};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Public metadata deliberately excludes original bytes and extracted document content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub media_type: String,
    pub size: usize,
    pub url: String,
}

fn metadata(
    id: String,
    conv: &str,
    name: String,
    kind: String,
    media_type: String,
    size: usize,
) -> Attachment {
    Attachment {
        url: format!("/api/conversations/{conv}/attachments/{id}"),
        id,
        name,
        kind,
        media_type,
        size,
    }
}

pub(crate) fn node_attachments(
    conn: &Connection,
    conv: &str,
    node: &str,
) -> Result<Vec<Attachment>> {
    let mut stmt = conn.prepare("SELECT a.id, a.name, a.kind, a.media_type, length(a.bytes) FROM node_attachments n JOIN attachments a ON a.id=n.attachment_id WHERE n.node_id=?1 AND a.conversation_id=?2 ORDER BY n.position")?;
    let rows = stmt.query_map(params![node, conv], |r| {
        Ok(metadata(
            r.get(0)?,
            conv,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get::<_, i64>(4)? as usize,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl Store {
    pub fn insert_attachment(
        &self,
        conv: &str,
        name: &str,
        kind: &str,
        media_type: &str,
        bytes: &[u8],
        extracted_text: Option<&str>,
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
        conn.execute("INSERT INTO attachments(id,conversation_id,name,kind,media_type,bytes,extracted_text,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![id,conv,name,kind,media_type,bytes,extracted_text,now_ms()])?;
        Ok(metadata(
            id,
            conv,
            name.into(),
            kind.into(),
            media_type.into(),
            bytes.len(),
        ))
    }

    pub fn get_attachment(&self, conv: &str, id: &str) -> Result<Option<(Attachment, Vec<u8>)>> {
        Ok(self.conn().query_row(
            "SELECT name,kind,media_type,bytes FROM attachments WHERE id=?1 AND conversation_id=?2",
            params![id, conv],
            |r| {
                let bytes: Vec<u8> = r.get(3)?;
                Ok((metadata(id.into(), conv, r.get(0)?, r.get(1)?, r.get(2)?, bytes.len()), bytes))
            },
        ).optional()?)
    }

    pub fn attachment_metadata(&self, conv: &str, ids: &[String]) -> Result<Vec<Attachment>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT name,kind,media_type,length(bytes) FROM attachments WHERE id=?1 AND conversation_id=?2")?;
        ids.iter()
            .map(|id| {
                stmt.query_row(params![id, conv], |r| {
                    Ok(metadata(
                        id.clone(),
                        conv,
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get::<_, i64>(3)? as usize,
                    ))
                })
                .optional()?
                .ok_or(StoreError::NotFound)
            })
            .collect()
    }

    /// Reads only the representation needed by the model: image bytes OR document text.
    /// Raw document bytes never need copying into the context-building path.
    pub fn attachment_model_content(
        &self,
        conv: &str,
        id: &str,
    ) -> Result<Option<(Attachment, Option<Vec<u8>>, Option<String>)>> {
        Ok(self.conn().query_row(
            "SELECT name,kind,media_type,length(bytes),CASE WHEN kind='image' THEN bytes END,extracted_text FROM attachments WHERE id=?1 AND conversation_id=?2",
            params![id,conv],
            |r| Ok((metadata(id.into(),conv,r.get(0)?,r.get(1)?,r.get(2)?,r.get::<_,i64>(3)? as usize),r.get(4)?,r.get(5)?)),
        ).optional()?)
    }
}
