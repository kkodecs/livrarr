use crate::sqlite::SqliteDb;
use crate::{DbError, MetadataCacheDb, MetadataCacheRow};

use chrono::Utc;

use crate::sqlite_common::{map_db_err, parse_dt};

fn to_str<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .expect("enum serialization is infallible")
        .as_str()
        .expect("enum serializes to string")
        .to_string()
}

impl MetadataCacheDb for SqliteDb {
    async fn metadata_cache_get(
        &self,
        work_id: livrarr_domain::WorkId,
        provider: livrarr_domain::MetadataProvider,
        max_age: std::time::Duration,
    ) -> Result<Option<MetadataCacheRow>, DbError> {
        let provider_str = to_str(provider);
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT payload_json, fetched_at FROM metadata_cache \
             WHERE work_id = ? AND provider = ?",
        )
        .bind(work_id)
        .bind(&provider_str)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?;

        let Some((payload_json, fetched_at_str)) = row else {
            return Ok(None);
        };
        let fetched_at = parse_dt(&fetched_at_str)?;
        let fresh = match chrono::Duration::from_std(max_age) {
            Ok(max) => Utc::now().signed_duration_since(fetched_at) <= max,
            Err(_) => true,
        };
        if fresh {
            Ok(Some(MetadataCacheRow {
                payload_json,
                fetched_at,
            }))
        } else {
            Ok(None)
        }
    }

    async fn metadata_cache_put(
        &self,
        work_id: livrarr_domain::WorkId,
        provider: livrarr_domain::MetadataProvider,
        payload_json: &str,
    ) -> Result<(), DbError> {
        let provider_str = to_str(provider);
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO metadata_cache (work_id, provider, fetched_at, payload_json) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(work_id, provider) DO UPDATE SET \
             fetched_at = excluded.fetched_at, \
             payload_json = excluded.payload_json",
        )
        .bind(work_id)
        .bind(&provider_str)
        .bind(&now)
        .bind(payload_json)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;
        Ok(())
    }
}
