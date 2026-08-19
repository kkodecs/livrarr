use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{from_str, map_db_err, parse_dt, to_str};
use crate::{DbError, MetadataProvider, ProviderCacheEntry};

fn row_to_cache_entry(row: sqlx::sqlite::SqliteRow) -> Result<ProviderCacheEntry, DbError> {
    let provider_str: String = row
        .try_get("provider")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let anchor_type: String = row
        .try_get("anchor_type")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let anchor: String = row
        .try_get("anchor")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let payload_json: String = row
        .try_get("payload")
        .map_err(|e| DbError::Io(Box::new(e)))?;
    let fetched_at_str: String = row
        .try_get("fetched_at")
        .map_err(|e| DbError::Io(Box::new(e)))?;

    Ok(ProviderCacheEntry {
        provider: from_str(&provider_str)?,
        anchor_type,
        anchor,
        payload_json,
        fetched_at: parse_dt(&fetched_at_str)?,
    })
}

impl crate::ProviderResponseCacheDb for SqliteDb {
    async fn get_provider_cache_entry(
        &self,
        provider: MetadataProvider,
        anchor_type: &str,
        anchor: &str,
    ) -> Result<Option<ProviderCacheEntry>, DbError> {
        let provider_str = to_str(provider);
        let row = sqlx::query(
            "SELECT provider, anchor_type, anchor, payload, fetched_at \
             FROM provider_response_cache \
             WHERE provider = ? AND anchor_type = ? AND anchor = ?",
        )
        .bind(&provider_str)
        .bind(anchor_type)
        .bind(anchor)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        row.map(row_to_cache_entry).transpose()
    }

    async fn upsert_provider_cache_entry(&self, entry: ProviderCacheEntry) -> Result<(), DbError> {
        let provider_str = to_str(entry.provider);
        let fetched_at_str = entry.fetched_at.to_rfc3339();

        sqlx::query(
            "INSERT INTO provider_response_cache \
             (provider, anchor_type, anchor, payload, fetched_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (provider, anchor_type, anchor) DO UPDATE SET \
             payload = excluded.payload, \
             fetched_at = excluded.fetched_at",
        )
        .bind(&provider_str)
        .bind(&entry.anchor_type)
        .bind(&entry.anchor)
        .bind(&entry.payload_json)
        .bind(&fetched_at_str)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        Ok(())
    }

    async fn evict_provider_cache_to_cap(&self, max_rows: i64) -> Result<u64, DbError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(map_db_err)?;

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_response_cache")
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;
        let excess = remaining - max_rows;
        let evicted = if excess > 0 {
            sqlx::query(
                "DELETE FROM provider_response_cache WHERE rowid IN (\
                 SELECT rowid FROM provider_response_cache \
                 ORDER BY fetched_at ASC, rowid ASC LIMIT ?)",
            )
            .bind(excess)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?
            .rows_affected()
        } else {
            0
        };

        tx.commit().await.map_err(map_db_err)?;
        Ok(evicted)
    }

    async fn count_provider_cache_entries(&self) -> Result<i64, DbError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_response_cache")
            .fetch_one(self.pool())
            .await
            .map_err(map_db_err)
    }
}
