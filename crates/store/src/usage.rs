//! Per-request usage records (no conversation id, no content).

use nekochat_core::Usage;
use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsagePurpose {
    Chat,
}

impl UsagePurpose {
    fn as_str(self) -> &'static str {
        match self {
            UsagePurpose::Chat => "chat",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "chat" => Some(UsagePurpose::Chat),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UsageRecord {
    pub at_ms: i64,
    pub provider: String,
    pub upstream_id: String,
    pub model_label: String,
    pub usage: Usage,
    pub cost_usd: Option<f64>,
    pub purpose: UsagePurpose,
}

const USAGE_COLS: &str = "at_ms, provider, upstream_id, model_label, cached_in, uncached_in, reasoning, output, cost_usd, upstream_cost_usd, allowance_usd, purpose";

fn tokens(r: &Row, idx: usize) -> rusqlite::Result<u64> {
    u64::try_from(r.get::<_, i64>(idx)?).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Integer, Box::new(e))
    })
}

fn record_from_row(r: &Row) -> rusqlite::Result<UsageRecord> {
    let purpose: String = r.get(11)?;
    Ok(UsageRecord {
        at_ms: r.get(0)?,
        provider: r.get(1)?,
        upstream_id: r.get(2)?,
        model_label: r.get(3)?,
        usage: Usage {
            cached_in: tokens(r, 4)?,
            uncached_in: tokens(r, 5)?,
            reasoning: tokens(r, 6)?,
            output: tokens(r, 7)?,
            upstream_cost_usd: r.get(9)?,
            allowance_usd: r.get(10)?,
        },
        cost_usd: r.get(8)?,
        purpose: UsagePurpose::parse(&purpose).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Text,
                format!("unknown usage purpose {purpose:?}").into(),
            )
        })?,
    })
}

impl Store {
    pub fn insert_usage(&self, rec: &UsageRecord) -> Result<()> {
        let u = &rec.usage;
        let count = |n: u64| {
            i64::try_from(n).map_err(|e| {
                StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })
        };
        self.conn().execute(
            &format!(
                "INSERT INTO usage ({USAGE_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
            ),
            params![
                rec.at_ms,
                rec.provider,
                rec.upstream_id,
                rec.model_label,
                count(u.cached_in)?,
                count(u.uncached_in)?,
                count(u.reasoning)?,
                count(u.output)?,
                rec.cost_usd,
                u.upstream_cost_usd,
                u.allowance_usd,
                rec.purpose.as_str()
            ],
        )?;
        Ok(())
    }

    /// Records with `from_ms <= at_ms < to_ms`, oldest first.
    pub fn usage_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<UsageRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {USAGE_COLS} FROM usage WHERE at_ms >= ?1 AND at_ms < ?2 ORDER BY at_ms, id"
        ))?;
        let rows = stmt.query_map(params![from_ms, to_ms], record_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
