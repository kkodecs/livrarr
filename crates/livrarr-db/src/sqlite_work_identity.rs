use chrono::Utc;
use livrarr_domain::identity::*;
use livrarr_domain::normalization::{normalize_asin, normalize_gr_key, normalize_isbn13, AsinNorm};
use livrarr_domain::services::{WorkIdentityError, WorkIdentityRepository};
use livrarr_domain::WorkId;

use crate::sqlite::SqliteDb;

impl WorkIdentityRepository for SqliteDb {
    async fn confirm_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        if value.trim().is_empty() {
            return Err(WorkIdentityError::InvalidAnchorValue);
        }
        // Defense in depth (REQ-029): a typed identifier must already be in its
        // canonical form before it is persisted. Callers normalize via
        // WorkSeed::sanitized, but validating here means a malformed value can
        // never reach a row regardless of the call path.
        let canonical = match anchor_type.as_str() {
            AnchorType::ISBN_13 => normalize_isbn13(value).as_deref() == Some(value),
            AnchorType::GR_WORK => normalize_gr_key(value).as_deref() == Some(value),
            AnchorType::ASIN => matches!(normalize_asin(value), AsinNorm::Asin(a) if a == value),
            // ol_work / hc_work have no canonical form beyond the non-empty check.
            _ => true,
        };
        if !canonical {
            return Err(WorkIdentityError::InvalidAnchorValue);
        }
        let now = Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(setter)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "user".to_string());
        let anchor_type_str = anchor_type.as_str().to_string();

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

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
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        // Keep the legacy denormalized work column in sync with the confirmed
        // anchor across every federated type (REQ-022 convergence reads these
        // columns; an OL/GR-only sync leaves hc_key/isbn_13/asin stale).
        match anchor_type.as_str() {
            AnchorType::OL_WORK => {
                sqlx::query("UPDATE works SET ol_key = ?1 WHERE id = ?2")
                    .bind(value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            AnchorType::GR_WORK => {
                sqlx::query("UPDATE works SET gr_key = ?1 WHERE id = ?2")
                    .bind(value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            AnchorType::HC_WORK => {
                sqlx::query("UPDATE works SET hc_key = ?1 WHERE id = ?2")
                    .bind(value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            AnchorType::ISBN_13 => {
                sqlx::query("UPDATE works SET isbn_13 = ?1 WHERE id = ?2")
                    .bind(value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            AnchorType::ASIN => {
                sqlx::query("UPDATE works SET asin = ?1 WHERE id = ?2")
                    .bind(value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            _ => {}
        }

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn supersede_anchor(
        &self,
        work_id: WorkId,
        anchor_type: AnchorType,
        old_value: &str,
        new_value: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        if old_value.trim().is_empty() || new_value.trim().is_empty() || old_value == new_value {
            return Err(WorkIdentityError::InvalidAnchorValue);
        }
        let now = Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(setter)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "redirect".to_string());
        let anchor_type_str = anchor_type.as_str().to_string();

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let rows = sqlx::query(
            "UPDATE work_identity_anchors SET confidence = 'superseded', superseded_by = ?1
             WHERE work_id = ?2 AND anchor_type = ?3 AND anchor_value = ?4 AND confidence = 'confirmed'",
        )
        .bind(new_value)
        .bind(work_id)
        .bind(&anchor_type_str)
        .bind(old_value)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        if rows.rows_affected() == 0 {
            return Err(WorkIdentityError::AnchorNotFound);
        }

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
        .bind(new_value)
        .bind(&setter_str)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        match anchor_type.as_str() {
            AnchorType::OL_WORK => {
                sqlx::query("UPDATE works SET ol_key = ?1 WHERE id = ?2")
                    .bind(new_value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            AnchorType::GR_WORK => {
                sqlx::query("UPDATE works SET gr_key = ?1 WHERE id = ?2")
                    .bind(new_value)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            }
            _ => {}
        }

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
                    self.confirm_anchor(work_id, anchor_type.clone(), value, AnchorSetter::Import)
                        .await?;
                    merged.push(anchor_type);
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

        let confirmed: std::collections::HashMap<String, String> = existing
            .iter()
            .filter(|a| a.confidence == AnchorConfidence::Confirmed)
            .map(|a| (a.anchor_type.as_str().to_string(), a.anchor_value.clone()))
            .collect();

        let mut conflicts = Vec::new();
        let checks: &[(&str, Option<&str>, IdentityConflictKind)] = &[
            (
                AnchorType::OL_WORK,
                incoming.ol_key.as_deref(),
                IdentityConflictKind::IncomingDifferentOlKey,
            ),
            (
                AnchorType::GR_WORK,
                incoming.gr_key.as_deref(),
                IdentityConflictKind::IncomingDifferentGrKey,
            ),
            (
                AnchorType::HC_WORK,
                incoming.hc_key.as_deref(),
                IdentityConflictKind::IncomingDifferentHcKey,
            ),
        ];

        for &(anchor_type_str, incoming_value, kind) in checks {
            if let Some(incoming_val) = incoming_value {
                if let Some(existing_val) = confirmed.get(anchor_type_str) {
                    if incoming_val != existing_val.as_str() {
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
        let kind_str = serde_json::to_value(conflict.kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        // Idempotency (REQ-020): one open conflict per (work, kind). A repeated
        // converge/add pass must not duplicate an already-surfaced conflict.
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM work_identity_conflicts
             WHERE existing_work_id = ?1 AND kind = ?2 AND status = 'open'
             ORDER BY id DESC LIMIT 1",
        )
        .bind(conflict.existing_work_id)
        .bind(&kind_str)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        let conflict_id = if let Some(id) = existing {
            id
        } else {
            let incoming_json = serde_json::to_string(&conflict.incoming)
                .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
            self.create_identity_conflict(
                conflict.user_id,
                conflict.existing_work_id,
                conflict.kind,
                &incoming_json,
                Utc::now(),
                conflict.raised_by,
                conflict.raised_source_path.as_deref(),
            )
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?
        };
        // An open identity contradiction now exists for this work — reflect it in
        // the persisted identity badge (REQ-014/D-013) so reads surface Conflict.
        sqlx::query("UPDATE works SET identity_status = 'conflict' WHERE id = ?1 AND user_id = ?2")
            .bind(conflict.existing_work_id)
            .bind(conflict.user_id)
            .execute(self.pool())
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

        let mut tx = self
            .pool()
            .begin()
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
        sqlx::query("UPDATE works SET identity_status = 'needs_review' WHERE id = ?1")
            .bind(work_id)
            .execute(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn set_identity_confirmed(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        sqlx::query("UPDATE works SET identity_status = 'confirmed' WHERE id = ?1")
            .bind(work_id)
            .execute(self.pool())
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn set_identity_provisional(&self, work_id: WorkId) -> Result<(), WorkIdentityError> {
        sqlx::query("UPDATE works SET identity_status = 'provisional' WHERE id = ?1")
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
                .unwrap_or(AnchorSetter::User);
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
        .execute(self.pool())
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
}
