//! RED-first behavioral coverage for identity-review-fixes U1 (REQ-001 / AC-001).
//!
//! This is a standalone `[[test]]` target. HTTP cases traverse the production
//! router and authentication middleware, all state is a real `SqliteDb` from
//! `create_test_db()` (SQLite `:memory:`), and the CLI cases invoke both the
//! production command function and the real `livrarr` binary. U1 has no
//! provider-I/O dependency, so this harness deliberately registers no provider
//! clients or scripted provider transport.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDbCreate,
};
use livrarr_domain::identity::{AnchorType, ConflictResolutionAction, IncomingConflictPayload};
use livrarr_domain::identity_layer::{self as ilr};
use livrarr_domain::identity_layer::{
    EditionRepository, IdentityRoadService, ReviewActor, ReviewResolutionCommand,
    WorkIdentityRepository,
};
use livrarr_domain::UserRole;
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::state::AppState;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

static CASE_ID: AtomicU64 = AtomicU64::new(1);
static LIVRARR_BINARY: OnceLock<PathBuf> = OnceLock::new();

// The PO stopped merge development on 2026-09-07. This exercises the real
// authenticated HTTP doors; a read-only refusal must not mint a review card.
#[tokio::test]
async fn containment_manual_merge_is_read_only_unavailable() {
    let harness = build_route_harness().await;
    let (survivor, _) = seed_work(&harness.db, harness.user_id, "contained-survivor").await;
    let (loser, _) = seed_work(&harness.db, harness.user_id, "contained-loser").await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    for (method, suffix, body) in [
        (Method::GET, "/preview", None),
        (Method::POST, "", Some(json!({"choices": []}))),
    ] {
        let response = call_router_json(
            &harness,
            method,
            format!("/api/v1/work/{survivor}/merge/{loser}{suffix}"),
            body,
        )
        .await;
        assert!(
            response.status.is_client_error(),
            "{} {}",
            response.status,
            response.json
        );
        assert!(
            response
                .json
                .to_string()
                .contains("Merging is currently unavailable"),
            "{}",
            response.json
        );
        assert_eq!(
            user_state_snapshot(&harness.db, harness.user_id).await,
            before
        );
    }
}

struct RouteHarness {
    app: Router,
    state: AppState,
    api_key: String,
    db: SqliteDb,
    user_id: i64,
    _tmp: tempfile::TempDir,
}

struct RouteResponse {
    status: StatusCode,
    json: Value,
}

async fn build_route_harness() -> RouteHarness {
    // Packet law: this is the real, migrated, single-connection SQLite
    // `:memory:` helper. U1 does not need the activated-index-only variant.
    build_route_harness_on(create_test_db().await).await
}

/// Production-shaped schema: the activated identity index without the legacy
/// test-only Work index, so identity uniqueness behaves as it does live.
async fn build_activated_route_harness() -> RouteHarness {
    build_route_harness_on(livrarr_db::test_helpers::create_activated_test_db().await).await
}

async fn build_route_harness_on(db: SqliteDb) -> RouteHarness {
    let tmp = tempfile::tempdir().expect("U1 route harness tempdir");
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = "irf-u1-admin-api-key".to_string();
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash U1 API key");
    let user = db
        .create_user(CreateUserDbRequest {
            username: "irf-u1-admin".to_string(),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create authenticated U1 user");

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
    let http_fetcher = livrarr_http::fetcher::HttpFetcherImpl::new().expect("shared HTTP fetcher");
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

    // Empty maps/queues are production implementations with no registered
    // provider. Nothing in U1 may cross a provider HTTP boundary.
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
        api_key,
        db,
        user_id: user.id,
        _tmp: tmp,
    }
}

async fn call_router_json(
    harness: &RouteHarness,
    method: Method,
    path: impl Into<String>,
    body: Option<Value>,
) -> RouteResponse {
    call_app_json(harness.app.clone(), &harness.api_key, method, path, body).await
}

async fn call_app_json(
    app: Router,
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
    let mut request = request.body(request_body).expect("build U1 request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 77)),
        31_001,
    )));
    let response = app
        .oneshot(request)
        .await
        .expect("production router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read response body");
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"unparsedBody": String::from_utf8_lossy(&bytes).into_owned()}));
    RouteResponse { status, json }
}

async fn seed_user(db: &SqliteDb, label: &str) -> i64 {
    let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    db.create_user(CreateUserDbRequest {
        username: format!("irf-u1-{label}-{n}"),
        password_hash: "unused".to_string(),
        role: UserRole::Admin,
        api_key_hash: format!("unused-u1-{label}-{n}"),
    })
    .await
    .expect("seed U1 user")
    .id
}

async fn seed_work(db: &SqliteDb, user_id: i64, label: &str) -> (i64, i64) {
    let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    let title = format!("U1 {label} {n}");
    let author_name = format!("U1 Author {label} {n}");
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: author_name.clone(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed U1 Author");
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: title.clone(),
            author_name,
            normalized_title: title.to_ascii_lowercase(),
            normalized_author: format!("u1 author {label} {n}").to_ascii_lowercase(),
            author_id: Some(author.id),
            language: Some("en".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed U1 Work");
    (work.id, author.id)
}

async fn work_generation(db: &SqliteDb, work_id: i64) -> i64 {
    sqlx::query_scalar("SELECT identity_generation FROM works WHERE id=?1")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("read Work generation")
}

fn title_tuple(label: &str) -> ilr::IdentityTitleTuple {
    ilr::IdentityTitleTuple {
        main: label.to_string(),
        subtitle: None,
        volume: None,
        normalized_main: label.to_ascii_lowercase(),
        normalized_subtitle: String::new(),
        normalized_volume: String::new(),
        provenance: ilr::EvidenceProvenance::User,
    }
}

fn settlement_commit(
    user_id: i64,
    author_id: i64,
    label: &str,
    card: ilr::SettlementReviewCard,
) -> ilr::SettlementCommit {
    ilr::SettlementCommit {
        creation_facts: None,
        user_id,
        existing_work_id: None,
        add_source: None,
        identity_title: title_tuple(label),
        text_distinction: None,
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
        review_cards: vec![card],
    }
}

#[derive(Clone, Debug)]
struct MintedRefusal {
    kind: ilr::ReviewKind,
    card_id: i64,
    generation: i64,
    work_id: Option<i64>,
}

async fn pending_card(db: &SqliteDb, user_id: i64, kind: ilr::ReviewKind) -> MintedRefusal {
    let (card_id, generation, work_id): (i64, i64, Option<i64>) = sqlx::query_as(
        "SELECT id, generation, work_id FROM identity_review_cards \
           WHERE user_id=?1 AND kind=?2 AND status='pending' ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(kind.storage_code())
    .fetch_one(db.pool())
    .await
    .expect("read minted refusal card");
    MintedRefusal {
        kind,
        card_id,
        generation,
        work_id,
    }
}

async fn mint_refused_card(db: &SqliteDb, user_id: i64, kind: ilr::ReviewKind) -> MintedRefusal {
    match kind {
        ilr::ReviewKind::EditionEvidence => {
            // Production mint: contradictory Work-level edition evidence is
            // one of the two live EditionEvidence writers named by ST-007.
            // U7 keeps this refused-card fixture; it is not a U5 suppression test.
            let (work_id, _) = seed_work(db, user_id, "edition-evidence").await;
            EditionRepository::apply_work_evidence(
                db,
                ilr::EditionWorkEvidenceCommand {
                    user_id,
                    work_id,
                    format: ilr::EditionFormat::Ebook,
                    language: Some("en".to_string()),
                    provenance: ilr::EvidenceProvenance::OwnedFile,
                },
            )
            .await
            .expect("seed active Edition through production writer");
            let contradiction = EditionRepository::apply_work_evidence(
                db,
                ilr::EditionWorkEvidenceCommand {
                    user_id,
                    work_id,
                    format: ilr::EditionFormat::Audiobook,
                    language: Some("en".to_string()),
                    provenance: ilr::EvidenceProvenance::OwnedFile,
                },
            )
            .await;
            assert!(matches!(
                contradiction,
                Err(ilr::EditionRepositoryError::ContradictoryEvidenceParked)
            ));
            pending_card(db, user_id, kind).await
        }
        ilr::ReviewKind::ImportIdentity => {
            // After U2, ImportIdentity has no production producer and this
            // card exists only to preserve U1's continuation-refusal coverage.
            let (_, author_id) = seed_work(db, user_id, "import-identity").await;
            let evidence = ilr::IdentityEvidenceBundle {
                user_choice: None,
                owned_files: vec![],
                provider_identity: vec![],
                minimum: Some(ilr::MinimumWorkEvidence {
                    title: "U1 parked import".to_string(),
                    authors: vec![],
                }),
            };
            let card = ilr::SettlementReviewCard::ImportIdentity {
                work_id: None,
                evidence,
            };
            let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
            let committed = WorkIdentityRepository::commit_settlement(
                db,
                settlement_commit(user_id, author_id, &format!("U1 import identity {n}"), card),
            )
            .await
            .expect("commit serialized compatibility ImportIdentity card");
            let minted = committed.review_cards[0];
            MintedRefusal {
                kind,
                card_id: minted.id,
                generation: minted.generation,
                work_id: Some(committed.identity.own_work_id),
            }
        }
        ilr::ReviewKind::IdentityConflict
        | ilr::ReviewKind::FieldResolution
        | ilr::ReviewKind::ContributorOrder
        | ilr::ReviewKind::MigrationRepair
        | ilr::ReviewKind::InvariantRepair => {
            // AC-001 compatibility fixture: these kinds have no production
            // mint door today (ST-007). The fixture uses the production
            // SettlementReviewCard payload and sole settlement writer.
            let (_, author_id) = seed_work(db, user_id, "compat-principal").await;
            let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
            let card = match kind {
                ilr::ReviewKind::IdentityConflict => ilr::SettlementReviewCard::IdentityConflict {
                    conflict_id: 700_000 + n as i64,
                    work_id: 0,
                },
                ilr::ReviewKind::FieldResolution => ilr::SettlementReviewCard::FieldResolution {
                    work_id: 0,
                    evidence_ids: vec![17],
                },
                ilr::ReviewKind::ContributorOrder => ilr::SettlementReviewCard::ContributorOrder {
                    work_id: 0,
                    contributors: vec![],
                },
                ilr::ReviewKind::MigrationRepair => ilr::SettlementReviewCard::MigrationRepair {
                    legacy_key: format!("legacy-u1-{n}"),
                    reason: "AC-001 compatibility fixture".to_string(),
                },
                ilr::ReviewKind::InvariantRepair => ilr::SettlementReviewCard::InvariantRepair {
                    work_id: None,
                    invariant: "AC-001 compatibility fixture".to_string(),
                },
                _ => unreachable!(),
            };
            let committed = WorkIdentityRepository::commit_settlement(
                db,
                settlement_commit(user_id, author_id, &format!("U1 compatibility {n}"), card),
            )
            .await
            .expect("commit serialized compatibility card");
            let minted = committed.review_cards[0];
            MintedRefusal {
                kind,
                card_id: minted.id,
                generation: minted.generation,
                work_id: Some(committed.identity.own_work_id),
            }
        }
        ilr::ReviewKind::PendingRoute | ilr::ReviewKind::GroupIdentity => {
            panic!("available kind passed to refused-card fixture")
        }
    }
}

fn refusal_command(card: &MintedRefusal) -> ReviewResolutionCommand {
    match card.kind {
        ilr::ReviewKind::IdentityConflict => ReviewResolutionCommand::IdentityConflict {
            card_id: card.card_id,
            expected_generation: card.generation,
            action: ilr::IdentityConflictResolution::Reject {
                surviving_routes: vec![],
            },
        },
        ilr::ReviewKind::FieldResolution => ReviewResolutionCommand::FieldResolution {
            card_id: card.card_id,
            expected_generation: card.generation,
            action: ilr::FieldResolutionAction::ExplicitAbsence,
        },
        ilr::ReviewKind::ContributorOrder => {
            let author = ilr::AuthorRef("u1-author".to_string());
            ReviewResolutionCommand::ContributorOrder {
                card_id: card.card_id,
                expected_generation: card.generation,
                partition: vec![],
                order: vec![author.clone()],
                primary: author,
            }
        }
        ilr::ReviewKind::EditionEvidence => ReviewResolutionCommand::EditionEvidence {
            card_id: card.card_id,
            expected_generation: card.generation,
            action: ilr::EditionEvidenceAction::RetainUnknownOrAbsent,
        },
        ilr::ReviewKind::ImportIdentity => ReviewResolutionCommand::ImportIdentity {
            card_id: card.card_id,
            expected_generation: card.generation,
            action: ilr::ImportIdentityAction::CorrectedMetadataRetry {
                evidence: ilr::IdentityEvidenceBundle {
                    user_choice: None,
                    owned_files: vec![],
                    provider_identity: vec![],
                    minimum: None,
                },
            },
        },
        ilr::ReviewKind::MigrationRepair => ReviewResolutionCommand::MigrationRepair {
            card_id: card.card_id,
            expected_generation: card.generation,
            action: ilr::MigrationRepairAction::DiscardProvenNonIdentity {
                reason: "U1 refusal".to_string(),
            },
        },
        ilr::ReviewKind::InvariantRepair => ReviewResolutionCommand::InvariantRepair {
            card_id: card.card_id,
            expected_generation: card.generation,
            action: ilr::InvariantRepairAction::Recompute,
        },
        ilr::ReviewKind::PendingRoute | ilr::ReviewKind::GroupIdentity => {
            panic!("available kind passed to refusal command")
        }
    }
}

fn cli_action(command: &ReviewResolutionCommand) -> Value {
    match command {
        ReviewResolutionCommand::IdentityConflict { action, .. } => json!(action),
        ReviewResolutionCommand::FieldResolution { action, .. } => json!(action),
        ReviewResolutionCommand::ContributorOrder { .. } => json!(command),
        ReviewResolutionCommand::EditionEvidence { action, .. } => json!(action),
        ReviewResolutionCommand::ImportIdentity { action, .. } => json!(action),
        ReviewResolutionCommand::MigrationRepair { action, .. } => json!(action),
        ReviewResolutionCommand::InvariantRepair { action, .. } => json!(action),
        ReviewResolutionCommand::PendingRoute { action, .. } => json!(action),
        ReviewResolutionCommand::GroupIdentity { action, .. } => json!(action),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct UserStateSnapshot {
    works: String,
    routes: String,
    cards: String,
    audits: String,
    legacy_conflicts: String,
    editions: String,
    library_items: String,
}

async fn json_snapshot(db: &SqliteDb, user_id: i64, sql: &str) -> String {
    sqlx::query_scalar(sql)
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("snapshot identity state")
}

async fn user_state_snapshot(db: &SqliteDb, user_id: i64) -> UserStateSnapshot {
    UserStateSnapshot {
        works: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'title',title,'author',author_name,'author_id',author_id,\
                'generation',identity_generation,'main',normalized_identity_main,\
                'subtitle',normalized_identity_subtitle,'volume',normalized_identity_volume,\
                'distinction',text_distinction,'ol',ol_key,'gr',gr_key,'hc',hc_key,\
                'isbn',isbn_13,'asin',asin)), '[]') FROM (\
                SELECT * FROM works WHERE user_id=?1 ORDER BY id)",
        )
        .await,
        routes: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'owner_type',owner_type,'work_id',work_id,'edition_id',edition_id,\
                'resolved_work_id',resolved_work_id,'provider',provider,'kind',kind,\
                'value',provider_scoped_id,'state',state,'provenance',provenance,\
                'confirmed',user_confirmed,'observed_at',observed_at)), '[]') FROM (\
                SELECT * FROM identity_routes WHERE user_id=?1 ORDER BY id)",
        )
        .await,
        cards: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'work_id',work_id,'kind',kind,'generation',generation,\
                'status',status,'payload',payload,'resolved_at',resolved_at)), '[]') FROM (\
                SELECT * FROM identity_review_cards WHERE user_id=?1 ORDER BY id)",
        )
        .await,
        audits: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'work_id',work_id,'event_kind',event_kind,'actor',actor,\
                'payload',payload,'created_at',created_at)), '[]') FROM (\
                SELECT * FROM identity_audit_events WHERE user_id=?1 ORDER BY id)",
        )
        .await,
        legacy_conflicts: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'work',existing_work_id,'kind',kind,'incoming',incoming_payload_json,\
                'status',status,'resolved_at',resolved_at,'action',resolution_action,\
                'notes',resolution_notes)), '[]') FROM (\
                SELECT * FROM work_identity_conflicts WHERE user_id=?1 ORDER BY id)",
        )
        .await,
        editions: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'work_id',work_id,'format',format,'language',language,'state',state)),\
                '[]') FROM (SELECT * FROM editions WHERE user_id=?1 ORDER BY id)",
        )
        .await,
        library_items: json_snapshot(
            db,
            user_id,
            "SELECT COALESCE(json_group_array(json_object(\
                'id',id,'work_id',work_id,'path',path,'tag_status',tag_status,\
                'generation',tagged_at_generation)), '[]') FROM (\
                SELECT * FROM library_items WHERE user_id=?1 ORDER BY id)",
        )
        .await,
    }
}

fn continuation_message(kind: ilr::ReviewKind) -> String {
    ilr::IdentityRoadError::ContinuationUnavailable { kind }.to_string()
}

#[derive(Clone, Copy)]
enum ReviewHttpIngress {
    Typed,
    LegacyAlias,
}

impl ReviewHttpIngress {
    fn path(self, card_id: i64) -> String {
        match self {
            Self::Typed => format!("/api/v1/identity-review-card/{card_id}/resolve"),
            Self::LegacyAlias => format!("/api/v1/identity-review/{card_id}/resolve"),
        }
    }
}

async fn assert_http_refusal(kind: ilr::ReviewKind, ingress: ReviewHttpIngress) {
    let harness = build_route_harness().await;
    let card = mint_refused_card(&harness.db, harness.user_id, kind).await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let response = call_router_json(
        &harness,
        Method::POST,
        ingress.path(card.card_id),
        Some(json!({"command": refusal_command(&card)})),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
    assert_eq!(response.json["message"], continuation_message(kind));
    assert!(response.json["message"]
        .as_str()
        .is_some_and(|message| message.contains(kind.storage_code())));
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "{kind:?} refusal must precede every write"
    );
    if let Some(work_id) = card.work_id {
        assert_eq!(work_generation(&harness.db, work_id).await, card.generation);
    }
}

async fn assert_direct_road_refusal(kind: ilr::ReviewKind) {
    let harness = build_route_harness().await;
    let card = mint_refused_card(&harness.db, harness.user_id, kind).await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let result = harness
        .state
        .identity_road
        .resolve_review(
            ReviewActor::AuthenticatedUser {
                user_id: harness.user_id,
            },
            refusal_command(&card),
        )
        .await;
    assert!(matches!(
        result,
        Err(ilr::IdentityRoadError::ContinuationUnavailable { kind: observed })
            if observed == kind
    ));
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before
    );
}

#[tokio::test]
async fn sqlite_pending_route_road_refuses_import_identity_without_delta() {
    let harness = build_route_harness().await;
    let card = mint_refused_card(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::ImportIdentity,
    )
    .await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let road = livrarr_behavioral::stubs::SqlitePendingRouteRoad::new(harness.db.clone());
    let result = IdentityRoadService::resolve_review(
        &road,
        ReviewActor::AuthenticatedUser {
            user_id: harness.user_id,
        },
        refusal_command(&card),
    )
    .await;
    assert!(matches!(
        result,
        Err(ilr::IdentityRoadError::ContinuationUnavailable {
            kind: ilr::ReviewKind::ImportIdentity
        })
    ));
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "refused-kind regression through SqlitePendingRouteRoad writes nothing"
    );
}

const REFUSED_KINDS: [ilr::ReviewKind; 7] = [
    ilr::ReviewKind::IdentityConflict,
    ilr::ReviewKind::FieldResolution,
    ilr::ReviewKind::ContributorOrder,
    ilr::ReviewKind::EditionEvidence,
    ilr::ReviewKind::ImportIdentity,
    ilr::ReviewKind::MigrationRepair,
    ilr::ReviewKind::InvariantRepair,
];

// RED-UNTIL-U1: today there is no exhaustive availability decision and all seven kinds can fabricate success.
#[test]
fn irf_u1_guard_refuses_all_seven_unavailable_kinds() {
    for kind in REFUSED_KINDS {
        assert!(matches!(
            ilr::require_continuation(kind),
            Err(ilr::IdentityRoadError::ContinuationUnavailable { kind: observed })
                if observed == kind
        ));
    }
}

// PIN: GroupIdentity and PendingRoute continue; only the merge answer is refused.
#[test]
fn irf_u1_guard_keeps_group_and_pending_available() {
    assert_eq!(
        ilr::require_continuation(ilr::ReviewKind::GroupIdentity),
        Ok(())
    );
    assert_eq!(
        ilr::require_continuation(ilr::ReviewKind::PendingRoute),
        Ok(())
    );
}

// RED-UNTIL-U1: today the production road commits fabricated resolutions for all seven refused kinds.
#[tokio::test]
async fn irf_u1_road_returns_named_error_before_any_write() {
    for kind in REFUSED_KINDS {
        assert_direct_road_refusal(kind).await;
    }
}

// RED-UNTIL-U1: today FieldResolution fabricates success through the typed route.
#[tokio::test]
async fn typed_field_resolution_is_refused() {
    assert_http_refusal(ilr::ReviewKind::FieldResolution, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today ContributorOrder fabricates success through the typed route.
#[tokio::test]
async fn typed_contributor_order_is_refused() {
    assert_http_refusal(ilr::ReviewKind::ContributorOrder, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today MigrationRepair fabricates success through the typed route.
#[tokio::test]
async fn typed_migration_repair_is_refused() {
    assert_http_refusal(ilr::ReviewKind::MigrationRepair, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today InvariantRepair fabricates success through the typed route.
#[tokio::test]
async fn typed_invariant_repair_is_refused() {
    assert_http_refusal(ilr::ReviewKind::InvariantRepair, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today ImportIdentity fabricates success through the typed route.
#[tokio::test]
async fn typed_import_identity_is_refused() {
    assert_http_refusal(ilr::ReviewKind::ImportIdentity, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today EditionEvidence fabricates success through the typed route.
#[tokio::test]
async fn typed_edition_evidence_is_refused() {
    assert_http_refusal(ilr::ReviewKind::EditionEvidence, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today IdentityConflict fabricates success through the typed route.
#[tokio::test]
async fn typed_identity_conflict_is_refused() {
    assert_http_refusal(ilr::ReviewKind::IdentityConflict, ReviewHttpIngress::Typed).await;
}

// RED-UNTIL-U1: today FieldResolution fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_field_resolution_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::FieldResolution,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

// RED-UNTIL-U1: today ContributorOrder fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_contributor_order_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::ContributorOrder,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

// RED-UNTIL-U1: today MigrationRepair fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_migration_repair_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::MigrationRepair,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

// RED-UNTIL-U1: today InvariantRepair fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_invariant_repair_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::InvariantRepair,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

// RED-UNTIL-U1: today ImportIdentity fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_import_identity_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::ImportIdentity,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

// RED-UNTIL-U1: today EditionEvidence fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_edition_evidence_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::EditionEvidence,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

// RED-UNTIL-U1: today IdentityConflict fabricates success through the legacy alias.
#[tokio::test]
async fn legacy_alias_identity_conflict_is_refused() {
    assert_http_refusal(
        ilr::ReviewKind::IdentityConflict,
        ReviewHttpIngress::LegacyAlias,
    )
    .await;
}

struct CliFixture {
    data_dir: tempfile::TempDir,
    user_id: i64,
    card: MintedRefusal,
    action_file: PathBuf,
}

async fn prepare_cli_fixture(kind: ilr::ReviewKind) -> CliFixture {
    // The real cutover command necessarily owns a file-backed data directory;
    // every HTTP/road case above uses the packet-mandated in-memory helper.
    let data_dir = tempfile::tempdir().expect("U1 CLI data dir");
    let pool = livrarr_db::pool::create_sqlite_pool(data_dir.path())
        .await
        .expect("create CLI database");
    livrarr_db::pool::run_migrations(&pool)
        .await
        .expect("migrate CLI database");
    let db = SqliteDb::new(pool);
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
         ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
    )
    .execute(db.pool())
    .await
    .expect("install production Author identity index");
    db.ensure_identity_authority_ready()
        .await
        .expect("activate empty CLI database");
    let user_id = seed_user(&db, "cli").await;
    let card = mint_refused_card(&db, user_id, kind).await;
    let action_file = data_dir.path().join(format!("u1-{:?}-action.json", kind));
    std::fs::write(
        &action_file,
        serde_json::to_vec(&cli_action(&refusal_command(&card))).expect("encode CLI action"),
    )
    .expect("write CLI action file");
    db.pool().close().await;
    CliFixture {
        data_dir,
        user_id,
        card,
        action_file,
    }
}

async fn snapshot_cli_dir(path: &Path, user_id: i64) -> UserStateSnapshot {
    let pool = livrarr_db::pool::create_sqlite_pool(path)
        .await
        .expect("open CLI database for snapshot");
    let db = SqliteDb::new(pool);
    let snapshot = user_state_snapshot(&db, user_id).await;
    db.pool().close().await;
    snapshot
}

fn production_livrarr_binary() -> &'static PathBuf {
    LIVRARR_BINARY.get_or_init(|| {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("resolve workspace root");
        let build = std::process::Command::new("cargo")
            .args([
                "build",
                "--quiet",
                "-p",
                "livrarr-server",
                "--bin",
                "livrarr",
            ])
            .env("RTK_DISABLED", "1")
            .current_dir(&workspace)
            .output()
            .expect("build production livrarr binary");
        assert!(
            build.status.success(),
            "production binary build failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let configured_target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace.join("target"));
        let target = if configured_target.is_absolute() {
            configured_target
        } else {
            workspace.join(configured_target)
        };
        target.join("debug/livrarr")
    })
}

async fn assert_cli_refusal(kind: ilr::ReviewKind) {
    let fixture = prepare_cli_fixture(kind).await;
    let before = snapshot_cli_dir(fixture.data_dir.path(), fixture.user_id).await;

    let result = livrarr_server::identity_layer::run_identity_cutover_command(
        livrarr_server::identity_layer::IdentityCutoverCliCommand::ResolveReview {
            card_id: fixture.card.card_id,
            expected_generation: fixture.card.generation,
            action_file: fixture.action_file.clone(),
        },
        fixture.data_dir.path().to_path_buf(),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(
        result,
        Err(livrarr_server::identity_layer::IdentityCutoverCommandError::ContinuationUnavailable(
            observed
        )) if observed == kind
    ));
    assert_eq!(
        snapshot_cli_dir(fixture.data_dir.path(), fixture.user_id).await,
        before,
        "library cutover command must not write on {kind:?} refusal"
    );

    let output = std::process::Command::new(production_livrarr_binary())
        .arg("--data")
        .arg(fixture.data_dir.path())
        .args(["identity-cutover", "resolve"])
        .arg(fixture.card.card_id.to_string())
        .arg("--expected-generation")
        .arg(fixture.card.generation.to_string())
        .arg("--action-file")
        .arg(&fixture.action_file)
        .output()
        .expect("run real identity-cutover resolve command");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(kind.storage_code()),
        "CLI stderr must name {kind:?}: {stderr}"
    );
    assert!(!stderr.contains("database error"));
    assert_eq!(
        snapshot_cli_dir(fixture.data_dir.path(), fixture.user_id).await,
        before,
        "binary cutover command must not write on {kind:?} refusal"
    );
}

// RED-UNTIL-U1: today the CLI maps FieldResolution's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_field_resolution_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::FieldResolution).await;
}

// RED-UNTIL-U1: today the CLI maps ContributorOrder's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_contributor_order_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::ContributorOrder).await;
}

// RED-UNTIL-U1: today the CLI maps MigrationRepair's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_migration_repair_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::MigrationRepair).await;
}

// RED-UNTIL-U1: today the CLI maps InvariantRepair's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_invariant_repair_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::InvariantRepair).await;
}

// RED-UNTIL-U1: today the CLI maps ImportIdentity's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_import_identity_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::ImportIdentity).await;
}

// RED-UNTIL-U1: today the CLI maps EditionEvidence's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_edition_evidence_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::EditionEvidence).await;
}

// RED-UNTIL-U1: today the CLI maps IdentityConflict's fabricated success instead of the named refusal.
#[tokio::test]
async fn cli_identity_conflict_is_refused_by_name() {
    assert_cli_refusal(ilr::ReviewKind::IdentityConflict).await;
}

struct ConflictFixture {
    external_id: i64,
    typed_card_id: Option<i64>,
}

fn incoming_conflict(label: &str) -> IncomingConflictPayload {
    IncomingConflictPayload {
        ol_key: Some(format!("OL-{label}-NEW-W")),
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: format!("Incoming {label}"),
        author_name: "U1 Conflict Author".to_string(),
        year: Some(2026),
        cover_url: None,
        top_candidates: vec![],
    }
}

async fn raise_legacy_conflict(
    harness: &RouteHarness,
    user_id: i64,
    work_id: i64,
    label: &str,
) -> i64 {
    // Legacy rows have no production writer any more — seed the surviving
    // table directly, mirroring the one kept runtime mint's row shape.
    let incoming_json = serde_json::to_string(&incoming_conflict(label)).expect("payload json");
    let id = sqlx::query(
        "INSERT INTO work_identity_conflicts \
         (user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, \
          raised_source_path, status) \
         VALUES (?1, ?2, 'incoming_different_ol_key', ?3, ?4, 'manual_add', NULL, 'open')",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(&incoming_json)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(harness.db.pool())
    .await
    .expect("seed legacy conflict row")
    .last_insert_rowid();
    sqlx::query("UPDATE works SET identity_status = 'conflict' WHERE id = ?1 AND user_id = ?2")
        .bind(work_id)
        .bind(user_id)
        .execute(harness.db.pool())
        .await
        .expect("reflect legacy conflict badge");
    id
}

async fn insert_typed_conflict_compatibility(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
) -> ConflictFixture {
    // AC-001 compatibility fixture: IdentityConflict has no production typed
    // mint door (ST-007). Serialize the production payload type; do not invent
    // a parallel representation or a legacy row.
    let external_id = 800_000 + CASE_ID.fetch_add(1, Ordering::Relaxed) as i64;
    let generation = work_generation(db, work_id).await;
    let payload = serde_json::to_string(&ilr::SettlementReviewCard::IdentityConflict {
        conflict_id: external_id,
        work_id,
    })
    .expect("serialize production IdentityConflict payload");
    let card_id = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id, work_id, kind, generation, status, payload, created_at) \
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(ilr::ReviewKind::IdentityConflict.storage_code())
    .bind(generation)
    .bind(payload)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("insert typed compatibility card")
    .last_insert_rowid();
    ConflictFixture {
        external_id,
        typed_card_id: Some(card_id),
    }
}

async fn assert_conflict_id_404(harness: &RouteHarness, id: i64) {
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let resolve = call_router_json(
        harness,
        Method::POST,
        format!("/api/v1/identity-conflict/{id}/resolve"),
        Some(json!({"action": ConflictResolutionAction::KeepExisting})),
    )
    .await;
    assert_eq!(resolve.status, StatusCode::NOT_FOUND, "{}", resolve.json);
    let dismiss = call_router_json(
        harness,
        Method::POST,
        format!("/api/v1/identity-conflict/{id}/dismiss"),
        None,
    )
    .await;
    assert_eq!(dismiss.status, StatusCode::NOT_FOUND, "{}", dismiss.json);
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before,
        "404 scope/existence checks write nothing"
    );
}

// PIN: unknown ids remain 404 through both conflict endpoints.
#[tokio::test]
async fn conflict_unknown_ids_remain_404() {
    let harness = build_route_harness().await;
    assert_conflict_id_404(&harness, 9_999_991).await;
}

// PIN: foreign legacy and typed ids remain indistinguishable 404s.
#[tokio::test]
async fn conflict_foreign_ids_remain_404() {
    let harness = build_route_harness().await;
    let foreign_user = seed_user(&harness.db, "foreign-conflict").await;
    let (foreign_work, _) = seed_work(&harness.db, foreign_user, "foreign-conflict").await;
    let foreign_legacy =
        raise_legacy_conflict(&harness, foreign_user, foreign_work, "FOREIGN").await;
    let foreign_typed =
        insert_typed_conflict_compatibility(&harness.db, foreign_user, foreign_work).await;
    assert_conflict_id_404(&harness, foreign_legacy).await;
    assert_conflict_id_404(&harness, foreign_typed.external_id).await;
    let legacy_status: String =
        sqlx::query_scalar("SELECT status FROM work_identity_conflicts WHERE id=?1")
            .bind(foreign_legacy)
            .fetch_one(harness.db.pool())
            .await
            .expect("foreign legacy row survives");
    let typed_status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
            .bind(foreign_typed.typed_card_id.expect("foreign typed card"))
            .fetch_one(harness.db.pool())
            .await
            .expect("foreign typed row survives");
    assert_eq!(
        (legacy_status.as_str(), typed_status.as_str()),
        ("open", "pending")
    );
}

// PIN: closed legacy and typed ids remain 404 through both conflict endpoints.
#[tokio::test]
async fn conflict_closed_ids_remain_404() {
    let harness = build_route_harness().await;
    let (legacy_work, _) = seed_work(&harness.db, harness.user_id, "closed-legacy").await;
    let legacy_id = raise_legacy_conflict(&harness, harness.user_id, legacy_work, "CLOSED").await;
    sqlx::query("UPDATE work_identity_conflicts SET status = 'dismissed' WHERE id = ?1")
        .bind(legacy_id)
        .execute(harness.db.pool())
        .await
        .expect("close legacy row directly");

    let (typed_work, _) = seed_work(&harness.db, harness.user_id, "closed-typed").await;
    let typed = insert_typed_conflict_compatibility(&harness.db, harness.user_id, typed_work).await;
    let generic_dismiss = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/identity-review-card/{}/dismiss",
            typed.typed_card_id.expect("typed card id")
        ),
        None,
    )
    .await;
    assert_eq!(generic_dismiss.status, StatusCode::NO_CONTENT);

    assert_conflict_id_404(&harness, legacy_id).await;
    assert_conflict_id_404(&harness, typed.external_id).await;
}

async fn mint_existing_group(harness: &RouteHarness, label: &str) -> (i64, i64, i64, i64) {
    // Persist a pre-containment card through the real writer: new HTTP merge
    // requests are disabled, but existing installations retain these records.
    let (survivor, author_id) =
        seed_work(&harness.db, harness.user_id, &format!("{label}-survivor")).await;
    let (loser, _) = seed_work(&harness.db, harness.user_id, &format!("{label}-loser")).await;
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, survivor)
            .await
            .unwrap();
    let card = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![survivor, loser],
        proposed_identity: None,
        merge_choices: vec![],
    };
    let mut command = settlement_commit(
        harness.user_id,
        author_id,
        &captured.identity_title.main,
        card,
    );
    command.existing_work_id = Some(survivor);
    command.identity_title = captured.identity_title;
    command.expected_generation = work_generation(&harness.db, survivor).await;
    let committed = WorkIdentityRepository::commit_settlement(&harness.db, command)
        .await
        .unwrap();
    let card = committed.review_cards[0];
    (survivor, loser, card.id, card.generation)
}

async fn mint_pending_route_card(
    db: &SqliteDb,
    user_id: i64,
    label: &str,
) -> (i64, ilr::MintedReviewCard, ilr::RouteKey) {
    let (work_id, _) = seed_work(db, user_id, label).await;
    let generation = work_generation(db, work_id).await;
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: format!("OL-U1-{}-W", CASE_ID.fetch_add(1, Ordering::Relaxed)),
    };
    let minted = WorkIdentityRepository::commit_pending_route_review(
        db,
        user_id,
        work_id,
        generation,
        ilr::ParkedRouteCandidate {
            route: route.clone(),
            proposed_owner: ilr::RouteOwner::Work(work_id),
        },
    )
    .await
    .expect("production PendingRoute writer");
    (work_id, minted, route)
}

async fn assert_merge_answer_refused_through(ingress: ReviewHttpIngress) {
    let harness = build_route_harness().await;
    let (survivor, _, card_id, generation) = mint_existing_group(&harness, "contained-group").await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    // A stale generation is refused as merging, before any generation check.
    for expected_generation in [generation, generation - 1] {
        let command = ReviewResolutionCommand::GroupIdentity {
            card_id,
            expected_generation,
            action: ilr::GroupIdentityAction::AttachOrMerge { anchor: survivor },
        };
        let response = call_router_json(
            &harness,
            Method::POST,
            ingress.path(card_id),
            Some(json!({"command": command})),
        )
        .await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
        assert_eq!(
            response.json["message"],
            "Merging is currently unavailable."
        );
        let direct = WorkIdentityRepository::commit_review_continuation(
            &harness.db,
            ReviewActor::AuthenticatedUser {
                user_id: harness.user_id,
            },
            command,
            CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(
                direct,
                Err(ilr::IdentityRepositoryError::MergingUnavailable)
            ),
            "{direct:?}"
        );
        assert_eq!(
            user_state_snapshot(&harness.db, harness.user_id).await,
            before
        );
    }
    assert!(listed_card(&harness, card_id).await.is_some());
}

async fn assert_pending_available_through(ingress: ReviewHttpIngress) {
    let harness = build_route_harness().await;
    let (work_id, card, route) =
        mint_pending_route_card(&harness.db, harness.user_id, "available-pending").await;
    let response = call_router_json(
        &harness,
        Method::POST,
        ingress.path(card.id),
        Some(json!({"command": {
            "PendingRoute": {
                "card_id": card.id,
                "expected_generation": card.generation,
                "action": {"Affirm": {"surviving_routes": [route]}}
            }
        }})),
    )
    .await;
    assert!(response.status.is_success(), "{}", response.json);
    let (status, confirmed): (String, i64) = sqlx::query_as(
        "SELECT c.status, (SELECT COUNT(*) FROM identity_routes r \
            WHERE r.user_id=?1 AND r.resolved_work_id=?2 AND r.user_confirmed=1) \
         FROM identity_review_cards c WHERE c.user_id=?1 AND c.id=?3",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(card.id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read PendingRoute result");
    assert_eq!(status, "resolved");
    assert_eq!(confirmed, 1);
}

// AC-012: both aliases and the repository refuse the merge answer without writes.
#[tokio::test]
async fn group_identity_merge_answer_is_refused_through_both_review_routes() {
    assert_merge_answer_refused_through(ReviewHttpIngress::Typed).await;
    assert_merge_answer_refused_through(ReviewHttpIngress::LegacyAlias).await;
}

// PIN: the typed and legacy-alias doors retain the current PendingRoute continuation.
#[tokio::test]
async fn pending_route_remains_available_through_both_review_routes() {
    assert_pending_available_through(ReviewHttpIngress::Typed).await;
    assert_pending_available_through(ReviewHttpIngress::LegacyAlias).await;
}

// PO containment: merge choices cannot bypass the refusal or mint a new card.
#[tokio::test]
async fn inline_merge_with_choices_is_refused_without_minting_a_card() {
    let harness = build_route_harness().await;
    let (survivor, _) = seed_work(&harness.db, harness.user_id, "contained-choices-survivor").await;
    let (loser, _) = seed_work(&harness.db, harness.user_id, "contained-choices-loser").await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{survivor}/merge/{loser}"),
        Some(json!({"choices": [{"field":"series_name","choice":"keep_survivor"}]})),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
    assert_eq!(
        response.json["message"],
        "Merging is currently unavailable."
    );
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before
    );
}

// PIN: inline pending-route Affirm retains its scalar claim, route mutation, audit, and 204 response.
#[tokio::test]
async fn inline_pending_route_affirm_is_unchanged() {
    let harness = build_route_harness().await;
    let (work_id, _) = seed_work(&harness.db, harness.user_id, "inline-affirm").await;
    let value = format!("U1-GR-{}", CASE_ID.fetch_add(1, Ordering::Relaxed));
    livrarr_db::test_helpers::record_pending_anchor_fixture(
        &harness.db,
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        &value,
    )
    .await
    .expect("seed pending anchor through production writer");
    let generation = work_generation(&harness.db, work_id).await;
    harness.state.identity_road.test_recorder().clear();

    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{work_id}/pending-anchors/gr_work/affirm"),
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.json);

    let calls = harness.state.identity_road.test_recorder().snapshot();
    assert_eq!(calls.len(), 2, "affirm is one settle plus one continuation");
    assert!(matches!(
        &calls[0],
        livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
            if request.origin == ilr::IdentityRoadOrigin::AffirmPendingRoute
    ));
    let (card_id, expected_generation) = match &calls[1] {
        livrarr_server::identity_layer::IdentityRoadCall::Resolve {
            actor: ReviewActor::AuthenticatedUser { user_id },
            command:
                ReviewResolutionCommand::PendingRoute {
                    card_id,
                    expected_generation,
                    action: ilr::PendingRouteAction::Affirm { surviving_routes },
                },
        } if *user_id == harness.user_id
            && surviving_routes.len() == 1
            && surviving_routes[0].value == value =>
        {
            (*card_id, *expected_generation)
        }
        other => panic!("affirm continuation changed: {other:?}"),
    };
    assert_eq!(expected_generation, generation + 1);
    assert_eq!(work_generation(&harness.db, work_id).await, generation + 2);
    let (status, confirmed, audits): (String, i64, i64) = sqlx::query_as(
        "SELECT c.status, \
            (SELECT COUNT(*) FROM identity_routes r WHERE r.user_id=?1 \
                AND r.resolved_work_id=?2 AND r.provider_scoped_id=?3 \
                AND r.user_confirmed=1 AND r.provenance='\"UserChoice\"'), \
            (SELECT COUNT(*) FROM identity_audit_events a WHERE a.user_id=?1 \
                AND a.event_kind='review-resolution' \
                AND json_extract(a.payload, '$.PendingRoute.card_id')=?4) \
         FROM identity_review_cards c WHERE c.user_id=?1 AND c.id=?4",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .bind(&value)
    .bind(card_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read inline Affirm mutation/audit");
    assert_eq!((status.as_str(), confirmed, audits), ("resolved", 1, 1));
}

// PIN: ReviewProposalInvalidated remains a 409 row in the typed mapper.
#[tokio::test]
async fn contained_review_stays_read_only_when_a_member_has_been_deleted() {
    let harness = build_route_harness().await;
    let (survivor, loser, card_id, _) = mint_existing_group(&harness, "mapper-invalidated").await;
    let deleted = call_router_json(
        &harness,
        Method::DELETE,
        format!("/api/v1/work/{loser}"),
        None,
    )
    .await;
    assert!(deleted.status.is_success(), "{}", deleted.json);
    let listed =
        call_router_json(&harness, Method::GET, "/api/v1/identity-review-card", None).await;
    let generation = listed
        .json
        .as_array()
        .and_then(|cards| cards.iter().find(|card| card["id"] == card_id))
        .and_then(|card| card["generation"].as_i64())
        .expect("invalidated card remains listed");
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/resolve"),
        Some(json!({"command": {
            "GroupIdentity": {
                "card_id": card_id,
                "expected_generation": generation,
                "action": {"AttachOrMerge": {"anchor": survivor}}
            }
        }})),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
    assert_eq!(
        response.json["message"],
        "Merging is currently unavailable."
    );
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before
    );
}

// PIN: ReviewKindMismatch remains a 400 row in the typed mapper.
#[tokio::test]
async fn typed_mapper_keeps_review_kind_mismatch_at_400() {
    let harness = build_route_harness().await;
    let (_work_id, card, _route) =
        mint_pending_route_card(&harness.db, harness.user_id, "mapper-mismatch").await;
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{}/resolve", card.id),
        Some(json!({"command": {
            "GroupIdentity": {
                "card_id": card.id,
                "expected_generation": card.generation,
                "action": "DifferentFromAll"
            }
        }})),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{}",
        response.json
    );
    assert_eq!(response.json["message"], "review kind mismatch");
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before
    );
}

#[tokio::test]
async fn containment_direct_settlement_and_startup_heals_cannot_absorb() {
    let harness = build_route_harness().await;
    let (survivor, author_id) = seed_work(&harness.db, harness.user_id, "direct-survivor").await;
    let (loser, _) = seed_work(&harness.db, harness.user_id, "direct-loser").await;
    let captured =
        WorkIdentityRepository::read_captured_identity(&harness.db, harness.user_id, survivor)
            .await
            .unwrap();
    let before = user_state_snapshot(&harness.db, harness.user_id).await;
    let mut command = settlement_commit(
        harness.user_id,
        author_id,
        &captured.identity_title.main,
        ilr::SettlementReviewCard::GroupIdentity {
            work_ids: vec![survivor, loser],
            proposed_identity: None,
            merge_choices: vec![],
        },
    );
    command.existing_work_id = Some(survivor);
    command.identity_title = captured.identity_title;
    command.expected_generation = work_generation(&harness.db, survivor).await;
    command.review_cards.clear();
    command.absorbed_work_ids = vec![loser];
    let result = WorkIdentityRepository::commit_settlement(&harness.db, command).await;
    assert!(
        matches!(
            result,
            Err(ilr::IdentityRepositoryError::MergingUnavailable)
        ),
        "{result:?}"
    );
    let before_markers: Vec<(String, String)> =
        sqlx::query_as("SELECT key,value FROM _livrarr_meta ORDER BY key")
            .fetch_all(harness.db.pool())
            .await
            .unwrap();
    livrarr_db::identity_layer::heal_identity_dedup_residue(harness.db.pool())
        .await
        .unwrap();
    livrarr_db::pool::heal_identity_title_policy(harness.db.pool())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (String, String)>("SELECT key,value FROM _livrarr_meta ORDER BY key")
            .fetch_all(harness.db.pool())
            .await
            .unwrap(),
        before_markers
    );
    assert_eq!(
        user_state_snapshot(&harness.db, harness.user_id).await,
        before
    );
}

// ---------------------------------------------------------------------------
// card-edits-lift (spec-card-edits-lift.md v4): title/author edits and the
// "different book" answer work through their real doors; the merge answer,
// manual merge and automatic absorption stay refused.
// ---------------------------------------------------------------------------

/// Letters only, so no fixture title grows a trailing number that the title
/// parser would read as a volume.
fn lift_label(stem: &str) -> String {
    let mut n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    let mut suffix = String::new();
    loop {
        suffix.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
    }
    format!("{stem} Lift {suffix}")
}

fn lift_ol_value() -> String {
    format!("OL{}W", 880_000 + CASE_ID.fetch_add(1, Ordering::Relaxed))
}

async fn lift_author(db: &SqliteDb, user_id: i64, name: &str) -> i64 {
    db.create_author(CreateAuthorDbRequest {
        user_id,
        name: name.to_string(),
        sort_name: None,
        ol_key: None,
        gr_key: None,
        hc_key: None,
        import_id: None,
    })
    .await
    .expect("seed Author")
    .0
    .id
}

fn lift_tuple(main: &str, subtitle: Option<&str>, volume: Option<&str>) -> ilr::IdentityTitleTuple {
    ilr::IdentityTitleTuple {
        main: main.to_string(),
        subtitle: subtitle.map(str::to_string),
        volume: volume.map(str::to_string),
        normalized_main: main.to_lowercase(),
        normalized_subtitle: subtitle.unwrap_or_default().to_lowercase(),
        normalized_volume: volume.unwrap_or_default().to_string(),
        provenance: ilr::EvidenceProvenance::User,
    }
}

fn lift_route(user_id: i64, value: &str) -> ilr::WorkRoute {
    ilr::WorkRoute {
        id: 0,
        user_id,
        owner: ilr::RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        provider_scoped_id: value.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::OpenLibrary),
        user_confirmed: false,
        observed_at: Utc::now(),
    }
}

/// A Work created through the sole settlement writer.
async fn lift_work(
    db: &SqliteDb,
    user_id: i64,
    title: ilr::IdentityTitleTuple,
    author_id: i64,
    routes: Vec<ilr::WorkRoute>,
) -> i64 {
    WorkIdentityRepository::commit_settlement(
        db,
        ilr::SettlementCommit {
            creation_facts: None,
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title: title,
            text_distinction: None,
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes,
            absorbed_work_ids: vec![],
            expected_generation: 0,
            review_cards: vec![],
        },
    )
    .await
    .expect("seed Work through the settlement writer")
    .identity
    .own_work_id
}

/// Re-settles the anchor's own identity: a legitimate identity write that
/// advances its generation, as enrichment would.
async fn lift_settle_again(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
    review_cards: Vec<ilr::SettlementReviewCard>,
) -> ilr::SettlementCommitOutcome {
    let captured = WorkIdentityRepository::read_captured_identity(db, user_id, work_id)
        .await
        .expect("read anchor identity");
    WorkIdentityRepository::commit_settlement(
        db,
        ilr::SettlementCommit {
            creation_facts: None,
            user_id,
            existing_work_id: Some(work_id),
            add_source: None,
            identity_title: captured.identity_title.clone(),
            text_distinction: None,
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id,
                author_id: captured.primary_author_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes: captured.active_routes.clone(),
            absorbed_work_ids: vec![],
            expected_generation: captured.identity_generation,
            review_cards,
        },
    )
    .await
    .expect("settle anchor through the settlement writer")
}

/// A pending GroupIdentity card minted through the settlement writer, as
/// settlement parks one for a group.
async fn lift_group_card(
    db: &SqliteDb,
    user_id: i64,
    anchor: i64,
    work_ids: Vec<i64>,
    proposed_identity: Option<ilr::WorkIdentityEvidence>,
    merge_choices: Value,
) -> i64 {
    let committed = lift_settle_again(
        db,
        user_id,
        anchor,
        vec![ilr::SettlementReviewCard::GroupIdentity {
            work_ids,
            proposed_identity,
            merge_choices: serde_json::from_value(merge_choices).expect("merge choices"),
        }],
    )
    .await;
    committed.review_cards[0].id
}

fn lift_proposal(user_id: i64, author_id: i64, stem: &str) -> ilr::WorkIdentityEvidence {
    ilr::WorkIdentityEvidence {
        title: lift_tuple(&lift_label(stem), Some("Proposed"), Some("7")),
        primary_author_id: author_id,
        routes: vec![lift_route(user_id, &lift_ol_value())],
    }
}

async fn lift_work_row(db: &SqliteDb, work_id: i64) -> Value {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT json_object('title',title,'subtitle',subtitle,'volume',identity_volume,\
            'author',author_name,'author_id',author_id,'primary_author_id',primary_author_id,\
            'main',normalized_identity_main,'nsub',normalized_identity_subtitle,\
            'nvol',normalized_identity_volume,'distinction',text_distinction,\
            'generation',identity_generation,'series',series_name,\
            'monitor_ebook',monitor_ebook,'monitor_audiobook',monitor_audiobook) \
         FROM works WHERE id=?1",
    )
    .bind(work_id)
    .fetch_optional(db.pool())
    .await
    .expect("read Work row");
    raw.map(|raw| serde_json::from_str(&raw).expect("Work row JSON"))
        .unwrap_or(Value::Null)
}

async fn lift_text(db: &SqliteDb, sql: &str, bind: i64) -> String {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(db.pool())
        .await
        .expect("read snapshot text")
}

async fn lift_routes(db: &SqliteDb, work_id: i64) -> String {
    lift_text(
        db,
        "SELECT COALESCE(json_group_array(json_object('provider',provider,'kind',kind,\
            'value',provider_scoped_id,'owner',owner_type,'confirmed',user_confirmed)),'[]') \
         FROM (SELECT * FROM identity_routes WHERE resolved_work_id=?1 AND state='active' ORDER BY id)",
        work_id,
    )
    .await
}

async fn lift_contributors(db: &SqliteDb, work_id: i64) -> String {
    lift_text(
        db,
        "SELECT COALESCE(json_group_array(json_object('author_id',c.author_id,'ordinal',c.ordinal,\
            'roles',json((SELECT json_group_array(r.role || '|' || r.provenance) \
                FROM work_contributor_roles r \
                WHERE r.work_id=c.work_id AND r.author_id=c.author_id)))),'[]') \
         FROM (SELECT * FROM work_contributors WHERE work_id=?1 ORDER BY ordinal, author_id) c",
        work_id,
    )
    .await
}

async fn lift_count(db: &SqliteDb, sql: &str, bind: i64) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(db.pool())
        .await
        .expect("read count")
}

async fn lift_pending_cards(db: &SqliteDb, user_id: i64) -> i64 {
    lift_count(
        db,
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND status='pending'",
        user_id,
    )
    .await
}

async fn lift_works(db: &SqliteDb, user_id: i64) -> i64 {
    lift_count(db, "SELECT COUNT(*) FROM works WHERE user_id=?1", user_id).await
}

async fn lift_resolution_audits(db: &SqliteDb, user_id: i64) -> i64 {
    lift_count(
        db,
        "SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1 AND event_kind='review-resolution'",
        user_id,
    )
    .await
}

async fn lift_card_status(db: &SqliteDb, card_id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
        .bind(card_id)
        .fetch_optional(db.pool())
        .await
        .expect("read card status")
}

async fn listed_card(harness: &RouteHarness, card_id: i64) -> Option<Value> {
    let listed = call_router_json(harness, Method::GET, "/api/v1/identity-review-card", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.json);
    listed
        .json
        .as_array()
        .and_then(|cards| cards.iter().find(|card| card["id"] == card_id).cloned())
}

async fn listed_generation(harness: &RouteHarness, card_id: i64) -> i64 {
    listed_card(harness, card_id).await.expect("card is listed")["generation"]
        .as_i64()
        .expect("listed generation")
}

async fn resolve_group(
    harness: &RouteHarness,
    ingress: ReviewHttpIngress,
    card_id: i64,
    generation: i64,
    action: Value,
) -> RouteResponse {
    call_router_json(
        harness,
        Method::POST,
        ingress.path(card_id),
        Some(json!({"command": {"GroupIdentity": {
            "card_id": card_id,
            "expected_generation": generation,
            "action": action,
        }}})),
    )
    .await
}

fn with_permitted_answer_changes(before: &Value, card_id: i64, generation: i64) -> Value {
    let mut expected = before.clone();
    expected["distinction"] = json!(format!("different:review:{card_id}"));
    expected["generation"] = json!(generation + 1);
    expected
}

async fn assert_different_book_keeps_proposal_off_the_work(ingress: ReviewHttpIngress) {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let bob = lift_author(db, user, &lift_label("Bob")).await;
    let work = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let card = lift_group_card(
        db,
        user,
        work,
        vec![work],
        Some(lift_proposal(user, bob, "Beta")),
        json!([]),
    )
    .await;
    let generation = listed_generation(&harness, card).await;
    let before = lift_work_row(db, work).await;
    let routes = lift_routes(db, work).await;
    let contributors = lift_contributors(db, work).await;
    let works = lift_works(db, user).await;
    let audits = lift_resolution_audits(db, user).await;

    let response = resolve_group(
        &harness,
        ingress,
        card,
        generation,
        json!("DifferentFromAll"),
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        lift_work_row(db, work).await,
        with_permitted_answer_changes(&before, card, generation),
        "only the distinction and one generation step may change"
    );
    assert_eq!(lift_routes(db, work).await, routes);
    assert_eq!(lift_contributors(db, work).await, contributors);
    assert_eq!(lift_works(db, user).await, works, "no Work is created");
    assert_eq!(lift_resolution_audits(db, user).await, audits + 1);
    assert_eq!(
        lift_card_status(db, card).await.as_deref(),
        Some("resolved")
    );
    assert!(listed_card(&harness, card).await.is_none());
}

// AC-001: "different book" never writes the proposal onto the existing Work.
#[tokio::test]
async fn card_edits_lift_ac001_different_book_via_typed_route() {
    assert_different_book_keeps_proposal_off_the_work(ReviewHttpIngress::Typed).await;
}

// AC-001, legacy alias door.
#[tokio::test]
async fn card_edits_lift_ac001_different_book_via_legacy_alias() {
    assert_different_book_keeps_proposal_off_the_work(ReviewHttpIngress::LegacyAlias).await;
}

async fn assert_group_answer_touches_only_anchor_bookkeeping(with_proposal: bool) {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let bob = lift_author(db, user, &lift_label("Bob")).await;
    let first = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let second = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let proposal = with_proposal.then(|| lift_proposal(user, bob, "Beta"));
    let card = lift_group_card(db, user, first, vec![first, second], proposal, json!([])).await;
    let generation = listed_generation(&harness, card).await;
    let (first_before, second_before) = (
        lift_work_row(db, first).await,
        lift_work_row(db, second).await,
    );
    let (first_routes, second_routes) =
        (lift_routes(db, first).await, lift_routes(db, second).await);
    let audits = lift_resolution_audits(db, user).await;
    let works = lift_works(db, user).await;

    let response = resolve_group(
        &harness,
        ReviewHttpIngress::Typed,
        card,
        generation,
        json!("DifferentFromAll"),
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        lift_work_row(db, first).await,
        with_permitted_answer_changes(&first_before, card, generation)
    );
    assert_eq!(lift_work_row(db, second).await, second_before);
    assert_eq!(lift_routes(db, first).await, first_routes);
    assert_eq!(lift_routes(db, second).await, second_routes);
    assert_eq!(lift_resolution_audits(db, user).await, audits + 1);
    assert_eq!(lift_works(db, user).await, works);
    assert_eq!(
        lift_card_status(db, card).await.as_deref(),
        Some("resolved")
    );
}

// AC-002: a many-book card with a proposal changes no member's identity.
#[tokio::test]
async fn card_edits_lift_ac002_many_book_card_with_proposal() {
    assert_group_answer_touches_only_anchor_bookkeeping(true).await;
}

// AC-003: a card without a proposal gets exactly the permitted changes.
#[tokio::test]
async fn card_edits_lift_ac003_card_without_proposal() {
    assert_group_answer_touches_only_anchor_bookkeeping(false).await;
}

// AC-004: the actionable generation is the one listed when the user decides.
#[tokio::test]
async fn card_edits_lift_ac004_stale_answer_refused_older_mint_actionable() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let bob = lift_author(db, user, &lift_label("Bob")).await;
    let work = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let stale = lift_group_card(
        db,
        user,
        work,
        vec![work],
        Some(lift_proposal(user, bob, "Beta")),
        json!([]),
    )
    .await;
    let seen = listed_generation(&harness, stale).await;
    lift_settle_again(db, user, work, vec![]).await;
    let row = lift_work_row(db, work).await;
    let routes = lift_routes(db, work).await;
    let contributors = lift_contributors(db, work).await;
    let (works, cards, audits) = (
        lift_works(db, user).await,
        lift_pending_cards(db, user).await,
        lift_resolution_audits(db, user).await,
    );

    let refused = resolve_group(
        &harness,
        ReviewHttpIngress::Typed,
        stale,
        seen,
        json!("DifferentFromAll"),
    )
    .await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(refused.json["message"], "stale identity generation");
    assert_eq!(lift_work_row(db, work).await, row);
    assert_eq!(lift_routes(db, work).await, routes);
    assert_eq!(lift_contributors(db, work).await, contributors);
    assert_eq!(lift_works(db, user).await, works);
    assert_eq!(lift_pending_cards(db, user).await, cards);
    assert_eq!(lift_resolution_audits(db, user).await, audits);
    assert_eq!(
        lift_card_status(db, stale).await.as_deref(),
        Some("pending")
    );
    assert_eq!(listed_generation(&harness, stale).await, seen + 1);
    let dismissed = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{stale}/dismiss"),
        None,
    )
    .await;
    assert_eq!(
        dismissed.status,
        StatusCode::NO_CONTENT,
        "{}",
        dismissed.json
    );

    let older = lift_group_card(
        db,
        user,
        work,
        vec![work],
        Some(lift_proposal(user, bob, "Gamma")),
        json!([]),
    )
    .await;
    let minted: i64 = lift_count(
        db,
        "SELECT generation FROM identity_review_cards WHERE id=?1",
        older,
    )
    .await;
    lift_settle_again(db, user, work, vec![]).await;
    let current = listed_generation(&harness, older).await;
    assert!(
        current > minted,
        "fixture: the card predates the anchor's generation"
    );
    let title = lift_work_row(db, work).await["title"].clone();

    let accepted = resolve_group(
        &harness,
        ReviewHttpIngress::Typed,
        older,
        current,
        json!("DifferentFromAll"),
    )
    .await;

    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.json);
    assert_eq!(lift_work_row(db, work).await["title"], title);
}

// AC-005: a deleted non-anchor member leaves an actionable card; a deleted
// anchor takes its card with it.
#[tokio::test]
async fn card_edits_lift_ac005_deleted_member_and_deleted_anchor() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let bob = lift_author(db, user, &lift_label("Bob")).await;
    let mut works = Vec::new();
    for _ in 0..4 {
        works.push(
            lift_work(
                db,
                user,
                lift_tuple(&lift_label("Alpha"), None, None),
                ann,
                vec![lift_route(user, &lift_ol_value())],
            )
            .await,
        );
    }
    let card = lift_group_card(
        db,
        user,
        works[0],
        vec![works[0], works[1]],
        Some(lift_proposal(user, bob, "Beta")),
        json!([]),
    )
    .await;
    let deleted = call_router_json(
        &harness,
        Method::DELETE,
        format!("/api/v1/work/{}", works[1]),
        None,
    )
    .await;
    assert!(deleted.status.is_success(), "{}", deleted.json);
    let generation = listed_generation(&harness, card).await;
    let before = lift_work_row(db, works[0]).await;
    let routes = lift_routes(db, works[0]).await;

    let response = resolve_group(
        &harness,
        ReviewHttpIngress::Typed,
        card,
        generation,
        json!("DifferentFromAll"),
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        lift_work_row(db, works[0]).await,
        with_permitted_answer_changes(&before, card, generation)
    );
    assert_eq!(lift_routes(db, works[0]).await, routes);

    let orphaned = lift_group_card(
        db,
        user,
        works[2],
        vec![works[2], works[3]],
        Some(lift_proposal(user, bob, "Gamma")),
        json!([]),
    )
    .await;
    let orphaned_generation = listed_generation(&harness, orphaned).await;
    let deleted_anchor = call_router_json(
        &harness,
        Method::DELETE,
        format!("/api/v1/work/{}", works[2]),
        None,
    )
    .await;
    assert!(
        deleted_anchor.status.is_success(),
        "{}",
        deleted_anchor.json
    );
    assert!(listed_card(&harness, orphaned).await.is_none());
    let missing = resolve_group(
        &harness,
        ReviewHttpIngress::Typed,
        orphaned,
        orphaned_generation,
        json!("DifferentFromAll"),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.json);
}

async fn lift_cli_database(dir: &Path) -> SqliteDb {
    let pool = livrarr_db::pool::create_sqlite_pool(dir)
        .await
        .expect("open CLI database");
    livrarr_db::pool::run_migrations(&pool)
        .await
        .expect("migrate CLI database");
    let db = SqliteDb::new(pool);
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_identity \
         ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL",
    )
    .execute(db.pool())
    .await
    .expect("install production Author identity index");
    db.ensure_identity_authority_ready()
        .await
        .expect("activate CLI database");
    db
}

async fn lift_cli(
    command: livrarr_server::identity_layer::IdentityCutoverCliCommand,
    dir: &Path,
) -> Result<
    livrarr_server::identity_layer::IdentityCutoverCliOutcome,
    livrarr_server::identity_layer::IdentityCutoverCommandError,
> {
    livrarr_server::identity_layer::run_identity_cutover_command(
        command,
        dir.to_path_buf(),
        CancellationToken::new(),
    )
    .await
}

// AC-006 and the command-line half of AC-012.
#[tokio::test]
async fn card_edits_lift_ac006_cli_show_review_generation_is_actionable() {
    use livrarr_server::identity_layer::{
        IdentityCutoverCliCommand as Cli, IdentityCutoverCliOutcome as Out,
        IdentityCutoverCommandError as CliError,
    };
    let dir = tempfile::tempdir().expect("CLI data dir");
    let db = lift_cli_database(dir.path()).await;
    let user = seed_user(&db, "lift-cli").await;
    let ann = lift_author(&db, user, &lift_label("Ann")).await;
    let bob = lift_author(&db, user, &lift_label("Bob")).await;
    let work = lift_work(
        &db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let card = lift_group_card(
        &db,
        user,
        work,
        vec![work],
        Some(lift_proposal(user, bob, "Beta")),
        json!([]),
    )
    .await;
    let minted = lift_count(
        &db,
        "SELECT generation FROM identity_review_cards WHERE id=?1",
        card,
    )
    .await;
    lift_settle_again(&db, user, work, vec![]).await;
    let before = lift_work_row(&db, work).await;
    let routes = lift_routes(&db, work).await;
    let audits = lift_resolution_audits(&db, user).await;
    db.pool().close().await;

    let listed = match lift_cli(Cli::ListReviews, dir.path()).await {
        Ok(Out::ReviewList(rows)) => rows,
        other => panic!("ListReviews: {other:?}"),
    };
    assert_eq!(
        listed
            .iter()
            .find(|row| row.card_id == card)
            .map(|row| row.generation),
        Some(minted),
        "ListReviews reports the stored mint generation"
    );
    let shown = match lift_cli(Cli::ShowReview { card_id: card }, dir.path()).await {
        Ok(Out::ReviewDetail(detail)) => detail.generation,
        other => panic!("ShowReview: {other:?}"),
    };
    assert_eq!(
        shown,
        minted + 1,
        "ShowReview reports the anchor's current generation"
    );
    let action_file = dir.path().join("different-book.json");
    std::fs::write(&action_file, br#""DifferentFromAll""#).expect("write action file");

    let stale = lift_cli(
        Cli::ResolveReview {
            card_id: card,
            expected_generation: minted,
            action_file: action_file.clone(),
        },
        dir.path(),
    )
    .await;
    assert!(matches!(stale, Err(CliError::StaleGeneration)), "{stale:?}");
    let reopened = SqliteDb::new(
        livrarr_db::pool::create_sqlite_pool(dir.path())
            .await
            .unwrap(),
    );
    assert_eq!(lift_work_row(&reopened, work).await, before);
    reopened.pool().close().await;

    let accepted = lift_cli(
        Cli::ResolveReview {
            card_id: card,
            expected_generation: shown,
            action_file,
        },
        dir.path(),
    )
    .await;
    assert!(
        matches!(accepted, Ok(Out::ReviewResolved(_))),
        "{accepted:?}"
    );
    let reopened = SqliteDb::new(
        livrarr_db::pool::create_sqlite_pool(dir.path())
            .await
            .unwrap(),
    );
    assert_eq!(
        lift_work_row(&reopened, work).await,
        with_permitted_answer_changes(&before, card, shown)
    );
    assert_eq!(lift_routes(&reopened, work).await, routes);
    assert_eq!(
        lift_card_status(&reopened, card).await.as_deref(),
        Some("resolved")
    );
    assert_eq!(lift_resolution_audits(&reopened, user).await, audits + 1);
    assert_eq!(
        lift_count(
            &reopened,
            "SELECT COUNT(*) FROM identity_audit_events WHERE event_kind='review-resolution' \
               AND json_extract(payload, '$.GroupIdentity.card_id')=?1",
            card,
        )
        .await,
        1,
        "exactly one resolution audit row names the card"
    );

    let merge_card = lift_group_card(
        &reopened,
        user,
        work,
        vec![work],
        Some(lift_proposal(user, bob, "Gamma")),
        json!([]),
    )
    .await;
    let merge_generation = lift_row_generation(&reopened, work).await;
    let merge_before = user_state_snapshot(&reopened, user).await;
    reopened.pool().close().await;
    let merge_file = dir.path().join("merge.json");
    std::fs::write(
        &merge_file,
        serde_json::to_vec(&json!({"AttachOrMerge": {"anchor": work}})).unwrap(),
    )
    .expect("write merge action file");
    let merged = lift_cli(
        Cli::ResolveReview {
            card_id: merge_card,
            expected_generation: merge_generation,
            action_file: merge_file,
        },
        dir.path(),
    )
    .await;
    assert!(
        merged.as_ref().err().is_some_and(|error| error
            .to_string()
            .contains("Merging is currently unavailable")),
        "{merged:?}"
    );
    let reopened = SqliteDb::new(
        livrarr_db::pool::create_sqlite_pool(dir.path())
            .await
            .unwrap(),
    );
    assert_eq!(user_state_snapshot(&reopened, user).await, merge_before);
    reopened.pool().close().await;
}

async fn lift_row_generation(db: &SqliteDb, work_id: i64) -> i64 {
    lift_work_row(db, work_id).await["generation"]
        .as_i64()
        .expect("Work generation")
}

async fn lift_seed_plain(harness: &RouteHarness) -> (i64, String, String) {
    let author_name = lift_label("Ann");
    let title = lift_label("Alpha");
    let ann = lift_author(&harness.db, harness.user_id, &author_name).await;
    let work = lift_work(
        &harness.db,
        harness.user_id,
        lift_tuple(&title, None, None),
        ann,
        vec![lift_route(harness.user_id, &lift_ol_value())],
    )
    .await;
    (work, title, author_name)
}

async fn put_work(harness: &RouteHarness, work_id: i64, body: Value) -> RouteResponse {
    call_router_json(
        harness,
        Method::PUT,
        format!("/api/v1/work/{work_id}"),
        Some(body),
    )
    .await
}

async fn get_work(harness: &RouteHarness, work_id: i64) -> Value {
    let response = call_router_json(
        harness,
        Method::GET,
        format!("/api/v1/work/{work_id}"),
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json
}

/// The body the rendered edit dialog sends: every editable field, changed or not.
fn dialog_body(detail: &Value, changes: Value) -> Value {
    let mut body = json!({
        "title": detail["title"],
        "authorName": detail["authorName"],
        "seriesName": detail["seriesName"],
        "seriesPosition": detail["seriesPosition"],
        "monitorEbook": detail["monitorEbook"],
        "monitorAudiobook": detail["monitorAudiobook"],
    });
    for (key, value) in changes.as_object().expect("changes object") {
        body[key] = value.clone();
    }
    body
}

// AC-007: a title edit applies, keeps author, provider ids and distinction,
// and leaves no card.
#[tokio::test]
async fn card_edits_lift_ac007_title_edit_applies_without_a_card() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let (work, _, author_name) = lift_seed_plain(&harness).await;
    let before = lift_work_row(db, work).await;
    let routes = lift_routes(db, work).await;
    let works = lift_works(db, user).await;
    let new_title = lift_label("Gamma");

    let response = put_work(&harness, work, json!({"title": new_title})).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["title"], new_title);
    let detail = get_work(&harness, work).await;
    assert_eq!(detail["title"], new_title);
    assert_eq!(detail["authorName"], author_name);
    let after = lift_work_row(db, work).await;
    assert_eq!(after["main"], new_title.to_lowercase());
    for unchanged in [
        "author",
        "author_id",
        "primary_author_id",
        "distinction",
        "subtitle",
        "volume",
    ] {
        assert_eq!(after[unchanged], before[unchanged], "{unchanged}");
    }
    assert_eq!(lift_routes(db, work).await, routes);
    assert_eq!(lift_pending_cards(db, user).await, 0);
    assert_eq!(lift_works(db, user).await, works);
}

async fn assert_author_edit_keeps_title_tuple(through_dialog: bool) {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let work = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), Some("Beta"), Some("2")),
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let before = lift_work_row(db, work).await;
    let routes = lift_routes(db, work).await;
    let new_author = lift_label("Bob");
    let body = if through_dialog {
        dialog_body(
            &get_work(&harness, work).await,
            json!({"authorName": new_author}),
        )
    } else {
        json!({"authorName": new_author})
    };

    let response = put_work(&harness, work, body).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let after = lift_work_row(db, work).await;
    for unchanged in [
        "title",
        "subtitle",
        "volume",
        "main",
        "nsub",
        "nvol",
        "distinction",
    ] {
        assert_eq!(after[unchanged], before[unchanged], "{unchanged}");
    }
    assert_eq!(after["author"], new_author);
    assert_eq!(after["author_id"], after["primary_author_id"]);
    let stored_name: String = sqlx::query_scalar("SELECT name FROM authors WHERE id=?1")
        .bind(after["author_id"].as_i64().expect("author id"))
        .fetch_one(db.pool())
        .await
        .expect("read Author");
    assert_eq!(stored_name, new_author);
    assert_eq!(get_work(&harness, work).await["authorName"], new_author);
    assert_eq!(lift_routes(db, work).await, routes);
    assert_eq!(lift_pending_cards(db, user).await, 0);
}

// AC-008 (a): API request that omits the title.
#[tokio::test]
async fn card_edits_lift_ac008_author_only_edit_via_api_keeps_title_tuple() {
    assert_author_edit_keeps_title_tuple(false).await;
}

// AC-008 (b): the dialog's unchanged-title submission.
#[tokio::test]
async fn card_edits_lift_ac008_author_only_edit_via_dialog_keeps_title_tuple() {
    assert_author_edit_keeps_title_tuple(true).await;
}

// AC-008: a title-and-author edit applies both.
#[tokio::test]
async fn card_edits_lift_ac008_title_and_author_edit_applies_both() {
    let harness = build_route_harness().await;
    let (work, _, _) = lift_seed_plain(&harness).await;
    let (title, author) = (lift_label("Delta"), lift_label("Dana"));

    let response = put_work(
        &harness,
        work,
        json!({"title": title, "authorName": author}),
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let detail = get_work(&harness, work).await;
    assert_eq!(
        (detail["title"].clone(), detail["authorName"].clone()),
        (json!(title), json!(author))
    );
    assert_eq!(lift_pending_cards(&harness.db, harness.user_id).await, 0);
}

// AC-009: unchanged identity plus a flag change saves the flag only.
#[tokio::test]
async fn card_edits_lift_ac009_unchanged_identity_with_flag_change() {
    let harness = build_route_harness().await;
    let (work, _, _) = lift_seed_plain(&harness).await;
    let detail = get_work(&harness, work).await;
    let flipped = !detail["monitorEbook"].as_bool().expect("monitorEbook");
    let mut expected = lift_work_row(&harness.db, work).await;
    expected["monitor_ebook"] = json!(i64::from(flipped));

    let response = put_work(
        &harness,
        work,
        dialog_body(&detail, json!({"monitorEbook": flipped})),
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(lift_work_row(&harness.db, work).await, expected);
    assert_eq!(lift_pending_cards(&harness.db, harness.user_id).await, 0);
}

// AC-020: title, series and a monitor flag in one dialog Save all persist.
#[tokio::test]
async fn card_edits_lift_ac020_mixed_save_persists_every_field() {
    let harness = build_route_harness().await;
    let (work, _, _) = lift_seed_plain(&harness).await;
    let detail = get_work(&harness, work).await;
    let flipped = !detail["monitorEbook"].as_bool().expect("monitorEbook");
    let (title, series) = (lift_label("Gamma"), lift_label("Saga"));

    let response = put_work(
        &harness,
        work,
        dialog_body(
            &detail,
            json!({"title": title, "seriesName": series, "monitorEbook": flipped}),
        ),
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let saved = get_work(&harness, work).await;
    assert_eq!(saved["title"], title);
    assert_eq!(saved["seriesName"], series);
    assert_eq!(saved["monitorEbook"], flipped);
}

// AC-011: validation classes are unchanged.
#[tokio::test]
async fn card_edits_lift_ac011_empty_or_null_identity_fields_are_422() {
    let harness = build_route_harness().await;
    let (work, _, _) = lift_seed_plain(&harness).await;
    for body in [
        json!({"title": ""}),
        json!({"title": null}),
        json!({"authorName": "  "}),
        json!({"authorName": null}),
    ] {
        let response = put_work(&harness, work, body.clone()).await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body} {}",
            response.json
        );
    }
}

// AC-010: an injected fault inside an edit that changes both title and author
// leaves no trace, and the same edit without the fault is one identity write
// carrying both values, so a title/author pair split across commits fails.
#[tokio::test]
async fn card_edits_lift_ac010_injected_fault_leaves_no_trace() {
    use livrarr_db::identity_layer::{set_identity_db_failpoint_for_tests, IdentityDbFailpoint};
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let (work, _, _) = lift_seed_plain(&harness).await;
    let bob_name = lift_label("Bob");
    let bob = lift_author(db, user, &bob_name).await;
    let before = lift_work_row(db, work).await;
    let routes = lift_routes(db, work).await;
    let contributors = lift_contributors(db, work).await;
    let edit = json!({"title": lift_label("Delta"), "authorName": bob_name});

    set_identity_db_failpoint_for_tests(IdentityDbFailpoint::CommitAfterContributors);
    let response = put_work(&harness, work, edit.clone()).await;
    set_identity_db_failpoint_for_tests(IdentityDbFailpoint::None);

    assert!(
        response.status.is_server_error(),
        "{} {}",
        response.status,
        response.json
    );
    assert_eq!(lift_work_row(db, work).await, before);
    assert_eq!(lift_routes(db, work).await, routes);
    assert_eq!(lift_contributors(db, work).await, contributors);
    assert_eq!(lift_pending_cards(db, user).await, 0);

    let applied = put_work(&harness, work, edit.clone()).await;
    assert_eq!(applied.status, StatusCode::OK, "{}", applied.json);
    let after = lift_work_row(db, work).await;
    assert_eq!(
        (after["title"].clone(), after["primary_author_id"].clone()),
        (edit["title"].clone(), json!(bob))
    );
    assert_eq!(
        after["generation"].as_i64(),
        before["generation"]
            .as_i64()
            .map(|generation| generation + 1),
        "title and author commit in one identity write"
    );
}

// AC-022: a rival identity write between the edit's read and its write wins.
#[tokio::test]
async fn card_edits_lift_ac022_stale_claim_keeps_rival_values() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let (work, _, _) = lift_seed_plain(&harness).await;
    let pause = livrarr_db::identity_layer::install_settlement_pause_for_tests(user, work);
    let edit_a = lift_label("Delta");
    let (app, key, path) = (
        harness.app.clone(),
        harness.api_key.clone(),
        format!("/api/v1/work/{work}"),
    );
    let body_a = json!({"title": edit_a});
    let task =
        tokio::spawn(
            async move { call_app_json(app, &key, Method::PUT, path, Some(body_a)).await },
        );
    tokio::time::timeout(Duration::from_secs(10), pause.wait_until_paused())
        .await
        .expect("edit A reaches its settlement write");
    let (gamma, cara) = (lift_label("Gamma"), lift_label("Cara"));
    let rival = put_work(&harness, work, json!({"title": gamma, "authorName": cara})).await;
    assert_eq!(rival.status, StatusCode::OK, "{}", rival.json);
    pause.release();

    let refused = task.await.expect("edit A task");

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    let after = lift_work_row(db, work).await;
    assert_eq!(
        (after["title"].clone(), after["author"].clone()),
        (json!(gamma), json!(cara))
    );
    assert_eq!(lift_pending_cards(db, user).await, 0);
    assert_eq!(
        lift_count(
            db,
            "SELECT COUNT(*) FROM works WHERE user_id=?1 AND title LIKE 'Delta Lift %'",
            user
        )
        .await,
        0
    );
    let retried = put_work(&harness, work, json!({"title": edit_a})).await;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.json);
}

/// Parks an edit at the road's own read of the Work, after every read its
/// handler made: each captured-identity read of the Work is parked in turn and
/// let through until the recorder shows the edit inside the road.
async fn park_edit_at_road_read(
    harness: &RouteHarness,
    work: i64,
    first: livrarr_db::identity_layer::SettlementPauseGuard,
) -> livrarr_db::identity_layer::SettlementPauseGuard {
    let mut pause = first;
    loop {
        tokio::time::timeout(Duration::from_secs(10), pause.wait_until_paused())
            .await
            .expect("the edit reads the Work");
        let in_road = harness
            .state
            .identity_road
            .test_recorder()
            .snapshot()
            .iter()
            .any(|call| {
                matches!(
                    call,
                    livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
                        if request.existing_work_id == Some(work)
                )
            });
        if in_road {
            return pause;
        }
        drop(pause);
        pause = livrarr_db::identity_layer::install_captured_read_pause_for_tests(
            harness.user_id,
            work,
            0,
        );
    }
}

async fn assert_rival_before_the_road_read_wins(edit_a: Value) {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let (work, _, _) = lift_seed_plain(&harness).await;
    harness.state.identity_road.test_recorder().clear();
    let first = livrarr_db::identity_layer::install_captured_read_pause_for_tests(user, work, 0);
    let (app, key, path, body_a) = (
        harness.app.clone(),
        harness.api_key.clone(),
        format!("/api/v1/work/{work}"),
        edit_a.clone(),
    );
    let task =
        tokio::spawn(
            async move { call_app_json(app, &key, Method::PUT, path, Some(body_a)).await },
        );
    let pause = park_edit_at_road_read(&harness, work, first).await;
    let (gamma, cara) = (lift_label("Gamma"), lift_label("Cara"));
    let rival = put_work(&harness, work, json!({"title": gamma, "authorName": cara})).await;
    assert_eq!(rival.status, StatusCode::OK, "{}", rival.json);
    let rival_row = lift_work_row(db, work).await;
    let rival_routes = lift_routes(db, work).await;
    let rival_credits = lift_contributors(db, work).await;
    pause.release();

    let refused = task.await.expect("edit A task");

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(
        (rival_row["title"].clone(), rival_row["author"].clone()),
        (json!(gamma), json!(cara))
    );
    assert_eq!(
        lift_work_row(db, work).await,
        rival_row,
        "edit A writes nothing"
    );
    assert_eq!(lift_routes(db, work).await, rival_routes);
    assert_eq!(lift_contributors(db, work).await, rival_credits);
    assert_eq!(lift_pending_cards(db, user).await, 0);

    let retried = put_work(&harness, work, edit_a).await;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.json);
}

// AC-022: a rival write after the title-only edit's handler read and before
// the road's read wins; the edit's kept author never claims a later generation.
#[tokio::test]
async fn card_edits_lift_ac022_rival_before_road_read_wins_title_only() {
    assert_rival_before_the_road_read_wins(json!({"title": lift_label("Delta")})).await;
}

// AC-022: the same window for an author-only edit and its kept title.
#[tokio::test]
async fn card_edits_lift_ac022_rival_before_road_read_wins_author_only() {
    assert_rival_before_the_road_read_wins(json!({"authorName": lift_label("Dana")})).await;
}

/// The stored title tuple of a Work, as storage and a later GET report it.
async fn lift_title_tuple(harness: &RouteHarness, work: i64) -> Value {
    let row = lift_work_row(&harness.db, work).await;
    let detail = get_work(harness, work).await;
    json!({
        "title": row["title"],
        "subtitle": row["subtitle"],
        "volume": row["volume"],
        "main": row["main"],
        "nsub": row["nsub"],
        "nvol": row["nvol"],
        "get_title": detail["title"],
        "get_subtitle": detail["subtitle"],
    })
}

// AC-008 / ST-012: author-only edits keep a comma-volume title's whole stored
// tuple, whether the request omits the title or the dialog sends it unchanged;
// a changed title is still split.
#[tokio::test]
async fn card_edits_lift_ac008_author_only_edits_keep_comma_volume_title_tuple() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let parsed =
        ilr::title_parts_from_provider(format!("{}, Vol. 3: Bar", lift_label("Foo")), None)
            .expect("parse fixture title");
    assert_eq!(
        (parsed.subtitle.as_deref(), parsed.volume.as_deref()),
        (Some("Bar"), Some("3")),
        "fixture: a comma-volume title with a subtitle"
    );
    let work = lift_work(
        db,
        user,
        parsed,
        ann,
        vec![lift_route(user, &lift_ol_value())],
    )
    .await;
    let tuple = lift_title_tuple(&harness, work).await;
    let routes = lift_routes(db, work).await;

    let bob = lift_label("Bob");
    let omitted = put_work(&harness, work, json!({"authorName": bob})).await;
    assert_eq!(omitted.status, StatusCode::OK, "{}", omitted.json);
    assert_eq!(
        lift_title_tuple(&harness, work).await,
        tuple,
        "title omitted"
    );
    assert_eq!(lift_work_row(db, work).await["author"], bob);

    let cara = lift_label("Cara");
    let dialog = put_work(
        &harness,
        work,
        dialog_body(&get_work(&harness, work).await, json!({"authorName": cara})),
    )
    .await;
    assert_eq!(dialog.status, StatusCode::OK, "{}", dialog.json);
    assert_eq!(
        lift_title_tuple(&harness, work).await,
        tuple,
        "unchanged dialog title"
    );
    assert_eq!(lift_work_row(db, work).await["author"], cara);
    assert_eq!(lift_routes(db, work).await, routes);
    assert_eq!(lift_pending_cards(db, user).await, 0);

    let changed = format!("{}, Vol. 4: Quux", lift_label("Qux"));
    let expected =
        ilr::title_parts_from_provider(changed.clone(), None).expect("parse changed title");
    let retitled = put_work(&harness, work, json!({"title": changed})).await;
    assert_eq!(retitled.status, StatusCode::OK, "{}", retitled.json);
    let row = lift_work_row(db, work).await;
    assert_eq!(
        (
            row["title"].clone(),
            row["subtitle"].clone(),
            row["volume"].clone(),
            row["nvol"].clone(),
        ),
        (
            json!(expected.main),
            json!(expected.subtitle),
            json!(expected.volume),
            json!(expected.normalized_volume),
        ),
        "a changed title is split"
    );
}

// REQ-002: an author edit whose typed name resolves to an existing Author under
// another spelling shows the typed spelling: that Author is renamed, so every
// Work by the Author shows it, and no second Author is created.
#[tokio::test]
async fn card_edits_lift_author_edit_applies_typed_spelling() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let walter = lift_author(db, user, "Walter Isaacson").await;
    let einstein = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Einstein"), None, None),
        walter,
        vec![],
    )
    .await;
    let jobs = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Jobs"), None, None),
        walter,
        vec![],
    )
    .await;
    let typed = "Walter Isaacson, Jr.";

    let response = put_work(&harness, einstein, json!({"authorName": typed})).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["authorName"], typed, "response");
    assert_eq!(get_work(&harness, einstein).await["authorName"], typed);
    assert_eq!(
        get_work(&harness, jobs).await["authorName"],
        typed,
        "the other Work by the Author"
    );
    assert_eq!(
        lift_count(db, "SELECT COUNT(*) FROM authors WHERE user_id=?1", user).await,
        1,
        "no new Author"
    );
    assert_eq!(
        lift_work_row(db, einstein).await["primary_author_id"],
        json!(walter)
    );
    assert_eq!(lift_pending_cards(db, user).await, 0);
}

/// A Work credited to several Authors in order, each with one role.
async fn lift_credited_work(db: &SqliteDb, user_id: i64, credits: &[(i64, &str)]) -> i64 {
    WorkIdentityRepository::commit_settlement(
        db,
        ilr::SettlementCommit {
            creation_facts: None,
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title: lift_tuple(&lift_label("Alpha"), None, None),
            text_distinction: None,
            contributors: credits
                .iter()
                .enumerate()
                .map(|(ordinal, (author_id, role))| ilr::WorkContributor {
                    user_id,
                    work_id: 0,
                    author_id: *author_id,
                    ordinal: ordinal as u32,
                    roles: vec![ilr::SourcedValue {
                        value: role.to_string(),
                        provenance: ilr::EvidenceProvenance::User,
                        observed_at: Utc::now(),
                    }],
                })
                .collect(),
            routes: vec![lift_route(user_id, &lift_ol_value())],
            absorbed_work_ids: vec![],
            expected_generation: 0,
            review_cards: vec![],
        },
    )
    .await
    .expect("seed credited Work through the settlement writer")
    .identity
    .own_work_id
}

// REQ-002: title-only edits leave every contributor's ordinal and role as
// they were; an author edit replaces only the primary contributor.
#[tokio::test]
async fn card_edits_lift_title_edits_keep_every_contributor_and_role() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let bob = lift_author(db, user, &lift_label("Bob")).await;
    let cara = lift_author(db, user, &lift_label("Cara")).await;
    let work = lift_credited_work(
        db,
        user,
        &[(ann, "author"), (bob, "translator"), (cara, "illustrator")],
    )
    .await;
    let credits = lift_contributors(db, work).await;
    let seeded: Value = serde_json::from_str(&credits).expect("credits JSON");
    assert_eq!(
        seeded
            .as_array()
            .map(|rows| rows.iter().map(|row| row["author_id"].clone()).collect()),
        Some(vec![json!(ann), json!(bob), json!(cara)]),
        "fixture: three ordered credits: {credits}"
    );

    for stem in ["Delta", "Epsilon"] {
        let response = put_work(&harness, work, json!({"title": lift_label(stem)})).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.json);
        assert_eq!(
            lift_contributors(db, work).await,
            credits,
            "after the {stem} title edit"
        );
    }

    let response = put_work(&harness, work, json!({"authorName": lift_label("Dan")})).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let dan = lift_work_row(db, work).await["primary_author_id"]
        .as_i64()
        .expect("primary author");
    assert_ne!(dan, ann);
    let mut expected = seeded.clone();
    expected[0] = json!({"author_id": dan, "ordinal": 0, "roles": []});
    let after: Value =
        serde_json::from_str(&lift_contributors(db, work).await).expect("credits JSON");
    assert_eq!(after, expected, "only the primary contributor changes");
}

const REVIEW_CARD_LIST_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../frontend/src/pages/review/fixtures/groupIdentityCardList.json"
);

/// Replaces each database id in a serialized card list with its stand-in, so
/// a checked-in fixture stays stable across runs. Ids are looked up by the
/// table their field names: ("card", "user", "work" or "author").
fn stand_in_ids(value: Value, ids: &[(&str, i64, i64)], key: Option<&str>) -> Value {
    let table = match key {
        Some("id" | "card_id") => Some("card"),
        Some("userId") => Some("user"),
        Some("workId" | "work_ids" | "work_id") => Some("work"),
        Some("primary_author_id") => Some("author"),
        _ => None,
    };
    match value {
        Value::Number(number) if table.is_some() => {
            let id = number.as_i64().expect("integer id");
            let stand_in = ids
                .iter()
                .find(|(kind, actual, _)| Some(*kind) == table && *actual == id)
                .map(|(_, _, stand_in)| *stand_in)
                .unwrap_or_else(|| panic!("no stand-in for {key:?} {id}"));
            json!(stand_in)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| stand_in_ids(item, ids, key))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(field, item)| {
                    let item = stand_in_ids(item, ids, Some(field.as_str()));
                    (field, item)
                })
                .collect(),
        ),
        other => other,
    }
}

// AC-021: the review page's fixture is the card list the real route serializes
// for a card with a proposal, a card whose proposed Author was deleted, and a
// card without a proposal. Set LIVRARR_WRITE_REVIEW_FIXTURE=1 to rewrite it.
#[tokio::test]
async fn card_edits_lift_ac021_review_fixture_is_the_real_card_list() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, "Ann").await;
    let bob = lift_author(db, user, "Bob").await;
    let carl = lift_author(db, user, "Carl").await;
    let alpha = lift_work(db, user, lift_tuple("Alpha", None, None), ann, vec![]).await;
    let gamma = lift_work(db, user, lift_tuple("Gamma", None, None), ann, vec![]).await;
    let epsilon = lift_work(db, user, lift_tuple("Epsilon", None, None), ann, vec![]).await;
    let proposal = |title, primary_author_id| ilr::WorkIdentityEvidence {
        title,
        primary_author_id,
        routes: vec![],
    };
    let compared = lift_group_card(
        db,
        user,
        alpha,
        vec![alpha],
        Some(proposal(
            lift_tuple("Beta", Some("Proposed"), Some("7")),
            bob,
        )),
        json!([]),
    )
    .await;
    let orphaned = lift_group_card(
        db,
        user,
        gamma,
        vec![gamma],
        Some(proposal(lift_tuple("Delta", None, None), carl)),
        json!([]),
    )
    .await;
    let bare = lift_group_card(db, user, epsilon, vec![epsilon], None, json!([])).await;
    let deleted = call_router_json(
        &harness,
        Method::DELETE,
        format!("/api/v1/author/{carl}"),
        None,
    )
    .await;
    assert!(
        deleted.status.is_success(),
        "{} {}",
        deleted.status,
        deleted.json
    );

    let listed =
        call_router_json(&harness, Method::GET, "/api/v1/identity-review-card", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.json);
    let ids = [
        ("user", user, 1),
        ("author", ann, 8),
        ("author", bob, 9),
        ("author", carl, 10),
        ("card", compared, 17),
        ("card", orphaned, 18),
        ("card", bare, 19),
        ("work", alpha, 71),
        ("work", gamma, 72),
        ("work", epsilon, 73),
    ];
    let payload = stand_in_ids(listed.json, &ids, None);
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&payload).expect("serialize fixture")
    );
    if std::env::var_os("LIVRARR_WRITE_REVIEW_FIXTURE").is_some() {
        std::fs::write(REVIEW_CARD_LIST_FIXTURE, &serialized).expect("write review fixture");
    }
    let fixture = std::fs::read_to_string(REVIEW_CARD_LIST_FIXTURE).expect("read review fixture");
    assert_eq!(
        fixture, serialized,
        "the review page fixture matches the real card list"
    );
}

async fn lift_author_state(db: &SqliteDb, user_id: i64) -> (String, i64) {
    let authors = lift_text(
        db,
        "SELECT COALESCE(json_group_array(json_object('id',id,'name',name,'sort_name',sort_name,\
            'normalized',normalized_name,'monitored',monitored)),'[]') \
         FROM (SELECT * FROM authors WHERE user_id=?1 ORDER BY id)",
        user_id,
    )
    .await;
    let link_rows = lift_count(
        db,
        "SELECT COUNT(*) FROM author_link_progress p JOIN authors a ON a.id=p.author_id WHERE a.user_id=?1",
        user_id,
    )
    .await;
    (authors, link_rows)
}

// AC-023: an edit onto another Work's exact identity is refused before any write.
#[tokio::test]
async fn card_edits_lift_ac023_duplicate_identity_edit_is_refused() {
    let harness = build_activated_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let (ann_name, bob_name) = (lift_label("Ann"), lift_label("Bob"));
    let ann = lift_author(db, user, &ann_name).await;
    let bob = lift_author(db, user, &bob_name).await;
    let beta = lift_label("Beta");
    let x = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![],
    )
    .await;
    let y = lift_work(db, user, lift_tuple(&beta, None, None), bob, vec![]).await;
    let (x_row, y_row) = (lift_work_row(db, x).await, lift_work_row(db, y).await);
    let (x_credits, y_credits) = (
        lift_contributors(db, x).await,
        lift_contributors(db, y).await,
    );
    let authors_before = lift_author_state(db, user).await;

    let refused = put_work(&harness, x, json!({"title": beta, "authorName": bob_name})).await;

    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(
        refused.json["message"],
        "Another book already has this title and author."
    );
    assert_eq!(lift_work_row(db, x).await, x_row);
    assert_eq!(lift_work_row(db, y).await, y_row);
    assert_eq!(lift_contributors(db, x).await, x_credits);
    assert_eq!(lift_contributors(db, y).await, y_credits);
    assert_eq!(lift_works(db, user).await, 2, "nothing is absorbed");
    assert_eq!(lift_pending_cards(db, user).await, 0, "no card");
    assert_eq!(lift_author_state(db, user).await, authors_before);

    let y_before = lift_work_row(db, y).await;
    let allowed = put_work(
        &harness,
        x,
        json!({"title": beta, "authorName": lift_label("Carl")}),
    )
    .await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.json);
    assert_eq!(lift_work_row(db, x).await["title"], beta);
    assert_eq!(lift_work_row(db, y).await, y_before);
    assert_eq!(lift_works(db, user).await, 2);
    assert_eq!(lift_pending_cards(db, user).await, 0);
}

// AC-013: a card minted with merge choices stays listed and dismissible.
#[tokio::test]
async fn card_edits_lift_ac013_merge_choice_card_is_listed_and_dismissible() {
    let harness = build_route_harness().await;
    let (db, user) = (&harness.db, harness.user_id);
    let ann = lift_author(db, user, &lift_label("Ann")).await;
    let first = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![],
    )
    .await;
    let second = lift_work(
        db,
        user,
        lift_tuple(&lift_label("Alpha"), None, None),
        ann,
        vec![],
    )
    .await;
    let card = lift_group_card(
        db,
        user,
        first,
        vec![first, second],
        None,
        json!([{"field": "series_name", "choice": "keep_survivor"}]),
    )
    .await;
    assert!(listed_card(&harness, card).await.is_some());
    let dismissed = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card}/dismiss"),
        None,
    )
    .await;
    assert_eq!(
        dismissed.status,
        StatusCode::NO_CONTENT,
        "{}",
        dismissed.json
    );
    assert!(listed_card(&harness, card).await.is_none());
}
