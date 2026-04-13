use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{
    AuthorId, CreateSeriesDbRequest, DbError, LinkWorkToSeriesRequest, Series, SeriesDb, UserId,
};

fn row_to_series(row: sqlx::sqlite::SqliteRow) -> Result<Series, DbError> {
    let added_at_str: String = row
        .try_get("added_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    Ok(Series {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        author_id: row
            .try_get::<i64, _>("author_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        name: row.try_get("name").map_err(|e| DbError::Io(Box::new(e)))?,
        gr_key: row
            .try_get("gr_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_ebook: row
            .try_get::<bool, _>("monitor_ebook")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        monitor_audiobook: row
            .try_get::<bool, _>("monitor_audiobook")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_count: row
            .try_get::<i32, _>("work_count")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        added_at: crate::sqlite_common::parse_dt(&added_at_str)?,
    })
}

impl SeriesDb for SqliteDb {
    async fn list_all_series(&self, user_id: UserId) -> Result<Vec<Series>, DbError> {
        let rows = sqlx::query("SELECT * FROM series WHERE user_id = ? ORDER BY name")
            .bind(user_id)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
        rows.into_iter().map(row_to_series).collect()
    }

    async fn get_series(&self, id: i64) -> Result<Option<Series>, DbError> {
        let row = sqlx::query("SELECT * FROM series WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(map_db_err)?;
        row.map(row_to_series).transpose()
    }

    async fn list_series_for_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<Series>, DbError> {
        let rows =
            sqlx::query("SELECT * FROM series WHERE user_id = ? AND author_id = ? ORDER BY name")
                .bind(user_id)
                .bind(author_id)
                .fetch_all(self.pool())
                .await
                .map_err(map_db_err)?;
        rows.into_iter().map(row_to_series).collect()
    }

    async fn upsert_series(&self, req: CreateSeriesDbRequest) -> Result<Series, DbError> {
        let now = Utc::now().to_rfc3339();
        let id = sqlx::query(
            "INSERT INTO series (user_id, author_id, name, gr_key, monitor_ebook, monitor_audiobook, work_count, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id, author_id, gr_key) DO UPDATE SET \
             monitor_ebook = excluded.monitor_ebook, \
             monitor_audiobook = excluded.monitor_audiobook, \
             name = excluded.name, \
             work_count = excluded.work_count \
             RETURNING id",
        )
        .bind(req.user_id)
        .bind(req.author_id)
        .bind(&req.name)
        .bind(&req.gr_key)
        .bind(req.monitor_ebook)
        .bind(req.monitor_audiobook)
        .bind(req.work_count)
        .bind(&now)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?
        .try_get::<i64, _>("id")
        .map_err(|e| DbError::Io(Box::new(e)))?;

        self.get_series(id)
            .await?
            .ok_or(DbError::NotFound { entity: "series" })
    }

    async fn update_series_flags(
        &self,
        id: i64,
        monitor_ebook: bool,
        monitor_audiobook: bool,
    ) -> Result<Series, DbError> {
        sqlx::query("UPDATE series SET monitor_ebook = ?, monitor_audiobook = ? WHERE id = ?")
            .bind(monitor_ebook)
            .bind(monitor_audiobook)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        // Propagate flags to linked works.
        sqlx::query(
            "UPDATE works SET monitor_ebook = ?, monitor_audiobook = ? WHERE series_id = ?",
        )
        .bind(monitor_ebook)
        .bind(monitor_audiobook)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        self.get_series(id)
            .await?
            .ok_or(DbError::NotFound { entity: "series" })
    }

    async fn update_series_work_count(&self, id: i64, work_count: i32) -> Result<(), DbError> {
        sqlx::query("UPDATE series SET work_count = ? WHERE id = ?")
            .bind(work_count)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn link_work_to_series(&self, req: LinkWorkToSeriesRequest) -> Result<(), DbError> {
        // Assignment guard: only update if current series_id is NULL or new series
        // has fewer books (more specific).
        sqlx::query(
            "UPDATE works SET \
             series_id = ?, series_name = ?, series_position = ?, \
             monitor_ebook = ?, monitor_audiobook = ? \
             WHERE id = ? AND (\
               series_id IS NULL \
               OR (SELECT work_count FROM series WHERE id = works.series_id) > ? \
             )",
        )
        .bind(req.series_id)
        .bind(&req.series_name)
        .bind(req.series_position)
        .bind(req.monitor_ebook)
        .bind(req.monitor_audiobook)
        .bind(req.work_id)
        .bind(req.series_work_count)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_monitored_series_for_authors(
        &self,
        author_ids: &[AuthorId],
    ) -> Result<Vec<Series>, DbError> {
        if author_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<&str> = author_ids.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT * FROM series WHERE author_id IN ({}) AND (monitor_ebook = 1 OR monitor_audiobook = 1)",
            placeholders.join(", ")
        );
        let mut query = sqlx::query(&sql);
        for id in author_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(self.pool()).await.map_err(map_db_err)?;
        rows.into_iter().map(row_to_series).collect()
    }
}
