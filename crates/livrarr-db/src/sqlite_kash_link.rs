use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::map_db_err;
use crate::{DbError, KashLink, KashLinkDb, LibraryItemId, NewKashLink};
use livrarr_domain::kash::DURATION_TOLERANCE_SECS;

fn row_to_kash_link(row: sqlx::sqlite::SqliteRow) -> Result<KashLink, DbError> {
    Ok(KashLink {
        id: row
            .try_get::<i64, _>("id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        audio_item_id: row
            .try_get::<i64, _>("audio_item_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        ebook_item_id: row
            .try_get::<i64, _>("ebook_item_id")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        container_duration_secs: row
            .try_get::<f64, _>("container_duration_secs")
            .map_err(|e| DbError::Io(Box::new(e)))?,
        epub_hash: row
            .try_get("epub_hash")
            .map_err(|e| DbError::Io(Box::new(e)))?,
    })
}

impl KashLinkDb for SqliteDb {
    async fn upsert_link(&self, link: NewKashLink) -> Result<KashLink, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        // Check for an existing link keyed by this audio item.
        let existing = sqlx::query(
            "SELECT id, audio_item_id, ebook_item_id, container_duration_secs, epub_hash
             FROM kash_links
             WHERE audio_item_id = ?",
        )
        .bind(link.audio_item_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_db_err)?;

        let result = if let Some(row) = existing {
            let existing_link = row_to_kash_link(row)?;

            // Determine whether the link identity has changed.
            let duration_drift =
                (existing_link.container_duration_secs - link.container_duration_secs).abs();
            let identity_changed = existing_link.ebook_item_id != link.ebook_item_id
                || existing_link.epub_hash != link.epub_hash
                || duration_drift > DURATION_TOLERANCE_SECS;

            if identity_changed {
                // Wipe per-user furthest marks: an old mark on a different
                // alignment/timeline must never be reinterpreted (IR v2 R-001).
                sqlx::query("DELETE FROM cross_format_state WHERE kash_link_id = ?")
                    .bind(existing_link.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_db_err)?;
            }

            // Update the link row; the UNIQUE(ebook_item_id) constraint surfaces
            // as DbError::Constraint when that ebook belongs to another link.
            sqlx::query(
                "UPDATE kash_links
                 SET ebook_item_id = ?,
                     container_duration_secs = ?,
                     epub_hash = ?
                 WHERE id = ?",
            )
            .bind(link.ebook_item_id)
            .bind(link.container_duration_secs)
            .bind(&link.epub_hash)
            .bind(existing_link.id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

            KashLink {
                id: existing_link.id,
                audio_item_id: link.audio_item_id,
                ebook_item_id: link.ebook_item_id,
                container_duration_secs: link.container_duration_secs,
                epub_hash: link.epub_hash,
            }
        } else {
            // Fresh insert: the UNIQUE(ebook_item_id) constraint surfaces as
            // DbError::Constraint when that ebook is already linked to another
            // audio item (first-link-wins — caller logs and continues).
            let inserted_id = sqlx::query(
                "INSERT INTO kash_links
                     (audio_item_id, ebook_item_id, container_duration_secs, epub_hash,
                      created_at)
                 VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            )
            .bind(link.audio_item_id)
            .bind(link.ebook_item_id)
            .bind(link.container_duration_secs)
            .bind(&link.epub_hash)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?
            .last_insert_rowid();

            KashLink {
                id: inserted_id,
                audio_item_id: link.audio_item_id,
                ebook_item_id: link.ebook_item_id,
                container_duration_secs: link.container_duration_secs,
                epub_hash: link.epub_hash,
            }
        };

        tx.commit().await.map_err(map_db_err)?;
        Ok(result)
    }

    async fn link_for_item(
        &self,
        library_item_id: LibraryItemId,
    ) -> Result<Option<KashLink>, DbError> {
        // UNIQUE on both sides guarantees at most one row matches.
        let row = sqlx::query(
            "SELECT id, audio_item_id, ebook_item_id, container_duration_secs, epub_hash
             FROM kash_links
             WHERE audio_item_id = ? OR ebook_item_id = ?
             LIMIT 1",
        )
        .bind(library_item_id)
        .bind(library_item_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(Some(row_to_kash_link(r)?)),
            None => Ok(None),
        }
    }

    async fn delete_link_for_audio(&self, audio_item_id: LibraryItemId) -> Result<(), DbError> {
        // cross_format_state rows cascade via ON DELETE CASCADE on kash_links.
        // Idempotent when no row exists.
        sqlx::query("DELETE FROM kash_links WHERE audio_item_id = ?")
            .bind(audio_item_id)
            .execute(self.pool())
            .await
            .map_err(map_db_err)?;
        Ok(())
    }
}
