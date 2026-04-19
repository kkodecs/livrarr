use async_trait::async_trait;
use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    DbError, FieldProvenance, MetadataProvider, ProvenanceSetter, SetFieldProvenanceRequest,
    UserId, WorkField, WorkId,
};

// ---------------------------------------------------------------------------
// Enum ↔ string helpers (leverages serde rename_all = "snake_case")
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
// Invariant validation
// ---------------------------------------------------------------------------

fn validate_req(req: &SetFieldProvenanceRequest) -> Result<(), DbError> {
    match req.setter {
        ProvenanceSetter::Provider => {
            if req.source.is_none() {
                return Err(DbError::Constraint {
                    message: "provider setter requires a non-null source".to_string(),
                });
            }
            if req.cleared {
                return Err(DbError::Constraint {
                    message: "provider setter cannot have cleared=true".to_string(),
                });
            }
        }
        ProvenanceSetter::User
        | ProvenanceSetter::System
        | ProvenanceSetter::AutoAdded
        | ProvenanceSetter::Imported => {
            if req.source.is_some() {
                return Err(DbError::Constraint {
                    message: "user/system/auto_added setter must not have a source".to_string(),
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Row → FieldProvenance
// ---------------------------------------------------------------------------

fn row_to_provenance(row: sqlx::sqlite::SqliteRow) -> Result<FieldProvenance, DbError> {
    let field_str: String = row.try_get("field").map_err(|e| DbError::Io(Box::new(e)))?;
    let setter_str: String = row
        .try_get("setter")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let source_str: Option<String> = row
        .try_get("source")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let set_at_str: String = row
        .try_get("set_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let cleared_int: i64 = row
        .try_get("cleared")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    let field: WorkField = from_str(&field_str)?;
    let setter: ProvenanceSetter = from_str(&setter_str)?;
    let source: Option<MetadataProvider> = source_str.as_deref().map(from_str).transpose()?;

    Ok(FieldProvenance {
        user_id: row
            .try_get("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        field,
        source,
        set_at: parse_dt(&set_at_str)?,
        setter,
        cleared: cleared_int != 0,
    })
}

// ---------------------------------------------------------------------------
// ProvenanceDb impl
// ---------------------------------------------------------------------------

#[async_trait]
impl crate::ProvenanceDb for SqliteDb {
    async fn set_field_provenance(&self, req: SetFieldProvenanceRequest) -> Result<(), DbError> {
        validate_req(&req)?;
        let now = Utc::now().to_rfc3339();
        let field_str = to_str(req.field);
        let setter_str = to_str(req.setter);
        let source_str = req.source.map(to_str);

        sqlx::query(
            "INSERT INTO work_metadata_provenance \
             (user_id, work_id, field, source, set_at, setter, cleared) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(work_id, field) DO UPDATE SET \
             user_id = excluded.user_id, \
             source = excluded.source, \
             set_at = excluded.set_at, \
             setter = excluded.setter, \
             cleared = excluded.cleared",
        )
        .bind(req.user_id)
        .bind(req.work_id)
        .bind(&field_str)
        .bind(&source_str)
        .bind(&now)
        .bind(&setter_str)
        .bind(req.cleared as i64)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn set_field_provenance_batch(
        &self,
        reqs: Vec<SetFieldProvenanceRequest>,
    ) -> Result<(), DbError> {
        if reqs.is_empty() {
            return Ok(());
        }

        // Validate all upfront before opening a transaction.
        for req in &reqs {
            validate_req(req)?;
        }

        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        for req in reqs {
            let field_str = to_str(req.field);
            let setter_str = to_str(req.setter);
            let source_str = req.source.map(to_str);

            sqlx::query(
                "INSERT INTO work_metadata_provenance \
                 (user_id, work_id, field, source, set_at, setter, cleared) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(work_id, field) DO UPDATE SET \
                 user_id = excluded.user_id, \
                 source = excluded.source, \
                 set_at = excluded.set_at, \
                 setter = excluded.setter, \
                 cleared = excluded.cleared",
            )
            .bind(req.user_id)
            .bind(req.work_id)
            .bind(&field_str)
            .bind(&source_str)
            .bind(&now)
            .bind(&setter_str)
            .bind(req.cleared as i64)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn get_field_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
        field: WorkField,
    ) -> Result<Option<FieldProvenance>, DbError> {
        let field_str = to_str(field);
        let row = sqlx::query(
            "SELECT p.user_id, p.work_id, p.field, p.source, p.set_at, p.setter, p.cleared \
             FROM work_metadata_provenance p \
             JOIN works w ON p.work_id = w.id \
             WHERE p.work_id = ? AND p.field = ? AND w.user_id = ?",
        )
        .bind(work_id)
        .bind(&field_str)
        .bind(user_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        row.map(row_to_provenance).transpose()
    }

    async fn list_work_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<FieldProvenance>, DbError> {
        let rows = sqlx::query(
            "SELECT p.user_id, p.work_id, p.field, p.source, p.set_at, p.setter, p.cleared \
             FROM work_metadata_provenance p \
             JOIN works w ON p.work_id = w.id \
             WHERE p.work_id = ? AND w.user_id = ?",
        )
        .bind(work_id)
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter().map(row_to_provenance).collect()
    }

    async fn delete_field_provenance_batch(
        &self,
        user_id: UserId,
        work_id: WorkId,
        fields: Vec<WorkField>,
    ) -> Result<(), DbError> {
        if fields.is_empty() {
            return Ok(());
        }

        // Verify work ownership.
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM works WHERE id = ? AND user_id = ?")
                .bind(work_id)
                .bind(user_id)
                .fetch_optional(self.pool())
                .await
                .map_err(map_db_err)?;

        if exists.is_none() {
            return Err(DbError::NotFound { entity: "work" });
        }

        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        for field in fields {
            let field_str = to_str(field);
            sqlx::query("DELETE FROM work_metadata_provenance WHERE work_id = ? AND field = ?")
                .bind(work_id)
                .bind(&field_str)
                .execute(&mut *tx)
                .await
                .map_err(map_db_err)?;
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn clear_work_provenance(&self, user_id: UserId, work_id: WorkId) -> Result<(), DbError> {
        // Verify work ownership, then delete all rows.
        sqlx::query(
            "DELETE FROM work_metadata_provenance \
             WHERE work_id = ? \
             AND EXISTS (SELECT 1 FROM works WHERE id = ? AND user_id = ?)",
        )
        .bind(work_id)
        .bind(work_id)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }
}
