//! Characters, settings and listed models.

use nekochat_core::{Background, GenerationParameters, Price, Protocol, ReasoningSpec};
use rusqlite::{ErrorCode, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError, new_id, now_ms};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Character {
    pub id: String,
    #[serde(flatten)]
    pub spec: CharacterSpec,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CharacterSpec {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub search_service_id: Option<String>,
    #[serde(default)]
    pub parameters: GenerationParameters,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub sort: i64,
    #[serde(default)]
    pub background: Option<Background>,
    #[serde(default)]
    pub resource_packs: Vec<String>,
    #[serde(default = "identity_default")]
    pub sticker_identity: String,
    #[serde(default = "frequency_default")]
    pub sticker_frequency: String,
}

fn enabled_default() -> bool {
    true
}
fn identity_default() -> String {
    "neutral".into()
}
fn frequency_default() -> String {
    "often".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    DefaultCharacter,
    SearchConnection,
}

impl SettingKey {
    fn as_str(self) -> &'static str {
        match self {
            SettingKey::DefaultCharacter => "default_character_id",
            SettingKey::SearchConnection => "search_connection",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ModelSpec {
    pub provider: String,
    pub upstream_id: String,
    pub display_name: String,
    pub sort: i64,
    pub protocol: Protocol,
    pub reasoning: ReasoningSpec,
    pub vision: bool,
    pub context_window: Option<i64>,
    pub price: Price,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ListedModel {
    pub id: String,
    #[serde(flatten)]
    pub spec: ModelSpec,
}

const CHARACTER_COLS: &str = "id, name, system_prompt, created_at, updated_at, display_name, model_id, parameters, enabled, sort, background, resource_packs, sticker_identity, sticker_frequency, avatar, search_service_id";
const MODEL_COLS: &str = "id, provider, upstream_id, display_name, sort, protocol, reasoning, vision, context_window, price";

fn character_from_row(r: &Row) -> rusqlite::Result<Character> {
    fn json<T: serde::de::DeserializeOwned>(r: &Row, i: usize) -> rusqlite::Result<T> {
        let value: String = r.get(i)?;
        serde_json::from_str(&value).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
        })
    }
    Ok(Character {
        id: r.get(0)?,
        spec: CharacterSpec {
            name: r.get(1)?,
            system_prompt: r.get(2)?,
            display_name: r.get(5)?,
            model_id: r.get(6)?,
            parameters: json(r, 7)?,
            enabled: r.get(8)?,
            sort: r.get(9)?,
            background: r
                .get::<_, Option<String>>(10)?
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
            resource_packs: json(r, 11)?,
            sticker_identity: r.get(12)?,
            sticker_frequency: r.get(13)?,
            avatar: r.get(14)?,
            search_service_id: r.get(15)?,
        },
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

/// Raw model row; JSON columns are decoded outside rusqlite's row closure.
struct ModelRow {
    id: String,
    provider: String,
    upstream_id: String,
    display_name: String,
    sort: i64,
    protocol: String,
    reasoning: String,
    vision: bool,
    context_window: Option<i64>,
    price: String,
}

fn model_row(r: &Row) -> rusqlite::Result<ModelRow> {
    Ok(ModelRow {
        id: r.get(0)?,
        provider: r.get(1)?,
        upstream_id: r.get(2)?,
        display_name: r.get(3)?,
        sort: r.get(4)?,
        protocol: r.get(5)?,
        reasoning: r.get(6)?,
        vision: r.get(7)?,
        context_window: r.get(8)?,
        price: r.get(9)?,
    })
}

impl ModelRow {
    fn decode(self) -> Result<ListedModel> {
        Ok(ListedModel {
            id: self.id,
            spec: ModelSpec {
                provider: self.provider,
                upstream_id: self.upstream_id,
                display_name: self.display_name,
                sort: self.sort,
                protocol: serde_json::from_value(serde_json::Value::String(self.protocol))?,
                reasoning: serde_json::from_str(&self.reasoning)?,
                vision: self.vision,
                context_window: self.context_window,
                price: serde_json::from_str(&self.price)?,
            },
        })
    }
}

/// Encoded JSON/text columns of a `ModelSpec`: (protocol, reasoning, price).
fn encode_spec(spec: &ModelSpec) -> Result<(String, String, String)> {
    let protocol = match serde_json::to_value(&spec.protocol)? {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    Ok((
        protocol,
        serde_json::to_string(&spec.reasoning)?,
        serde_json::to_string(&spec.price)?,
    ))
}

fn map_unique(e: rusqlite::Error, spec: &ModelSpec) -> StoreError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation => {
            StoreError::Conflict(format!(
                "model {}/{} already listed",
                spec.provider, spec.upstream_id
            ))
        }
        _ => StoreError::Sqlite(e),
    }
}

impl Store {
    pub fn create_character(&self, spec: &CharacterSpec) -> Result<Character> {
        let now = now_ms();
        let c = Character {
            id: new_id(),
            spec: spec.clone(),
            created_at: now,
            updated_at: now,
        };
        self.conn().execute(
            &format!("INSERT INTO characters ({CHARACTER_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"),
            params![c.id, spec.name, spec.system_prompt, now, now, spec.display_name, spec.model_id,
                serde_json::to_string(&spec.parameters)?, spec.enabled, spec.sort,
                spec.background.as_ref().map(serde_json::to_string).transpose()?,
                serde_json::to_string(&spec.resource_packs)?, spec.sticker_identity, spec.sticker_frequency, spec.avatar, spec.search_service_id],
        )?;
        Ok(c)
    }

    pub fn update_character(&self, id: &str, spec: &CharacterSpec) -> Result<Character> {
        let conn = self.conn();
        conn.query_row(
            &format!("UPDATE characters SET name=?2, system_prompt=?3, display_name=?4, model_id=?5, parameters=?6, enabled=?7, sort=?8, background=?9, resource_packs=?10, sticker_identity=?11, sticker_frequency=?12, updated_at=?13, avatar=?14, search_service_id=?15 WHERE id=?1 RETURNING {CHARACTER_COLS}"),
            params![id, spec.name, spec.system_prompt, spec.display_name, spec.model_id,
                serde_json::to_string(&spec.parameters)?, spec.enabled, spec.sort,
                spec.background.as_ref().map(serde_json::to_string).transpose()?,
                serde_json::to_string(&spec.resource_packs)?, spec.sticker_identity, spec.sticker_frequency, now_ms(), spec.avatar, spec.search_service_id],
            character_from_row,
        ).optional()?.ok_or(StoreError::NotFound)
    }

    pub fn delete_character(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if tx.execute("DELETE FROM characters WHERE id = ?1", [id])? == 0 {
            return Err(StoreError::NotFound);
        }
        tx.execute(
            "DELETE FROM settings WHERE key = ?1 AND value = ?2",
            params![SettingKey::DefaultCharacter.as_str(), id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_characters(&self) -> Result<Vec<Character>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CHARACTER_COLS} FROM characters ORDER BY sort, created_at, id"
        ))?;
        let rows = stmt.query_map([], character_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_character(&self, id: &str) -> Result<Option<Character>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {CHARACTER_COLS} FROM characters WHERE id = ?1"),
                [id],
                character_from_row,
            )
            .optional()?)
    }

    pub fn search_service_in_use(&self, service_id: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM characters WHERE search_service_id=?1)",
            [service_id],
            |row| row.get(0),
        )?)
    }

    pub fn get_setting(&self, key: SettingKey) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [key.as_str()],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: SettingKey, value: Option<&str>) -> Result<()> {
        let conn = self.conn();
        match value {
            Some(v) => conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key.as_str(), v],
            )?,
            None => conn.execute("DELETE FROM settings WHERE key = ?1", [key.as_str()])?,
        };
        Ok(())
    }

    pub fn insert_model(&self, spec: &ModelSpec) -> Result<ListedModel> {
        let (protocol, reasoning, price) = encode_spec(spec)?;
        let id = new_id();
        self.conn()
            .execute(
                &format!(
                    "INSERT INTO models ({MODEL_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
                ),
                params![
                    id,
                    spec.provider,
                    spec.upstream_id,
                    spec.display_name,
                    spec.sort,
                    protocol,
                    reasoning,
                    spec.vision,
                    spec.context_window,
                    price
                ],
            )
            .map_err(|e| map_unique(e, spec))?;
        Ok(ListedModel {
            id,
            spec: spec.clone(),
        })
    }

    pub fn update_model(&self, id: &str, spec: &ModelSpec) -> Result<ListedModel> {
        let (protocol, reasoning, price) = encode_spec(spec)?;
        let n = self
            .conn()
            .execute(
                "UPDATE models SET provider = ?2, upstream_id = ?3, display_name = ?4, sort = ?5, protocol = ?6, \
                 reasoning = ?7, vision = ?8, context_window = ?9, price = ?10 WHERE id = ?1",
                params![
                    id,
                    spec.provider,
                    spec.upstream_id,
                    spec.display_name,
                    spec.sort,
                    protocol,
                    reasoning,
                    spec.vision,
                    spec.context_window,
                    price
                ],
            )
            .map_err(|e| map_unique(e, spec))?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(ListedModel {
            id: id.to_owned(),
            spec: spec.clone(),
        })
    }

    pub fn delete_model(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if tx.execute("DELETE FROM models WHERE id = ?1", [id])? == 0 {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_models(&self) -> Result<Vec<ListedModel>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MODEL_COLS} FROM models ORDER BY sort, display_name"
        ))?;
        let rows = stmt.query_map([], model_row)?;
        rows.map(|r| r?.decode()).collect()
    }

    pub fn get_model(&self, id: &str) -> Result<Option<ListedModel>> {
        self.conn()
            .query_row(
                &format!("SELECT {MODEL_COLS} FROM models WHERE id = ?1"),
                [id],
                model_row,
            )
            .optional()?
            .map(ModelRow::decode)
            .transpose()
    }
}
