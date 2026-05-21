use chrono::Utc;
use livrarr_domain::identity::*;
use livrarr_domain::services::{WorkIdentityError, WorkIdentityRepository};
use livrarr_domain::WorkId;

use crate::sqlite::SqliteDb;

impl WorkIdentityRepository for SqliteDb {
    async fn confirm_ol_anchor(
        &self,
        work_id: WorkId,
        ol_key: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        if ol_key.trim().is_empty() {
            return Err(WorkIdentityError::InvalidAnchorValue);
        }

        let now = Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(&setter)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "user".to_string());

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query(
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at)
             VALUES (?1, 'ol_work', ?2, 'confirmed', ?3, ?4)
             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                 confidence = 'confirmed',
                 setter = ?3,
                 set_at = ?4,
                 superseded_by = NULL"
        )
        .bind(work_id)
        .bind(ol_key)
        .bind(&setter_str)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query("UPDATE works SET ol_key = ?1 WHERE id = ?2")
            .bind(ol_key)
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn supersede_ol_anchor(
        &self,
        work_id: WorkId,
        old_ol_key: &str,
        new_ol_key: &str,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        if old_ol_key.trim().is_empty() || new_ol_key.trim().is_empty() || old_ol_key == new_ol_key
        {
            return Err(WorkIdentityError::InvalidAnchorValue);
        }
        let now = Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(&setter)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "redirect".to_string());

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        let rows = sqlx::query(
            "UPDATE work_identity_anchors SET confidence = 'superseded', superseded_by = ?1
             WHERE work_id = ?2 AND anchor_type = 'ol_work' AND anchor_value = ?3 AND confidence = 'confirmed'"
        )
        .bind(new_ol_key)
        .bind(work_id)
        .bind(old_ol_key)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        if rows.rows_affected() == 0 {
            return Err(WorkIdentityError::AnchorNotFound);
        }

        sqlx::query(
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at)
             VALUES (?1, 'ol_work', ?2, 'confirmed', ?3, ?4)
             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                 confidence = 'confirmed',
                 setter = ?3,
                 set_at = ?4,
                 superseded_by = NULL"
        )
        .bind(work_id)
        .bind(new_ol_key)
        .bind(&setter_str)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        sqlx::query("UPDATE works SET ol_key = ?1 WHERE id = ?2")
            .bind(new_ol_key)
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| WorkIdentityError::Db(e.to_string()))?;
        Ok(())
    }

    async fn set_identity_pending(
        &self,
        work_id: WorkId,
        _reason: PendingReason,
        setter: AnchorSetter,
    ) -> Result<(), WorkIdentityError> {
        let now = Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(&setter)
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
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at)
             VALUES (?1, 'ol_work', '', 'pending', ?2, ?3)
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

        sqlx::query(
            "UPDATE works SET ol_key = NULL, enrichment_status = 'identity_pending' WHERE id = ?1",
        )
        .bind(work_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| WorkIdentityError::Db(e.to_string()))?;

        tx.commit()
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
}
