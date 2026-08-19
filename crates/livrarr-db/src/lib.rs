// =============================================================================
// CRATE: livrarr-db
// =============================================================================
// All SQL queries. Trait-based data access.
// Every user-scoped query takes explicit user_id -- no unscoped queries (AUTH-003).

pub use livrarr_domain::services::ProgressKind;
pub use livrarr_domain::services::{ProviderCallRecord, ProviderStats};
pub use livrarr_domain::settings::{
    EmailConfig, MediaManagementConfig, MetadataConfig, NamingConfig, ProwlarrConfig,
};
pub use livrarr_domain::{
    ApplyMergeOutcome, AudiobookChapter, Author, AuthorId, Bookmark, CrossFormatState, DbError,
    DownloadClient, DownloadClientId, DownloadClientImplementation, EnrichmentStatus, EventType,
    ExternalIdRowId, ExternalIdType, FieldDissent, FieldProvenance, Grab, GrabId, GrabStatus,
    HistoryEvent, HistoryFilter, HistoryId, Import, Indexer, IndexerConfig, IndexerId,
    IndexerRssState, KashLink, LibraryItem, LibraryItemId, LlmProvider, MediaType, MergeResolved,
    MetadataProvider, NarrationType, NewKashLink, Notification, NotificationId, NotificationType,
    OutcomeClass, PlaybackProgress, ProvenanceSetter, RemotePathMapping, RemotePathMappingId,
    RootFolder, RootFolderId, Series, Session, TagStatus, User, UserId, UserRole, Work, WorkField,
    WorkId,
};

mod api;
pub use api::*;

pub mod identity_layer;
pub mod pool;
pub mod sqlite;
mod sqlite_author;
mod sqlite_author_link;
mod sqlite_author_link_codec;
mod sqlite_bibliography;
mod sqlite_bookmarks;
mod sqlite_chapters;
pub(crate) mod sqlite_common;
mod sqlite_config;
mod sqlite_cross_format_state;
mod sqlite_download_client;
mod sqlite_external_id;
mod sqlite_field_dissents;
mod sqlite_grab;
mod sqlite_history;
mod sqlite_identity_conflict;
pub use sqlite_identity_conflict::ConflictApplyError;
mod sqlite_import;
mod sqlite_import_intent;
mod sqlite_indexer;
mod sqlite_kash_link;
mod sqlite_library_item;
mod sqlite_list_import;
mod sqlite_notification;
mod sqlite_playback_progress;
mod sqlite_provenance;
mod sqlite_provider_cache;
mod sqlite_provider_calls;
mod sqlite_provider_policy;
mod sqlite_readarr_origin;
mod sqlite_remote_path_mapping;
mod sqlite_retry_state;
mod sqlite_root_folder;
mod sqlite_series;
mod sqlite_series_cache;
mod sqlite_series_roster;
mod sqlite_session;
mod sqlite_user;
mod sqlite_work;
mod sqlite_work_identity;
pub use sqlite_work_identity::backfill_work_identity_ledger;

#[cfg(test)]
mod cross_user_isolation_tests;
#[cfg(test)]
mod playback_enhancement_tests;
#[cfg(test)]
mod sqlite_affirm_anchor_tests;
#[cfg(test)]
mod sqlite_identity_conflict_tests;

// ---------------------------------------------------------------------------
// Test Helpers
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers {
    use super::sqlite::SqliteDb;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn create_test_db_with_legacy_work_index(legacy_work_index: bool) -> SqliteDb {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        // Post-repair author schema: `create_author`'s named ON CONFLICT
        // target requires this index (production gets it from
        // `backfill_author_identity` at startup). Repair tests drop it by
        // name to seed the legacy pre-index state.
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
             ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
        )
        .execute(&pool)
        .await
        .unwrap();
        let db = SqliteDb::new(pool);
        db.ensure_identity_authority_ready()
            .await
            .expect("activate identity v2 on an empty test database");
        if legacy_work_index {
            #[cfg(test)]
            sqlx::query(
                "CREATE UNIQUE INDEX idx_works_user_normalized \
                 ON works(user_id, normalized_title, normalized_author)",
            )
            .execute(db.pool())
            .await
            .expect("retain the unit-test backfill index lifecycle");
            #[cfg(all(feature = "test-helpers", not(test)))]
            sqlx::query(
                "CREATE UNIQUE INDEX idx_works_test_helper_creation_dedup \
                 ON works(user_id, normalized_title, normalized_author)",
            )
            .execute(db.pool())
            .await
            .expect("retain legacy create_work's conflict target in external test fixtures");
        }
        db
    }

    /// Create the compatibility test database used by pre-cutover persistence
    /// tests. It retains the legacy Work-identity index those tests exercise.
    pub async fn create_test_db() -> SqliteDb {
        create_test_db_with_legacy_work_index(true).await
    }

    /// Create the live post-activation schema shape.
    ///
    /// Bug reproduction: identity-layer-rewrite F-1 — the authority marker and
    /// `idx_works_identity_v2` are present, while every legacy Work-identity
    /// index is absent exactly as it is after production activation.
    pub async fn create_activated_test_db() -> SqliteDb {
        create_test_db_with_legacy_work_index(false).await
    }

    /// Real single-connection SQLite `:memory:` database with migrations
    /// 082/083 applied and the supplied legacy rows seeded before readiness.
    /// Never calls `ensure_identity_authority_ready`; the authority marker
    /// stays `NotRun`/inactive. `#[cfg(any(test, feature = "test-helpers"))]`
    /// per IR v1 (ir-v1-identity-layer-rewrite.yaml:1141-1144).
    pub async fn create_pre_cutover_identity_test_db(
        fixture: crate::identity_layer::LegacyIdentityFixture,
    ) -> crate::identity_layer::PreCutoverIdentityTestDb {
        use crate::identity_layer::PreCutoverIdentityTestDb;
        use livrarr_domain::identity_layer::IdentityMigrationError;

        let invalid_label = fixture
            .works_and_authors
            .iter()
            .any(|row| row.label.trim().is_empty())
            || fixture
                .legacy_badge_route_matrix
                .iter()
                .any(|row| row.label.trim().is_empty())
            || fixture
                .monitoring_flags
                .iter()
                .any(|row| row.label.trim().is_empty());
        if invalid_label {
            panic!("{}", IdentityMigrationError::InvalidFixture);
        }

        let tempdir = tempfile::tempdir().expect("create pre-cutover fixture directory");
        let path = tempdir.path().join("pre-cutover-library.sqlite");
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("parse pre-cutover in-memory database URL")
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("open pre-cutover fixture database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate pre-cutover fixture database");

        let mut work_ids = Vec::new();
        for (index, work) in fixture.works_and_authors.iter().enumerate() {
            let author = sqlx::query(
                "INSERT INTO authors (user_id, name, normalized_name, added_at) \
                 VALUES (1, ?1, ?2, ?3)",
            )
            .bind(format!("Legacy Author {}", work.label))
            .bind(format!("legacy author {}", work.label.to_lowercase()))
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .expect("seed pre-cutover author");
            let row = sqlx::query(
                "INSERT INTO works \
                    (user_id, title, author_name, author_id, normalized_title, \
                     normalized_author, ol_key, added_at) \
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .bind(format!("Legacy Work {}", work.label))
            .bind(format!("Legacy Author {}", work.label))
            .bind(author.last_insert_rowid())
            .bind(format!("legacy work {}", work.label.to_lowercase()))
            .bind(format!("legacy author {}", work.label.to_lowercase()))
            .bind(format!("OL-LEGACY-{index}"))
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .expect("seed pre-cutover work");
            work_ids.push(row.last_insert_rowid());
        }
        if !fixture.legacy_routes_ledgers_and_reviews.label.is_empty() {
            let work_id = *work_ids
                .first()
                .unwrap_or_else(|| panic!("{}", IdentityMigrationError::InvalidFixture));
            sqlx::query(
                "INSERT INTO external_ids (work_id, id_type, id_value) \
                 VALUES (?1, 'GoodreadsBook', ?2)",
            )
            .bind(work_id)
            .bind(&fixture.legacy_routes_ledgers_and_reviews.label)
            .execute(&pool)
            .await
            .expect("seed pre-cutover external identity row");
        }
        for (index, badge) in fixture.legacy_badge_route_matrix.iter().enumerate() {
            let work_id = *work_ids
                .first()
                .unwrap_or_else(|| panic!("{}", IdentityMigrationError::InvalidFixture));
            sqlx::query(
                "INSERT INTO external_ids (work_id, id_type, id_value) \
                 VALUES (?1, ?2, ?3)",
            )
            .bind(work_id)
            .bind(format!("LegacyBadge{index}"))
            .bind(&badge.label)
            .execute(&pool)
            .await
            .expect("seed pre-cutover badge route row");
        }
        if !fixture.monitoring_flags.is_empty() {
            for work_id in &work_ids {
                sqlx::query("UPDATE works SET monitor_ebook = 1 WHERE user_id = 1 AND id = ?1")
                    .bind(work_id)
                    .execute(&pool)
                    .await
                    .expect("seed pre-cutover monitoring flag");
            }
        }
        sqlx::query("VACUUM INTO ?1")
            .bind(path.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .expect("copy pre-cutover fixture snapshot");

        PreCutoverIdentityTestDb {
            db: SqliteDb::new(pool),
            path,
            _tempdir: tempdir,
        }
    }
}

/// Re-export for external test crates that depend on `feature = "test-helpers"`.
#[cfg(any(test, feature = "test-helpers"))]
pub use test_helpers::create_test_db;
