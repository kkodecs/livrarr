use async_trait::async_trait;
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
            .map_err(|e| DbError::Io(e.to_string()))?,
        username: row
            .try_get("username")
            .map_err(|e| DbError::Io(e.to_string()))?,
        password_hash: row
            .try_get("password_hash")
            .map_err(|e| DbError::Io(e.to_string()))?,
        role: parse_role(
            row.try_get::<String, _>("role")
                .map_err(|e| DbError::Io(e.to_string()))?
                .as_str(),
        ),
        api_key_hash: row
            .try_get("api_key_hash")
            .map_err(|e| DbError::Io(e.to_string()))?,
        setup_pending: row
            .try_get::<bool, _>("setup_pending")
            .map_err(|e| DbError::Io(e.to_string()))?,
        created_at: parse_dt(
            &row.try_get::<String, _>("created_at")
                .map_err(|e| DbError::Io(e.to_string()))?,
        )?,
        updated_at: parse_dt(
            &row.try_get::<String, _>("updated_at")
                .map_err(|e| DbError::Io(e.to_string()))?,
        )?,
    })
}

fn parse_role(s: &str) -> UserRole {
    match s {
        "admin" => UserRole::Admin,
        _ => UserRole::User,
    }
}

#[async_trait]
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
        // Verify user exists first.
        self.get_user(id).await?;

        let now = Utc::now().to_rfc3339();

        if let Some(username) = &req.username {
            sqlx::query("UPDATE users SET username = ?, updated_at = ? WHERE id = ?")
                .bind(username)
                .bind(&now)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(password_hash) = &req.password_hash {
            sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
                .bind(password_hash)
                .bind(&now)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }
        if let Some(role) = &req.role {
            let role_str = match role {
                UserRole::Admin => "admin",
                UserRole::User => "user",
            };
            sqlx::query("UPDATE users SET role = ?, updated_at = ? WHERE id = ?")
                .bind(role_str)
                .bind(&now)
                .bind(id)
                .execute(self.pool())
                .await
                .map_err(map_db_err)?;
        }

        self.get_user(id).await
    }

    async fn delete_user(&self, id: UserId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    async fn count_admins(&self) -> Result<i64, DbError> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM users WHERE role = 'admin'")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)?;
        row.try_get::<i64, _>("cnt")
            .map_err(|e| DbError::Io(e.to_string()))
    }

    async fn complete_setup(&self, req: CompleteSetupDbRequest) -> Result<User, DbError> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE users SET username = ?, password_hash = ?, api_key_hash = ?, \
             setup_pending = 0, updated_at = ? WHERE id = 1 AND setup_pending = 1",
        )
        .bind(&req.username)
        .bind(&req.password_hash)
        .bind(&req.api_key_hash)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::Constraint {
                message: "setup already completed".to_string(),
            });
        }

        self.get_user(1).await
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
            return Err(DbError::NotFound);
        }
        Ok(())
    }
}
