//! SQL-backed administration aggregates; no conversation data or raw usage leaves these queries.
use std::collections::BTreeMap;

use rusqlite::{Row, params};
use serde::Serialize;

use crate::{Result, Store};

#[derive(Clone, Debug, Default, Serialize)]
pub struct UsageTotals {
    pub requests: u64,
    pub cached_in: u64,
    pub uncached_in: u64,
    pub reasoning: u64,
    pub output: u64,
    pub cost_usd: Option<f64>,
    pub unpriced_requests: u64,
}

impl UsageTotals {
    pub fn add(&mut self, other: &Self) {
        self.requests += other.requests;
        self.cached_in += other.cached_in;
        self.uncached_in += other.uncached_in;
        self.reasoning += other.reasoning;
        self.output += other.output;
        self.unpriced_requests += other.unpriced_requests;
        if let Some(cost) = other.cost_usd {
            *self.cost_usd.get_or_insert(0.0) += cost;
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UsagePoint {
    pub at_ms: i64,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelUsageStats {
    pub provider: String,
    pub upstream_id: String,
    pub model_label: String,
    pub totals: UsageTotals,
    pub points: Vec<UsagePoint>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AllowanceWindow {
    pub name: String,
    pub used_usd: Option<f64>,
    pub unknown_requests: u64,
}

#[derive(Clone, Debug)]
pub struct ModelAllowance {
    pub provider: String,
    pub upstream_id: String,
    pub model_label: String,
    pub windows: Vec<AllowanceWindow>,
}

fn count(row: &Row, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn totals(row: &Row, start: usize) -> rusqlite::Result<UsageTotals> {
    Ok(UsageTotals {
        requests: count(row, start)?,
        cached_in: count(row, start + 1)?,
        uncached_in: count(row, start + 2)?,
        reasoning: count(row, start + 3)?,
        output: count(row, start + 4)?,
        cost_usd: row.get(start + 5)?,
        unpriced_requests: count(row, start + 6)?,
    })
}

impl Store {
    /// Half-open interval, epoch-aligned nonempty buckets. Labels come from the latest
    /// historical record, including models subsequently removed from the catalog.
    pub fn usage_stats(
        &self,
        from_ms: i64,
        to_ms: i64,
        bucket_ms: i64,
    ) -> Result<Vec<ModelUsageStats>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "WITH grouped AS (
                SELECT provider, upstream_id,
                    (at_ms / ?3 - CASE WHEN at_ms < 0 AND at_ms % ?3 != 0 THEN 1 ELSE 0 END) * ?3 AS bucket,
                    COUNT(*) AS requests, SUM(cached_in) AS cached_in, SUM(uncached_in) AS uncached_in,
                    SUM(reasoning) AS reasoning, SUM(output) AS output,
                    SUM(COALESCE(upstream_cost_usd, cost_usd)) AS cost_usd,
                    SUM(COALESCE(upstream_cost_usd, cost_usd) IS NULL) AS unpriced_requests
                FROM usage WHERE at_ms >= ?1 AND at_ms < ?2
                GROUP BY provider, upstream_id, bucket
            )
            SELECT g.provider, g.upstream_id,
                (SELECT model_label FROM usage u WHERE u.provider = g.provider AND u.upstream_id = g.upstream_id
                    ORDER BY at_ms DESC, id DESC LIMIT 1),
                bucket, requests, cached_in, uncached_in, reasoning, output, cost_usd, unpriced_requests
            FROM grouped g ORDER BY g.provider, g.upstream_id, bucket"
        )?;
        let rows = stmt.query_map(params![from_ms, to_ms, bucket_ms], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                UsagePoint {
                    at_ms: r.get(3)?,
                    totals: totals(r, 4)?,
                },
            ))
        })?;
        let mut models: BTreeMap<(String, String), ModelUsageStats> = BTreeMap::new();
        for row in rows {
            let (provider, upstream_id, model_label, point) = row?;
            let model = models
                .entry((provider.clone(), upstream_id.clone()))
                .or_insert_with(|| ModelUsageStats {
                    provider,
                    upstream_id,
                    model_label,
                    totals: UsageTotals::default(),
                    points: Vec::new(),
                });
            model.totals.add(&point.totals);
            model.points.push(point);
        }
        Ok(models.into_values().collect())
    }

    /// Three rolling windows, aggregated directly by SQLite. A window with no calls is
    /// known zero; calls with no allowance are counted separately, never priced from tokens.
    pub fn allowance_windows(&self, as_of_ms: i64) -> Result<Vec<ModelAllowance>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "WITH windows(position, name, duration) AS (VALUES (0, '5h', 18000000), (1, '7d', 604800000), (2, '30d', 2592000000)),
            models AS (
                SELECT provider, upstream_id FROM usage WHERE at_ms >= ?1 - 2592000000 AND at_ms < ?1
                GROUP BY provider, upstream_id
            )
            SELECT m.provider, m.upstream_id,
                (SELECT model_label FROM usage u WHERE u.provider = m.provider AND u.upstream_id = m.upstream_id
                    ORDER BY at_ms DESC, id DESC LIMIT 1),
                w.name, CASE WHEN COUNT(u.id) = 0 THEN 0.0 ELSE SUM(u.allowance_usd) END,
                COUNT(u.id) - COUNT(u.allowance_usd)
            FROM models m CROSS JOIN windows w LEFT JOIN usage u
                ON u.provider = m.provider AND u.upstream_id = m.upstream_id
                AND u.at_ms >= ?1 - w.duration AND u.at_ms < ?1
            GROUP BY m.provider, m.upstream_id, w.position ORDER BY m.provider, m.upstream_id, w.position"
        )?;
        let rows = stmt.query_map([as_of_ms], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                AllowanceWindow {
                    name: r.get(3)?,
                    used_usd: r.get(4)?,
                    unknown_requests: count(r, 5)?,
                },
            ))
        })?;
        let mut models: BTreeMap<(String, String), ModelAllowance> = BTreeMap::new();
        for row in rows {
            let (provider, upstream_id, model_label, window) = row?;
            models
                .entry((provider.clone(), upstream_id.clone()))
                .or_insert_with(|| ModelAllowance {
                    provider,
                    upstream_id,
                    model_label,
                    windows: Vec::new(),
                })
                .windows
                .push(window);
        }
        Ok(models.into_values().collect())
    }
}
