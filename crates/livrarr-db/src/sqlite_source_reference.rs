//! Created-branch persistence and reads for creation facts: the ordinary Work
//! columns a creation door supplied, their provenance rows, and the typed
//! source-reference rows. The writer runs inside the settlement transaction
//! so a committed Work never exists without the facts it was created with.

use chrono::Utc;
use sqlx::Row;

use crate::sqlite::SqliteDb;
use crate::sqlite_common::{absolute_http_cover_url, from_str, map_db_err, to_str};
use crate::{
    CreationFacts, DbError, MetadataProvider, ProvenanceSetter, SourceReference, UserId, WorkId,
};

/// Stored provider text for references whose input named no provider.
const LEGACY_PROVIDER: &str = "legacy";

/// Write every part of `facts` for the Work just inserted by the settlement
/// transaction: column values, one provenance row per populated value, and
/// the source-reference rows. Any failure propagates so the caller's
/// transaction rolls back the Work row and its birth event with it.
pub(crate) async fn write_creation_facts_in_tx(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    user_id: UserId,
    work_id: WorkId,
    facts: &CreationFacts,
) -> Result<(), DbError> {
    for provenance in &facts.provenance {
        validate_provenance(provenance.setter, provenance.source)?;
    }

    let fields = &facts.fields;
    let genres_json = fields
        .genres
        .as_ref()
        .filter(|genres| !genres.is_empty())
        .map(|genres| serde_json::to_string(genres).map_err(|e| DbError::Io(Box::new(e))))
        .transpose()?;
    let result = sqlx::query(
        "UPDATE works SET language = ?, description = ?, description_truncated = ?, \
         year = ?, original_publish_date = ?, publish_date = ?, publisher = ?, \
         page_count = ?, series_name = ?, series_position = ?, genres = ?, \
         rating = ?, rating_count = ?, cover_url = ? \
         WHERE user_id = ? AND id = ?",
    )
    .bind(fields.language.as_deref())
    .bind(fields.description.as_deref())
    .bind(fields.description_truncated)
    .bind(fields.year)
    .bind(fields.original_publish_date.as_deref())
    .bind(fields.publish_date.as_deref())
    .bind(fields.publisher.as_deref())
    .bind(fields.page_count)
    .bind(fields.series_name.as_deref())
    .bind(fields.series_position)
    .bind(genres_json.as_deref())
    .bind(fields.rating)
    .bind(fields.rating_count)
    .bind(absolute_http_cover_url(fields.cover_url.as_deref()))
    .bind(user_id)
    .bind(work_id)
    .execute(&mut **tx)
    .await
    .map_err(map_db_err)?;
    if result.rows_affected() != 1 {
        return Err(DbError::NotFound { entity: "work" });
    }

    let now = Utc::now().to_rfc3339();
    for provenance in &facts.provenance {
        sqlx::query(
            "INSERT INTO work_metadata_provenance \
             (user_id, work_id, field, source, set_at, setter, cleared) \
             VALUES (?, ?, ?, ?, ?, ?, 0) \
             ON CONFLICT(work_id, field) DO UPDATE SET \
             user_id = excluded.user_id, \
             source = excluded.source, \
             set_at = excluded.set_at, \
             setter = excluded.setter, \
             cleared = excluded.cleared",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(to_str(provenance.field))
        .bind(provenance.source.map(to_str))
        .bind(&now)
        .bind(to_str(provenance.setter))
        .execute(&mut **tx)
        .await
        .map_err(map_db_err)?;
    }

    for reference in &facts.references {
        sqlx::query(
            "INSERT INTO work_source_references \
             (user_id, work_id, provider, kind, value, ordinal) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(provider_text(reference.provider))
        .bind(to_str(reference.kind))
        .bind(&reference.value)
        .bind(reference.ordinal)
        .execute(&mut **tx)
        .await
        .map_err(map_db_err)?;
    }
    Ok(())
}

/// The same setter/source invariant the provenance writers enforce: a
/// provider-set value names its provider; every other setter is source-free.
fn validate_provenance(
    setter: ProvenanceSetter,
    source: Option<MetadataProvider>,
) -> Result<(), DbError> {
    match setter {
        ProvenanceSetter::Provider if source.is_none() => Err(DbError::Constraint {
            message: "provider setter requires a non-null source".to_string(),
        }),
        ProvenanceSetter::Provider => Ok(()),
        _ if source.is_some() => Err(DbError::Constraint {
            message: "user/system/auto_added setter must not have a source".to_string(),
        }),
        _ => Ok(()),
    }
}

fn provider_text(provider: Option<MetadataProvider>) -> String {
    provider.map_or_else(|| LEGACY_PROVIDER.to_string(), to_str)
}

fn provider_from_text(text: &str) -> Result<Option<MetadataProvider>, DbError> {
    if text == LEGACY_PROVIDER {
        Ok(None)
    } else {
        from_str(text).map(Some)
    }
}

impl crate::SourceReferenceDb for SqliteDb {
    async fn list_source_references(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<SourceReference>, DbError> {
        let rows = sqlx::query(
            "SELECT provider, kind, value, ordinal FROM work_source_references \
             WHERE user_id = ? AND work_id = ? \
             ORDER BY kind, ordinal, provider, value",
        )
        .bind(user_id)
        .bind(work_id)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;
        rows.into_iter()
            .map(|row| {
                let provider: String = row
                    .try_get("provider")
                    .map_err(|e| DbError::Io(Box::new(e)))?;
                let kind: String = row.try_get("kind").map_err(|e| DbError::Io(Box::new(e)))?;
                Ok(SourceReference {
                    provider: provider_from_text(&provider)?,
                    kind: from_str(&kind)?,
                    value: row.try_get("value").map_err(|e| DbError::Io(Box::new(e)))?,
                    ordinal: row
                        .try_get("ordinal")
                        .map_err(|e| DbError::Io(Box::new(e)))?,
                })
            })
            .collect()
    }
}
