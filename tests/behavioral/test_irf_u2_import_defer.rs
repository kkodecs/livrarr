//! RED-first behavioral coverage for identity-review-fixes U2 (REQ-003 / AC-003).
//!
//! This is a standalone `[[test]]` target. Batch cases traverse the production
//! router and `RequireAdmin`, use the live `AppIdentityRoad`, a real migrated
//! `SqliteDb`, and real files under a temporary library root. The provider
//! fetcher uses a no-socket sentinel, and every test asserts that it was never
//! invoked. Constructed-state cases are called out at their use sites.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_activated_test_db;
use livrarr_db::{
    AuthorLinkDb, CreateAuthorGateRequest, CreateUserDbRequest, RootFolderDb, UserDb,
};
use livrarr_domain::identity_layer::{self as ilr, IdentityRoadService, WorkIdentityRepository};
use livrarr_domain::{AuthorLinkTrigger, AuthorNameSource, MediaType, UserRole};
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::identity_layer::IdentityRoadCall;
use livrarr_server::state::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Column, Row};
use tower::ServiceExt;

const ISBN_FIXTURE: &str = "9780306406157";

// These regressions reuse the real router/SQLite harness below. Only Readarr's
// external HTTP service is a local fixture; materialization is never mocked.
#[cfg(unix)]
mod readarr_independent_copies {
    use super::*;
    use livrarr_db::{CreateLibraryItemDbRequest, LibraryItemDb, TagStatus};
    use livrarr_domain::services::ReadarrImportWorkflow;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[derive(Clone, Copy)]
    enum Existing {
        Absent,
        OrphanLink,
        RecordedLink,
        RecordedSourceLink,
        RecordedEdited,
        OtherWork,
        WrongSize,
        SamePath,
        TargetSymlink,
        UnwritableParent,
    }

    struct RestorePermissions(PathBuf, std::fs::Permissions);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, self.1.clone());
        }
    }

    struct AbortServer(tokio::task::JoinHandle<()>);
    impl Drop for AbortServer {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    fn identity(path: &Path) -> (u64, u64) {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    async fn check(case: Existing) {
        let mut harness = build_route_harness().await;
        let root = configure_root(&harness, MediaType::Ebook).await;
        let root_id = harness.db.list_root_folders().await.unwrap()[0].id;
        let relative = format!("{}/Copy Author/Readarr Copy.epub", harness.user_id);
        let target = root.join(&relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let source = if matches!(case, Existing::SamePath) {
            target.clone()
        } else {
            incoming_path(&harness, "Readarr Copy.epub")
        };
        write_epub(&source, "Readarr Copy");
        let source_before = std::fs::read(&source).unwrap();
        let author_id = seed_author(&harness.db, harness.user_id, "Copy Author").await;
        let work_id = seed_work(
            &harness.db,
            harness.user_id,
            author_id,
            "Readarr Copy",
            None,
        )
        .await;
        let peer = harness.tmp.path().join("previous-library-link.epub");
        // Prove this fixture permits hardlinks: a cross-device fallback would
        // conceal the original Readarr bug.
        std::fs::hard_link(&source, &peer).unwrap();
        assert_eq!(identity(&source), identity(&peer));
        std::fs::remove_file(&peer).unwrap();
        match case {
            Existing::Absent | Existing::SamePath => {}
            Existing::OrphanLink
            | Existing::RecordedSourceLink
            | Existing::OtherWork
            | Existing::UnwritableParent => {
                std::fs::hard_link(&source, &target).unwrap();
            }
            Existing::RecordedLink => {
                // The source may have been replaced since the old import.
                // Preserve the edited library bytes and separate every link,
                // even when source and destination no longer share an inode.
                std::fs::write(&peer, b"existing edited library content").unwrap();
                std::fs::hard_link(&peer, &target).unwrap();
            }
            Existing::RecordedEdited | Existing::WrongSize => {
                std::fs::write(&target, b"existing edited library content").unwrap();
            }
            Existing::TargetSymlink => {
                std::os::unix::fs::symlink(&source, &target).unwrap();
            }
        }
        let permissions_before = target.exists().then(|| {
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
            std::fs::metadata(&target).unwrap().permissions()
        });
        let _restore_permissions = if matches!(case, Existing::UnwritableParent) {
            let parent = target.parent().unwrap();
            let guard = RestorePermissions(
                parent.to_owned(),
                std::fs::metadata(parent).unwrap().permissions(),
            );
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
            assert!(
                tempfile::NamedTempFile::new_in(parent).is_err(),
                "fixture must actually deny staging writes"
            );
            Some(guard)
        } else {
            None
        };
        let target_before = std::fs::read(&target).ok();
        let target_identity_before = target.exists().then(|| identity(&target));
        let owner = if matches!(case, Existing::OtherWork) {
            seed_work(
                &harness.db,
                harness.user_id,
                author_id,
                "Another Book",
                None,
            )
            .await
        } else {
            work_id
        };
        // Constructed-state justification: these are real writer-produced
        // rows and real files from a prior successful import, or an orphan
        // left between file publication and row creation. No outcome is injected.
        let existing_id = if matches!(
            case,
            Existing::RecordedLink
                | Existing::RecordedSourceLink
                | Existing::RecordedEdited
                | Existing::OtherWork
        ) {
            Some(
                harness
                    .db
                    .create_library_item(CreateLibraryItemDbRequest {
                        user_id: harness.user_id,
                        work_id: owner,
                        root_folder_id: root_id,
                        path: relative.clone(),
                        media_type: MediaType::Ebook,
                        file_size: std::fs::metadata(&target).unwrap().len() as i64,
                        import_id: None,
                        tag_status: TagStatus::Pending,
                        tagged_at_generation: 0,
                    })
                    .await
                    .unwrap()
                    .id,
            )
        } else {
            None
        };

        let source_for_http = source.clone();
        let source_size = source_before.len();
        let payload = move |uri: axum::http::Uri| {
            let value = match uri.path() {
                "/api/v1/system/status" => json!({"appName": "Readarr", "version": "test"}),
                "/api/v1/author" => json!([{"id": 1, "authorName": "Copy Author"}]),
                "/api/v1/book" => {
                    json!([{"id": 2, "authorId": 1, "title": "Readarr Copy", "monitored": true}])
                }
                "/api/v1/bookfile" => json!([{"id": 3, "authorId": 1, "bookId": 2,
                    "path": source_for_http, "size": source_size}]),
                "/api/v1/rootfolder" => {
                    json!([{"id": 4, "path": source_for_http.parent().unwrap()}])
                }
                "/api/v1/edition" => json!([]),
                _ => panic!("unexpected fixture endpoint: {uri}"),
            };
            async move { axum::Json(value) }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = AbortServer(tokio::spawn(async move {
            axum::serve(listener, Router::new().fallback(payload))
                .await
                .unwrap();
        }));
        harness.state.readarr_import_wf = Arc::new(
            livrarr_server::readarr_import_workflow::LiveReadarrImportWorkflow::new(
                livrarr_http::fetcher::HttpFetcherImpl::new().unwrap(),
                harness.state.readarr_import_service.clone(),
                harness.state.readarr_import_progress.clone(),
                harness.state.data_dir.clone(),
                harness.state.work_service.clone(),
                harness.db.clone(),
                harness.state.import_workflow.clone(),
            )
            .with_identity_road(harness.state.identity_road.clone()),
        );
        harness
            .state
            .readarr_import_wf
            .add_origin(url.clone())
            .await
            .unwrap();
        harness.app = livrarr_server::router::build_router(
            harness.state.clone(),
            harness.tmp.path().join("no-ui"),
        );
        let response = call_router_json(
            &harness.app,
            &harness.api_key,
            Method::POST,
            "/api/v1/import/readarr/start",
            Some(json!({
                "url": url, "apiKey": "local-fixture-key", "readarrRootFolderId": 4,
                "livrarrRootFolderId": root_id, "filesOnly": true
            })),
        )
        .await;
        assert!(response.status.is_success(), "{:?}", response);
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if !harness.state.readarr_import_progress.lock().await.running {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Readarr run completes");
        drop(server);
        let progress = harness.state.readarr_import_progress.lock().await;
        assert_eq!(
            progress.files_processed, 1,
            "run reached file import: {progress:?}"
        );
        assert_eq!(std::fs::read(&source).unwrap(), source_before);
        if matches!(
            case,
            Existing::OtherWork
                | Existing::WrongSize
                | Existing::SamePath
                | Existing::TargetSymlink
                | Existing::UnwritableParent
        ) {
            assert!(
                !progress.errors.is_empty(),
                "import must report refusal: {progress:?}"
            );
            assert!(
                progress
                    .errors
                    .iter()
                    .any(|e| e.starts_with("File for 'Readarr Copy':")),
                "the file operation must report the refusal: {progress:?}"
            );
            if matches!(case, Existing::OtherWork | Existing::WrongSize) {
                assert!(
                    progress.errors.iter().any(|e| e.contains("path collision")),
                    "{progress:?}"
                );
            }
            assert_eq!(std::fs::read(&target).unwrap(), target_before.unwrap());
            assert_eq!(Some(identity(&target)), target_identity_before);
            if matches!(case, Existing::TargetSymlink) {
                assert!(std::fs::symlink_metadata(&target).unwrap().is_symlink());
            }
            assert!(harness
                .db
                .list_library_items_by_work(harness.user_id, work_id)
                .await
                .unwrap()
                .is_empty());
            if let Some(id) = existing_id {
                assert_eq!(
                    harness
                        .db
                        .list_library_items_by_work(harness.user_id, owner)
                        .await
                        .unwrap()[0]
                        .id,
                    id
                );
            }
        } else {
            assert!(progress.errors.is_empty(), "{progress:?}");
            let rows = harness
                .db
                .list_library_items_by_work(harness.user_id, work_id)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].path, relative);
            if let Some(permissions) = permissions_before {
                assert_eq!(
                    std::fs::metadata(&target).unwrap().permissions(),
                    permissions
                );
            }
            if let Some(id) = existing_id {
                assert_eq!(rows[0].id, id);
            }
            assert_eq!(
                std::fs::read(&target).unwrap(),
                target_before.unwrap_or_else(|| source_before.clone())
            );
            assert_ne!(
                identity(&source),
                identity(&target),
                "Readarr must create an independent copy"
            );
            assert_eq!(
                std::fs::metadata(&target).unwrap().nlink(),
                1,
                "retry retained an old hardlink"
            );
            let peer_before = std::fs::read(&peer).ok();
            // File::create truncates in place; an atomic replace would hide a link.
            std::fs::write(&target, b"changed library copy").unwrap();
            assert_eq!(std::fs::read(&source).unwrap(), source_before);
            if let Some(bytes) = peer_before {
                assert_eq!(std::fs::read(&peer).unwrap(), bytes);
            }
        }
        assert_eq!(harness.provider_attempts.load(Ordering::SeqCst), 0);
    }

    /// REQ-001 / AC-001: real Readarr route, real importer, hardlinks available.
    #[tokio::test]
    async fn new_import_is_independent() {
        check(Existing::Absent).await;
    }
    /// REQ-002 / AC-002: a crashed pre-fix import left a linked orphan.
    #[tokio::test]
    async fn orphan_retry_is_independent() {
        check(Existing::OrphanLink).await;
    }
    /// REQ-002 / AC-003: preserve existing library edits and the LibraryItem ID.
    #[tokio::test]
    async fn recorded_retry_separates_old_link_and_keeps_edits() {
        check(Existing::RecordedLink).await;
    }
    #[tokio::test]
    async fn recorded_retry_separates_current_source_link() {
        check(Existing::RecordedSourceLink).await;
    }
    #[tokio::test]
    async fn recorded_retry_keeps_independent_edits() {
        check(Existing::RecordedEdited).await;
    }
    /// REQ-003 / AC-004: ownership and collision rejection still precede writes.
    #[tokio::test]
    async fn another_works_target_is_unchanged() {
        check(Existing::OtherWork).await;
    }
    #[tokio::test]
    async fn mismatched_orphan_is_unchanged() {
        check(Existing::WrongSize).await;
    }
    #[tokio::test]
    async fn same_path_is_refused_without_mutation() {
        check(Existing::SamePath).await;
    }
    #[tokio::test]
    async fn target_symlink_is_refused_without_mutation() {
        check(Existing::TargetSymlink).await;
    }
    #[tokio::test]
    async fn staging_failure_preserves_existing_link_and_bytes() {
        check(Existing::UnwritableParent).await;
    }
}

struct RouteHarness {
    app: Router,
    state: AppState,
    db: SqliteDb,
    api_key: String,
    user_id: i64,
    provider_attempts: Arc<AtomicU64>,
    source_snapshot: std::sync::Mutex<Option<FileTreeSnapshot>>,
    tmp: tempfile::TempDir,
}

#[derive(Debug)]
struct RouteResponse {
    status: StatusCode,
    json: Value,
}

async fn build_route_harness() -> RouteHarness {
    build_route_harness_from(
        create_activated_test_db().await,
        tempfile::tempdir().expect("U2 tempdir"),
    )
    .await
}

/// AC-003(6) needs more than the single connection intentionally used by
/// `create_activated_test_db()`: the contender must enter SQLite and wait at the real
/// `BEGIN IMMEDIATE`. This uses the production pool builder (WAL, four
/// connections, busy timeout), production migrations, production activation,
/// and still wraps the resulting pool in the real `SqliteDb`.
async fn build_concurrent_route_harness() -> RouteHarness {
    let tmp = tempfile::tempdir().expect("U2 concurrent tempdir");
    let db_dir = tmp.path().join("sqlite");
    std::fs::create_dir_all(&db_dir).expect("create concurrent db directory");
    let pool = livrarr_db::pool::create_sqlite_pool(&db_dir)
        .await
        .expect("create production SQLite pool");
    livrarr_db::pool::run_migrations(&pool)
        .await
        .expect("run production migrations");
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
         ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
    )
    .execute(&pool)
    .await
    .expect("install startup author identity index");
    let db = SqliteDb::new(pool);
    assert_eq!(
        db.ensure_identity_authority_ready()
            .await
            .expect("activate identity authority"),
        ilr::IdentityAuthorityReadiness::ActivatedFresh
    );
    build_route_harness_from(db, tmp).await
}

async fn build_route_harness_from(db: SqliteDb, tmp: tempfile::TempDir) -> RouteHarness {
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = "irf-u2-admin-api-key".to_string();
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash U2 API key");
    let user = db
        .create_user(CreateUserDbRequest {
            username: format!(
                "irf-u2-admin-{}",
                Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create authenticated U2 user");

    let auth_service = Arc::new(livrarr_server::auth_service::ServerAuthService::new(
        db.clone(),
        RealAuthCrypto,
    ));
    let user_agent = livrarr_http::livrarr_user_agent();
    let http_client = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(&user_agent)
        .build()
        .expect("HTTP client");
    let http_client_safe = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(&user_agent)
        .ssrf_safe(true)
        .build()
        .expect("SSRF-safe HTTP client");

    // No provider HTTP is allowed in U2. This hermetic callback replaces only
    // the socket exchange and records an attempted crossing; post-U2 every
    // test below observes zero. It is deliberately not a provider fixture.
    let provider_attempts = Arc::new(AtomicU64::new(0));
    let provider_attempts_for_fetcher = provider_attempts.clone();
    let http_fetcher = livrarr_http::fetcher::HttpFetcherImpl::new()
        .expect("shared HTTP fetcher")
        .with_scripted_transport(move |_| {
            provider_attempts_for_fetcher.fetch_add(1, Ordering::SeqCst);
            livrarr_http::fetcher::ScriptedTransportOutcome::Error {
                delay: Duration::ZERO,
                error: livrarr_domain::services::FetchError::Connection(
                    "U2 forbids provider HTTP".to_string(),
                ),
            }
        });
    let llm_http_client = livrarr_http::HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(&user_agent)
        .build()
        .expect("LLM HTTP client");
    let live_metadata_config =
        livrarr_external_data::live_config::LiveMetadataConfig::new(livrarr_db::MetadataConfig {
            hardcover_enabled: false,
            hardcover_api_token: None,
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: None,
        });
    let transport_cache = Arc::new(livrarr_external_data::transport_cache::TransportCache::new(
        Duration::from_secs(300),
    ));
    let import_semaphore = Arc::new(tokio::sync::Semaphore::new(2));
    let cover_proxy_cache = Arc::new(livrarr_server::infra::cover_cache::CoverProxyCache::new());
    let rss_last_run = Arc::new(AtomicI64::new(0));
    let rss_sync_running = Arc::new(AtomicBool::new(false));
    let manual_import_scans_shared: Arc<livrarr_server::state::ManualImportScanMap> =
        Arc::new(Default::default());
    let log_buffer = Arc::new(livrarr_server::state::LogBuffer::new());
    let log_level_handle = {
        let (_layer, handle) =
            tracing_subscriber::reload::Layer::new(tracing_subscriber::EnvFilter::new("info"));
        Arc::new(livrarr_server::state::LogLevelHandle::new(handle, "info"))
    };
    let settings_service_arc =
        Arc::new(livrarr_server::services::settings_service::LiveSettingsService::new(db.clone()));
    let import_io_arc = Arc::new(livrarr_server::import_io_service::ImportIoServiceImpl::new(
        db.clone(),
    ));
    let import_workflow_arc = Arc::new(livrarr_library::import_workflow::ImportWorkflowImpl::new(
        db.clone(),
        import_semaphore.clone(),
        data_dir_arc.clone(),
        Arc::new(livrarr_server::chapter_extractor::ChapterExtractorImpl),
    ));
    let tag_service_arc = Arc::new(livrarr_server::tag_service::LiveTagService::new(
        import_io_arc.clone(),
        data_dir_arc.clone(),
        db.clone(),
    ));
    let import_svc_arc = Arc::new(livrarr_server::import_service::LiveImportService::new(
        import_io_arc.clone(),
        import_workflow_arc.clone(),
        tag_service_arc.clone(),
        settings_service_arc.clone(),
        http_client_safe.clone(),
    ));
    let trusted_origins_arc = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    let readarr_import_service_arc =
        Arc::new(livrarr_server::readarr_import_service::LiveReadarrImportService::new(db.clone()));
    let readarr_import_progress_arc = Arc::new(tokio::sync::Mutex::new(
        livrarr_server::readarr_import_service::ReadarrImportProgress::default(),
    ));
    let identity_resolver_arc = livrarr_server::state::build_live_identity_resolver(
        std::collections::HashMap::new(),
        transport_cache,
        livrarr_metadata::english_identity_resolver::ResolverConfig::default(),
    );
    let db_arc = Arc::new(db.clone());
    let queue =
        Arc::new(livrarr_metadata::DefaultProviderQueueBuilder::new().build(db_arc.clone()));
    let enrichment_service = Arc::new(livrarr_metadata::EnrichmentServiceImpl::new(
        db_arc,
        queue.clone(),
        Arc::new(livrarr_metadata::DefaultMergeEngine::new(
            livrarr_metadata::PriorityModel::english(),
        )),
        false,
    ));
    let work_service_arc: Arc<livrarr_server::state::LiveWorkService> =
        Arc::new(livrarr_server::state::build_live_work_service(
            db.clone(),
            enrichment_service.clone(),
            http_fetcher.clone(),
            data_dir.clone(),
            identity_resolver_arc.clone(),
        ));
    let discovery_service_arc = Arc::new(
        livrarr_metadata::discovery_service::DiscoveryServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                live_metadata_config.clone(),
                llm_http_client.clone(),
            ),
        )
        .with_resolver(identity_resolver_arc.clone()),
    );
    let hmac_key = livrarr_server::cover_service::generate_hmac_key();
    let cover_service = Arc::new(livrarr_server::cover_service::LiveCoverService::new(
        db.clone(),
        http_fetcher.clone(),
        std::collections::HashMap::new(),
        hmac_key.clone(),
        data_dir_arc.clone(),
    ));
    let identity_road_arc = Arc::new(
        livrarr_server::identity_layer::build_recording_identity_road(
            db.clone(),
            http_fetcher.clone(),
            http_client.clone(),
            live_metadata_config.clone(),
        ),
    );

    let state = AppState {
        db: db.clone(),
        auth_service,
        http_client: http_client.clone(),
        http_client_safe,
        http_fetcher: http_fetcher.clone(),
        config: Arc::new(livrarr_server::config::AppConfig::default()),
        data_dir: data_dir_arc.clone(),
        startup_time: Utc::now(),
        job_runner: None,
        cover_proxy_cache: cover_proxy_cache.clone(),
        live_metadata_config: live_metadata_config.clone(),
        log_buffer: log_buffer.clone(),
        log_level_handle: log_level_handle.clone(),
        import_semaphore: import_semaphore.clone(),
        rss_last_run: rss_last_run.clone(),
        rss_sync_running: rss_sync_running.clone(),
        readarr_import_progress: readarr_import_progress_arc.clone(),
        manual_import_scans: manual_import_scans_shared.clone(),
        provider_queue: queue,
        enrichment_service: enrichment_service.clone(),
        identity_road: identity_road_arc.clone(),
        author_service: Arc::new(livrarr_metadata::author_service::AuthorServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                live_metadata_config.clone(),
                llm_http_client.clone(),
            ),
        )),
        author_link_service: Arc::new(
            livrarr_server::services::author_linking_service::LiveAuthorLinkingService,
        ),
        series_service: Arc::new(livrarr_metadata::series_service::SeriesServiceImpl::new(
            db.clone(),
        )),
        series_query_service: Arc::new(
            livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
                db.clone(),
                http_fetcher.clone(),
                work_service_arc.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    llm_http_client.clone(),
                ),
            )
            .with_identity_road(identity_road_arc.clone()),
        ),
        work_service: work_service_arc.clone(),
        discovery_service: discovery_service_arc,
        grab_service: Arc::new(livrarr_download::grab_service::GrabServiceImpl::new(
            db.clone(),
        )),
        release_service: Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            trusted_origins_arc.clone(),
        )),
        file_service: Arc::new(livrarr_library::file_service::FileServiceImpl::new(
            db.clone(),
        )),
        chapter_service: Arc::new(livrarr_library::chapter_service::ChapterServiceImpl::new(
            db.clone(),
        )),
        bookmark_service: Arc::new(livrarr_library::bookmark_service::BookmarkServiceImpl::new(
            db.clone(),
        )),
        cross_format_service: Arc::new(
            livrarr_library::cross_format_service::CrossFormatServiceImpl::new(
                db.clone(),
                livrarr_library::file_service::FileServiceImpl::new(db.clone()),
            ),
        ),
        import_workflow: import_workflow_arc.clone(),
        rss_sync_workflow: {
            let release_service =
                Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
                    db.clone(),
                    http_fetcher.clone(),
                    trusted_origins_arc.clone(),
                ));
            Arc::new(
                livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                    Arc::new(db.clone()),
                    Arc::new(http_fetcher.clone()),
                    release_service,
                ),
            )
        },
        list_service: {
            let work_service = livrarr_server::state::build_live_work_service(
                db.clone(),
                enrichment_service.clone(),
                http_fetcher.clone(),
                data_dir.clone(),
                identity_resolver_arc.clone(),
            );
            Arc::new(
                livrarr_metadata::list_service::ListServiceImpl::with_identity_road(
                    db.clone(),
                    work_service,
                    http_fetcher.clone(),
                    livrarr_metadata::list_service::NoOpBibliographyTrigger,
                    identity_road_arc.clone(),
                ),
            )
        },
        identity_resolver: identity_resolver_arc.clone(),
        enrichment_workflow: Arc::new(
            livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            ),
        ),
        author_monitor_workflow: {
            let work_service = livrarr_server::state::build_live_work_service(
                db.clone(),
                enrichment_service.clone(),
                http_fetcher.clone(),
                data_dir.clone(),
                identity_resolver_arc.clone(),
            );
            Arc::new(
                livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
                    Arc::new(db.clone()),
                    Arc::new(work_service),
                    Arc::new(http_fetcher.clone()),
                    identity_road_arc.clone(),
                ),
            )
        },
        readarr_import_service: readarr_import_service_arc.clone(),
        settings_service: settings_service_arc.clone(),
        notification_service: Arc::new(
            livrarr_server::notification_service::NotificationServiceImpl::new(db.clone()),
        ),
        history_service: Arc::new(livrarr_server::history_service::HistoryServiceImpl::new(
            db.clone(),
        )),
        queue_service: Arc::new(livrarr_server::queue_service::QueueServiceImpl::new(
            db.clone(),
            http_client.clone(),
        )),
        import_io_service: import_io_arc.clone(),
        manual_import_db_service: Arc::new(
            livrarr_server::manual_import_service::ManualImportServiceImpl::new(db.clone()),
        ),
        rss_sync_state: livrarr_server::state::RssSyncState {
            running: rss_sync_running,
            last_run: rss_last_run,
        },
        system_state: livrarr_server::state::SystemState {
            log_buffer,
            log_level_handle,
        },
        provider_stats_service: Arc::new(livrarr_server::state::LiveProviderStatsService::new(
            db.clone(),
        )),
        log_surface_accessor: livrarr_server::state::LogSurfaceAccessorImpl {
            log_dir: data_dir.join("logs"),
            init_error: None,
        },
        live_metadata_config_accessor: livrarr_server::state::LiveMetadataConfigAccessorImpl(
            live_metadata_config,
        ),
        cover_proxy_cache_accessor: livrarr_server::state::CoverProxyCacheAccessorImpl(
            cover_proxy_cache,
        ),
        tag_service: tag_service_arc,
        email_svc: Arc::new(livrarr_server::email_service::LiveEmailService::new(
            settings_service_arc,
        )),
        import_svc: import_svc_arc,
        matching_svc: livrarr_server::matching_service::LiveMatchingService,
        manual_import_scan_svc:
            livrarr_server::manual_import_scan_service::LiveManualImportScanService {
                scans: manual_import_scans_shared,
            },
        readarr_import_wf: Arc::new(
            livrarr_server::readarr_import_workflow::LiveReadarrImportWorkflow::new(
                http_fetcher,
                readarr_import_service_arc,
                readarr_import_progress_arc,
                data_dir_arc,
                work_service_arc,
                db.clone(),
                import_workflow_arc,
            )
            .with_identity_road(identity_road_arc),
        ),
        cover_service,
        preadd_cover_service: Arc::new(
            livrarr_metadata::preadd_cover_service::LivePreaddCoverService::new(
                std::collections::HashMap::new(),
            ),
        ),
        hmac_key,
        trusted_origins_rebuilder: livrarr_server::state::TrustedOriginsRebuilderImpl(
            trusted_origins_arc,
        ),
    };
    let ui_dir = state.data_dir.join("ui-not-present-in-test");
    RouteHarness {
        app: livrarr_server::router::build_router(state.clone(), ui_dir),
        state,
        db,
        api_key,
        user_id: user.id,
        provider_attempts,
        source_snapshot: std::sync::Mutex::new(None),
        tmp,
    }
}

async fn call_router_json(
    app: &Router,
    api_key: &str,
    method: Method,
    path: impl Into<String>,
    body: Option<Value>,
) -> RouteResponse {
    let mut request = Request::builder()
        .method(method)
        .uri(path.into())
        .header("x-api-key", api_key);
    let request_body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request.body(request_body).expect("build U2 request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 82)),
        31_003,
    )));
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("production router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read U2 response body");
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"unparsedBody": String::from_utf8_lossy(&bytes).into_owned()}));
    RouteResponse { status, json }
}

async fn import_batch(harness: &RouteHarness, items: Vec<Value>) -> RouteResponse {
    call_router_json(
        &harness.app,
        &harness.api_key,
        Method::POST,
        "/api/v1/manualimport/import",
        Some(json!({ "items": items })),
    )
    .await
}

fn minimum_item(path: &Path, title: &str, author: &str) -> Value {
    json!({
        "path": path,
        "olKey": "",
        "title": title,
        "author": author,
        "deleteExisting": false
    })
}

fn isbn_item(path: &Path, title: &str, author: &str) -> Value {
    let mut item = minimum_item(path, title, author);
    item["isbn"] = json!(ISBN_FIXTURE);
    item
}

async fn configure_root(harness: &RouteHarness, media_type: MediaType) -> PathBuf {
    let label = match media_type {
        MediaType::Ebook => "ebook-library",
        MediaType::Audiobook => "audio-library",
    };
    let root = harness.tmp.path().join(label);
    std::fs::create_dir_all(&root).expect("create U2 library root");
    harness
        .db
        .create_root_folder(root.to_str().expect("UTF-8 root"), media_type)
        .await
        .expect("register real root folder");
    root
}

fn incoming_path(harness: &RouteHarness, name: &str) -> PathBuf {
    let incoming = harness.tmp.path().join("incoming");
    std::fs::create_dir_all(&incoming).expect("create incoming directory");
    incoming.join(name)
}

fn write_epub(path: &Path, title: &str) {
    let file = std::fs::File::create(path).expect("create EPUB fixture");
    let mut zip = zip::ZipWriter::new(file);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).expect("EPUB mimetype");
    zip.write_all(b"application/epub+zip")
        .expect("write EPUB mimetype");
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/container.xml", deflated)
        .expect("EPUB container");
    zip.write_all(br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)
        .expect("write EPUB container");
    zip.start_file("OEBPS/content.opf", deflated)
        .expect("EPUB OPF");
    zip.write_all(
        format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:language>en</dc:language></metadata><manifest/><spine/></package>"#).as_bytes(),
    )
    .expect("write EPUB OPF");
    zip.finish().expect("finish EPUB fixture");
}

fn write_m4b(path: &Path, marker: &[u8]) {
    let mut bytes = b"U2-real-file-evidence-".to_vec();
    bytes.extend_from_slice(marker);
    std::fs::write(path, bytes).expect("write M4B fixture");
}

type FileTreeSnapshot = Vec<(PathBuf, Vec<u8>)>;

fn files_under(root: &Path) -> FileTreeSnapshot {
    fn visit(root: &Path, path: &Path, output: &mut FileTreeSnapshot) {
        if !path.exists() {
            return;
        }
        for entry in std::fs::read_dir(path).expect("read fixture directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("file remains under snapshot root")
                        .to_path_buf(),
                    std::fs::read(&path).expect("read fixture snapshot"),
                ));
            }
        }
    }
    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort();
    output
}

fn incoming_files(harness: &RouteHarness) -> FileTreeSnapshot {
    files_under(&harness.tmp.path().join("incoming"))
}

async fn imported_target(db: &SqliteDb, user_id: i64, work_id: i64, root: &Path) -> PathBuf {
    let relative: String = sqlx::query_scalar(
        "SELECT path FROM library_items WHERE user_id=?1 AND work_id=?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("read imported LibraryItem path");
    root.join(relative)
}

async fn assert_imported_copy(harness: &RouteHarness, work_id: i64, root: &Path, source: &Path) {
    let target = imported_target(&harness.db, harness.user_id, work_id, root).await;
    assert!(source.exists(), "ManualImport Copy preserves its source");
    assert_ne!(target, source, "the LibraryItem names the library copy");
    assert!(target.is_file(), "the imported target exists: {target:?}");
}

async fn seed_author(db: &SqliteDb, user_id: i64, name: &str) -> i64 {
    let (author, created) = AuthorLinkDb::create_or_adopt_author(
        db,
        CreateAuthorGateRequest {
            user_id,
            name: name.to_string(),
            sort_name: None,
            import_id: None,
            initial_name_source: AuthorNameSource::User,
            trigger: AuthorLinkTrigger::AuthorCreated,
        },
    )
    .await
    .expect("seed Author through production create/adopt gate");
    assert!(created, "fixture Author must be new");
    author.id
}

async fn seed_work(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    title: &str,
    distinction: Option<&str>,
) -> i64 {
    let mut identity_title =
        ilr::title_parts_from_provider(title.to_string(), None).expect("valid fixture title");
    identity_title.provenance = ilr::EvidenceProvenance::User;
    let committed = WorkIdentityRepository::commit_settlement(
        db,
        ilr::SettlementCommit {
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title,
            text_distinction: distinction.map(str::to_string),
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes: vec![],
            absorbed_work_ids: vec![],
            expected_generation: 0,
            review_cards: vec![],
        },
    )
    .await
    .expect("seed Work through sole settlement writer");
    committed.identity.own_work_id
}

async fn owned_file_evidence(path: &Path) -> ilr::OwnedFileEvidence {
    let bytes = tokio::fs::read(path)
        .await
        .expect("read owned-file fixture");
    let metadata = tokio::fs::metadata(path)
        .await
        .expect("stat owned-file fixture");
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos() as i128);
    ilr::OwnedFileEvidence {
        library_item_id: 0,
        file_revision: ilr::FileRevision {
            size_bytes: metadata.len(),
            modified_ns,
            sha256: Sha256::digest(&bytes).into(),
        },
    }
}

fn singular_defer(title: &str, author: &str) -> String {
    format!(
        "Not imported: you already have \"{title}\" by {author}. Choose Edit title and author for this row. To add this file to that book, enter exactly \"{title}\" and \"{author}\", then retry. To add it as a different book, enter a different main title (a different subtitle alone is not enough)."
    )
}

fn plural_defer(members: &[(&str, &str)]) -> String {
    let books = members
        .iter()
        .enumerate()
        .map(|(index, (title, author))| format!("({}) \"{title}\" by {author}", index + 1))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Not imported: this title matches multiple existing books: {books}. Manual import cannot choose among them. Choose Edit title and author for this row and enter a different main title to create a different book (a different subtitle alone is not enough)."
    )
}

fn assert_raw_failed_result(response: &RouteResponse, expected_error: &str) {
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "failed");
    assert_eq!(response.json["results"][0]["workId"], Value::Null);
    assert_eq!(response.json["results"][0]["error"], expected_error);
    let error = response.json["results"][0]["error"]
        .as_str()
        .expect("failed result error");
    assert!(!error.starts_with("work creation failed:"));
    assert!(!error.starts_with("conflict:"));
}

fn assert_deferred(outcome: ilr::IdentityRoadOutcome, expected: &str) {
    match outcome {
        ilr::IdentityRoadOutcome::Deferred { reason } => assert_eq!(reason.0, expected),
        other => panic!("expected Deferred({expected:?}), got {other:?}"),
    }
}

fn clear_road_calls(harness: &RouteHarness) {
    harness.state.identity_road.test_recorder().clear();
    *harness.source_snapshot.lock().expect("U2 source snapshot") = Some(incoming_files(harness));
}

fn minimum_road_commands(harness: &RouteHarness) -> Vec<ilr::ManualImportMinimumCommand> {
    harness
        .state
        .identity_road
        .test_recorder()
        .snapshot()
        .into_iter()
        .map(|call| match call {
            IdentityRoadCall::SettleManualImportMinimum(command) => command,
            IdentityRoadCall::Settle(request) => {
                panic!("minimum-only handler used ordinary settle: {request:?}")
            }
            IdentityRoadCall::Resolve { command, .. } => {
                panic!("minimum-only handler reached review continuation: {command:?}")
            }
        })
        .collect()
}

fn assert_no_provider_http(harness: &RouteHarness) {
    let expected_sources = harness.source_snapshot.lock().expect("U2 source snapshot");
    assert_eq!(
        incoming_files(harness),
        *expected_sources
            .as_ref()
            .expect("clear_road_calls captures source files before the action"),
        "the real import door leaves every source file byte-identical"
    );
    assert_eq!(
        harness.provider_attempts.load(Ordering::SeqCst),
        0,
        "U2 has no provider HTTP surface"
    );
}

// These are the real table names enumerated from migrations 001, 068, 078,
// and 082. In particular the Author-side writer surface is `authors`,
// `author_name_variants`, `author_link_progress` (the author-link task), and
// `author_provider_routes`; no guessed `author_links`/`author_tasks` aliases
// are used.
const USER_STATE_TABLES: &[&str] = &[
    "authors",
    "author_name_variants",
    "author_link_progress",
    "author_provider_routes",
    "works",
    "work_contributors",
    "work_contributor_roles",
    "work_identity_review_candidates",
    "identity_routes",
    "editions",
    "identity_conflicts_v2",
    "identity_review_cards",
    "identity_route_archives",
    "identity_merge_archives",
    "identity_audit_events",
    "library_items",
    "embedded_cover_inspections",
];

type RowSnapshot = Vec<(String, Option<String>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserStateSnapshot {
    tables: BTreeMap<String, Vec<RowSnapshot>>,
    // AC-003 names generations separately even though the complete Work rows
    // above already contain both columns.
    generations: Vec<(i64, i64, i64)>,
}

fn sqlite_cell(row: &sqlx::sqlite::SqliteRow, index: usize) -> Option<String> {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value.map(|value| format!("text:{value}"));
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value.map(|value| format!("integer:{value}"));
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value.map(|value| format!("real:{value:?}"));
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value.map(|bytes| {
            let hex = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            format!("blob:{hex}")
        });
    }
    panic!("unsupported SQLite cell {}", row.column(index).name());
}

fn sqlite_row(row: &sqlx::sqlite::SqliteRow) -> RowSnapshot {
    (0..row.len())
        .map(|index| {
            (
                row.column(index).name().to_string(),
                sqlite_cell(row, index),
            )
        })
        .collect()
}

async fn table_snapshot(db: &SqliteDb, user_id: i64, table: &str) -> Vec<RowSnapshot> {
    assert!(USER_STATE_TABLES.contains(&table), "unreviewed table name");
    let rows = sqlx::query(&format!(
        "SELECT * FROM {table} WHERE user_id=?1 ORDER BY rowid"
    ))
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .unwrap_or_else(|error| panic!("snapshot {table}: {error}"));
    rows.iter().map(sqlite_row).collect()
}

async fn user_state_snapshot(db: &SqliteDb, user_id: i64) -> UserStateSnapshot {
    let mut tables = BTreeMap::new();
    for table in USER_STATE_TABLES {
        tables.insert(
            (*table).to_string(),
            table_snapshot(db, user_id, table).await,
        );
    }
    let generations = sqlx::query_as(
        "SELECT id, identity_generation, merge_generation FROM works \
         WHERE user_id=?1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("snapshot Work generations");
    UserStateSnapshot {
        tables,
        generations,
    }
}

async fn row_by_id_snapshot(db: &SqliteDb, table: &str, id: i64) -> RowSnapshot {
    assert!(matches!(table, "works" | "authors" | "library_items"));
    let row = sqlx::query(&format!("SELECT * FROM {table} WHERE id=?1"))
        .bind(id)
        .fetch_one(db.pool())
        .await
        .unwrap_or_else(|error| panic!("snapshot {table}/{id}: {error}"));
    sqlite_row(&row)
}

async fn user_count(db: &SqliteDb, user_id: i64, table: &str) -> i64 {
    assert!(USER_STATE_TABLES.contains(&table), "unreviewed table name");
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE user_id=?1"))
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .unwrap_or_else(|error| panic!("count {table}: {error}"))
}

async fn work_generation(db: &SqliteDb, work_id: i64) -> i64 {
    sqlx::query_scalar("SELECT identity_generation FROM works WHERE id=?1")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("read Work generation")
}

async fn assert_one_unattached_failure(
    db: &SqliteDb,
    user_id: i64,
    path: &Path,
    expected_error: Option<&str>,
) {
    let rows: Vec<(Option<i64>, String)> = sqlx::query_as(
        "SELECT work_id, data FROM history WHERE user_id=?1 AND event_type='importFailed' \
         AND json_extract(data, '$.path')=?2 ORDER BY id",
    )
    .bind(user_id)
    .bind(path.to_string_lossy().as_ref())
    .fetch_all(db.pool())
    .await
    .expect("read failure-history event");
    assert_eq!(
        rows.len(),
        1,
        "exactly one failure-history event for {path:?}"
    );
    assert_eq!(rows[0].0, None, "defer is unattached history");
    let data: Value = serde_json::from_str(&rows[0].1).expect("history JSON");
    assert_eq!(data["path"], path.to_string_lossy().as_ref());
    assert_eq!(data["media_type"], "ebook");
    if let Some(error) = expected_error {
        assert_eq!(data["error"], error);
    }
}

fn strip_rust_comments(source: &str) -> String {
    let mut chars = source.chars().peekable();
    let mut output = String::with_capacity(source.len());
    let mut block_depth = 0usize;
    let mut in_line = false;
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_line {
            if ch == '\n' {
                in_line = false;
                output.push(ch);
            }
            continue;
        }
        if block_depth > 0 {
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_depth += 1;
            } else if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_depth -= 1;
            }
            continue;
        }
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            in_line = true;
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_depth = 1;
        } else {
            if ch == '"' {
                in_string = true;
            }
            output.push(ch);
        }
    }
    output
}

// RED-UNTIL-U2: today the handler runs AuthorService::add before the road, and the road parks an unattached ImportIdentity card instead of creating/importing the minimum-only Work.
#[tokio::test]
async fn irf_u2_a_valid_minimum_only_creates_zero_route_work_and_imports() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let source = incoming_path(&harness, "a-minimum.epub");
    write_epub(&source, "A Minimum Book");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![minimum_item(&source, "A Minimum Book", "A Minimum Author")],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "imported");
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    let work_id = response.json["results"][0]["workId"]
        .as_i64()
        .expect("created Work id");
    let captured = harness
        .db
        .read_captured_identity(harness.user_id, work_id)
        .await
        .expect("read zero-route Work");
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    assert!(captured.active_routes.is_empty());
    assert_eq!(captured.identity_generation, 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 1);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_name_variants").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_link_progress").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "editions").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        1
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;

    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].title, "A Minimum Book");
    assert_eq!(commands[0].author, "A Minimum Author");
    assert_eq!(commands[0].owned_file, owned_file_evidence(&source).await);
    assert_eq!(commands[0].request_start_work_hint, None);
    assert_eq!(commands[0].author_route, None);
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today every valid minimum-only row parks ImportIdentity after AuthorService::add, while invalid/unreadable rows can leave Author-side residue because validation is not seated before that writer.
#[tokio::test]
async fn irf_u2_b_mixed_batch_is_independent_and_invalid_rows_leave_only_history() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let valid_1 = incoming_path(&harness, "b-valid-1.epub");
    let blank_title = incoming_path(&harness, "b-blank-title.epub");
    let valid_2 = incoming_path(&harness, "b-valid-2.epub");
    let punctuation = incoming_path(&harness, "b-punctuation.epub");
    let valid_3 = incoming_path(&harness, "b-valid-3.epub");
    let blank_author = incoming_path(&harness, "b-blank-author.epub");
    let unreadable = incoming_path(&harness, "b-unreadable.epub");
    for (path, title) in [
        (&valid_1, "B Valid One"),
        (&blank_title, "Blank"),
        (&valid_2, "B Valid Two"),
        (&punctuation, "Punctuation"),
        (&valid_3, "B Valid Three"),
        (&blank_author, "Blank Author"),
    ] {
        write_epub(path, title);
    }
    assert!(
        !unreadable.exists(),
        "unreadable fixture is a missing .epub"
    );
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![
            minimum_item(&valid_1, "B Valid One", "B Author One"),
            minimum_item(&blank_title, "   ", "Blank Title Author"),
            minimum_item(&valid_2, "B Valid Two", "B Author Two"),
            minimum_item(&punctuation, "!!! ... ---", "Punctuation Author"),
            minimum_item(&valid_3, "B Valid Three", "B Author Three"),
            minimum_item(&blank_author, "Blank Author Book", "   "),
            minimum_item(&unreadable, "Unreadable Book", "Unreadable Author"),
        ],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let statuses = response.json["results"]
        .as_array()
        .expect("batch results")
        .iter()
        .map(|result| result["status"].as_str().unwrap_or("missing"))
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        vec!["imported", "failed", "imported", "failed", "imported", "failed", "failed"]
    );
    let paths = response.json["results"]
        .as_array()
        .expect("batch results")
        .iter()
        .map(|result| result["path"].as_str().expect("result path"))
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            &valid_1,
            &blank_title,
            &valid_2,
            &punctuation,
            &valid_3,
            &blank_author,
            &unreadable
        ]
        .map(|path| path.to_str().expect("UTF-8 fixture path"))
    );
    let after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(before, after, "the three valid rows must commit");
    assert!(files_before.is_empty());

    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 3);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_name_variants").await,
        3
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_link_progress").await,
        3
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_provider_routes").await,
        0
    );
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 3);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "work_contributors").await,
        3
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "work_contributor_roles").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_routes").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "editions").await,
        3
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        3
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        3
    );
    let generations = &after.generations;
    assert_eq!(generations.len(), 3);
    assert!(generations
        .iter()
        .all(|(_, identity_generation, merge_generation)| {
            *identity_generation == 1 && *merge_generation == 0
        }));

    for forbidden_author in [
        "Blank Title Author",
        "Punctuation Author",
        "Unreadable Author",
    ] {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE user_id=?1 AND name=?2")
                .bind(harness.user_id)
                .bind(forbidden_author)
                .fetch_one(harness.db.pool())
                .await
                .expect("check failed-row Author residue");
        assert_eq!(count, 0, "failed row created Author {forbidden_author}");
    }
    for forbidden_title in ["Blank Author Book", "Unreadable Book"] {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1 AND title=?2")
                .bind(harness.user_id)
                .bind(forbidden_title)
                .fetch_one(harness.db.pool())
                .await
                .expect("check failed-row Work residue");
        assert_eq!(count, 0, "failed row created Work {forbidden_title}");
    }
    for path in [&blank_title, &punctuation, &blank_author, &unreadable] {
        assert_one_unattached_failure(&harness.db, harness.user_id, path, None).await;
    }
    for result_index in [1, 3, 5, 6] {
        assert!(response.json["results"][result_index]["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()));
        assert_eq!(
            response.json["results"][result_index]["workId"],
            Value::Null
        );
        assert_eq!(response.json["results"][result_index]["mediaType"], "ebook");
    }
    let history_counts: (i64, i64) = sqlx::query_as(
        "SELECT SUM(event_type='imported'), SUM(event_type='importFailed') \
         FROM history WHERE user_id=?1",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read mixed history counts");
    assert_eq!(history_counts, (3, 4));
    assert_eq!(files_after.len(), 3);
    for (result_index, source) in [(0, &valid_1), (2, &valid_2), (4, &valid_3)] {
        let work_id = response.json["results"][result_index]["workId"]
            .as_i64()
            .expect("valid mixed-row Work id");
        assert_eq!(response.json["results"][result_index]["error"], Value::Null);
        assert_imported_copy(&harness, work_id, &library_root, source).await;
    }
    for source in [&blank_title, &punctuation, &blank_author] {
        assert!(
            source.is_file(),
            "invalid source remains untouched: {source:?}"
        );
    }
    assert!(!unreadable.exists());

    let commands = minimum_road_commands(&harness);
    assert_eq!(
        commands.len(),
        3,
        "invalid title/author/file preflight never enters the write transaction"
    );
    assert!(commands
        .iter()
        .all(|command| command.request_start_work_hint.is_none()));
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today the minimum-only parking arm accepts a marker-only title after AuthorService::add, leaving Author/card residue instead of rejecting it before the write seat.
#[tokio::test]
async fn irf_u2_b_marker_only_title_is_rejected_before_the_write_seat() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let source = incoming_path(&harness, "b-marker-only.epub");
    write_epub(&source, "Embedded Fixture Title");
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![minimum_item(&source, ": Book 3", "Marker Only Author")],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "failed");
    assert_eq!(response.json["results"][0]["workId"], Value::Null);
    assert!(response.json["results"][0]["error"]
        .as_str()
        .is_some_and(|error| !error.is_empty()));
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "marker-only validation precedes every Author/Work write"
    );
    assert_eq!(files_under(&library_root), files_before);
    assert_one_unattached_failure(&harness.db, harness.user_id, &source, None).await;
    assert!(minimum_road_commands(&harness).is_empty());
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today an exact request-start match reaches AuthorService::add and ordinary settle; it does not traverse the new hint-revalidating coordinator.
#[tokio::test]
async fn irf_u2_c1_request_start_exact_text_hint_attaches() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C1 Author").await;
    let work_id = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "C1 Exact Book",
        None,
    )
    .await;
    let source = incoming_path(&harness, "c1-exact.epub");
    write_epub(&source, "C1 Exact Book");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![minimum_item(&source, "C1 Exact Book", "C1 Author")],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "imported");
    assert_eq!(response.json["results"][0]["workId"], work_id);
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert!(!response.json.to_string().contains("Not imported:"));
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(work_generation(&harness.db, work_id).await, 2);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, Some(work_id));
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today both rows use the stale empty request-start snapshot, call AuthorService::add, and park unattached ImportIdentity cards instead of the second transaction seeing the first Work.
#[tokio::test]
async fn irf_u2_c2_second_file_of_new_book_attaches_from_live_group() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Audiobook).await;
    let first = incoming_path(&harness, "c2-disc-01.m4b");
    let second = incoming_path(&harness, "c2-disc-02.m4b");
    write_m4b(&first, b"disc-one");
    write_m4b(&second, b"disc-two");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![
            minimum_item(&first, "C2 One New Book", "C2 Author"),
            minimum_item(&second, "C2 One New Book", "C2 Author"),
        ],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "imported");
    assert_eq!(response.json["results"][1]["status"], "imported");
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert_eq!(response.json["results"][1]["error"], Value::Null);
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 2);
    let work_id = response.json["results"][0]["workId"]
        .as_i64()
        .expect("first Work id");
    assert_eq!(response.json["results"][1]["workId"], work_id);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(work_generation(&harness.db, work_id).await, 2);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "editions").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        2
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert!(
        first.exists() && second.exists(),
        "Copy preserves both sources"
    );
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 2);
    assert!(commands
        .iter()
        .all(|command| command.request_start_work_hint.is_none()));
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today the exact dedup hint reaches ordinary settle, whose complete-group reconciliation AutoMerges and absorbs the subtitle sibling; its Work row disappears instead of the hint being the one decision.
#[tokio::test]
async fn irf_u2_c3_hint_first_ignores_a_grey_sibling() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C3 Author").await;
    let exact_id = seed_work(&harness.db, harness.user_id, author_id, "Book", None).await;
    let grey_id = seed_work(&harness.db, harness.user_id, author_id, "Book: Tail", None).await;
    let grey_before = row_by_id_snapshot(&harness.db, "works", grey_id).await;
    let source = incoming_path(&harness, "c3-hint-first.epub");
    write_epub(&source, "Book");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(&harness, vec![minimum_item(&source, "Book", "C3 Author")]).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "imported");
    assert_eq!(response.json["results"][0]["workId"], exact_id);
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert!(!response.json.to_string().contains("Not imported:"));
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    assert_eq!(work_generation(&harness.db, exact_id).await, 2);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        3
    );
    assert_eq!(
        row_by_id_snapshot(&harness.db, "works", grey_id).await,
        grey_before,
        "the grey sibling is not consulted or mutated"
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_imported_copy(&harness, exact_id, &library_root, &source).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, Some(exact_id));
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today AuthorService::add re-arms the exact Author and attaches its supplied route before ordinary settle parks an unattached ImportIdentity card; the atomic coordinator never attaches the subtitle contender.
#[tokio::test]
async fn irf_u2_c4_one_member_subtitle_exact_author_attaches_with_route() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C4 Exact Author").await;
    let work_id = seed_work(&harness.db, harness.user_id, author_id, "Book", None).await;
    let source = incoming_path(&harness, "c4-exact-author-subtitle.epub");
    write_epub(&source, "Book: Tail");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let mut item = minimum_item(&source, "Book: Tail", "C4 Exact Author");
    item["authorOlKey"] = json!("OL4004A");
    let response = import_batch(&harness, vec![item]).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        response.json["results"][0]["status"], "imported",
        "{}",
        response.json
    );
    assert_eq!(response.json["results"][0]["workId"], work_id);
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert!(!response.json.to_string().contains("Not imported:"));
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_provider_routes").await,
        1
    );
    let route: (i64, String, String, String) = sqlx::query_as(
        "SELECT author_id, provider, route_value, state FROM author_provider_routes \
         WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read successfully attached Author route");
    assert_eq!(
        route,
        (
            author_id,
            "open_library".to_string(),
            "OL4004A".to_string(),
            "active".to_string(),
        )
    );
    assert_eq!(work_generation(&harness.db, work_id).await, 2);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        1
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, None);
    assert_eq!(
        commands[0].author_route,
        Some(
            livrarr_domain::AuthorRouteKey::parse(
                livrarr_domain::AuthorProvider::OpenLibrary,
                "OL4004A",
            )
            .expect("fixture Author route"),
        )
    );
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today AuthorService::add adopts/re-arms the unambiguous Author before ordinary settle parks an unattached ImportIdentity card; the atomic coordinator never performs the successful attach.
#[tokio::test]
async fn irf_u2_c4_one_member_subtitle_unambiguous_author_attaches_without_new_author() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "Robert A. Heinlein").await;
    let work_id = seed_work(&harness.db, harness.user_id, author_id, "Book", None).await;
    let source = incoming_path(&harness, "c4-unambiguous-author-subtitle.epub");
    write_epub(&source, "Book: Tail");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![minimum_item(&source, "Book: Tail", "Robert Heinlein")],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        response.json["results"][0]["status"], "imported",
        "{}",
        response.json
    );
    assert_eq!(response.json["results"][0]["workId"], work_id);
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert!(!response.json.to_string().contains("Not imported:"));
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 1);
    let contributor_author: i64 = sqlx::query_scalar(
        "SELECT author_id FROM work_contributors WHERE user_id=?1 AND work_id=?2",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read adopted Work contributor");
    assert_eq!(contributor_author, author_id);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(work_generation(&harness.db, work_id).await, 2);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        1
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].author, "Robert Heinlein");
    assert_eq!(commands[0].request_start_work_hint, None);
    assert_eq!(commands[0].author_route, None);
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today the exact distinguished Work reaches AuthorService::add plus ordinary settle rather than the coordinator's rule-1 hint bypass.
#[tokio::test]
async fn irf_u2_c4_audited_distinction_exact_hint_still_attaches() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C4 Distinguished Author").await;
    let work_id = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "Distinguished Book",
        Some("audited-distinction"),
    )
    .await;
    let source = incoming_path(&harness, "c4-distinguished-exact.epub");
    write_epub(&source, "Distinguished Book");
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![minimum_item(
            &source,
            "Distinguished Book",
            "C4 Distinguished Author",
        )],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["results"][0]["status"], "imported");
    assert_eq!(response.json["results"][0]["workId"], work_id);
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert!(!response.json.to_string().contains("Not imported:"));
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    assert_eq!(work_generation(&harness.db, work_id).await, 2);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, Some(work_id));
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today exact-name AuthorService::add re-arms the Author and attaches its supplied route before the unattached parking arm returns a wrapped failure; the no-hint audited distinction is not one atomic defer decision.
#[tokio::test]
async fn irf_u2_c4_audited_distinction_without_hint_exact_author_defers_without_residue() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C4 Audit Author").await;
    let work_id = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "Book",
        Some("audited-distinction"),
    )
    .await;
    let source = incoming_path(&harness, "c4-distinguished-no-hint.epub");
    write_epub(&source, "Book: Tail");
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let work_before = row_by_id_snapshot(&harness.db, "works", work_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);
    let expected = singular_defer("Book", "C4 Audit Author");

    let mut item = minimum_item(&source, "Book: Tail", "C4 Audit Author");
    item["authorOlKey"] = json!("OL4404A");
    let response = import_batch(&harness, vec![item]).await;

    assert_raw_failed_result(&response, &expected);
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "audited defer rolls back exact-hit update/re-arm and the supplied Author route"
    );
    assert_eq!(
        row_by_id_snapshot(&harness.db, "works", work_id).await,
        work_before,
        "the distinguished Work remains byte-identical"
    );
    assert_eq!(files_under(&library_root), files_before);
    assert!(source.is_file());
    assert_one_unattached_failure(&harness.db, harness.user_id, &source, Some(&expected)).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, None);
    assert_eq!(
        commands[0].author_route,
        Some(
            livrarr_domain::AuthorRouteKey::parse(
                livrarr_domain::AuthorProvider::OpenLibrary,
                "OL4404A",
            )
            .expect("fixture Author route"),
        )
    );
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today unambiguous AuthorService::add adopts/re-arms the existing Author before the unattached parking arm returns a wrapped failure; that Author-side residue is not rolled back with the audited-distinction defer.
#[tokio::test]
async fn irf_u2_c4_audited_distinction_without_hint_unambiguous_author_defers_without_adoption() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "Robert A. Heinlein").await;
    let work_id = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "Book",
        Some("audited-distinction"),
    )
    .await;
    let source = incoming_path(&harness, "c4-distinguished-unambiguous.epub");
    write_epub(&source, "Book: Tail");
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let work_before = row_by_id_snapshot(&harness.db, "works", work_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);
    let expected = singular_defer("Book", "Robert A. Heinlein");

    let response = import_batch(
        &harness,
        vec![minimum_item(&source, "Book: Tail", "Robert Heinlein")],
    )
    .await;

    assert_raw_failed_result(&response, &expected);
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "audited defer rolls back unambiguous Author adoption/re-arm"
    );
    assert_eq!(
        row_by_id_snapshot(&harness.db, "works", work_id).await,
        work_before,
        "the distinguished Work remains byte-identical"
    );
    assert_eq!(files_under(&library_root), files_before);
    assert!(source.is_file());
    assert_one_unattached_failure(&harness.db, harness.user_id, &source, Some(&expected)).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].author, "Robert Heinlein");
    assert_eq!(commands[0].request_start_work_hint, None);
    assert_eq!(commands[0].author_route, None);
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today AuthorService::add runs before ordinary settle parks an unattached ImportIdentity card, so the legal subtitle cohort never returns the exact plural defer without residue.
#[tokio::test]
async fn irf_u2_c5_legal_two_member_subtitle_cohort_defers_plural_stably() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C5 Author").await;
    seed_work(&harness.db, harness.user_id, author_id, "Book", None).await;
    seed_work(&harness.db, harness.user_id, author_id, "Book: Other", None).await;

    let source = incoming_path(&harness, "c5-legal-cohort.epub");
    write_epub(&source, "Book: Tail");
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);
    let expected = plural_defer(&[("Book", "C5 Author"), ("Book: Other", "C5 Author")]);

    let response = import_batch(
        &harness,
        vec![minimum_item(&source, "Book: Tail", "C5 Author")],
    )
    .await;

    assert_raw_failed_result(&response, &expected);
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "plural defer absorbs nothing and writes no state"
    );
    assert_eq!(files_under(&library_root), files_before);
    assert!(source.is_file());
    assert_one_unattached_failure(&harness.db, harness.user_id, &source, Some(&expected)).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, None);
    assert_no_provider_http(&harness);
}

async fn run_begin_immediate_order(first_title: &str, second_title: &str, label: &str) {
    let harness = build_concurrent_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let first_source = incoming_path(&harness, &format!("c6-{label}-first.epub"));
    let second_source = incoming_path(&harness, &format!("c6-{label}-second.epub"));
    write_epub(&first_source, first_title);
    write_epub(&second_source, second_title);
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    // This test hook only pauses the production transaction after its real
    // authoritative reads and reports before/after the production
    // `pool::begin_write`. It neither acquires nor substitutes a lock.
    let interleaving =
        livrarr_db::identity_layer::install_manual_import_minimum_interleaving_for_tests(
            harness.user_id,
            vec![first_title.to_string(), second_title.to_string()],
        );

    let first_app = harness.app.clone();
    let first_key = harness.api_key.clone();
    let first_body = json!({
        "items": [minimum_item(&first_source, first_title, "C6 Author")]
    });
    let first_task = tokio::spawn(async move {
        call_router_json(
            &first_app,
            &first_key,
            Method::POST,
            "/api/v1/manualimport/import",
            Some(first_body),
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        interleaving.wait_after_authoritative_reads(first_title),
    )
    .await
    .expect("first request reached the production after-read pause");
    assert!(
        interleaving.begin_acquired(first_title),
        "the first request owns the production BEGIN IMMEDIATE transaction"
    );

    let second_app = harness.app.clone();
    let second_key = harness.api_key.clone();
    let second_body = json!({
        "items": [minimum_item(&second_source, second_title, "C6 Author")]
    });
    let second_task = tokio::spawn(async move {
        call_router_json(
            &second_app,
            &second_key,
            Method::POST,
            "/api/v1/manualimport/import",
            Some(second_body),
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        interleaving.wait_before_begin(second_title),
    )
    .await
    .expect("second request attempted the production BEGIN IMMEDIATE");
    tokio::task::yield_now().await;
    assert!(
        !interleaving.begin_acquired(second_title),
        "the contender is waiting at BEGIN IMMEDIATE while the first production transaction owns SQLite's writer slot"
    );

    interleaving.release_after_authoritative_reads(first_title);
    tokio::time::timeout(
        Duration::from_secs(5),
        interleaving.wait_after_authoritative_reads(second_title),
    )
    .await
    .expect("contender acquired BEGIN and read the first committed group");
    assert!(interleaving.begin_acquired(second_title));

    // At this point request one has committed its Author/Work settlement and
    // request two has performed reads only. Snapshot the sole winning Work
    // before releasing the contender's attach decision.
    let winning_work_id: i64 =
        sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id LIMIT 1")
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("first committed Work is visible to the contender");
    let winning_generation = work_generation(&harness.db, winning_work_id).await;

    interleaving.release_after_authoritative_reads(second_title);
    let (first_response, second_response) = tokio::time::timeout(Duration::from_secs(10), async {
        (
            first_task.await.expect("first route task"),
            second_task.await.expect("second route task"),
        )
    })
    .await
    .expect("interleaved import routes completed");

    assert_eq!(
        first_response.status,
        StatusCode::OK,
        "{}",
        first_response.json
    );
    assert_eq!(first_response.json["results"][0]["status"], "imported");
    assert_eq!(first_response.json["results"][0]["workId"], winning_work_id);
    assert_eq!(first_response.json["results"][0]["error"], Value::Null);
    assert_eq!(
        second_response.status,
        StatusCode::OK,
        "{}",
        second_response.json
    );
    assert_eq!(second_response.json["results"][0]["status"], "imported");
    assert_eq!(
        second_response.json["results"][0]["workId"],
        winning_work_id
    );
    assert_eq!(second_response.json["results"][0]["error"], Value::Null);
    assert!(!first_response.json.to_string().contains("Not imported:"));
    assert!(!second_response.json.to_string().contains("Not imported:"));
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 2);
    assert_eq!(
        work_generation(&harness.db, winning_work_id).await,
        winning_generation + 1,
        "the contender attaches through a second audited settlement"
    );
    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "work_contributors").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "editions").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2,
        "both the create and the contender's attach are audited"
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        2
    );
    for table in [
        "work_contributor_roles",
        "work_identity_review_candidates",
        "identity_routes",
        "identity_conflicts_v2",
        "identity_route_archives",
        "identity_merge_archives",
        "embedded_cover_inspections",
    ] {
        assert_eq!(
            user_count(&harness.db, harness.user_id, table).await,
            0,
            "the two successful settlements leave no unrelated state in {table}"
        );
    }
    assert_imported_copy(&harness, winning_work_id, &library_root, &first_source).await;
    assert_imported_copy(&harness, winning_work_id, &library_root, &second_source).await;
    let history_counts: (i64, i64) = sqlx::query_as(
        "SELECT SUM(event_type='imported'), SUM(event_type='importFailed') \
         FROM history WHERE user_id=?1",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read interleaved import history counts");
    assert_eq!(history_counts, (2, 0));
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 2);
    assert!(commands
        .iter()
        .all(|command| command.request_start_work_hint.is_none()));
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today there is no coordinator BEGIN IMMEDIATE spanning Author/group reads and settlement; the route never reaches the production observation hook, so Book-then-subtitle cannot prove queue-then-attach.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn irf_u2_c6_begin_immediate_serializes_book_then_subtitle() {
    run_begin_immediate_order("C6 Forward Book", "C6 Forward Book: Tail", "forward").await;
}

// RED-UNTIL-U2: today there is no coordinator BEGIN IMMEDIATE spanning Author/group reads and settlement; the route never reaches the production observation hook, so subtitle-then-main cannot prove reverse-order queue-then-attach.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn irf_u2_c6_begin_immediate_serializes_subtitle_then_book() {
    run_begin_immediate_order("C6 Reverse Book: Tail", "C6 Reverse Book", "reverse").await;
}

fn ordinary_manual_request(
    user_id: i64,
    author_id: i64,
    title: &str,
    evidence: ilr::OwnedFileEvidence,
) -> ilr::IdentityRoadRequest {
    let minimum = ilr::MinimumWorkEvidence {
        title: title.to_string(),
        authors: vec![author_id],
    };
    ilr::IdentityRoadRequest {
        user_id,
        origin: ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ManualImport),
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: Some(ilr::UserIdentityChoice::ExplicitCreate(minimum.clone())),
            owned_files: vec![evidence],
            provider_identity: vec![],
            minimum: Some(minimum),
        },
        interaction: ilr::IdentityRoadInteraction::HumanWatching,
        existing_work_id: None,
    }
}

// RED-UNTIL-U2: today ordinary settle's first arm parks an unattached ImportIdentity card before the audited-distinction Review or legal two-member AutoMerge reconciliation; neither reaches the required Deferred backstop.
#[tokio::test]
async fn irf_u2_c7_ordinary_settle_is_a_no_card_no_absorption_backstop() {
    let review = build_route_harness().await;
    let review_root = configure_root(&review, MediaType::Ebook).await;
    let review_author = seed_author(&review.db, review.user_id, "C7 Review Author").await;
    seed_work(
        &review.db,
        review.user_id,
        review_author,
        "Book",
        Some("audited-distinction"),
    )
    .await;
    let review_source = incoming_path(&review, "c7-audited-review.epub");
    write_epub(&review_source, "Book: Tail");
    let review_before = user_state_snapshot(&review.db, review.user_id).await;
    let review_files_before = files_under(&review_root);
    clear_road_calls(&review);
    let review_expected = singular_defer("Book", "C7 Review Author");
    // Call ordinary `settle` directly as a compatibility/backstop fixture.
    let review_outcome = review
        .state
        .identity_road
        .settle(ordinary_manual_request(
            review.user_id,
            review_author,
            "Book: Tail",
            owned_file_evidence(&review_source).await,
        ))
        .await
        .expect("ordinary audited-distinction backstop outcome");
    assert_deferred(review_outcome, &review_expected);
    assert_eq!(
        user_state_snapshot(&review.db, review.user_id).await,
        review_before,
        "ordinary Review backstop commits no card or audit"
    );
    assert_eq!(files_under(&review_root), review_files_before);
    assert!(review_source.is_file());
    let review_calls = review.state.identity_road.test_recorder().snapshot();
    assert!(matches!(
        review_calls.as_slice(),
        [IdentityRoadCall::Settle(_)]
    ));
    assert_no_provider_http(&review);

    let multiple = build_route_harness().await;
    let multiple_root = configure_root(&multiple, MediaType::Ebook).await;
    let multiple_author = seed_author(&multiple.db, multiple.user_id, "C7 Multi Author").await;
    seed_work(
        &multiple.db,
        multiple.user_id,
        multiple_author,
        "Book",
        None,
    )
    .await;
    seed_work(
        &multiple.db,
        multiple.user_id,
        multiple_author,
        "Book: Other",
        None,
    )
    .await;
    let multiple_source = incoming_path(&multiple, "c7-legal-cohort.epub");
    write_epub(&multiple_source, "Book: Tail");
    let multiple_before = user_state_snapshot(&multiple.db, multiple.user_id).await;
    let multiple_files_before = files_under(&multiple_root);
    clear_road_calls(&multiple);
    let multiple_expected = plural_defer(&[
        ("Book", "C7 Multi Author"),
        ("Book: Other", "C7 Multi Author"),
    ]);
    // Call ordinary `settle` directly as a compatibility/backstop fixture.
    let multiple_outcome = multiple
        .state
        .identity_road
        .settle(ordinary_manual_request(
            multiple.user_id,
            multiple_author,
            "Book: Tail",
            owned_file_evidence(&multiple_source).await,
        ))
        .await
        .expect("ordinary 2+ backstop outcome");
    assert_deferred(multiple_outcome, &multiple_expected);
    assert_eq!(
        user_state_snapshot(&multiple.db, multiple.user_id).await,
        multiple_before,
        "ordinary backstop neither commits a card nor consumes an absorption list"
    );
    assert_eq!(files_under(&multiple_root), multiple_files_before);
    assert!(multiple_source.is_file());
    let multiple_calls = multiple.state.identity_road.test_recorder().snapshot();
    assert!(matches!(
        multiple_calls.as_slice(),
        [IdentityRoadCall::Settle(_)]
    ));
    assert_no_provider_http(&multiple);
}

// PIN: a provider-bearing ISBN item remains on ordinary settle with the pre-U2 request, result, Work, route, card, generation, LibraryItem, and file behavior.
#[tokio::test]
async fn irf_u2_c9_provider_bearing_isbn_is_byte_identical_and_out_of_scope() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "C9 Provider Author").await;
    let author_before = row_by_id_snapshot(&harness.db, "authors", author_id).await;
    let variants_before =
        table_snapshot(&harness.db, harness.user_id, "author_name_variants").await;
    let trigger_before: String = sqlx::query_scalar(
        "SELECT trigger FROM author_link_progress WHERE user_id=?1 AND author_id=?2",
    )
    .bind(harness.user_id)
    .bind(author_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read pre-U2 provider-pin Author task");
    assert_eq!(trigger_before, "author_created");
    let work_id = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "C9 Provider Book",
        None,
    )
    .await;
    let work_before = row_by_id_snapshot(&harness.db, "works", work_id).await;
    let generation_before = work_generation(&harness.db, work_id).await;
    let source = incoming_path(&harness, "c9-provider.epub");
    write_epub(&source, "C9 Provider Book");
    let expected_owned_file = owned_file_evidence(&source).await;
    let state_before = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_before = files_under(&library_root);
    clear_road_calls(&harness);

    let response = import_batch(
        &harness,
        vec![isbn_item(&source, "C9 Provider Book", "C9 Provider Author")],
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        response.json,
        json!({
            "results": [{
                "path": source,
                "status": "imported",
                "workId": work_id,
                "error": null,
                "mediaType": "ebook"
            }]
        }),
        "the provider-bearing response is the recorded pre-U2 shape"
    );
    let state_after = user_state_snapshot(&harness.db, harness.user_id).await;
    let files_after = files_under(&library_root);
    assert_ne!(state_after, state_before);
    assert!(files_before.is_empty());
    assert_eq!(files_after.len(), 1);
    let calls = harness.state.identity_road.test_recorder().snapshot();
    let [IdentityRoadCall::Settle(request)] = calls.as_slice() else {
        panic!("provider-bearing ISBN must make exactly one ordinary settle call: {calls:?}");
    };
    assert_eq!(
        request.origin,
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ManualImport)
    );
    assert_eq!(request.existing_work_id, Some(work_id));
    assert_eq!(request.evidence.minimum, None);
    assert_eq!(
        request.evidence.owned_files,
        vec![expected_owned_file.clone()]
    );
    assert!(matches!(
        request.evidence.user_choice,
        Some(ilr::UserIdentityChoice::ExistingWork(id)) if id == work_id
    ));
    assert_eq!(request.evidence.provider_identity.len(), 1);
    assert_eq!(
        request.evidence.provider_identity[0].provider,
        ilr::IdentityProvider::IsbnRegistry
    );
    assert_eq!(
        request.evidence.provider_identity[0].route.kind,
        ilr::RouteKind::Isbn13Edition
    );
    assert_eq!(
        request.evidence.provider_identity[0].route.value,
        ISBN_FIXTURE
    );

    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 1);
    assert_eq!(
        row_by_id_snapshot(&harness.db, "authors", author_id).await,
        author_before,
        "ordinary provider import retains the exact-hit Author row"
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_name_variants").await,
        1
    );
    assert_eq!(
        table_snapshot(&harness.db, harness.user_id, "author_name_variants").await,
        variants_before
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_link_progress").await,
        1
    );
    let trigger_after: String = sqlx::query_scalar(
        "SELECT trigger FROM author_link_progress WHERE user_id=?1 AND author_id=?2",
    )
    .bind(harness.user_id)
    .bind(author_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read post-import provider-pin Author task");
    assert_eq!(
        trigger_after, "author_adopted",
        "provider-bearing import keeps today's exact-hit re-arm behavior"
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "author_provider_routes").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "work_contributors").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_routes").await,
        1
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "editions").await,
        1
    );
    assert_ne!(
        row_by_id_snapshot(&harness.db, "works", work_id).await,
        work_before,
        "ordinary provider settlement claims its existing generation as before U2"
    );
    assert_eq!(
        work_generation(&harness.db, work_id).await,
        generation_before + 1
    );
    let captured = harness
        .db
        .read_captured_identity(harness.user_id, work_id)
        .await
        .expect("read provider-bearing identity");
    assert_eq!(captured.own_work_id, work_id);
    assert_eq!(captured.primary_author_id, author_id);
    assert_eq!(captured.identity_title.main, "C9 Provider Book");
    assert_eq!(captured.identity_title.subtitle, None);
    assert_eq!(captured.text_distinction, "common");
    assert_eq!(captured.identity_generation, generation_before + 1);
    assert_eq!(captured.status, ilr::IdentityStatus::Connected);
    assert!(captured.active_routes.iter().any(|route| {
        route.provider == ilr::IdentityProvider::IsbnRegistry
            && route.kind == ilr::RouteKind::Isbn13Edition
            && route.provider_scoped_id == ISBN_FIXTURE
            && !route.user_confirmed
            && matches!(
                &route.provenance,
                ilr::RouteProvenance::OwnedFile {
                    library_item_id: Some(0),
                    file_revision
                } if *file_revision == expected_owned_file.file_revision
            )
    }));
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_audit_events").await,
        2,
        "the seed and the unchanged provider settlement each have one audit"
    );
    for table in [
        "work_identity_review_candidates",
        "identity_conflicts_v2",
        "identity_route_archives",
        "identity_merge_archives",
    ] {
        assert_eq!(user_count(&harness.db, harness.user_id, table).await, 0);
    }
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        1
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;
    assert_no_provider_http(&harness);
}

// RED-UNTIL-U2: today `commit_unattached_import_review` and the minimum-only ManualImport arm still exist and are the production writer that parks ImportIdentity.
#[test]
fn irf_u2_c9_unattached_import_writer_and_sole_road_arm_are_removed() {
    let domain = strip_rust_comments(include_str!(
        "../../crates/livrarr-domain/src/identity_layer/services.rs"
    ));
    let db = strip_rust_comments(include_str!(
        "../../crates/livrarr-db/src/identity_layer.rs"
    ));
    let road = strip_rust_comments(include_str!(
        "../../crates/livrarr-metadata/src/identity_road.rs"
    ));
    for (file, source) in [
        ("livrarr-domain identity services", domain),
        ("livrarr-db identity repository", db),
        ("livrarr-metadata identity road", road),
    ] {
        assert!(
            !source.contains("commit_unattached_import_review"),
            "{file} still exposes the deleted unattached-import writer"
        );
    }
}

// RED-UNTIL-U2-FIX: today the exact-hint attach skips apply_existing_author_effects_on (apply_author=false), so the re-arm and the supplied route are missing.
#[tokio::test]
async fn irf_u2_fix_pin_exact_hint_attach_applies_author_effects_atomically() {
    let harness = build_route_harness().await;
    let library_root = configure_root(&harness, MediaType::Ebook).await;
    let author_id = seed_author(&harness.db, harness.user_id, "Hint Attach Author").await;
    let work_id = seed_work(&harness.db, harness.user_id, author_id, "Book", None).await;
    let source = incoming_path(&harness, "fix-pin-exact-hint.epub");
    write_epub(&source, "Book");
    clear_road_calls(&harness);

    let mut item = minimum_item(&source, "Book", "Hint Attach Author");
    item["authorOlKey"] = json!("OL9002A");
    let response = import_batch(&harness, vec![item]).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        response.json["results"][0]["status"], "imported",
        "{}",
        response.json
    );
    assert_eq!(response.json["results"][0]["workId"], work_id);
    assert_eq!(response.json["results"][0]["error"], Value::Null);
    assert_eq!(user_count(&harness.db, harness.user_id, "works").await, 1);
    assert_eq!(user_count(&harness.db, harness.user_id, "authors").await, 1);
    assert_eq!(
        user_count(&harness.db, harness.user_id, "identity_review_cards").await,
        0
    );
    assert_eq!(
        user_count(&harness.db, harness.user_id, "library_items").await,
        1
    );
    assert_imported_copy(&harness, work_id, &library_root, &source).await;
    let commands = minimum_road_commands(&harness);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].request_start_work_hint, Some(work_id));
    assert_eq!(
        commands[0].author_route,
        Some(
            livrarr_domain::AuthorRouteKey::parse(
                livrarr_domain::AuthorProvider::OpenLibrary,
                "OL9002A",
            )
            .expect("fixture Author route"),
        )
    );
    assert_no_provider_http(&harness);

    let trigger: String = sqlx::query_scalar(
        "SELECT trigger FROM author_link_progress WHERE user_id=?1 AND author_id=?2",
    )
    .bind(harness.user_id)
    .bind(author_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read exact-hint Author task");
    let routes: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT author_id, provider, route_value, state FROM author_provider_routes \
         WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("read exact-hint Author routes");
    assert_eq!(
        (trigger, routes),
        (
            "author_adopted".to_string(),
            vec![(
                author_id,
                "open_library".to_string(),
                "OL9002A".to_string(),
                "active".to_string(),
            )],
        ),
        "the successful exact-hint attach commits both Author-side effects"
    );
}

// PIN: the dependency-neutral formatter and ordering helper use Unicode `to_lowercase` keys, with Work id as the final tie-breaker.
#[test]
fn irf_u2_fix_pin_defer_order_uses_lowercase_keys_then_work_id() {
    // This calls pure domain functions over hand-built members: no persistence
    // participates in either ordering or rendering, so a DB harness would add
    // no observable behavior to this pin.
    fn member(work_id: i64, title: &str, author: &str) -> ilr::CompleteGroupMember {
        ilr::CompleteGroupMember {
            work_id,
            display_title: title.to_string(),
            display_author: author.to_string(),
            identity: ilr::CapturedIdentity {
                user_id: 1,
                own_work_id: work_id,
                identity_title: ilr::title_parts_from_provider(title.to_string(), None)
                    .expect("fixture title"),
                primary_author_id: 1,
                text_distinction: "common".to_string(),
                active_routes: vec![],
                status: ilr::IdentityStatus::NotConnected,
                identity_generation: 0,
            },
        }
    }

    let lowercase_key_members = vec![
        member(1, "Straße", "Case Author"),
        member(2, "Strasse", "Case Author"),
    ];
    assert_eq!(
        ilr::format_manual_import_defer(&lowercase_key_members).0,
        "Not imported: this title matches multiple existing books: (1) \"Strasse\" by Case Author; (2) \"Straße\" by Case Author. Manual import cannot choose among them. Choose Edit title and author for this row and enter a different main title to create a different book (a different subtitle alone is not enough)."
    );

    let descending_id_members = vec![
        member(90, "Identical Book", "Identical Author"),
        member(7, "Identical Book", "Identical Author"),
    ];
    assert_eq!(
        ilr::format_manual_import_defer(&descending_id_members).0,
        "Not imported: this title matches multiple existing books: (1) \"Identical Book\" by Identical Author; (2) \"Identical Book\" by Identical Author. Manual import cannot choose among them. Choose Edit title and author for this row and enter a different main title to create a different book (a different subtitle alone is not enough)."
    );
    let mut directly_ordered = descending_id_members;
    ilr::order_complete_group_members(&mut directly_ordered);
    assert_eq!(
        directly_ordered
            .iter()
            .map(|member| member.work_id)
            .collect::<Vec<_>>(),
        vec![7, 90],
        "equal lowercase title+author keys are ordered by ascending Work id"
    );
}
