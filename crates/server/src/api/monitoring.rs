//! Admin-only usage and provider monitoring. Never serialize provider credentials.
use std::collections::BTreeMap;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use nekochat_gateway::{Provider, USER_AGENT};
use nekochat_store::{AllowanceWindow, ModelAllowance, ModelUsageStats, UsageTotals, now_ms};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, blocking};
use crate::config::{GoPlan, MonitoringKind, ProviderEntry};
use crate::model_defaults::{monthly_limit, official_host};
use crate::state::AppState;

const DAY_MS: i64 = 86_400_000;

#[derive(Default, Deserialize)]
pub struct StatsQuery {
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    bucket_ms: Option<i64>,
}

#[derive(Serialize)]
pub struct Stats {
    from_ms: i64,
    to_ms: i64,
    bucket_ms: i64,
    totals: UsageTotals,
    models: Vec<StatsModel>,
}

#[derive(Serialize)]
struct StatsModel {
    key: String,
    provider_label: String,
    #[serde(flatten)]
    usage: ModelUsageStats,
}

impl StatsQuery {
    fn range(self, now: i64) -> Result<(i64, i64, i64), ApiError> {
        let to = self.to_ms.unwrap_or(now);
        let from = self.from_ms.unwrap_or(to.saturating_sub(7 * DAY_MS));
        let bucket = self.bucket_ms.unwrap_or(3_600_000);
        let span = to
            .checked_sub(from)
            .filter(|v| *v > 0 && *v <= 366 * DAY_MS);
        // Nonnegative epoch times also keep bucket arithmetic in SQLite's integer range.
        if from < 0 || span.is_none() || bucket < 60_000 {
            return Err(ApiError::bad_request("统计时间范围或桶大小无效"));
        }
        // Count the epoch-aligned buckets touched, not just ceil(duration / bucket).
        if (to - 1) / bucket - from / bucket + 1 > 1500 {
            return Err(ApiError::bad_request("统计每模型最多允许1500个时间桶"));
        }
        Ok((from, to, bucket))
    }
}

pub async fn stats(
    State(state): State<AppState>,
    query: Result<Query<StatsQuery>, QueryRejection>,
) -> Result<Json<Stats>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::bad_request("统计查询参数必须是整数"))?;
    let (from_ms, to_ms, bucket_ms) = query.range(now_ms())?;
    let store = state.store.clone();
    let usage = blocking(move || store.usage_stats(from_ms, to_ms, bucket_ms)).await?;
    let mut totals = UsageTotals::default();
    let mut models = Vec::with_capacity(usage.len());
    for usage in usage {
        totals.add(&usage.totals);
        models.push(StatsModel {
            key: serde_json::to_string(&[&usage.provider, &usage.upstream_id])
                .map_err(|_| ApiError::internal())?,
            provider_label: state
                .provider(&usage.provider)
                .map_or_else(|| usage.provider.clone(), |p| p.label.clone()),
            usage,
        });
    }
    Ok(Json(Stats {
        from_ms,
        to_ms,
        bucket_ms,
        totals,
        models,
    }))
}

#[derive(Serialize)]
pub struct Quotas {
    as_of_ms: i64,
    providers: Vec<ProviderQuota>,
}

#[derive(Serialize)]
struct ProviderQuota {
    key: String,
    label: String,
    kind: &'static str,
    status: &'static str,
    note: Option<String>,
    balance: Option<Balance>,
    models: Vec<QuotaModel>,
}

#[derive(Serialize)]
struct Balance {
    available: bool,
    balances: Vec<BalanceAmount>,
}

#[derive(Serialize)]
struct BalanceAmount {
    currency: String,
    total: String,
    granted: String,
    topped_up: String,
}

#[derive(Serialize)]
struct QuotaModel {
    upstream_id: String,
    model_label: String,
    windows: Vec<QuotaWindow>,
}

#[derive(Serialize)]
struct QuotaWindow {
    name: String,
    limit_usd: Option<f64>,
    used_usd: Option<f64>,
    remaining_usd: Option<f64>,
    unknown_requests: u64,
}

fn kind(provider: &Provider, entry: Option<&ProviderEntry>) -> Option<MonitoringKind> {
    entry
        .and_then(|e| e.monitoring.as_ref())
        .map(|m| m.kind)
        .or_else(|| {
            if official_host(&provider.base_url, "api.deepseek.com") {
                Some(MonitoringKind::Deepseek)
            } else if official_host(&provider.base_url, "opencode.ai") {
                Some(MonitoringKind::OpencodeGo)
            } else {
                None
            }
        })
}

fn quota_window(window: AllowanceWindow, monthly: Option<f64>, fraction: f64) -> QuotaWindow {
    let limit_usd = monthly.map(|v| v * fraction);
    QuotaWindow {
        name: window.name,
        limit_usd,
        used_usd: window.used_usd,
        remaining_usd: limit_usd
            .zip(window.used_usd)
            .map(|(limit, used)| (limit - used).max(0.0)),
        unknown_requests: window.unknown_requests,
    }
}

fn empty_windows() -> Vec<AllowanceWindow> {
    ["5h", "7d", "30d"]
        .into_iter()
        .map(|name| AllowanceWindow {
            name: name.into(),
            used_usd: Some(0.0),
            unknown_requests: 0,
        })
        .collect()
}

pub async fn quotas(State(state): State<AppState>) -> Result<Json<Quotas>, ApiError> {
    let as_of_ms = now_ms();
    let store = state.store.clone();
    let (allowances, listed) =
        blocking(move || Ok((store.allowance_windows(as_of_ms)?, store.list_models()?))).await?;
    let mut providers = Vec::with_capacity(state.providers.len());
    // At most one request per DeepSeek provider, each with a total deadline and response cap.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::internal())?;
    for provider in state.providers.iter() {
        let entry = state
            .config
            .providers
            .iter()
            .find(|p| p.key == provider.key);
        let monitoring = entry.and_then(|e| e.monitoring.as_ref());
        let mut quota = ProviderQuota {
            key: provider.key.clone(),
            label: provider.label.clone(),
            kind: "unknown",
            status: "unavailable",
            note: Some("此提供商没有可用的额度监控适配器".into()),
            balance: None,
            models: Vec::new(),
        };
        match kind(provider, entry) {
            Some(MonitoringKind::Deepseek) => {
                quota.kind = "deepseek";
                let url = monitoring.and_then(|m| m.balance_url.clone()).or_else(|| {
                    // The official endpoint is origin-relative even when chat uses /v1.
                    let mut base = reqwest::Url::parse(&provider.base_url).ok()?;
                    if official_host(&provider.base_url, "api.deepseek.com") {
                        base.set_path("/user/balance");
                        base.set_query(None);
                        base.set_fragment(None);
                        Some(base.to_string())
                    } else {
                        None
                    }
                });
                if let Some(url) = url {
                    match deepseek_balance(&client, provider, &url).await {
                        Ok(balance) => {
                            quota.status = "ok";
                            quota.note = None;
                            quota.balance = Some(balance);
                        }
                        Err(note) => {
                            quota.status = "error";
                            quota.note = Some(note.into());
                        }
                    }
                } else {
                    quota.note =
                        Some("自定义DeepSeek地址需要明确配置monitoring.balance_url".into());
                }
            }
            Some(MonitoringKind::OpencodeGo) => {
                quota.kind = "opencode_go";
                quota.status = "ok";
                let plan = monitoring.map_or(GoPlan::Go, |m| m.plan);
                quota.note = Some(format!(
                    "{} · 本机滚动窗口估算，不含其他客户端用量；缺失额度计数时已知用量和剩余额度仅供参考，非账户权威余额",
                    match plan {
                        GoPlan::Go => "Go（默认）",
                        GoPlan::GoPlus => "Go Plus",
                    }
                ));
                let mut models: BTreeMap<String, ModelAllowance> = allowances
                    .iter()
                    .filter(|m| m.provider == provider.key)
                    .map(|m| (m.upstream_id.clone(), m.clone()))
                    .collect();
                for model in listed.iter().filter(|m| m.spec.provider == provider.key) {
                    models
                        .entry(model.spec.upstream_id.clone())
                        .or_insert_with(|| ModelAllowance {
                            provider: provider.key.clone(),
                            upstream_id: model.spec.upstream_id.clone(),
                            model_label: model.spec.display_name.clone(),
                            windows: empty_windows(),
                        });
                }
                quota.models = models
                    .into_values()
                    .map(|model| {
                        let monthly = monitoring
                            .and_then(|m| m.monthly_limits.get(&model.upstream_id).copied())
                            .or_else(|| monthly_limit(&model.upstream_id, plan));
                        QuotaModel {
                            upstream_id: model.upstream_id,
                            model_label: model.model_label,
                            windows: model
                                .windows
                                .into_iter()
                                .zip([0.2, 0.5, 1.0])
                                .map(|(w, fraction)| quota_window(w, monthly, fraction))
                                .collect(),
                        }
                    })
                    .collect();
            }
            None => {}
        }
        providers.push(quota);
    }
    Ok(Json(Quotas {
        as_of_ms,
        providers,
    }))
}

#[derive(Deserialize)]
struct DeepSeekBalance {
    is_available: bool,
    balance_infos: Vec<DeepSeekAmount>,
}
#[derive(Deserialize)]
struct DeepSeekAmount {
    currency: String,
    total_balance: String,
    granted_balance: String,
    topped_up_balance: String,
}

async fn deepseek_balance(
    client: &reqwest::Client,
    provider: &Provider,
    url: &str,
) -> Result<Balance, &'static str> {
    let mut response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .bearer_auth(&provider.api_key)
        .send()
        .await
        .map_err(|_| "余额请求失败或超时")?;
    if !response.status().is_success() {
        return Err("上游拒绝了余额请求");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取余额失败或超时")? {
        if body.len() + chunk.len() > 65_536 {
            return Err("上游余额响应过大");
        }
        body.extend_from_slice(&chunk);
    }
    let raw: DeepSeekBalance = serde_json::from_slice(&body).map_err(|_| "上游余额格式无效")?;
    let mut balances = Vec::with_capacity(raw.balance_infos.len());
    for amount in raw.balance_infos {
        // Only the documented currency and numeric amounts reach the browser; arbitrary
        // upstream strings (including error details or reflected credentials) never do.
        if !matches!(amount.currency.as_str(), "USD" | "CNY")
            || [
                &amount.total_balance,
                &amount.granted_balance,
                &amount.topped_up_balance,
            ]
            .iter()
            .any(|v| {
                v.len() > 64
                    || v.is_empty()
                    || !v
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
                    || v.parse::<f64>().map_or(true, |n| !n.is_finite())
            })
        {
            return Err("上游余额格式无效");
        }
        balances.push(BalanceAmount {
            currency: amount.currency,
            total: amount.total_balance,
            granted: amount.granted_balance,
            topped_up: amount.topped_up_balance,
        });
    }
    Ok(Balance {
        available: raw.is_available,
        balances,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_allowance_stays_unknown_and_partial_remaining_is_clamped() {
        let missing = quota_window(
            AllowanceWindow {
                name: "5h".into(),
                used_usd: None,
                unknown_requests: 1,
            },
            Some(60.0),
            0.2,
        );
        assert_eq!(missing.remaining_usd, None);
        let partial = quota_window(
            AllowanceWindow {
                name: "5h".into(),
                used_usd: Some(13.0),
                unknown_requests: 2,
            },
            Some(60.0),
            0.2,
        );
        assert_eq!(
            (
                partial.limit_usd,
                partial.remaining_usd,
                partial.unknown_requests
            ),
            (Some(12.0), Some(0.0), 2)
        );
        assert_eq!(monthly_limit("glm-5.2", GoPlan::GoPlus), Some(180.0));
    }

    #[test]
    fn epoch_aligned_bucket_limit_and_invalid_ranges_are_rejected() {
        assert!(
            StatsQuery {
                from_ms: Some(1),
                to_ms: Some(90_000_001),
                bucket_ms: Some(60_000)
            }
            .range(0)
            .is_err()
        );
        assert!(
            StatsQuery {
                from_ms: Some(0),
                to_ms: Some(90_000_000),
                bucket_ms: Some(60_000)
            }
            .range(0)
            .is_ok()
        );
        assert!(
            StatsQuery {
                from_ms: Some(i64::MIN),
                to_ms: Some(i64::MAX),
                bucket_ms: None
            }
            .range(0)
            .is_err()
        );
    }
}
