//! Shared helpers for SqliteDb trait implementations.

use chrono::{DateTime, Utc};

use crate::DbError;

/// Map sqlx errors to DbError.
///
/// `entity` provides context for NotFound errors (e.g. "user", "work").
pub fn map_db_err_with(entity: &'static str) -> impl Fn(sqlx::Error) -> DbError {
    move |e: sqlx::Error| match e {
        sqlx::Error::RowNotFound => DbError::NotFound { entity },
        sqlx::Error::Database(ref db_err)
            if db_err.is_unique_violation() || db_err.is_foreign_key_violation() =>
        {
            DbError::Constraint {
                message: db_err.message().to_string(),
            }
        }
        other => DbError::Io(Box::new(other)),
    }
}

/// Map sqlx errors to DbError with a generic entity context.
pub fn map_db_err(e: sqlx::Error) -> DbError {
    map_db_err_with("record")(e)
}

/// Parse datetime from either RFC3339 or SQLite's native format.
///
/// Intentionally loose about fractional seconds: RFC3339 parsing handles them
/// natively (`2024-01-01T00:00:00.123Z`), while the fallback patterns match
/// SQLite's `datetime('now')` output which never includes fractional seconds.
/// If SQLite triggers or user inserts produce fractional-second strings not
/// matching RFC3339, they will fail here — that's acceptable since all Livrarr
/// writes use RFC3339.
pub fn parse_dt(s: &str) -> Result<DateTime<Utc>, DbError> {
    // Try RFC3339 first (our canonical format — handles fractional seconds).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Fallback: SQLite datetime('now') format variants (no fractional seconds).
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ") {
        return Ok(naive.and_utc());
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(naive.and_utc());
    }
    Err(DbError::Io(format!("invalid datetime: {s}").into()))
}
