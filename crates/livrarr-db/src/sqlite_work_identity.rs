use chrono::Utc;
use livrarr_domain::identity::*;
use livrarr_domain::identity_edit::IdentityEditError;
use livrarr_domain::identity_layer::{EditionFormat, IdentityProvider, RouteKind, RouteProvenance};
use livrarr_domain::normalization::{normalize_asin, normalize_gr_key, normalize_isbn13, AsinNorm};
use livrarr_domain::services::{
    ClearedSlot, CollisionInfo, IdentityCompletion, IdentityCompletionOutcome, IdentityEditBasis,
    IdentitySlotBasis, WorkIdentityError, WorkIdentityRepository,
};
use livrarr_domain::{IdentityStatus, UserId, Work, WorkId};
use sqlx::SqliteConnection;

use crate::sqlite::SqliteDb;
use crate::sqlite_work::row_to_work;

/// DB-local typed error for the in-transaction identity write helpers. Keeps
/// `sqlx::Error` typed through composite transactions (the edit transaction
/// classifies constraint/storage failures at its boundary); legacy repository
/// wrappers map it to `WorkIdentityError` only after their transaction ends.
#[derive(Debug)]
pub(crate) enum IdentityTxError {
    InvalidValue,
    Sqlx(sqlx::Error),
}

impl From<sqlx::Error> for IdentityTxError {
    fn from(e: sqlx::Error) -> Self {
        IdentityTxError::Sqlx(e)
    }
}

impl IdentityTxError {
    fn into_work_identity(self) -> WorkIdentityError {
        match self {
            IdentityTxError::InvalidValue => WorkIdentityError::InvalidAnchorValue,
            IdentityTxError::Sqlx(e) => WorkIdentityError::Db(e.to_string()),
        }
    }
}

/// True when the slot's raw column value passes the same canonical
/// normalizers the write chokepoint enforces — the "validated column" half of
/// the ledger∪column projection. A quarantined invalid value stays visible
/// and clearable but earns no badge or collision authority.
pub(crate) fn column_value_valid(anchor_type: &str, value: &str) -> bool {
    if value.trim().is_empty() {
        return false;
    }
    match anchor_type {
        AnchorType::ISBN_13 => normalize_isbn13(value).as_deref() == Some(value),
        AnchorType::GR_WORK => normalize_gr_key(value).as_deref() == Some(value),
        AnchorType::ASIN => matches!(normalize_asin(value), AsinNorm::Asin(a) if a == value),
        // ol_work / hc_work follow the current nonempty contract.
        _ => true,
    }
}

/// Advance the work's durable `identity_generation` unconditionally, inside
/// the caller's transaction. Every committed identity mutation advances it —
/// composite transactions may bump more than once, which is valid.
pub(crate) async fn bump_identity_generation(
    conn: &mut SqliteConnection,
    work_id: WorkId,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE works SET identity_generation = identity_generation + 1 WHERE id = ?1")
        .bind(work_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// First-statement conditional generation claim: `Ok(true)` when this
/// transaction won (the row matched `expected_generation` and was advanced),
/// `Ok(false)` when a different identity mutation won since the caller's
/// coherent read. There is no read-then-compare window.
pub(crate) async fn claim_identity_generation(
    conn: &mut SqliteConnection,
    work_id: WorkId,
    expected_generation: i64,
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE works SET identity_generation = identity_generation + 1 \
         WHERE id = ?1 AND identity_generation = ?2",
    )
    .bind(work_id)
    .bind(expected_generation)
    .execute(conn)
    .await?;
    Ok(rows.rows_affected() == 1)
}

/// Core in-transaction anchor write: canonical validation + generation bump +
/// anchor upsert + denormalized column sync.
///
/// This is the single point that enforces the identity write contract (REQ-029). Every caller
/// that wants to persist a confirmed anchor — `confirm_anchor`, `confirm_anchor_and_recompute_badge`,
/// and conflict-resolution writes in `sqlite_identity_conflict` — must go through this helper so
/// the validation contract can never be bypassed regardless of call path. It advances
/// `identity_generation` before the upsert, so any preview or delayed completion holding an
/// older generation goes durably stale.
///
/// Returns `IdentityTxError::InvalidValue` when either the value is empty or the value
/// is not in canonical form for the anchor type.
pub(crate) async fn confirm_anchor_in_tx(
    conn: &mut SqliteConnection,
    work_id: WorkId,
    anchor_type: AnchorType,
    value: &str,
    setter: AnchorSetter,
) -> Result<(), IdentityTxError> {
    if value.trim().is_empty() {
        return Err(IdentityTxError::InvalidValue);
    }
    // Defense in depth (REQ-029): a typed identifier must already be in its canonical form
    // before it is persisted. Callers normalize via WorkSeed::sanitized, but validating here
    // means a malformed value can never reach a row regardless of the call path.
    let canonical = match anchor_type.as_str() {
        AnchorType::ISBN_13 => normalize_isbn13(value).as_deref() == Some(value),
        AnchorType::GR_WORK => normalize_gr_key(value).as_deref() == Some(value),
        AnchorType::ASIN => matches!(normalize_asin(value), AsinNorm::Asin(a) if a == value),
        // ol_work / hc_work have no canonical form beyond the non-empty check.
        _ => true,
    };
    if !canonical {
        return Err(IdentityTxError::InvalidValue);
    }

    bump_identity_generation(conn, work_id).await?;

    let now = Utc::now().to_rfc3339();
    let setter_str = serde_json::to_value(setter)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "user".to_string());
    let anchor_type_str = anchor_type.as_str().to_string();

    sqlx::query(
        "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
         VALUES (?1, ?2, ?3, 'confirmed', ?4, ?5, (SELECT user_id FROM works WHERE id = ?1))
         ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
             confidence = 'confirmed',
             setter = ?4,
             set_at = ?5,
             superseded_by = NULL,
             user_id = (SELECT user_id FROM works WHERE id = ?1)",
    )
    .bind(work_id)
    .bind(&anchor_type_str)
    .bind(value)
    .bind(&setter_str)
    .bind(&now)
    .execute(&mut *conn)
    .await?;

    // Sync the legacy denormalized work column (REQ-022 convergence reads these columns;
    // an OL/GR-only sync leaves hc_key/isbn_13/asin stale).
    let update_sql: Option<&str> = match anchor_type.as_str() {
        AnchorType::OL_WORK => Some("UPDATE works SET ol_key = ?1 WHERE id = ?2"),
        AnchorType::GR_WORK => Some("UPDATE works SET gr_key = ?1 WHERE id = ?2"),
        AnchorType::HC_WORK => Some("UPDATE works SET hc_key = ?1 WHERE id = ?2"),
        AnchorType::ISBN_13 => Some("UPDATE works SET isbn_13 = ?1 WHERE id = ?2"),
        AnchorType::ASIN => Some("UPDATE works SET asin = ?1 WHERE id = ?2"),
        _ => None,
    };
    if let Some(sql) = update_sql {
        sqlx::query(sql)
            .bind(value)
            .bind(work_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

impl WorkIdentityRepository for SqliteDb {
    async fn confirm_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        confirm_anchor_in_tx(&mut tx, work_id, anchor_type, value, setter)
            .await
            .map_err(IdentityTxError::into_work_identity)?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn merge_missing_anchors(
        &self,
        work_id: WorkId,
        incoming: &CapturedIdentity,
    ) -> Result<Vec<AnchorType>, WorkIdentityError> {
        let existing = self.list_anchors(work_id).await?;
        let confirmed_types: std::collections::HashSet<String> = existing
            .iter()
            .filter(|a| a.confidence == AnchorConfidence::Confirmed)
            .map(|a| a.anchor_type.as_str().to_string())
            .collect();

        let anchors: &[(&str, Option<&str>)] = &[
            (AnchorType::OL_WORK, incoming.ol_key.as_deref()),
            (AnchorType::GR_WORK, incoming.gr_key.as_deref()),
            (AnchorType::HC_WORK, incoming.hc_key.as_deref()),
            (AnchorType::ISBN_13, incoming.isbn_13.as_deref()),
            (AnchorType::ASIN, incoming.asin.as_deref()),
        ];

        let mut merged = Vec::new();
        for &(anchor_type_str, maybe_value) in anchors {
            if let Some(value) = maybe_value {
                if !confirmed_types.contains(anchor_type_str) {
                    let anchor_type = AnchorType::new(anchor_type_str);
                    match self
                        .confirm_anchor(work_id, anchor_type.clone(), value, AnchorSetter::Import)
                        .await
                    {
                        Ok(()) => {
                            merged.push(anchor_type);
                        }
                        Err(WorkIdentityError::InvalidAnchorValue) => {
                            tracing::warn!(
                                work_id,
                                anchor_type = anchor_type_str,
                                value,
                                "skipping invalid anchor value"
                            );
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }

        Ok(merged)
    }

    async fn detect_conflicting_anchors(
        &self,
        existing_work_id: WorkId,
        incoming: &CapturedIdentity,
        source: ConflictSource,
    ) -> Result<Vec<NewIdentityConflict>, WorkIdentityError> {
        let existing = self.list_anchors(existing_work_id).await?;
        let user_id: i64 = sqlx::query_scalar("SELECT user_id FROM works WHERE id = ?")
            .bind(existing_work_id)
            .fetch_one(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // Track value AND setter: a User-set anchor must never generate a conflict
        // (the user already chose this value; a machine result cannot override it).
        let confirmed: std::collections::HashMap<String, (String, AnchorSetter)> = existing
            .iter()
            .filter(|a| a.confidence == AnchorConfidence::Confirmed)
            .map(|a| {
                (
                    a.anchor_type.as_str().to_string(),
                    (a.anchor_value.clone(), a.setter),
                )
            })
            .collect();

        let mut conflicts = Vec::new();
        // (anchor_type_str, incoming_value, conflict_kind, json_path_in_payload)
        let checks: &[(&str, Option<&str>, IdentityConflictKind, &str)] = &[
            (
                AnchorType::OL_WORK,
                incoming.ol_key.as_deref(),
                IdentityConflictKind::IncomingDifferentOlKey,
                "$.ol_key",
            ),
            (
                AnchorType::GR_WORK,
                incoming.gr_key.as_deref(),
                IdentityConflictKind::IncomingDifferentGrKey,
                "$.gr_key",
            ),
            (
                AnchorType::HC_WORK,
                incoming.hc_key.as_deref(),
                IdentityConflictKind::IncomingDifferentHcKey,
                "$.hc_key",
            ),
        ];

        for &(anchor_type_str, incoming_value, kind, json_field) in checks {
            if let Some(incoming_val) = incoming_value {
                if let Some((existing_val, existing_setter)) = confirmed.get(anchor_type_str) {
                    if incoming_val != existing_val.as_str() {
                        // A User-set anchor is the top of the confidence hierarchy —
                        // the user already made the identity call for this type.
                        // Drop the differing machine value rather than raising a conflict.
                        //
                        // TODO(phase2-3): This suppression is intentionally blanket for now.
                        // Upstream provider redirects/merges (e.g. OpenLibrary merging two work
                        // entries and issuing a redirect) cannot be detected until the Phase 2-3
                        // look-up/redirect machinery exists. A `Refresh` or `Convergence` source
                        // that carries a different value for a User-set anchor is currently
                        // silently dropped here even if the provider is telling us the old ID is
                        // defunct. Real redirect handling will land with the Phase 2-3 provider
                        // re-fetch work; at that point this branch should become source-aware.
                        if existing_setter == &AnchorSetter::User {
                            continue;
                        }

                        // If this exact contradiction (work × kind × incoming value) was
                        // already dismissed or resolved, do not re-raise it.
                        let kind_str = serde_json::to_value(kind)
                            .ok()
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default();
                        let closed_count: i64 = sqlx::query_scalar(
                            "SELECT COUNT(*) FROM work_identity_conflicts
                             WHERE existing_work_id = ?1 AND kind = ?2
                               AND status IN ('dismissed', 'resolved')
                               AND json_extract(incoming_payload_json, ?3) = ?4",
                        )
                        .bind(existing_work_id)
                        .bind(&kind_str)
                        .bind(json_field)
                        .bind(incoming_val)
                        .fetch_one(self.pool())
                        .await
                        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
                        if closed_count > 0 {
                            continue;
                        }

                        conflicts.push(NewIdentityConflict {
                            user_id,
                            existing_work_id,
                            kind,
                            incoming: IncomingConflictPayload {
                                ol_key: incoming.ol_key.clone(),
                                gr_key: incoming.gr_key.clone(),
                                hc_key: incoming.hc_key.clone(),
                                isbn_13: incoming.isbn_13.clone(),
                                asin: incoming.asin.clone(),
                                title: incoming.title.clone(),
                                author_name: incoming.author_name.clone(),
                                year: None,
                                cover_url: None,
                                top_candidates: Vec::new(),
                            },
                            raised_by: source,
                            raised_source_path: None,
                        });
                    }
                }
            }
        }

        Ok(conflicts)
    }

    async fn raise_identity_conflict(
        &self,
        conflict: NewIdentityConflict,
    ) -> Result<i64, WorkIdentityError> {
        // All operations — generation bump, dedup-SELECT, conflict INSERT,
        // badge UPDATE — run inside a single transaction so no partial state
        // is ever visible.
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let conflict_id = raise_identity_conflict_in_tx(&mut tx, &conflict)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        Ok(conflict_id)
    }

    async fn set_identity_pending(
        &self,
        work_id: WorkId,
        _reason: PendingReason,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        let now = Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(setter)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "auto_search".to_string());

        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // Create-time initialization claims the generation before its
        // pending-row + OL-column mutation (design §Claims).
        bump_identity_generation(&mut tx, work_id)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // Empty string sentinel for pending anchor_value per IR v2 decision.
        // Reason is logged but not persisted in the anchor table.
        sqlx::query(
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
             VALUES (?1, 'ol_work', '', 'pending', ?2, ?3, (SELECT user_id FROM works WHERE id = ?1))
             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                 confidence = 'pending',
                 setter = ?2,
                 set_at = ?3"
        )
        .bind(work_id)
        .bind(&setter_str)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query("UPDATE works SET ol_key = NULL, identity_status = 'pending' WHERE id = ?1")
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn set_needs_review(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        // Raw status arm: the mutation advances identity_generation in the
        // same SQL statement (design §Claims — no loopholes).
        sqlx::query(
            "UPDATE works SET identity_status = 'needs_review', \
             identity_generation = identity_generation + 1 WHERE id = ?1",
        )
        .bind(work_id)
        .execute(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn record_review_candidates(
        &self,
        work_id: WorkId,
        candidates: &[Candidate],
    ) -> Result<(), WorkIdentityError> {
        let json =
            serde_json::to_string(candidates).map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO work_identity_review_candidates (work_id, user_id, candidates_json, recorded_at)
             VALUES (?1, (SELECT user_id FROM works WHERE id = ?1), ?2, ?3)
             ON CONFLICT (work_id) DO UPDATE SET
                 candidates_json = ?2,
                 recorded_at = ?3",
        )
        .bind(work_id)
        .bind(&json)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn get_review_candidates(
        &self,
        work_id: WorkId,
    ) -> Result<Option<Vec<Candidate>>, WorkIdentityError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT candidates_json FROM work_identity_review_candidates WHERE work_id = ?1",
        )
        .bind(work_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        match row {
            Some((json,)) => {
                let candidates: Vec<Candidate> = serde_json::from_str(&json)
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
                Ok(Some(candidates))
            }
            None => Ok(None),
        }
    }

    async fn list_needs_review_works(
        &self,
        user_id: UserId,
    ) -> Result<Vec<Work>, WorkIdentityError> {
        let rows = sqlx::query(
            "SELECT * FROM works WHERE user_id = ?1 AND identity_status = 'needs_review' \
             ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            match row_to_work(row) {
                Ok(w) => results.push(w),
                Err(e) => {
                    tracing::warn!("needs-review works: skipping corrupt row: {e}");
                }
            }
        }
        Ok(results)
    }

    async fn apply_review_candidate(
        &self,
        work_id: WorkId,
        candidate: &Candidate,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // ── TOCTOU claim (mirrors apply_conflict_resolution's status='open'
        // guard) ─────────────────────────────────────────────────────────────
        // First statement of the transaction: atomically verify the work is
        // parked NeedsReview and claim it. Being a WRITE, this acquires the
        // write lock immediately, so a concurrent resolve/dismiss either
        // committed first (rows_affected = 0 → clean abort, no writes) or
        // queues behind this transaction — the handler's read-candidates-then-
        // apply window cannot double-apply. A stale candidates row on a
        // settled work is inert: the guard is on the badge, not the row.
        // The interim 'pending' is invisible outside the transaction and is
        // overwritten by the derived badge below before commit.
        let guard = sqlx::query(
            "UPDATE works SET identity_status = 'pending', \
             identity_generation = identity_generation + 1 \
             WHERE id = ?1 AND identity_status = 'needs_review'",
        )
        .bind(work_id)
        .execute(&mut *tx as &mut SqliteConnection)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        if guard.rows_affected() == 0 {
            return Err(WorkIdentityError::NotParked);
        }

        // Confirm every anchor the chosen candidate carries. Fails closed (the
        // transaction never commits) if any value fails canonical validation —
        // a partially-applied pick would be worse than an unapplied one.
        let anchors: &[(&str, Option<&str>)] = &[
            (AnchorType::OL_WORK, candidate.anchors.ol_key.as_deref()),
            (AnchorType::GR_WORK, candidate.anchors.gr_key.as_deref()),
            (AnchorType::HC_WORK, candidate.anchors.hc_key.as_deref()),
            (AnchorType::ISBN_13, candidate.anchors.isbn_13.as_deref()),
            (AnchorType::ASIN, candidate.anchors.asin.as_deref()),
        ];
        for &(anchor_type_str, maybe_value) in anchors {
            if let Some(value) = maybe_value {
                confirm_anchor_in_tx(
                    &mut tx,
                    work_id,
                    AnchorType::new(anchor_type_str),
                    value,
                    setter,
                )
                .await
                .map_err(IdentityTxError::into_work_identity)?;
            }
        }

        // Atomically derive and write the badge from the anchors just written —
        // same derivation `confirm_anchor_and_recompute_badge` and conflict
        // resolution use, so a picked candidate un-parks to whatever its anchors
        // actually earn (Confirmed/Provisional/Pending), never a new status.
        let badge = crate::sqlite_identity_conflict::derive_badge_in_tx(
            &mut *tx as &mut SqliteConnection,
            work_id,
        )
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
            .bind(work_id)
            .execute(&mut *tx as &mut SqliteConnection)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // The park is resolved — clear the recorded candidate set (REQ-010).
        sqlx::query("DELETE FROM work_identity_review_candidates WHERE work_id = ?1")
            .bind(work_id)
            .execute(&mut *tx as &mut SqliteConnection)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn dismiss_review(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // ── TOCTOU claim (same shape as apply_review_candidate) ─────────────
        // No anchor writes, no merge — the work simply stops needing review and
        // stands alone as Pending (AC-013 dismiss semantics). Conditional on the
        // work actually being parked: a settled work (Confirmed/Provisional/
        // Conflict) must never be downgraded to Pending by a direct dismiss
        // POST. rows_affected = 0 → not parked → abort with zero writes; the
        // candidates-row clear below only ever runs under a won claim.
        let guard = sqlx::query(
            "UPDATE works SET identity_status = 'pending', \
             identity_generation = identity_generation + 1 \
             WHERE id = ?1 AND identity_status = 'needs_review'",
        )
        .bind(work_id)
        .execute(&mut *tx as &mut SqliteConnection)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        if guard.rows_affected() == 0 {
            return Err(WorkIdentityError::NotParked);
        }

        sqlx::query("DELETE FROM work_identity_review_candidates WHERE work_id = ?1")
            .bind(work_id)
            .execute(&mut *tx as &mut SqliteConnection)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn set_identity_confirmed(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        // Raw status arm: same-statement generation advance (design §Claims).
        sqlx::query(
            "UPDATE works SET identity_status = 'confirmed', \
             identity_generation = identity_generation + 1 WHERE id = ?1",
        )
        .bind(work_id)
        .execute(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn set_identity_provisional(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        // Raw status arm: same-statement generation advance (design §Claims).
        sqlx::query(
            "UPDATE works SET identity_status = 'provisional', \
             identity_generation = identity_generation + 1 WHERE id = ?1",
        )
        .bind(work_id)
        .execute(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn verify_anchor_cache_consistency(
        &self,
    ) -> Result<Vec<ConsistencyDivergence>, WorkIdentityError> {
        let rows: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT w.id, w.ol_key, a.anchor_value
             FROM works w
             LEFT JOIN work_identity_anchors a
                 ON a.work_id = w.id AND a.anchor_type = 'ol_work' AND a.confidence = 'confirmed'
             WHERE w.ol_key IS NOT NULL OR a.anchor_value IS NOT NULL",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let mut divergences = Vec::new();
        for (work_id, cache, anchor) in rows {
            match (&cache, &anchor) {
                (Some(c), Some(a)) if c == a => {}
                (Some(_), None) => {
                    divergences.push(ConsistencyDivergence::CacheAhead {
                        work_id,
                        cache,
                        anchor,
                    });
                }
                (None, Some(a)) => {
                    divergences.push(ConsistencyDivergence::AnchorAhead {
                        work_id,
                        anchor: a.clone(),
                    });
                }
                (Some(_), Some(_)) => {
                    divergences.push(ConsistencyDivergence::CacheAhead {
                        work_id,
                        cache,
                        anchor,
                    });
                }
                (None, None) => {}
            }
        }
        Ok(divergences)
    }

    async fn find_work_by_anchor(
        &self,
        user_id: livrarr_domain::UserId,
        anchor_type: &AnchorType,
        anchor_value: &str,
    ) -> Result<Option<WorkId>, WorkIdentityError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT a.work_id FROM work_identity_anchors a
             JOIN works w ON w.id = a.work_id AND w.user_id = ?1
             WHERE a.anchor_type = ?2 AND a.anchor_value = ?3 AND a.confidence = 'confirmed'
             LIMIT 1",
        )
        .bind(user_id)
        .bind(anchor_type.as_str())
        .bind(anchor_value)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        Ok(row.map(|(id,)| id))
    }

    async fn list_anchors(
        &self,
        work_id: WorkId,
    ) -> Result<Vec<WorkIdentityAnchor>, WorkIdentityError> {
        let rows: Vec<(String, String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT anchor_type, anchor_value, confidence, setter, set_at, superseded_by
             FROM work_identity_anchors WHERE work_id = ?1
             ORDER BY set_at DESC",
        )
        .bind(work_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let mut anchors = Vec::new();
        for (atype, aval, conf, setter, set_at, superseded) in rows {
            let confidence = match conf.as_str() {
                "confirmed" => AnchorConfidence::Confirmed,
                "pending" => AnchorConfidence::Pending,
                "superseded" => AnchorConfidence::Superseded,
                _ => continue,
            };
            let setter = serde_json::from_value(serde_json::Value::String(setter.clone()))
                // Least-privilege default: an unreadable setter must never be treated
                // as user-authoritative — that would suppress legitimate conflict raises.
                .unwrap_or(AnchorSetter::AutoSearch);
            let set_at = chrono::DateTime::parse_from_rfc3339(&set_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            anchors.push(WorkIdentityAnchor {
                work_id,
                anchor_type: AnchorType::new(atype),
                anchor_value: aval,
                confidence,
                setter,
                set_at,
                superseded_by: superseded,
            });
        }
        Ok(anchors)
    }

    async fn backfill_gr_numeric(&self) -> Result<(), WorkIdentityError> {
        let rows: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT id, user_id, gr_key FROM works WHERE gr_key IS NOT NULL AND gr_key != ''",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        for (work_id, user_id, raw_key) in &rows {
            let Some(normalized) = normalize_gr_key(raw_key) else {
                continue;
            };

            if normalized != *raw_key {
                sqlx::query("UPDATE works SET gr_key = ? WHERE id = ?")
                    .bind(&normalized)
                    .bind(work_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }

            // Idempotent anchor backfill: insert the bare-numeric GR anchor only
            // when the work has no confirmed gr_work anchor yet. The
            // INSERT ... SELECT ... WHERE NOT EXISTS guard respects the
            // uniq_primary_confirmed_anchor partial index (one confirmed anchor
            // per type) — a plain VALUES insert of a value differing from an
            // already-confirmed gr_work anchor would violate that index and abort
            // the whole startup backfill. set_at uses RFC3339 to match the format
            // confirm_anchor writes (list_anchors parses it as RFC3339).
            let now = Utc::now().to_rfc3339();
            sqlx::query(
                "INSERT INTO work_identity_anchors \
                 (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
                 SELECT ?1, 'gr_work', ?2, 'confirmed', 'import', ?4, ?3 \
                 WHERE NOT EXISTS ( \
                     SELECT 1 FROM work_identity_anchors \
                     WHERE work_id = ?1 AND anchor_type = 'gr_work' AND confidence = 'confirmed' \
                 )",
            )
            .bind(work_id)
            .bind(&normalized)
            .bind(user_id)
            .bind(&now)
            .execute(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        }

        Ok(())
    }

    async fn record_pending_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
    ) -> Result<(), WorkIdentityError> {
        if value.trim().is_empty() {
            return Err(WorkIdentityError::InvalidAnchorValue);
        }
        // A fuzzy-guessed anchor lives only in the ledger as a pending guess: it
        // never syncs the denormalized works.* column enrichment reads, so a wrong
        // guess can be neither fetched nor displayed until a user affirms it. The
        // ON CONFLICT guard refuses to downgrade an already-confirmed anchor.
        let now = Utc::now().to_rfc3339();
        let anchor_type_str = anchor_type.as_str().to_string();

        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // A landed pending guess is an identity mutation a live preview must
        // observe: the guess becomes affirmable, so a snapshot taken before it
        // cannot claim to describe the slot set it certified against.
        bump_identity_generation(&mut tx, work_id)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query(
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
             VALUES (?1, ?2, ?3, 'pending', 'auto_search', ?4, (SELECT user_id FROM works WHERE id = ?1))
             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                 confidence = 'pending',
                 setter = 'auto_search',
                 set_at = ?4
             WHERE work_identity_anchors.confidence != 'confirmed'",
        )
        .bind(work_id)
        .bind(&anchor_type_str)
        .bind(value)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        Ok(())
    }

    async fn bump_anchor_attempt(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
    ) -> Result<(), WorkIdentityError> {
        let now = Utc::now().to_rfc3339();
        let anchor_type_str = anchor_type.as_str().to_string();
        sqlx::query(
            "INSERT INTO work_anchor_dead_ends (work_id, anchor_type, attempt_count, last_attempt_at, user_id)
             VALUES (?1, ?2, 1, ?3, (SELECT user_id FROM works WHERE id = ?1))
             ON CONFLICT (work_id, anchor_type) DO UPDATE SET
                 attempt_count = attempt_count + 1,
                 last_attempt_at = ?3",
        )
        .bind(work_id)
        .bind(&anchor_type_str)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn list_anchor_dead_ends(
        &self,
        work_id: WorkId,
    ) -> Result<Vec<AnchorDeadEnd>, WorkIdentityError> {
        let rows: Vec<(String, i64, String)> = sqlx::query_as(
            "SELECT anchor_type, attempt_count, last_attempt_at
             FROM work_anchor_dead_ends WHERE work_id = ?1",
        )
        .bind(work_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(anchor_type, attempt_count, last_attempt_at)| AnchorDeadEnd {
                    work_id,
                    anchor_type: AnchorType::new(anchor_type),
                    attempt_count: attempt_count as u32,
                    last_attempt_at: chrono::DateTime::parse_from_rfc3339(&last_attempt_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                },
            )
            .collect())
    }

    async fn clear_anchor_dead_end(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
    ) -> Result<(), WorkIdentityError> {
        sqlx::query("DELETE FROM work_anchor_dead_ends WHERE work_id = ?1 AND anchor_type = ?2")
            .bind(work_id)
            .bind(anchor_type.as_str())
            .execute(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn clear_anchor_dead_ends(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        sqlx::query("DELETE FROM work_anchor_dead_ends WHERE work_id = ?1")
            .bind(work_id)
            .execute(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn confirm_anchor_and_recompute_badge(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // Validate + upsert + column sync through the single in-tx helper.
        confirm_anchor_in_tx(&mut tx, work_id, anchor_type, value, setter)
            .await
            .map_err(IdentityTxError::into_work_identity)?;

        // Atomically derive the new badge and write it.
        let badge = crate::sqlite_identity_conflict::derive_badge_in_tx(
            &mut *tx as &mut SqliteConnection,
            work_id,
        )
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
            .bind(work_id)
            .execute(&mut *tx as &mut SqliteConnection)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn apply_identity_edit(
        &self,
        work_id: WorkId,
        user_id: UserId,
        slot: AnchorType,
        new_value: &str,
        expected_generation: i64,
        drop_slots: &[AnchorType],
    ) -> Result<(), IdentityEditError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(classify_edit_sqlx)?;

        let result = apply_identity_edit_in_tx(
            &mut tx,
            work_id,
            user_id,
            &slot,
            new_value,
            expected_generation,
            drop_slots,
        )
        .await;

        match result {
            Ok(()) => {
                tx.commit().await.map_err(classify_edit_sqlx)?;
                Ok(())
            }
            Err(EditTxFailure::Edit(e)) => {
                // Typed edit outcome (stale/collision/invalid) — the drop of
                // `tx` rolls the whole transaction back.
                drop(tx);
                Err(e)
            }
            Err(EditTxFailure::Sqlx(e)) => {
                drop(tx);
                // The per-user work-key unique index is the race backstop: a
                // violation means a competing same-user owner committed after
                // the in-tx recheck — reclassify to the same-user Collision,
                // never a 500. A violation with no other owner is an internal
                // invariant error, not a fabricated collision.
                let unique = e
                    .as_database_error()
                    .is_some_and(|d| d.is_unique_violation());
                if unique {
                    if let Ok(Some(owner)) =
                        find_anchor_owner_on(self.pool(), user_id, &slot, new_value, work_id).await
                    {
                        return Err(IdentityEditError::Collision {
                            owning_work_id: owner.owning_work_id,
                            owning_work_title: owner.owning_work_title,
                        });
                    }
                    return Err(IdentityEditError::Db(format!(
                        "unique violation with no competing owner (invariant): {e}"
                    )));
                }
                Err(classify_edit_sqlx(e))
            }
        }
    }

    async fn apply_identity_clear(
        &self,
        work_id: WorkId,
        user_id: UserId,
        slot: AnchorType,
    ) -> Result<ClearedSlot, IdentityEditError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(classify_edit_sqlx)?;

        // First statement: user-scoped generation bump — claims the
        // then-current slot against delayed completions. Zero rows means a
        // foreign or absent work.
        let claimed = sqlx::query(
            "UPDATE works SET identity_generation = identity_generation + 1 \
             WHERE id = ?1 AND user_id = ?2",
        )
        .bind(work_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(classify_edit_sqlx)?;
        if claimed.rows_affected() == 0 {
            return Err(IdentityEditError::NotFound);
        }

        let v2_active = identity_v2_active(&mut tx)
            .await
            .map_err(classify_edit_sqlx)?;
        let (route_graph_before, active_route_values) = if v2_active {
            let graph = crate::identity_layer::read_active_route_graph(&mut tx, user_id, work_id)
                .await
                .map_err(classify_edit_sqlx)?;
            let values = active_identity_slot_routes(&mut tx, user_id, work_id, &slot)
                .await
                .map_err(classify_edit_sqlx)?
                .into_iter()
                .map(|(_, _, value)| value)
                .collect::<Vec<_>>();
            (Some(graph), values)
        } else {
            (None, Vec::new())
        };

        let slot_str = slot.as_str().to_string();
        let confirmed: Option<String> = sqlx::query_scalar(
            "SELECT anchor_value FROM work_identity_anchors \
             WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed'",
        )
        .bind(work_id)
        .bind(&slot_str)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_edit_sqlx)?;
        let column: Option<String> = sqlx::query_scalar(&format!(
            "SELECT {} FROM works WHERE id = ?1",
            column_for(&slot)
        ))
        .bind(work_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(classify_edit_sqlx)?;
        let column = column.filter(|v| !v.trim().is_empty());
        let pending: Vec<String> = sqlx::query_scalar(
            "SELECT anchor_value FROM work_identity_anchors \
             WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'pending'",
        )
        .bind(work_id)
        .bind(&slot_str)
        .fetch_all(&mut *tx)
        .await
        .map_err(classify_edit_sqlx)?;

        // Empty means no confirmed row, no nonempty column, and no pending
        // row; historical superseded rows do not make a slot nonempty. The
        // rollback (drop) also undoes the claim bump.
        // Presence, not value: the `clear_identity_slot` contract defines an empty slot
        // as no confirmed row, no nonempty column, AND no pending row. A pending row
        // whose value is empty is still a row, and it is exactly the one a user has no
        // other way to get rid of — filtering it out here strands it forever.
        if confirmed.is_none()
            && column.is_none()
            && pending.is_empty()
            && active_route_values.is_empty()
        {
            drop(tx);
            return Err(IdentityEditError::EmptySlot);
        }
        let old_value = confirmed
            .clone()
            .or_else(|| column.clone())
            .or_else(|| pending.iter().find(|v| !v.is_empty()).cloned())
            .or_else(|| active_route_values.first().cloned())
            .unwrap_or_default();

        sqlx::query(
            "UPDATE work_identity_anchors \
             SET confidence = 'superseded', superseded_by = NULL \
             WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed'",
        )
        .bind(work_id)
        .bind(&slot_str)
        .execute(&mut *tx)
        .await
        .map_err(classify_edit_sqlx)?;
        sqlx::query(&format!(
            "UPDATE works SET {} = NULL WHERE id = ?1",
            column_for(&slot)
        ))
        .bind(work_id)
        .execute(&mut *tx)
        .await
        .map_err(classify_edit_sqlx)?;
        delete_slot_residue(&mut tx, work_id, &slot_str)
            .await
            .map_err(classify_edit_sqlx)?;
        sqlx::query("UPDATE works SET merge_generation = merge_generation + 1 WHERE id = ?1")
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(classify_edit_sqlx)?;

        let badge = crate::sqlite_identity_conflict::derive_badge_in_tx(&mut tx, work_id)
            .await
            .map_err(classify_edit_sqlx)?;
        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(classify_edit_sqlx)?;

        if let Some(before) = route_graph_before.as_ref() {
            retire_all_identity_slot_routes(&mut tx, user_id, work_id, &slot)
                .await
                .map_err(classify_edit_sqlx)?;
            recompute_v2_identity_status(&mut tx, user_id, work_id)
                .await
                .map_err(classify_edit_sqlx)?;
            crate::identity_layer::invalidate_retry_state_if_route_graph_changed(
                &mut tx, user_id, work_id, before,
            )
            .await
            .map_err(classify_edit_sqlx)?;
        }

        tx.commit().await.map_err(classify_edit_sqlx)?;
        Ok(ClearedSlot {
            old_value,
            parked_by_conflicts: badge == IdentityStatus::Conflict,
        })
    }

    async fn read_identity_edit_basis(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<IdentityEditBasis, WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        type BasisRow = (
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        );
        let row: Option<BasisRow> = sqlx::query_as(
            "SELECT identity_generation, ol_key, gr_key, hc_key, isbn_13, asin, \
                 identity_status FROM works WHERE id = ?1 AND user_id = ?2",
        )
        .bind(work_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let Some((generation, ol, gr, hc, isbn, asin, stored_str)) = row else {
            return Err(WorkIdentityError::Db("work not found".into()));
        };

        let anchor_rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT anchor_type, anchor_value, confidence, setter \
             FROM work_identity_anchors WHERE work_id = ?1",
        )
        .bind(work_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let dead_end_rows: Vec<(String,)> =
            sqlx::query_as("SELECT anchor_type FROM work_anchor_dead_ends WHERE work_id = ?1")
                .bind(work_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let conflict_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT kind FROM work_identity_conflicts \
             WHERE existing_work_id = ?1 AND status = 'open'",
        )
        .bind(work_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let slot_basis = |anchor_type: &str, column: &Option<String>| -> IdentitySlotBasis {
            let column = column.clone().filter(|v| !v.trim().is_empty());
            let confirmed = anchor_rows
                .iter()
                .find(|(t, _, c, _)| t == anchor_type && c == "confirmed")
                .map(|(_, v, _, s)| {
                    let setter = serde_json::from_value(serde_json::Value::String(s.clone()))
                        .unwrap_or(AnchorSetter::AutoSearch);
                    (v.clone(), setter)
                });
            IdentitySlotBasis {
                column_valid: column
                    .as_deref()
                    .is_some_and(|v| column_value_valid(anchor_type, v)),
                column,
                confirmed,
                pending: anchor_rows
                    .iter()
                    .filter(|(t, v, c, _)| t == anchor_type && c == "pending" && !v.is_empty())
                    .map(|(_, v, _, _)| v.clone())
                    .collect(),
                dead_end: dead_end_rows.iter().any(|(t,)| t == anchor_type),
            }
        };

        let basis_slots = IdentityEditBasis {
            generation,
            ol_work: slot_basis(AnchorType::OL_WORK, &ol),
            gr_work: slot_basis(AnchorType::GR_WORK, &gr),
            hc_work: slot_basis(AnchorType::HC_WORK, &hc),
            isbn_13: slot_basis(AnchorType::ISBN_13, &isbn),
            asin: slot_basis(AnchorType::ASIN, &asin),
            open_conflict_kinds: conflict_rows
                .into_iter()
                .filter_map(|(k,)| serde_json::from_value(serde_json::Value::String(k)).ok())
                .collect(),
            stored_badge: parse_identity_status(&stored_str),
            derived_badge: IdentityStatus::Pending,
        };
        let mut basis = basis_slots;
        basis.derived_badge = derive_union_badge(&basis);
        Ok(basis)
    }

    async fn find_anchor_owner(
        &self,
        user_id: UserId,
        anchor_type: &AnchorType,
        value: &str,
        exclude_work_id: WorkId,
    ) -> Result<Option<CollisionInfo>, WorkIdentityError> {
        find_anchor_owner_on(self.pool(), user_id, anchor_type, value, exclude_work_id)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))
    }

    async fn complete_anchors(
        &self,
        work_id: WorkId,
        expected_generation: i64,
        completion: IdentityCompletion,
    ) -> Result<IdentityCompletionOutcome, WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // First statement: conditional generation claim. A zero-row claim
        // means a newer identity mutation (edit/clear/affirm/…) won — the
        // stale resolution writes nothing.
        if !claim_identity_generation(&mut tx, work_id, expected_generation)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?
        {
            return Ok(IdentityCompletionOutcome::Superseded);
        }

        let mut anchors_merged = Vec::new();
        if let Some(incoming) = &completion.merge_anchors {
            let confirmed_types: std::collections::HashSet<String> =
                sqlx::query_scalar::<_, String>(
                    "SELECT anchor_type FROM work_identity_anchors \
                 WHERE work_id = ?1 AND confidence = 'confirmed'",
                )
                .bind(work_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?
                .into_iter()
                .collect();
            let anchors: &[(&str, Option<&str>)] = &[
                (AnchorType::OL_WORK, incoming.ol_key.as_deref()),
                (AnchorType::GR_WORK, incoming.gr_key.as_deref()),
                (AnchorType::HC_WORK, incoming.hc_key.as_deref()),
                (AnchorType::ISBN_13, incoming.isbn_13.as_deref()),
                (AnchorType::ASIN, incoming.asin.as_deref()),
            ];
            for &(anchor_type_str, maybe_value) in anchors {
                let Some(value) = maybe_value else { continue };
                if confirmed_types.contains(anchor_type_str) {
                    continue;
                }
                let anchor_type = AnchorType::new(anchor_type_str);
                match confirm_anchor_in_tx(
                    &mut tx,
                    work_id,
                    anchor_type.clone(),
                    value,
                    AnchorSetter::Import,
                )
                .await
                {
                    Ok(()) => anchors_merged.push(anchor_type),
                    Err(IdentityTxError::InvalidValue) => {
                        tracing::warn!(
                            work_id,
                            anchor_type = anchor_type_str,
                            value,
                            "claimed completion: skipping invalid anchor value"
                        );
                    }
                    Err(e) => return Err(e.into_work_identity()),
                }
            }
        }

        let now = Utc::now().to_rfc3339();
        for (anchor_type, value) in &completion.pending_guesses {
            sqlx::query(
                "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
                 VALUES (?1, ?2, ?3, 'pending', 'auto_search', ?4, (SELECT user_id FROM works WHERE id = ?1))
                 ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                     confidence = 'pending',
                     setter = 'auto_search',
                     set_at = ?4
                 WHERE work_identity_anchors.confidence != 'confirmed'",
            )
            .bind(work_id)
            .bind(anchor_type.as_str())
            .bind(value)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        }

        if let Some(candidates) = &completion.review_candidates {
            let json = serde_json::to_string(candidates)
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            sqlx::query(
                "INSERT INTO work_identity_review_candidates (work_id, user_id, candidates_json, recorded_at)
                 VALUES (?1, (SELECT user_id FROM works WHERE id = ?1), ?2, ?3)
                 ON CONFLICT (work_id) DO UPDATE SET
                     candidates_json = ?2,
                     recorded_at = ?3",
            )
            .bind(work_id)
            .bind(&json)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            sqlx::query("UPDATE works SET identity_status = 'needs_review' WHERE id = ?1")
                .bind(work_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        }

        for conflict in &completion.conflicts {
            raise_identity_conflict_in_tx(&mut tx, conflict)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        }

        if let Some(badge) = completion.target_badge {
            sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
                .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
                .bind(work_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(IdentityCompletionOutcome::Applied { anchors_merged })
    }

    async fn get_work_with_identity_generation(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(Work, i64), WorkIdentityError> {
        // One SELECT: the row and its generation are inherently coherent.
        let row = sqlx::query("SELECT * FROM works WHERE id = ?1 AND user_id = ?2")
            .bind(work_id)
            .bind(user_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let Some(row) = row else {
            return Err(WorkIdentityError::Db("work not found".into()));
        };
        use sqlx::Row as _;
        let generation: i64 = row
            .try_get("identity_generation")
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let work = row_to_work(row).map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok((work, generation))
    }

    async fn read_anchors_with_generation(
        &self,
        work_id: WorkId,
    ) -> Result<(i64, Vec<WorkIdentityAnchor>), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let generation: i64 =
            sqlx::query_scalar("SELECT identity_generation FROM works WHERE id = ?1")
                .bind(work_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let rows: Vec<(String, String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT anchor_type, anchor_value, confidence, setter, set_at, superseded_by
             FROM work_identity_anchors WHERE work_id = ?1
             ORDER BY set_at DESC",
        )
        .bind(work_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok((generation, rows_to_anchors(work_id, rows)))
    }

    async fn read_review_candidates_with_generation(
        &self,
        work_id: WorkId,
    ) -> Result<(i64, Option<Vec<Candidate>>), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let generation: i64 =
            sqlx::query_scalar("SELECT identity_generation FROM works WHERE id = ?1")
                .bind(work_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT candidates_json FROM work_identity_review_candidates WHERE work_id = ?1",
        )
        .bind(work_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let candidates = match row {
            Some((json,)) => Some(
                serde_json::from_str(&json).map_err(|e| WorkIdentityError::Db(e.to_string()))?,
            ),
            None => None,
        };
        Ok((generation, candidates))
    }

    async fn affirm_anchor_claimed(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
        expected_generation: i64,
    ) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        if !claim_identity_generation(&mut tx, work_id, expected_generation)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?
        {
            return Err(WorkIdentityError::StaleIdentity);
        }

        confirm_anchor_in_tx(&mut tx, work_id, anchor_type, value, setter)
            .await
            .map_err(IdentityTxError::into_work_identity)?;

        let badge = crate::sqlite_identity_conflict::derive_badge_in_tx(&mut tx, work_id)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn apply_review_candidate_claimed(
        &self,
        work_id: WorkId,
        candidate: &Candidate,
        setter: AnchorSetter,
        expected_generation: i64,
    ) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // First-statement conditional generation claim, BEFORE the existing
        // parked-state claim: a lost claim is StaleIdentity (409
        // identity_review_stale at the door), never mislabeled NotParked.
        if !claim_identity_generation(&mut tx, work_id, expected_generation)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?
        {
            return Err(WorkIdentityError::StaleIdentity);
        }

        let guard = sqlx::query(
            "UPDATE works SET identity_status = 'pending' \
             WHERE id = ?1 AND identity_status = 'needs_review'",
        )
        .bind(work_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        if guard.rows_affected() == 0 {
            return Err(WorkIdentityError::NotParked);
        }

        let anchors: &[(&str, Option<&str>)] = &[
            (AnchorType::OL_WORK, candidate.anchors.ol_key.as_deref()),
            (AnchorType::GR_WORK, candidate.anchors.gr_key.as_deref()),
            (AnchorType::HC_WORK, candidate.anchors.hc_key.as_deref()),
            (AnchorType::ISBN_13, candidate.anchors.isbn_13.as_deref()),
            (AnchorType::ASIN, candidate.anchors.asin.as_deref()),
        ];
        for &(anchor_type_str, maybe_value) in anchors {
            if let Some(value) = maybe_value {
                confirm_anchor_in_tx(
                    &mut tx,
                    work_id,
                    AnchorType::new(anchor_type_str),
                    value,
                    setter,
                )
                .await
                .map_err(IdentityTxError::into_work_identity)?;
            }
        }

        let badge = crate::sqlite_identity_conflict::derive_badge_in_tx(&mut tx, work_id)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
            .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        sqlx::query("DELETE FROM work_identity_review_candidates WHERE work_id = ?1")
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn dismiss_review_claimed(
        &self,
        work_id: WorkId,
        expected_generation: i64,
    ) -> Result<(), WorkIdentityError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // Claim before the parked-state guard — same contract as
        // apply_review_candidate_claimed.
        if !claim_identity_generation(&mut tx, work_id, expected_generation)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?
        {
            return Err(WorkIdentityError::StaleIdentity);
        }

        let guard = sqlx::query(
            "UPDATE works SET identity_status = 'pending' \
             WHERE id = ?1 AND identity_status = 'needs_review'",
        )
        .bind(work_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        if guard.rows_affected() == 0 {
            return Err(WorkIdentityError::NotParked);
        }

        sqlx::query("DELETE FROM work_identity_review_candidates WHERE work_id = ?1")
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Identity-edit helpers (design identity-edit r4)
// ---------------------------------------------------------------------------

/// The denormalized works column for a slot. Whitelisted — never interpolate
/// caller strings into SQL.
fn column_for(slot: &AnchorType) -> &'static str {
    match slot.as_str() {
        AnchorType::OL_WORK => "ol_key",
        AnchorType::GR_WORK => "gr_key",
        AnchorType::HC_WORK => "hc_key",
        AnchorType::ISBN_13 => "isbn_13",
        _ => "asin",
    }
}

pub(crate) fn is_work_key(slot: &AnchorType) -> bool {
    matches!(
        slot.as_str(),
        AnchorType::OL_WORK | AnchorType::GR_WORK | AnchorType::HC_WORK
    )
}

fn parse_identity_status(s: &str) -> IdentityStatus {
    match s {
        "confirmed" => IdentityStatus::Confirmed,
        "provisional" => IdentityStatus::Provisional,
        "conflict" => IdentityStatus::Conflict,
        "needs_review" => IdentityStatus::NeedsReview,
        "not_found" => IdentityStatus::NotFound,
        _ => IdentityStatus::Pending,
    }
}

/// The shared ledger∪column badge derivation over an already-read basis:
/// open conflict → Conflict; effective work key → Confirmed; effective
/// bridge → Provisional; else Pending. Quarantined-invalid columns earn
/// nothing.
pub(crate) fn derive_union_badge(basis: &IdentityEditBasis) -> IdentityStatus {
    if !basis.open_conflict_kinds.is_empty() {
        return IdentityStatus::Conflict;
    }
    let has_work_key = [&basis.ol_work, &basis.gr_work, &basis.hc_work]
        .iter()
        .any(|s| s.effective().is_some());
    if has_work_key {
        return IdentityStatus::Confirmed;
    }
    let has_bridge = [&basis.isbn_13, &basis.asin]
        .iter()
        .any(|s| s.effective().is_some());
    if has_bridge {
        return IdentityStatus::Provisional;
    }
    IdentityStatus::Pending
}

/// Classify an edit-transaction sqlx failure per the approved 503 taxonomy:
/// BUSY/LOCKED exhaustion and FULL/IOERR/NOMEM storage failures are
/// retryable-service errors; everything else stays an internal Db error.
fn classify_edit_sqlx(e: sqlx::Error) -> IdentityEditError {
    if let Some(db) = e.as_database_error() {
        if let Some(code) = db.code() {
            if let Ok(code) = code.parse::<i64>() {
                // Primary SQLite result code (low byte of extended codes).
                if matches!(code & 0xff, 5 | 6 | 7 | 10 | 13) {
                    return IdentityEditError::Unavailable;
                }
            }
        }
    }
    IdentityEditError::Db(e.to_string())
}

/// Same-user owner lookup over the validated v2-route∪ledger∪column union — the
/// preview/commit collision authority. The ledger half joins `works` on
/// `user_id` (explicit-user invariant); the column half applies the same
/// filter, so another user's id/title can never be returned. The queried
/// value is already canonical, so column equality implies a valid column.
async fn find_anchor_owner_on<'e, E>(
    executor: E,
    user_id: UserId,
    anchor_type: &AnchorType,
    value: &str,
    exclude_work_id: WorkId,
) -> Result<Option<CollisionInfo>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let route_kind = match anchor_type.as_str() {
        AnchorType::OL_WORK => livrarr_domain::identity_layer::RouteKind::OpenLibraryWork,
        AnchorType::GR_WORK => livrarr_domain::identity_layer::RouteKind::GoodreadsWork,
        AnchorType::HC_WORK => livrarr_domain::identity_layer::RouteKind::HardcoverWork,
        AnchorType::ISBN_13 => livrarr_domain::identity_layer::RouteKind::Isbn13Edition,
        AnchorType::ASIN => livrarr_domain::identity_layer::RouteKind::AsinEdition,
        _ => return Ok(None),
    };
    let route_kind = serde_json::to_string(&route_kind).expect("RouteKind serialization");
    let sql = format!(
        "SELECT id, title FROM ( \
           SELECT w.id AS id, w.title AS title FROM identity_routes r \
             JOIN works w ON w.id = r.resolved_work_id AND w.user_id = r.user_id \
             WHERE r.user_id = ?1 AND r.kind = ?5 AND r.provider_scoped_id = ?3 \
               AND r.state = 'active' AND w.id != ?4 \
           UNION \
           SELECT w.id AS id, w.title AS title FROM work_identity_anchors a \
             JOIN works w ON w.id = a.work_id AND w.user_id = ?1 \
             WHERE a.anchor_type = ?2 AND a.anchor_value = ?3 \
               AND a.confidence = 'confirmed' AND w.id != ?4 \
           UNION \
           SELECT w.id AS id, w.title AS title FROM works w \
             WHERE w.user_id = ?1 AND w.{col} = ?3 AND w.id != ?4 \
         ) ORDER BY id LIMIT 1",
        col = column_for(anchor_type)
    );
    let row: Option<(i64, String)> = sqlx::query_as(&sql)
        .bind(user_id)
        .bind(anchor_type.as_str())
        .bind(value)
        .bind(exclude_work_id)
        .bind(route_kind)
        .fetch_optional(executor)
        .await?;
    Ok(
        row.map(|(owning_work_id, owning_work_title)| CollisionInfo {
            owning_work_id,
            owning_work_title,
        }),
    )
}

/// Delete every pending row and the dead-end row for one slot — required for
/// re-chase: `chaseable_anchor_types` rejects a missing slot while any
/// pending row exists, and the dead-end filter blocks it at threshold.
async fn delete_slot_residue(
    conn: &mut SqliteConnection,
    work_id: WorkId,
    slot_str: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'pending'",
    )
    .bind(work_id)
    .bind(slot_str)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM work_anchor_dead_ends WHERE work_id = ?1 AND anchor_type = ?2")
        .bind(work_id)
        .bind(slot_str)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[derive(Clone)]
struct IdentityRouteSlot {
    provider: IdentityProvider,
    kind: RouteKind,
    edition_scoped: bool,
}

fn identity_route_slot(slot: &AnchorType) -> Option<IdentityRouteSlot> {
    let (provider, kind, edition_scoped) = match slot.as_str() {
        AnchorType::OL_WORK => (
            IdentityProvider::OpenLibrary,
            RouteKind::OpenLibraryWork,
            false,
        ),
        // The legacy `gr_work` edit slot carries a Goodreads Book id. The v2
        // cutover maps that column to edition-scoped evidence, not a Work id.
        AnchorType::GR_WORK => (
            IdentityProvider::Goodreads,
            RouteKind::GoodreadsBookEdition,
            true,
        ),
        AnchorType::HC_WORK => (IdentityProvider::Hardcover, RouteKind::HardcoverWork, false),
        AnchorType::ISBN_13 => (
            IdentityProvider::IsbnRegistry,
            RouteKind::Isbn13Edition,
            true,
        ),
        AnchorType::ASIN => (IdentityProvider::Amazon, RouteKind::AsinEdition, true),
        _ => return None,
    };
    Some(IdentityRouteSlot {
        provider,
        kind,
        edition_scoped,
    })
}

async fn identity_v2_active(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _livrarr_meta \
                        WHERE key='identity_authority_v2' AND value='active')",
    )
    .fetch_one(conn)
    .await
}

async fn active_identity_slot_routes(
    conn: &mut SqliteConnection,
    user_id: UserId,
    work_id: WorkId,
    slot: &AnchorType,
) -> Result<Vec<(i64, Option<i64>, String)>, sqlx::Error> {
    let Some(spec) = identity_route_slot(slot) else {
        return Ok(Vec::new());
    };
    let provider = serde_json::to_string(&spec.provider).expect("IdentityProvider serialization");
    let kind = serde_json::to_string(&spec.kind).expect("RouteKind serialization");
    sqlx::query_as(
        "SELECT id, edition_id, provider_scoped_id FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider=?3 AND kind=?4 \
            AND state='active' ORDER BY id",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(provider)
    .bind(kind)
    .fetch_all(conn)
    .await
}

async fn retire_all_identity_slot_routes(
    conn: &mut SqliteConnection,
    user_id: UserId,
    work_id: WorkId,
    slot: &AnchorType,
) -> Result<(), sqlx::Error> {
    let Some(spec) = identity_route_slot(slot) else {
        return Ok(());
    };
    let provider = serde_json::to_string(&spec.provider).expect("IdentityProvider serialization");
    let kind = serde_json::to_string(&spec.kind).expect("RouteKind serialization");
    sqlx::query(
        "UPDATE identity_routes SET state='retired' \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider=?3 AND kind=?4 \
            AND state='active'",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(provider)
    .bind(kind)
    .execute(conn)
    .await?;
    Ok(())
}

/// Synchronize the certified legacy edit slot into the active v2 route graph.
/// A single existing route (the normal A→B shape) is retired and its Edition
/// reused; plural evidence is preserved unless the frozen legacy projection
/// identifies the exact route being replaced.
async fn sync_identity_edit_route(
    conn: &mut SqliteConnection,
    user_id: UserId,
    work_id: WorkId,
    slot: &AnchorType,
    old_value: Option<&str>,
    new_value: &str,
) -> Result<(), sqlx::Error> {
    let Some(spec) = identity_route_slot(slot) else {
        return Ok(());
    };
    let provider = serde_json::to_string(&spec.provider).expect("IdentityProvider serialization");
    let kind = serde_json::to_string(&spec.kind).expect("RouteKind serialization");
    let provenance =
        serde_json::to_string(&RouteProvenance::UserChoice).expect("RouteProvenance serialization");
    let routes = active_identity_slot_routes(conn, user_id, work_id, slot).await?;
    let singleton = routes.len() == 1;
    let mut replacement_edition = None;
    for (route_id, edition_id, value) in &routes {
        if value == new_value {
            continue;
        }
        let replaces_projected = old_value.is_some_and(|old| old == value);
        if replaces_projected || (old_value.is_none() && singleton) {
            replacement_edition = replacement_edition.or(*edition_id);
            sqlx::query(
                "UPDATE identity_routes SET state='retired' \
                  WHERE user_id=?1 AND id=?2 AND state='active'",
            )
            .bind(user_id)
            .bind(route_id)
            .execute(&mut *conn)
            .await?;
        }
    }

    if routes.iter().any(|(_, _, value)| value == new_value) {
        sqlx::query(
            "UPDATE identity_routes SET provenance=?1, user_confirmed=1, observed_at=?2 \
              WHERE user_id=?3 AND resolved_work_id=?4 AND provider=?5 AND kind=?6 \
                AND provider_scoped_id=?7 AND state='active'",
        )
        .bind(&provenance)
        .bind(Utc::now().to_rfc3339())
        .bind(user_id)
        .bind(work_id)
        .bind(&provider)
        .bind(&kind)
        .bind(new_value)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }

    let retired: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, edition_id FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider=?3 AND kind=?4 \
            AND provider_scoped_id=?5 AND state='retired' ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(&provider)
    .bind(&kind)
    .bind(new_value)
    .fetch_optional(&mut *conn)
    .await?;

    let edition_id = if spec.edition_scoped {
        match replacement_edition.or_else(|| retired.and_then(|(_, edition_id)| edition_id)) {
            Some(edition_id) => {
                sqlx::query(
                    "UPDATE editions SET state='active', source_provider=?1, \
                            provider_edition_id=?2 \
                      WHERE user_id=?3 AND id=?4 AND work_id=?5",
                )
                .bind(&provider)
                .bind(new_value)
                .bind(user_id)
                .bind(edition_id)
                .bind(work_id)
                .execute(&mut *conn)
                .await?;
                Some(edition_id)
            }
            None => Some(
                sqlx::query(
                    "INSERT INTO editions \
                        (user_id, work_id, format, source_provider, provider_edition_id, state) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
                )
                .bind(user_id)
                .bind(work_id)
                .bind(
                    serde_json::to_string(&EditionFormat::Unknown)
                        .expect("EditionFormat serialization"),
                )
                .bind(&provider)
                .bind(new_value)
                .execute(&mut *conn)
                .await?
                .last_insert_rowid(),
            ),
        }
    } else {
        None
    };
    let (owner_type, owner_work_id) = if spec.edition_scoped {
        ("edition", None)
    } else {
        ("work", Some(work_id))
    };
    if let Some((route_id, _)) = retired {
        sqlx::query(
            "UPDATE identity_routes \
                SET owner_type=?1, work_id=?2, edition_id=?3, state='active', \
                    provenance=?4, user_confirmed=1, observed_at=?5 \
              WHERE user_id=?6 AND id=?7",
        )
        .bind(owner_type)
        .bind(owner_work_id)
        .bind(edition_id)
        .bind(&provenance)
        .bind(Utc::now().to_rfc3339())
        .bind(user_id)
        .bind(route_id)
        .execute(&mut *conn)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO identity_routes \
                (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
                 provider_scoped_id, state, provenance, user_confirmed, observed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, 1, ?10)",
        )
        .bind(user_id)
        .bind(owner_type)
        .bind(owner_work_id)
        .bind(edition_id)
        .bind(work_id)
        .bind(&provider)
        .bind(&kind)
        .bind(new_value)
        .bind(&provenance)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn recompute_v2_identity_status(
    conn: &mut SqliteConnection,
    user_id: UserId,
    work_id: WorkId,
) -> Result<(), sqlx::Error> {
    let (route_count, any_confirmed): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(MAX(user_confirmed), 0) FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND state='active'",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_one(&mut *conn)
    .await?;
    let status = if any_confirmed != 0 {
        "user_confirmed"
    } else if route_count != 0 {
        "connected"
    } else {
        "not_connected"
    };
    sqlx::query("UPDATE works SET identity_status_v2=?1 WHERE user_id=?2 AND id=?3")
        .bind(status)
        .bind(user_id)
        .bind(work_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Edit-transaction failure: a typed edit outcome, or a raw `sqlx::Error`
/// kept typed until the boundary's constraint classification (unique-index
/// backstop → Collision; BUSY/FULL taxonomy → 503-class).
enum EditTxFailure {
    Edit(IdentityEditError),
    Sqlx(sqlx::Error),
}

impl From<sqlx::Error> for EditTxFailure {
    fn from(e: sqlx::Error) -> Self {
        EditTxFailure::Sqlx(e)
    }
}

/// The certified-edit transaction body (design §Commit). The caller owns
/// commit/rollback and the unique-violation backstop reclassification.
#[allow(clippy::too_many_arguments)]
async fn apply_identity_edit_in_tx(
    tx: &mut SqliteConnection,
    work_id: WorkId,
    user_id: UserId,
    slot: &AnchorType,
    new_value: &str,
    expected_generation: i64,
    drop_slots: &[AnchorType],
) -> Result<(), EditTxFailure> {
    // 0. Generation CAS is the transaction's FIRST statement — zero rows is
    // typed StalePreview; there is no read-then-compare window.
    let claimed = sqlx::query(
        "UPDATE works SET identity_generation = identity_generation + 1 \
         WHERE id = ?1 AND user_id = ?2 AND identity_generation = ?3",
    )
    .bind(work_id)
    .bind(user_id)
    .bind(expected_generation)
    .execute(&mut *tx)
    .await?;
    if claimed.rows_affected() == 0 {
        return Err(EditTxFailure::Edit(IdentityEditError::StalePreview));
    }

    let v2_active = identity_v2_active(&mut *tx).await?;
    let (route_graph_before, old_route_value) = if v2_active {
        let graph =
            crate::identity_layer::read_active_route_graph(&mut *tx, user_id, work_id).await?;
        let value: Option<String> = sqlx::query_scalar(&format!(
            "SELECT {} FROM works WHERE user_id=?1 AND id=?2",
            column_for(slot)
        ))
        .bind(user_id)
        .bind(work_id)
        .fetch_one(&mut *tx)
        .await?;
        (Some(graph), value.filter(|value| !value.trim().is_empty()))
    } else {
        (None, None)
    };

    // 1. Work-key slots: in-tx, user-filtered collision re-check over the
    // validated ledger∪column projection. The per-user work-key unique index
    // is the race backstop (classified by the caller).
    if is_work_key(slot) {
        if let Some(owner) =
            find_anchor_owner_on(&mut *tx, user_id, slot, new_value, work_id).await?
        {
            return Err(EditTxFailure::Edit(IdentityEditError::Collision {
                owning_work_id: owner.owning_work_id,
                owning_work_title: owner.owning_work_title,
            }));
        }
    }

    let slot_str = slot.as_str().to_string();

    // 2. Current confirmed row for the slot (value ≠ new): superseded, with
    // the replacing value recorded. A column-only current value has no row to
    // supersede; the caller captured it for history before the transaction.
    sqlx::query(
        "UPDATE work_identity_anchors \
         SET confidence = 'superseded', superseded_by = ?3 \
         WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed' \
           AND anchor_value != ?3",
    )
    .bind(work_id)
    .bind(&slot_str)
    .bind(new_value)
    .execute(&mut *tx)
    .await?;

    // 3. Validation, generation bump, anchor upsert, and column sync at the
    // one anchor-write chokepoint.
    confirm_anchor_in_tx(
        &mut *tx,
        work_id,
        slot.clone(),
        new_value,
        AnchorSetter::User,
    )
    .await
    .map_err(|e| match e {
        IdentityTxError::InvalidValue => EditTxFailure::Edit(IdentityEditError::InvalidValue(
            "identifier is not in canonical form".into(),
        )),
        IdentityTxError::Sqlx(e) => EditTxFailure::Sqlx(e),
    })?;

    // 4. Close superseded disputes: same-slot open conflicts, plus QuorumTie
    // for a work-key commit (a user-certified work key IS the work-level
    // tie-break). Closing is what disarms a stale "Use New Match" replay.
    let mut closed_kinds: Vec<&str> = match slot.as_str() {
        AnchorType::OL_WORK => vec!["incoming_different_ol_key", "ol_redirect_collision"],
        AnchorType::GR_WORK => vec!["incoming_different_gr_key"],
        AnchorType::HC_WORK => vec!["incoming_different_hc_key"],
        _ => vec![],
    };
    if is_work_key(slot) {
        closed_kinds.push("quorum_tie");
    }
    if !closed_kinds.is_empty() {
        let now = Utc::now().to_rfc3339();
        let placeholders = closed_kinds
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE work_identity_conflicts \
             SET status = 'resolved', resolved_at = ?2, \
                 resolution_notes = 'superseded by user identity edit' \
             WHERE existing_work_id = ?1 AND status = 'open' AND kind IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql).bind(work_id).bind(&now);
        for kind in &closed_kinds {
            query = query.bind(*kind);
        }
        query.execute(&mut *tx).await?;
    }

    // 5. Sibling drops: exactly the snapshot's drop set (the generation claim
    // guarantees no identity writer changed it since preview). Dropped ≠
    // destroyed: cleared and re-chaseable. Bridges never enter a drop set.
    for drop_slot in drop_slots {
        let drop_str = drop_slot.as_str().to_string();
        sqlx::query(
            "UPDATE work_identity_anchors \
             SET confidence = 'superseded', superseded_by = NULL \
             WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed'",
        )
        .bind(work_id)
        .bind(&drop_str)
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "UPDATE works SET {} = NULL WHERE id = ?1",
            column_for(drop_slot)
        ))
        .bind(work_id)
        .execute(&mut *tx)
        .await?;
        delete_slot_residue(&mut *tx, work_id, &drop_str).await?;
    }

    // 6. The edited slot's pending rows and dead-end go too: a replaced
    // pending guess is neither visible nor later affirmable.
    delete_slot_residue(&mut *tx, work_id, &slot_str).await?;

    // 7. The ENRICHMENT CAS: an in-flight old-anchor field merge fails and
    // reports Superseded.
    sqlx::query("UPDATE works SET merge_generation = merge_generation + 1 WHERE id = ?1")
        .bind(work_id)
        .execute(&mut *tx)
        .await?;

    // 8. Badge over the validated ledger∪column projection.
    let badge = crate::sqlite_identity_conflict::derive_badge_in_tx(&mut *tx, work_id).await?;
    sqlx::query("UPDATE works SET identity_status = ?1 WHERE id = ?2")
        .bind(crate::sqlite_identity_conflict::identity_status_str(badge))
        .bind(work_id)
        .execute(&mut *tx)
        .await?;

    if let Some(before) = route_graph_before.as_ref() {
        for drop_slot in drop_slots {
            retire_all_identity_slot_routes(&mut *tx, user_id, work_id, drop_slot).await?;
        }
        sync_identity_edit_route(
            &mut *tx,
            user_id,
            work_id,
            slot,
            old_route_value.as_deref(),
            new_value,
        )
        .await?;
        recompute_v2_identity_status(&mut *tx, user_id, work_id).await?;
        crate::identity_layer::invalidate_retry_state_if_route_graph_changed(
            &mut *tx, user_id, work_id, before,
        )
        .await?;
    }

    Ok(())
}

/// In-transaction conflict raise: generation bump + idempotent dedup +
/// insert + Conflict badge, shared by `raise_identity_conflict` and the
/// claimed completion path.
pub(crate) async fn raise_identity_conflict_in_tx(
    conn: &mut SqliteConnection,
    conflict: &NewIdentityConflict,
) -> Result<i64, sqlx::Error> {
    let kind_str = serde_json::to_value(conflict.kind)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    let raised_by_str = serde_json::to_value(conflict.raised_by)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "manual_add".to_string());
    let incoming_json = serde_json::to_string(&conflict.incoming)
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    let raised_at_str = Utc::now().to_rfc3339();

    // A raised conflict advances the generation even when no slot changes —
    // an edit previewed before the raise must not silently resolve a
    // conflict the preview never showed (design §Claims).
    bump_identity_generation(conn, conflict.existing_work_id).await?;

    // Idempotency (REQ-020): one open conflict per (work, kind). A repeated
    // converge/add pass must not duplicate an already-surfaced conflict.
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM work_identity_conflicts
         WHERE existing_work_id = ?1 AND kind = ?2 AND status = 'open'
         ORDER BY id DESC LIMIT 1",
    )
    .bind(conflict.existing_work_id)
    .bind(&kind_str)
    .fetch_optional(&mut *conn)
    .await?;

    let conflict_id = if let Some(id) = existing {
        id
    } else {
        let result = sqlx::query(
            "INSERT INTO work_identity_conflicts
             (user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, raised_source_path, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open')",
        )
        .bind(conflict.user_id)
        .bind(conflict.existing_work_id)
        .bind(&kind_str)
        .bind(&incoming_json)
        .bind(&raised_at_str)
        .bind(&raised_by_str)
        .bind(conflict.raised_source_path.as_deref())
        .execute(&mut *conn)
        .await?;
        result.last_insert_rowid()
    };

    // An open identity contradiction now exists for this work — reflect it in
    // the persisted identity badge (REQ-014/D-013) so reads surface Conflict.
    sqlx::query("UPDATE works SET identity_status = 'conflict' WHERE id = ?1 AND user_id = ?2")
        .bind(conflict.existing_work_id)
        .bind(conflict.user_id)
        .execute(&mut *conn)
        .await?;

    Ok(conflict_id)
}

/// Parse raw anchor rows into domain anchors (shared by `list_anchors` and
/// the generation-coherent read).
fn rows_to_anchors(
    work_id: WorkId,
    rows: Vec<(String, String, String, String, String, Option<String>)>,
) -> Vec<WorkIdentityAnchor> {
    let mut anchors = Vec::new();
    for (atype, aval, conf, setter, set_at, superseded) in rows {
        let confidence = match conf.as_str() {
            "confirmed" => AnchorConfidence::Confirmed,
            "pending" => AnchorConfidence::Pending,
            "superseded" => AnchorConfidence::Superseded,
            _ => continue,
        };
        let setter = serde_json::from_value(serde_json::Value::String(setter.clone()))
            // Least-privilege default: an unreadable setter must never be treated
            // as user-authoritative — that would suppress legitimate conflict raises.
            .unwrap_or(AnchorSetter::AutoSearch);
        let set_at = chrono::DateTime::parse_from_rfc3339(&set_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        anchors.push(WorkIdentityAnchor {
            work_id,
            anchor_type: AnchorType::new(atype),
            anchor_value: aval,
            confidence,
            setter,
            set_at,
            superseded_by: superseded,
        });
    }
    anchors
}

// ---------------------------------------------------------------------------
// Startup ledger completion (design §Migration 076 + startup ledger completion)
// ---------------------------------------------------------------------------

/// One-shot startup pass that completes the identity ledger from the legacy
/// `works.*` columns (ground truth 6b: HC/ISBN/ASIN were never backfilled and
/// the GR backfill has no production caller). Runs in the exclusive
/// pre-service startup sequence, so no `identity_generation` bump is needed —
/// no online writer can race it. Rust, not SQL: the canonical normalizers are
/// not callable from a migration.
///
/// Atomic marker-last: rows and the completion marker commit together; any
/// database/storage error rolls both back and fails startup. Invalid user
/// data is a quarantined column (kept visible/clearable, no ledger row), not
/// a pass failure.
pub async fn backfill_work_identity_ledger(pool: &sqlx::SqlitePool) -> Result<(), String> {
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key = 'work_identity_ledger_backfill_complete'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("read work_identity_ledger_backfill_complete: {e}"))?;
    if marker.as_deref() == Some("1") {
        return Ok(());
    }

    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|e| format!("begin identity ledger backfill: {e}"))?;

    type WorkRow = (
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let works: Vec<WorkRow> = sqlx::query_as(
        "SELECT id, user_id, ol_key, gr_key, hc_key, isbn_13, asin FROM works \
         WHERE ol_key IS NOT NULL OR gr_key IS NOT NULL OR hc_key IS NOT NULL \
            OR isbn_13 IS NOT NULL OR asin IS NOT NULL \
         ORDER BY user_id, id",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| format!("scan works columns: {e}"))?;

    // The ledger carries its own `user_id` (migration 044) and the 076 uniqueness index
    // keys on THAT column, so ownership must be read from it directly. Deriving the user
    // by joining through `works` instead only sees works with a non-NULL legacy column,
    // which makes a ledger-only owner invisible: a second owner is then elected for the
    // same (user, type, value) and the insert fails the index, failing startup.
    let confirmed_rows: Vec<(i64, Option<i64>, String, String)> = sqlx::query_as(
        "SELECT work_id, user_id, anchor_type, anchor_value FROM work_identity_anchors \
         WHERE confidence = 'confirmed'",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| format!("scan confirmed ledger rows: {e}"))?;
    // (work_id, anchor_type) → confirmed value; and (user, type, value) → owner.
    let mut confirmed_by_work: std::collections::HashMap<(i64, String), String> =
        std::collections::HashMap::new();
    for (w, _, t, v) in &confirmed_rows {
        confirmed_by_work.insert((*w, t.clone()), v.clone());
    }
    let mut owner_of: std::collections::HashMap<(i64, String, String), i64> =
        std::collections::HashMap::new();
    for (w, user, t, v) in &confirmed_rows {
        // 044 made `user_id` nullable. A NULL row sits outside the 076 partial index
        // and so cannot collide with anything — it never claims ownership.
        if let Some(user) = user {
            owner_of.insert((*user, t.clone(), v.clone()), *w);
        }
    }

    // Per (work, slot) decision, in (user_id, id, slot) order.
    struct PlannedRow {
        work_id: i64,
        user_id: i64,
        slot: &'static str,
        canonical: String,
        rewrite_column: bool,
    }
    let mut planned: Vec<PlannedRow> = Vec::new();
    for (work_id, user_id, ol, gr, hc, isbn, asin) in &works {
        let slots: [(&'static str, &Option<String>); 5] = [
            (AnchorType::OL_WORK, ol),
            (AnchorType::GR_WORK, gr),
            (AnchorType::HC_WORK, hc),
            (AnchorType::ISBN_13, isbn),
            (AnchorType::ASIN, asin),
        ];
        for (slot, raw) in slots {
            let Some(raw) = raw.as_deref().filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            // Same domain normalizers as the write chokepoint; a valid
            // noncanonical value is rewritten, an invalid one is quarantined
            // in place (raw column kept so the user can see and clear it).
            let canonical = match slot {
                AnchorType::GR_WORK => normalize_gr_key(raw),
                AnchorType::ISBN_13 => normalize_isbn13(raw),
                AnchorType::ASIN => match normalize_asin(raw) {
                    AsinNorm::Asin(a) => Some(a),
                    _ => None,
                },
                _ => Some(raw.to_string()),
            };
            let Some(canonical) = canonical else {
                tracing::warn!(
                    work_id,
                    slot,
                    "identity backfill: quarantined invalid column"
                );
                continue;
            };
            if let Some(existing) = confirmed_by_work.get(&(*work_id, slot.to_string())) {
                if existing != &canonical {
                    // Pre-existing ledger/column disagreement — never
                    // overwritten; left for the consistency surface.
                    tracing::warn!(
                        work_id,
                        slot,
                        "identity backfill: ledger/column disagreement left in place"
                    );
                }
                continue;
            }
            planned.push(PlannedRow {
                work_id: *work_id,
                user_id: *user_id,
                slot,
                canonical: canonical.clone(),
                rewrite_column: canonical != raw,
            });
        }
    }

    // Work-key ownership per (user, type, canonical): an existing confirmed
    // owner is preserved; with no owner the lowest work id deterministically
    // gets the ledger row; every other member stays canonical column-only.
    // Bridges insert per work (076 removed their same-user uniqueness).
    let mut group_winner: std::collections::HashMap<(i64, String, String), i64> =
        std::collections::HashMap::new();
    for row in &planned {
        if !is_work_key(&AnchorType::new(row.slot)) {
            continue;
        }
        let key = (row.user_id, row.slot.to_string(), row.canonical.clone());
        if let Some(owner) = owner_of.get(&key) {
            group_winner.insert(key, *owner);
        } else {
            let entry = group_winner.entry(key).or_insert(row.work_id);
            *entry = (*entry).min(row.work_id);
        }
    }

    let now = Utc::now().to_rfc3339();
    for row in &planned {
        if row.rewrite_column {
            sqlx::query(&format!(
                "UPDATE works SET {} = ?1 WHERE id = ?2",
                column_for(&AnchorType::new(row.slot))
            ))
            .bind(&row.canonical)
            .bind(row.work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("canonicalize column: {e}"))?;
        }
        if is_work_key(&AnchorType::new(row.slot)) {
            let key = (row.user_id, row.slot.to_string(), row.canonical.clone());
            if group_winner.get(&key) != Some(&row.work_id) {
                continue;
            }
        }
        sqlx::query(
            "INSERT INTO work_identity_anchors \
             (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
             VALUES (?1, ?2, ?3, 'confirmed', 'import', ?4, ?5)",
        )
        .bind(row.work_id)
        .bind(row.slot)
        .bind(&row.canonical)
        .bind(&now)
        .bind(row.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("insert ledger row: {e}"))?;
    }

    // Marker is the LAST statement before commit — a mid-pass failure rolls
    // back rows and marker together (the existing atomic marker-last idiom).
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('work_identity_ledger_backfill_complete', '1') \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("stamp identity ledger backfill marker: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("commit identity ledger backfill: {e}"))?;
    Ok(())
}
