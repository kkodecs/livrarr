use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
use livrarr_domain::identity::*;
use livrarr_domain::services::{ConflictError, IdentityConflictService};
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
        let incoming_json = serde_json::to_string(&conflict.incoming)
            .map_err(|e| ConflictError::SerializationFailed(e.to_string()))?;

        if let Some(existing_id) = self
            .db
            .find_existing_open_conflict(
                conflict.existing_work_id,
                conflict.incoming.ol_key.as_deref().unwrap_or(""),
            )
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))?
        {
            return Ok(existing_id);
        }

        let id = self
            .db
            .create_identity_conflict(
                conflict.user_id,
                conflict.existing_work_id,
                conflict.kind,
                &incoming_json,
                Utc::now(),
                conflict.raised_by,
                conflict.raised_source_path.as_deref(),
            )
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))?;

        Ok(id)
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
        _user_id: UserId,
    ) -> Result<Option<IdentityConflict>, ConflictError> {
        self.db
            .get_identity_conflict(id)
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
            .get_identity_conflict(id)
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
            .resolve_identity_conflict(id, action, notes.as_deref(), Utc::now())
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))
    }

    async fn dismiss(&self, id: i64, user_id: UserId) -> Result<(), ConflictError> {
        let conflict = self
            .db
            .get_identity_conflict(id)
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
            .dismiss_identity_conflict(id, Utc::now())
            .await
            .map_err(|e| ConflictError::Db(e.to_string()))
    }
}
