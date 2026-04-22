use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;

use crate::rate_limit::SmartIpKeyExtractor;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::middleware::auth_middleware;
use crate::state::AppState;

/// Build the Axum router.
///
/// Satisfies: RUNTIME-SERVER-005, RUNTIME-COMPOSE-003, RUNTIME-COMPOSE-004
pub fn build_router(state: AppState, ui_dir: std::path::PathBuf) -> Router {
    // Parse trusted_proxies from config (empty = direct exposure, peer IP only).
    let trusted_proxies: Vec<crate::rate_limit::IpNet> = state
        .config
        .server
        .trusted_proxies
        .iter()
        .filter_map(|s| crate::rate_limit::IpNet::parse(s))
        .collect();
    let extractor = SmartIpKeyExtractor::new(trusted_proxies);

    // Rate limiter for login: 5 requests per 60 seconds per IP.
    let login_governor = GovernorConfigBuilder::default()
        .key_extractor(extractor.clone())
        .period(Duration::from_secs(12)) // 1 token per 12s = 5 per 60s
        .burst_size(5)
        .finish()
        .expect("login rate limiter config");

    // Global rate limiter: 100 requests per second sustained per peer IP.
    let global_governor = GovernorConfigBuilder::default()
        .key_extractor(extractor)
        .per_millisecond(10) // 1 token per 10ms = 100/sec sustained
        .burst_size(50)
        .finish()
        .expect("global rate limiter config");

    // Public API routes (no auth required).
    let public = Router::new()
        .route(
            "/setup/status",
            get(livrarr_handlers::setup::setup_status::<AppState>),
        )
        .route("/setup", post(livrarr_handlers::setup::setup::<AppState>))
        .route(
            "/auth/login",
            post(livrarr_handlers::auth::login::<AppState>)
                .layer(GovernorLayer::new(login_governor)),
        );

    // Protected API routes (auth middleware applied).
    let protected = Router::new()
        // Auth
        .route(
            "/auth/logout",
            post(livrarr_handlers::auth::logout::<AppState>),
        )
        .route("/auth/me", get(livrarr_handlers::auth::me::<AppState>))
        .route(
            "/auth/profile",
            put(livrarr_handlers::profile::update_profile::<AppState>),
        )
        .route(
            "/auth/apikey",
            post(livrarr_handlers::profile::regenerate_api_key::<AppState>),
        )
        // Users (admin)
        .route(
            "/user",
            get(livrarr_handlers::user::list::<AppState>)
                .post(livrarr_handlers::user::create::<AppState>),
        )
        .route(
            "/user/{id}",
            get(livrarr_handlers::user::get::<AppState>)
                .put(livrarr_handlers::user::update::<AppState>)
                .delete(livrarr_handlers::user::delete::<AppState>),
        )
        .route(
            "/user/{id}/apikey",
            post(livrarr_handlers::user::regenerate_user_api_key::<AppState>),
        )
        // Root folders
        .route(
            "/rootfolder",
            get(livrarr_handlers::root_folder::list::<AppState>)
                .post(livrarr_handlers::root_folder::create::<AppState>),
        )
        .route(
            "/rootfolder/{id}",
            delete(livrarr_handlers::root_folder::delete::<AppState>),
        )
        .route(
            "/rootfolder/{id}/scan",
            post(livrarr_handlers::root_folder::scan::<AppState>),
        )
        // Unmapped file scan (arbitrary path)
        .route(
            "/unmapped/scan",
            post(livrarr_handlers::root_folder::scan_path::<AppState>),
        )
        // Download clients
        .route(
            "/downloadclient",
            get(livrarr_handlers::download_client::list::<AppState>)
                .post(livrarr_handlers::download_client::create::<AppState>),
        )
        .route(
            "/downloadclient/test",
            post(livrarr_handlers::download_client::test::<AppState>),
        )
        .route(
            "/downloadclient/import/prowlarr",
            post(livrarr_handlers::download_client::import_from_prowlarr::<AppState>),
        )
        .route(
            "/downloadclient/{id}",
            get(livrarr_handlers::download_client::get::<AppState>)
                .put(livrarr_handlers::download_client::update::<AppState>)
                .delete(livrarr_handlers::download_client::delete::<AppState>),
        )
        .route(
            "/downloadclient/{id}/test",
            post(livrarr_handlers::download_client::test_saved::<AppState>),
        )
        // Remote path mappings
        .route(
            "/remotepathmapping",
            get(livrarr_handlers::remote_path_mapping::list::<AppState>)
                .post(livrarr_handlers::remote_path_mapping::create::<AppState>),
        )
        .route(
            "/remotepathmapping/{id}",
            get(livrarr_handlers::remote_path_mapping::get::<AppState>)
                .put(livrarr_handlers::remote_path_mapping::update::<AppState>)
                .delete(livrarr_handlers::remote_path_mapping::delete::<AppState>),
        )
        // Config
        .route(
            "/config/naming",
            get(livrarr_handlers::config::get_naming::<AppState>),
        )
        .route(
            "/config/mediamanagement",
            get(livrarr_handlers::config::get_media_management::<AppState>)
                .put(livrarr_handlers::config::update_media_management::<AppState>),
        )
        .route(
            "/config/prowlarr",
            get(livrarr_handlers::config::get_prowlarr::<AppState>)
                .put(livrarr_handlers::config::update_prowlarr::<AppState>),
        )
        .route(
            "/config/email",
            get(livrarr_handlers::config::get_email::<AppState>)
                .put(livrarr_handlers::config::update_email::<AppState>),
        )
        .route(
            "/config/email/test",
            post(livrarr_handlers::config::test_email::<AppState>),
        )
        .route(
            "/config/indexer",
            get(livrarr_handlers::config::get_indexer_config::<AppState>)
                .put(livrarr_handlers::config::update_indexer_config::<AppState>),
        )
        // RSS sync trigger
        .route(
            "/command/rss-sync",
            post(livrarr_handlers::config::trigger_rss_sync::<AppState>),
        )
        // Indexers (replaces /config/prowlarr — DEFERRED-001)
        .route(
            "/indexer",
            get(livrarr_handlers::indexer::list::<AppState>)
                .post(livrarr_handlers::indexer::create::<AppState>),
        )
        .route(
            "/indexer/test",
            post(livrarr_handlers::indexer::test::<AppState>),
        )
        .route(
            "/indexer/import/prowlarr",
            post(livrarr_handlers::indexer::import_from_prowlarr::<AppState>),
        )
        .route(
            "/indexer/{id}",
            get(livrarr_handlers::indexer::get::<AppState>)
                .put(livrarr_handlers::indexer::update::<AppState>)
                .delete(livrarr_handlers::indexer::delete::<AppState>),
        )
        .route(
            "/indexer/{id}/test",
            post(livrarr_handlers::indexer::test_saved::<AppState>),
        )
        .route(
            "/config/metadata",
            get(livrarr_handlers::config::get_metadata::<AppState>)
                .put(livrarr_handlers::config::update_metadata::<AppState>),
        )
        .route(
            "/config/metadata/test/hardcover",
            post(livrarr_handlers::config::test_hardcover::<AppState>),
        )
        .route(
            "/config/metadata/test/audnexus",
            post(livrarr_handlers::config::test_audnexus::<AppState>),
        )
        .route(
            "/config/metadata/test/llm",
            post(livrarr_handlers::config::test_llm::<AppState>),
        )
        // Works
        .route(
            "/work/lookup",
            get(livrarr_handlers::work::lookup::<AppState>),
        )
        .route(
            "/work/refresh",
            post(livrarr_handlers::work::refresh_all::<AppState>),
        )
        .route(
            "/work",
            get(livrarr_handlers::work::list::<AppState>)
                .post(livrarr_handlers::work::add::<AppState>),
        )
        .route(
            "/work/{id}",
            get(livrarr_handlers::work::get::<AppState>)
                .put(livrarr_handlers::work::update::<AppState>)
                .delete(livrarr_handlers::work::delete::<AppState>),
        )
        .route(
            "/work/{id}/cover",
            post(livrarr_handlers::work::upload_cover::<AppState>)
                .layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route(
            "/work/{id}/refresh",
            post(livrarr_handlers::work::refresh::<AppState>),
        )
        // Authors
        .route(
            "/author/lookup",
            get(livrarr_handlers::author::lookup::<AppState>),
        )
        .route(
            "/author/search",
            post(livrarr_handlers::work::author_search::<AppState>),
        )
        .route(
            "/author",
            get(livrarr_handlers::author::list::<AppState>)
                .post(livrarr_handlers::author::add::<AppState>),
        )
        .route(
            "/author/{id}",
            get(livrarr_handlers::author::get::<AppState>)
                .put(livrarr_handlers::author::update::<AppState>)
                .delete(livrarr_handlers::author::delete::<AppState>),
        )
        .route(
            "/author/{id}/bibliography",
            get(livrarr_handlers::author::bibliography::<AppState>),
        )
        .route(
            "/author/{id}/bibliography/refresh",
            post(livrarr_handlers::author::refresh_bibliography::<AppState>),
        )
        // Series
        .route(
            "/series",
            get(livrarr_handlers::series::list_all::<AppState>),
        )
        .route(
            "/author/{id}/resolve-gr",
            post(livrarr_handlers::series::resolve_gr::<AppState>),
        )
        .route(
            "/author/{id}/series",
            get(livrarr_handlers::series::list_series::<AppState>),
        )
        .route(
            "/author/{id}/series/refresh",
            post(livrarr_handlers::series::refresh_series::<AppState>),
        )
        .route(
            "/author/{id}/series/monitor",
            post(livrarr_handlers::series::monitor_series::<AppState>),
        )
        .route(
            "/series/{id}",
            get(livrarr_handlers::series::get_detail::<AppState>)
                .put(livrarr_handlers::series::update_series::<AppState>),
        )
        // Queue
        .route("/queue", get(livrarr_handlers::queue::list::<AppState>))
        .route(
            "/queue/{id}",
            delete(livrarr_handlers::queue::remove::<AppState>),
        )
        // Grabs
        .route(
            "/grab/{id}/retry",
            post(livrarr_handlers::queue::retry_import::<AppState>),
        )
        // Releases
        .route(
            "/release",
            get(livrarr_handlers::release::search::<AppState>),
        )
        .route(
            "/release/grab",
            post(livrarr_handlers::release::grab::<AppState>),
        )
        // Notifications
        .route(
            "/notification",
            get(livrarr_handlers::notification::list::<AppState>)
                .delete(livrarr_handlers::notification::dismiss_all::<AppState>),
        )
        .route(
            "/notification/{id}",
            put(livrarr_handlers::notification::mark_read::<AppState>)
                .delete(livrarr_handlers::notification::dismiss::<AppState>),
        )
        // History
        .route("/history", get(livrarr_handlers::history::list::<AppState>))
        // System
        .route("/health", get(livrarr_handlers::system::health::<AppState>))
        .route(
            "/system/status",
            get(livrarr_handlers::system::status::<AppState>),
        )
        .route(
            "/system/logs/tail",
            get(livrarr_handlers::system::log_tail::<AppState>),
        )
        .route(
            "/system/logs/level",
            put(livrarr_handlers::system::set_log_level::<AppState>),
        )
        // Filesystem browse
        .route(
            "/filesystem",
            get(livrarr_handlers::filesystem::browse::<AppState>),
        )
        // Manual import
        .route(
            "/manualimport/scan",
            post(livrarr_handlers::manual_import::scan::<AppState>),
        )
        .route(
            "/manualimport/progress/{scan_id}",
            get(livrarr_handlers::manual_import::scan_progress::<AppState>),
        )
        .route(
            "/manualimport/import",
            post(livrarr_handlers::manual_import::import::<AppState>),
        )
        .route(
            "/manualimport/search",
            post(livrarr_handlers::manual_import::search::<AppState>),
        )
        // Readarr import
        .route(
            "/import/readarr/connect",
            post(livrarr_handlers::readarr_import::connect::<AppState>),
        )
        .route(
            "/import/readarr/preview",
            post(livrarr_handlers::readarr_import::preview::<AppState>),
        )
        .route(
            "/import/readarr/start",
            post(livrarr_handlers::readarr_import::start::<AppState>),
        )
        .route(
            "/import/readarr/progress",
            get(livrarr_handlers::readarr_import::progress::<AppState>),
        )
        .route(
            "/import/readarr/history",
            get(livrarr_handlers::readarr_import::history::<AppState>),
        )
        .route(
            "/import/readarr/{import_id}",
            delete(livrarr_handlers::readarr_import::undo::<AppState>),
        )
        // List imports (CSV: Goodreads, Hardcover)
        .route(
            "/listimport",
            get(livrarr_handlers::list_import::list::<AppState>),
        )
        .route(
            "/listimport/preview",
            post(livrarr_handlers::list_import::preview::<AppState>),
        )
        .route(
            "/listimport/confirm",
            post(livrarr_handlers::list_import::confirm::<AppState>),
        )
        .route(
            "/listimport/{import_id}/complete",
            post(livrarr_handlers::list_import::complete::<AppState>),
        )
        .route(
            "/listimport/{import_id}",
            delete(livrarr_handlers::list_import::undo::<AppState>),
        )
        // Library files
        .route(
            "/workfile",
            get(livrarr_handlers::workfile::list::<AppState>),
        )
        .route(
            "/workfile/{id}",
            get(livrarr_handlers::workfile::get::<AppState>)
                .delete(livrarr_handlers::workfile::delete::<AppState>),
        )
        .route(
            "/workfile/{id}/send-email",
            post(livrarr_handlers::work::send_email::<AppState>),
        )
        .route(
            "/workfile/{id}/download",
            get(livrarr_handlers::work::download::<AppState>),
        )
        .route(
            "/workfile/{id}/progress",
            get(livrarr_handlers::workfile::get_progress::<AppState>)
                .put(livrarr_handlers::workfile::update_progress::<AppState>),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Stream endpoint — token auth via query param for HTML5 audio/video.
    let stream = Router::new().route(
        "/stream/{id}",
        get(livrarr_handlers::work::stream::<AppState>),
    );

    // Media cover serving (no auth — images loaded by browser directly).
    let mediacover = Router::new()
        .route(
            "/mediacover/{id}/cover.jpg",
            get(livrarr_handlers::mediacover::get_cover::<AppState>),
        )
        .route(
            "/mediacover/{id}/thumb.jpg",
            get(livrarr_handlers::mediacover::get_thumb::<AppState>),
        );

    // Cover proxy requires auth (user-supplied URLs → SSRF surface).
    let protected = protected.route(
        "/coverproxy",
        get(livrarr_handlers::coverproxy::proxy_cover::<AppState>),
    );

    // Combine API routes. Unmatched API paths return 404.
    let api = Router::new()
        .merge(public)
        .merge(protected)
        .merge(stream)
        .merge(mediacover)
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(GovernorLayer::new(global_governor));

    // OPDS routes — top level, before SPA fallback. Basic Auth handled per-handler.
    let opds = Router::new()
        .route("/", get(livrarr_handlers::opds::root::<AppState>))
        .route("/recent", get(livrarr_handlers::opds::recent::<AppState>))
        .route(
            "/author",
            get(livrarr_handlers::opds::author_list::<AppState>),
        )
        .route(
            "/author/{id}",
            get(livrarr_handlers::opds::author_works::<AppState>),
        )
        .route("/search", get(livrarr_handlers::opds::search::<AppState>))
        .route("/osd", get(livrarr_handlers::opds::opensearch::<AppState>))
        .route(
            "/cover/{work_id}",
            get(livrarr_handlers::opds::cover::<AppState>),
        )
        .route(
            "/download/{library_item_id}",
            get(livrarr_handlers::opds::download::<AppState>),
        );

    let app = Router::new().nest("/api/v1", api).nest("/opds", opds);

    // Static file serving with SPA fallback.
    let app = if ui_dir.is_dir() {
        let index_path = ui_dir.join("index.html");
        let serve_dir = ServeDir::new(&ui_dir).append_index_html_on_directories(true);
        let spa_fallback = ServeFile::new(index_path);
        app.fallback_service(serve_dir.fallback(spa_fallback))
    } else {
        app
    };

    // Security headers per security-model-policy.md
    app.layer(SetResponseHeaderLayer::overriding(
        axum::http::header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
        axum::http::header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
        axum::http::header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' blob:; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob: https: http:; connect-src 'self'; \
             worker-src 'self' blob:; frame-src 'self' blob:; \
             frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'",
        ),
    ))
    .with_state(state)
}
