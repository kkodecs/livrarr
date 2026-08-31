//! RED-first behavioral coverage for identity-review-fixes U5 (REQ-005 / AC-005).
//!
//! This is a standalone `[[test]]` target. HTTP cases traverse the production
//! router and authentication middleware. Every database is a real `SqliteDb`
//! from `create_activated_test_db()` (the deployed post-activation shape), and
//! every ordinary fixture is created through the production writer named by
//! AC-005. The only direct SQL construction is explicitly labelled where the
//! state can exist only because it predates U7/U5 or is malformed upgrade data.
//! Provider responses are hermetic `StubProviderClient`/`StubHttpFetcher`
//! values; this suite performs no provider network HTTP.
//!
//! U5 boundary: no EditionEvidence revocation, no legacy-conflict migration or
//! other wave-B surface, no cutover-staging suppression, and no HTTP/CLI field,
//! flag, or action-shape change. Every test is intended to be GREEN in U5.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_activated_test_db;
use livrarr_db::{
    AuthorDb, AuthorLinkDb, CreateAuthorDbRequest, CreateAuthorGateRequest, CreateSeriesDbRequest,
    CreateUserDbRequest, CreateWorkDbRequest, RootFolderDb, SeriesDb, UpdateAuthorDbRequest,
    UpdateWorkEnrichmentDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::AnchorType;
use livrarr_domain::identity_layer::{self as ilr};
use livrarr_domain::identity_layer::{
    EditionRepository, IdentityRoadService, ReviewActor, ReviewResolutionCommand,
    WorkIdentityRepository,
};
use livrarr_domain::services::WorkIdentityRepository as LegacyWorkIdentityRepository;
use livrarr_domain::services::{
    AuthorMonitorWorkflow, ListService, SeriesQueryService, WorkService,
};
use livrarr_domain::{
    AuthorLinkTrigger, AuthorNameSource, AuthorProvider, AuthorRouteKey, MediaType, UserRole,
};
use livrarr_server::auth_crypto::{AuthCryptoService, RealAuthCrypto};
use livrarr_server::state::AppState;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tracing_test::traced_test;

static CASE_ID: AtomicU64 = AtomicU64::new(1);
static TRACE_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

const STANDING_DISMISSAL: &str = "standing dismissal";
const ADOPTION_MARKER: &str = "identity_review_dismissal_adoption_v1";

const U5_SERIES_ROSTER: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;U5 Series&quot;,&quot;subtitle&quot;:&quot;1 primary work • 1 total work&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47212&quot;,&quot;title&quot;:&quot;Suppressed Series Book (U5 Series, #1)&quot;,&quot;bookTitleBare&quot;:&quot;Suppressed Series Book&quot;,&quot;publicationDate&quot;:&quot;2026&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:1,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

const U5_SERIES_EXISTING_MATCH_ROSTER: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;U5 Bound Series&quot;,&quot;subtitle&quot;:&quot;1 primary work • 1 total work&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47213&quot;,&quot;title&quot;:&quot;U5 Incoming Series Identity (U5 Bound Series, #1)&quot;,&quot;bookTitleBare&quot;:&quot;U5 Incoming Series Identity&quot;,&quot;publicationDate&quot;:&quot;2026&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:1,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

struct RouteHarness {
    app: Router,
    state: AppState,
    api_key: String,
    db: SqliteDb,
    user_id: i64,
    open_library_stub: Option<livrarr_external_data::StubProviderClient>,
    _tmp: tempfile::TempDir,
}

struct RouteResponse {
    status: StatusCode,
    json: Value,
}

async fn build_route_harness() -> RouteHarness {
    build_route_harness_with_providers(None, Vec::new(), false).await
}

async fn build_route_harness_with_empty_author_search() -> RouteHarness {
    build_route_harness_with_providers(None, Vec::new(), true).await
}

async fn build_route_harness_with_open_library(
    detail: livrarr_external_data::NormalizedWorkDetail,
) -> RouteHarness {
    build_route_harness_with_providers(Some(detail), Vec::new(), false).await
}

async fn build_route_harness_with_providers(
    open_library_detail: Option<livrarr_external_data::NormalizedWorkDetail>,
    identity_details: Vec<(
        livrarr_domain::MetadataProvider,
        livrarr_external_data::NormalizedWorkDetail,
    )>,
    stub_empty_author_search: bool,
) -> RouteHarness {
    let db = create_activated_test_db().await;
    let tmp = tempfile::tempdir().expect("U5 route harness tempdir");
    let data_dir = tmp.path().to_path_buf();
    let data_dir_arc = Arc::new(data_dir.clone());

    let api_key = format!("irf-u5-admin-{}", CASE_ID.fetch_add(1, Ordering::Relaxed));
    let api_key_hash = RealAuthCrypto
        .hash_token(&api_key)
        .await
        .expect("hash U5 API key");
    let user = db
        .create_user(CreateUserDbRequest {
            username: format!("irf-u5-admin-{}", CASE_ID.fetch_add(1, Ordering::Relaxed)),
            password_hash: "unused-password-hash".to_string(),
            role: UserRole::Admin,
            api_key_hash,
        })
        .await
        .expect("create authenticated U5 user");
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
    let http_fetcher = livrarr_http::fetcher::HttpFetcherImpl::new()
        .expect("shared HTTP fetcher")
        .with_scripted_transport(move |request| {
            if stub_empty_author_search
                && request
                    .url
                    .starts_with("https://openlibrary.org/search/authors.json?")
            {
                return livrarr_http::fetcher::ScriptedTransportOutcome::Response {
                    delay: Duration::ZERO,
                    response: livrarr_domain::services::FetchResponse {
                        status: 200,
                        headers: Vec::new(),
                        body: br#"{"numFound":0,"docs":[]}"#.to_vec(),
                    },
                };
            }
            panic!("U5 attempted provider HTTP: {}", request.url)
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
    let identity_clients = identity_details
        .into_iter()
        .map(|(provider, detail)| {
            (
                provider,
                livrarr_external_data::ProviderClient::Stub(
                    livrarr_external_data::StubProviderClient::new(
                        provider,
                        livrarr_external_data::ProviderOutcome::Success(Box::new(detail)),
                    ),
                ),
            )
        })
        .collect();
    let identity_resolver_arc = livrarr_server::state::build_live_identity_resolver(
        identity_clients,
        transport_cache,
        livrarr_metadata::english_identity_resolver::ResolverConfig::default(),
    );

    let import_semaphore = Arc::new(tokio::sync::Semaphore::new(2));
    let cover_proxy_cache = Arc::new(livrarr_server::infra::cover_cache::CoverProxyCache::new());
    let rss_last_run = Arc::new(AtomicI64::new(0));
    let rss_sync_running = Arc::new(AtomicBool::new(false));
    let manual_import_scans: Arc<livrarr_server::state::ManualImportScanMap> =
        Arc::new(Default::default());
    let log_buffer = Arc::new(livrarr_server::state::LogBuffer::new());
    let log_level_handle = {
        let (_layer, handle) =
            tracing_subscriber::reload::Layer::new(tracing_subscriber::EnvFilter::new("info"));
        Arc::new(livrarr_server::state::LogLevelHandle::new(handle, "info"))
    };
    let settings_service =
        Arc::new(livrarr_server::services::settings_service::LiveSettingsService::new(db.clone()));
    let import_io = Arc::new(livrarr_server::import_io_service::ImportIoServiceImpl::new(
        db.clone(),
    ));
    let import_workflow = Arc::new(livrarr_library::import_workflow::ImportWorkflowImpl::new(
        db.clone(),
        import_semaphore.clone(),
        data_dir_arc.clone(),
        Arc::new(livrarr_server::chapter_extractor::ChapterExtractorImpl),
    ));
    let tag_service = Arc::new(livrarr_server::tag_service::LiveTagService::new(
        import_io.clone(),
        data_dir_arc.clone(),
        db.clone(),
    ));
    let import_service = Arc::new(livrarr_server::import_service::LiveImportService::new(
        import_io.clone(),
        import_workflow.clone(),
        tag_service.clone(),
        settings_service.clone(),
        http_client_safe.clone(),
    ));
    let trusted_origins = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    let readarr_import_service =
        Arc::new(livrarr_server::readarr_import_service::LiveReadarrImportService::new(db.clone()));
    let readarr_import_progress = Arc::new(tokio::sync::Mutex::new(
        livrarr_server::readarr_import_service::ReadarrImportProgress::default(),
    ));

    let db_arc = Arc::new(db.clone());
    let mut queue_builder = livrarr_metadata::DefaultProviderQueueBuilder::new();
    let open_library_stub = open_library_detail.map(|detail| {
        livrarr_external_data::StubProviderClient::new(
            livrarr_domain::MetadataProvider::OpenLibrary,
            livrarr_external_data::ProviderOutcome::Success(Box::new(detail)),
        )
    });
    if let Some(stub) = open_library_stub.clone() {
        queue_builder = queue_builder.add_provider(
            livrarr_domain::MetadataProvider::OpenLibrary,
            livrarr_external_data::ProviderClient::Stub(stub),
            livrarr_enrichment::ProviderQueueConfig {
                provider: livrarr_domain::MetadataProvider::OpenLibrary,
                max_attempts: 1,
            },
        );
    }
    let queue = Arc::new(queue_builder.build(db_arc.clone()));
    let enrichment_service = Arc::new(livrarr_metadata::EnrichmentServiceImpl::new(
        db_arc,
        queue.clone(),
        Arc::new(livrarr_metadata::DefaultMergeEngine::new(
            livrarr_metadata::PriorityModel::english(),
        )),
        false,
    ));
    let work_service: Arc<livrarr_server::state::LiveWorkService> =
        Arc::new(livrarr_server::state::build_live_work_service(
            db.clone(),
            enrichment_service.clone(),
            http_fetcher.clone(),
            data_dir.clone(),
            identity_resolver_arc.clone(),
        ));
    let discovery_service = Arc::new(
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
        HashMap::new(),
        hmac_key.clone(),
        data_dir_arc.clone(),
    ));
    let identity_road = Arc::new(
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
        readarr_import_progress: readarr_import_progress.clone(),
        manual_import_scans: manual_import_scans.clone(),
        provider_queue: queue,
        enrichment_service: enrichment_service.clone(),
        identity_road: identity_road.clone(),
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
                work_service.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    llm_http_client.clone(),
                ),
            )
            .with_identity_road(identity_road.clone()),
        ),
        work_service: work_service.clone(),
        discovery_service,
        grab_service: Arc::new(livrarr_download::grab_service::GrabServiceImpl::new(db.clone())),
        release_service: Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
            db.clone(),
            http_fetcher.clone(),
            trusted_origins.clone(),
        )),
        file_service: Arc::new(livrarr_library::file_service::FileServiceImpl::new(db.clone())),
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
        import_workflow: import_workflow.clone(),
        rss_sync_workflow: Arc::new(
            livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                Arc::new(db.clone()),
                Arc::new(http_fetcher.clone()),
                Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
                    db.clone(),
                    http_fetcher.clone(),
                    trusted_origins.clone(),
                )),
            ),
        ),
        list_service: Arc::new(
            livrarr_metadata::list_service::ListServiceImpl::with_identity_road(
                db.clone(),
                livrarr_server::state::build_live_work_service(
                    db.clone(),
                    enrichment_service.clone(),
                    http_fetcher.clone(),
                    data_dir.clone(),
                    identity_resolver_arc.clone(),
                ),
                http_fetcher.clone(),
                livrarr_metadata::list_service::NoOpBibliographyTrigger,
                identity_road.clone(),
            ),
        ),
        identity_resolver: identity_resolver_arc.clone(),
        enrichment_workflow: Arc::new(
            livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            ),
        ),
        author_monitor_workflow: Arc::new(
            livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
                Arc::new(db.clone()),
                Arc::new(livrarr_server::state::build_live_work_service(
                    db.clone(),
                    enrichment_service,
                    http_fetcher.clone(),
                    data_dir.clone(),
                    identity_resolver_arc,
                )),
                Arc::new(http_fetcher.clone()),
                identity_road.clone(),
            ),
        ),
        readarr_import_service: readarr_import_service.clone(),
        settings_service: settings_service.clone(),
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
        import_io_service: import_io.clone(),
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
        tag_service,
        email_svc: Arc::new(livrarr_server::email_service::LiveEmailService::new(
            settings_service,
        )),
        import_svc: import_service,
        matching_svc: livrarr_server::matching_service::LiveMatchingService,
        manual_import_scan_svc:
            livrarr_server::manual_import_scan_service::LiveManualImportScanService {
                scans: manual_import_scans,
            },
        readarr_import_wf: Arc::new(
            livrarr_server::readarr_import_workflow::LiveReadarrImportWorkflow::new(
                http_fetcher,
                readarr_import_service,
                readarr_import_progress,
                data_dir_arc,
                work_service,
                db.clone(),
                import_workflow,
            )
            .with_identity_road(identity_road),
        ),
        cover_service,
        preadd_cover_service: Arc::new(
            livrarr_metadata::preadd_cover_service::LivePreaddCoverService::new(HashMap::new()),
        ),
        hmac_key,
        trusted_origins_rebuilder: livrarr_server::state::TrustedOriginsRebuilderImpl(
            trusted_origins,
        ),
    };
    let app = livrarr_server::router::build_router(
        state.clone(),
        state.data_dir.join("ui-not-present-in-test"),
    );
    RouteHarness {
        app,
        state,
        api_key,
        db,
        user_id: user.id,
        open_library_stub,
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
    let mut request = request.body(request_body).expect("build U5 request");
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 55)),
        35_005,
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
        .expect("read U5 response body");
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"unparsedBody": String::from_utf8_lossy(&bytes).into_owned()}));
    RouteResponse { status, json }
}

async fn seed_user(db: &SqliteDb, label: &str) -> i64 {
    let n = CASE_ID.fetch_add(1, Ordering::Relaxed);
    db.create_user(CreateUserDbRequest {
        username: format!("irf-u5-{label}-{n}"),
        password_hash: "unused".to_string(),
        role: UserRole::Admin,
        api_key_hash: format!("unused-irf-u5-{label}-{n}"),
    })
    .await
    .expect("seed U5 user")
    .id
}

async fn seed_author(db: &SqliteDb, user_id: i64, name: &str) -> i64 {
    let (author, _) = AuthorLinkDb::create_or_adopt_author(
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
    .expect("seed U5 Author through create/adopt gate");
    author.id
}

async fn seed_work(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    title: &str,
    text_distinction: Option<&str>,
) -> ilr::CapturedIdentity {
    let mut identity_title =
        ilr::title_parts_from_provider(title.to_string(), None).expect("valid U5 fixture title");
    identity_title.provenance = ilr::EvidenceProvenance::User;
    WorkIdentityRepository::commit_settlement(
        db,
        ilr::SettlementCommit {
            user_id,
            existing_work_id: None,
            add_source: None,
            identity_title,
            text_distinction: text_distinction.map(str::to_string),
            contributors: vec![ilr::WorkContributor {
                user_id,
                work_id: 0,
                author_id,
                ordinal: 0,
                roles: Vec::new(),
            }],
            routes: Vec::new(),
            absorbed_work_ids: Vec::new(),
            expected_generation: 0,
            review_cards: Vec::new(),
        },
    )
    .await
    .expect("seed Work through production settlement")
    .identity
}

async fn captured(db: &SqliteDb, user_id: i64, work_id: i64) -> ilr::CapturedIdentity {
    WorkIdentityRepository::read_captured_identity(db, user_id, work_id)
        .await
        .expect("read captured U5 identity")
}

async fn seed_work_with_route(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    title: &str,
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: &str,
) -> ilr::CapturedIdentity {
    let mut commit = settlement_with_card(
        &seed_work(db, user_id, author_id, title, None).await,
        user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: None,
            invariant: "removed before commit; U5 route-seed fixture".to_string(),
        },
    );
    commit.review_cards.clear();
    commit.routes.push(ilr::WorkRoute {
        id: 0,
        user_id,
        owner: ilr::RouteOwner::Work(commit.existing_work_id.unwrap()),
        resolved_work_id: commit.existing_work_id.unwrap(),
        provider: provider.clone(),
        kind,
        provider_scoped_id: value.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(provider),
        user_confirmed: false,
        observed_at: Utc::now(),
    });
    WorkIdentityRepository::commit_settlement(db, commit)
        .await
        .expect("seed Work route through production settlement")
        .identity
}

fn pending_candidate(
    _work_id: i64,
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: impl Into<String>,
    owner: ilr::RouteOwner,
) -> ilr::ParkedRouteCandidate {
    ilr::ParkedRouteCandidate {
        route: ilr::RouteKey {
            provider,
            kind,
            value: value.into(),
        },
        proposed_owner: owner,
    }
}

fn pending_card(candidate: ilr::ParkedRouteCandidate, work_id: i64) -> ilr::SettlementReviewCard {
    ilr::SettlementReviewCard::PendingRoute { work_id, candidate }
}

fn settlement_with_card(
    identity: &ilr::CapturedIdentity,
    user_id: i64,
    card: ilr::SettlementReviewCard,
) -> ilr::SettlementCommit {
    ilr::SettlementCommit {
        user_id,
        existing_work_id: Some(identity.own_work_id),
        add_source: None,
        identity_title: identity.identity_title.clone(),
        text_distinction: (identity.text_distinction != "common")
            .then(|| identity.text_distinction.clone()),
        contributors: vec![ilr::WorkContributor {
            user_id,
            work_id: identity.own_work_id,
            author_id: identity.primary_author_id,
            ordinal: 0,
            roles: Vec::new(),
        }],
        routes: identity.active_routes.clone(),
        absorbed_work_ids: Vec::new(),
        expected_generation: identity.identity_generation,
        review_cards: vec![card],
    }
}

fn group_machine_request(
    user_id: i64,
    author_id: i64,
    title: &str,
    origin: ilr::IdentityRoadOrigin,
    provider: ilr::IdentityProvider,
    kind: ilr::RouteKind,
    value: &str,
) -> ilr::IdentityRoadRequest {
    let identity_title =
        ilr::title_parts_from_provider(title.to_string(), None).expect("valid group request title");
    ilr::IdentityRoadRequest {
        user_id,
        origin,
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: None,
            owned_files: Vec::new(),
            provider_identity: vec![ilr::ProviderIdentityEvidence {
                provider: provider.clone(),
                route: ilr::RouteKey {
                    provider,
                    kind,
                    value: value.to_string(),
                },
                work_core: Some(ilr::ProviderWorkIdentityCore {
                    identity_title,
                    primary_author_id: author_id,
                }),
                provenance: Default::default(),
            }],
            minimum: None,
        },
        interaction: ilr::IdentityRoadInteraction::MachineAlone,
        existing_work_id: None,
    }
}

async fn mint_group_from_machine_request(
    harness: &RouteHarness,
    request: ilr::IdentityRoadRequest,
) -> (i64, i64) {
    let outcome = harness
        .state
        .identity_road
        .settle(request)
        .await
        .expect("machine request reaches production road");
    match outcome {
        ilr::IdentityRoadOutcome::ReviewPending {
            review_id,
            expected_generation,
            kind: ilr::ReviewKind::GroupIdentity,
            ..
        } => (review_id, expected_generation),
        other => panic!("expected GroupIdentity ReviewPending, got {other:?}"),
    }
}

async fn mint_group_heal_card(
    db: &SqliteDb,
    identity: &ilr::CapturedIdentity,
    user_id: i64,
) -> i64 {
    let card = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![identity.own_work_id],
        proposed_identity: None,
        merge_choices: Vec::new(),
    };
    WorkIdentityRepository::commit_settlement(db, settlement_with_card(identity, user_id, card))
        .await
        .expect("mint GroupIdentity heal through generic production settlement")
        .review_cards[0]
        .id
}

async fn mint_pending_from_handoff(
    harness: &RouteHarness,
    work_id: i64,
    origin: ilr::IdentityRoadOrigin,
    route: ilr::RouteKey,
) -> i64 {
    let generation = captured(&harness.db, harness.user_id, work_id)
        .await
        .identity_generation;
    let outcome = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work_id,
            origin,
            ilr::CapturedRouteHandoff {
                metadata_generation: generation,
                provider_identity: Vec::new(),
                route_proposals: vec![route],
            },
        )
        .await
        .expect("production captured-route handoff")
        .expect("unsuppressed proposal returns a review");
    match outcome {
        ilr::IdentityRoadOutcome::ReviewPending {
            review_id,
            kind: ilr::ReviewKind::PendingRoute,
            ..
        } => review_id,
        other => panic!("expected PendingRoute review, got {other:?}"),
    }
}

fn open_library_route_detail(
    title: &str,
    author: &str,
    ol_key: &str,
    gr_work_key: &str,
) -> livrarr_external_data::NormalizedWorkDetail {
    livrarr_external_data::NormalizedWorkDetail {
        title: Some(title.to_string()),
        author_name: Some(author.to_string()),
        ol_key: Some(ol_key.to_string()),
        gr_work_key: Some(gr_work_key.to_string()),
        ..Default::default()
    }
}

async fn dismiss_pending_route_key(
    harness: &RouteHarness,
    work_id: i64,
    origin: ilr::IdentityRoadOrigin,
    route: ilr::RouteKey,
) -> i64 {
    let card_id = mint_pending_from_handoff(harness, work_id, origin, route).await;
    let response = dismiss_card(harness, card_id).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.json);
    card_id
}

async fn identity_audit_count(db: &SqliteDb, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count identity audits")
}

async fn pending_card_count(db: &SqliteDb, user_id: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND status='pending'",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("count pending cards")
}

async fn dismiss_card(harness: &RouteHarness, card_id: i64) -> RouteResponse {
    call_router_json(
        harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{card_id}/dismiss"),
        None,
    )
    .await
}

async fn card_status(db: &SqliteDb, card_id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id=?1")
        .bind(card_id)
        .fetch_one(db.pool())
        .await
        .expect("read review-card status")
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct LedgerRow {
    id: i64,
    user_id: i64,
    kind: String,
    key_version: i64,
    canonical_key: String,
    dismissed_at: String,
    source_card_id: i64,
    revoked_at: Option<String>,
    revoke_reason: Option<String>,
}

async fn ledger_rows(db: &SqliteDb, user_id: i64) -> Vec<LedgerRow> {
    sqlx::query_as(
        "SELECT id, user_id, kind, key_version, canonical_key, dismissed_at, \
                source_card_id, revoked_at, revoke_reason \
           FROM identity_review_dismissals WHERE user_id=?1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("read dismissal ledger")
}

async fn active_ledger_count(db: &SqliteDb, user_id: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_dismissals \
          WHERE user_id=?1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("count active dismissal ledger rows")
}

async fn ledger_work_ids(db: &SqliteDb, dismissal_id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT work_id FROM identity_review_dismissal_works \
          WHERE dismissal_id=?1 ORDER BY work_id",
    )
    .bind(dismissal_id)
    .fetch_all(db.pool())
    .await
    .expect("read dismissal Work-membership projection")
}

async fn assert_one_active_ledger(
    db: &SqliteDb,
    user_id: i64,
    kind: ilr::ReviewKind,
    source_card_id: i64,
) -> LedgerRow {
    let rows = ledger_rows(db, user_id).await;
    let active = rows
        .into_iter()
        .filter(|row| row.revoked_at.is_none())
        .collect::<Vec<_>>();
    assert_eq!(active.len(), 1, "one active semantic tombstone");
    assert_eq!(active[0].kind, kind.storage_code());
    assert_eq!(active[0].key_version, 1);
    assert_eq!(active[0].source_card_id, source_card_id);
    assert!(active[0].revoke_reason.is_none());
    active.into_iter().next().unwrap()
}

async fn review_dismissal_audit(
    db: &SqliteDb,
    user_id: i64,
    selected_card_id: i64,
) -> (String, String) {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor, payload FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind='review-dismissal' ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("read dismissal audit");
    let matching = rows
        .into_iter()
        .filter(|(_, payload)| payload.contains(&selected_card_id.to_string()))
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "exactly one dismissal audit names the selected card"
    );
    let row = matching.into_iter().next().unwrap();
    let actor: ReviewActor = serde_json::from_str(&row.0).expect("dismissal actor is ReviewActor");
    assert_eq!(actor, ReviewActor::AuthenticatedUser { user_id });
    row
}

async fn install_ledger_abort(db: &SqliteDb, label: &str) {
    sqlx::query(&format!(
        "CREATE TRIGGER u5_abort_{label} BEFORE INSERT ON identity_review_dismissals \
         BEGIN SELECT RAISE(ABORT, 'U5 dismissal atomicity probe'); END"
    ))
    .execute(db.pool())
    .await
    .expect("install dismissal-ledger abort probe");
}

async fn drop_ledger_abort(db: &SqliteDb, label: &str) {
    sqlx::query(&format!("DROP TRIGGER u5_abort_{label}"))
        .execute(db.pool())
        .await
        .expect("drop dismissal-ledger abort probe");
}

async fn install_revoke_abort(db: &SqliteDb, label: &str) {
    sqlx::query(&format!(
        "CREATE TRIGGER u5_revoke_abort_{label} \
         BEFORE UPDATE OF revoked_at ON identity_review_dismissals \
         WHEN OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL \
         BEGIN SELECT RAISE(ABORT, 'U5 revocation atomicity probe'); END"
    ))
    .execute(db.pool())
    .await
    .expect("install dismissal-revocation abort probe");
}

async fn drop_revoke_abort(db: &SqliteDb, label: &str) {
    sqlx::query(&format!("DROP TRIGGER u5_revoke_abort_{label}"))
        .execute(db.pool())
        .await
        .expect("drop dismissal-revocation abort probe");
}

async fn active_ledger_for_kind(db: &SqliteDb, user_id: i64, kind: ilr::ReviewKind) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_dismissals \
          WHERE user_id=?1 AND kind=?2 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(kind.storage_code())
    .fetch_one(db.pool())
    .await
    .expect("count active ledger rows by kind")
}

async fn assert_failed_dismiss_rolled_back(
    harness: &RouteHarness,
    card_id: i64,
    generation_before: Option<(i64, i64)>,
) {
    let response = dismiss_card(harness, card_id).await;
    assert_eq!(
        response.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        response.json
    );
    assert_eq!(card_status(&harness.db, card_id).await, "pending");
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    let audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind='review-dismissal'",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("count rolled-back dismissal audits");
    assert_eq!(audits, 0);
    if let Some((work_id, generation)) = generation_before {
        assert_eq!(
            captured(&harness.db, harness.user_id, work_id)
                .await
                .identity_generation,
            generation
        );
    }
}

fn clear_mint_trace() {
    livrarr_db::identity_layer::clear_review_card_mint_observations_for_tests();
}

fn take_mint_trace() -> Vec<livrarr_db::identity_layer::ReviewCardMintObservation> {
    livrarr_db::identity_layer::take_review_card_mint_observations_for_tests()
}

fn assert_suppressed_trace(
    site: livrarr_db::identity_layer::ReviewCardMintSite,
    kind: ilr::ReviewKind,
) {
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 1, "one proposal reaches the U7 authority");
    assert_eq!(trace[0].site, site);
    assert_eq!(trace[0].kind, kind);
    assert!(matches!(
        trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::SuppressedByDismissal
    ));
}

async fn notification_count(db: &SqliteDb, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE user_id=?1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count notifications")
}

async fn latest_bulk_enrichment_result(db: &SqliteDb, user_id: i64) -> (String, Value) {
    let (message, data): (String, String) = sqlx::query_as(
        "SELECT message,data FROM notifications \
          WHERE user_id=?1 AND type='bulkEnrichmentComplete' ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("read latest BulkEnrichmentComplete result");
    (
        message,
        serde_json::from_str(&data).expect("parse BulkEnrichmentComplete result data"),
    )
}

async fn pending_card_ids(db: &SqliteDb, user_id: i64, kind: ilr::ReviewKind) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT id FROM identity_review_cards \
          WHERE user_id=?1 AND kind=?2 AND status='pending' ORDER BY id",
    )
    .bind(user_id)
    .bind(kind.storage_code())
    .fetch_all(db.pool())
    .await
    .expect("list pending review cards")
}

async fn edition_card_ids(db: &SqliteDb, user_id: i64, edition_id: i64) -> Vec<i64> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, payload FROM identity_review_cards \
          WHERE user_id=?1 AND kind='EditionEvidence' AND status='pending' ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("read EditionEvidence cards");
    rows.into_iter()
        .filter_map(|(id, payload)| {
            let card: ilr::SettlementReviewCard = serde_json::from_str(&payload).ok()?;
            matches!(
                card,
                ilr::SettlementReviewCard::EditionEvidence {
                    edition_id: observed,
                    ..
                } if observed == edition_id
            )
            .then_some(id)
        })
        .collect()
}

async fn seed_edition(db: &SqliteDb, user_id: i64, work_id: i64) -> ilr::Edition {
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
    .expect("seed Edition through apply_work_evidence")
    .edition
}

async fn contradict_edition_direct(
    db: &SqliteDb,
    user_id: i64,
    edition_id: i64,
) -> Result<ilr::EditionEvidenceOutcome, ilr::EditionRepositoryError> {
    EditionRepository::apply_evidence(
        db,
        ilr::EditionEvidenceCommand {
            user_id,
            edition_id,
            format: Some(ilr::EditionFormat::Audiobook),
            language: Some("fr".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
}

async fn edition_row_bytes(db: &SqliteDb, edition_id: i64) -> String {
    sqlx::query_scalar(
        "SELECT json_object('id',id,'work_id',work_id,'format',format,'language',language, \
                            'state',state,'source_provider',source_provider, \
                            'provider_edition_id',provider_edition_id) \
           FROM editions WHERE id=?1",
    )
    .bind(edition_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot Edition")
}

fn stable_edition_aggregate(edition: &ilr::Edition) -> Value {
    let mut aggregate = serde_json::to_value(edition).expect("serialize Edition aggregate");
    if let Some(subtitle) = aggregate.get_mut("subtitle").and_then(Value::as_object_mut) {
        // Edition rows do not persist a subtitle observation timestamp, so a
        // normal hydration supplies `now`; compare every durable aggregate
        // field while deliberately excluding only that synthetic instant.
        subtitle.remove("observed_at");
    }
    aggregate
}

fn write_epub(path: &Path, title: &str, language: &str) {
    let file = std::fs::File::create(path).expect("create U5 EPUB");
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
        format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:language>{language}</dc:language></metadata><manifest/><spine/></package>"#).as_bytes(),
    )
    .expect("write EPUB OPF");
    zip.finish().expect("finish U5 EPUB");
}

async fn drive_manual_import(
    harness: &RouteHarness,
    path: &Path,
    title: &str,
    author: &str,
    language: &str,
) -> RouteResponse {
    write_epub(path, title, language);
    call_router_json(
        harness,
        Method::POST,
        "/api/v1/manualimport/import",
        Some(json!({"items": [{
            "path": path,
            "olKey": "",
            "title": title,
            "author": author,
            "deleteExisting": false,
            "language": language,
            "authorOlKey": null,
            "year": 2026,
            "coverUrl": null,
            "isbn": null,
            "description": null,
            "seriesName": null,
            "seriesPosition": null,
            "candidateId": null,
            "hcKey": null,
            "grKey": null,
            "asin": null
        }]})),
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MachineSnapshot {
    work_json: Option<String>,
    work_count: i64,
    pending_cards: i64,
    notifications: i64,
    audits: i64,
}

async fn machine_snapshot(db: &SqliteDb, user_id: i64, work_id: Option<i64>) -> MachineSnapshot {
    let work_json = match work_id {
        Some(work_id) => Some(
            serde_json::to_string(&captured(db, user_id, work_id).await)
                .expect("serialize complete captured identity"),
        ),
        None => None,
    };
    let work_count = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("snapshot Work count");
    let pending_cards = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND status='pending'",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot pending cards");
    let notifications = notification_count(db, user_id).await;
    let audits = sqlx::query_scalar("SELECT COUNT(*) FROM identity_audit_events WHERE user_id=?1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("snapshot identity audits");
    MachineSnapshot {
        work_json,
        work_count,
        pending_cards,
        notifications,
        audits,
    }
}

async fn legacy_user_dismiss_at(
    db: &SqliteDb,
    user_id: i64,
    card_id: i64,
    at: chrono::DateTime<Utc>,
) {
    // Constructed-state compatibility justification (verbatim): U5 replaces
    // the former Dismiss writer, so post-U5 code cannot invoke that historical
    // selected-row-only transaction. The card itself was minted through the
    // real production writer that existed then; these two statements reproduce
    // exactly its retained cancelled row plus ReviewActor audit, and deliberately
    // do not call the new ledger writer whose adoption is under test.
    let (work_id, kind): (Option<i64>, String) =
        sqlx::query_as("SELECT work_id,kind FROM identity_review_cards WHERE user_id=?1 AND id=?2")
            .bind(user_id)
            .bind(card_id)
            .fetch_one(db.pool())
            .await
            .expect("read historical Dismiss source");
    sqlx::query(
        "UPDATE identity_review_cards SET status='cancelled',resolved_at=?1 \
          WHERE user_id=?2 AND id=?3 AND status='pending'",
    )
    .bind(at.to_rfc3339())
    .bind(user_id)
    .bind(card_id)
    .execute(db.pool())
    .await
    .expect("reconstruct historical selected-row cancellation");
    sqlx::query(
        "INSERT INTO identity_audit_events \
            (user_id,work_id,event_kind,actor,payload,created_at) \
         VALUES (?1,?2,'review-dismissal',?3,?4,?5)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(
        serde_json::to_string(&ReviewActor::AuthenticatedUser { user_id })
            .expect("encode historical ReviewActor"),
    )
    .bind(format!("card_id={card_id};kind={kind}"))
    .bind(at.to_rfc3339())
    .execute(db.pool())
    .await
    .expect("reconstruct historical ReviewActor audit");
}

async fn legacy_machine_cancel_at(
    db: &SqliteDb,
    user_id: i64,
    card_id: i64,
    event_kind: &str,
    actor: &str,
    at: chrono::DateTime<Utc>,
) {
    let work_id: Option<i64> =
        sqlx::query_scalar("SELECT work_id FROM identity_review_cards WHERE id=?1")
            .bind(card_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    sqlx::query(
        "UPDATE identity_review_cards SET status='cancelled',resolved_at=?1 \
          WHERE user_id=?2 AND id=?3 AND status='pending'",
    )
    .bind(at.to_rfc3339())
    .bind(user_id)
    .bind(card_id)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO identity_audit_events \
            (user_id,work_id,event_kind,actor,payload,created_at) \
         VALUES (?1,?2,?3,?4,?5,?6)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(event_kind)
    .bind(actor)
    .bind(format!(
        "card_id={card_id};reason=historical-machine-cancel"
    ))
    .bind(at.to_rfc3339())
    .execute(db.pool())
    .await
    .unwrap();
}

async fn adoption_card_snapshot(db: &SqliteDb, user_id: i64) -> String {
    sqlx::query_scalar(
        "SELECT COALESCE(json_group_array(json_object( \
            'id',id,'work_id',work_id,'kind',kind,'generation',generation, \
            'status',status,'payload',payload,'created_at',created_at, \
            'resolved_at',resolved_at)), '[]') FROM ( \
            SELECT * FROM identity_review_cards WHERE user_id=?1 ORDER BY id)",
    )
    .bind(user_id)
    .fetch_one(db.pool())
    .await
    .expect("snapshot pre-activation review rows")
}

async fn adoption_marker(db: &SqliteDb) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key=?1")
        .bind(ADOPTION_MARKER)
        .fetch_optional(db.pool())
        .await
        .expect("read dismissal-adoption marker")
}

async fn assert_ledger_key_is_shared_u7_key(db: &SqliteDb, ledger: &LedgerRow, card_id: i64) {
    let (work_id, payload): (Option<i64>, String) =
        sqlx::query_as("SELECT work_id, payload FROM identity_review_cards WHERE id=?1")
            .bind(card_id)
            .fetch_one(db.pool())
            .await
            .expect("read source card for canonical-key assertion");
    let card: ilr::SettlementReviewCard =
        serde_json::from_str(&payload).expect("decode production card payload");
    let expected = ilr::ReviewDismissalKeyV1::from_card(ledger.user_id, &card, work_id)
        .expect("keyed U5 card")
        .canonical_json()
        .expect("canonical version-1 JSON");
    assert_eq!(
        ledger.canonical_key, expected,
        "Dismiss and U7 pending reuse must consume the same canonical key"
    );
}

async fn seed_title_heal_collision(db: &SqliteDb, user_id: i64) -> (i64, i64, i64) {
    let author_id = seed_author(db, user_id, "U5 Startup Heal Author").await;
    let first = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "U5 Cloud Cuckoo Land target".to_string(),
            author_name: "U5 Startup Heal Author".to_string(),
            normalized_title: "u5 cloud cuckoo land target".to_string(),
            normalized_author: "u5 startup heal author".to_string(),
            author_id: Some(author_id),
            ..Default::default()
        })
        .await
        .expect("seed title-heal target")
        .0;
    let second = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "U5 Cloud Cuckoo Land legacy".to_string(),
            author_name: "U5 Startup Heal Author".to_string(),
            normalized_title: "u5 cloud cuckoo land legacy".to_string(),
            normalized_author: "u5 startup heal author".to_string(),
            author_id: Some(author_id),
            ..Default::default()
        })
        .await
        .expect("seed title-heal source")
        .0;

    // Constructed-state compatibility justification: startup title-policy
    // heal exists specifically for pre-policy Work tuples. Current production
    // writers canonicalize them and cannot recreate this historical state, so
    // the fixture creates both Works through the former production writer and
    // alters only the legacy tuple columns the real heal consumes.
    sqlx::query(
        "UPDATE works SET title='U5 Cloud Cuckoo Land', subtitle='large print edition', \
                normalized_title='u5 cloud cuckoo land', \
                normalized_identity_main='u5 cloud cuckoo land', \
                normalized_identity_subtitle='large print edition', \
                normalized_identity_volume='', primary_author_id=?1, \
                text_distinction='common', identity_generation=1 \
          WHERE user_id=?2 AND id=?3",
    )
    .bind(author_id)
    .bind(user_id)
    .bind(first.id)
    .execute(db.pool())
    .await
    .expect("shape title-heal target");
    sqlx::query(
        "UPDATE works SET title='U5 Cloud Cuckoo Land (Large Print Edition)', subtitle=NULL, \
                normalized_title='u5 cloud cuckoo land (large print edition)', \
                normalized_identity_main='u5 cloud cuckoo land (large print edition)', \
                normalized_identity_subtitle='', normalized_identity_volume='', \
                primary_author_id=?1, text_distinction='common', identity_generation=1 \
          WHERE user_id=?2 AND id=?3",
    )
    .bind(author_id)
    .bind(user_id)
    .bind(second.id)
    .execute(db.pool())
    .await
    .expect("shape title-heal source");
    (author_id, first.id, second.id)
}

// RED-UNTIL-U5: today user Dismiss cancels only the selected GroupIdentity row and writes its ReviewActor audit; equivalent pending rows survive, no ledger exists, the helper errors if SuppressedByDismissal is surfaced, and the same machine proposal mints again.
#[tokio::test]
async fn group_identity_dismiss_bundle_is_atomic_and_machine_replay_is_suppressed() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Group Author").await;
    let anchor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Group Standing",
        Some("audited-distinction"),
    )
    .await;
    let request = group_machine_request(
        harness.user_id,
        author_id,
        "U5 Group Standing",
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-U5-GROUP-STANDING-W",
    );
    let (card_id, _) = mint_group_from_machine_request(&harness, request.clone()).await;
    // Constructed-state compatibility justification: U7 prevents equivalent
    // pending siblings, but U5 must close rows that predate U7. Copying the
    // production row reproduces only that historical duplicate shape.
    let equivalent_id = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id,work_id,kind,generation,status,payload,created_at) \
         SELECT user_id,work_id,kind,generation,'pending',payload,?1 \
           FROM identity_review_cards WHERE id=?2",
    )
    .bind((Utc::now() + chrono::Duration::seconds(1)).to_rfc3339())
    .bind(card_id)
    .execute(harness.db.pool())
    .await
    .expect("reconstruct pre-U7 Group sibling")
    .last_insert_rowid();
    let generation = captured(&harness.db, harness.user_id, anchor.own_work_id)
        .await
        .identity_generation;

    install_ledger_abort(&harness.db, "group").await;
    assert_failed_dismiss_rolled_back(&harness, card_id, Some((anchor.own_work_id, generation)))
        .await;
    assert_eq!(card_status(&harness.db, equivalent_id).await, "pending");
    drop_ledger_abort(&harness.db, "group").await;

    let dismissed = dismiss_card(&harness, card_id).await;
    assert_eq!(
        dismissed.status,
        StatusCode::NO_CONTENT,
        "{}",
        dismissed.json
    );
    assert_eq!(card_status(&harness.db, card_id).await, "cancelled");
    assert_eq!(card_status(&harness.db, equivalent_id).await, "cancelled");
    assert_eq!(
        captured(&harness.db, harness.user_id, anchor.own_work_id)
            .await
            .identity_generation,
        generation,
        "Dismiss is generation-neutral"
    );
    let ledger = assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::GroupIdentity,
        card_id,
    )
    .await;
    assert_ledger_key_is_shared_u7_key(&harness.db, &ledger, card_id).await;
    assert_eq!(
        ledger_work_ids(&harness.db, ledger.id).await,
        vec![anchor.own_work_id]
    );
    let (_, audit_payload) = review_dismissal_audit(&harness.db, harness.user_id, card_id).await;
    assert!(audit_payload.contains(&card_id.to_string()));
    assert!(audit_payload.contains(&equivalent_id.to_string()));

    let before = machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await;
    clear_mint_trace();
    let replay = harness
        .state
        .identity_road
        .settle(request)
        .await
        .expect("standing GroupIdentity dismissal defers normally");
    assert_eq!(
        replay,
        ilr::IdentityRoadOutcome::Deferred {
            reason: ilr::DeferReason(STANDING_DISMISSAL.to_string())
        }
    );
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await,
        before,
        "suppression precedes settlement mutation"
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
    );
}

// RED-UNTIL-U5: today user Dismiss cancels only the selected PendingRoute row and writes its ReviewActor audit; no ledger exists, the helper errors if SuppressedByDismissal is surfaced, and the captured-route replay mints a card again.
#[tokio::test]
async fn pending_route_dismiss_bundle_is_atomic_and_all_suppressed_handoff_is_none() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Pending Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Pending Standing",
        None,
    )
    .await;
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-PENDING-STANDING-W".to_string(),
    };
    let card_id = mint_pending_from_handoff(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        route.clone(),
    )
    .await;
    // Constructed-state compatibility justification: this is the exact
    // production PendingRoute row copied once to represent a pre-U7 sibling.
    let equivalent_id = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id,work_id,kind,generation,status,payload,created_at) \
         SELECT user_id,work_id,kind,generation,'pending',payload,?1 \
           FROM identity_review_cards WHERE id=?2",
    )
    .bind((Utc::now() + chrono::Duration::seconds(1)).to_rfc3339())
    .bind(card_id)
    .execute(harness.db.pool())
    .await
    .expect("reconstruct pre-U7 Pending sibling")
    .last_insert_rowid();
    let generation = captured(&harness.db, harness.user_id, work.own_work_id)
        .await
        .identity_generation;

    install_ledger_abort(&harness.db, "pending").await;
    assert_failed_dismiss_rolled_back(&harness, card_id, Some((work.own_work_id, generation)))
        .await;
    assert_eq!(card_status(&harness.db, equivalent_id).await, "pending");
    drop_ledger_abort(&harness.db, "pending").await;
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(card_status(&harness.db, equivalent_id).await, "cancelled");
    let ledger = assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::PendingRoute,
        card_id,
    )
    .await;
    assert_ledger_key_is_shared_u7_key(&harness.db, &ledger, card_id).await;
    assert_eq!(
        ledger_work_ids(&harness.db, ledger.id).await,
        vec![work.own_work_id]
    );
    let (_, audit_payload) = review_dismissal_audit(&harness.db, harness.user_id, card_id).await;
    assert!(audit_payload.contains(&card_id.to_string()));
    assert!(audit_payload.contains(&equivalent_id.to_string()));

    let before = machine_snapshot(&harness.db, harness.user_id, Some(work.own_work_id)).await;
    clear_mint_trace();
    let replay = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            ilr::CapturedRouteHandoff {
                metadata_generation: generation,
                provider_identity: Vec::new(),
                route_proposals: vec![route],
            },
        )
        .await
        .expect("standing PendingRoute dismissal is a successful handoff");
    assert_eq!(replay, None, "all-suppressed captured handoff is Ok(None)");
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(work.own_work_id)).await,
        before
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
    );
}

// RED-UNTIL-U5: today user Dismiss cancels only the selected EditionEvidence row and writes its ReviewActor audit; no ledger exists, the helper errors if SuppressedByDismissal is surfaced, and apply_evidence parks the same contradiction again instead of returning normal success.
#[tokio::test]
async fn edition_evidence_dismiss_bundle_is_atomic_and_direct_writer_returns_unchanged_success() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Edition Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Edition Standing",
        None,
    )
    .await;
    let edition = seed_edition(&harness.db, harness.user_id, work.own_work_id).await;

    // Constructed-state compatibility justification: `apply_evidence` has no
    // live caller in the v11 tree; AC-005 expressly requires this repository
    // method as the compatibility fixture, and it remains a runtime-capable
    // production mint writer.
    assert!(matches!(
        contradict_edition_direct(&harness.db, harness.user_id, edition.id).await,
        Err(ilr::EditionRepositoryError::ContradictoryEvidenceParked)
    ));
    let card_id = edition_card_ids(&harness.db, harness.user_id, edition.id).await[0];
    let before_edition = edition_row_bytes(&harness.db, edition.id).await;

    install_ledger_abort(&harness.db, "edition").await;
    assert_failed_dismiss_rolled_back(
        &harness,
        card_id,
        Some((work.own_work_id, work.identity_generation)),
    )
    .await;
    drop_ledger_abort(&harness.db, "edition").await;
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );
    let ledger = assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::EditionEvidence,
        card_id,
    )
    .await;
    assert_ledger_key_is_shared_u7_key(&harness.db, &ledger, card_id).await;
    review_dismissal_audit(&harness.db, harness.user_id, card_id).await;

    let machine_before =
        machine_snapshot(&harness.db, harness.user_id, Some(work.own_work_id)).await;
    let cards_before = pending_card_ids(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::EditionEvidence,
    )
    .await;
    clear_mint_trace();
    let replay = contradict_edition_direct(&harness.db, harness.user_id, edition.id)
        .await
        .expect("dismissed contradiction is a normal successful no-op");
    assert_eq!(replay.edition.id, edition.id);
    assert_eq!(
        edition_row_bytes(&harness.db, edition.id).await,
        before_edition
    );
    assert_eq!(
        pending_card_ids(
            &harness.db,
            harness.user_id,
            ilr::ReviewKind::EditionEvidence
        )
        .await,
        cards_before
    );
    assert_eq!(notification_count(&harness.db, harness.user_id).await, 0);
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(work.own_work_id)).await,
        machine_before,
        "suppressed apply_evidence writes no card, notification, audit, or Work mutation"
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::ApplyEvidence,
        ilr::ReviewKind::EditionEvidence,
    );
}

// RED-UNTIL-U5-R2 (OAI-U5-010): suppression currently returns sparse
// synthetic Editions. Both repository writers must return the same complete
// existing aggregate that a normal hydration returns.
#[tokio::test]
async fn suppressed_edition_writers_return_full_existing_aggregate() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Rich Edition Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Rich Edition",
        None,
    )
    .await;
    let route_value = "9555299";
    let mut commit = settlement_with_card(
        &work,
        harness.user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: Some(work.own_work_id),
            invariant: "removed before commit; rich Edition fixture".to_string(),
        },
    );
    commit.review_cards.clear();
    commit.routes.push(ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: ilr::RouteOwner::Work(work.own_work_id),
        resolved_work_id: work.own_work_id,
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsBookEdition,
        provider_scoped_id: route_value.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads),
        user_confirmed: false,
        observed_at: Utc::now(),
    });
    WorkIdentityRepository::commit_settlement(&harness.db, commit)
        .await
        .expect("materialize rich Edition route through production settlement");
    let edition_id: i64 = sqlx::query_scalar(
        "SELECT edition_id FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id=?3",
    )
    .bind(harness.user_id)
    .bind(work.own_work_id)
    .bind(route_value)
    .fetch_one(harness.db.pool())
    .await
    .expect("production settlement materialized Edition owner");

    // Constructed-state compatibility justification: no public Edition
    // metadata writer exists in the v11 tree. Shape the persisted fields that
    // both real repository writers are required to hydrate; the route and
    // Edition themselves were created by the production settlement writer.
    sqlx::query(
        "UPDATE editions SET subtitle=?1, subtitle_provenance=?2 \
          WHERE user_id=?3 AND id=?4",
    )
    .bind("The Durable Subtitle")
    .bind(
        serde_json::to_string(&ilr::EvidenceProvenance::Provider(
            ilr::IdentityProvider::Goodreads,
        ))
        .unwrap(),
    )
    .bind(harness.user_id)
    .bind(edition_id)
    .execute(harness.db.pool())
    .await
    .expect("shape rich Edition metadata");

    let expected = EditionRepository::apply_evidence(
        &harness.db,
        ilr::EditionEvidenceCommand {
            user_id: harness.user_id,
            edition_id,
            format: Some(ilr::EditionFormat::Ebook),
            language: Some("en".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .expect("normal writer hydrates complete existing Edition")
    .edition;
    assert_eq!(expected.routes.len(), 1, "fixture carries its real route");
    assert!(expected.subtitle.is_some(), "fixture carries its subtitle");
    assert_eq!(
        expected.source_provider,
        Some(ilr::IdentityProvider::Goodreads)
    );
    assert_eq!(expected.provider_edition_id.as_deref(), Some(route_value));
    assert_eq!(expected.state, ilr::EditionState::Active);

    assert!(matches!(
        contradict_edition_direct(&harness.db, harness.user_id, edition_id).await,
        Err(ilr::EditionRepositoryError::ContradictoryEvidenceParked)
    ));
    let card_id = edition_card_ids(&harness.db, harness.user_id, edition_id).await[0];
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );

    clear_mint_trace();
    let direct = contradict_edition_direct(&harness.db, harness.user_id, edition_id)
        .await
        .expect("suppressed apply_evidence returns existing aggregate")
        .edition;
    let work_facing = EditionRepository::apply_work_evidence(
        &harness.db,
        ilr::EditionWorkEvidenceCommand {
            user_id: harness.user_id,
            work_id: work.own_work_id,
            format: ilr::EditionFormat::Audiobook,
            language: Some("fr".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .expect("suppressed apply_work_evidence returns existing aggregate")
    .edition;
    let expected = stable_edition_aggregate(&expected);
    assert_eq!(
        vec![
            stable_edition_aggregate(&direct),
            stable_edition_aggregate(&work_facing),
        ],
        vec![expected.clone(), expected],
        "both suppressed writers return every durable field of the existing Edition"
    );
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 2);
    assert!(trace.iter().all(|entry| matches!(
        entry.outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::SuppressedByDismissal
    )));
}

// RED-UNTIL-U5: today U7 duplicate cleanup correctly keeps the oldest EditionEvidence row and machine-cancels the later row without a ReviewActor audit, but a later user Dismiss still cancels only its selected row and creates no tombstone.
#[tokio::test]
async fn edition_duplicates_clean_up_without_tombstone_then_user_dismiss_closes_every_equivalent() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Duplicate Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Duplicate Edition",
        None,
    )
    .await;
    let edition = seed_edition(&harness.db, harness.user_id, work.own_work_id).await;

    // Constructed-state compatibility justification: equivalent pending
    // duplicates are historical pre-U7 state that no correct post-U7 door can
    // create. The oldest row is minted through the real `apply_evidence`
    // writer; INSERT..SELECT copies that exact production payload and row shape
    // to reconstruct the second call made before U7.
    assert!(
        contradict_edition_direct(&harness.db, harness.user_id, edition.id)
            .await
            .is_err()
    );
    let oldest = edition_card_ids(&harness.db, harness.user_id, edition.id).await[0];
    let pre_u7_later = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id, work_id, kind, generation, status, payload, created_at) \
         SELECT user_id, work_id, kind, generation, status, payload, ?1 \
           FROM identity_review_cards WHERE id=?2",
    )
    .bind((Utc::now() + chrono::Duration::seconds(1)).to_rfc3339())
    .bind(oldest)
    .execute(harness.db.pool())
    .await
    .expect("reconstruct pre-U7 duplicate")
    .last_insert_rowid();

    assert!(
        contradict_edition_direct(&harness.db, harness.user_id, edition.id)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_as::<_, (i64, String)>(
            "SELECT id,status FROM identity_review_cards WHERE id IN (?1,?2) ORDER BY id"
        )
        .bind(oldest)
        .bind(pre_u7_later)
        .fetch_all(harness.db.pool())
        .await
        .unwrap(),
        vec![
            (oldest, "pending".to_string()),
            (pre_u7_later, "cancelled".to_string())
        ]
    );
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    clear_mint_trace();
    assert!(matches!(
        contradict_edition_direct(&harness.db, harness.user_id, edition.id).await,
        Err(ilr::EditionRepositoryError::ContradictoryEvidenceParked)
    ));
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    let replay_trace = take_mint_trace();
    assert_eq!(replay_trace.len(), 1);
    assert!(matches!(
        replay_trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::ReusedPending(card)
            if card.id == oldest
    ));
    let cleanup_actor: String = sqlx::query_scalar(
        "SELECT actor FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind='review-card-duplicate-cleanup' ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .expect("U7 duplicate cleanup audit");
    assert!(serde_json::from_str::<ReviewActor>(&cleanup_actor).is_err());

    // Reconstruct one more genuine pre-U7 equivalent pending row. This arm is
    // deliberately separate from helper cleanup: a user Dismiss must close
    // every equivalent that is still pending, regardless of row id.
    let dismiss_later = sqlx::query(
        "INSERT INTO identity_review_cards \
            (user_id, work_id, kind, generation, status, payload, created_at) \
         SELECT user_id, work_id, kind, generation, 'pending', payload, ?1 \
           FROM identity_review_cards WHERE id=?2",
    )
    .bind((Utc::now() + chrono::Duration::seconds(2)).to_rfc3339())
    .bind(oldest)
    .execute(harness.db.pool())
    .await
    .expect("reconstruct duplicate for user Dismiss")
    .last_insert_rowid();
    assert_eq!(
        dismiss_card(&harness, oldest).await.status,
        StatusCode::NO_CONTENT
    );
    let statuses: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id,status FROM identity_review_cards WHERE id IN (?1,?2) ORDER BY id",
    )
    .bind(oldest)
    .bind(dismiss_later)
    .fetch_all(harness.db.pool())
    .await
    .expect("read duplicate Dismiss statuses");
    assert_eq!(
        statuses,
        vec![
            (oldest, "cancelled".to_string()),
            (dismiss_later, "cancelled".to_string())
        ]
    );
    let (_, payload) = review_dismissal_audit(&harness.db, harness.user_id, oldest).await;
    assert!(payload.contains(&oldest.to_string()));
    assert!(payload.contains(&dismiss_later.to_string()));
    assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::EditionEvidence,
        oldest,
    )
    .await;
}

// PIN: an unkeyed kind remains selected-row-only; its equal-looking sibling stays pending and no dismissal-ledger row is written.
#[tokio::test]
async fn unkeyed_dismiss_remains_selected_row_only() {
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Unkeyed Author").await;
    let mut work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Unkeyed Card",
        None,
    )
    .await;
    let card = ilr::SettlementReviewCard::InvariantRepair {
        work_id: Some(work.own_work_id),
        invariant: "U5 unkeyed compatibility fixture".to_string(),
    };

    // Constructed-state compatibility justification: InvariantRepair has no
    // production producer (ST-007). Generic settlement is nevertheless the
    // runtime-capable production card writer and is the required backstop for
    // pinning an unkeyed Dismiss without inventing a new door.
    let first = WorkIdentityRepository::commit_settlement(
        &harness.db,
        settlement_with_card(&work, harness.user_id, card.clone()),
    )
    .await
    .expect("mint first unkeyed compatibility card");
    work = first.identity;
    let second = WorkIdentityRepository::commit_settlement(
        &harness.db,
        settlement_with_card(&work, harness.user_id, card),
    )
    .await
    .expect("mint equal-looking unkeyed sibling");
    let first_id = first.review_cards[0].id;
    let second_id = second.review_cards[0].id;
    assert_ne!(first_id, second_id);

    assert_eq!(
        dismiss_card(&harness, first_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(card_status(&harness.db, first_id).await, "cancelled");
    assert_eq!(card_status(&harness.db, second_id).await, "pending");
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    let (_, payload) = review_dismissal_audit(&harness.db, harness.user_id, first_id).await;
    assert!(payload.contains(&first_id.to_string()));
    assert!(!payload.contains(&second_id.to_string()));
}

// RED-UNTIL-U5: today ListImport synthesizes ExplicitCreate merely because an ordinary confirmed row is anchorless, so after Dismiss the equivalent row bypasses nothing durable and mints/reuses another card instead of returning the standing-dismissal Deferred message.
#[tokio::test]
async fn list_import_without_genuine_choice_returns_exact_add_failed_deferred() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "Suppressed List Author").await;
    let anchor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "Suppressed List Book",
        Some("audited-distinction"),
    )
    .await;
    let csv = b"Book Id,Title,Author,ISBN,ISBN13,My Rating,Exclusive Shelf\n=\"\",Suppressed List Book,Suppressed List Author,=\"\",=\"\",5,read\n";

    let preview = harness
        .state
        .list_service
        .preview(harness.user_id, csv.to_vec())
        .await
        .expect("production ListImport preview");
    let first = harness
        .state
        .list_service
        .confirm(harness.user_id, &preview.preview_id, None, &[0], None)
        .await
        .expect("production ListImport confirm envelope");
    assert_eq!(first.results[0].status, "add_failed");
    let card_id = pending_card_ids(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity)
        .await
        .into_iter()
        .next()
        .expect("first ListImport attempt mints GroupIdentity");
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );

    let before = machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await;
    clear_mint_trace();
    let replay_preview = harness
        .state
        .list_service
        .preview(harness.user_id, csv.to_vec())
        .await
        .expect("fresh ListImport preview");
    let replay = harness
        .state
        .list_service
        .confirm(
            harness.user_id,
            &replay_preview.preview_id,
            Some(&first.import_id),
            &[0],
            None,
        )
        .await
        .expect("row-level ListImport suppression keeps batch successful");
    assert_eq!(replay.results.len(), 1);
    assert_eq!(replay.results[0].status, "add_failed");
    assert_eq!(
        replay.results[0].message.as_deref(),
        Some("identity review required: Deferred { reason: DeferReason(\"standing dismissal\") }")
    );
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await,
        before
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
    );
}

// RED-UNTIL-U5: today no ledger can make the choice/bypass distinction; this compatibility call only reuses or mints because SuppressedByDismissal is unreachable, not because validated explicit choice overrides a tombstone.
#[tokio::test]
async fn validated_list_candidate_choice_bypasses_and_mints_one_pending_notification() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 List Choice Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 List Choice",
        None,
    )
    .await;
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "551100".to_string(),
    };
    let dismissed_id = mint_pending_from_handoff(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        route.clone(),
    )
    .await;
    assert_eq!(
        dismiss_card(&harness, dismissed_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::PendingRoute,
        dismissed_id,
    )
    .await;
    let ledger_before = ledger_rows(&harness.db, harness.user_id).await;
    let notifications_before = notification_count(&harness.db, harness.user_id).await;

    // Constructed-state compatibility justification: the current ListImport
    // confirm wire carries row selection, not a per-book candidate pick.
    // AC-005 therefore requires this validated road/service compatibility
    // fixture. It passes the production writer's already-validated choice bit,
    // adds no production route, never reaches GroupIdentity, and changes no
    // HTTP shape.
    clear_mint_trace();
    let candidate = pending_candidate(
        work.own_work_id,
        route.provider,
        route.kind,
        format!("  {}  ", route.value),
        ilr::RouteOwner::Edition(9_555_001),
    );
    let minted = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        harness.user_id,
        work.own_work_id,
        work.identity_generation,
        candidate,
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ListImport),
        true,
    )
    .await
    .expect("validated ListImport choice bypasses tombstone");
    assert_ne!(minted.id, dismissed_id);
    assert_eq!(card_status(&harness.db, minted.id).await, "pending");
    assert_eq!(
        notification_count(&harness.db, harness.user_id).await,
        notifications_before + 1
    );
    let routes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id=?3",
    )
    .bind(harness.user_id)
    .bind(work.own_work_id)
    .bind(&route.value)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(routes, 0, "compatibility fixture adds no route");
    assert_eq!(
        ledger_rows(&harness.db, harness.user_id).await,
        ledger_before,
        "validated ListImport choice bypasses but does not revoke"
    );
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 1);
    assert!(matches!(
        trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(card) if card.id == minted.id
    ));
}

// RED-UNTIL-U5: today after Dismiss the equivalent AuthorMonitor replay mints a GroupIdentity card again; there is no standing ledger, and the helper never returns SuppressedByDismissal.
#[tokio::test]
async fn author_monitor_reports_no_work_and_no_notification_after_suppression() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "Suppressed Monitor Author").await;
    harness
        .db
        .attach_route_as_user(
            harness.user_id,
            author_id,
            AuthorRouteKey::parse(AuthorProvider::OpenLibrary, "OL9555210A")
                .expect("canonical author route"),
        )
        .await
        .expect("attach monitored Author route");
    harness
        .db
        .update_author(
            harness.user_id,
            author_id,
            UpdateAuthorDbRequest {
                name: None,
                sort_name: None,
                ol_key: None,
                gr_key: None,
                monitored: Some(true),
                monitor_new_items: Some(true),
                monitor_since: None,
                monitor_language: None,
            },
        )
        .await
        .expect("enable AuthorMonitor");
    let anchor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "Suppressed Monitor Book",
        Some("audited-distinction"),
    )
    .await;
    let request = group_machine_request(
        harness.user_id,
        author_id,
        "Suppressed Monitor Book",
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL9555211W",
    );
    let (card_id, _) = mint_group_from_machine_request(&harness, request).await;
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );
    let before = machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await;

    let fetcher = StubHttpFetcher::with_ok(
        200,
        br#"{"entries":[{"key":"/works/OL9555211W","title":"Suppressed Monitor Book","first_publish_date":"2026"}]}"#.to_vec(),
    );
    let workflow =
        livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
            Arc::new(harness.db.clone()),
            harness.state.work_service.clone(),
            Arc::new(fetcher),
            harness.state.identity_road.clone(),
        );
    clear_mint_trace();
    let report = workflow
        .run_monitor(harness.user_id, CancellationToken::new())
        .await
        .expect("production AuthorMonitor workflow");
    assert_eq!(report.new_works_found, 1);
    assert_eq!(report.works_added, 0);
    assert_eq!(report.notifications_created, 0);
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await,
        before
    );
    let auto_added: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id=?1 AND type='workAutoAdded'",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(auto_added, 0);
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
    );
}

// RED-UNTIL-U5: today after Dismiss the equivalent SeriesMonitor replay mints another GroupIdentity card; no ledger suppresses before settlement, so the non-settled branch is not durable.
#[tokio::test]
#[traced_test]
async fn series_monitor_logs_non_settled_and_leaves_counters_and_membership_unchanged() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let (author, _) = harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: "Suppressed Series Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: Some("A-U5-SERIES".to_string()),
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed SeriesMonitor Author");
    let series = harness
        .db
        .upsert_series(CreateSeriesDbRequest {
            user_id: harness.user_id,
            author_id: author.id,
            name: "U5 Series".to_string(),
            gr_key: "S-U5-SERIES".to_string(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("seed monitored series");
    let anchor = seed_work(
        &harness.db,
        harness.user_id,
        author.id,
        "Suppressed Series Book",
        Some("audited-distinction"),
    )
    .await;
    let request = group_machine_request(
        harness.user_id,
        author.id,
        "Suppressed Series Book",
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::SeriesMonitor),
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsBookEdition,
        "47212",
    );
    let (card_id, _) = mint_group_from_machine_request(&harness, request).await;
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );
    let works_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
    let membership_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1 AND series_id=?2")
            .bind(harness.user_id)
            .bind(series.id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let series_before: (i32, i64) = sqlx::query_as(
        "SELECT work_count, (SELECT COUNT(*) FROM works WHERE user_id=?1 AND series_id=?2) \
           FROM series WHERE id=?2",
    )
    .bind(harness.user_id)
    .bind(series.id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let machine_before =
        machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await;

    let service = livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
        harness.db.clone(),
        StubHttpFetcher::with_ok(200, U5_SERIES_ROSTER.as_bytes().to_vec()),
        harness.state.work_service.clone(),
        livrarr_metadata::discovery_service::StubNoLlm,
    )
    .with_identity_road(harness.state.identity_road.clone());
    clear_mint_trace();
    service
        .run_series_monitor_worker(livrarr_domain::services::SeriesMonitorWorkerParams {
            cancel: CancellationToken::new(),
            user_id: harness.user_id,
            author_id: author.id,
            series_id: series.id,
            series_name: series.name,
            series_gr_key: series.gr_key,
            monitor_ebook: true,
            monitor_audiobook: false,
        })
        .await
        .expect("production SeriesMonitor worker");
    let works_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
    let membership_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1 AND series_id=?2")
            .bind(harness.user_id)
            .bind(series.id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let series_after: (i32, i64) = sqlx::query_as(
        "SELECT work_count, (SELECT COUNT(*) FROM works WHERE user_id=?1 AND series_id=?2) \
           FROM series WHERE id=?2",
    )
    .bind(harness.user_id)
    .bind(series.id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        (works_after, membership_after),
        (works_before, membership_before)
    );
    assert_eq!(series_after, series_before);
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await,
        machine_before,
        "SeriesMonitor suppression writes no card, notification, audit, or Work mutation"
    );
    assert!(logs_contain("series identity road did not settle work"));
    assert!(logs_contain("series monitor worker complete"));
    assert_eq!(
        captured(&harness.db, harness.user_id, anchor.own_work_id)
            .await
            .identity_generation,
        anchor.identity_generation + 1,
        "only the original card mint claimed generation; suppression did not"
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
    );
}

// RED-UNTIL-U5-R2 (OAI-U5-007): the existing-match branch currently sends an
// unbound road request. Provider-key matching can select one Work while that
// request creates or settles another before the original id is linked.
#[tokio::test]
async fn series_existing_match_binds_and_links_exactly_the_matched_work() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Bound Series Author").await;
    let matched = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Provider-Key Matched Legacy Title",
        None,
    )
    .await;
    let matched_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "47213".to_string(),
    };
    let affirm_card = mint_pending_from_handoff(
        &harness,
        matched.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        matched_route.clone(),
    )
    .await;
    let affirm_generation: i64 =
        sqlx::query_scalar("SELECT generation FROM identity_review_cards WHERE id=?1")
            .bind(affirm_card)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let affirm = harness
        .state
        .identity_road
        .resolve_review(
            ReviewActor::AuthenticatedUser {
                user_id: harness.user_id,
            },
            ReviewResolutionCommand::PendingRoute {
                card_id: affirm_card,
                expected_generation: affirm_generation,
                action: ilr::PendingRouteAction::Affirm {
                    surviving_routes: vec![matched_route],
                },
            },
        )
        .await
        .expect("seed provider-key match through production PendingRoute Affirm");
    assert!(matches!(
        affirm,
        ilr::IdentityRoadOutcome::Settled { work_id, .. } if work_id == matched.own_work_id
    ));
    assert_eq!(
        WorkDb::get_work(&harness.db, harness.user_id, matched.own_work_id)
            .await
            .unwrap()
            .gr_key
            .as_deref(),
        Some("47213"),
        "the real matching seat sees the affirmed provider key"
    );
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);

    let series = harness
        .db
        .upsert_series(CreateSeriesDbRequest {
            user_id: harness.user_id,
            author_id,
            name: "U5 Bound Series".to_string(),
            gr_key: "S-U5-BOUND".to_string(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("seed bound monitored series");
    let works_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
    let service = livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
        harness.db.clone(),
        StubHttpFetcher::with_ok(200, U5_SERIES_EXISTING_MATCH_ROSTER.as_bytes().to_vec()),
        harness.state.work_service.clone(),
        livrarr_metadata::discovery_service::StubNoLlm,
    )
    .with_identity_road(harness.state.identity_road.clone());
    service
        .run_series_monitor_worker(livrarr_domain::services::SeriesMonitorWorkerParams {
            cancel: CancellationToken::new(),
            user_id: harness.user_id,
            author_id,
            series_id: series.id,
            series_name: series.name,
            series_gr_key: series.gr_key,
            monitor_ebook: true,
            monitor_audiobook: false,
        })
        .await
        .expect("run existing-match SeriesMonitor path");

    let works_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1")
        .bind(harness.user_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
    assert_eq!(
        works_after, works_before,
        "an existing provider-key match must not let an unbound road result create another Work"
    );
    let linked_series: Option<i64> =
        sqlx::query_scalar("SELECT series_id FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(matched.own_work_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(linked_series, Some(series.id));
    let linked_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id=?1 AND series_id=?2")
            .bind(harness.user_id)
            .bind(series.id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(
        linked_count, 1,
        "only the provider-key matched Work is linked"
    );
}

// RED-UNTIL-U5: today after Dismiss the live Readarr item path creates the equivalent review again and reports a review id; no ledger yields the exact standing-dismissal per-book error or the normal no-map/no-import exit.
#[tokio::test]
async fn live_readarr_item_returns_exact_error_and_maps_no_work_or_file() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "Suppressed Readarr Author").await;
    let anchor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "Suppressed Book",
        Some("audited-distinction"),
    )
    .await;
    let rd_author: livrarr_server::readarr_client::RdAuthor = serde_json::from_value(json!({
        "id": 5101,
        "authorName": "Suppressed Readarr Author"
    }))
    .expect("decode Readarr Author fixture");
    let rd_book: livrarr_server::readarr_client::RdBook = serde_json::from_value(json!({
        "id": 5102,
        "title": "Suppressed Book",
        "authorId": 5101,
        "foreignBookId": "55102",
        "editions": null
    }))
    .expect("decode Readarr Book fixture");

    let first = harness
        .state
        .readarr_import_wf
        .process_single_work_item_for_tests(
            "u5-readarr-first",
            harness.user_id,
            rd_book.clone(),
            rd_author.clone(),
            author_id,
        )
        .await
        .expect_err("first live Readarr item parks review");
    assert_eq!(first, "Readarr process_works item did not produce a Work");
    let card_id = pending_card_ids(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity)
        .await
        .into_iter()
        .next()
        .expect("Readarr GroupIdentity card");
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );
    let before = machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await;
    let library_items_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM library_items WHERE user_id=?1")
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();

    clear_mint_trace();
    let replay = harness
        .state
        .readarr_import_wf
        .process_single_work_item_for_tests(
            "u5-readarr-replay",
            harness.user_id,
            rd_book,
            rd_author,
            author_id,
        )
        .await
        .expect_err("suppressed Readarr item has no Work mapping");
    assert_eq!(
        replay,
        "Work 'Suppressed Book': identity did not settle (Deferred { reason: DeferReason(\"standing dismissal\") })"
    );
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(anchor.own_work_id)).await,
        before
    );
    let library_items_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM library_items WHERE user_id=?1")
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(library_items_after, library_items_before);
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
    );
}

// RED-UNTIL-U5-R2 (OAI-U5-009): the per-item helper currently searches the
// shared progress record from the beginning and can attribute an earlier
// book's standing-dismissal error to the current book.
#[tokio::test]
async fn readarr_item_error_ignores_an_earlier_books_progress_history() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let earlier = "Work 'Earlier Book': identity did not settle (Deferred { reason: DeferReason(\"standing dismissal\") })".to_string();
    harness
        .state
        .readarr_import_progress
        .lock()
        .await
        .errors
        .push(earlier.clone());

    let author_id = seed_author(&harness.db, harness.user_id, "U5 Current Readarr Author").await;
    seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Current Readarr Book",
        Some("audited-distinction"),
    )
    .await;
    let rd_author: livrarr_server::readarr_client::RdAuthor = serde_json::from_value(json!({
        "id": 5191,
        "authorName": "U5 Current Readarr Author"
    }))
    .expect("decode current Readarr Author fixture");
    let rd_book: livrarr_server::readarr_client::RdBook = serde_json::from_value(json!({
        "id": 5192,
        "title": "U5 Current Readarr Book",
        "authorId": 5191,
        "foreignBookId": "55192",
        "editions": null
    }))
    .expect("decode current Readarr Book fixture");

    let current = harness
        .state
        .readarr_import_wf
        .process_single_work_item_for_tests(
            "u5-readarr-current",
            harness.user_id,
            rd_book,
            rd_author,
            author_id,
        )
        .await
        .expect_err("current item parks review and has no Work mapping");
    assert_eq!(
        current, "Readarr process_works item did not produce a Work",
        "only errors appended while processing the current book may be attributed to it"
    );
    assert_ne!(current, earlier);
    assert_eq!(
        pending_card_ids(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity)
            .await
            .len(),
        1,
        "the current item reached its real ReviewPending result"
    );
}

// RED-UNTIL-U5: today generic EnrichmentPass settlement reuses or mints the equivalent GroupIdentity card and its captured handoff reuses or mints PendingRoute; the helper cannot report SuppressedByDismissal and the add-background adapter has no standing-dismissal non-settled/false or successful-none branch.
#[tokio::test]
async fn enrichment_pass_add_background_handoff_takes_the_non_settled_false_branch() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Enrichment Handoff";
    let author_name = "U5 Enrichment Author";
    let ol_key = "OL-U5-ENRICHMENT-W";
    let gr_key = "9555101";
    let harness = build_route_harness_with_open_library(open_library_route_detail(
        title,
        author_name,
        ol_key,
        gr_key,
    ))
    .await;
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    let group_anchor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Enrichment Group",
        None,
    )
    .await;
    seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Enrichment Group",
        Some("second-audited-distinction"),
    )
    .await;
    let mut group_request = group_machine_request(
        harness.user_id,
        author_id,
        "U5 Enrichment Group",
        ilr::IdentityRoadOrigin::EnrichmentPass,
        ilr::IdentityProvider::Hardcover,
        ilr::RouteKind::HardcoverWork,
        "HC-U5-ENRICHMENT-GROUP-W",
    );
    group_request.existing_work_id = Some(group_anchor.own_work_id);
    let (group_id, _) = mint_group_from_machine_request(&harness, group_request.clone()).await;
    assert_eq!(
        dismiss_card(&harness, group_id).await.status,
        StatusCode::NO_CONTENT
    );
    let group_before =
        machine_snapshot(&harness.db, harness.user_id, Some(group_anchor.own_work_id)).await;
    clear_mint_trace();
    assert_eq!(
        harness
            .state
            .identity_road
            .settle(group_request)
            .await
            .expect("generic Enrichment suppression is a normal road outcome"),
        ilr::IdentityRoadOutcome::Deferred {
            reason: ilr::DeferReason(STANDING_DISMISSAL.to_string())
        }
    );
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(group_anchor.own_work_id)).await,
        group_before,
        "generic Enrichment suppression takes the non-settled/false branch"
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::GenericSettlement,
        ilr::ReviewKind::GroupIdentity,
    );

    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        ol_key,
    )
    .await;
    let proposed = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: gr_key.to_string(),
    };
    dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::EnrichmentPass,
        proposed,
    )
    .await;

    let mut refreshed = harness
        .state
        .work_service
        .refresh(
            harness.user_id,
            work.own_work_id,
            livrarr_domain::services::RefreshSurface::Bulk,
        )
        .await
        .expect("production background/add refresh");
    let handoff = refreshed
        .route_handoff
        .take()
        .expect("provider result carries the add-background handoff");
    let before = machine_snapshot(&harness.db, harness.user_id, Some(work.own_work_id)).await;
    clear_mint_trace();
    let outcome = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::EnrichmentPass,
            handoff,
        )
        .await
        .expect("production add-background handoff accepts suppression");
    assert_eq!(
        outcome, None,
        "all-suppressed handoff is the adapter's successful no-op"
    );
    assert_eq!(
        machine_snapshot(&harness.db, harness.user_id, Some(work.own_work_id)).await,
        before,
        "the false/non-settled branch performs no proposal mutation"
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
    );
}

// RED-UNTIL-U5-R2 (OAI-U5-008): a successful enrichment with a fresh route
// handoff is currently rewritten to its old Failed/Unenriched status before
// the caller learns that the handoff settles normally.
#[tokio::test]
async fn successful_enrichment_with_settled_handoff_ends_complete() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Successful Enrichment";
    let author_name = "U5 Successful Enrichment Author";
    let ol_key = "OL-U5-SUCCESSFUL-ENRICHMENT-W";
    let gr_key = "9555198";
    let harness = build_route_harness_with_open_library(open_library_route_detail(
        title,
        author_name,
        ol_key,
        gr_key,
    ))
    .await;
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        ol_key,
    )
    .await;
    let before = WorkDb::get_work(&harness.db, harness.user_id, work.own_work_id)
        .await
        .unwrap();
    assert!(matches!(
        before.enrichment_status,
        livrarr_domain::EnrichmentStatus::Failed | livrarr_domain::EnrichmentStatus::Unenriched
    ));
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);

    let mut refreshed = harness
        .state
        .work_service
        .refresh(
            harness.user_id,
            work.own_work_id,
            livrarr_domain::services::RefreshSurface::Bulk,
        )
        .await
        .expect("ordinary enrichment pass succeeds");
    assert!(!refreshed.provider_unavailable);
    let handoff = refreshed
        .route_handoff
        .take()
        .expect("successful enrichment produced a fresh route handoff");
    let handoff_result = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::EnrichmentPass,
            handoff,
        )
        .await
        .expect("ordinary enrichment handoff is accepted");
    assert!(
        matches!(
            &handoff_result,
            Some(ilr::IdentityRoadOutcome::Settled { work_id, .. })
                if *work_id == work.own_work_id
        ) || handoff_result.is_none(),
        "the status pin applies only to a settled or proposal-less handoff: {handoff_result:?}"
    );

    let persisted = WorkDb::get_work(&harness.db, harness.user_id, work.own_work_id)
        .await
        .unwrap();
    assert!(
        matches!(
            persisted.enrichment_status,
            livrarr_domain::EnrichmentStatus::Enriched | livrarr_domain::EnrichmentStatus::Thin
        ),
        "successful enrichment must not remain {:?} after a settled handoff",
        persisted.enrichment_status
    );
}

// RED-UNTIL-U5: today the registered ManualRefresh route's equivalent captured proposal reuses or mints a PendingRoute card because no standing ledger is consulted.
#[tokio::test]
async fn manual_refresh_returns_ordinary_response_and_leaves_anchor_unchanged() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Manual Refresh";
    let author_name = "U5 Refresh Author";
    let ol_key = "OL-U5-REFRESH-W";
    let gr_key = "9555102";
    let harness = build_route_harness_with_open_library(open_library_route_detail(
        title,
        author_name,
        ol_key,
        gr_key,
    ))
    .await;
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        ol_key,
    )
    .await;
    dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ManualRefresh,
        ilr::RouteKey {
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            value: gr_key.to_string(),
        },
    )
    .await;
    let anchor_before = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let cards_before = pending_card_count(&harness.db, harness.user_id).await;
    let notifications_before = notification_count(&harness.db, harness.user_id).await;
    let audits_before = identity_audit_count(&harness.db, harness.user_id).await;

    clear_mint_trace();
    let response = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/work/{}/refresh", work.own_work_id),
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert!(
        response.json.get("work").is_some(),
        "ordinary refresh body: {}",
        response.json
    );
    assert_eq!(
        captured(&harness.db, harness.user_id, work.own_work_id).await,
        anchor_before
    );
    assert_eq!(
        pending_card_count(&harness.db, harness.user_id).await,
        cards_before
    );
    assert_eq!(
        notification_count(&harness.db, harness.user_id).await,
        notifications_before
    );
    assert_eq!(
        identity_audit_count(&harness.db, harness.user_id).await,
        audits_before
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
    );
}

// OAI-U5-201 control: Retry-All must decide from the resolved handoff, not from
// the mere presence of a handoff object. A same-Work settlement therefore keeps
// the successful refresh status and counts the Work as recovered.
#[tokio::test]
async fn convergence_visit_retry_all_settled_handoff_retains_complete_and_counts_recovered() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Retry Settled";
    let author_name = "U5 Retry Settled Author";
    let ol_key = "OL-U5-RETRY-SETTLED-W";
    let gr_key = "9555201";
    let harness = build_route_harness_with_open_library(open_library_route_detail(
        title,
        author_name,
        ol_key,
        gr_key,
    ))
    .await;
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        ol_key,
    )
    .await;
    WorkDb::update_work_enrichment(
        &harness.db,
        harness.user_id,
        work.own_work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: livrarr_domain::EnrichmentStatus::Failed,
            ..Default::default()
        },
    )
    .await
    .expect("make the settled-control Retry-All Work incomplete");
    let identity_before = captured(&harness.db, harness.user_id, work.own_work_id).await;
    assert!(!identity_before.active_routes.iter().any(|route| {
        route.provider == ilr::IdentityProvider::Goodreads
            && route.kind == ilr::RouteKind::GoodreadsWork
            && route.provider_scoped_id == gr_key
    }));
    let calls_before = harness.open_library_stub.as_ref().unwrap().call_count();
    let notifications_before = notification_count(&harness.db, harness.user_id).await;

    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work/retry-incomplete",
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if harness.open_library_stub.as_ref().unwrap().call_count() == calls_before + 1
                && notification_count(&harness.db, harness.user_id).await
                    == notifications_before + 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("one real Retry-All provider chase, settled handoff, and result notification");

    let identity_after = captured(&harness.db, harness.user_id, work.own_work_id).await;
    assert!(
        identity_after.active_routes.iter().any(|route| {
            route.provider == ilr::IdentityProvider::Goodreads
                && route.kind == ilr::RouteKind::GoodreadsWork
                && route.provider_scoped_id == gr_key
        }),
        "the real ConvergenceVisit handoff settles the fresh route onto the same Work"
    );
    assert_eq!(
        identity_after.identity_generation,
        identity_before.identity_generation + 1,
        "same-Work settlement claims exactly one generation"
    );
    let persisted = WorkDb::get_work(&harness.db, harness.user_id, work.own_work_id)
        .await
        .expect("read settled-control Retry-All Work");
    assert!(
        matches!(
            persisted.enrichment_status,
            livrarr_domain::EnrichmentStatus::Enriched
                | livrarr_domain::EnrichmentStatus::Thin
        ),
        "a handoff object that settles for the same Work must retain the successful refresh status, observed {:?}",
        persisted.enrichment_status
    );
    let (message, summary) = latest_bulk_enrichment_result(&harness.db, harness.user_id).await;
    assert_eq!(summary["total"].as_u64(), Some(1), "{message}");
    assert_eq!(summary["recovered"].as_u64(), Some(1), "{message}");
    assert_eq!(summary["still_incomplete"].as_u64(), Some(0), "{message}");
}

// RED-UNTIL-U5-R3 (OAI-U5-201): suppression now returns the correct successful
// all-suppressed None, but Retry-All still declares recovery before that result
// is known and never restores the Work's recorded incomplete status afterward.
#[tokio::test]
async fn convergence_visit_retry_all_stays_incomplete_with_one_card_or_miss_pass() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Retry Standing";
    let author_name = "U5 Retry Author";
    let ol_key = "OL-U5-RETRY-SEED-W";
    let gr_key = "9555103";
    let harness = build_route_harness_with_open_library(open_library_route_detail(
        title,
        author_name,
        ol_key,
        gr_key,
    ))
    .await;
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        ol_key,
    )
    .await;
    WorkDb::update_work_enrichment(
        &harness.db,
        harness.user_id,
        work.own_work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: livrarr_domain::EnrichmentStatus::Failed,
            ..Default::default()
        },
    )
    .await
    .expect("make the Retry-All Work incomplete");
    let prior_incomplete_status = WorkDb::get_work(&harness.db, harness.user_id, work.own_work_id)
        .await
        .expect("read the prior incomplete Retry-All status")
        .enrichment_status;
    assert_eq!(
        prior_incomplete_status,
        livrarr_domain::EnrichmentStatus::Failed
    );
    dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        ilr::RouteKey {
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            value: gr_key.to_string(),
        },
    )
    .await;
    let before = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let calls_before = harness.open_library_stub.as_ref().unwrap().call_count();
    let notifications_before = notification_count(&harness.db, harness.user_id).await;
    let cards_before = pending_card_count(&harness.db, harness.user_id).await;
    let audits_before = identity_audit_count(&harness.db, harness.user_id).await;
    let review_notifications_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id=?1 AND type='identityReviewNeeded'",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();

    clear_mint_trace();
    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work/retry-incomplete",
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if harness.open_library_stub.as_ref().unwrap().call_count() == calls_before + 1
                && notification_count(&harness.db, harness.user_id).await
                    == notifications_before + 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("one real Retry-All provider chase and handoff");
    let trace = take_mint_trace();
    assert_eq!(
        trace.len(),
        1,
        "one fired provider chase records one CardOrMiss proposal pass"
    );
    assert!(matches!(
        trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::SuppressedByDismissal
    ));
    let review_notifications_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id=?1 AND type='identityReviewNeeded'",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        review_notifications_after, review_notifications_before,
        "the only new notification is the ordinary BulkEnrichmentComplete result"
    );
    assert_eq!(
        pending_card_count(&harness.db, harness.user_id).await,
        cards_before,
        "suppression creates no review card"
    );
    assert_eq!(
        identity_audit_count(&harness.db, harness.user_id).await,
        audits_before,
        "suppression creates no identity audit"
    );
    assert_eq!(
        captured(&harness.db, harness.user_id, work.own_work_id).await,
        before
    );
    let persisted = WorkDb::get_work(&harness.db, harness.user_id, work.own_work_id)
        .await
        .expect("read incomplete Work");
    let (message, summary) = latest_bulk_enrichment_result(&harness.db, harness.user_id).await;
    let observed_recovered = summary["recovered"].as_u64();
    let observed_still_incomplete = summary["still_incomplete"].as_u64();
    assert!(
        persisted.enrichment_status == prior_incomplete_status
            && observed_recovered == Some(0)
            && observed_still_incomplete == Some(1),
        "after the all-suppressed handoff resolved None: prior status={prior_incomplete_status:?}, observed status={:?}, recovered={observed_recovered:?}, still_incomplete={observed_still_incomplete:?}, notification={message}",
        persisted.enrichment_status
    );
}

// RED-UNTIL-U5: today a captured handoff reuses the dismissed proposal and returns it before considering the changed-key sibling; no proposal can be skipped as SuppressedByDismissal.
#[tokio::test]
async fn captured_handoff_skips_suppressed_proposal_and_mints_changed_key_sibling() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Mixed Handoff Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Mixed Handoff",
        None,
    )
    .await;
    let suppressed = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555104".to_string(),
    };
    let changed = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-MIXED-B-W".to_string(),
    };
    let dismissed = dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        suppressed.clone(),
    )
    .await;
    let generation = captured(&harness.db, harness.user_id, work.own_work_id)
        .await
        .identity_generation;

    clear_mint_trace();
    let outcome = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            ilr::CapturedRouteHandoff {
                metadata_generation: generation,
                provider_identity: Vec::new(),
                route_proposals: vec![suppressed, changed.clone()],
            },
        )
        .await
        .expect("mixed captured handoff")
        .expect("changed-key sibling returns a card");
    let ilr::IdentityRoadOutcome::ReviewPending {
        review_id,
        kind: ilr::ReviewKind::PendingRoute,
        ..
    } = outcome
    else {
        panic!("changed-key sibling must park one PendingRoute")
    };
    assert_ne!(review_id, dismissed);
    let payload: String =
        sqlx::query_scalar("SELECT payload FROM identity_review_cards WHERE id=?1")
            .bind(review_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let payload: ilr::SettlementReviewCard = serde_json::from_str(&payload).unwrap();
    assert!(matches!(
        payload,
        ilr::SettlementReviewCard::PendingRoute { candidate, .. }
            if candidate.route == changed
    ));
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 2);
    assert!(matches!(
        trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::SuppressedByDismissal
    ));
    assert!(matches!(
        trace[1].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(ref card)
            if card.id == review_id
    ));
}

// RED-UNTIL-U5-R2 (OAI-U5-003): the provider-identity fallback mints a
// changed-key sibling after generic settlement is deferred, but discards that
// successful card and always reports None.
#[tokio::test]
async fn provider_identity_handoff_returns_changed_sibling_and_none_only_when_all_suppressed() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Provider Handoff Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Provider Handoff",
        None,
    )
    .await;
    let suppressed = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555193".to_string(),
    };
    let changed = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-PROVIDER-HANDOFF-B-W".to_string(),
    };
    dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        suppressed.clone(),
    )
    .await;
    let evidence = |route: ilr::RouteKey| ilr::ProviderIdentityEvidence {
        provider: route.provider.clone(),
        route,
        work_core: None,
        provenance: Default::default(),
    };
    let notifications_before = notification_count(&harness.db, harness.user_id).await;
    let generation = captured(&harness.db, harness.user_id, work.own_work_id)
        .await
        .identity_generation;

    clear_mint_trace();
    let outcome = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            ilr::CapturedRouteHandoff {
                metadata_generation: generation,
                provider_identity: vec![evidence(suppressed.clone()), evidence(changed.clone())],
                route_proposals: Vec::new(),
            },
        )
        .await
        .expect("mixed provider-identity handoff")
        .expect("changed-key provider sibling returns ReviewPending");
    let ilr::IdentityRoadOutcome::ReviewPending {
        review_id,
        kind: ilr::ReviewKind::PendingRoute,
        ..
    } = outcome
    else {
        panic!("changed-key provider sibling must return PendingRoute ReviewPending")
    };
    let payload: String =
        sqlx::query_scalar("SELECT payload FROM identity_review_cards WHERE id=?1")
            .bind(review_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let payload: ilr::SettlementReviewCard = serde_json::from_str(&payload).unwrap();
    assert!(matches!(
        payload,
        ilr::SettlementReviewCard::PendingRoute { candidate, .. }
            if candidate.route == changed
    ));
    assert_eq!(
        notification_count(&harness.db, harness.user_id).await,
        notifications_before + 1,
        "the surviving sibling has its one review notification"
    );

    assert_eq!(
        dismiss_card(&harness, review_id).await.status,
        StatusCode::NO_CONTENT
    );
    let notifications_before_all_suppressed =
        notification_count(&harness.db, harness.user_id).await;
    let generation = captured(&harness.db, harness.user_id, work.own_work_id)
        .await
        .identity_generation;
    clear_mint_trace();
    let all_suppressed = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            ilr::CapturedRouteHandoff {
                metadata_generation: generation,
                provider_identity: vec![evidence(suppressed), evidence(changed)],
                route_proposals: Vec::new(),
            },
        )
        .await
        .expect("all-suppressed provider-identity handoff");
    assert_eq!(
        all_suppressed, None,
        "None means every candidate was suppressed"
    );
    assert_eq!(
        notification_count(&harness.db, harness.user_id).await,
        notifications_before_all_suppressed
    );
    assert!(
        pending_card_ids(&harness.db, harness.user_id, ilr::ReviewKind::PendingRoute)
            .await
            .is_empty()
    );
}

// RED-UNTIL-U5: today a second startup title-policy heal after user Dismiss mints the equivalent GroupIdentity card again because cancellation is not durable and no ledger is consulted.
#[tokio::test]
async fn startup_heal_suppresses_on_second_boot_but_changed_cohort_key_mints() {
    let _serial = TRACE_LOCK.lock().await;
    let db = create_activated_test_db().await;
    let user_id = seed_user(&db, "startup-standing").await;
    let (author_id, anchor_id, _peer_id) = seed_title_heal_collision(&db, user_id).await;

    let first = livrarr_db::pool::heal_identity_title_policy(db.pool())
        .await
        .expect("first title-policy collision boot");
    assert_eq!(first.review_cards_minted, 1);
    let first_id = pending_card_ids(&db, user_id, ilr::ReviewKind::GroupIdentity).await[0];
    WorkIdentityRepository::dismiss_pending_review(
        &db,
        ReviewActor::AuthenticatedUser { user_id },
        first_id,
    )
    .await
    .expect("real typed Dismiss repository writer");
    let audits_before = identity_audit_count(&db, user_id).await;
    let notifications_before = notification_count(&db, user_id).await;
    let generation_before = captured(&db, user_id, anchor_id).await.identity_generation;

    clear_mint_trace();
    let second = livrarr_db::pool::heal_identity_title_policy(db.pool())
        .await
        .expect("second boot with standing dismissal");
    assert_eq!(second.review_cards_minted, 0);
    assert!(
        pending_card_ids(&db, user_id, ilr::ReviewKind::GroupIdentity)
            .await
            .is_empty()
    );
    assert_eq!(identity_audit_count(&db, user_id).await, audits_before);
    assert_eq!(notification_count(&db, user_id).await, notifications_before);
    assert_eq!(
        captured(&db, user_id, anchor_id).await.identity_generation,
        generation_before
    );
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_title_policy_generation'",
    )
    .fetch_optional(db.pool())
    .await
    .unwrap();
    assert!(
        marker.is_none(),
        "blocked collision keeps the existing heal marker unset"
    );
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::StartupTitleHeal,
        ilr::ReviewKind::GroupIdentity,
    );

    let author_name: String = sqlx::query_scalar("SELECT name FROM authors WHERE id=?1")
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    let third = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "U5 Cloud Cuckoo Land third legacy".to_string(),
            author_name,
            normalized_title: "u5 cloud cuckoo land third legacy".to_string(),
            normalized_author: "u5 startup heal author".to_string(),
            author_id: Some(author_id),
            ..Default::default()
        })
        .await
        .expect("seed changed cohort member")
        .0;
    // Constructed-state compatibility justification: the changed-key control
    // is another pre-policy Work tuple. The production writer creates the row;
    // only the legacy title-policy columns consumed by the real startup heal
    // are shaped, exactly as in `seed_title_heal_collision`.
    sqlx::query(
        "UPDATE works SET title='U5 Cloud Cuckoo Land (Large-Print Edition)', \
                normalized_title='u5 cloud cuckoo land (large-print edition)', \
                normalized_identity_main='u5 cloud cuckoo land (large-print edition)', \
                normalized_identity_subtitle='large print edition', \
                normalized_identity_volume='', primary_author_id=?1, \
                text_distinction='common', identity_generation=1 \
          WHERE user_id=?2 AND id=?3",
    )
    .bind(author_id)
    .bind(user_id)
    .bind(third.id)
    .execute(db.pool())
    .await
    .expect("shape changed cohort member");
    clear_mint_trace();
    let changed = livrarr_db::pool::heal_identity_title_policy(db.pool())
        .await
        .expect("boot after cohort-key change");
    assert_eq!(changed.review_cards_minted, 1);
    assert_eq!(
        pending_card_ids(&db, user_id, ilr::ReviewKind::GroupIdentity)
            .await
            .len(),
        1,
        "sorted cohort Work ids are a semantic key element"
    );
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 1);
    assert!(matches!(
        trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    ));
}

// RED-UNTIL-U5-R2: today's collision uniquifier imports a second file instead
// of preserving the pre-existing occupied-destination skip rule. The Edition
// dismissal itself is a normal successful no-op at this seat.
#[tokio::test]
async fn live_manual_import_apply_work_evidence_recurrence_is_normal_success() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let library_root = harness._tmp.path().join("u5-edition-library");
    let incoming = harness._tmp.path().join("u5-edition-incoming");
    std::fs::create_dir_all(&library_root).expect("create U5 Edition library root");
    std::fs::create_dir_all(&incoming).expect("create U5 Edition incoming root");
    harness
        .db
        .create_root_folder(
            library_root.to_str().expect("UTF-8 U5 Edition root"),
            MediaType::Ebook,
        )
        .await
        .expect("register U5 Edition root");
    let author = "U5 Manual Edition Author";
    let title = "U5 Manual Edition Standing";
    seed_author(&harness.db, harness.user_id, author).await;
    let seed = drive_manual_import(
        &harness,
        &incoming.join("seed-en.epub"),
        title,
        author,
        "en",
    )
    .await;
    assert_eq!(seed.status, StatusCode::OK, "{}", seed.json);
    assert_eq!(seed.json["results"][0]["status"], "imported");
    let work_id = seed.json["results"][0]["workId"].as_i64().unwrap();
    let edition_id: i64 = sqlx::query_scalar(
        "SELECT id FROM editions WHERE user_id=?1 AND work_id=?2 AND state='active'",
    )
    .bind(harness.user_id)
    .bind(work_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    let before_edition = edition_row_bytes(&harness.db, edition_id).await;

    let first = drive_manual_import(
        &harness,
        &incoming.join("conflict-fr-first.epub"),
        title,
        author,
        "fr",
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.json);
    assert_eq!(first.json["results"][0]["status"], "failed");
    let card_id = edition_card_ids(&harness.db, harness.user_id, edition_id).await[0];
    assert_eq!(
        dismiss_card(&harness, card_id).await.status,
        StatusCode::NO_CONTENT
    );
    let files_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM library_items WHERE user_id=?1 AND work_id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let cards_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1")
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let notifications_before = notification_count(&harness.db, harness.user_id).await;
    let audit_cursor: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(id), 0) FROM identity_audit_events WHERE user_id=?1",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();

    clear_mint_trace();
    let replay = drive_manual_import(
        &harness,
        &incoming.join("conflict-fr-replay.epub"),
        title,
        author,
        "fr",
    )
    .await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.json);
    assert_eq!(
        replay.json["results"][0]["status"], "skipped",
        "{}",
        replay.json
    );
    assert!(replay.json["results"][0]["error"].as_str().is_none());
    assert_eq!(replay.json["results"][0]["workId"].as_i64(), Some(work_id));
    assert_eq!(
        edition_row_bytes(&harness.db, edition_id).await,
        before_edition
    );
    let files_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM library_items WHERE user_id=?1 AND work_id=?2")
            .bind(harness.user_id)
            .bind(work_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(
        files_after, files_before,
        "an occupied computed destination is the normal already-imported skip"
    );
    let cards_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1")
            .bind(harness.user_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(
        cards_after, cards_before,
        "suppression mints no review card"
    );
    assert_eq!(
        notification_count(&harness.db, harness.user_id).await,
        notifications_before,
        "suppression sends no notification"
    );
    let replay_audit_kinds: Vec<String> = sqlx::query_scalar(
        "SELECT event_kind FROM identity_audit_events \
          WHERE user_id=?1 AND id>?2 ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(audit_cursor)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert!(
        replay_audit_kinds.iter().all(|kind| kind == "settlement"),
        "Edition suppression adds no audit; ordinary exact-hint attach settlement audits are tolerated: {replay_audit_kinds:?}"
    );
    // AC-005 forbids the contradictory Edition proposal's effects. It does
    // not freeze the surrounding successful ManualImport attach: Work
    // generation and its settlement audit are deliberately tolerated so U2's
    // Author effects and exact-hint settlement remain observable.
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::ApplyWorkEvidence,
        ilr::ReviewKind::EditionEvidence,
    );
}

// RED-UNTIL-U5: today no tombstone exists for an explicit DirectAdd to bypass; the repeated proposal merely mints/reuses under U7 and cannot prove that validated user intent, rather than DoorKind alone, controls suppression.
#[tokio::test]
async fn direct_add_explicit_create_bypasses_equivalent_group_tombstone() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_name = "U5 Direct Choice Author";
    let title = "U5 Direct Choice (Large Print Edition)";
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Direct Choice",
        Some("audited-distinction"),
    )
    .await;
    let request = group_machine_request(
        harness.user_id,
        author_id,
        title,
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsBookEdition,
        "9555201",
    );
    let (dismissed, _) = mint_group_from_machine_request(&harness, request).await;
    assert_eq!(
        dismiss_card(&harness, dismissed).await.status,
        StatusCode::NO_CONTENT
    );
    assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::GroupIdentity,
        dismissed,
    )
    .await;
    let ledger_before = ledger_rows(&harness.db, harness.user_id).await;

    clear_mint_trace();
    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work",
        Some(json!({
            "olKey": null,
            "title": title,
            "authorName": author_name,
            "authorOlKey": null,
            "year": 2026,
            "coverUrl": null,
            "language": "en",
            "detailUrl": null,
            "coverManual": false,
            "isbn13": null,
            "candidateId": null,
            "hcKey": null,
            "grKey": "9555201",
            "asin": null
        })),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.json);
    assert!(response
        .json
        .to_string()
        .contains("direct add requires review card"));
    let pending =
        pending_card_ids(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity).await;
    assert_eq!(pending.len(), 1);
    assert_ne!(
        pending[0], dismissed,
        "explicit create mints the ordinary new review"
    );
    assert_eq!(
        ledger_rows(&harness.db, harness.user_id).await,
        ledger_before,
        "DirectAdd bypasses but does not revoke"
    );
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 1);
    assert!(matches!(
        trace[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    ));
}

// RED-UNTIL-U5: today no tombstone exists for ManualImport's explicit minimum/candidate choice to bypass; recurrence simply reuses or mints and cannot distinguish the user's intent from an automatic producer.
#[tokio::test]
async fn manual_import_explicit_choice_bypasses_equivalent_group_tombstone() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness_with_empty_author_search().await;
    let author = "U5 Manual Choice Author";
    let title = "U5 Manual Choice (Large Print Edition)";
    let author_id = seed_author(&harness.db, harness.user_id, author).await;
    seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Manual Choice",
        Some("audited-distinction"),
    )
    .await;
    let request = group_machine_request(
        harness.user_id,
        author_id,
        title,
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::IsbnRegistry,
        ilr::RouteKind::Isbn13Edition,
        "9780306406157",
    );
    let (dismissed, _) = mint_group_from_machine_request(&harness, request).await;
    assert_eq!(
        dismiss_card(&harness, dismissed).await.status,
        StatusCode::NO_CONTENT
    );
    assert_one_active_ledger(
        &harness.db,
        harness.user_id,
        ilr::ReviewKind::GroupIdentity,
        dismissed,
    )
    .await;
    let ledger_before = ledger_rows(&harness.db, harness.user_id).await;
    let library_root = harness._tmp.path().join("u5-choice-library");
    let incoming = harness._tmp.path().join("u5-choice-incoming");
    std::fs::create_dir_all(&library_root).unwrap();
    std::fs::create_dir_all(&incoming).unwrap();
    harness
        .db
        .create_root_folder(library_root.to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();
    let path = incoming.join("explicit-choice.epub");
    write_epub(&path, title, "en");

    clear_mint_trace();
    let response = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/manualimport/import",
        Some(json!({"items": [{
            "path": path,
            "olKey": "",
            "title": title,
            "author": author,
            "deleteExisting": false,
            "language": "en",
            "authorOlKey": null,
            "year": 2026,
            "coverUrl": null,
            "isbn": "9780306406157",
            "description": null,
            "seriesName": null,
            "seriesPosition": null,
            "candidateId": null,
            "hcKey": null,
            "grKey": null,
            "asin": null
        }]})),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(
        response.json["results"][0]["status"], "failed",
        "{}",
        response.json
    );
    assert!(
        response.json["results"][0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("manual import requires review card")),
        "{}",
        response.json
    );
    let pending =
        pending_card_ids(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity).await;
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0], dismissed);
    assert_eq!(
        ledger_rows(&harness.db, harness.user_id).await,
        ledger_before,
        "ManualImport choice bypasses but does not revoke"
    );
    assert!(matches!(
        take_mint_trace()[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    ));
}

// RED-UNTIL-U5: today the helper never suppresses, so owner-insensitivity, per-user isolation, and changed semantic elements cannot be distinguished from ordinary post-dismiss reminting.
#[tokio::test]
async fn pending_key_excludes_owner_but_includes_user_work_and_trimmed_route_tuple() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Pending Key Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Pending Key",
        None,
    )
    .await;
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-PENDING-KEY-W".to_string(),
    };
    let dismissed = mint_pending_from_handoff(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        route.clone(),
    )
    .await;
    assert_eq!(
        dismiss_card(&harness, dismissed).await.status,
        StatusCode::NO_CONTENT
    );

    let work_owned = pending_card(
        pending_candidate(
            work.own_work_id,
            route.provider.clone(),
            route.kind.clone(),
            route.value.clone(),
            ilr::RouteOwner::Work(work.own_work_id),
        ),
        work.own_work_id,
    );
    let edition_owned = pending_card(
        pending_candidate(
            work.own_work_id,
            route.provider.clone(),
            route.kind.clone(),
            format!("  {}  ", route.value),
            ilr::RouteOwner::Edition(9_555_202),
        ),
        work.own_work_id,
    );
    assert_eq!(
        ilr::ReviewDismissalKeyV1::from_card(harness.user_id, &work_owned, Some(work.own_work_id))
            .unwrap()
            .canonical_json()
            .unwrap(),
        ilr::ReviewDismissalKeyV1::from_card(
            harness.user_id,
            &edition_owned,
            Some(work.own_work_id),
        )
        .unwrap()
        .canonical_json()
        .unwrap(),
        "PendingRoute owner is excluded while whitespace is trimmed"
    );
    clear_mint_trace();
    let owner_changed = harness
        .state
        .identity_road
        .apply_captured_route_handoff(
            harness.user_id,
            work.own_work_id,
            ilr::IdentityRoadOrigin::ConvergenceVisit,
            ilr::CapturedRouteHandoff {
                metadata_generation: captured(&harness.db, harness.user_id, work.own_work_id)
                    .await
                    .identity_generation,
                provider_identity: Vec::new(),
                route_proposals: vec![route.clone()],
            },
        )
        .await
        .expect("owner-insensitive replay");
    assert_eq!(owner_changed, None);
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::CapturedRouteHandoff,
        ilr::ReviewKind::PendingRoute,
    );

    let changed = mint_pending_from_handoff(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        ilr::RouteKey {
            value: "OL-U5-PENDING-KEY-CHANGED-W".to_string(),
            ..route.clone()
        },
    )
    .await;
    assert_ne!(changed, dismissed, "changed semantic route value mints");

    let other_user = seed_user(&harness.db, "pending-key-other-user").await;
    let other_author = seed_author(&harness.db, other_user, "U5 Pending Key Author").await;
    let other_work = seed_work(
        &harness.db,
        other_user,
        other_author,
        "U5 Pending Key",
        None,
    )
    .await;
    let other = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        other_user,
        other_work.own_work_id,
        other_work.identity_generation,
        pending_candidate(
            other_work.own_work_id,
            route.provider,
            route.kind,
            route.value,
            ilr::RouteOwner::Work(other_work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("another user's same tuple is unaffected");
    assert_eq!(card_status(&harness.db, other.id).await, "pending");
}

// RED-UNTIL-U5: today there is no standing ledger to challenge the three inline origins; update, merge-with-choices, and affirm mint then resolve only because SuppressedByDismissal is unreachable.
#[tokio::test]
async fn registered_update_merge_and_affirm_bypass_exact_tombstones_byte_identically() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Inline Author").await;
    let update = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Inline Update",
        None,
    )
    .await;
    let survivor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Inline Survivor",
        None,
    )
    .await;
    let loser = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Inline Loser",
        None,
    )
    .await;
    let affirm = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Inline Affirm",
        None,
    )
    .await;

    let update_request = ilr::IdentityRoadRequest {
        user_id: harness.user_id,
        origin: ilr::IdentityRoadOrigin::WorkUpdateRekey,
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: None,
            owned_files: Vec::new(),
            provider_identity: Vec::new(),
            minimum: Some(ilr::MinimumWorkEvidence {
                title: "U5 Inline Updated Title".to_string(),
                authors: vec![author_id],
            }),
        },
        interaction: ilr::IdentityRoadInteraction::HumanWatching,
        existing_work_id: Some(update.own_work_id),
    };
    let update_card = match harness
        .state
        .identity_road
        .settle(update_request)
        .await
        .expect("preseed exact update proposal")
    {
        ilr::IdentityRoadOutcome::ReviewPending { review_id, .. } => review_id,
        other => panic!("update must originate review, got {other:?}"),
    };
    assert_eq!(
        dismiss_card(&harness, update_card).await.status,
        StatusCode::NO_CONTENT
    );

    let choices = vec![livrarr_domain::services::MergeFieldChoiceEntry {
        field: livrarr_domain::services::MergeableField::SeriesName,
        choice: livrarr_domain::services::MergeFieldChoice::KeepSurvivor,
    }];
    let merge_request = ilr::IdentityRoadRequest {
        user_id: harness.user_id,
        origin: ilr::IdentityRoadOrigin::ManualWorkMerge {
            loser_work_id: loser.own_work_id,
            choices: choices.clone(),
        },
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: None,
            owned_files: Vec::new(),
            provider_identity: Vec::new(),
            minimum: Some(ilr::MinimumWorkEvidence {
                title: "U5 Inline Survivor".to_string(),
                authors: vec![author_id],
            }),
        },
        interaction: ilr::IdentityRoadInteraction::HumanWatching,
        existing_work_id: Some(survivor.own_work_id),
    };
    let merge_card = match harness
        .state
        .identity_road
        .settle(merge_request)
        .await
        .expect("preseed exact merge proposal")
    {
        ilr::IdentityRoadOutcome::ReviewPending { review_id, .. } => review_id,
        other => panic!("merge must originate review, got {other:?}"),
    };
    assert_eq!(
        dismiss_card(&harness, merge_card).await.status,
        StatusCode::NO_CONTENT
    );

    let affirm_value = "9555203";
    livrarr_db::test_helpers::record_pending_anchor_fixture(
        &harness.db,
        affirm.own_work_id,
        AnchorType::new(AnchorType::GR_WORK),
        affirm_value,
    )
    .await
    .expect("seed pending affirm anchor through production writer");
    let affirm_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsBookEdition,
        value: affirm_value.to_string(),
    };
    let affirm_request = ilr::IdentityRoadRequest {
        user_id: harness.user_id,
        origin: ilr::IdentityRoadOrigin::AffirmPendingRoute,
        evidence: ilr::IdentityEvidenceBundle {
            user_choice: None,
            owned_files: Vec::new(),
            provider_identity: vec![ilr::ProviderIdentityEvidence {
                provider: ilr::IdentityProvider::Goodreads,
                route: affirm_route,
                work_core: None,
                provenance: Default::default(),
            }],
            minimum: None,
        },
        interaction: ilr::IdentityRoadInteraction::HumanWatching,
        existing_work_id: Some(affirm.own_work_id),
    };
    let affirm_card = match harness
        .state
        .identity_road
        .settle(affirm_request)
        .await
        .expect("preseed exact affirm proposal")
    {
        ilr::IdentityRoadOutcome::ReviewPending { review_id, .. } => review_id,
        other => panic!("affirm must originate review, got {other:?}"),
    };
    assert_eq!(
        dismiss_card(&harness, affirm_card).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        active_ledger_count(&harness.db, harness.user_id).await,
        3,
        "all three exact inline tombstones exist before their registered bypasses"
    );
    let notifications_before = notification_count(&harness.db, harness.user_id).await;

    clear_mint_trace();
    let updated = call_router_json(
        &harness,
        Method::PUT,
        format!("/api/v1/work/{}", update.own_work_id),
        Some(json!({"title": "U5 Inline Updated Title"})),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["title"], "U5 Inline Updated Title");
    let merged = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/merge/{}",
            survivor.own_work_id, loser.own_work_id
        ),
        Some(json!({"choices": [{
            "field": "series_name",
            "choice": "keep_survivor"
        }]})),
    )
    .await;
    assert_eq!(merged.status, StatusCode::OK, "{}", merged.json);
    let affirmed = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/pending-anchors/gr_work/affirm",
            affirm.own_work_id
        ),
        None,
    )
    .await;
    assert_eq!(affirmed.status, StatusCode::NO_CONTENT, "{}", affirmed.json);

    let source_payloads: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM identity_review_cards WHERE id IN (?1,?2,?3) \
          ORDER BY CASE id WHEN ?1 THEN 1 WHEN ?2 THEN 2 ELSE 3 END",
    )
    .bind(update_card)
    .bind(merge_card)
    .bind(affirm_card)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    let inline_cards: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT kind,status,payload FROM identity_review_cards \
          WHERE user_id=?1 AND id>?2 ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(affirm_card)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        inline_cards,
        vec![
            (
                "GroupIdentity".to_string(),
                "resolved".to_string(),
                source_payloads[0].clone(),
            ),
            (
                "GroupIdentity".to_string(),
                "resolved".to_string(),
                source_payloads[1].clone(),
            ),
            (
                "PendingRoute".to_string(),
                "resolved".to_string(),
                source_payloads[2].clone(),
            ),
        ],
        "the registered doors retain their exact proposal bytes and mint-then-resolve statuses"
    );
    assert_eq!(
        notification_count(&harness.db, harness.user_id).await,
        notifications_before
    );
    let trace = take_mint_trace();
    assert_eq!(trace.len(), 3);
    assert!(trace.iter().all(|row| matches!(
        row.outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    )));
}

// RED-UNTIL-U5: today WorkService::commit_identity_edit changes the certified route but no dismissal row exists to revoke, and a transaction failure therefore cannot prove mutation/revoke rollback.
#[tokio::test]
async fn certified_identity_edit_revokes_every_member_group_and_pending_key_atomically() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Certified Edit";
    let author_name = "U5 Certified Edit Author";
    let old_ol = "OL9555301W";
    let new_ol = "OL9555302W";
    let harness =
        build_route_harness_with_open_library(livrarr_external_data::NormalizedWorkDetail {
            title: Some(title.to_string()),
            author_name: Some(author_name.to_string()),
            ol_key: Some(new_ol.to_string()),
            ..Default::default()
        })
        .await;
    let author_id = seed_author(&harness.db, harness.user_id, author_name).await;
    let group_anchor = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Certified Edit Cohort Anchor",
        None,
    )
    .await;
    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        old_ol,
    )
    .await;
    let group_members = vec![group_anchor.own_work_id, work.own_work_id];
    let group_id = WorkIdentityRepository::commit_settlement(
        &harness.db,
        settlement_with_card(
            &group_anchor,
            harness.user_id,
            ilr::SettlementReviewCard::GroupIdentity {
                work_ids: group_members.clone(),
                proposed_identity: None,
                merge_choices: Vec::new(),
            },
        ),
    )
    .await
    .expect("mint multi-Work GroupIdentity through generic production settlement")
    .review_cards[0]
        .id;
    assert_eq!(
        dismiss_card(&harness, group_id).await.status,
        StatusCode::NO_CONTENT
    );
    let current = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let pending_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555303".to_string(),
    };
    let pending_id = dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        pending_route.clone(),
    )
    .await;
    let edition = seed_edition(&harness.db, harness.user_id, work.own_work_id).await;
    assert!(
        contradict_edition_direct(&harness.db, harness.user_id, edition.id)
            .await
            .is_err()
    );
    let edition_id = edition_card_ids(&harness.db, harness.user_id, edition.id).await[0];
    assert_eq!(
        dismiss_card(&harness, edition_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        active_ledger_for_kind(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity).await,
        1
    );
    assert_eq!(
        active_ledger_for_kind(&harness.db, harness.user_id, ilr::ReviewKind::PendingRoute).await,
        1
    );
    assert_eq!(
        active_ledger_for_kind(
            &harness.db,
            harness.user_id,
            ilr::ReviewKind::EditionEvidence
        )
        .await,
        1
    );
    let group_ledger_id: i64 = sqlx::query_scalar(
        "SELECT id FROM identity_review_dismissals \
          WHERE user_id=?1 AND kind='GroupIdentity' AND revoked_at IS NULL",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        ledger_work_ids(&harness.db, group_ledger_id).await,
        group_members,
        "the edited Work is a non-anchor member of the Group tombstone"
    );

    // Compatibility justification (verbatim from AC-005): production writer; its HTTP handlers are unregistered.
    // The suite calls WorkService directly while retaining its real preview
    // provider and SQLite repository.
    let preview = harness
        .state
        .work_service
        .preview_identity_edit(
            harness.user_id,
            work.own_work_id,
            new_ol,
            Some(AnchorType::new(AnchorType::OL_WORK)),
        )
        .await
        .expect("certified identity-edit preview");
    let token = preview.preview_id.expect("certifiable edit preview");
    let before_failure = captured(&harness.db, harness.user_id, work.own_work_id).await;
    install_revoke_abort(&harness.db, "certified_edit").await;
    let failed = harness
        .state
        .work_service
        .commit_identity_edit(
            harness.user_id,
            work.own_work_id,
            AnchorType::new(AnchorType::OL_WORK),
            &token,
        )
        .await;
    assert!(
        failed.is_err(),
        "forced ledger failure aborts the certified action"
    );
    assert_eq!(
        captured(&harness.db, harness.user_id, work.own_work_id).await,
        before_failure
    );
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 3);
    drop_revoke_abort(&harness.db, "certified_edit").await;

    let retry_preview = harness
        .state
        .work_service
        .preview_identity_edit(
            harness.user_id,
            work.own_work_id,
            new_ol,
            Some(AnchorType::new(AnchorType::OL_WORK)),
        )
        .await
        .expect("retry certified preview");
    let committed = harness
        .state
        .work_service
        .commit_identity_edit(
            harness.user_id,
            work.own_work_id,
            AnchorType::new(AnchorType::OL_WORK),
            retry_preview.preview_id.as_deref().unwrap(),
        )
        .await
        .expect("certified edit and revocation commit together");
    assert!(!committed.no_op);
    assert_eq!(committed.new_value, new_ol);
    assert_eq!(
        active_ledger_for_kind(&harness.db, harness.user_id, ilr::ReviewKind::GroupIdentity).await,
        0
    );
    assert_eq!(
        active_ledger_for_kind(&harness.db, harness.user_id, ilr::ReviewKind::PendingRoute).await,
        0
    );
    assert_eq!(
        active_ledger_for_kind(
            &harness.db,
            harness.user_id,
            ilr::ReviewKind::EditionEvidence
        )
        .await,
        1,
        "EditionEvidence has no U5 revocation policy"
    );
    let revoked: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT kind,revoked_at,revoke_reason FROM identity_review_dismissals \
          WHERE user_id=?1 AND kind IN ('GroupIdentity','PendingRoute') ORDER BY kind",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(revoked.len(), 2);
    assert!(revoked.iter().all(|row| {
        row.1.is_some()
            && row
                .2
                .as_deref()
                .is_some_and(|reason| !reason.trim().is_empty())
    }));
    assert_eq!(card_status(&harness.db, group_id).await, "cancelled");
    assert_eq!(card_status(&harness.db, pending_id).await, "cancelled");
    assert_eq!(card_status(&harness.db, edition_id).await, "cancelled");

    let group_anchor_after = captured(&harness.db, harness.user_id, group_anchor.own_work_id).await;
    let reminted_group = WorkIdentityRepository::commit_settlement(
        &harness.db,
        settlement_with_card(
            &group_anchor_after,
            harness.user_id,
            ilr::SettlementReviewCard::GroupIdentity {
                work_ids: group_members.clone(),
                proposed_identity: None,
                merge_choices: Vec::new(),
            },
        ),
    )
    .await
    .expect("same multi-Work Group key mints after explicit revoke")
    .review_cards[0]
        .id;
    let generation = captured(&harness.db, harness.user_id, work.own_work_id)
        .await
        .identity_generation;
    let reminted_pending = mint_pending_from_handoff(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        pending_route,
    )
    .await;
    assert_ne!(reminted_group, group_id);
    assert_ne!(reminted_pending, pending_id);
    assert_eq!(
        dismiss_card(&harness, reminted_group).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        dismiss_card(&harness, reminted_pending).await.status,
        StatusCode::NO_CONTENT
    );
    let reactivated: Vec<(String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT kind,source_card_id,revoked_at,revoke_reason \
           FROM identity_review_dismissals \
          WHERE user_id=?1 AND kind IN ('GroupIdentity','PendingRoute') ORDER BY kind",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        reactivated,
        vec![
            ("GroupIdentity".to_string(), reminted_group, None, None),
            ("PendingRoute".to_string(), reminted_pending, None, None),
        ],
        "a later Dismiss reactivates and updates the one semantic ledger row"
    );
    assert_eq!(ledger_rows(&harness.db, harness.user_id).await.len(), 3);
    let reactivated_group_id: i64 = sqlx::query_scalar(
        "SELECT id FROM identity_review_dismissals \
          WHERE user_id=?1 AND kind='GroupIdentity' AND revoked_at IS NULL",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        ledger_work_ids(&harness.db, reactivated_group_id).await,
        group_members
    );
    assert_eq!(
        captured(&harness.db, harness.user_id, work.own_work_id)
            .await
            .identity_generation,
        generation,
        "PendingRoute mint does not mutate the Work"
    );
    assert_eq!(current.own_work_id, work.own_work_id);
}

// RED-UNTIL-U5: today a same-value certified edit and a stale preview have no ledger effects because revocation does not exist anywhere.
#[tokio::test]
async fn certified_same_value_revokes_but_stale_preview_revokes_nothing() {
    let _serial = TRACE_LOCK.lock().await;
    let title = "U5 Same Certified Value";
    let author = "U5 Same Certified Author";
    let ol_key = "OL9555310W";
    let harness =
        build_route_harness_with_open_library(livrarr_external_data::NormalizedWorkDetail {
            title: Some(title.to_string()),
            author_name: Some(author.to_string()),
            ol_key: Some(ol_key.to_string()),
            ..Default::default()
        })
        .await;
    let author_id = seed_author(&harness.db, harness.user_id, author).await;
    let work = seed_work_with_route(
        &harness.db,
        harness.user_id,
        author_id,
        title,
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        ol_key,
    )
    .await;
    let first = dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        ilr::RouteKey {
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            value: "9555311".to_string(),
        },
    )
    .await;
    let preview = harness
        .state
        .work_service
        .preview_identity_edit(
            harness.user_id,
            work.own_work_id,
            ol_key,
            Some(AnchorType::new(AnchorType::OL_WORK)),
        )
        .await
        .expect("same-value certified preview");
    let committed = harness
        .state
        .work_service
        .commit_identity_edit(
            harness.user_id,
            work.own_work_id,
            AnchorType::new(AnchorType::OL_WORK),
            preview.preview_id.as_deref().unwrap(),
        )
        .await
        .expect("same-value certification still runs the transactional writer");
    assert!(
        !committed.no_op,
        "provider-set value is newly user-certified"
    );
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    assert_eq!(card_status(&harness.db, first).await, "cancelled");

    let current = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let second = dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        ilr::RouteKey {
            provider: ilr::IdentityProvider::Hardcover,
            kind: ilr::RouteKind::HardcoverWork,
            value: "HC-U5-STALE-W".to_string(),
        },
    )
    .await;
    let stale_value = "OL9555312W";
    let stale_preview = harness
        .state
        .work_service
        .preview_identity_edit(
            harness.user_id,
            work.own_work_id,
            stale_value,
            Some(AnchorType::new(AnchorType::OL_WORK)),
        )
        .await
        .expect("preview before forced staleness");
    let latest = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let mut bump = settlement_with_card(
        &latest,
        harness.user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: Some(work.own_work_id),
            invariant: "removed before commit; U5 stale-preview generation bump".to_string(),
        },
    );
    bump.review_cards.clear();
    WorkIdentityRepository::commit_settlement(&harness.db, bump)
        .await
        .expect("force preview stale through production generation claim");
    let before_stale = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let stale = harness
        .state
        .work_service
        .commit_identity_edit(
            harness.user_id,
            work.own_work_id,
            AnchorType::new(AnchorType::OL_WORK),
            stale_preview.preview_id.as_deref().unwrap(),
        )
        .await;
    assert!(matches!(
        stale,
        Err(livrarr_domain::identity_edit::IdentityEditError::StalePreview)
    ));
    assert_eq!(
        captured(&harness.db, harness.user_id, work.own_work_id).await,
        before_stale
    );
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 1);
    assert_eq!(card_status(&harness.db, second).await, "cancelled");
    assert!(current.identity_generation < before_stale.identity_generation);
}

// RED-UNTIL-U5: today the registered PendingRoute Affirm resolves its inline card and route but cannot revoke any Group/Pending standing dismissal, and no failure can exercise mutation/revoke rollback.
#[tokio::test]
async fn registered_affirm_revokes_all_member_keys_in_its_continuation_transaction() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Affirm Revoke Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Affirm Revoke",
        None,
    )
    .await;
    let group_id = mint_group_heal_card(&harness.db, &work, harness.user_id).await;
    assert_eq!(
        dismiss_card(&harness, group_id).await.status,
        StatusCode::NO_CONTENT
    );
    let route_value = "9555320";
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: route_value.to_string(),
    };
    let pending_id = dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        route.clone(),
    )
    .await;
    livrarr_db::test_helpers::record_pending_anchor_fixture(
        &harness.db,
        work.own_work_id,
        AnchorType::new(AnchorType::GR_WORK),
        route_value,
    )
    .await
    .expect("seed registered affirm input");
    let before_failure = captured(&harness.db, harness.user_id, work.own_work_id).await;

    install_revoke_abort(&harness.db, "affirm").await;
    let failed = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/pending-anchors/gr_work/affirm",
            work.own_work_id
        ),
        None,
    )
    .await;
    assert_eq!(
        failed.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        failed.json
    );
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 2);
    let after_failure = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let mut expected_after_failure = before_failure.clone();
    expected_after_failure.identity_generation += 1;
    assert_eq!(
        after_failure, expected_after_failure,
        "step-one settlement mint commits its generation claim, while the aborted continuation applies no route or other identity mutation"
    );
    let continuation_cards: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id,status FROM identity_review_cards \
          WHERE user_id=?1 AND work_id=?2 AND id>?3 ORDER BY id",
    )
    .bind(harness.user_id)
    .bind(work.own_work_id)
    .bind(pending_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        continuation_cards.len(),
        1,
        "step one commits exactly one continuation card"
    );
    let continuation_card_id = continuation_cards[0].0;
    assert_eq!(
        continuation_cards[0].1, "pending",
        "the revoke-aborted continuation does not resolve its card"
    );
    let pending_anchor: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
          WHERE user_id=?1 AND work_id=?2 AND anchor_type='gr_work' \
            AND anchor_value=?3 AND confidence='pending'",
    )
    .bind(harness.user_id)
    .bind(work.own_work_id)
    .bind(route_value)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(pending_anchor, 1);
    drop_revoke_abort(&harness.db, "affirm").await;

    let affirmed = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/pending-anchors/gr_work/affirm",
            work.own_work_id
        ),
        None,
    )
    .await;
    assert_eq!(affirmed.status, StatusCode::NO_CONTENT, "{}", affirmed.json);
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    assert_eq!(
        card_status(&harness.db, continuation_card_id).await,
        "resolved",
        "retry resolves the card left pending by the aborted continuation"
    );
    let reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT revoke_reason FROM identity_review_dismissals \
          WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(reasons.len(), 2);
    assert!(reasons.iter().all(|reason| reason
        .as_deref()
        .is_some_and(|reason| !reason.trim().is_empty())));
    assert_eq!(card_status(&harness.db, group_id).await, "cancelled");
    assert_eq!(card_status(&harness.db, pending_id).await, "cancelled");

    let now = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let next_group = mint_group_heal_card(&harness.db, &now, harness.user_id).await;
    let next_pending = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        harness.user_id,
        work.own_work_id,
        captured(&harness.db, harness.user_id, work.own_work_id)
            .await
            .identity_generation,
        pending_candidate(
            work.own_work_id,
            route.provider,
            route.kind,
            route.value,
            ilr::RouteOwner::Work(work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("same-key machine proposal mints after explicit revoke");
    assert_ne!(next_group, group_id);
    assert_ne!(next_pending.id, pending_id);
}

// RED-UNTIL-U5: today title/author update, merge, DirectAdd, ListImport choice, and refused EditionEvidence resolution have no ledger to preserve; the negative revocation policy is unobservable.
#[tokio::test]
async fn all_non_revoking_user_actions_preserve_tombstones_and_reopen_no_card() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Nonrevoke Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Nonrevoke Work",
        None,
    )
    .await;
    let group_id = mint_group_heal_card(&harness.db, &work, harness.user_id).await;
    assert_eq!(
        dismiss_card(&harness, group_id).await.status,
        StatusCode::NO_CONTENT
    );
    let pending_id = dismiss_pending_route_key(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        ilr::RouteKey {
            provider: ilr::IdentityProvider::Goodreads,
            kind: ilr::RouteKind::GoodreadsWork,
            value: "9555330".to_string(),
        },
    )
    .await;
    let edition = seed_edition(&harness.db, harness.user_id, work.own_work_id).await;
    assert!(
        contradict_edition_direct(&harness.db, harness.user_id, edition.id)
            .await
            .is_err()
    );
    let edition_id = edition_card_ids(&harness.db, harness.user_id, edition.id).await[0];
    assert_eq!(
        dismiss_card(&harness, edition_id).await.status,
        StatusCode::NO_CONTENT
    );
    let ledger_before = ledger_rows(&harness.db, harness.user_id).await;

    let updated = call_router_json(
        &harness,
        Method::PUT,
        format!("/api/v1/work/{}", work.own_work_id),
        Some(json!({
            "title": "U5 Nonrevoke Work Updated",
            "authorName": "U5 Nonrevoke Author Updated"
        })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 3);

    let loser = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Nonrevoke Merge Loser",
        None,
    )
    .await;
    let merged = call_router_json(
        &harness,
        Method::POST,
        format!(
            "/api/v1/work/{}/merge/{}",
            work.own_work_id, loser.own_work_id
        ),
        Some(json!({"choices": [{
            "field": "series_name",
            "choice": "keep_survivor"
        }]})),
    )
    .await;
    assert_eq!(merged.status, StatusCode::OK, "{}", merged.json);
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 3);

    let direct = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work",
        Some(json!({
            "olKey": null,
            "title": "U5 Nonrevoke Direct",
            "authorName": "U5 Nonrevoke Direct Author",
            "authorOlKey": null,
            "year": null,
            "coverUrl": null,
            "language": "en",
            "detailUrl": null,
            "coverManual": false,
            "isbn13": null,
            "candidateId": null,
            "hcKey": null,
            "grKey": null,
            "asin": null
        })),
    )
    .await;
    assert!(direct.status.is_success(), "{}", direct.json);
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 3);

    let list_target = captured(&harness.db, harness.user_id, work.own_work_id).await;
    WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        harness.user_id,
        list_target.own_work_id,
        list_target.identity_generation,
        pending_candidate(
            list_target.own_work_id,
            ilr::IdentityProvider::Hardcover,
            ilr::RouteKind::HardcoverWork,
            "HC-U5-NONREVOKE-LIST-W",
            ilr::RouteOwner::Work(list_target.own_work_id),
        ),
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ListImport),
        true,
    )
    .await
    .expect("validated ListImport choice retains ordinary behavior");
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 3);

    let other_work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Refused Edition Review",
        None,
    )
    .await;
    let other_edition = seed_edition(&harness.db, harness.user_id, other_work.own_work_id).await;
    assert!(
        contradict_edition_direct(&harness.db, harness.user_id, other_edition.id)
            .await
            .is_err()
    );
    let refused_id = edition_card_ids(&harness.db, harness.user_id, other_edition.id).await[0];
    let refused_generation: i64 =
        sqlx::query_scalar("SELECT generation FROM identity_review_cards WHERE id=?1")
            .bind(refused_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    let refused = call_router_json(
        &harness,
        Method::POST,
        format!("/api/v1/identity-review-card/{refused_id}/resolve"),
        Some(
            json!({"command": ilr::ReviewResolutionCommand::EditionEvidence {
                card_id: refused_id,
                expected_generation: refused_generation,
                action: ilr::EditionEvidenceAction::RetainUnknownOrAbsent,
            }}),
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 3);

    let after = ledger_rows(&harness.db, harness.user_id).await;
    assert_eq!(
        after, ledger_before,
        "non-revoking actions leave ledger bytes unchanged"
    );
    for card_id in [group_id, pending_id, edition_id] {
        assert_eq!(card_status(&harness.db, card_id).await, "cancelled");
    }
    clear_mint_trace();
    let suppressed = contradict_edition_direct(&harness.db, harness.user_id, edition.id)
        .await
        .expect("EditionEvidence remains suppressed as normal success");
    assert_eq!(suppressed.edition.id, edition.id);
    assert_suppressed_trace(
        livrarr_db::identity_layer::ReviewCardMintSite::ApplyEvidence,
        ilr::ReviewKind::EditionEvidence,
    );
}

// PIN: satisfied-route cancellation is an identity-engine cleanup, writes no dismissal ledger, and therefore a later identical proposal reaches the normal mint path rather than suppression.
#[tokio::test]
async fn satisfied_route_machine_cancellation_writes_no_tombstone() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Satisfied Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Satisfied Route",
        None,
    )
    .await;
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555401".to_string(),
    };
    let card_id = mint_pending_from_handoff(
        &harness,
        work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        route.clone(),
    )
    .await;
    let current = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let mut settle = settlement_with_card(
        &current,
        harness.user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: Some(work.own_work_id),
            invariant: "removed before commit; satisfied-route fixture".to_string(),
        },
    );
    settle.review_cards.clear();
    settle.routes.push(ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: ilr::RouteOwner::Work(work.own_work_id),
        resolved_work_id: work.own_work_id,
        provider: route.provider.clone(),
        kind: route.kind.clone(),
        provider_scoped_id: route.value.clone(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(route.provider.clone()),
        user_confirmed: false,
        observed_at: Utc::now(),
    });
    let settled = WorkIdentityRepository::commit_settlement(&harness.db, settle)
        .await
        .expect("production settlement satisfies route");
    assert_eq!(card_status(&harness.db, card_id).await, "cancelled");
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    let actor: String = sqlx::query_scalar(
        "SELECT actor FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind='pending-route-satisfied' ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(actor, "identity-engine");

    clear_mint_trace();
    let reminted = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        harness.user_id,
        work.own_work_id,
        settled.identity.identity_generation,
        pending_candidate(
            work.own_work_id,
            route.provider,
            route.kind,
            route.value,
            ilr::RouteOwner::Work(work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("same proposal is not suppressed after machine cancellation");
    assert_ne!(reminted.id, card_id);
    assert!(matches!(
        take_mint_trace()[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    ));
}

// PIN: dedup-residue heal's machine cancellation is not a ReviewActor Dismiss, writes no ledger, and the same semantic proposal continues through U7's normal reuse path.
#[tokio::test]
async fn dedup_residue_machine_cancellation_writes_no_tombstone() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Dedup Heal Author").await;
    let work = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Dedup Heal",
        None,
    )
    .await;
    let mut proposed_title = work.identity_title.clone();
    proposed_title.volume = Some("99".to_string());
    proposed_title.normalized_volume = "99".to_string();
    let card = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![work.own_work_id],
        proposed_identity: Some(ilr::WorkIdentityEvidence {
            title: proposed_title,
            primary_author_id: author_id,
            routes: Vec::new(),
        }),
        merge_choices: Vec::new(),
    };
    let payload = serde_json::to_string(&card).unwrap();
    // Constructed-state compatibility justification: equivalent pending
    // duplicates are historical pre-U7 state. Both rows use the exact typed
    // payload accepted by the production settlement writer; direct insertion
    // is required because no correct U7 writer can create the later sibling.
    let mut ids = Vec::new();
    for offset in [0_i64, 1_i64] {
        ids.push(
            sqlx::query(
                "INSERT INTO identity_review_cards \
                    (user_id,work_id,kind,generation,status,payload,created_at) \
                 VALUES (?1,?2,'GroupIdentity',?3,'pending',?4,?5)",
            )
            .bind(harness.user_id)
            .bind(work.own_work_id)
            .bind(work.identity_generation)
            .bind(&payload)
            .bind((Utc::now() + chrono::Duration::seconds(offset)).to_rfc3339())
            .execute(harness.db.pool())
            .await
            .unwrap()
            .last_insert_rowid(),
        );
    }
    let report = livrarr_db::identity_layer::heal_identity_dedup_residue(harness.db.pool())
        .await
        .expect("production dedup-residue heal");
    assert_eq!(report.duplicate_cards_cancelled, 1);
    assert_eq!(card_status(&harness.db, ids[0]).await, "pending");
    assert_eq!(card_status(&harness.db, ids[1]).await, "cancelled");
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);

    clear_mint_trace();
    let current = captured(&harness.db, harness.user_id, work.own_work_id).await;
    let reused = WorkIdentityRepository::commit_settlement(
        &harness.db,
        settlement_with_card(&current, harness.user_id, card),
    )
    .await
    .expect("same proposal proceeds normally")
    .review_cards[0]
        .id;
    assert_eq!(reused, ids[0]);
    assert!(matches!(
        take_mint_trace()[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::ReusedPending(_)
    ));
}

// PIN: sweep-heal cancellation uses the machine actor, writes no ledger, and the same PendingRoute proposal mints normally when submitted again.
#[tokio::test]
async fn sweep_heal_machine_cancellation_writes_no_tombstone() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let author_id = seed_author(&harness.db, harness.user_id, "U5 Sweep Author").await;
    let owner = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Sweep Owner",
        None,
    )
    .await;
    let target = seed_work(
        &harness.db,
        harness.user_id,
        author_id,
        "U5 Sweep Target",
        None,
    )
    .await;
    let route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555402".to_string(),
    };
    let mut owner_commit = settlement_with_card(
        &owner,
        harness.user_id,
        ilr::SettlementReviewCard::InvariantRepair {
            work_id: Some(owner.own_work_id),
            invariant: "removed before commit; sweep owner fixture".to_string(),
        },
    );
    owner_commit.review_cards.clear();
    owner_commit.routes.push(ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: ilr::RouteOwner::Work(owner.own_work_id),
        resolved_work_id: owner.own_work_id,
        provider: route.provider.clone(),
        kind: route.kind.clone(),
        provider_scoped_id: route.value.clone(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::Provider(route.provider.clone()),
        user_confirmed: false,
        observed_at: Utc::now(),
    });
    WorkIdentityRepository::commit_settlement(&harness.db, owner_commit)
        .await
        .expect("seed foreign route owner through production settlement");
    let stale = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        harness.user_id,
        target.own_work_id,
        target.identity_generation,
        pending_candidate(
            target.own_work_id,
            route.provider.clone(),
            route.kind.clone(),
            route.value.clone(),
            ilr::RouteOwner::Work(target.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("seed stale PendingRoute through production writer");
    let report = livrarr_db::pool::heal_identity_sweep_findings(harness.db.pool())
        .await
        .expect("production sweep heal");
    assert_eq!(report.invalid_cards_dismissed, 1);
    assert_eq!(card_status(&harness.db, stale.id).await, "cancelled");
    assert_eq!(active_ledger_count(&harness.db, harness.user_id).await, 0);
    let actor: String = sqlx::query_scalar(
        "SELECT actor FROM identity_audit_events \
          WHERE user_id=?1 AND event_kind='review-dismissal' ORDER BY id DESC LIMIT 1",
    )
    .bind(harness.user_id)
    .fetch_one(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(actor, "identity-sweep-heal");
    assert!(serde_json::from_str::<ReviewActor>(&actor).is_err());

    clear_mint_trace();
    let reminted = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        &harness.db,
        harness.user_id,
        target.own_work_id,
        target.identity_generation,
        pending_candidate(
            target.own_work_id,
            route.provider,
            route.kind,
            route.value,
            ilr::RouteOwner::Work(target.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("same machine proposal mints after sweep cancellation");
    assert_ne!(reminted.id, stale.id);
    assert!(matches!(
        take_mint_trace()[0].outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    ));
}

// RED-UNTIL-U5: today there is no marker-gated dismissal-adoption pass, no ledger or Work-membership index to populate, and no adoption failpoint; every historical cancelled card remains ordinary audit history and every equivalent producer can ask again.
#[tokio::test]
#[traced_test]
async fn upgrade_adopts_exact_u1_through_u15_fixture_once_and_replays_by_policy() {
    let _serial = TRACE_LOCK.lock().await;
    let harness = build_route_harness().await;
    let user_id = harness.user_id;
    let db = &harness.db;
    let base = Utc::now() - chrono::Duration::hours(1);
    let mut ids = BTreeMap::<&'static str, i64>::new();

    // u1 — user-dismissed GroupIdentity through generic production settlement.
    let u1_author = seed_author(db, user_id, "U5 Adoption U1 Author").await;
    seed_work(
        db,
        user_id,
        u1_author,
        "U5 Adoption U1",
        Some("audited-distinction"),
    )
    .await;
    let u1_request = group_machine_request(
        user_id,
        u1_author,
        "U5 Adoption U1",
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-U5-ADOPT-U1-W",
    );
    let (u1, _) = mint_group_from_machine_request(&harness, u1_request.clone()).await;
    legacy_user_dismiss_at(db, user_id, u1, base + chrono::Duration::seconds(1)).await;
    ids.insert("u1", u1);

    // u2 — user-dismissed PendingRoute.
    let u2_author = seed_author(db, user_id, "U5 Adoption U2 Author").await;
    let u2_work = seed_work(db, user_id, u2_author, "U5 Adoption U2", None).await;
    let u2_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555502".to_string(),
    };
    let u2 = mint_pending_from_handoff(
        &harness,
        u2_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u2_route.clone(),
    )
    .await;
    legacy_user_dismiss_at(db, user_id, u2, base + chrono::Duration::seconds(2)).await;
    ids.insert("u2", u2);

    // u3 — EditionEvidence from callerless apply_evidence.
    let u3_author = seed_author(db, user_id, "U5 Adoption U3 Author").await;
    let u3_work = seed_work(db, user_id, u3_author, "U5 Adoption U3", None).await;
    let u3_edition = seed_edition(db, user_id, u3_work.own_work_id).await;
    assert!(contradict_edition_direct(db, user_id, u3_edition.id)
        .await
        .is_err());
    let u3 = edition_card_ids(db, user_id, u3_edition.id).await[0];
    legacy_user_dismiss_at(db, user_id, u3, base + chrono::Duration::seconds(3)).await;
    ids.insert("u3", u3);

    // u4 — EditionEvidence from apply_work_evidence.
    let u4_author = seed_author(db, user_id, "U5 Adoption U4 Author").await;
    let u4_work = seed_work(db, user_id, u4_author, "U5 Adoption U4", None).await;
    let u4_edition = EditionRepository::apply_work_evidence(
        db,
        ilr::EditionWorkEvidenceCommand {
            user_id,
            work_id: u4_work.own_work_id,
            format: ilr::EditionFormat::Ebook,
            language: Some("en".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .unwrap()
    .edition;
    assert!(EditionRepository::apply_work_evidence(
        db,
        ilr::EditionWorkEvidenceCommand {
            user_id,
            work_id: u4_work.own_work_id,
            format: ilr::EditionFormat::Audiobook,
            language: Some("fr".to_string()),
            provenance: ilr::EvidenceProvenance::OwnedFile,
        },
    )
    .await
    .is_err());
    let u4 = edition_card_ids(db, user_id, u4_edition.id).await[0];
    legacy_user_dismiss_at(db, user_id, u4, base + chrono::Duration::seconds(4)).await;
    ids.insert("u4", u4);

    // u5 — certified edit before dismissal, then a later machine re-observation.
    let u5_author = seed_author(db, user_id, "U5 Adoption U5 Author").await;
    let u5_work = seed_work(db, user_id, u5_author, "U5 Adoption U5", None).await;
    LegacyWorkIdentityRepository::apply_identity_edit(
        db,
        u5_work.own_work_id,
        user_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL9555505W",
        u5_work.identity_generation,
        &[],
    )
    .await
    .expect("u5 historical certified edit before Dismiss");
    let u5_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Hardcover,
        kind: ilr::RouteKind::HardcoverWork,
        value: "HC-U5-ADOPT-U5-W".to_string(),
    };
    let u5 = mint_pending_from_handoff(
        &harness,
        u5_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u5_route.clone(),
    )
    .await;
    legacy_user_dismiss_at(db, user_id, u5, base + chrono::Duration::seconds(5)).await;
    // Constructed-state compatibility justification: the later machine
    // observation existed before U5 and updates only the operational timestamp
    // that ST-019 says adoption must ignore.
    sqlx::query(
        "UPDATE identity_routes SET observed_at=?1 \
          WHERE user_id=?2 AND resolved_work_id=?3",
    )
    .bind((base + chrono::Duration::seconds(50)).to_rfc3339())
    .bind(user_id)
    .bind(u5_work.own_work_id)
    .execute(db.pool())
    .await
    .expect("u5 reconstruct later machine observed_at refresh");
    ids.insert("u5", u5);

    // u6 — dismissal followed by the irrecoverable pre-upgrade certified edit.
    let u6_author = seed_author(db, user_id, "U5 Adoption U6 Author").await;
    let u6_work = seed_work(db, user_id, u6_author, "U5 Adoption U6", None).await;
    let u6_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Goodreads,
        kind: ilr::RouteKind::GoodreadsWork,
        value: "9555506".to_string(),
    };
    let u6 = mint_pending_from_handoff(
        &harness,
        u6_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u6_route.clone(),
    )
    .await;
    legacy_user_dismiss_at(db, user_id, u6, base + chrono::Duration::seconds(6)).await;
    let u6_generation = captured(db, user_id, u6_work.own_work_id)
        .await
        .identity_generation;
    LegacyWorkIdentityRepository::apply_identity_edit(
        db,
        u6_work.own_work_id,
        user_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL9555506W",
        u6_generation,
        &[],
    )
    .await
    .expect("u6 historical certified edit after Dismiss");
    ids.insert("u6", u6);

    // u7 — dismissal followed by a later real PendingRoute Affirm on the Work.
    let u7_author = seed_author(db, user_id, "U5 Adoption U7 Author").await;
    let u7_work = seed_work(db, user_id, u7_author, "U5 Adoption U7", None).await;
    let u7_dismissed_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-ADOPT-U7-A-W".to_string(),
    };
    let u7 = mint_pending_from_handoff(
        &harness,
        u7_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u7_dismissed_route.clone(),
    )
    .await;
    legacy_user_dismiss_at(db, user_id, u7, base + chrono::Duration::seconds(7)).await;
    let u7_affirm_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Hardcover,
        kind: ilr::RouteKind::HardcoverWork,
        value: "HC-U5-ADOPT-U7-B-W".to_string(),
    };
    let u7_affirm = match harness
        .state
        .identity_road
        .settle(ilr::IdentityRoadRequest {
            user_id,
            origin: ilr::IdentityRoadOrigin::AffirmPendingRoute,
            evidence: ilr::IdentityEvidenceBundle {
                user_choice: None,
                owned_files: Vec::new(),
                provider_identity: vec![ilr::ProviderIdentityEvidence {
                    provider: u7_affirm_route.provider.clone(),
                    route: u7_affirm_route.clone(),
                    work_core: None,
                    provenance: Default::default(),
                }],
                minimum: None,
            },
            interaction: ilr::IdentityRoadInteraction::HumanWatching,
            existing_work_id: Some(u7_work.own_work_id),
        })
        .await
        .expect("u7 originate later production PendingRoute Affirm")
    {
        ilr::IdentityRoadOutcome::ReviewPending {
            review_id,
            kind: ilr::ReviewKind::PendingRoute,
            ..
        } => review_id,
        other => panic!("u7 later Affirm must originate review, got {other:?}"),
    };
    let u7_generation: i64 =
        sqlx::query_scalar("SELECT generation FROM identity_review_cards WHERE id=?1")
            .bind(u7_affirm)
            .fetch_one(db.pool())
            .await
            .unwrap();
    harness
        .state
        .identity_road
        .resolve_review(
            ReviewActor::AuthenticatedUser { user_id },
            ReviewResolutionCommand::PendingRoute {
                card_id: u7_affirm,
                expected_generation: u7_generation,
                action: ilr::PendingRouteAction::Affirm {
                    surviving_routes: vec![u7_affirm_route],
                },
            },
        )
        .await
        .expect("u7 later production PendingRoute Affirm");
    ids.insert("u7", u7);

    // u8 — dismissal followed by DirectAdd and separately validated ListImport choice.
    let u8_author = seed_author(db, user_id, "U5 Adoption U8 Author").await;
    let _u8_anchor = seed_work(
        db,
        user_id,
        u8_author,
        "U5 Adoption U8",
        Some("audited-distinction"),
    )
    .await;
    let u8_request = group_machine_request(
        user_id,
        u8_author,
        "U5 Adoption U8",
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsBookEdition,
        "9555508",
    );
    let (u8, _) = mint_group_from_machine_request(&harness, u8_request.clone()).await;
    legacy_user_dismiss_at(db, user_id, u8, base + chrono::Duration::seconds(8)).await;
    let direct_title = "U5 Adoption U8 Direct Action";
    let mut direct = group_machine_request(
        user_id,
        u8_author,
        direct_title,
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::DirectAdd),
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-U5-ADOPT-U8-DIRECT-W",
    );
    direct.interaction = ilr::IdentityRoadInteraction::HumanWatching;
    direct.evidence.user_choice = Some(ilr::UserIdentityChoice::ExplicitCreate(
        ilr::MinimumWorkEvidence {
            title: direct_title.to_string(),
            authors: vec![u8_author],
        },
    ));
    let direct_work_id = match harness.state.identity_road.settle(direct).await.unwrap() {
        ilr::IdentityRoadOutcome::Settled {
            work_id, created, ..
        } => {
            assert!(created);
            work_id
        }
        other => panic!("u8 historical DirectAdd must settle, got {other:?}"),
    };

    let u8_list_work = seed_work(db, user_id, u8_author, "U5 Adoption U8 List Action", None).await;
    let u8_list_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::Hardcover,
        kind: ilr::RouteKind::HardcoverWork,
        value: "HC-U5-ADOPT-U8-LIST-W".to_string(),
    };
    let list_outcome = harness
        .state
        .identity_road
        .settle(ilr::IdentityRoadRequest {
            user_id,
            origin: ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::ListImport),
            evidence: ilr::IdentityEvidenceBundle {
                user_choice: Some(ilr::UserIdentityChoice::ExistingWork(
                    u8_list_work.own_work_id,
                )),
                owned_files: Vec::new(),
                provider_identity: vec![ilr::ProviderIdentityEvidence {
                    provider: ilr::IdentityProvider::Hardcover,
                    route: u8_list_route,
                    work_core: None,
                    provenance: Default::default(),
                }],
                minimum: Some(ilr::MinimumWorkEvidence {
                    title: "U5 Adoption U8 List Action".to_string(),
                    authors: vec![u8_author],
                }),
            },
            interaction: ilr::IdentityRoadInteraction::HumanWatching,
            existing_work_id: Some(u8_list_work.own_work_id),
        })
        .await
        .expect("u8 validated ListImport UserChoice route");
    assert!(matches!(
        list_outcome,
        ilr::IdentityRoadOutcome::Settled { work_id, .. }
            if work_id == u8_list_work.own_work_id
    ));
    let u8_user_choice_routes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id IN (?2,?3) AND user_confirmed=1",
    )
    .bind(user_id)
    .bind(direct_work_id)
    .bind(u8_list_work.own_work_id)
    .fetch_one(db.pool())
    .await
    .expect("read u8 historical user-choice routes");
    assert_eq!(
        u8_user_choice_routes, 2,
        "DirectAdd and ListImport each wrote the historical user-confirmed route that adoption must not mistake for a revoke"
    );
    ids.insert("u8", u8);

    // u9/u10/u11 — the three historical machine cancellation actors.
    // Constructed-state compatibility justification: their real current
    // writers are exercised independently above; direct shaping here is
    // required because an upgrade fixture must retain all three old rows
    // simultaneously without running marker-gated startup heals early. The
    // event-kind/actor pairs exactly match those production writers.
    // u9 — satisfied-route machine cancellation.
    let u9_author = seed_author(db, user_id, "U5 Adoption U9 Author").await;
    let u9_work = seed_work(db, user_id, u9_author, "U5 Adoption U9", None).await;
    let u9_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-ADOPT-U9-W".to_string(),
    };
    let u9 = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        db,
        user_id,
        u9_work.own_work_id,
        u9_work.identity_generation,
        pending_candidate(
            u9_work.own_work_id,
            u9_route.provider.clone(),
            u9_route.kind.clone(),
            u9_route.value.clone(),
            ilr::RouteOwner::Work(u9_work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .unwrap()
    .id;
    legacy_machine_cancel_at(
        db,
        user_id,
        u9,
        "pending-route-satisfied",
        "identity-engine",
        base + chrono::Duration::seconds(9),
    )
    .await;
    ids.insert("u9", u9);

    // u10 — dedup-residue GroupIdentity machine cancellation.
    let u10_author = seed_author(db, user_id, "U5 Adoption U10 Author").await;
    let u10_work = seed_work(db, user_id, u10_author, "U5 Adoption U10", None).await;
    let mut u10_title = u10_work.identity_title.clone();
    u10_title.volume = Some("10".to_string());
    u10_title.normalized_volume = "10".to_string();
    let u10_card = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![u10_work.own_work_id],
        proposed_identity: Some(ilr::WorkIdentityEvidence {
            title: u10_title,
            primary_author_id: u10_author,
            routes: Vec::new(),
        }),
        merge_choices: Vec::new(),
    };
    let u10 = WorkIdentityRepository::commit_settlement(
        db,
        settlement_with_card(&u10_work, user_id, u10_card.clone()),
    )
    .await
    .unwrap()
    .review_cards[0]
        .id;
    legacy_machine_cancel_at(
        db,
        user_id,
        u10,
        "review-dismissal",
        "identity-dedup-residue-heal",
        base + chrono::Duration::seconds(10),
    )
    .await;
    ids.insert("u10", u10);

    // u11 — sweep-heal PendingRoute machine cancellation.
    let u11_author = seed_author(db, user_id, "U5 Adoption U11 Author").await;
    let u11_work = seed_work(db, user_id, u11_author, "U5 Adoption U11", None).await;
    let u11_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-ADOPT-U11-W".to_string(),
    };
    let u11 = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        db,
        user_id,
        u11_work.own_work_id,
        u11_work.identity_generation,
        pending_candidate(
            u11_work.own_work_id,
            u11_route.provider.clone(),
            u11_route.kind.clone(),
            u11_route.value.clone(),
            ilr::RouteOwner::Work(u11_work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .unwrap()
    .id;
    legacy_machine_cancel_at(
        db,
        user_id,
        u11,
        "review-dismissal",
        "identity-sweep-heal",
        base + chrono::Duration::seconds(11),
    )
    .await;
    ids.insert("u11", u11);

    // u12 — two equivalent user dismissals; latest resolved_at/card id wins.
    let u12_author = seed_author(db, user_id, "U5 Adoption U12 Author").await;
    seed_work(
        db,
        user_id,
        u12_author,
        "U5 Adoption U12",
        Some("audited-distinction"),
    )
    .await;
    let u12_request = group_machine_request(
        user_id,
        u12_author,
        "U5 Adoption U12",
        ilr::IdentityRoadOrigin::CreationDoor(ilr::DoorKind::AuthorMonitor),
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-U5-ADOPT-U12-W",
    );
    let (u12_first, _) = mint_group_from_machine_request(&harness, u12_request.clone()).await;
    legacy_user_dismiss_at(db, user_id, u12_first, base + chrono::Duration::seconds(12)).await;
    let (u12_latest, _) = mint_group_from_machine_request(&harness, u12_request.clone()).await;
    legacy_user_dismiss_at(
        db,
        user_id,
        u12_latest,
        base + chrono::Duration::seconds(13),
    )
    .await;
    ids.insert("u12-first", u12_first);
    ids.insert("u12-latest", u12_latest);

    // u13 — multi-Work GroupIdentity whose non-anchor member is later deleted.
    let u13_author = seed_author(db, user_id, "U5 Adoption U13 Author").await;
    let u13_anchor = seed_work(db, user_id, u13_author, "U5 Adoption U13 A", None).await;
    let u13_member = seed_work(db, user_id, u13_author, "U5 Adoption U13 B", None).await;
    let u13_card = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![u13_anchor.own_work_id, u13_member.own_work_id],
        proposed_identity: None,
        merge_choices: Vec::new(),
    };
    let u13 = WorkIdentityRepository::commit_settlement(
        db,
        settlement_with_card(&u13_anchor, user_id, u13_card),
    )
    .await
    .unwrap()
    .review_cards[0]
        .id;
    legacy_user_dismiss_at(db, user_id, u13, base + chrono::Duration::seconds(14)).await;
    WorkDb::delete_work(db, user_id, u13_member.own_work_id)
        .await
        .expect("u13 delete non-anchor member through production writer");
    ids.insert("u13", u13);

    // u14 — later historical no-op EditionEvidence review-resolution is not
    // the one PendingRoute Affirm event that blocks adoption. Constructed-state
    // compatibility justification: EditionEvidence continuation is deliberately
    // unregistered in U5, so the serialized historical audit is the only shape
    // that can represent this pre-upgrade no-op without inventing a new door.
    let u14_author = seed_author(db, user_id, "U5 Adoption U14 Author").await;
    let u14_work = seed_work(db, user_id, u14_author, "U5 Adoption U14", None).await;
    let u14_edition = seed_edition(db, user_id, u14_work.own_work_id).await;
    assert!(contradict_edition_direct(db, user_id, u14_edition.id)
        .await
        .is_err());
    let u14 = edition_card_ids(db, user_id, u14_edition.id).await[0];
    legacy_user_dismiss_at(db, user_id, u14, base + chrono::Duration::seconds(15)).await;
    let u14_command = ReviewResolutionCommand::EditionEvidence {
        card_id: u14,
        expected_generation: 0,
        action: ilr::EditionEvidenceAction::RetainUnknownOrAbsent,
    };
    sqlx::query(
        "INSERT INTO identity_audit_events \
            (user_id,work_id,event_kind,actor,payload,created_at) \
         VALUES (?1,?2,'review-resolution',?3,?4,?5)",
    )
    .bind(user_id)
    .bind(u14_work.own_work_id)
    .bind(serde_json::to_string(&ReviewActor::AuthenticatedUser { user_id }).unwrap())
    .bind(serde_json::to_string(&u14_command).unwrap())
    .bind((base + chrono::Duration::seconds(16)).to_rfc3339())
    .execute(db.pool())
    .await
    .expect("reconstruct u14 historical no-op resolution audit");
    ids.insert("u14", u14);

    // u15 — malformed actor and malformed payload are independent skip rows.
    // Constructed-state compatibility justification: valid production writers
    // cannot create malformed retained upgrade data; only the actor/card bytes
    // are corrupted after production cards are created.
    let u15_author = seed_author(db, user_id, "U5 Adoption U15 Author").await;
    let u15_work = seed_work(db, user_id, u15_author, "U5 Adoption U15", None).await;
    let u15_actor_route = ilr::RouteKey {
        provider: ilr::IdentityProvider::OpenLibrary,
        kind: ilr::RouteKind::OpenLibraryWork,
        value: "OL-U5-ADOPT-U15-A-W".to_string(),
    };
    let u15_actor = mint_pending_from_handoff(
        &harness,
        u15_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u15_actor_route.clone(),
    )
    .await;
    legacy_machine_cancel_at(
        db,
        user_id,
        u15_actor,
        "review-dismissal",
        "{malformed-review-actor",
        base + chrono::Duration::seconds(17),
    )
    .await;
    let u15_payload = mint_pending_from_handoff(
        &harness,
        u15_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        ilr::RouteKey {
            value: "OL-U5-ADOPT-U15-B-W".to_string(),
            ..u15_actor_route.clone()
        },
    )
    .await;
    legacy_user_dismiss_at(
        db,
        user_id,
        u15_payload,
        base + chrono::Duration::seconds(18),
    )
    .await;
    sqlx::query("UPDATE identity_review_cards SET payload='{malformed-card' WHERE id=?1")
        .bind(u15_payload)
        .execute(db.pool())
        .await
        .expect("corrupt u15 historical card payload");
    ids.insert("u15-actor", u15_actor);
    ids.insert("u15-payload", u15_payload);

    assert_eq!(active_ledger_count(db, user_id).await, 0);
    assert!(adoption_marker(db).await.is_none());
    let cards_before = adoption_card_snapshot(db, user_id).await;

    livrarr_db::identity_layer::set_identity_db_failpoint_for_tests(
        livrarr_db::identity_layer::IdentityDbFailpoint::DismissalAdoptionBeforeCommit,
    );
    assert!(
        livrarr_db::pool::adopt_identity_review_dismissals(db.pool())
            .await
            .is_err()
    );
    assert!(adoption_marker(db).await.is_none());
    assert_eq!(active_ledger_count(db, user_id).await, 0);
    assert_eq!(adoption_card_snapshot(db, user_id).await, cards_before);

    livrarr_db::pool::adopt_identity_review_dismissals(db.pool())
        .await
        .expect("retry marker-gated U5 adoption");
    assert!(adoption_marker(db).await.is_some());
    let adopted_sources: BTreeSet<i64> = sqlx::query_scalar(
        "SELECT source_card_id FROM identity_review_dismissals \
          WHERE user_id=?1 AND revoked_at IS NULL ORDER BY source_card_id",
    )
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .unwrap()
    .into_iter()
    .collect();
    let expected: BTreeSet<i64> = [
        ids["u1"],
        ids["u2"],
        ids["u3"],
        ids["u4"],
        ids["u5"],
        ids["u6"],
        ids["u8"],
        ids["u12-latest"],
        ids["u14"],
    ]
    .into_iter()
    .collect();
    assert_eq!(adopted_sources, expected);
    for skipped in [
        "u7",
        "u9",
        "u10",
        "u11",
        "u12-first",
        "u13",
        "u15-actor",
        "u15-payload",
    ] {
        assert!(
            !adopted_sources.contains(&ids[skipped]),
            "{skipped} must not adopt"
        );
    }
    assert!(
        adopted_sources.contains(&ids["u5"]),
        "observed_at is ignored"
    );
    assert!(
        adopted_sources.contains(&ids["u6"]),
        "irrecoverable edit timing is conservative"
    );
    assert_eq!(adoption_card_snapshot(db, user_id).await, cards_before);
    assert!(logs_contain("identity dismissal adoption skipped"));

    let rows_before_second = ledger_rows(db, user_id).await;
    let marker_before_second = adoption_marker(db).await;
    livrarr_db::pool::adopt_identity_review_dismissals(db.pool())
        .await
        .expect("second start is a marker-gated no-op");
    assert_eq!(ledger_rows(db, user_id).await, rows_before_second);
    assert_eq!(adoption_marker(db).await, marker_before_second);
    assert_eq!(adoption_card_snapshot(db, user_id).await, cards_before);

    clear_mint_trace();
    assert_eq!(
        harness
            .state
            .identity_road
            .settle(u1_request)
            .await
            .unwrap(),
        ilr::IdentityRoadOutcome::Deferred {
            reason: ilr::DeferReason(STANDING_DISMISSAL.to_string())
        }
    );
    assert_eq!(
        harness
            .state
            .identity_road
            .apply_captured_route_handoff(
                user_id,
                u2_work.own_work_id,
                ilr::IdentityRoadOrigin::ConvergenceVisit,
                ilr::CapturedRouteHandoff {
                    metadata_generation: captured(db, user_id, u2_work.own_work_id)
                        .await
                        .identity_generation,
                    provider_identity: Vec::new(),
                    route_proposals: vec![u2_route],
                },
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        contradict_edition_direct(db, user_id, u3_edition.id)
            .await
            .expect("adopted apply_evidence suppression")
            .edition
            .id,
        u3_edition.id
    );
    assert_eq!(
        EditionRepository::apply_work_evidence(
            db,
            ilr::EditionWorkEvidenceCommand {
                user_id,
                work_id: u4_work.own_work_id,
                format: ilr::EditionFormat::Audiobook,
                language: Some("fr".to_string()),
                provenance: ilr::EvidenceProvenance::OwnedFile,
            },
        )
        .await
        .expect("adopted apply_work_evidence suppression")
        .edition
        .id,
        u4_edition.id
    );
    assert_eq!(
        harness
            .state
            .identity_road
            .apply_captured_route_handoff(
                user_id,
                u5_work.own_work_id,
                ilr::IdentityRoadOrigin::ConvergenceVisit,
                ilr::CapturedRouteHandoff {
                    metadata_generation: captured(db, user_id, u5_work.own_work_id)
                        .await
                        .identity_generation,
                    provider_identity: Vec::new(),
                    route_proposals: vec![u5_route],
                },
            )
            .await
            .expect("adopted u5 machine refresh suppression"),
        None
    );
    assert_eq!(
        harness
            .state
            .identity_road
            .apply_captured_route_handoff(
                user_id,
                u6_work.own_work_id,
                ilr::IdentityRoadOrigin::ConvergenceVisit,
                ilr::CapturedRouteHandoff {
                    metadata_generation: captured(db, user_id, u6_work.own_work_id)
                        .await
                        .identity_generation,
                    provider_identity: Vec::new(),
                    route_proposals: vec![u6_route],
                },
            )
            .await
            .expect("adopted u6 conservative suppression"),
        None
    );
    assert_eq!(
        harness
            .state
            .identity_road
            .settle(u8_request)
            .await
            .expect("adopted u8 machine replay"),
        ilr::IdentityRoadOutcome::Deferred {
            reason: ilr::DeferReason(STANDING_DISMISSAL.to_string())
        }
    );
    assert_eq!(
        harness
            .state
            .identity_road
            .settle(u12_request)
            .await
            .expect("adopted u12-latest machine replay"),
        ilr::IdentityRoadOutcome::Deferred {
            reason: ilr::DeferReason(STANDING_DISMISSAL.to_string())
        }
    );
    assert_eq!(
        contradict_edition_direct(db, user_id, u14_edition.id)
            .await
            .expect("adopted u14 apply_evidence suppression")
            .edition
            .id,
        u14_edition.id
    );
    let adopted_trace = take_mint_trace();
    assert_eq!(
        adopted_trace.len(),
        9,
        "every adopted semantic key is replayed through its real producer"
    );
    assert!(adopted_trace.iter().all(|observation| matches!(
        observation.outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::SuppressedByDismissal
    )));

    clear_mint_trace();
    let u7_remint = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        db,
        user_id,
        u7_work.own_work_id,
        captured(db, user_id, u7_work.own_work_id)
            .await
            .identity_generation,
        pending_candidate(
            u7_work.own_work_id,
            u7_dismissed_route.provider,
            u7_dismissed_route.kind,
            u7_dismissed_route.value,
            ilr::RouteOwner::Work(u7_work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("u7 non-adopted key mints");
    let u15_remint = WorkIdentityRepository::commit_pending_route_review_with_review_context(
        db,
        user_id,
        u15_work.own_work_id,
        captured(db, user_id, u15_work.own_work_id)
            .await
            .identity_generation,
        pending_candidate(
            u15_work.own_work_id,
            u15_actor_route.provider,
            u15_actor_route.kind,
            u15_actor_route.value,
            ilr::RouteOwner::Work(u15_work.own_work_id),
        ),
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        false,
    )
    .await
    .expect("u15 malformed-actor key mints");
    let u9_remint = mint_pending_from_handoff(
        &harness,
        u9_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u9_route,
    )
    .await;
    let u10_current = captured(db, user_id, u10_work.own_work_id).await;
    let u10_remint = WorkIdentityRepository::commit_settlement(
        db,
        settlement_with_card(&u10_current, user_id, u10_card),
    )
    .await
    .expect("u10 machine-cancelled Group key mints")
    .review_cards[0]
        .id;
    let u11_remint = mint_pending_from_handoff(
        &harness,
        u11_work.own_work_id,
        ilr::IdentityRoadOrigin::ConvergenceVisit,
        u11_route,
    )
    .await;
    assert_ne!(u7_remint.id, ids["u7"]);
    assert_ne!(u15_remint.id, ids["u15-actor"]);
    assert_ne!(u9_remint, ids["u9"]);
    assert_ne!(u10_remint, ids["u10"]);
    assert_ne!(u11_remint, ids["u11"]);
    let non_adopted_trace = take_mint_trace();
    assert_eq!(
        non_adopted_trace.len(),
        5,
        "every replayable non-adopted user/machine/malformed-actor key mints"
    );
    assert!(non_adopted_trace.iter().all(|observation| matches!(
        observation.outcome,
        livrarr_db::identity_layer::ReviewCardMintOutcome::Minted(_)
    )));
}

// PIN: version-1 canonical keys sort every unordered Group element, normalize the proposed tuple, omit route owners, trim Pending values, and key EditionEvidence only by user plus Edition id.
#[test]
fn canonical_version_one_key_matrix_is_exact_and_shared() {
    let mut title = ilr::title_parts_from_provider(
        "  U5 Canonical Book (Large Print Edition)  ".to_string(),
        None,
    )
    .unwrap();
    title.provenance = ilr::EvidenceProvenance::Provider(ilr::IdentityProvider::OpenLibrary);
    let route = |provider, kind, value: &str, owner| ilr::WorkRoute {
        id: 99,
        user_id: 41,
        owner,
        resolved_work_id: 7,
        provider,
        kind,
        provider_scoped_id: value.to_string(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    };
    let route_a = route(
        ilr::IdentityProvider::OpenLibrary,
        ilr::RouteKind::OpenLibraryWork,
        "OL-U5-CANONICAL-W",
        ilr::RouteOwner::Work(7),
    );
    let route_b = route(
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsBookEdition,
        "9555601",
        ilr::RouteOwner::Edition(88),
    );
    let choice_a = livrarr_domain::services::MergeFieldChoiceEntry {
        field: livrarr_domain::services::MergeableField::SeriesName,
        choice: livrarr_domain::services::MergeFieldChoice::KeepSurvivor,
    };
    let choice_b = livrarr_domain::services::MergeFieldChoiceEntry {
        field: livrarr_domain::services::MergeableField::SeriesPosition,
        choice: livrarr_domain::services::MergeFieldChoice::TakeLoser,
    };
    let group = |work_ids, routes, choices| ilr::SettlementReviewCard::GroupIdentity {
        work_ids,
        proposed_identity: Some(ilr::WorkIdentityEvidence {
            title: title.clone(),
            primary_author_id: 73,
            routes,
        }),
        merge_choices: choices,
    };
    let left = group(
        vec![9, 7, 9],
        vec![route_b.clone(), route_a.clone(), route_a.clone()],
        vec![choice_b, choice_a, choice_a],
    );
    let mut route_a_other_owner = route_a.clone();
    route_a_other_owner.owner = ilr::RouteOwner::Edition(777);
    route_a_other_owner.id = 1234;
    route_a_other_owner.resolved_work_id = 777;
    route_a_other_owner.provenance =
        ilr::RouteProvenance::Provider(ilr::IdentityProvider::Goodreads);
    route_a_other_owner.user_confirmed = false;
    route_a_other_owner.observed_at = Utc::now() + chrono::Duration::days(1);
    let mut right = group(
        vec![7, 9],
        vec![route_a_other_owner, route_b],
        vec![choice_a, choice_b],
    );
    let ilr::SettlementReviewCard::GroupIdentity {
        proposed_identity: Some(right_proposed),
        ..
    } = &mut right
    else {
        unreachable!()
    };
    right_proposed.title.main = "presentation-only main".to_string();
    right_proposed.title.subtitle = Some("presentation-only subtitle".to_string());
    right_proposed.title.volume = Some("presentation-only volume".to_string());
    right_proposed.title.provenance = ilr::EvidenceProvenance::Migrated;
    let key = |user, card: &ilr::SettlementReviewCard, fallback| {
        ilr::ReviewDismissalKeyV1::from_card(user, card, fallback)
            .unwrap()
            .canonical_json()
            .unwrap()
    };
    assert_eq!(key(41, &left, Some(7)), key(41, &right, Some(7)));
    assert_ne!(key(41, &left, Some(7)), key(42, &left, Some(7)));
    let changed_cohort = group(vec![7, 10], vec![route_a.clone()], vec![choice_a, choice_b]);
    assert_ne!(key(41, &left, Some(7)), key(41, &changed_cohort, Some(7)));
    let mut changed_title = left.clone();
    let ilr::SettlementReviewCard::GroupIdentity {
        proposed_identity: Some(proposed),
        ..
    } = &mut changed_title
    else {
        unreachable!()
    };
    proposed.title.normalized_subtitle = "different normalized subtitle".to_string();
    assert_ne!(key(41, &left, Some(7)), key(41, &changed_title, Some(7)));
    let mut changed_author = left.clone();
    let ilr::SettlementReviewCard::GroupIdentity {
        proposed_identity: Some(proposed),
        ..
    } = &mut changed_author
    else {
        unreachable!()
    };
    proposed.primary_author_id += 1;
    assert_ne!(key(41, &left, Some(7)), key(41, &changed_author, Some(7)));
    let mut changed_routes = left.clone();
    let ilr::SettlementReviewCard::GroupIdentity {
        proposed_identity: Some(proposed),
        ..
    } = &mut changed_routes
    else {
        unreachable!()
    };
    proposed.routes.remove(0);
    assert_ne!(key(41, &left, Some(7)), key(41, &changed_routes, Some(7)));
    let mut changed_choices = left.clone();
    let ilr::SettlementReviewCard::GroupIdentity { merge_choices, .. } = &mut changed_choices
    else {
        unreachable!()
    };
    merge_choices.remove(0);
    assert_ne!(key(41, &left, Some(7)), key(41, &changed_choices, Some(7)));

    let heal_a = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![9, 7],
        proposed_identity: None,
        merge_choices: Vec::new(),
    };
    let heal_b = ilr::SettlementReviewCard::GroupIdentity {
        work_ids: vec![7, 9],
        proposed_identity: None,
        merge_choices: Vec::new(),
    };
    assert_eq!(key(41, &heal_a, Some(7)), key(41, &heal_b, Some(9)));
    assert_ne!(key(41, &heal_a, Some(7)), key(41, &left, Some(7)));

    let pending_a = pending_card(
        pending_candidate(
            7,
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            " OL-U5-PENDING-CANONICAL-W ",
            ilr::RouteOwner::Work(7),
        ),
        7,
    );
    let pending_b = pending_card(
        pending_candidate(
            7,
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            "OL-U5-PENDING-CANONICAL-W",
            ilr::RouteOwner::Edition(999),
        ),
        7,
    );
    let pending_other_work = pending_card(
        pending_candidate(
            8,
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            "OL-U5-PENDING-CANONICAL-W",
            ilr::RouteOwner::Work(8),
        ),
        8,
    );
    assert_eq!(key(41, &pending_a, Some(7)), key(41, &pending_b, Some(7)));
    assert_ne!(
        key(41, &pending_a, Some(7)),
        key(41, &pending_other_work, Some(8))
    );

    let edition_a = ilr::SettlementReviewCard::EditionEvidence {
        edition_id: 501,
        evidence_ids: vec![3, 2, 1],
    };
    let edition_b = ilr::SettlementReviewCard::EditionEvidence {
        edition_id: 501,
        evidence_ids: vec![99],
    };
    let edition_changed = ilr::SettlementReviewCard::EditionEvidence {
        edition_id: 502,
        evidence_ids: vec![3, 2, 1],
    };
    assert_eq!(key(41, &edition_a, Some(7)), key(41, &edition_b, Some(99)));
    assert_ne!(
        key(41, &edition_a, Some(7)),
        key(41, &edition_changed, Some(7))
    );
}

fn braced_item<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker}"));
    let open = start + source[start..].find('{').expect("opening brace");
    let mut depth = 0_i64;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated {marker}")
}

// RED-UNTIL-U5: today migration 086, the shared versioned key type, Dismiss/continuation revocation hooks, and marker-gated startup pass do not exist; U7's helper contains a deliberately unreachable suppression error arm.
#[test]
fn one_shared_key_and_one_helper_own_u5_while_cutover_and_wave_b_stay_excluded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let migration = std::fs::read_to_string(
        root.join("crates/livrarr-db/migrations/086_identity_review_dismissals.sql"),
    )
    .expect("U5 is a new migration, never an edit to an applied migration");
    for fragment in [
        "CREATE TABLE identity_review_dismissals",
        "key_version",
        "canonical_key",
        "dismissed_at",
        "source_card_id",
        "revoked_at",
        "revoke_reason",
        "UNIQUE",
        "CREATE TABLE identity_review_dismissal_works",
        "work_id",
        "CREATE INDEX",
    ] {
        assert!(
            migration.contains(fragment),
            "migration 086 missing {fragment}"
        );
    }
    assert!(!migration.contains("FOREIGN KEY(source_card_id)"));

    let domain =
        std::fs::read_to_string(root.join("crates/livrarr-domain/src/identity_layer/dismissal.rs"))
            .expect("read U5 domain key source");
    assert_eq!(domain.matches("pub enum ReviewDismissalKeyV1").count(), 1);
    assert!(domain.contains("pub fn canonical_json"));
    assert!(
        !domain.contains("work_identity_conflicts"),
        "wave-B legacy store stays excluded"
    );

    let repository = std::fs::read_to_string(root.join("crates/livrarr-db/src/identity_layer.rs"))
        .expect("read identity repository");
    let helper = braced_item(
        &repository,
        "async fn mint_reuse_or_suppress_review_card_in_tx(",
    );
    assert_eq!(
        repository
            .matches("async fn mint_reuse_or_suppress_review_card_in_tx(")
            .count(),
        1
    );
    assert!(helper.contains("ReviewDismissalKeyV1"));
    assert!(helper.contains("SuppressedByDismissal"));
    assert!(!helper.contains("suppression is not available in this unit"));
    let dismiss = braced_item(&repository, "async fn dismiss_pending_review(");
    assert!(dismiss.contains("ReviewDismissalKeyV1"));
    assert!(dismiss.contains("identity_review_dismissals"));
    assert!(dismiss.contains("identity_review_dismissal_works"));
    let revoker = braced_item(&repository, "async fn revoke_identity_review_dismissals(");
    assert!(revoker.contains("identity_review_dismissal_works"));
    let continuation = braced_item(&repository, "async fn commit_review_continuation(");
    assert!(continuation.contains("revoke_identity_review_dismissals"));

    let edit = std::fs::read_to_string(root.join("crates/livrarr-db/src/sqlite_work_identity.rs"))
        .expect("read certified-edit transaction");
    assert!(braced_item(&edit, "async fn apply_identity_edit_in_tx(")
        .contains("revoke_identity_review_dismissals"));
    let pool = std::fs::read_to_string(root.join("crates/livrarr-db/src/pool.rs"))
        .expect("read startup passes");
    assert!(pool.contains("pub async fn adopt_identity_review_dismissals("));
    assert!(pool.contains(ADOPTION_MARKER));
    assert!(pool.contains("identity_review_dismissal_works"));
    let server = std::fs::read_to_string(root.join("crates/livrarr-server/src/main.rs"))
        .expect("read startup ordering");
    let startup = braced_item(&server, "async fn init_database(");
    let authority_ready = startup
        .find("ensure_identity_authority_ready_before_serve")
        .unwrap();
    let adopt = startup.find("adopt_identity_review_dismissals").unwrap();
    let title_heal = startup.find("heal_identity_title_policy").unwrap();
    assert!(
        authority_ready < adopt && adopt < title_heal,
        "adoption runs after activation and before producer startup heals"
    );

    let cutover = braced_item(&repository, "async fn stage_legacy_identity_rows(");
    assert!(!cutover.contains("mint_reuse_or_suppress_review_card_in_tx"));
    assert!(!cutover.contains("identity_review_dismissals"));
    assert!(!cutover.contains("ReviewDismissalKeyV1"));
    let frontend = std::fs::read_to_string(root.join("frontend/src/types/api.ts"))
        .expect("read unchanged public wire types");
    assert!(!frontend.contains("ReviewDismissalKeyV1"));
    assert!(!frontend.contains("keyVersion"));
    assert!(!frontend.contains("revokedAt"));
}
