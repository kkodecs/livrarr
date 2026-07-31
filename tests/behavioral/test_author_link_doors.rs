//! Authenticated production-router door pins for author-provider linking.
//!
//! Every request enters through `livrarr_server::router::build_router`, the
//! real `/api/v1` nesting, and the production authentication middleware.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use chrono::Utc;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    AuthorDb, AuthorLinkDb, AuthorNameVariantDb, CreateAuthorDbRequest, CreateUserDbRequest,
    CreateWorkDbRequest, UserDb, WorkDbCreate,
};
use livrarr_domain::identity_matching::AuthorVerdict;
use livrarr_domain::{
    normalize_for_matching, AuthorCandidateAlternateNameEvidence, AuthorCandidateCatalogState,
    AuthorLinkCandidate, AuthorLinkCandidateReason, AuthorLinkCandidateStatus, AuthorLinkTrigger,
    AuthorNameSource, AuthorProvider, AuthorRouteKey, ProviderAuthorNameObservation, UserRole,
};
use livrarr_metadata as metadata;
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::state::AppState;
use serde_json::Value;
use tower::ServiceExt;

struct RouteHarness {
    app: Router,
    api_key: String,
    db: SqliteDb,
    user_id: i64,
    _tmp: tempfile::TempDir,
}

async fn build_route_harness() -> RouteHarness {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let tmp = tempfile::tempdir().expect("author-link route harness tempdir");
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = "author-link-door-api-key".to_string();
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash route API key");
    let user = db
        .create_user(CreateUserDbRequest {
            username: "author-link-door-admin".to_string(),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create authenticated route user");

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

    let identity_resolver_arc = Arc::new(
        metadata::english_identity_resolver::LiveEnglishIdentityResolver {
            clients: std::collections::HashMap::new(),
            cache: transport_cache,
            config: metadata::english_identity_resolver::ResolverConfig::default(),
        },
    );
    let db_arc = Arc::new(db.clone());
    let queue = Arc::new(metadata::DefaultProviderQueueBuilder::new().build(db_arc.clone()));
    let merge_engine = Arc::new(metadata::DefaultMergeEngine::new(
        metadata::PriorityModel::english(),
    ));
    let enrichment_service = Arc::new(metadata::EnrichmentServiceImpl::new(
        db_arc,
        queue.clone(),
        merge_engine,
        false,
    ));
    let work_service_arc: Arc<livrarr_server::state::LiveWorkService> = {
        let workflow = metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
            enrichment_service.clone(),
        );
        Arc::new(
            metadata::work_service::WorkServiceImpl::new(
                db.clone(),
                workflow,
                http_fetcher.clone(),
                data_dir.clone(),
            )
            .with_resolver(identity_resolver_arc.clone()),
        )
    };
    let discovery_service_arc = Arc::new(
        metadata::discovery_service::DiscoveryServiceImpl::new(
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

    let harness_db = db.clone();
    let state = AppState {
        db: db.clone(),
        auth_service,
        http_client: http_client.clone(),
        http_client_safe,
        http_fetcher: http_fetcher.clone(),
        config: Arc::new(livrarr_server::config::AppConfig::default()),
        data_dir: data_dir_arc.clone(),
        startup_time: chrono::Utc::now(),
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
        author_service: Arc::new(metadata::author_service::AuthorServiceImpl::new(
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
        series_service: Arc::new(metadata::series_service::SeriesServiceImpl::new(db.clone())),
        series_query_service: Arc::new(
            metadata::series_query_service::SeriesQueryServiceImpl::new(
                db.clone(),
                http_fetcher.clone(),
                work_service_arc.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    llm_http_client.clone(),
                ),
            ),
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
            Arc::new(metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                Arc::new(db.clone()),
                Arc::new(http_fetcher.clone()),
                release_service,
            ))
        },
        list_service: {
            let workflow = metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            );
            let work_service = metadata::work_service::WorkServiceImpl::new(
                db.clone(),
                workflow,
                http_fetcher.clone(),
                data_dir.clone(),
            );
            Arc::new(metadata::list_service::ListServiceImpl::new(
                db.clone(),
                work_service,
                http_fetcher.clone(),
                metadata::list_service::NoOpBibliographyTrigger,
            ))
        },
        identity_conflict_service: Arc::new(
            livrarr_server::services::identity_conflict_service::LiveIdentityConflictService::new(
                db.clone(),
            ),
        ),
        identity_resolver: identity_resolver_arc,
        enrichment_workflow: Arc::new(
            metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            ),
        ),
        author_monitor_workflow: {
            let workflow = metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            );
            let work_service = metadata::work_service::WorkServiceImpl::new(
                db.clone(),
                workflow,
                http_fetcher.clone(),
                data_dir.clone(),
            );
            Arc::new(
                metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::new(
                    Arc::new(db.clone()),
                    Arc::new(work_service),
                    Arc::new(http_fetcher.clone()),
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
                db,
                import_workflow_arc,
            ),
        ),
        cover_service,
        preadd_cover_service: Arc::new(
            metadata::preadd_cover_service::LivePreaddCoverService::new(
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
        app: livrarr_server::router::build_router(state, ui_dir),
        api_key,
        db: harness_db,
        user_id: user.id,
        _tmp: tmp,
    }
}

fn route(raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw)
        .expect("door fixture route must be canonical")
}

async fn seed_author(harness: &RouteHarness, label: &str) -> (i64, i64) {
    let name = format!("{label} Author");
    let (author, created) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: name.clone(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("production door author writer");
    assert!(created);
    let title = format!("{label} Work");
    let (work, work_created) = harness
        .db
        .create_work(CreateWorkDbRequest {
            user_id: harness.user_id,
            title: title.clone(),
            author_name: name.clone(),
            normalized_title: normalize_for_matching(&title),
            normalized_author: normalize_for_matching(&name),
            author_id: Some(author.id),
            language: Some("en".to_string()),
            ..Default::default()
        })
        .await
        .expect("production door work writer");
    assert!(work_created);
    (author.id, work.id)
}

fn review_candidate(author_id: i64, raw: &str, name: &str) -> AuthorLinkCandidate {
    AuthorLinkCandidate {
        id: 0,
        author_id,
        key: route(raw),
        candidate_name: name.to_string(),
        reason: AuthorLinkCandidateReason::Tier2NameSearch,
        name_verdict: AuthorVerdict::Agree,
        primary_name_verdict: AuthorVerdict::Agree,
        alternate_name_evidence: vec![AuthorCandidateAlternateNameEvidence {
            name: format!("{name} Alias"),
            verdict: AuthorVerdict::Agree,
        }],
        top_work_preview: Some("Door Work Preview".to_string()),
        catalog_evidence_state: AuthorCandidateCatalogState::Complete,
        corroborated_title_count: 1,
        settled_work_count: 1,
        previously_removed: false,
        status: AuthorLinkCandidateStatus::Pending,
        evidence_generation: 0,
        observed_at: Utc::now(),
        evidence_work_id: None,
        evidence_work_title: None,
        revoked_at: None,
    }
}

async fn seed_candidate(harness: &RouteHarness, author_id: i64, raw: &str, name: &str) -> i64 {
    harness
        .db
        .ensure_enqueued(harness.user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("production door enqueue writer");
    let now = Utc::now();
    let claim = harness
        .db
        .claim_due(now, now + chrono::Duration::minutes(5), 10)
        .await
        .expect("production door claim writer")
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("door author claim");
    harness
        .db
        .record_candidates(claim, vec![review_candidate(author_id, raw, name)])
        .await
        .expect("production door candidate writer");
    harness
        .db
        .list_review(harness.user_id)
        .await
        .expect("door review rows")
        .into_iter()
        .find(|review| review.author.id == author_id)
        .expect("seeded review author")
        .candidates[0]
        .id
}

async fn seed_name_variant(harness: &RouteHarness, work_id: i64, name: &str) -> i64 {
    harness
        .db
        .record_observed_names(
            harness.user_id,
            work_id,
            &[ProviderAuthorNameObservation {
                source: AuthorNameSource::OpenLibrary,
                name: name.to_string(),
            }],
        )
        .await
        .expect("production observed-name writer");
    sqlx::query_scalar(
        "SELECT id FROM author_name_variants
          WHERE user_id=? AND name=?
          ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .bind(name)
    .fetch_one(harness.db.pool())
    .await
    .expect("seeded variant id")
}

async fn request(
    harness: &RouteHarness,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-api-key", &harness.api_key);
    let request_body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request
        .body(request_body)
        .expect("build author-link request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 176)),
        31000,
    )));
    harness
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("production author-link route response")
}

async fn response_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("author-link response body");
    serde_json::from_slice(&bytes).expect("author-link JSON response")
}

/// Door: Review Authors list via the production router and auth middleware.
/// AC-007: authenticated users see current-generation pending review.
#[tokio::test]
async fn ac007_real_router_drives_review_list_door() {
    let harness = build_route_harness().await;
    let (author_id, _work_id) = seed_author(&harness, "Review List").await;
    let candidate_id = seed_candidate(&harness, author_id, "OL9911A", "Review List Author").await;
    let response = request(&harness, Method::GET, "/api/v1/author-link-review", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let rows = body.as_array().expect("review JSON array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["author"]["id"], author_id);
    assert_eq!(rows[0]["candidates"][0]["id"], candidate_id);
    assert_eq!(
        rows[0]["candidates"][0]["candidate_name"],
        "Review List Author"
    );
    assert_eq!(
        rows[0]["candidates"][0]["alternate_name_evidence"][0]["name"],
        "Review List Author Alias"
    );
    assert_eq!(
        rows[0]["candidates"][0]["catalog_evidence_state"],
        "complete"
    );
}

/// Door: Review Authors candidate pick via the production router.
/// AC-007 / AC-010: the candidate id reaches the user-sovereign pick service.
#[tokio::test]
async fn ac007_ac010_real_router_drives_candidate_pick_door() {
    let harness = build_route_harness().await;
    let (author_id, _work_id) = seed_author(&harness, "Candidate Pick").await;
    let candidate_id =
        seed_candidate(&harness, author_id, "OL9921A", "Candidate Pick Author").await;
    let response = request(
        &harness,
        Method::POST,
        &format!("/api/v1/author-link-review/{candidate_id}/pick"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["provider"], "open_library");
    assert_eq!(body["value"], "OL9921A");
    assert_eq!(body["state"], "active");
    assert_eq!(body["provenance"], "user_picked");

    let routes = harness
        .db
        .list_active_routes(harness.user_id, author_id, None)
        .await
        .expect("picked route rows");
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].key, route("OL9921A"));
    let status: String = sqlx::query_scalar("SELECT status FROM author_link_candidates WHERE id=?")
        .bind(candidate_id)
        .fetch_one(harness.db.pool())
        .await
        .expect("picked candidate state");
    assert_eq!(status, "picked");
    let names: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, open_library_role FROM author_name_variants
          WHERE user_id=? AND author_id=? AND source='open_library'
          ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(author_id)
    .fetch_all(harness.db.pool())
    .await
    .expect("candidate-pick names");
    assert_eq!(
        names,
        vec![
            (
                "Candidate Pick Author".to_string(),
                Some("primary".to_string())
            ),
            (
                "Candidate Pick Author Alias".to_string(),
                Some("alias".to_string())
            ),
        ]
    );
}

/// Door: Review Authors candidate dismiss via the production router.
/// AC-007: dismissal is authenticated and distinct from route removal.
#[tokio::test]
async fn ac007_real_router_drives_candidate_dismiss_door() {
    let harness = build_route_harness().await;
    let (author_id, _work_id) = seed_author(&harness, "Candidate Dismiss").await;
    let candidate_id =
        seed_candidate(&harness, author_id, "OL9931A", "Candidate Dismiss Author").await;
    let response = request(
        &harness,
        Method::POST,
        &format!("/api/v1/author-link-review/{candidate_id}/dismiss"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let status: String = sqlx::query_scalar("SELECT status FROM author_link_candidates WHERE id=?")
        .bind(candidate_id)
        .fetch_one(harness.db.pool())
        .await
        .expect("dismissed candidate state");
    assert_eq!(status, "dismissed");
    assert!(harness
        .db
        .list_active_routes(harness.user_id, author_id, None)
        .await
        .expect("routes after candidate dismissal")
        .is_empty());
}

/// Door: Author route removal via the production router.
/// AC-010: the author/route tuple reaches the tombstone operation.
#[tokio::test]
async fn ac010_real_router_drives_route_removal_door() {
    let harness = build_route_harness().await;
    let (author_id, _work_id) = seed_author(&harness, "Route Removal").await;
    let active = harness
        .db
        .attach_route_as_user(harness.user_id, author_id, route("OL9941A"))
        .await
        .expect("production route writer");
    let response = request(
        &harness,
        Method::DELETE,
        &format!("/api/v1/author/{author_id}/route/{}", active.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let tombstone: (String, Option<String>) =
        sqlx::query_as("SELECT state, removed_at FROM author_provider_routes WHERE id=?")
            .bind(active.id)
            .fetch_one(harness.db.pool())
            .await
            .expect("route tombstone");
    assert_eq!(tombstone.0, "removed");
    assert!(tombstone.1.is_some());
    let progress: (String, i64) = sqlx::query_as(
        "SELECT trigger, julianday(next_attempt_at) <= julianday('now')
           FROM author_link_progress WHERE author_id=?",
    )
    .bind(author_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("removal progress");
    assert_eq!(progress.0, "user_re_resolve");
    assert_eq!(progress.1, 1, "removal must leave a due task");
}

/// Door: Author re-resolution via the production router.
/// AC-006 / AC-007: a real author id is required before durable enqueue.
#[tokio::test]
async fn ac006_ac007_real_router_drives_async_reresolve_door() {
    let harness = build_route_harness().await;
    let (author_id, _work_id) = seed_author(&harness, "Re Resolve").await;
    let response = request(
        &harness,
        Method::POST,
        &format!("/api/v1/author/{author_id}/resolve"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let progress: (String, String, i64) = sqlx::query_as(
        "SELECT state, trigger, julianday(next_attempt_at) <= julianday('now')
           FROM author_link_progress WHERE author_id=?",
    )
    .bind(author_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("re-resolve progress");
    assert_eq!(progress.0, "queued");
    assert_eq!(progress.1, "user_re_resolve");
    assert_eq!(progress.2, 1, "re-resolve must be immediately due");
}

/// Door: Author rename via the production router.
/// AC-008 / AC-009: display-name cascade remains a classified user action.
#[tokio::test]
async fn ac008_ac009_real_router_drives_author_rename_door() {
    let harness = build_route_harness().await;
    let (author_id, work_id) = seed_author(&harness, "Rename Door").await;
    let normalized_before: String =
        sqlx::query_scalar("SELECT normalized_author FROM works WHERE id=?")
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .expect("normalized author before rename");
    let response = request(
        &harness,
        Method::PUT,
        &format!("/api/v1/author/{author_id}/name"),
        Some(serde_json::json!({"name": "Renamed Door Author"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["id"], author_id);
    assert_eq!(body["name"], "Renamed Door Author");
    let row: (String, String, String) = sqlx::query_as(
        "SELECT a.name, w.author_name, w.normalized_author
           FROM authors a JOIN works w ON w.author_id=a.id
          WHERE a.id=? AND w.id=?",
    )
    .bind(author_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("rename cascade");
    assert_eq!(row.0, "Renamed Door Author");
    assert_eq!(row.1, "Renamed Door Author");
    assert_eq!(row.2, normalized_before);
}

/// Door: Stored author-name variant pick via the production router.
/// AC-008 / AC-009: a variant id reaches the shared cascade policy.
#[tokio::test]
async fn ac008_ac009_real_router_drives_display_name_pick_door() {
    let harness = build_route_harness().await;
    let (author_id, work_id) = seed_author(&harness, "Display Pick").await;
    let variant_id = seed_name_variant(&harness, work_id, "Chosen Display Author").await;
    let response = request(
        &harness,
        Method::PUT,
        &format!("/api/v1/author/{author_id}/display-name"),
        Some(serde_json::json!({"variantId": variant_id})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["name"], "Chosen Display Author");
    let row: (String, String, Option<String>) = sqlx::query_as(
        "SELECT a.name, w.author_name, v.user_selected_at
           FROM authors a
           JOIN works w ON w.author_id=a.id
           JOIN author_name_variants v ON v.author_id=a.id
          WHERE a.id=? AND w.id=? AND v.id=?",
    )
    .bind(author_id)
    .bind(work_id)
    .bind(variant_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("display-name cascade");
    assert_eq!(row.0, "Chosen Display Author");
    assert_eq!(row.1, "Chosen Display Author");
    assert!(
        row.2.is_some(),
        "selected variant must record user authority"
    );
}

/// Door: Author-link sweep progress via the production router.
/// AC-006 / AC-007: user-scoped durable progress is externally observable.
#[tokio::test]
async fn ac006_ac007_real_router_drives_sweep_progress_door() {
    let harness = build_route_harness().await;
    let (author_id, _work_id) = seed_author(&harness, "Sweep Progress").await;
    harness
        .db
        .ensure_enqueued(harness.user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("production progress writer");
    let response = request(
        &harness,
        Method::GET,
        "/api/v1/author-link-sweep/progress",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["queued"], 1);
    assert_eq!(body["running"], 0);
    assert_eq!(body["parked"], 0);
    assert_eq!(body["needs_review"], 0);
    assert_eq!(body["key_retryable"], 0);
    assert!(body["oldest_due_at"].is_string());
}
