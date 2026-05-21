use chrono::{DateTime, Utc};
use livrarr_domain::identity::*;
use livrarr_domain::{UserId, WorkId};

use crate::sqlite::SqliteDb;

impl SqliteDb {
    pub async fn find_existing_open_conflict(
        &self,
        user_id: UserId,
        existing_work_id: WorkId,
        incoming_ol_key: &str,
    ) -> Result<Option<i64>, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM work_identity_conflicts
             WHERE user_id = ?1 AND existing_work_id = ?2 AND status = 'open'
             AND json_extract(incoming_payload_json, '$.ol_key') = ?3
             ORDER BY id DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(existing_work_id)
        .bind(incoming_ol_key)
        .fetch_optional(self.pool())
        .await?;

        Ok(row.map(|(id,)| id))
    }

    pub async fn create_identity_conflict(
        &self,
        user_id: UserId,
        existing_work_id: WorkId,
        kind: IdentityConflictKind,
        incoming_json: &str,
        raised_at: DateTime<Utc>,
        raised_by: ConflictSource,
        raised_source_path: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        let kind_str = serde_json::to_value(&kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "incoming_different_ol_key".to_string());
        let raised_by_str = serde_json::to_value(&raised_by)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "manual_add".to_string());
        let raised_at_str = raised_at.to_rfc3339();

        let result = sqlx::query(
            "INSERT INTO work_identity_conflicts
             (user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, raised_source_path, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open')",
        )
        .bind(user_id)
        .bind(existing_work_id)
        .bind(&kind_str)
        .bind(incoming_json)
        .bind(&raised_at_str)
        .bind(&raised_by_str)
        .bind(raised_source_path)
        .execute(self.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn list_identity_conflicts_by_status(
        &self,
        user_id: UserId,
        status: ConflictStatus,
    ) -> Result<Vec<IdentityConflict>, sqlx::Error> {
        let status_str = serde_json::to_value(&status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "open".to_string());

        let rows: Vec<(i64, i64, i64, String, String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT id, user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, raised_source_path, status, resolved_at, resolution_action, resolution_notes
                 FROM work_identity_conflicts
                 WHERE user_id = ?1 AND status = ?2
                 ORDER BY id DESC",
            )
            .bind(user_id)
            .bind(&status_str)
            .fetch_all(self.pool())
            .await?;

        Ok(rows
            .into_iter()
            .filter_map(|r| parse_conflict_row(r).ok())
            .collect())
    }

    pub async fn get_identity_conflict(
        &self,
        id: i64,
        user_id: UserId,
    ) -> Result<Option<IdentityConflict>, sqlx::Error> {
        let row: Option<(i64, i64, i64, String, String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT id, user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, raised_source_path, status, resolved_at, resolution_action, resolution_notes
                 FROM work_identity_conflicts WHERE id = ?1 AND user_id = ?2",
            )
            .bind(id)
            .bind(user_id)
            .fetch_optional(self.pool())
            .await?;

        Ok(row.and_then(|r| parse_conflict_row(r).ok()))
    }

    pub async fn resolve_identity_conflict(
        &self,
        id: i64,
        action: ConflictResolutionAction,
        notes: Option<&str>,
        resolved_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let action_str = serde_json::to_value(&action)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "keep_existing".to_string());
        let resolved_at_str = resolved_at.to_rfc3339();

        sqlx::query(
            "UPDATE work_identity_conflicts
             SET status = 'resolved', resolved_at = ?1, resolution_action = ?2, resolution_notes = ?3
             WHERE id = ?4",
        )
        .bind(&resolved_at_str)
        .bind(&action_str)
        .bind(notes)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    pub async fn dismiss_identity_conflict(
        &self,
        id: i64,
        dismissed_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let dismissed_at_str = dismissed_at.to_rfc3339();

        sqlx::query(
            "UPDATE work_identity_conflicts
             SET status = 'dismissed', resolved_at = ?1
             WHERE id = ?2",
        )
        .bind(&dismissed_at_str)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(())
    }
}

fn parse_conflict_row(
    row: (
        i64,
        i64,
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
) -> Result<IdentityConflict, String> {
    let (
        id,
        user_id,
        existing_work_id,
        kind_str,
        payload_json,
        raised_at_str,
        raised_by_str,
        source_path,
        status_str,
        resolved_at_str,
        action_str,
        notes,
    ) = row;

    let kind: IdentityConflictKind =
        serde_json::from_value(serde_json::Value::String(kind_str)).map_err(|e| e.to_string())?;
    let incoming: IncomingConflictPayload =
        serde_json::from_str(&payload_json).map_err(|e| e.to_string())?;
    let raised_at = DateTime::parse_from_rfc3339(&raised_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| e.to_string())?;
    let raised_by: ConflictSource =
        serde_json::from_value(serde_json::Value::String(raised_by_str))
            .map_err(|e| e.to_string())?;
    let status: ConflictStatus =
        serde_json::from_value(serde_json::Value::String(status_str)).map_err(|e| e.to_string())?;
    let resolved_at = resolved_at_str
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let resolution_action =
        action_str.and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok());

    Ok(IdentityConflict {
        id,
        user_id,
        existing_work_id,
        kind,
        incoming,
        raised_at,
        raised_by,
        raised_source_path: source_path,
        status,
        resolved_at,
        resolution_action,
        resolution_notes: notes,
    })
}
