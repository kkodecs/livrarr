use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
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
            .map_err(|e| ConflictError::Db(e.to_string()))
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
            .map_err(|e| ConflictError::Db(e.to_string()))
    }
}
