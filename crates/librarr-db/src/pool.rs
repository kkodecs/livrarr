use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

/// Create and configure a SQLite connection pool.
///
/// Satisfies: RUNTIME-SQLITE-001, RUNTIME-SQLITE-002
///
/// - Max 4 connections, WAL journal mode, busy timeout 30s, foreign_keys ON.
/// - Database file created automatically on first boot.
pub async fn create_sqlite_pool(data_dir: &Path) -> Result<SqlitePool, sqlx::Error> {
    let db_path = data_dir.join("librarr.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());

    let options = SqliteConnectOptions::from_str(&url)?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(30))
        .pragma("foreign_keys", "ON");

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .min_connections(1)
        .connect_with(options)
        .await?;

    Ok(pool)
}

/// Run embedded migrations.
///
/// Satisfies: RUNTIME-SQLITE-003
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
