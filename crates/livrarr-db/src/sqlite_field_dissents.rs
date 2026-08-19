use chrono::{DateTime, Utc};
use livrarr_domain::{DissentReason, FieldDissent};

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, FieldDissentDb, UserId, WorkId};

fn reason_str(reason: DissentReason) -> &'static str {
    match reason {
        DissentReason::PayloadMismatch => "payload_mismatch",
        DissentReason::FieldConflict => "field_conflict",
        DissentReason::LanguageIncompatible => "language_incompatible",
    }
}

fn reason_from_str(raw: &str) -> Result<DissentReason, DbError> {
    match raw {
        "payload_mismatch" => Ok(DissentReason::PayloadMismatch),
        "field_conflict" => Ok(DissentReason::FieldConflict),
        "language_incompatible" => Ok(DissentReason::LanguageIncompatible),
        other => Err(DbError::Constraint {
            message: format!("unknown dissent reason: {other}"),
        }),
    }
}

/// (work_id, provider, field, offered_value, winning_value, reason,
/// merge_generation, recorded_at) as stored.
type DissentRow = (
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    String,
);

impl FieldDissentDb for SqliteDb {
    async fn record_field_dissents(
        &self,
        user_id: UserId,
        work_id: WorkId,
        dissents: Vec<FieldDissent>,
    ) -> Result<(), DbError> {
        // The batch's generation supersedes all older rows for the work
        // (REQ-014); the store always reflects the latest completed merge. An
        // empty batch means the merge produced no dissents — stale rows from
        // earlier generations are cleared.
        let Some(generation) = dissents.iter().map(|d| d.merge_generation).max() else {
            sqlx::query("DELETE FROM work_field_dissents WHERE user_id = ? AND work_id = ?")
                .bind(user_id)
                .bind(work_id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
            return Ok(());
        };
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;
        sqlx::query(
            "DELETE FROM work_field_dissents \
             WHERE user_id = ? AND work_id = ? AND merge_generation < ?",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(generation)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;
        for d in &dissents {
            sqlx::query(
                "INSERT INTO work_field_dissents \
                 (user_id, work_id, provider, field, offered_value, winning_value, \
                  reason, merge_generation, recorded_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(user_id)
            .bind(work_id)
            .bind(&d.provider)
            .bind(&d.field)
            .bind(&d.offered_value)
            .bind(d.winning_value.as_deref())
            .bind(reason_str(d.reason))
            .bind(d.merge_generation)
            .bind(d.recorded_at.to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }
        tx.commit().await.map_err(map_db_err)
    }

    async fn list_field_dissents(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<FieldDissent>, DbError> {
        let rows: Vec<DissentRow> = sqlx::query_as(
            "SELECT work_id, provider, field, offered_value, winning_value, \
                    reason, merge_generation, recorded_at \
             FROM work_field_dissents \
             WHERE user_id = ? AND work_id = ? AND merge_generation = \
                   (SELECT MAX(merge_generation) FROM work_field_dissents \
                    WHERE user_id = ? AND work_id = ?) \
             ORDER BY id",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(user_id)
        .bind(work_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter()
            .map(
                |(
                    work_id,
                    provider,
                    field,
                    offered_value,
                    winning_value,
                    reason,
                    generation,
                    recorded_at,
                )| {
                    Ok(FieldDissent {
                        work_id,
                        provider,
                        field,
                        offered_value,
                        winning_value,
                        reason: reason_from_str(&reason)?,
                        merge_generation: generation,
                        recorded_at: DateTime::parse_from_rfc3339(&recorded_at)
                            .map_err(|e| DbError::Io(Box::new(e)))?
                            .with_timezone(&Utc),
                    })
                },
            )
            .collect()
    }
}
