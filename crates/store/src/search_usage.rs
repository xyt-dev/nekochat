//! Search accounting deliberately excludes queries, evidence, URLs and user metadata.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchUsageRecord {
    pub id: String,
    pub at_ms: i64,
    pub service_id: String,
    pub status: String,
    pub usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageMetadata {
    requests: u32,
    cached_in: u64,
    uncached_in: u64,
    output: u64,
    reasoning: u64,
    cost: Option<MoneyMetadata>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoneyMetadata {
    amount: f64,
    currency: String,
}

impl Store {
    pub fn insert_search_usage(&self, record: &SearchUsageRecord) -> Result<()> {
        if let Some(usage) = &record.usage {
            let metadata = UsageMetadata::deserialize(usage)?;
            // Decode every numeric field and reject arbitrary JSON before it reaches disk.
            let _ = (metadata.requests, metadata.cached_in, metadata.uncached_in, metadata.output, metadata.reasoning);
            if metadata.cost.as_ref().is_some_and(|money| !money.amount.is_finite() || money.amount < 0.0 || !matches!(money.currency.as_str(), "CNY" | "USD")) {
                return Err(StoreError::Conflict("invalid search usage cost".into()));
            }
        }
        self.conn().execute(
            "INSERT INTO search_usage(id,at_ms,service_id,status,usage) VALUES(?1,?2,?3,?4,?5)",
            params![record.id, record.at_ms, record.service_id, record.status, record.usage.as_ref().map(serde_json::to_string).transpose()?],
        )?;
        Ok(())
    }

    pub fn list_search_usage(&self, from_ms: i64, to_ms: i64) -> Result<Vec<SearchUsageRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id,at_ms,service_id,status,usage FROM search_usage WHERE at_ms>=?1 AND at_ms<=?2 ORDER BY at_ms,id")?;
        let records = stmt.query_map(params![from_ms, to_ms], |row| {
            let usage: Option<String> = row.get(4)?;
            let usage = usage.map(|value| serde_json::from_str(&value)).transpose().map_err(|error| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error)))?;
            Ok(SearchUsageRecord { id: row.get(0)?, at_ms: row.get(1)?, service_id: row.get(2)?, status: row.get(3)?, usage })
        })?;
        Ok(records.collect::<rusqlite::Result<_>>()?)
    }
}
