use async_trait::async_trait;
use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{DbError, MetadataProvider, OutcomeClass, ProviderRetryState, UserId, WorkId};

// ---------------------------------------------------------------------------
// Enum ↔ string helpers
// ---------------------------------------------------------------------------

fn to_str<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .expect("enum serialization is infallible")
        .as_str()
        .expect("enum serializes to string")
        .to_string()
}

fn from_str<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, DbError> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| {
        DbError::IncompatibleData {
            detail: e.to_string(),
        }
    })
}

// ---------------------------------------------------------------------------
// Row → ProviderRetryState
// ---------------------------------------------------------------------------

fn row_to_retry_state(row: sqlx::sqlite::SqliteRow) -> Result<ProviderRetryState, DbError> {
    let provider_str: String = row
        .try_get("provider")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let last_outcome_str: Option<String> = row
        .try_get("last_outcome")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let next_attempt_str: Option<String> = row
        .try_get("next_attempt_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let first_suppressed_str: Option<String> = row
        .try_get("first_suppressed_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(ProviderRetryState {
        user_id: row
            .try_get("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        provider: from_str(&provider_str)?,
        attempts: row
            .try_get::<i64, _>("attempts")
            .map_err(|e| DbError::Io(Box::new(e)))? as u32,
        suppressed_passes: row
            .try_get::<i64, _>("suppressed_passes")
            .map_err(|e| DbError::Io(Box::new(e)))? as u32,
        last_outcome: last_outcome_str.as_deref().map(from_str).transpose()?,
        next_attempt_at: next_attempt_str.as_deref().map(parse_dt).transpose()?,
        first_suppressed_at: first_suppressed_str.as_deref().map(parse_dt).transpose()?,
        normalized_payload_json: row
            .try_get("normalized_payload_json")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

// ---------------------------------------------------------------------------
// ProviderRetryStateDb impl
// ---------------------------------------------------------------------------

#[async_trait]
impl crate::ProviderRetryStateDb for SqliteDb {
    async fn get_retry_state(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
    ) -> Result<Option<ProviderRetryState>, DbError> {
        let provider_str = to_str(provider);
        let row = sqlx::query(
            "SELECT prs.user_id, prs.work_id, prs.provider, prs.attempts, \
             prs.suppressed_passes, prs.last_outcome, prs.last_attempt_at, \
             prs.next_attempt_at, prs.normalized_payload_json, prs.first_suppressed_at \
             FROM provider_retry_state prs \
             JOIN works w ON prs.work_id = w.id \
             WHERE prs.work_id = ? AND prs.provider = ? AND w.user_id = ?",
        )
        .bind(work_id)
        .bind(&provider_str)
        .bind(user_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        row.map(row_to_retry_state).transpose()
    }

    async fn list_retry_states(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ProviderRetryState>, DbError> {
        let rows = sqlx::query(
            "SELECT prs.user_id, prs.work_id, prs.provider, prs.attempts, \
             prs.suppressed_passes, prs.last_outcome, prs.last_attempt_at, \
             prs.next_attempt_at, prs.normalized_payload_json, prs.first_suppressed_at \
             FROM provider_retry_state prs \
             JOIN works w ON prs.work_id = w.id \
             WHERE prs.work_id = ? AND w.user_id = ?",
        )
        .bind(work_id)
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter().map(row_to_retry_state).collect()
    }

    async fn record_will_retry(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderRetryState, DbError> {
        let provider_str = to_str(provider);
        let next_str = next_attempt_at.to_rfc3339();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO provider_retry_state \
             (user_id, work_id, provider, attempts, next_attempt_at, \
             normalized_payload_json, first_suppressed_at, last_attempt_at, last_outcome) \
             VALUES (?, ?, ?, 1, ?, NULL, NULL, ?, 'will_retry') \
             ON CONFLICT(work_id, provider) DO UPDATE SET \
             attempts = provider_retry_state.attempts + 1, \
             next_attempt_at = excluded.next_attempt_at, \
             normalized_payload_json = NULL, \
             first_suppressed_at = NULL, \
             last_attempt_at = excluded.last_attempt_at, \
             last_outcome = 'will_retry'",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(&provider_str)
        .bind(&next_str)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_retry_state(user_id, work_id, provider)
            .await?
            .ok_or(DbError::NotFound {
                entity: "provider_retry_state",
            })
    }

    async fn record_suppressed(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderRetryState, DbError> {
        let provider_str = to_str(provider);
        let until_str = until.to_rfc3339();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO provider_retry_state \
             (user_id, work_id, provider, suppressed_passes, next_attempt_at, \
             first_suppressed_at, last_attempt_at, last_outcome) \
             VALUES (?, ?, ?, 1, ?, ?, ?, 'suppressed') \
             ON CONFLICT(work_id, provider) DO UPDATE SET \
             suppressed_passes = provider_retry_state.suppressed_passes + 1, \
             next_attempt_at = excluded.next_attempt_at, \
             first_suppressed_at = COALESCE(provider_retry_state.first_suppressed_at, \
                                            excluded.first_suppressed_at), \
             last_attempt_at = excluded.last_attempt_at, \
             last_outcome = 'suppressed'",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(&provider_str)
        .bind(&until_str)
        .bind(&now) // first_suppressed_at = now on initial insert
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_retry_state(user_id, work_id, provider)
            .await?
            .ok_or(DbError::NotFound {
                entity: "provider_retry_state",
            })
    }

    async fn record_terminal_outcome(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        outcome: OutcomeClass,
        normalized_payload_json: Option<String>,
    ) -> Result<(), DbError> {
        // Validate: only terminal outcomes allowed.
        if !outcome.is_phase2_terminal() {
            return Err(DbError::Constraint {
                message: format!("outcome {outcome:?} is not terminal — use record_will_retry or record_suppressed"),
            });
        }

        // Validate: Success requires payload; non-Success forbids payload.
        match outcome {
            OutcomeClass::Success => {
                if normalized_payload_json.is_none() {
                    return Err(DbError::Constraint {
                        message: "Success outcome requires normalized_payload_json".to_string(),
                    });
                }
            }
            _ => {
                if normalized_payload_json.is_some() {
                    return Err(DbError::Constraint {
                        message: format!(
                            "non-Success outcome {outcome:?} must not have normalized_payload_json"
                        ),
                    });
                }
            }
        }

        let provider_str = to_str(provider);
        let outcome_str = to_str(outcome);
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO provider_retry_state \
             (user_id, work_id, provider, last_outcome, normalized_payload_json, \
             next_attempt_at, first_suppressed_at, last_attempt_at) \
             VALUES (?, ?, ?, ?, ?, NULL, NULL, ?) \
             ON CONFLICT(work_id, provider) DO UPDATE SET \
             last_outcome = excluded.last_outcome, \
             normalized_payload_json = excluded.normalized_payload_json, \
             next_attempt_at = NULL, \
             first_suppressed_at = NULL, \
             last_attempt_at = excluded.last_attempt_at",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(&provider_str)
        .bind(&outcome_str)
        .bind(&normalized_payload_json)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn reset_all_retry_states(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        sqlx::query("DELETE FROM provider_retry_state WHERE work_id = ? AND user_id = ?")
            .bind(work_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        Ok(())
    }

    async fn list_works_due_for_retry(
        &self,
        user_id: UserId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(WorkId, MetadataProvider)>, DbError> {
        let now_str = now.to_rfc3339();
        let rows = sqlx::query(
            "SELECT prs.work_id, prs.provider \
             FROM provider_retry_state prs \
             JOIN works w ON prs.work_id = w.id \
             WHERE w.user_id = ? AND prs.next_attempt_at IS NOT NULL \
             AND prs.next_attempt_at <= ? \
             ORDER BY w.added_at ASC, prs.work_id ASC",
        )
        .bind(user_id)
        .bind(&now_str)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter()
            .map(|row| {
                let work_id: WorkId = row
                    .try_get("work_id")
                    .map_err(|e| DbError::Io(Box::new(e)))?;
                let provider_str: String = row
                    .try_get("provider")
                    .map_err(|e| DbError::Io(Box::new(e)))?;
                let provider: MetadataProvider = from_str(&provider_str)?;
                Ok((work_id, provider))
            })
            .collect()
    }

    async fn list_works_with_terminal_provider_rows(
        &self,
        user_id: UserId,
    ) -> Result<Vec<(WorkId, Vec<MetadataProvider>)>, DbError> {
        let rows = sqlx::query(
            "SELECT prs.work_id, prs.provider \
             FROM provider_retry_state prs \
             JOIN works w ON prs.work_id = w.id \
             WHERE w.user_id = ? \
             AND prs.last_outcome IN ('success', 'not_found', 'permanent_failure') \
             AND NOT EXISTS ( \
                 SELECT 1 FROM provider_retry_state prs2 \
                 WHERE prs2.work_id = prs.work_id \
                 AND (prs2.last_outcome NOT IN ('success', 'not_found', 'permanent_failure') \
                      OR prs2.last_outcome IS NULL) \
             ) \
             ORDER BY prs.work_id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        // Group by work_id.
        let mut result: Vec<(WorkId, Vec<MetadataProvider>)> = Vec::new();
        for row in rows {
            let work_id: WorkId = row
                .try_get("work_id")
                .map_err(|e| DbError::Io(Box::new(e)))?;
            let provider_str: String = row
                .try_get("provider")
                .map_err(|e| DbError::Io(Box::new(e)))?;
            let provider: MetadataProvider = from_str(&provider_str)?;

            if let Some(entry) = result.iter_mut().find(|(wid, _)| *wid == work_id) {
                entry.1.push(provider);
            } else {
                result.push((work_id, vec![provider]));
            }
        }

        Ok(result)
    }
}
