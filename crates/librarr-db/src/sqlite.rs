use sqlx::SqlitePool;

/// Production database implementation backed by SQLite.
///
/// Satisfies: RUNTIME-SQLITE-005
///
/// Implements all Db traits against a sqlx::SqlitePool.
/// Trait implementations are added per-flow as the backend is built out.
#[derive(Clone)]
pub struct SqliteDb {
    pool: SqlitePool,
}

impl SqliteDb {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
