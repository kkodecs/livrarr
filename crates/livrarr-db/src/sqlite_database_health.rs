use livrarr_domain::services::DatabaseHealth;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::DbError;

impl DatabaseHealth for SqliteDb {
    async fn check_database(&self) -> Result<(), DbError> {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sqlite_schema")
            .fetch_one(self.pool())
            .await
            .map(drop)
            .map_err(map_db_err)
    }
}
