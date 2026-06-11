use chrono::{DateTime, Duration, Utc};
use livrarr_domain::services::{CallOperation, CallOutcomeClass};

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, ProviderCallRecord, ProviderCallRecordDb, ProviderStats, RetentionPolicy};

/// Storage tokens match the serde `snake_case` vocabulary so the table and the
/// status API speak one language (REQ-001).
fn operation_str(op: CallOperation) -> &'static str {
    match op {
        CallOperation::Lookup => "lookup",
        CallOperation::Identity => "identity",
        CallOperation::Enrich => "enrich",
        CallOperation::Cover => "cover",
    }
}

fn outcome_str(outcome: CallOutcomeClass) -> &'static str {
    match outcome {
        CallOutcomeClass::Success => "success",
        CallOutcomeClass::NotFound => "not_found",
        CallOutcomeClass::RateLimited => "rate_limited",
        CallOutcomeClass::Timeout => "timeout",
        CallOutcomeClass::Error => "error",
        CallOutcomeClass::SkippedNoAnchor => "skipped_no_anchor",
        CallOutcomeClass::SkippedPolicy => "skipped_policy",
        CallOutcomeClass::LlmRejected => "llm_rejected",
        CallOutcomeClass::Cached => "cached",
    }
}

/// Network outcomes: the request actually went out. Skips and cache hits are
/// excluded from the latency and success-rate denominators (REQ-002).
fn is_network(outcome: &str) -> bool {
    matches!(
        outcome,
        "success" | "not_found" | "rate_limited" | "timeout" | "error"
    )
}

fn is_error(outcome: &str) -> bool {
    matches!(outcome, "rate_limited" | "timeout" | "error")
}

fn parse_started_at(raw: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| DbError::Io(Box::new(e)))
}

/// (started_at, duration_ms, outcome, detail) per record in the 24h window.
type CallRow = (DateTime<Utc>, i64, String, Option<String>);

impl ProviderCallRecordDb for SqliteDb {
    async fn record_provider_calls(&self, batch: Vec<ProviderCallRecord>) -> Result<(), DbError> {
        if batch.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        for rec in &batch {
            sqlx::query(
                "INSERT INTO provider_call_records \
                 (provider, operation, work_id, started_at, duration_ms, outcome, detail) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&rec.provider)
            .bind(operation_str(rec.operation))
            .bind(rec.work_id)
            .bind(rec.started_at.to_rfc3339())
            .bind(rec.duration_ms)
            .bind(outcome_str(rec.outcome))
            .bind(rec.detail.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }
        tx.commit().await.map_err(map_db_err)
    }

    async fn query_provider_stats_24h(&self) -> Result<Vec<ProviderStats>, DbError> {
        let cutoff = (Utc::now() - Duration::hours(24)).to_rfc3339();
        let rows: Vec<(String, String, i64, String, Option<String>)> = sqlx::query_as(
            "SELECT provider, started_at, duration_ms, outcome, detail \
             FROM provider_call_records \
             WHERE datetime(started_at) >= datetime(?)",
        )
        .bind(&cutoff)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        let mut by_provider: std::collections::BTreeMap<String, Vec<CallRow>> =
            std::collections::BTreeMap::new();
        for (provider, started_at, duration_ms, outcome, detail) in rows {
            let started_at = parse_started_at(&started_at)?;
            by_provider.entry(provider).or_default().push((
                started_at,
                duration_ms,
                outcome,
                detail,
            ));
        }

        let mut stats = Vec::with_capacity(by_provider.len());
        for (provider, rows) in by_provider {
            let calls_24h = rows.len() as i64;
            let mut network_durations: Vec<i64> = Vec::new();
            let mut successes = 0i64;
            let mut last_error: Option<(String, DateTime<Utc>)> = None;
            let mut last_success: Option<DateTime<Utc>> = None;
            for (started_at, duration_ms, outcome, detail) in &rows {
                if is_network(outcome) {
                    network_durations.push(*duration_ms);
                }
                if outcome == "success" && last_success.is_none_or(|prev| *started_at > prev) {
                    successes += 1;
                    last_success = Some(*started_at);
                } else if outcome == "success" {
                    successes += 1;
                }
                if is_error(outcome)
                    && last_error
                        .as_ref()
                        .is_none_or(|(_, prev)| *started_at > *prev)
                {
                    last_error = Some((detail.clone().unwrap_or_default(), *started_at));
                }
            }
            network_durations.sort_unstable();
            let median_latency_ms = match network_durations.len() {
                0 => 0,
                n if n % 2 == 1 => network_durations[n / 2],
                n => (network_durations[n / 2 - 1] + network_durations[n / 2]) / 2,
            };
            let network = network_durations.len() as i64;
            let success_rate = if network > 0 {
                successes as f64 / network as f64
            } else {
                0.0
            };
            stats.push(ProviderStats {
                provider,
                calls_24h,
                success_rate,
                median_latency_ms,
                last_error,
                last_success,
            });
        }
        Ok(stats)
    }

    async fn evict_call_records(&self, policy: RetentionPolicy) -> Result<u64, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        let cutoff = (Utc::now() - Duration::days(i64::from(policy.max_age_days))).to_rfc3339();
        let by_age = sqlx::query(
            "DELETE FROM provider_call_records WHERE datetime(started_at) < datetime(?)",
        )
        .bind(&cutoff)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?
        .rows_affected();

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_call_records")
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;
        let excess = remaining - policy.max_records as i64;
        let by_count = if excess > 0 {
            sqlx::query(
                "DELETE FROM provider_call_records WHERE id IN (\
                 SELECT id FROM provider_call_records \
                 ORDER BY datetime(started_at) ASC, id ASC LIMIT ?)",
            )
            .bind(excess)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?
            .rows_affected()
        } else {
            0
        };
        tx.commit().await.map_err(map_db_err)?;
        Ok(by_age + by_count)
    }
}
