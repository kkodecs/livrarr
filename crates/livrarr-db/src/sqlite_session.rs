use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{DbError, Session, SessionDb};

fn row_to_session(row: sqlx::sqlite::SqliteRow) -> Result<Session, DbError> {
    Ok(Session {
        token_hash: row
            .try_get("token_hash")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        persistent: row
            .try_get::<bool, _>("persistent")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        created_at: parse_dt(
            &row.try_get::<String, _>("created_at")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        )?,
        expires_at: parse_dt(
            &row.try_get::<String, _>("expires_at")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        )?,
    })
}

impl SessionDb for SqliteDb {
    async fn get_session(&self, token_hash: &str) -> Result<Option<Session>, DbError> {
        let now = Utc::now().to_rfc3339();
        let row = sqlx::query("SELECT * FROM sessions WHERE token_hash = ? AND expires_at > ?")
            .bind(token_hash)
            .bind(&now)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(Some(row_to_session(r)?)),
            None => Ok(None),
        }
    }

    async fn create_session(&self, session: &Session) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, persistent, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&session.token_hash)
        .bind(session.user_id)
        .bind(session.persistent)
        .bind(session.created_at.to_rfc3339())
        .bind(session.expires_at.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn delete_session(&self, token_hash: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn extend_session(
        &self,
        token_hash: &str,
        new_expires_at: DateTime<Utc>,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE sessions SET expires_at = ? WHERE token_hash = ?")
            .bind(new_expires_at.to_rfc3339())
            .bind(token_hash)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn delete_expired_sessions(&self) -> Result<u64, DbError> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(&now)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(result.rows_affected())
    }
}
