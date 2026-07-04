use chrono::{DateTime, Utc};
use livrarr_domain::identity::*;
use livrarr_domain::services::WorkIdentityError;
use livrarr_domain::{IdentityStatus, UserId, WorkId};
use sqlx::SqliteConnection;

use crate::sqlite::SqliteDb;

/// Error type returned by the atomic conflict-apply operations.
///
/// `AlreadyResolved` is distinct from `Db` so callers can return a typed
/// "already closed" response (→ 409 Conflict) rather than a generic 500.
/// `InvalidAnchorValue` propagates the primary-anchor validation failure
/// typed so the API layer can return 400 Bad Request.
#[derive(Debug, thiserror::Error)]
pub enum ConflictApplyError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("invalid anchor value")]
    InvalidAnchorValue,
    #[error("conflict already resolved or dismissed")]
    AlreadyResolved,
}

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
    ///
    /// **QuorumTie** is fully in scope: `async_resolver::llm_identity_verify` creates
    /// work-scoped QuorumTie conflicts (`existing_work_id = work.id`), so resolving
    /// them must recompute the badge just like any other conflict.
    /// The add-time QuorumTie (`existing_work_id = 0`, from `english_identity_resolver`)
    /// has no work to act on; routing it through here recomputes a non-existent work
    /// (harmless no-op). That add-time case is deferred to the Phase 2-3 "pick at add"
    /// reshape — documented, not claimed fixed here.
    ///
    /// **TOCTOU guard**: the conflict-row UPDATE is performed FIRST inside the tx with
    /// `AND status = 'open'`. If `rows_affected() == 0` a concurrent resolve/dismiss
    /// already closed this conflict; we return `ConflictApplyError::AlreadyResolved`
    /// and the tx is rolled back before any anchor mutations commit.
    pub async fn apply_conflict_resolution(
        &self,
        conflict: &IdentityConflict,
        action: ConflictResolutionAction,
        notes: Option<&str>,
        resolved_at: DateTime<Utc>,
    ) -> Result<(), ConflictApplyError> {
        let work_id = conflict.existing_work_id;
        let now = resolved_at.to_rfc3339();
        let action_str = serde_json::to_value(action)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "keep_existing".to_string());

        let mut tx = self.pool().begin().await?;

        // ── TOCTOU guard ──────────────────────────────────────────────────────
        // Claim the conflict row NOW, inside the tx, guarded by `status = 'open'`.
        // A concurrent resolve/dismiss will have already flipped the status, so
        // rows_affected will be 0 and we abort before any anchor mutation commits.
        let guard = sqlx::query(
            "UPDATE work_identity_conflicts
             SET status = 'resolved', resolved_at = ?1, resolution_action = ?2,
                 resolution_notes = ?3
             WHERE id = ?4 AND status = 'open'",
        )
        .bind(&now)
        .bind(&action_str)
        .bind(notes)
        .bind(conflict.id)
        .execute(&mut *tx)
        .await?;

        if guard.rows_affected() == 0 {
            return Err(ConflictApplyError::AlreadyResolved);
        }

        // All incoming anchors as (anchor_type_str, value) pairs — iterated by Merge
        // and by ReplaceAnchor/Merge on QuorumTie (which has no single implicated type).
        let anchors_to_merge: &[(&str, Option<&str>)] = &[
            (AnchorType::OL_WORK, conflict.incoming.ol_key.as_deref()),
            (AnchorType::GR_WORK, conflict.incoming.gr_key.as_deref()),
            (AnchorType::HC_WORK, conflict.incoming.hc_key.as_deref()),
            (AnchorType::ISBN_13, conflict.incoming.isbn_13.as_deref()),
            (AnchorType::ASIN, conflict.incoming.asin.as_deref()),
        ];

        match action {
            ConflictResolutionAction::KeepExisting | ConflictResolutionAction::AcceptSeparate => {
                // Re-stamp the kept anchor as User so future machine passes cannot
                // raise the same conflict again (part 1 protection).
                // For QuorumTie, anchor_type_for_kind returns None — no anchor to re-stamp;
                // the badge recompute in the tail is the only side-effect needed.
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
                if let Some(at) = anchor_type_for_kind(conflict.kind) {
                    // Standard anchor replacement: supersede the existing anchor and
                    // confirm the incoming value (User-stamped).
                    // Canonical validation runs inside confirm_anchor_in_tx.
                    // Primary-type validation failure → fail the whole resolution.
                    if let Some(incoming_val) =
                        incoming_value_for_kind(conflict.kind, &conflict.incoming)
                    {
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

                        crate::sqlite_work_identity::confirm_anchor_in_tx(
                            &mut tx,
                            work_id,
                            AnchorType::new(at),
                            incoming_val,
                            AnchorSetter::User,
                        )
                        .await
                        .map_err(|e| match e {
                            WorkIdentityError::InvalidAnchorValue => {
                                ConflictApplyError::InvalidAnchorValue
                            }
                            e => ConflictApplyError::Db(sqlx::Error::Protocol(e.to_string())),
                        })?;
                    }
                } else {
                    // QuorumTie has no single implicated anchor type.
                    // ReplaceAnchor on QuorumTie is treated as Merge: adopt the incoming
                    // candidate's anchors as secondary gap-fills (existing anchors are
                    // preserved; no supersede). Badge recompute runs in the tail below.
                    apply_gap_fills(
                        &mut tx,
                        work_id,
                        anchors_to_merge,
                        None,
                        "quorum-tie replace-anchor",
                    )
                    .await?;
                }
            }

            ConflictResolutionAction::Merge => {
                // The primary type is the anchor implicated by the conflict kind.
                // For QuorumTie, anchor_type_for_kind returns None → all types are
                // secondary (gap-fill only; no supersede of existing confirmed anchors).
                let primary_type: Option<&str> = anchor_type_for_kind(conflict.kind);

                // Primary anchor: supersede any different existing confirmed anchor,
                // then validate + confirm the incoming value.
                // Primary-type canonical validation failure → fail the whole resolution.
                if let Some(pt) = primary_type {
                    let primary_val = anchors_to_merge
                        .iter()
                        .find(|&&(at, _)| at == pt)
                        .and_then(|&(_, v)| v)
                        .filter(|v| !v.is_empty());

                    if let Some(val) = primary_val {
                        sqlx::query(
                            "UPDATE work_identity_anchors
                             SET confidence = 'superseded', superseded_by = ?1
                             WHERE work_id = ?2 AND anchor_type = ?3
                               AND confidence = 'confirmed' AND anchor_value != ?1",
                        )
                        .bind(val)
                        .bind(work_id)
                        .bind(pt)
                        .execute(&mut *tx)
                        .await?;

                        crate::sqlite_work_identity::confirm_anchor_in_tx(
                            &mut tx,
                            work_id,
                            AnchorType::new(pt),
                            val,
                            AnchorSetter::User,
                        )
                        .await
                        .map_err(|e| match e {
                            WorkIdentityError::InvalidAnchorValue => {
                                ConflictApplyError::InvalidAnchorValue
                            }
                            e => ConflictApplyError::Db(sqlx::Error::Protocol(e.to_string())),
                        })?;
                    }
                }

                // Secondary anchors (gap-fill): add incoming anchors ONLY if the work
                // has no confirmed anchor of that type at all — never overwrite an
                // existing User- or machine-set confirmed anchor (Fix 2 / R-001).
                // Secondary-type canonical validation failure → skip + warn; does not
                // block the primary resolution (malformed gap-fill data is non-fatal).
                apply_gap_fills(&mut tx, work_id, anchors_to_merge, primary_type, "merge").await?;
            }
        }

        // Recompute the badge now that this conflict row is resolved.
        // Works for all conflict kinds including QuorumTie.
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
    ///
    /// **TOCTOU guard**: the conflict-row UPDATE is performed FIRST inside the tx with
    /// `AND status = 'open'`. If `rows_affected() == 0` a concurrent dismiss/resolve
    /// already closed this conflict; we return `ConflictApplyError::AlreadyResolved`.
    pub async fn apply_conflict_dismiss(
        &self,
        conflict: &IdentityConflict,
        dismissed_at: DateTime<Utc>,
    ) -> Result<(), ConflictApplyError> {
        let work_id = conflict.existing_work_id;
        let now = dismissed_at.to_rfc3339();

        let mut tx = self.pool().begin().await?;

        // ── TOCTOU guard ──────────────────────────────────────────────────────
        let guard = sqlx::query(
            "UPDATE work_identity_conflicts
             SET status = 'dismissed', resolved_at = ?1
             WHERE id = ?2 AND status = 'open'",
        )
        .bind(&now)
        .bind(conflict.id)
        .execute(&mut *tx)
        .await?;

        if guard.rows_affected() == 0 {
            return Err(ConflictApplyError::AlreadyResolved);
        }

        // Recompute the badge now that this conflict row is dismissed.
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
/// for `QuorumTie` (which has no single implicated anchor type).
///
/// All four resolution actions handle `QuorumTie` via the anchor-agnostic branch
/// in `apply_conflict_resolution`: `KeepExisting`/`AcceptSeparate` make no anchor
/// change (the existing identity stands); `Merge` and `ReplaceAnchor` adopt the
/// incoming candidate's anchors as secondary gap-fills and recompute the badge.
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

/// Attempt to confirm each entry in `anchors` as a gap-fill anchor for `work_id`.
///
/// "Gap-fill" semantics: a type is only written if the work has **no** existing
/// confirmed anchor of that type (never overwrites User- or machine-set data).
///
/// Behaviour per entry:
/// - Skip if `at == exclude` (used by `Merge` to bypass the primary type, which is
///   handled separately with supersede semantics).
/// - Skip if the incoming value is absent or empty.
/// - Skip (silently) if a confirmed anchor of that type already exists.
/// - Skip + warn if `confirm_anchor_in_tx` returns `InvalidAnchorValue` (non-fatal
///   for gap-fills; malformed secondary data must not block the primary resolution).
/// - Return `ConflictApplyError::Db` for any other `confirm_anchor_in_tx` error.
async fn apply_gap_fills(
    tx: &mut SqliteConnection,
    work_id: WorkId,
    anchors: &[(&str, Option<&str>)],
    exclude: Option<&str>,
    warn_context: &str,
) -> Result<(), ConflictApplyError> {
    for &(at, val) in anchors {
        if exclude == Some(at) {
            continue;
        }
        let val = match val.filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => continue,
        };
        let has_confirmed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM work_identity_anchors
             WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed'",
        )
        .bind(work_id)
        .bind(at)
        .fetch_one(&mut *tx)
        .await?;
        if has_confirmed > 0 {
            continue;
        }
        match crate::sqlite_work_identity::confirm_anchor_in_tx(
            tx,
            work_id,
            AnchorType::new(at),
            val,
            AnchorSetter::User,
        )
        .await
        {
            Ok(()) => {}
            Err(WorkIdentityError::InvalidAnchorValue) => {
                tracing::warn!(
                    work_id = %work_id,
                    anchor_type = at,
                    context = warn_context,
                    "incoming anchor has invalid canonical form — skipping gap-fill for this type"
                );
            }
            Err(e) => {
                return Err(ConflictApplyError::Db(sqlx::Error::Protocol(e.to_string())));
            }
        }
    }
    Ok(())
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
