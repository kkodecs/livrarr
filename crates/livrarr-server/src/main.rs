#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use livrarr_db::{DownloadClientDb, IndexerDb};
use livrarr_server::config::{AppConfig, LogFormat, LogLevel};
use livrarr_server::router::build_router;
use livrarr_server::state::AppState;

/// Validate an LLM endpoint URL at startup (best-effort, non-fatal).
fn validate_llm_endpoint_startup(endpoint: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported scheme: {other}")),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL contains embedded credentials".into());
    }
    if let Some(host) = parsed.host_str() {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if livrarr_http::ssrf::is_private_ip(ip) {
                return Err("URL points to a private IP address".into());
            }
        }
    }
    Ok(())
}

/// Livrarr — self-hosted ebook and audiobook library manager.
#[derive(Parser)]
#[command(name = "livrarr", version)]
struct Cli {
    /// Data directory (config, database, covers).
    #[arg(long, default_value = "./data")]
    data: PathBuf,

    /// UI assets directory. Defaults to {data}/ui when not set.
    #[arg(long)]
    ui_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    let cli = Cli::parse();
    let data_dir = cli.data;
    let ui_dir = cli.ui_dir.unwrap_or_else(|| data_dir.join("ui"));

    // Step 1: Ensure data directory exists.
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!(
            "Failed to create data directory {}: {e}",
            data_dir.display()
        );
        std::process::exit(1);
    }

    // Step 2: Read config.toml.
    let config = match load_config(&data_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {e}");
            std::process::exit(1);
        }
    };

    // Step 3: Initialize tracing.
    let log_buffer = Arc::new(livrarr_server::state::LogBuffer::new());
    let (log_level_handle, log_surface) = init_tracing(&config.log, log_buffer.clone(), &data_dir);

    info!("Livrarr starting — data directory: {}", data_dir.display());

    // Step 4: Permission check — verify data dir is writable.
    if let Err(e) = livrarr_db::pool::check_data_dir_permissions(&data_dir) {
        error!("{e}");
        std::process::exit(1);
    }

    // Step 5: PID lock — ensure single instance.
    if let Err(e) = livrarr_db::pool::acquire_pid_lock(&data_dir) {
        error!("{e}");
        std::process::exit(1);
    }

    // Step 6: Connect to SQLite.
    let pool = match livrarr_db::pool::create_sqlite_pool(&data_dir).await {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to connect to SQLite: {e}");
            livrarr_db::pool::release_pid_lock(&data_dir);
            std::process::exit(1);
        }
    };

    // Step 7: Pre-migration backup (only if DB file already exists).
    let db_path = data_dir.join("livrarr.db");
    let db_exists = tokio::fs::try_exists(&db_path).await.unwrap_or(false);
    if db_exists {
        match livrarr_db::pool::create_backup(&pool, &data_dir).await {
            Ok(_) => {}
            Err(e) => {
                error!("Pre-migration backup failed: {e}");
                livrarr_db::pool::release_pid_lock(&data_dir);
                std::process::exit(1);
            }
        }
    }

    // Step 8: Run migrations.
    if let Err(e) = livrarr_db::pool::run_migrations(&pool).await {
        error!("Migration failed: {e}");
        livrarr_db::pool::release_pid_lock(&data_dir);
        std::process::exit(1);
    }
    info!("Database migrations complete");

    // Step 9: Version gate — verify DB compatibility.
    if let Err(e) = livrarr_db::pool::check_version_gate(&pool).await {
        error!("{e}");
        livrarr_db::pool::release_pid_lock(&data_dir);
        std::process::exit(1);
    }

    // Step 9b: Backfill normalized identity columns and create UNIQUE index.
    // Migration 038 adds columns with `__UNMIGRATED__` defaults; this hook
    // computes real values, resolves duplicates, and creates the index.
    if let Err(e) = livrarr_db::pool::backfill_normalized_identity(&pool).await {
        error!("normalized identity backfill failed: {e}");
        livrarr_db::pool::release_pid_lock(&data_dir);
        std::process::exit(1);
    }

    // Step 9c: Recompute works.normalized_title/normalized_author via the
    // identity_matching authority's identity_key recipe (REQ-014),
    // superseding the retired normalize_for_matching. Idempotent — migration
    // 069 seeds the generation marker this checks.
    if let Err(e) = livrarr_db::pool::backfill_identity_key_recompute(&pool).await {
        error!("identity-key recompute failed: {e}");
        livrarr_db::pool::release_pid_lock(&data_dir);
        std::process::exit(1);
    }

    // Step 10: Clean up old backups (keep 3).
    {
        let data_dir_clone = data_dir.clone();
        tokio::task::spawn_blocking(move || {
            livrarr_db::pool::cleanup_old_backups(&data_dir_clone, 3);
        })
        .await
        .ok();
    }

    // Construct AppState.
    let db = livrarr_db::sqlite::SqliteDb::new(pool);
    let auth_service = Arc::new(livrarr_server::auth_service::ServerAuthService::new(
        db.clone(),
        livrarr_server::auth_crypto::RealAuthCrypto,
    ));
    let ua = livrarr_http::livrarr_user_agent();
    let http_client = livrarr_http::HttpClient::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(&ua)
        .build()
        .expect("failed to build HTTP client");
    let http_client_safe = livrarr_http::HttpClient::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(&ua)
        .ssrf_safe(true)
        .build()
        .expect("failed to build SSRF-safe HTTP client");
    let http_fetcher =
        livrarr_http::fetcher::HttpFetcherImpl::new().expect("failed to build HTTP fetcher");
    let job_runner = livrarr_server::jobs::JobRunner::new();

    // Provider call-record sink (REQ-001): bounded fire-and-forget channel +
    // batching writer. Shares the job cancellation token; the handle is
    // awaited after job shutdown so the final drain completes.
    let (call_sink, call_sink_handle) =
        livrarr_server::call_sink::spawn_call_sink(db.clone(), job_runner.cancel_token());
    let call_sink: Arc<dyn livrarr_domain::services::ProviderCallSink> = Arc::new(call_sink);

    // Phase 1.5 plumbing: build the live DefaultProviderQueue + EnrichmentServiceImpl
    // from a startup-time snapshot of MetadataConfig. Live config changes (token
    // added, URL changed) require a server restart for now — runtime reload comes
    // alongside the orchestration cutover.
    // LiveMetadataConfig — all credential-dependent components hold a clone
    // of this and read fresh per call. The update_metadata_config handler
    // calls .replace() after a DB write so the new credentials are live on
    // the next enrichment without restart.
    let live_metadata_config = {
        use livrarr_db::ConfigDb;
        let initial = db.get_metadata_config().await.unwrap_or_else(|e| {
            warn!("Failed to read metadata config at startup ({e}); using defaults");
            livrarr_db::MetadataConfig {
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
            }
        });
        livrarr_external_data::live_config::LiveMetadataConfig::new(initial)
    };

    // Warn at startup if the configured LLM endpoint is invalid (but don't fail).
    {
        let cfg = live_metadata_config.snapshot();
        if let Some(ref endpoint) = cfg.llm_endpoint {
            if !endpoint.is_empty() {
                if let Err(reason) = validate_llm_endpoint_startup(endpoint) {
                    warn!("LLM endpoint validation: {reason} — LLM features may not work");
                }
            }
        }
    }

    // Shared payload transport cache (REQ-014/015): the identity resolver seeds it
    // during discovery; EnrichmentServiceImpl::enrich_work consumes it for a
    // candidate_id hit (zero-network reuse through the one road). The SAME Arc is
    // wired into both the enrichment service (below) and the identity resolver.
    let transport_cache = Arc::new(livrarr_external_data::transport_cache::TransportCache::new(
        std::time::Duration::from_secs(300),
    ));

    let (provider_queue, enrichment_service) = {
        use livrarr_domain::MetadataProvider as P;
        use livrarr_metadata as m;

        let cfg_snapshot = live_metadata_config.snapshot();

        let queue_cfg = |provider| m::ProviderQueueConfig {
            provider,
            max_attempts: 5,
            max_suppressed_passes: 3,
            max_suppression_window_secs: 3600,
        };

        let mut builder = m::DefaultProviderQueueBuilder::new();

        // Audnexus — always available. URL is captured at startup; if you
        // want a custom audnexus_url to take effect live too, that's a
        // small follow-up (same LiveMetadataConfig pattern).
        builder = builder.add_provider(
            P::Audnexus,
            livrarr_external_data::ProviderClient::Audnexus(
                livrarr_external_data::AudnexusClient::new(
                    http_fetcher.clone(),
                    cfg_snapshot.audnexus_url.clone(),
                ),
            )
            .with_call_sink(call_sink.clone()),
            queue_cfg(P::Audnexus),
        );

        // OpenLibrary — always available, no credentials needed.
        builder = builder.add_provider(
            P::OpenLibrary,
            livrarr_external_data::ProviderClient::OpenLibrary(
                livrarr_external_data::OpenLibraryClient::new(http_fetcher.clone()),
            )
            .with_call_sink(call_sink.clone()),
            queue_cfg(P::OpenLibrary),
        );

        // Hardcover — always registered. The client itself reads the live
        // config per-fetch; if `hardcover_enabled=false` or the token is
        // empty, it returns NotFound without a network call. Enabling HC
        // via the UI takes effect on the next enrichment.
        builder = builder.add_provider(
            P::Hardcover,
            livrarr_external_data::ProviderClient::Hardcover(
                livrarr_external_data::HardcoverClient::new(
                    http_fetcher.clone(),
                    live_metadata_config.clone(),
                ),
            )
            .with_call_sink(call_sink.clone()),
            queue_cfg(P::Hardcover),
        );

        // Goodreads — always registered. The LLM extraction fallback for
        // foreign-language pages reads live config per-fetch.
        let gr_client = livrarr_external_data::GoodreadsClient::production(
            http_fetcher.clone(),
            http_client.clone(),
        )
        .with_live_config(live_metadata_config.clone());
        builder = builder.add_provider(
            P::Goodreads,
            livrarr_external_data::ProviderClient::Goodreads(gr_client)
                .with_call_sink(call_sink.clone()),
            queue_cfg(P::Goodreads),
        );

        // Google Books — always registered. Reads API key from live config per-fetch.
        builder = builder.add_provider(
            P::GoogleBooks,
            livrarr_external_data::ProviderClient::GoogleBooks(
                livrarr_external_data::GoogleBooksClient::new(
                    http_fetcher.clone(),
                    live_metadata_config.clone(),
                ),
            )
            .with_call_sink(call_sink.clone()),
            queue_cfg(P::GoogleBooks),
        );

        // Audible — always registered. Unauthenticated API, no config needed.
        builder = builder.add_provider(
            P::Audible,
            livrarr_external_data::ProviderClient::Audible(
                livrarr_external_data::audible::AudibleCatalogClient::new(
                    http_fetcher.clone(),
                    5 * 60,
                ),
            )
            .with_call_sink(call_sink.clone()),
            queue_cfg(P::Audible),
        );

        builder = builder.with_applicability_rule(Arc::new(|provider, work| {
            if matches!(
                livrarr_external_data::language::provider_priority(work.language.as_deref()),
                livrarr_external_data::language::ProviderPriority::English
            ) {
                return !matches!(provider, P::GoogleBooks);
            }
            matches!(
                provider,
                P::Goodreads | P::Audnexus | P::GoogleBooks | P::Audible
            )
        }));

        // Pipeline-level skip records (no anchor / policy) flow through the
        // queue's own sink seam (REQ-001).
        builder = builder.with_call_sink(call_sink.clone());

        let db_arc = Arc::new(db.clone());
        let queue = Arc::new(builder.build(db_arc.clone()));

        // Merge engine: purely deterministic (REQ-005) — the per-merge priority
        // model comes from `MergeInput`; no LLM is consulted anywhere in merge.
        let merge_engine = Arc::new(m::DefaultMergeEngine::new(m::PriorityModel::english()));

        let llm_configured = live_metadata_config.snapshot().llm_enabled;
        let service = Arc::new(
            m::EnrichmentServiceImpl::new(db_arc, queue.clone(), merge_engine, llm_configured)
                .with_transport_cache(transport_cache.clone())
                .with_call_sink(call_sink.clone()),
        );
        (queue, service)
    };

    let svc_db = db.clone();
    let svc_enrichment = enrichment_service.clone();
    let import_semaphore = Arc::new(tokio::sync::Semaphore::new(2));
    let data_dir_arc = Arc::new(data_dir.clone());
    let cover_proxy_cache = Arc::new(livrarr_server::infra::cover_cache::CoverProxyCache::new());
    let rss_last_run = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let rss_sync_running = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let http_client_for_scan = http_client.clone();
    let settings_service_arc = Arc::new(
        livrarr_server::services::settings_service::LiveSettingsService::new(svc_db.clone()),
    );
    let import_io_arc = Arc::new(livrarr_server::import_io_service::ImportIoServiceImpl::new(
        svc_db.clone(),
    ));
    let manual_import_scans_shared = Arc::new(dashmap::DashMap::new());
    let import_workflow_arc = Arc::new(livrarr_library::import_workflow::ImportWorkflowImpl::new(
        svc_db.clone(),
        import_semaphore.clone(),
        data_dir_arc.clone(),
        Arc::new(livrarr_server::chapter_extractor::ChapterExtractorImpl),
    ));
    let tag_service_arc = Arc::new(livrarr_server::tag_service::LiveTagService::new(
        import_io_arc.clone(),
        data_dir_arc.clone(),
    ));
    let import_svc_arc = Arc::new(livrarr_server::import_service::LiveImportService::new(
        import_io_arc.clone(),
        import_workflow_arc.clone(),
        tag_service_arc.clone(),
        settings_service_arc.clone(),
        http_client_safe.clone(),
    ));
    // Build trusted origins from configured indexers + download clients.
    let trusted_origins = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    {
        let mut urls = Vec::new();
        if let Ok(indexers) = svc_db.list_indexers().await {
            urls.extend(indexers.iter().map(|i| i.url.clone()));
        }
        if let Ok(clients) = svc_db.list_download_clients().await {
            for c in &clients {
                let scheme = if c.use_ssl { "https" } else { "http" };
                urls.push(format!("{}://{}:{}", scheme, c.host, c.port));
            }
        }
        trusted_origins.rebuild(&urls);
    }
    let trusted_origins_arc = trusted_origins.clone();

    // Pre-construct shared Arcs for fields referenced by both AppState and readarr_import_wf.
    let readarr_import_service_arc = Arc::new(
        livrarr_server::readarr_import_service::LiveReadarrImportService::new(svc_db.clone()),
    );
    let readarr_import_progress_arc = Arc::new(tokio::sync::Mutex::new(
        livrarr_server::readarr_import_service::ReadarrImportProgress::default(),
    ));
    // Readarr URL is admin-configured trusted infrastructure (matches the
    // alpha4 trusted-infrastructure pattern: download clients, indexers,
    // Readarr import all use the unrestricted client because the admin
    // chose the endpoint).
    let http_client_for_readarr = http_client.clone();
    // Pre-construct WorkService Arc so series_query_service and readarr_import_wf can share it.
    // Shared provider-client map: one set of per-provider clients reused by the
    // cover service and the identity resolver's multi-provider fan-out.
    let provider_clients = {
        use livrarr_domain::MetadataProvider as P;
        let mut clients = std::collections::HashMap::new();
        clients.insert(
            P::Audnexus,
            livrarr_external_data::ProviderClient::Audnexus(
                livrarr_external_data::AudnexusClient::new(
                    http_fetcher.clone(),
                    live_metadata_config.snapshot().audnexus_url.clone(),
                ),
            ),
        );
        clients.insert(
            P::OpenLibrary,
            livrarr_external_data::ProviderClient::OpenLibrary(
                livrarr_external_data::OpenLibraryClient::new(http_fetcher.clone()),
            ),
        );
        clients.insert(
            P::Hardcover,
            livrarr_external_data::ProviderClient::Hardcover(
                livrarr_external_data::HardcoverClient::new(
                    http_fetcher.clone(),
                    live_metadata_config.clone(),
                ),
            ),
        );
        clients.insert(
            P::Goodreads,
            livrarr_external_data::ProviderClient::Goodreads(
                livrarr_external_data::GoodreadsClient::production(
                    http_fetcher.clone(),
                    http_client.clone(),
                )
                .with_live_config(live_metadata_config.clone()),
            ),
        );
        clients.insert(
            P::GoogleBooks,
            livrarr_external_data::ProviderClient::GoogleBooks(
                livrarr_external_data::GoogleBooksClient::new(
                    http_fetcher.clone(),
                    live_metadata_config.clone(),
                ),
            ),
        );
        clients.insert(
            P::Audible,
            livrarr_external_data::ProviderClient::Audible(
                livrarr_external_data::audible::AudibleCatalogClient::new(
                    http_fetcher.clone(),
                    5 * 60,
                ),
            ),
        );
        // Thread the call-record sink through every client (REQ-001).
        clients
            .into_iter()
            .map(|(p, c)| (p, c.with_call_sink(call_sink.clone())))
            .collect::<std::collections::HashMap<_, _>>()
    };
    // Multi-provider identity resolver. A single instance is shared between the
    // Add-Work handler's resolve() (AppState.identity_resolver) and the search
    // box's lookup_filtered (WorkService), so both route through the federated
    // fan-out and share one payload transport cache.
    let identity_resolver_arc = Arc::new(
        livrarr_metadata::english_identity_resolver::LiveEnglishIdentityResolver {
            clients: provider_clients.clone(),
            cache: transport_cache.clone(),
            config: {
                let cfg = live_metadata_config.snapshot();
                let gb_key_present = cfg
                    .google_books_api_key
                    .as_deref()
                    .is_some_and(|s| !s.is_empty());
                // Read once at startup, like every other credential/flag above
                // (REQ-007/REQ-013) — a settings change here takes effect on
                // the next restart, consistent with this whole block.
                let default_language_source = {
                    use livrarr_db::ConfigDb;
                    db.get_default_language().await.unwrap_or_else(|e| {
                        warn!("Failed to read default language at startup ({e}); using \"en\"");
                        "en".to_string()
                    })
                };
                livrarr_metadata::english_identity_resolver::ResolverConfig {
                    gb_key_present,
                    default_language_source,
                    ..Default::default()
                }
            },
        },
    );
    let work_service_arc: Arc<livrarr_server::state::LiveWorkService> = {
        let ew = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
            svc_enrichment.clone(),
            svc_db.clone(),
        );
        let ws_merge_engine =
            livrarr_metadata::DefaultMergeEngine::new(livrarr_metadata::PriorityModel::english());
        Arc::new(
            livrarr_metadata::work_service::WorkServiceImpl::new_with_all(
                svc_db.clone(),
                ew,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for work service"),
                http_client.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for work service"),
                ),
                data_dir.clone(),
                ws_merge_engine,
                tag_service_arc.clone(),
            )
            .with_resolver(identity_resolver_arc.clone()),
        )
    };
    // Cover service + HMAC key
    let hmac_key = livrarr_server::cover_service::generate_hmac_key();
    let cover_service = {
        let clients = provider_clients.clone();
        Arc::new(livrarr_server::cover_service::LiveCoverService::new(
            db.clone(),
            http_fetcher.clone(),
            clients,
            hmac_key.clone(),
            data_dir_arc.clone(),
        ))
    };
    let http_client_for_services = http_client.clone();
    let state = AppState {
        db,
        auth_service,
        http_client: http_client.clone(),
        http_client_safe,
        http_fetcher: http_fetcher.clone(),
        config: Arc::new(config.clone()),
        data_dir: data_dir_arc.clone(),
        startup_time: chrono::Utc::now(),
        job_runner: Some(job_runner.clone()),
        cover_proxy_cache: cover_proxy_cache.clone(),
        live_metadata_config: live_metadata_config.clone(),
        log_buffer: log_buffer.clone(),
        log_level_handle: log_level_handle.clone(),
        import_semaphore: import_semaphore.clone(),
        grab_search_cache: Arc::new(livrarr_server::state::GrabSearchCache::new()),
        rss_last_run: rss_last_run.clone(),
        rss_sync_running: rss_sync_running.clone(),
        readarr_import_progress: readarr_import_progress_arc.clone(),
        manual_import_scans: manual_import_scans_shared.clone(),
        provider_queue,
        enrichment_service: enrichment_service.clone(),

        // --- Service layer (Phase 4) ---
        author_service: Arc::new(livrarr_metadata::author_service::AuthorServiceImpl::new(
            svc_db.clone(),
            livrarr_http::fetcher::HttpFetcherImpl::new()
                .expect("HttpFetcherImpl construction for author service"),
            livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                live_metadata_config.clone(),
                livrarr_http::HttpClient::builder()
                    .build()
                    .expect("LLM HttpClient"),
            ),
        )),
        series_service: Arc::new(livrarr_metadata::series_service::SeriesServiceImpl::new(
            svc_db.clone(),
        )),
        series_query_service: Arc::new(
            livrarr_metadata::series_query_service::SeriesQueryServiceImpl::new(
                svc_db.clone(),
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for series query service"),
                work_service_arc.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for series query"),
                ),
            ),
        ),
        work_service: work_service_arc.clone(),
        grab_service: Arc::new(livrarr_download::grab_service::GrabServiceImpl::new(
            svc_db.clone(),
        )),
        release_service: Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
            svc_db.clone(),
            livrarr_http::fetcher::HttpFetcherImpl::new().expect("HttpFetcherImpl construction"),
            trusted_origins_arc.clone(),
        )),
        file_service: Arc::new(livrarr_library::file_service::FileServiceImpl::new(
            svc_db.clone(),
        )),
        chapter_service: Arc::new(livrarr_library::chapter_service::ChapterServiceImpl::new(
            svc_db.clone(),
        )),
        bookmark_service: Arc::new(livrarr_library::bookmark_service::BookmarkServiceImpl::new(
            svc_db.clone(),
        )),
        cross_format_service: Arc::new(
            livrarr_library::cross_format_service::CrossFormatServiceImpl::new(
                svc_db.clone(),
                livrarr_library::file_service::FileServiceImpl::new(svc_db.clone()),
            ),
        ),
        import_workflow: import_workflow_arc.clone(),
        rss_sync_workflow: {
            let rs = Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
                svc_db.clone(),
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for rss sync"),
                trusted_origins_arc.clone(),
            ));
            Arc::new(
                livrarr_metadata::rss_sync_workflow::RssSyncWorkflowImpl::new(
                    Arc::new(svc_db.clone()),
                    Arc::new(
                        livrarr_http::fetcher::HttpFetcherImpl::new()
                            .expect("HttpFetcherImpl construction for rss sync fetch"),
                    ),
                    rs,
                ),
            )
        },
        list_service: {
            let ew = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            );
            let ws = livrarr_metadata::work_service::WorkServiceImpl::new_with_all(
                svc_db.clone(),
                ew,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for list work service"),
                http_client_for_services.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for list service"),
                ),
                data_dir.clone(),
                livrarr_metadata::DefaultMergeEngine::new(
                    livrarr_metadata::PriorityModel::english(),
                ),
                tag_service_arc.clone(),
            );
            Arc::new(livrarr_metadata::list_service::ListServiceImpl::new(
                svc_db.clone(),
                ws,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for list service"),
                livrarr_metadata::list_service::NoOpBibliographyTrigger,
            ))
        },
        identity_conflict_service: Arc::new(
            livrarr_server::services::identity_conflict_service::LiveIdentityConflictService::new(
                svc_db.clone(),
            ),
        ),
        identity_resolver: identity_resolver_arc,
        enrichment_workflow: Arc::new(
            livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            ),
        ),
        author_monitor_workflow: {
            let ew = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                svc_enrichment.clone(),
                svc_db.clone(),
            );
            let ws = livrarr_metadata::work_service::WorkServiceImpl::new_with_all(
                svc_db.clone(),
                ew,
                livrarr_http::fetcher::HttpFetcherImpl::new()
                    .expect("HttpFetcherImpl construction for author monitor work service"),
                http_client_for_services.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    livrarr_http::HttpClient::builder()
                        .build()
                        .expect("LLM HttpClient for author monitor"),
                ),
                data_dir.clone(),
                livrarr_metadata::DefaultMergeEngine::new(
                    livrarr_metadata::PriorityModel::english(),
                ),
                tag_service_arc.clone(),
            );
            Arc::new(
                livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl::new(
                    Arc::new(svc_db.clone()),
                    Arc::new(ws),
                    Arc::new(
                        livrarr_http::fetcher::HttpFetcherImpl::new()
                            .expect("HttpFetcherImpl construction for author monitor"),
                    ),
                ),
            )
        },
        readarr_import_service: readarr_import_service_arc.clone(),
        settings_service: settings_service_arc.clone(),
        notification_service: Arc::new(
            livrarr_server::notification_service::NotificationServiceImpl::new(svc_db.clone()),
        ),
        history_service: Arc::new(livrarr_server::history_service::HistoryServiceImpl::new(
            svc_db.clone(),
        )),
        queue_service: Arc::new(livrarr_server::queue_service::QueueServiceImpl::new(
            svc_db.clone(),
            http_client_for_scan.clone(),
        )),
        import_io_service: import_io_arc.clone(),
        manual_import_db_service: Arc::new(
            livrarr_server::manual_import_service::ManualImportServiceImpl::new(svc_db.clone()),
        ),

        // --- Phase 5: infrastructure accessors (share Arcs with fields above) ---
        rss_sync_state: livrarr_server::state::RssSyncState {
            running: rss_sync_running.clone(),
            last_run: rss_last_run.clone(),
        },
        system_state: livrarr_server::state::SystemState {
            log_buffer: log_buffer.clone(),
            log_level_handle: log_level_handle.clone(),
        },
        provider_stats_service: Arc::new(livrarr_server::state::LiveProviderStatsService::new(
            svc_db.clone(),
        )),
        log_surface_accessor: livrarr_server::state::LogSurfaceAccessorImpl {
            log_dir: data_dir.join("logs"),
            init_error: log_surface.init_error.clone(),
        },
        live_metadata_config_accessor: livrarr_server::state::LiveMetadataConfigAccessorImpl(
            live_metadata_config.clone(),
        ),
        cover_proxy_cache_accessor: livrarr_server::state::CoverProxyCacheAccessorImpl(
            cover_proxy_cache.clone(),
        ),
        tag_service: tag_service_arc.clone(),
        email_svc: Arc::new(livrarr_server::email_service::LiveEmailService::new(
            settings_service_arc.clone(),
        )),
        import_svc: import_svc_arc,
        matching_svc: livrarr_server::matching_service::LiveMatchingService,
        manual_import_scan_svc:
            livrarr_server::manual_import_scan_service::LiveManualImportScanService {
                scans: manual_import_scans_shared.clone(),
            },
        readarr_import_wf: Arc::new(
            livrarr_server::readarr_import_workflow::LiveReadarrImportWorkflow::new(
                http_client_for_readarr,
                readarr_import_service_arc,
                readarr_import_progress_arc,
                data_dir_arc.clone(),
                work_service_arc.clone(),
                svc_db.clone(),
                import_workflow_arc.clone(),
            ),
        ),
        cover_service,
        preadd_cover_service: {
            use livrarr_domain::MetadataProvider as P;
            use livrarr_metadata as m;
            let mut preadd_clients = std::collections::HashMap::new();
            preadd_clients.insert(
                P::Hardcover,
                livrarr_external_data::ProviderClient::Hardcover(
                    livrarr_external_data::HardcoverClient::new(
                        http_fetcher.clone(),
                        live_metadata_config.clone(),
                    ),
                ),
            );
            preadd_clients.insert(
                P::OpenLibrary,
                livrarr_external_data::ProviderClient::OpenLibrary(
                    livrarr_external_data::OpenLibraryClient::new(http_fetcher.clone()),
                ),
            );
            preadd_clients.insert(
                P::Goodreads,
                livrarr_external_data::ProviderClient::Goodreads(
                    livrarr_external_data::GoodreadsClient::production(
                        http_fetcher.clone(),
                        http_client.clone(),
                    )
                    .with_live_config(live_metadata_config.clone()),
                ),
            );
            preadd_clients.insert(
                P::Audnexus,
                livrarr_external_data::ProviderClient::Audnexus(
                    livrarr_external_data::AudnexusClient::new(
                        http_fetcher.clone(),
                        live_metadata_config.snapshot().audnexus_url.clone(),
                    ),
                ),
            );
            preadd_clients.insert(
                P::Audible,
                livrarr_external_data::ProviderClient::Audible(
                    livrarr_external_data::audible::AudibleCatalogClient::new(
                        http_fetcher.clone(),
                        5 * 60,
                    ),
                ),
            );
            // Thread the call-record sink through every client (REQ-001).
            let preadd_clients = preadd_clients
                .into_iter()
                .map(|(p, c)| (p, c.with_call_sink(call_sink.clone())))
                .collect();
            Arc::new(m::preadd_cover_service::LivePreaddCoverService::new(
                preadd_clients,
            ))
        },
        hmac_key,
        trusted_origins_rebuilder: livrarr_server::state::TrustedOriginsRebuilderImpl(
            trusted_origins_arc.clone(),
        ),
    };

    // Step 11: Startup recovery — reset stale state from unclean shutdown (JOBS-003).
    livrarr_server::jobs::recover_interrupted_state(&state).await;

    // Pre-warm SQLite page cache so first request isn't slow.
    // Exception to "no SQL outside livrarr-db": these are throwaway startup
    // queries that touch hot pages. Not worth a trait method.
    let _ = sqlx::query("SELECT COUNT(*) FROM works")
        .fetch_one(state.db.pool())
        .await;
    let _ = sqlx::query("SELECT COUNT(*) FROM library_items")
        .fetch_one(state.db.pool())
        .await;

    // Step 12: Start background jobs (JOBS-001).
    job_runner.start(state.clone()).await;

    // Step 13: Build router.
    let app = build_router(state, ui_dir);

    // Step 14: Bind HTTP server.
    let addr = format!("{}:{}", config.server.bind_address, config.server.port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind to {addr}: {e}");
            std::process::exit(1);
        }
    };

    job_runner
        .spawn_startup_pass(
            "chapter_backfill",
            svc_db.clone(),
            livrarr_server::jobs::chapter_backfill::run_chapter_backfill(svc_db.clone()),
        )
        .await;
    // Covers startup sequence — ONE pass because the passes inside it are
    // order-dependent: the layout migration must settle the per-user
    // directory tree before recovery converges pending cover writes against
    // it, and the provenance backfill must only stamp rows recovery has
    // finished healing. The sequencing itself lives in
    // livrarr_metadata::cover_startup.
    job_runner
        .spawn_startup_pass(
            "cover_startup",
            svc_db.clone(),
            livrarr_server::jobs::cover_startup::run_cover_startup_passes(
                svc_db.clone(),
                data_dir.join("covers"),
            ),
        )
        .await;
    job_runner
        .spawn_startup_pass(
            "series_backfill",
            svc_db.clone(),
            livrarr_server::jobs::series_backfill::run_series_backfill(svc_db.clone()),
        )
        .await;

    info!("Listening on {addr}");

    // Step 15: Serve with graceful shutdown on SIGTERM/Ctrl+C.
    // Cancel background jobs immediately when signal fires (before HTTP drain).
    // Remove PID file early so a container restart doesn't deadlock on stale lock.
    let job_cancel = job_runner.cancel_token();
    let shutdown_data_dir = data_dir.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        info!("Cancelling background jobs");
        job_cancel.cancel();
        livrarr_db::pool::release_pid_lock(&shutdown_data_dir);
    })
    .await
    .unwrap_or_else(|e| {
        error!("Server error: {e}");
        std::process::exit(1);
    });

    // Await job completion (cancel already signalled above).
    job_runner.shutdown().await;

    // Drain the call-record sink (its writer shares the job cancel token).
    let _ = call_sink_handle.await;

    livrarr_db::pool::release_pid_lock(&data_dir);
    info!("Livrarr stopped");
}

fn load_config(data_dir: &std::path::Path) -> Result<AppConfig, String> {
    let config_path = data_dir.join("config.toml");

    let config: AppConfig = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("failed to read config.toml: {e}"))?;

        if raw.trim().is_empty() {
            AppConfig::default()
        } else {
            // Parse for unknown key warnings.
            if let Ok(val) = raw.parse::<toml::Value>() {
                livrarr_server::config::warn_unknown_keys(&val);
            }

            toml::from_str(&raw).map_err(|e| format!("failed to parse config.toml: {e}"))?
        }
    } else {
        AppConfig::default()
    };

    livrarr_server::config::validate_config(&config).map_err(|e| e.to_string())?;
    Ok(config)
}

fn init_tracing(
    log: &livrarr_server::config::LogConfig,
    log_buffer: Arc<livrarr_server::state::LogBuffer>,
    data_dir: &std::path::Path,
) -> (
    Arc<livrarr_server::state::LogLevelHandle>,
    livrarr_domain::LogSurfaceStatus,
) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    let level = match log.level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };

    let filter = EnvFilter::try_new(format!("livrarr={level},tower_http={level}"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(filter);

    // Console output — text or JSON per config.
    let use_json = log.format == LogFormat::Json;
    let fmt_layer: Box<dyn tracing_subscriber::Layer<_> + Send + Sync> = if use_json {
        Box::new(tracing_subscriber::fmt::layer().json().with_target(false))
    } else {
        Box::new(tracing_subscriber::fmt::layer().with_target(false))
    };

    // In-memory ring buffer for UI
    let buf_layer = LogBufferLayer(log_buffer);

    // File output: {data_dir}/logs/livrarr.log.<date> (daily roller).
    // REQ-003: a dir-creation or writability failure is captured and surfaced
    // (stderr now, status page later) — never swallowed; the file layer is
    // skipped so console + ring buffer keep working and the server boots.
    let log_dir = data_dir.join("logs");
    let surface = livrarr_server::log_surface::prepare_log_surface(&log_dir);
    let file_layer: Option<Box<dyn tracing_subscriber::Layer<_> + Send + Sync>> =
        if surface.init_error.is_none() {
            let file_appender = tracing_appender::rolling::daily(&log_dir, "livrarr.log");
            Some(if use_json {
                Box::new(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_target(false)
                        .with_ansi(false)
                        .with_writer(file_appender),
                )
            } else {
                Box::new(
                    tracing_subscriber::fmt::layer()
                        .with_target(false)
                        .with_ansi(false)
                        .with_writer(file_appender),
                )
            })
        } else {
            None
        };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(file_layer)
        .with(buf_layer)
        .init();

    (
        Arc::new(livrarr_server::state::LogLevelHandle::new(
            reload_handle,
            level,
        )),
        surface,
    )
}

/// Tracing layer that captures formatted log lines into a shared ring buffer.
struct LogBufferLayer(Arc<livrarr_server::state::LogBuffer>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogBufferLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);
        let line = format!(
            "{} {:>5} {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            meta.level(),
            message,
        );
        self.0.push(line);
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{:?}", value);
        } else if !self.0.is_empty() {
            let _ = write!(self.0, " {}={:?}", field.name(), value);
        } else {
            let _ = write!(self.0, "{}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{}", value);
        } else if !self.0.is_empty() {
            let _ = write!(self.0, " {}={}", field.name(), value);
        } else {
            let _ = write!(self.0, "{}={}", field.name(), value);
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
    info!("Shutdown signal received");
}
