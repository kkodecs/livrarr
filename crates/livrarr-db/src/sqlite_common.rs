//! Shared helpers for SqliteDb trait implementations.

use chrono::{DateTime, Utc};

use crate::DbError;

/// Map sqlx errors to DbError.
pub fn map_db_err(e: sqlx::Error) -> DbError {
    match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        sqlx::Error::Database(ref db_err)
            if db_err.is_unique_violation() || db_err.is_foreign_key_violation() =>
        {
            DbError::Constraint {
                message: db_err.message().to_string(),
            }
        }
        other => DbError::Io(other.to_string()),
    }
}

/// Parse datetime from either RFC3339 or SQLite's native format.
pub fn parse_dt(s: &str) -> Result<DateTime<Utc>, DbError> {
    // Try RFC3339 first (our canonical format).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Fallback: SQLite datetime('now') format variants.
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ") {
        return Ok(naive.and_utc());
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(naive.and_utc());
    }
    Err(DbError::Io(format!("invalid datetime: {s}")))
}
