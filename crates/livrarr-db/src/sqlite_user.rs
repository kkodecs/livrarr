use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::{
    CompleteSetupDbRequest, CreateUserDbRequest, DbError, UpdateUserDbRequest, User, UserDb,
    UserId, UserRole,
};

fn row_to_user(row: sqlx::sqlite::SqliteRow) -> Result<User, DbError> {
    Ok(User {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        username: row
            .try_get("username")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        password_hash: row
            .try_get("password_hash")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        role: parse_role(
            row.try_get::<String, _>("role")
                .map_err(|e| DbError::Io(Box::new(e)))?
                .as_str(),
        )?,
        api_key_hash: row
            .try_get("api_key_hash")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        setup_pending: row
            .try_get::<bool, _>("setup_pending")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        created_at: parse_dt(
            &row.try_get::<String, _>("created_at")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        )?,
        updated_at: parse_dt(
            &row.try_get::<String, _>("updated_at")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        )?,
    })
}

fn parse_role(s: &str) -> Result<UserRole, DbError> {
    match s {
        "admin" => Ok(UserRole::Admin),
        "user" => Ok(UserRole::User),
        _ => Err(DbError::IncompatibleData {
            detail: format!("unknown user role: {s}"),
        }),
    }
}

impl UserDb for SqliteDb {
    async fn get_user(&self, id: UserId) -> Result<User, DbError> {
        let row = sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_user(row)
    }

    async fn get_user_by_username(&self, username: &str) -> Result<User, DbError> {
        let row = sqlx::query("SELECT * FROM users WHERE LOWER(username) = LOWER(?)")
            .bind(username)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_user(row)
    }

    async fn get_user_by_api_key_hash(&self, hash: &str) -> Result<User, DbError> {
        let row = sqlx::query("SELECT * FROM users WHERE api_key_hash = ?")
            .bind(hash)
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row_to_user(row)
    }

    async fn list_users(&self) -> Result<Vec<User>, DbError> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY id")
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_user).collect()
    }

    async fn create_user(&self, req: CreateUserDbRequest) -> Result<User, DbError> {
        let now = Utc::now().to_rfc3339();
        let role_str = match req.role {
            UserRole::Admin => "admin",
            UserRole::User => "user",
        };

        let id = sqlx::query(
            "INSERT INTO users (username, password_hash, role, api_key_hash, setup_pending, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&req.username)
        .bind(&req.password_hash)
        .bind(role_str)
        .bind(&req.api_key_hash)
        .bind(&now)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        self.get_user(id).await
    }

    async fn update_user(&self, id: UserId, req: UpdateUserDbRequest) -> Result<User, DbError> {
        let current = self.get_user(id).await?;
        let now = Utc::now().to_rfc3339();

        let username = req.username.unwrap_or(current.username);
        let password_hash = req.password_hash.unwrap_or(current.password_hash);
        let role = req.role.unwrap_or(current.role);
        let role_str = match role {
            UserRole::Admin => "admin",
            UserRole::User => "user",
        };

        sqlx::query(
            "UPDATE users SET username = ?, password_hash = ?, role = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&username)
        .bind(&password_hash)
        .bind(role_str)
        .bind(&now)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_user(id).await
    }

    async fn delete_user(&self, id: UserId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "user" });
        }
        Ok(())
    }

    async fn count_admins(&self) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM users WHERE role = 'admin'")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row.try_get::<i64, _>("cnt")
            .map_err(|e| DbError::Io(Box::new(e)))
    }

    async fn complete_setup(&self, req: CompleteSetupDbRequest) -> Result<User, DbError> {
        let now = Utc::now().to_rfc3339();

        // Find the pending setup user (not hardcoded to id=1).
        let row =
            sqlx::query("SELECT id FROM users WHERE setup_pending = 1 ORDER BY id ASC LIMIT 1")
                .fetch_optional(self.pool())
                .await
                .map_err(map_db_err)?;

        let user_id: i64 = match row {
            Some(r) => r.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?,
            None => {
                return Err(DbError::Constraint {
                    message: "setup already completed".to_string(),
                });
            }
        };

        sqlx::query(
            "UPDATE users SET username = ?, password_hash = ?, api_key_hash = ?, \
             setup_pending = 0, updated_at = ? WHERE id = ? AND setup_pending = 1",
        )
        .bind(&req.username)
        .bind(&req.password_hash)
        .bind(&req.api_key_hash)
        .bind(&now)
        .bind(user_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_user(user_id).await
    }

    async fn update_api_key_hash(&self, user_id: UserId, hash: &str) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query("UPDATE users SET api_key_hash = ?, updated_at = ? WHERE id = ?")
            .bind(hash)
            .bind(&now)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "user" });
        }
        Ok(())
    }
}
