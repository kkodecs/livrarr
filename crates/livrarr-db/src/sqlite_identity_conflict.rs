use chrono::{DateTime, Utc};
use livrarr_domain::identity::*;
use livrarr_domain::{IdentityStatus, UserId, WorkId};
use sqlx::SqliteConnection;

use crate::sqlite::SqliteDb;

#[allow(clippy::type_complexity)]
type ConflictRow = (
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
);

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

    #[allow(clippy::too_many_arguments)]
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
        let kind_str = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "incoming_different_ol_key".to_string());
        let raised_by_str = serde_json::to_value(raised_by)
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
        let status_str = serde_json::to_value(status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "open".to_string());

        let rows: Vec<ConflictRow> =
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
        let row: Option<ConflictRow> =
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
        let action_str = serde_json::to_value(action)
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

    /// Apply a resolution action, update the conflict row, and recompute the work's
    /// identity badge — all in one atomic transaction.
    ///
    /// Every affirmative resolution is a user decision; the resulting anchor is
    /// stamped `AnchorSetter::User` so future detection passes skip it (fix #1).
    pub async fn apply_conflict_resolution(
        &self,
        conflict: &IdentityConflict,
        action: ConflictResolutionAction,
        notes: Option<&str>,
        resolved_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        if conflict.kind == IdentityConflictKind::QuorumTie {
            // QuorumTie (existing_work_id = 0) is out-of-scope for this resolution
            // path; the four standard actions do not cleanly map to it.
            return self
                .resolve_identity_conflict(conflict.id, action, notes, resolved_at)
                .await;
        }

        let work_id = conflict.existing_work_id;
        let now = resolved_at.to_rfc3339();
        let action_str = serde_json::to_value(action)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "keep_existing".to_string());

        let mut tx = self.pool().begin().await?;

        match action {
            ConflictResolutionAction::KeepExisting | ConflictResolutionAction::AcceptSeparate => {
                // Re-stamp the kept anchor as User so future machine passes cannot
                // raise the same conflict again (part 1 protection).
                if let Some(at) = anchor_type_for_kind(conflict.kind) {
                    sqlx::query(
                        "UPDATE work_identity_anchors
                         SET setter = 'user', set_at = ?1
                         WHERE work_id = ?2 AND anchor_type = ?3 AND confidence = 'confirmed'",
                    )
                    .bind(&now)
                    .bind(work_id)
                    .bind(at)
                    .execute(&mut *tx)
                    .await?;
                }
            }

            ConflictResolutionAction::ReplaceAnchor => {
                // Supersede the existing anchor with the incoming value, stamped User.
                if let Some(at) = anchor_type_for_kind(conflict.kind) {
                    if let Some(incoming_val) =
                        incoming_value_for_kind(conflict.kind, &conflict.incoming)
                    {
                        // Mark the old confirmed anchor as superseded
                        sqlx::query(
                            "UPDATE work_identity_anchors
                             SET confidence = 'superseded', superseded_by = ?1
                             WHERE work_id = ?2 AND anchor_type = ?3 AND confidence = 'confirmed'",
                        )
                        .bind(incoming_val)
                        .bind(work_id)
                        .bind(at)
                        .execute(&mut *tx)
                        .await?;

                        // Insert the new confirmed anchor (User-stamped)
                        sqlx::query(
                            "INSERT INTO work_identity_anchors
                             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
                             VALUES (?1, ?2, ?3, 'confirmed', 'user', ?4,
                                     (SELECT user_id FROM works WHERE id = ?1))
                             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                                 confidence = 'confirmed', setter = 'user', set_at = ?4,
                                 superseded_by = NULL,
                                 user_id = (SELECT user_id FROM works WHERE id = ?1)",
                        )
                        .bind(work_id)
                        .bind(at)
                        .bind(incoming_val)
                        .bind(&now)
                        .execute(&mut *tx)
                        .await?;

                        // Sync the denormalized works column
                        update_works_column_in_tx(&mut tx, at, incoming_val, work_id).await?;
                    }
                }
            }

            ConflictResolutionAction::Merge => {
                // Confirm all non-null incoming anchors with User setter.
                // For each type, supersede any existing confirmed anchor that
                // differs in value first — the partial unique index
                // (work_id, anchor_type WHERE confidence='confirmed') only
                // allows one confirmed row per type.
                let anchors_to_merge: &[(&str, Option<&str>)] = &[
                    (AnchorType::OL_WORK, conflict.incoming.ol_key.as_deref()),
                    (AnchorType::GR_WORK, conflict.incoming.gr_key.as_deref()),
                    (AnchorType::HC_WORK, conflict.incoming.hc_key.as_deref()),
                    (AnchorType::ISBN_13, conflict.incoming.isbn_13.as_deref()),
                    (AnchorType::ASIN, conflict.incoming.asin.as_deref()),
                ];
                for &(at, val) in anchors_to_merge {
                    if let Some(val) = val.filter(|v| !v.is_empty()) {
                        // Clear the slot for any same-type confirmed anchor that
                        // has a different value (do not touch same-value rows).
                        sqlx::query(
                            "UPDATE work_identity_anchors
                             SET confidence = 'superseded', superseded_by = ?1
                             WHERE work_id = ?2 AND anchor_type = ?3
                               AND confidence = 'confirmed' AND anchor_value != ?1",
                        )
                        .bind(val)
                        .bind(work_id)
                        .bind(at)
                        .execute(&mut *tx)
                        .await?;

                        sqlx::query(
                            "INSERT INTO work_identity_anchors
                             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
                             VALUES (?1, ?2, ?3, 'confirmed', 'user', ?4,
                                     (SELECT user_id FROM works WHERE id = ?1))
                             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                                 confidence = 'confirmed', setter = 'user', set_at = ?4,
                                 superseded_by = NULL,
                                 user_id = (SELECT user_id FROM works WHERE id = ?1)",
                        )
                        .bind(work_id)
                        .bind(at)
                        .bind(val)
                        .bind(&now)
                        .execute(&mut *tx)
                        .await?;

                        update_works_column_in_tx(&mut tx, at, val, work_id).await?;
                    }
                }
            }
        }

        // Mark the conflict row resolved
        sqlx::query(
            "UPDATE work_identity_conflicts
             SET status = 'resolved', resolved_at = ?1, resolution_action = ?2,
                 resolution_notes = ?3
             WHERE id = ?4",
        )
        .bind(&now)
        .bind(&action_str)
        .bind(notes)
        .bind(conflict.id)
        .execute(&mut *tx)
        .await?;

        // Recompute the badge now that this conflict row is resolved
        let new_status = derive_badge_in_tx(&mut tx, work_id).await?;
        let status_str = identity_status_str(new_status);
        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(status_str)
            .bind(work_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Dismiss a conflict row and recompute the work's identity badge — atomically.
    ///
    /// Does NOT re-stamp the existing anchor (the user is not asserting the existing
    /// value is correct — they are only deferring the decision). Re-raise is prevented
    /// by the closed-conflict check inside `detect_conflicting_anchors`.
    pub async fn apply_conflict_dismiss(
        &self,
        conflict: &IdentityConflict,
        dismissed_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let work_id = conflict.existing_work_id;
        let now = dismissed_at.to_rfc3339();

        let mut tx = self.pool().begin().await?;

        sqlx::query(
            "UPDATE work_identity_conflicts
             SET status = 'dismissed', resolved_at = ?1
             WHERE id = ?2",
        )
        .bind(&now)
        .bind(conflict.id)
        .execute(&mut *tx)
        .await?;

        // Recompute the badge now that this conflict row is dismissed
        let new_status = derive_badge_in_tx(&mut tx, work_id).await?;
        let status_str = identity_status_str(new_status);
        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(status_str)
            .bind(work_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Return the anchor-table type string for a given conflict kind, or `None`
/// for `QuorumTie` (which is handled separately / out of scope).
fn anchor_type_for_kind(kind: IdentityConflictKind) -> Option<&'static str> {
    match kind {
        IdentityConflictKind::IncomingDifferentOlKey
        | IdentityConflictKind::OlRedirectCollision => Some(AnchorType::OL_WORK),
        IdentityConflictKind::IncomingDifferentGrKey => Some(AnchorType::GR_WORK),
        IdentityConflictKind::IncomingDifferentHcKey => Some(AnchorType::HC_WORK),
        IdentityConflictKind::QuorumTie => None,
    }
}

/// Return the incoming anchor value for the key type implicated by `kind`.
fn incoming_value_for_kind(
    kind: IdentityConflictKind,
    incoming: &IncomingConflictPayload,
) -> Option<&str> {
    match kind {
        IdentityConflictKind::IncomingDifferentOlKey
        | IdentityConflictKind::OlRedirectCollision => incoming.ol_key.as_deref(),
        IdentityConflictKind::IncomingDifferentGrKey => incoming.gr_key.as_deref(),
        IdentityConflictKind::IncomingDifferentHcKey => incoming.hc_key.as_deref(),
        IdentityConflictKind::QuorumTie => None,
    }
}

/// Update the denormalized `works` column corresponding to `anchor_type`.
async fn update_works_column_in_tx(
    tx: &mut SqliteConnection,
    anchor_type: &str,
    value: &str,
    work_id: WorkId,
) -> Result<(), sqlx::Error> {
    let sql: Option<&str> = match anchor_type {
        AnchorType::OL_WORK => Some("UPDATE works SET ol_key = ?1 WHERE id = ?2"),
        AnchorType::GR_WORK => Some("UPDATE works SET gr_key = ?1 WHERE id = ?2"),
        AnchorType::HC_WORK => Some("UPDATE works SET hc_key = ?1 WHERE id = ?2"),
        AnchorType::ISBN_13 => Some("UPDATE works SET isbn_13 = ?1 WHERE id = ?2"),
        AnchorType::ASIN => Some("UPDATE works SET asin = ?1 WHERE id = ?2"),
        _ => None,
    };
    if let Some(s) = sql {
        sqlx::query(s).bind(value).bind(work_id).execute(tx).await?;
    }
    Ok(())
}

/// Derive the correct `IdentityStatus` badge from the work's remaining confirmed
/// anchors and open conflicts (mirrors `derived_identity_status`, D-013).
pub(crate) async fn derive_badge_in_tx(
    tx: &mut SqliteConnection,
    work_id: WorkId,
) -> Result<IdentityStatus, sqlx::Error> {
    // If any other open conflict exists for this work, keep the Conflict badge.
    let open_conflicts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_conflicts
         WHERE existing_work_id = ?1 AND status = 'open'",
    )
    .bind(work_id)
    .fetch_one(&mut *tx)
    .await?;

    if open_conflicts > 0 {
        return Ok(IdentityStatus::Conflict);
    }

    // Derive from confirmed anchors: work key → Confirmed; ISBN/ASIN → Provisional.
    let work_key_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors
         WHERE work_id = ?1 AND confidence = 'confirmed'
           AND anchor_type IN ('ol_work', 'gr_work', 'hc_work')",
    )
    .bind(work_id)
    .fetch_one(&mut *tx)
    .await?;

    if work_key_count > 0 {
        return Ok(IdentityStatus::Confirmed);
    }

    let bridge_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors
         WHERE work_id = ?1 AND confidence = 'confirmed'
           AND anchor_type IN ('isbn_13', 'asin')",
    )
    .bind(work_id)
    .fetch_one(&mut *tx)
    .await?;

    if bridge_count > 0 {
        return Ok(IdentityStatus::Provisional);
    }

    Ok(IdentityStatus::Pending)
}

/// Serialize an `IdentityStatus` to the snake_case string stored in the DB.
pub(crate) fn identity_status_str(s: IdentityStatus) -> &'static str {
    match s {
        IdentityStatus::Pending => "pending",
        IdentityStatus::Confirmed => "confirmed",
        IdentityStatus::Provisional => "provisional",
        IdentityStatus::Conflict => "conflict",
        IdentityStatus::NeedsReview => "needs_review",
        IdentityStatus::NotFound => "not_found",
    }
}

fn parse_conflict_row(row: ConflictRow) -> Result<IdentityConflict, String> {
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
