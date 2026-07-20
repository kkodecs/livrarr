use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{record_history, WorkDb};
use livrarr_domain::history_events;
use livrarr_domain::identity::*;
use livrarr_domain::services::{ConflictError, IdentityConflictService, WorkIdentityRepository};
use livrarr_domain::UserId;

pub struct LiveIdentityConflictService {
    db: SqliteDb,
}

impl LiveIdentityConflictService {
    pub fn new(db: SqliteDb) -> Self {
        Self { db }
    }
}

impl IdentityConflictService for LiveIdentityConflictService {
    async fn raise(&self, conflict: NewIdentityConflict) -> Result<i64, ConflictError> {
        // Delegate to the transactional repo method which deduplicates by (work, kind),
        // atomically inserts the conflict row, and stamps the work badge in one tx.
        // The old path (find_existing_open_conflict + create_identity_conflict) was
        // OL-key-only for dedup and did not update the badge (Fix 3 / R-3).
        self.db
            .raise_identity_conflict(conflict)
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))
    }

    async fn list_open(&self, user_id: UserId) -> Result<Vec<IdentityConflict>, ConflictError> {
        self.db
            .list_identity_conflicts_by_status(user_id, ConflictStatus::Open)
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))
    }

    async fn get(
        &self,
        id: i64,
        user_id: UserId,
    ) -> Result<Option<IdentityConflict>, ConflictError> {
        self.db
            .get_identity_conflict(id, user_id)
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))
    }

    async fn resolve(
        &self,
        id: i64,
        user_id: UserId,
        action: ConflictResolutionAction,
        notes: Option<String>,
    ) -> Result<(), ConflictError> {
        let conflict = self
            .db
            .get_identity_conflict(id, user_id)
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))?
            .ok_or(ConflictError::NotFound)?;

        if conflict.user_id != user_id {
            return Err(ConflictError::NotFound);
        }

        if conflict.status != ConflictStatus::Open {
            return Err(ConflictError::AlreadyResolved);
        }

        self.db
            .apply_conflict_resolution(&conflict, action, notes.as_deref(), Utc::now())
            .await
            .map_err(|e| match e {
                livrarr_db::ConflictApplyError::AlreadyResolved => ConflictError::AlreadyResolved,
                livrarr_db::ConflictApplyError::InvalidAnchorValue => {
                    ConflictError::InvalidPrimaryAnchor
                }
                livrarr_db::ConflictApplyError::Db(db_err) => ConflictError::Db(db_err.to_string()),
            })?;

        let work_title = self
            .db
            .get_work(user_id, conflict.existing_work_id)
            .await
            .map(|w| w.title)
            .unwrap_or_default();
        let action_label = match action {
            ConflictResolutionAction::KeepExisting => "keep-existing",
            ConflictResolutionAction::AcceptSeparate => "accept-separate",
            ConflictResolutionAction::ReplaceAnchor => "replace-anchor",
            ConflictResolutionAction::Merge => "merge",
        };
        let identity = format!(
            "{} — {}{}",
            conflict.incoming.title,
            conflict.incoming.author_name,
            first_anchor_suffix(&conflict.incoming)
        );
        record_history(
            &self.db,
            user_id,
            history_events::identity_resolved(
                conflict.existing_work_id,
                &work_title,
                action_label,
                identity,
            ),
        )
        .await;

        Ok(())
    }

    async fn dismiss(&self, id: i64, user_id: UserId) -> Result<(), ConflictError> {
        let conflict = self
            .db
            .get_identity_conflict(id, user_id)
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))?
            .ok_or(ConflictError::NotFound)?;

        if conflict.user_id != user_id {
            return Err(ConflictError::NotFound);
        }

        if conflict.status != ConflictStatus::Open {
            return Err(ConflictError::AlreadyResolved);
        }

        self.db
            .apply_conflict_dismiss(&conflict, Utc::now())
            .await
            .map_err(|e| match e {
                livrarr_db::ConflictApplyError::AlreadyResolved => ConflictError::AlreadyResolved,
                livrarr_db::ConflictApplyError::Db(db_err) => ConflictError::Db(db_err.to_string()),
                // apply_conflict_dismiss never returns InvalidAnchorValue; map defensively
                livrarr_db::ConflictApplyError::InvalidAnchorValue => {
                    ConflictError::Db("unexpected anchor validation error in dismiss".to_string())
                }
            })
    }
}

/// The first present anchor on an incoming conflict payload, formatted as
/// " (label value)" for appending to an identity summary — empty when the
/// payload carries no anchor.
fn first_anchor_suffix(incoming: &IncomingConflictPayload) -> String {
    let anchor = incoming
        .ol_key
        .as_deref()
        .map(|v| ("ol_key", v))
        .or_else(|| incoming.gr_key.as_deref().map(|v| ("gr_key", v)))
        .or_else(|| incoming.hc_key.as_deref().map(|v| ("hc_key", v)))
        .or_else(|| incoming.isbn_13.as_deref().map(|v| ("isbn_13", v)))
        .or_else(|| incoming.asin.as_deref().map(|v| ("asin", v)));
    match anchor {
        Some((label, value)) => format!(" ({label} {value})"),
        None => String::new(),
    }
}
