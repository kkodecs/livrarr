use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{AudiobookChapter, ChapterDb, DbError, LibraryItemId};

fn row_to_chapter(row: sqlx::sqlite::SqliteRow) -> Result<AudiobookChapter, DbError> {
    Ok(AudiobookChapter {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        library_item_id: row
            .try_get::<i64, _>("library_item_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        chapter_index: row
            .try_get::<i32, _>("chapter_index")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        title: row.try_get("title").map_err(|e| DbError::Io(Box::new(e)))?,
        start_time_secs: row
            .try_get("start_time_secs")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        end_time_secs: row
            .try_get("end_time_secs")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

impl ChapterDb for SqliteDb {
    async fn get_chapters(
        &self,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AudiobookChapter>, DbError> {
        let rows = sqlx::query(
            "SELECT id, library_item_id, chapter_index, title, start_time_secs, end_time_secs
             FROM audiobook_chapters
             WHERE library_item_id = ?
             ORDER BY start_time_secs ASC",
        )
        .bind(library_item_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter().map(row_to_chapter).collect()
    }

    async fn replace_chapters(
        &self,
        library_item_id: LibraryItemId,
        chapters: &[AudiobookChapter],
    ) -> Result<(), DbError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;

        sqlx::query("DELETE FROM audiobook_chapters WHERE library_item_id = ?")
            .bind(library_item_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

        for ch in chapters {
            sqlx::query(
                "INSERT INTO audiobook_chapters (library_item_id, chapter_index, title, start_time_secs, end_time_secs)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(library_item_id)
            .bind(ch.chapter_index)
            .bind(&ch.title)
            .bind(ch.start_time_secs)
            .bind(ch.end_time_secs)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn has_chapters(&self, library_item_id: LibraryItemId) -> Result<bool, DbError> {
        let row = sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM audiobook_chapters WHERE library_item_id = ?) AS has_ch",
        )
        .bind(library_item_id)
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(row.try_get::<bool, _>("has_ch").unwrap_or(false))
    }

    async fn list_unscanned_audiobook_items(
        &self,
    ) -> Result<Vec<(LibraryItemId, String)>, DbError> {
        let rows = sqlx::query(
            "SELECT li.id, rf.path || '/' || li.path AS full_path
             FROM library_items li
             JOIN root_folders rf ON li.root_folder_id = rf.id
             WHERE li.chapter_scan_status IS NULL AND li.media_type = 'audiobook'",
        )
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        rows.into_iter()
            .map(|r| {
                let id: i64 = r.try_get("id").map_err(|e| DbError::Io(Box::new(e)))?;
                let path: String = r
                    .try_get("full_path")
                    .map_err(|e| DbError::Io(Box::new(e)))?;
                Ok((id, path))
            })
            .collect()
    }

    async fn update_chapter_scan_result(
        &self,
        library_item_id: LibraryItemId,
        chapter_scan_status: &str,
        duration_seconds: Option<f64>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE library_items
             SET chapter_scan_status = ?, duration_seconds = COALESCE(?, duration_seconds)
             WHERE id = ?",
        )
        .bind(chapter_scan_status)
        .bind(duration_seconds)
        .bind(library_item_id)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }
}
