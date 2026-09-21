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
    ApplyMergeOutcome, AudiobookChapter, Author, AuthorId, Bookmark, CreationFacts, CreationFields,
    CreationProvenance, CrossFormatState, DbError, DownloadClient, DownloadClientId,
    DownloadClientImplementation, EnrichmentStatus, EventType, ExternalIdRowId, ExternalIdType,
    FieldDissent, FieldProvenance, Grab, GrabId, GrabStatus, HistoryEvent, HistoryFilter,
    HistoryId, Import, Indexer, IndexerConfig, IndexerId, IndexerRssState, KashLink, LibraryItem,
    LibraryItemId, LlmProvider, MediaType, MergeResolved, MetadataProvider, NarrationType,
    NewKashLink, Notification, NotificationId, NotificationType, OutcomeClass, PlaybackProgress,
    ProvenanceSetter, RemotePathMapping, RemotePathMappingId, RootFolder, RootFolderId, Series,
    Session, SourceReference, SourceReferenceKind, TagStatus, User, UserId, UserRole, Work,
    WorkField, WorkId,
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
mod sqlite_source_reference;
mod sqlite_user;
mod sqlite_work;
mod sqlite_work_identity;
pub use sqlite_work_identity::backfill_work_identity_ledger;

#[cfg(test)]
mod cross_user_isolation_tests;
#[cfg(test)]
mod playback_enhancement_tests;

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

    // ------------------------------------------------------------------
    // Legacy identity-state fixtures.
    //
    // The runtime writers for these rows are retired; live behaviour only
    // READS them (frozen scalar badge, pending-anchor affirm door, grey-park
    // candidates, dead-end table). Tests seed that state here, mirroring the
    // retired writers' row shapes exactly, through the crate-private
    // serializers so there is still one authority per encoding.
    // ------------------------------------------------------------------

    /// Seed the frozen legacy identity badge (and advance the identity
    /// generation, as every identity mutation must).
    pub async fn set_identity_status_fixture(
        db: &SqliteDb,
        user_id: i64,
        work_id: i64,
        status: livrarr_domain::IdentityStatus,
    ) -> Result<(), sqlx::Error> {
        let result = sqlx::query(
            "UPDATE works SET identity_status = ?, \
             identity_generation = identity_generation + 1 \
             WHERE id = ? AND user_id = ?",
        )
        .bind(crate::sqlite_identity_conflict::identity_status_str(status))
        .bind(work_id)
        .bind(user_id)
        .execute(db.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        Ok(())
    }

    /// Seed a fuzzy pending anchor guess in the ledger (no works.* sync).
    pub async fn record_pending_anchor_fixture(
        db: &SqliteDb,
        work_id: i64,
        anchor_type: livrarr_domain::identity::AnchorType,
        value: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = crate::pool::begin_write(db.pool()).await?;
        crate::sqlite_work_identity::bump_identity_generation(&mut tx, work_id).await?;
        sqlx::query(
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
             VALUES (?1, ?2, ?3, 'pending', 'auto_search', ?4, (SELECT user_id FROM works WHERE id = ?1))
             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                 confidence = 'pending',
                 setter = 'auto_search',
                 set_at = ?4
             WHERE work_identity_anchors.confidence != 'confirmed'",
        )
        .bind(work_id)
        .bind(anchor_type.as_str())
        .bind(value)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    /// Seed the retired add-path "pending, no candidates" shape: an
    /// empty-valued pending `ol_work` row plus the frozen Pending badge.
    pub async fn set_identity_pending_fixture(
        db: &SqliteDb,
        work_id: i64,
        _reason: livrarr_domain::identity::PendingReason,
        setter: livrarr_domain::identity::AnchorSetter,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let setter_str = serde_json::to_value(setter)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "auto_search".to_string());
        let mut tx = crate::pool::begin_write(db.pool()).await?;
        crate::sqlite_work_identity::bump_identity_generation(&mut tx, work_id).await?;
        sqlx::query(
            "INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id)
             VALUES (?1, 'ol_work', '', 'pending', ?2, ?3, (SELECT user_id FROM works WHERE id = ?1))
             ON CONFLICT (work_id, anchor_type, anchor_value) DO UPDATE SET
                 confidence = 'pending',
                 setter = ?2,
                 set_at = ?3",
        )
        .bind(work_id)
        .bind(&setter_str)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE works SET ol_key = NULL, identity_status = 'pending' WHERE id = ?1")
            .bind(work_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    /// Seed a grey-park candidate set for a work.
    pub async fn record_review_candidates_fixture(
        db: &SqliteDb,
        work_id: i64,
        candidates: &[livrarr_domain::identity::Candidate],
    ) -> Result<(), sqlx::Error> {
        let json =
            serde_json::to_string(candidates).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO work_identity_review_candidates (work_id, user_id, candidates_json, recorded_at)
             VALUES (?1, (SELECT user_id FROM works WHERE id = ?1), ?2, ?3)
             ON CONFLICT (work_id) DO UPDATE SET
                 candidates_json = ?2,
                 recorded_at = ?3",
        )
        .bind(work_id)
        .bind(&json)
        .bind(&now)
        .execute(db.pool())
        .await?;
        Ok(())
    }

    /// Seed one dead-end attempt for a missing anchor type.
    pub async fn bump_anchor_attempt_fixture(
        db: &SqliteDb,
        work_id: i64,
        anchor_type: livrarr_domain::identity::AnchorType,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO work_anchor_dead_ends (work_id, anchor_type, attempt_count, last_attempt_at, user_id)
             VALUES (?1, ?2, 1, ?3, (SELECT user_id FROM works WHERE id = ?1))
             ON CONFLICT (work_id, anchor_type) DO UPDATE SET
                 attempt_count = attempt_count + 1,
                 last_attempt_at = ?3",
        )
        .bind(work_id)
        .bind(anchor_type.as_str())
        .bind(&now)
        .execute(db.pool())
        .await?;
        Ok(())
    }

    /// Read the dead-end table for a work (observation only).
    pub async fn list_anchor_dead_ends_fixture(
        db: &SqliteDb,
        work_id: i64,
    ) -> Result<Vec<livrarr_domain::identity::AnchorDeadEnd>, sqlx::Error> {
        let rows: Vec<(String, i64, String)> = sqlx::query_as(
            "SELECT anchor_type, attempt_count, last_attempt_at
             FROM work_anchor_dead_ends WHERE work_id = ?1",
        )
        .bind(work_id)
        .fetch_all(db.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(anchor_type, attempt_count, last_attempt_at)| {
                livrarr_domain::identity::AnchorDeadEnd {
                    work_id,
                    anchor_type: livrarr_domain::identity::AnchorType::new(anchor_type),
                    attempt_count: attempt_count as u32,
                    last_attempt_at: chrono::DateTime::parse_from_rfc3339(&last_attempt_at)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                }
            })
            .collect())
    }

    /// Read a work's identity generation (observation only).
    pub async fn identity_generation_fixture(
        db: &SqliteDb,
        work_id: i64,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT identity_generation FROM works WHERE id = ?1")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
    }

    /// Seed a work the way live composition produces it: through the identity
    /// road's settlement commit, with an F2 captured identity whose active
    /// routes carry the given provider keys. Enrichment dispatch reads those
    /// routes; legacy scalar columns are never consulted.
    pub async fn settle_work_fixture(
        db: &SqliteDb,
        user_id: i64,
        title: &str,
        author_name: &str,
        language: Option<&str>,
        routes: &[(
            livrarr_domain::identity_layer::IdentityProvider,
            livrarr_domain::identity_layer::RouteKind,
            &str,
        )],
    ) -> livrarr_domain::Work {
        use livrarr_domain::identity_layer::{
            EvidenceProvenance, IdentityTitleTuple, RouteOwner, RouteProvenance, SettlementCommit,
            WorkContributor, WorkIdentityRepository, WorkRoute, WorkRouteState,
        };
        let (author, _) = crate::AuthorDb::create_author(
            db,
            crate::CreateAuthorDbRequest {
                user_id,
                name: author_name.to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            },
        )
        .await
        .expect("settle fixture: create author");
        let settled = WorkIdentityRepository::commit_settlement(
            db,
            SettlementCommit {
                user_id,
                existing_work_id: None,
                add_source: None,
                identity_title: IdentityTitleTuple {
                    main: title.to_string(),
                    subtitle: None,
                    volume: None,
                    normalized_main: livrarr_domain::normalize_for_matching(title),
                    normalized_subtitle: String::new(),
                    normalized_volume: String::new(),
                    provenance: EvidenceProvenance::User,
                },
                text_distinction: None,
                contributors: vec![WorkContributor {
                    user_id,
                    work_id: 0,
                    author_id: author.id,
                    ordinal: 0,
                    roles: Vec::new(),
                }],
                routes: routes
                    .iter()
                    .map(|(provider, kind, value)| WorkRoute {
                        id: 0,
                        user_id,
                        owner: RouteOwner::Work(0),
                        resolved_work_id: 0,
                        provider: provider.clone(),
                        kind: kind.clone(),
                        provider_scoped_id: (*value).to_string(),
                        state: WorkRouteState::Active,
                        provenance: RouteProvenance::UserChoice,
                        user_confirmed: true,
                        observed_at: chrono::Utc::now(),
                    })
                    .collect(),
                absorbed_work_ids: Vec::new(),
                expected_generation: 0,
                review_cards: Vec::new(),
                creation_facts: None,
            },
        )
        .await
        .expect("settle fixture: commit settlement");
        let work_id = settled.identity.own_work_id;
        if let Some(language) = language {
            sqlx::query("UPDATE works SET language = ?1 WHERE id = ?2")
                .bind(language)
                .bind(work_id)
                .execute(db.pool())
                .await
                .expect("settle fixture: language");
        }
        crate::WorkDb::get_work(db, user_id, work_id)
            .await
            .expect("settle fixture: read settled work")
    }
}

/// Re-export for external test crates that depend on `feature = "test-helpers"`.
#[cfg(any(test, feature = "test-helpers"))]
pub use test_helpers::create_test_db;
