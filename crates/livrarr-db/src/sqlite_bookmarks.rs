use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{map_db_err, parse_dt, parse_media_type};
use crate::{Bookmark, BookmarkDb, DbError, LibraryItemId, MediaType, UserId};

fn media_type_str(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Ebook => "ebook",
        MediaType::Audiobook => "audiobook",
    }
}

fn row_to_bookmark(row: sqlx::sqlite::SqliteRow) -> Result<Bookmark, DbError> {
    let media_type_str: String = row
        .try_get("media_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    Ok(Bookmark {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        user_id: row
            .try_get::<i64, _>("user_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        work_id: row
            .try_get::<i64, _>("work_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        library_item_id: row
            .try_get::<i64, _>("library_item_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        media_type: parse_media_type(&media_type_str)?,
        position: row
            .try_get("position")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        sort_key: row
            .try_get("sort_key")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        name: row.try_get("name").map_err(|e| DbError::Io(Box::new(e)))?,
        chapter_title: row
            .try_get("chapter_title")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        paired_bookmark_id: row
            .try_get("paired_bookmark_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        created_at: parse_dt(
            &row.try_get::<String, _>("created_at")
                .map_err(|e| DbError::Io(Box::new(e)))?,
        )?,
    })
}

impl BookmarkDb for SqliteDb {
    async fn list_bookmarks(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<Bookmark>, DbError> {
        let rows = sqlx::query(
            "SELECT id, user_id, work_id, library_item_id, media_type, position, sort_key,
                    name, chapter_title, paired_bookmark_id, created_at
             FROM bookmarks
             WHERE user_id = ? AND library_item_id = ?
             ORDER BY sort_key ASC",
        )
        .bind(user_id)
        .bind(library_item_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter().map(row_to_bookmark).collect()
    }

    async fn create_bookmark(&self, bookmark: &Bookmark) -> Result<Bookmark, DbError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mt = media_type_str(bookmark.media_type);

        let id = sqlx::query(
            "INSERT INTO bookmarks (user_id, work_id, library_item_id, media_type, position,
                                    sort_key, name, chapter_title, paired_bookmark_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(bookmark.user_id)
        .bind(bookmark.work_id)
        .bind(bookmark.library_item_id)
        .bind(mt)
        .bind(&bookmark.position)
        .bind(bookmark.sort_key)
        .bind(&bookmark.name)
        .bind(&bookmark.chapter_title)
        .bind(bookmark.paired_bookmark_id)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?
        .last_insert_rowid();

        let row = sqlx::query(
            "SELECT id, user_id, work_id, library_item_id, media_type, position, sort_key,
                    name, chapter_title, paired_bookmark_id, created_at
             FROM bookmarks WHERE id = ? AND user_id = ?",
        )
        .bind(id)
        .bind(bookmark.user_id)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;

        row_to_bookmark(row)
    }

    async fn rename_bookmark(
        &self,
        user_id: UserId,
        bookmark_id: i64,
        name: &str,
    ) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE bookmarks SET name = ? WHERE id = ? AND user_id = ?")
            .bind(name)
            .bind(bookmark_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "bookmark" });
        }
        Ok(())
    }

    async fn delete_bookmark(&self, user_id: UserId, bookmark_id: i64) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM bookmarks WHERE id = ? AND user_id = ?")
            .bind(bookmark_id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "bookmark" });
        }
        Ok(())
    }
}
