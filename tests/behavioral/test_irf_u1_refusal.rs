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
    let db = create_test_db().await;
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
    let mut request = Request::builder()
        .method(method)
        .uri(path.into())
        .header("x-api-key", &harness.api_key);
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
    let response = harness
        .app
        .clone()
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

// PIN: GroupIdentity and PendingRoute remain the only available continuations.
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

async fn mint_group_via_manual_merge(harness: &RouteHarness, label: &str) -> (i64, i64, i64, i64) {
    let (survivor, _) = seed_work(&harness.db, harness.user_id, &format!("{label}-survivor")).await;
    let (loser, _) = seed_work(&harness.db, harness.user_id, &format!("{label}-loser")).await;
    let generation = work_generation(&harness.db, survivor).await;
    let preview = call_router_json(
        harness,
        Method::GET,
        format!("/api/v1/work/{survivor}/merge/{loser}/preview"),
        None,
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    let minted = call_router_json(
        harness,
        Method::POST,
        format!("/api/v1/work/{survivor}/merge/{loser}"),
        Some(json!({"choices": []})),
    )
    .await;
    assert_eq!(minted.status, StatusCode::ACCEPTED, "{}", minted.json);
    assert_eq!(minted.json["kind"], "GroupIdentity");
    let card_id = minted.json["cardId"]
        .as_i64()
        .expect("manual merge returns card id");
    let expected_generation = minted.json["expectedGeneration"]
        .as_i64()
        .expect("manual merge returns scalar claim");
    assert_eq!(expected_generation, generation + 1);
    (survivor, loser, card_id, expected_generation)
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

async fn assert_group_available_through(ingress: ReviewHttpIngress) {
    let harness = build_route_harness().await;
    let (survivor, _loser, card_id, generation) =
        mint_group_via_manual_merge(&harness, "available-group").await;
    let response = call_router_json(
        &harness,
        Method::POST,
        ingress.path(card_id),
        Some(json!({"command": {
            "GroupIdentity": {
                "card_id": card_id,
                "expected_generation": generation,
                "action": "DifferentFromAll"
            }
        }})),
    )
    .await;
    assert!(response.status.is_success(), "{}", response.json);
    let status: String =
        sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(card_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("read GroupIdentity status");
    assert_eq!(status, "resolved");
    assert_eq!(work_generation(&harness.db, survivor).await, generation + 1);
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

// PIN: the typed and legacy-alias doors retain the current GroupIdentity continuation (including its wave-B defect).
#[tokio::test]
async fn group_identity_remains_available_through_both_review_routes() {
    assert_group_available_through(ReviewHttpIngress::Typed).await;
    assert_group_available_through(ReviewHttpIngress::LegacyAlias).await;
}

// PIN: the typed and legacy-alias doors retain the current PendingRoute continuation.
#[tokio::test]
async fn pending_route_remains_available_through_both_review_routes() {
    assert_pending_available_through(ReviewHttpIngress::Typed).await;
    assert_pending_available_through(ReviewHttpIngress::LegacyAlias).await;
}

// PIN: inline update still uses GroupIdentity::DifferentFromAll and preserves today's mutation/audit/response.
#[tokio::test]
async fn inline_work_update_different_from_all_is_unchanged() {
    let harness = build_route_harness().await;
    let (work_id, _) = seed_work(&harness.db, harness.user_id, "inline-update").await;
    let generation = work_generation(&harness.db, work_id).await;
    harness.state.identity_road.test_recorder().clear();

    let response = call_router_json(
        &harness,
        Method::PUT,
        format!("/api/v1/work/{work_id}"),
        Some(json!({"title": "U1 Explicit Updated Title"})),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["id"], work_id);
    assert_eq!(response.json["title"], "U1 Explicit Updated Title");

    let calls = harness.state.identity_road.test_recorder().snapshot();
    assert_eq!(calls.len(), 2, "update is one settle plus one continuation");
    assert!(matches!(
        &calls[0],
        livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
            if request.origin == ilr::IdentityRoadOrigin::WorkUpdateRekey
    ));
    let (card_id, expected_generation) = match &calls[1] {
        livrarr_server::identity_layer::IdentityRoadCall::Resolve {
            actor: ReviewActor::AuthenticatedUser { user_id },
            command:
                ReviewResolutionCommand::GroupIdentity {
                    card_id,
                    expected_generation,
                    action: ilr::GroupIdentityAction::DifferentFromAll,
                },
        } if *user_id == harness.user_id => (*card_id, *expected_generation),
        other => panic!("update continuation changed: {other:?}"),
    };
    assert_eq!(expected_generation, generation + 1);
    assert_eq!(work_generation(&harness.db, work_id).await, generation + 2);
    let (status, card_generation, audits): (String, i64, i64) = sqlx::query_as(
        "SELECT c.status, c.generation, (SELECT COUNT(*) FROM identity_audit_events a \
            WHERE a.user_id=?1 AND a.event_kind='review-resolution' \
              AND json_extract(a.payload, '$.GroupIdentity.card_id')=?2) \
         FROM identity_review_cards c WHERE c.user_id=?1 AND c.id=?2",
    )
    .bind(harness.user_id)
    .bind(card_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read inline update audit/card");
    assert_eq!(
        (status.as_str(), card_generation, audits),
        ("resolved", generation + 1, 1)
    );
}

// PIN: inline merge-with-choices still uses GroupIdentity::AttachOrMerge, including the known no-archive wave-B defect.
#[tokio::test]
async fn inline_merge_with_choices_attach_or_merge_is_unchanged() {
    let harness = build_route_harness().await;
    let (survivor, _) = seed_work(&harness.db, harness.user_id, "inline-merge-survivor").await;
    let (loser, _) = seed_work(&harness.db, harness.user_id, "inline-merge-loser").await;
    let generation = work_generation(&harness.db, survivor).await;
    let preview = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/work/{survivor}/merge/{loser}/preview"),
        None,
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.json);
    harness.state.identity_road.test_recorder().clear();

    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{survivor}/merge/{loser}"),
        Some(json!({"choices": [{
            "field": "series_name",
            "choice": "keep_survivor"
        }]})),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["survivor"]["id"], survivor);
    assert_eq!(response.json["libraryItemsMoved"], 0);
    assert_eq!(response.json["grabsMoved"], 0);

    let calls = harness.state.identity_road.test_recorder().snapshot();
    assert_eq!(calls.len(), 2, "merge is one settle plus one continuation");
    assert!(matches!(
        &calls[0],
        livrarr_server::identity_layer::IdentityRoadCall::Settle(request)
            if matches!(
                &request.origin,
                ilr::IdentityRoadOrigin::ManualWorkMerge { loser_work_id, .. }
                    if *loser_work_id == loser
            )
    ));
    let (card_id, expected_generation) = match &calls[1] {
        livrarr_server::identity_layer::IdentityRoadCall::Resolve {
            actor: ReviewActor::AuthenticatedUser { user_id },
            command:
                ReviewResolutionCommand::GroupIdentity {
                    card_id,
                    expected_generation,
                    action: ilr::GroupIdentityAction::AttachOrMerge { anchor },
                },
        } if *user_id == harness.user_id && *anchor == survivor => (*card_id, *expected_generation),
        other => panic!("merge continuation changed: {other:?}"),
    };
    assert_eq!(expected_generation, generation + 1);
    assert_eq!(work_generation(&harness.db, survivor).await, generation + 2);
    let loser_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(loser)
            .fetch_one(harness.db.pool())
            .await
            .expect("read known current merge effect");
    assert_eq!(loser_count, 0, "pin only; wave B owns archival correctness");
    let (status, audits): (String, i64) = sqlx::query_as(
        "SELECT c.status, (SELECT COUNT(*) FROM identity_audit_events a \
            WHERE a.user_id=?1 AND a.event_kind='review-resolution' \
              AND json_extract(a.payload, '$.GroupIdentity.card_id')=?2) \
         FROM identity_review_cards c WHERE c.user_id=?1 AND c.id=?2",
    )
    .bind(harness.user_id)
    .bind(card_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("read inline merge audit/card");
    assert_eq!((status.as_str(), audits), ("resolved", 1));
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
async fn typed_mapper_keeps_review_proposal_invalidation_at_409() {
    let harness = build_route_harness().await;
    let (survivor, loser, card_id, _) =
        mint_group_via_manual_merge(&harness, "mapper-invalidated").await;
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
        "review proposal invalidated: proposed merge work no longer exists"
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
