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

pub mod pool;
pub mod sqlite;
mod sqlite_author;
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

    /// Create a test database backed by SQLite `:memory:`.
    ///
    /// Single connection (`:memory:` is per-connection), migrated, FK-on,
    /// busy_timeout matching production config.
    pub async fn create_test_db() -> SqliteDb {
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
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_works_user_normalized \
             ON works(user_id, normalized_title, normalized_author)",
        )
        .execute(&pool)
        .await
        .unwrap();
        SqliteDb::new(pool)
    }
}

/// Re-export for external test crates that depend on `feature = "test-helpers"`.
#[cfg(any(test, feature = "test-helpers"))]
pub use test_helpers::create_test_db;
