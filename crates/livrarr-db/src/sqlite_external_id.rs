use async_trait::async_trait;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, ExternalId, ExternalIdRowId, UpsertExternalIdRequest, UserId, WorkId};

// ---------------------------------------------------------------------------
// Enum ↔ string helpers
// ---------------------------------------------------------------------------

fn to_str<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .expect("enum serialization is infallible")
        .as_str()
        .expect("enum serializes to string")
        .to_string()
}

fn from_str<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, DbError> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| {
        DbError::IncompatibleData {
            detail: e.to_string(),
        }
    })
}

// ---------------------------------------------------------------------------
// Row → ExternalId
// ---------------------------------------------------------------------------

fn row_to_external_id(
    row: sqlx::sqlite::SqliteRow,
    user_id: UserId,
) -> Result<ExternalId, DbError> {
    let id_type_str: String = row
        .try_get("id_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    Ok(ExternalId {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))? as ExternalIdRowId,
        user_id,
        work_id: row
            .try_get("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        id_type: from_str(&id_type_str)?,
        id_value: row
            .try_get("id_value")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

// ---------------------------------------------------------------------------
// ExternalIdDb impl
// ---------------------------------------------------------------------------

#[async_trait]
impl crate::ExternalIdDb for SqliteDb {
    async fn upsert_external_id(
        &self,
        user_id: UserId,
        req: UpsertExternalIdRequest,
    ) -> Result<(), DbError> {
        // Verify work belongs to user (FK validation + ownership check).
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM works WHERE id = ? AND user_id = ?")
                .bind(req.work_id)
                .bind(user_id)
                .fetch_optional(self.pool())
                .await
                .map_err(map_db_err)?;

        if exists.is_none() {
            return Err(DbError::NotFound { entity: "work" });
        }

        let id_type_str = to_str(req.id_type);
        sqlx::query(
            "INSERT INTO external_ids (work_id, id_type, id_value) \
             VALUES (?, ?, ?) \
             ON CONFLICT(work_id, id_type, id_value) DO NOTHING",
        )
        .bind(req.work_id)
        .bind(&id_type_str)
        .bind(&req.id_value)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn upsert_external_ids_batch(
        &self,
        user_id: UserId,
        reqs: Vec<UpsertExternalIdRequest>,
    ) -> Result<(), DbError> {
        if reqs.is_empty() {
            return Ok(());
        }

        // Collect unique work IDs and verify all belong to user.
        let mut work_ids: Vec<WorkId> = reqs.iter().map(|r| r.work_id).collect();
        work_ids.sort_unstable();
        work_ids.dedup();

        for work_id in &work_ids {
            let exists: Option<i64> =
                sqlx::query_scalar("SELECT id FROM works WHERE id = ? AND user_id = ?")
                    .bind(*work_id)
                    .bind(user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(map_db_err)?;

            if exists.is_none() {
                return Err(DbError::NotFound { entity: "work" });
            }
        }

        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        for req in reqs {
            let id_type_str = to_str(req.id_type);
            sqlx::query(
                "INSERT INTO external_ids (work_id, id_type, id_value) \
                 VALUES (?, ?, ?) \
                 ON CONFLICT(work_id, id_type, id_value) DO NOTHING",
            )
            .bind(req.work_id)
            .bind(&id_type_str)
            .bind(&req.id_value)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn list_external_ids(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ExternalId>, DbError> {
        let rows = sqlx::query(
            "SELECT e.id, e.work_id, e.id_type, e.id_value \
             FROM external_ids e \
             JOIN works w ON e.work_id = w.id \
             WHERE e.work_id = ? AND w.user_id = ?",
        )
        .bind(work_id)
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter()
            .map(|row| row_to_external_id(row, user_id))
            .collect()
    }
}
