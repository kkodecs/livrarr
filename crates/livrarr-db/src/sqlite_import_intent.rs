use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{from_str, map_db_err, parse_dt, to_str};
use crate::{
    CreateImportIntentDbRequest, DbError, ImportIntent, ImportIntentDb, ImportIntentState,
};

fn row_to_import_intent(row: sqlx::sqlite::SqliteRow) -> Result<ImportIntent, DbError> {
    let media_type_str: String = row
        .try_get("media_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let state_str: String = row.try_get("state").map_err(|e| DbError::Io(Box::new(e)))?;
    let created_at_str: String = row
        .try_get("created_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(ImportIntent {
        id: row.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get::<i64, _>("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        root_folder_id: row
            .try_get::<i64, _>("root_folder_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        media_type: from_str(&media_type_str)?,
        target_relative: row
            .try_get("target_relative")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        staging_path: row
            .try_get("staging_path")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        expected_size: row
            .try_get::<i64, _>("expected_size")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        import_id: row
            .try_get("import_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        state: from_str(&state_str)?,
        created_at: parse_dt(&created_at_str)?,
    })
}

impl ImportIntentDb for SqliteDb {
    async fn create_import_intent(
        &self,
        req: CreateImportIntentDbRequest,
    ) -> Result<ImportIntent, DbError> {
        let now = Utc::now().to_rfc3339();
        let media_type_str = to_str(req.media_type);
        let state_str = to_str(ImportIntentState::Staging);

        let id = sqlx::query(
            "INSERT INTO import_intents \
             (user_id, work_id, root_folder_id, media_type, target_relative, \
              staging_path, expected_size, import_id, state, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(req.user_id)
        .bind(req.work_id)
        .bind(req.root_folder_id)
        .bind(&media_type_str)
        .bind(&req.target_relative)
        .bind(&req.staging_path)
        .bind(req.expected_size)
        .bind(&req.import_id)
        .bind(&state_str)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        let row = sqlx::query("SELECT * FROM import_intents WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_import_intent(row)
    }

    async fn mark_import_intent_renamed(&self, id: i64) -> Result<(), DbError> {
        let state_str = to_str(ImportIntentState::Renamed);
        sqlx::query("UPDATE import_intents SET state = ? WHERE id = ?")
            .bind(&state_str)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_import_intents(&self) -> Result<Vec<ImportIntent>, DbError> {
        let rows = sqlx::query("SELECT * FROM import_intents ORDER BY id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_import_intent).collect()
    }

    async fn delete_import_intent(&self, id: i64) -> Result<(), DbError> {
        sqlx::query("DELETE FROM import_intents WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }
}
