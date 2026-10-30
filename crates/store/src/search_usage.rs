//! Search accounting deliberately excludes queries, evidence, URLs and user metadata.

use std::collections::BTreeMap;

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchUsageDashboard {
    pub from_ms: i64,
    pub to_ms: i64,
    pub bucket_ms: i64,
    pub totals: SearchUsageTotals,
    pub services: Vec<SearchServiceUsage>,
    pub records: Vec<SearchUsageRecord>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchUsageTotals {
    pub attempts: u64,
    pub requests: u64,
    pub cached_in: u64,
    pub uncached_in: u64,
    pub output: u64,
    pub reasoning: u64,
    pub known_costs: Vec<SearchUsageCost>,
    pub unknown_costs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchUsageCost {
    pub currency: String,
    pub amount: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchServiceUsage {
    pub service_id: String,
    pub totals: SearchUsageTotals,
    pub points: Vec<SearchUsagePoint>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchUsagePoint {
    pub at_ms: i64,
    pub attempts: u64,
    pub requests: u64,
    pub cached_in: u64,
    pub uncached_in: u64,
    pub output: u64,
    pub reasoning: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageMetadata<'a> {
    requests: u32,
    cached_in: u64,
    uncached_in: u64,
    output: u64,
    reasoning: u64,
    #[serde(borrow)]
    cost: Option<MoneyMetadata<'a>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoneyMetadata<'a> {
    amount: f64,
    currency: &'a str,
}

impl UsageMetadata<'_> {
    fn validate(&self) -> Result<()> {
        if self.cost.as_ref().is_some_and(|money| !money.amount.is_finite() || money.amount < 0.0 || !matches!(money.currency, "CNY" | "USD")) {
            return Err(StoreError::Conflict("invalid search usage cost".into()));
        }
        Ok(())
    }
}

fn add_count(total: &mut u64, value: u64) -> Result<()> {
    *total = total.checked_add(value)
        .ok_or_else(|| StoreError::Conflict("search usage counter overflow".into()))?;
    Ok(())
}

impl SearchUsageTotals {
    fn add_attempt(&mut self, usage: Option<&UsageMetadata>) -> Result<()> {
        add_count(&mut self.attempts, 1)?;
        if let Some(usage) = usage {
            add_count(&mut self.requests, u64::from(usage.requests))?;
            add_count(&mut self.cached_in, usage.cached_in)?;
            add_count(&mut self.uncached_in, usage.uncached_in)?;
            add_count(&mut self.output, usage.output)?;
            add_count(&mut self.reasoning, usage.reasoning)?;
        }
        if let Some(cost) = usage.and_then(|usage| usage.cost.as_ref()) {
            match self.known_costs.binary_search_by(|known| known.currency.as_str().cmp(cost.currency)) {
                Ok(index) => {
                    let amount = self.known_costs[index].amount + cost.amount;
                    if !amount.is_finite() {
                        return Err(StoreError::Conflict("search usage cost overflow".into()));
                    }
                    self.known_costs[index].amount = amount;
                }
                Err(index) => self.known_costs.insert(index, SearchUsageCost {
                    currency: cost.currency.to_owned(), amount: cost.amount,
                }),
            }
        } else {
            add_count(&mut self.unknown_costs, 1)?;
        }
        Ok(())
    }
}

impl SearchUsagePoint {
    fn add_attempt(&mut self, usage: Option<&UsageMetadata>) -> Result<()> {
        add_count(&mut self.attempts, 1)?;
        if let Some(usage) = usage {
            add_count(&mut self.requests, u64::from(usage.requests))?;
            add_count(&mut self.cached_in, usage.cached_in)?;
            add_count(&mut self.uncached_in, usage.uncached_in)?;
            add_count(&mut self.output, usage.output)?;
            add_count(&mut self.reasoning, usage.reasoning)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct ServiceAccumulator {
    totals: SearchUsageTotals,
    points: BTreeMap<i64, SearchUsagePoint>,
}

impl ServiceAccumulator {
    fn add_attempt(&mut self, bucket: i64, usage: Option<&UsageMetadata>) -> Result<()> {
        self.totals.add_attempt(usage)?;
        self.points.entry(bucket).or_insert_with(|| SearchUsagePoint {
            at_ms: bucket, ..SearchUsagePoint::default()
        }).add_attempt(usage)
    }
}

impl Store {
    pub fn insert_search_usage(&self, record: &SearchUsageRecord) -> Result<()> {
        if let Some(usage) = &record.usage {
            let metadata = UsageMetadata::deserialize(usage)?;
            // Decode every numeric field and reject arbitrary JSON before it reaches disk.
            metadata.validate()?;
        }
        self.conn().execute(
            "INSERT INTO search_usage(id,at_ms,service_id,status,usage) VALUES(?1,?2,?3,?4,?5)",
            params![record.id, record.at_ms, record.service_id, record.status, record.usage.as_ref().map(serde_json::to_string).transpose()?],
        )?;
        Ok(())
    }

    /// Inclusive dates, newest attempts first; metadata detail is capped independently
    /// of the full-period aggregates returned by `search_usage_dashboard`.
    pub fn list_search_usage(&self, from_ms: i64, to_ms: i64) -> Result<Vec<SearchUsageRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id,at_ms,service_id,status,usage FROM search_usage WHERE at_ms>=?1 AND at_ms<=?2 ORDER BY at_ms DESC,id DESC LIMIT 100")?;
        let records = stmt.query_map(params![from_ms, to_ms], |row| {
            let usage: Option<String> = row.get(4)?;
            let usage = usage.map(|value| serde_json::from_str(&value)).transpose().map_err(|error| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error)))?;
            Ok(SearchUsageRecord { id: row.get(0)?, at_ms: row.get(1)?, service_id: row.get(2)?, status: row.get(3)?, usage })
        })?;
        Ok(records.collect::<rusqlite::Result<_>>()?)
    }

    /// Stream one ordered SELECT while holding the connection: the detail cap never
    /// truncates totals, and every component observes the same SQLite read snapshot.
    /// Memory grows only with service/bucket pairs plus the latest 100 detail rows.
    pub fn search_usage_dashboard(&self, from_ms: i64, to_ms: i64, bucket_ms: i64) -> Result<SearchUsageDashboard> {
        if bucket_ms <= 0 || from_ms > to_ms {
            return Err(StoreError::Conflict("invalid search usage range or bucket".into()));
        }
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id,at_ms,service_id,status,usage FROM search_usage WHERE at_ms>=?1 AND at_ms<=?2 ORDER BY at_ms DESC,id DESC")?;
        let mut rows = stmt.query(params![from_ms, to_ms])?;
        let mut dashboard = SearchUsageDashboard {
            from_ms, to_ms, bucket_ms, totals: SearchUsageTotals::default(),
            services: Vec::new(), records: Vec::with_capacity(100),
        };
        let mut services: BTreeMap<String, ServiceAccumulator> = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let at_ms: i64 = row.get(1)?;
            let service_id = row.get_ref(2)?.as_str().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
            })?;
            let raw_usage = match row.get_ref(4)? {
                rusqlite::types::ValueRef::Null => None,
                value => Some(value.as_str().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
                })?),
            };
            let usage: Option<UsageMetadata> = raw_usage.map(serde_json::from_str).transpose()?;
            if let Some(usage) = &usage { usage.validate()?; }
            let bucket = at_ms.div_euclid(bucket_ms).checked_mul(bucket_ms)
                .ok_or_else(|| StoreError::Conflict("search usage bucket overflow".into()))?;
            dashboard.totals.add_attempt(usage.as_ref())?;
            if dashboard.records.len() < 100 {
                dashboard.records.push(SearchUsageRecord {
                    id: row.get(0)?, at_ms, service_id: service_id.to_owned(), status: row.get(3)?,
                    usage: raw_usage.map(serde_json::from_str).transpose()?,
                });
            }
            if let Some(service) = services.get_mut(service_id) {
                service.add_attempt(bucket, usage.as_ref())?;
            } else {
                let mut service = ServiceAccumulator::default();
                service.add_attempt(bucket, usage.as_ref())?;
                services.insert(service_id.to_owned(), service);
            }
        }
        dashboard.services = services.into_iter().map(|(service_id, service)| SearchServiceUsage {
            service_id, totals: service.totals, points: service.points.into_values().collect(),
        }).collect();
        Ok(dashboard)
    }
}
