use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, StatusCode};
use axum::routing::{delete, get, patch, post, put};
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

    // Rate limiter for setup: true <=5/min per IP — one attempt per 12s, no
    // burst head-start. burst_size(5) previously let 5 immediate attempts
    // through and then refilled every 12s, allowing 9 attempts inside one
    // minute instead of the intended 5.
    let setup_governor = GovernorConfigBuilder::default()
        .key_extractor(extractor.clone())
        .period(Duration::from_secs(12)) // 1 token per 12s, burst 1 => <=5 per 60s
        .burst_size(1)
        .finish()
        .expect("setup rate limiter config");

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
        .route(
            "/setup",
            post(livrarr_handlers::setup::setup::<AppState>)
                .layer(GovernorLayer::new(setup_governor)),
        )
        .route(
            "/auth/login",
            post(livrarr_handlers::auth::login::<AppState>)
                .layer(GovernorLayer::new(login_governor)),
        )
        .route("/health", get(livrarr_handlers::system::health::<AppState>));

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
            "/config/default-language",
            get(livrarr_handlers::config::get_default_language::<AppState>)
                .put(livrarr_handlers::config::update_default_language::<AppState>),
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
            "/work/preadd-covers",
            get(livrarr_handlers::work::preadd_cover_alternatives::<AppState>),
        )
        .route(
            "/work/refresh",
            post(livrarr_handlers::work::refresh_all::<AppState>),
        )
        .route(
            "/work/retry-incomplete",
            post(livrarr_handlers::work::retry_all_incomplete::<AppState>),
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
            "/work/{id}/cover/alternatives",
            get(livrarr_handlers::cover::get_cover_alternatives::<AppState>),
        )
        .route(
            "/work/{id}/cover/select",
            post(livrarr_handlers::cover::select_cover_handler::<AppState>),
        )
        .route(
            "/work/{id}/cover/upload",
            post(livrarr_handlers::cover::upload_cover_handler::<AppState>)
                .layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route(
            "/work/{id}/refresh",
            post(livrarr_handlers::work::refresh::<AppState>),
        )
        .route(
            "/work/{id}/pending-anchors",
            get(livrarr_handlers::work::list_pending_anchors::<AppState>),
        )
        .route(
            "/work/{id}/pending-anchors/{anchor_type}/affirm",
            post(livrarr_handlers::work::affirm_pending_anchor::<AppState>),
        )
        .route(
            "/work/{id}/identity/search",
            get(livrarr_handlers::work::manual_provider_search::<AppState>),
        )
        .route(
            "/work/{id}/merge/{loser_id}/preview",
            get(livrarr_handlers::work::preview_merge::<AppState>),
        )
        .route(
            "/work/{id}/merge/{loser_id}",
            post(livrarr_handlers::work::merge::<AppState>),
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
            "/author/{id}/merge",
            post(livrarr_handlers::author::merge::<AppState>),
        )
        .route(
            "/author/{id}/bibliography",
            get(livrarr_handlers::author::bibliography::<AppState>),
        )
        .route(
            "/author/{id}/bibliography/refresh",
            post(livrarr_handlers::author::refresh_bibliography::<AppState>),
        )
        // Author-provider linking
        .route(
            "/author-link-review",
            get(livrarr_handlers::author_link::list_author_link_review::<AppState>),
        )
        .route(
            "/author-link-review/{candidate_id}/pick",
            post(livrarr_handlers::author_link::pick_author_link_candidate::<AppState>),
        )
        .route(
            "/author-link-review/{candidate_id}/dismiss",
            post(livrarr_handlers::author_link::dismiss_author_link_candidate::<AppState>),
        )
        .route(
            "/author/{author_id}/route/{route_id}",
            delete(livrarr_handlers::author_link::remove_author_route::<AppState>),
        )
        .route(
            "/author/{author_id}/resolve",
            post(livrarr_handlers::author_link::re_resolve_author::<AppState>),
        )
        .route(
            "/author/{author_id}/name",
            put(livrarr_handlers::author_link::rename_author::<AppState>),
        )
        .route(
            "/author/{author_id}/display-name",
            put(livrarr_handlers::author_link::select_author_name::<AppState>),
        )
        .route(
            "/author-link-sweep/progress",
            get(livrarr_handlers::author_link::author_link_sweep_progress::<AppState>),
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
        .route(
            "/series/{id}/promote",
            post(livrarr_handlers::series::promote_series::<AppState>),
        )
        .route(
            "/series/{id}/books",
            get(livrarr_handlers::series::series_books::<AppState>),
        )
        // Queue
        .route("/queue", get(livrarr_handlers::queue::list::<AppState>))
        .route(
            "/queue/summary",
            get(livrarr_handlers::queue::summary::<AppState>),
        )
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
        .route(
            "/system/health-summary",
            get(livrarr_handlers::system::health_summary::<AppState>),
        )
        .route(
            "/system/provider-stats",
            get(livrarr_handlers::system::provider_stats::<AppState>),
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
        // Origin trust boundary (Unit B3 Part 1) — admin-managed allowlist of
        // private Readarr origins. `RequireAdmin` gates these two routes at
        // the handler; connect/preview/start above stay open to every
        // authenticated user.
        .route(
            "/import/readarr/origin",
            get(livrarr_handlers::readarr_import::list_origins::<AppState>)
                .post(livrarr_handlers::readarr_import::add_origin::<AppState>),
        )
        .route(
            "/import/readarr/origin/{id}",
            delete(livrarr_handlers::readarr_import::remove_origin::<AppState>),
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
        // Identity conflicts
        .route(
            "/identity-conflict",
            get(livrarr_handlers::identity_conflicts::list_open::<AppState>),
        )
        .route(
            "/identity-conflict/{id}",
            get(livrarr_handlers::identity_conflicts::get_detail::<AppState>),
        )
        .route(
            "/identity-conflict/{id}/resolve",
            post(livrarr_handlers::identity_conflicts::resolve::<AppState>),
        )
        .route(
            "/identity-conflict/{id}/dismiss",
            post(livrarr_handlers::identity_conflicts::dismiss::<AppState>),
        )
        // Identity review (AC-013 grey-park surface)
        .route(
            "/identity-review",
            get(livrarr_handlers::identity_review::list::<AppState>),
        )
        .route(
            "/identity-review/{work_id}/resolve",
            post(livrarr_handlers::identity_layer::resolve::<AppState>),
        )
        // Typed identity-v2 review cards. Keep the legacy route above while
        // clients migrate; its path parameter has always been the card id.
        .route(
            "/identity-review-card",
            get(livrarr_handlers::identity_layer::list::<AppState>),
        )
        .route(
            "/identity-review-card/{card_id}/resolve",
            post(livrarr_handlers::identity_layer::resolve::<AppState>),
        )
        .route(
            "/identity-review-card/{card_id}/dismiss",
            post(livrarr_handlers::identity_layer::dismiss::<AppState>),
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
        .route(
            "/workfile/{id}/chapters",
            get(livrarr_handlers::chapter::get_chapters::<AppState>),
        )
        .route(
            "/workfile/{id}/bookmarks",
            get(livrarr_handlers::bookmark::list_bookmarks::<AppState>)
                .post(livrarr_handlers::bookmark::create_bookmark::<AppState>),
        )
        .route(
            "/bookmarks/{id}",
            patch(livrarr_handlers::bookmark::rename_bookmark::<AppState>)
                .delete(livrarr_handlers::bookmark::delete_bookmark::<AppState>),
        )
        // Cross-format resume (Whispersync model)
        .route(
            "/workfile/{id}/cross-format/prompt",
            get(livrarr_handlers::cross_format::get_resume_prompt::<AppState>),
        )
        .route(
            "/workfile/{id}/cross-format/anchors",
            get(livrarr_handlers::cross_format::get_anchors::<AppState>),
        )
        .route(
            "/workfile/{id}/cross-format/decline",
            post(livrarr_handlers::cross_format::post_decline::<AppState>),
        )
        .route(
            "/workfile/{id}/cross-format/sync",
            post(livrarr_handlers::cross_format::post_sync_to_here::<AppState>),
        )
        // Unit C: mint a scoped, expiring stream token. Must be registered
        // here (before `.layer(auth_middleware)` below) — `Router::layer`
        // only wraps routes already present on the router, so a route
        // added afterward would NOT be auth-protected.
        .route(
            "/workfile/{id}/stream-token",
            post(livrarr_handlers::workfile::mint_stream_token_route::<AppState>),
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
        )
        .route(
            "/mediacover/{id}/audiocover.jpg",
            get(livrarr_handlers::cover::get_audiobook_cover::<AppState>),
        )
        .route(
            "/mediacover/{id}/audiocover_thumb.jpg",
            get(livrarr_handlers::cover::get_audiobook_thumb::<AppState>),
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
             img-src 'self' data: blob: https: http:; connect-src 'self' https://api.github.com; \
             worker-src 'self' blob:; frame-src 'self' blob:; \
             frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'",
        ),
    ))
    .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicBool, AtomicI64};
    use std::sync::Arc;
    use tower::ServiceExt;

    use livrarr_metadata as m;

    /// Build a full, real `AppState` so the test below can call the actual
    /// `build_router` — not a hand-duplicated stand-in — and exercise the
    /// production route table end to end. This mirrors `main.rs`'s
    /// composition root (same constructors, same order) with one
    /// simplification: no provider credentials/clients are wired (empty
    /// maps, zero registered providers, `job_runner: None`) because this
    /// test only ever reaches the `/setup` handler, which touches nothing
    /// but `auth_service` — the second (rate-limited) request never reaches
    /// a handler at all. Returns the backing `TempDir` alongside the state;
    /// it must outlive the router.
    async fn test_app_state() -> (AppState, tempfile::TempDir) {
        let db = livrarr_db::test_helpers::create_test_db().await;
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let data_dir_arc = Arc::new(data_dir.clone());

        let auth_service = Arc::new(crate::auth_service::ServerAuthService::new(
            db.clone(),
            crate::auth_crypto::RealAuthCrypto,
        ));

        let ua = livrarr_http::livrarr_user_agent();
        let http_client = livrarr_http::HttpClient::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(&ua)
            .build()
            .expect("http client");
        let http_client_safe = livrarr_http::HttpClient::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(&ua)
            .ssrf_safe(true)
            .build()
            .expect("ssrf-safe http client");
        let http_fetcher = livrarr_http::fetcher::HttpFetcherImpl::new().expect("http fetcher");
        let llm_http_client = livrarr_http::HttpClient::builder()
            .timeout(Duration::from_secs(60))
            .user_agent(&ua)
            .build()
            .expect("llm http client");

        let live_metadata_config = livrarr_external_data::live_config::LiveMetadataConfig::new(
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
            },
        );
        let transport_cache = Arc::new(
            livrarr_external_data::transport_cache::TransportCache::new(Duration::from_secs(300)),
        );

        let import_semaphore = Arc::new(tokio::sync::Semaphore::new(2));
        let cover_proxy_cache = Arc::new(crate::infra::cover_cache::CoverProxyCache::new());
        let rss_last_run = Arc::new(AtomicI64::new(0));
        let rss_sync_running = Arc::new(AtomicBool::new(false));
        let manual_import_scans_shared = Arc::new(dashmap::DashMap::new());
        let log_buffer = Arc::new(crate::state::LogBuffer::new());
        let log_level_handle = {
            let (_layer, handle): (
                tracing_subscriber::reload::Layer<
                    tracing_subscriber::EnvFilter,
                    tracing_subscriber::Registry,
                >,
                tracing_subscriber::reload::Handle<
                    tracing_subscriber::EnvFilter,
                    tracing_subscriber::Registry,
                >,
            ) = tracing_subscriber::reload::Layer::new(tracing_subscriber::EnvFilter::new("info"));
            Arc::new(crate::state::LogLevelHandle::new(handle, "info"))
        };

        let settings_service_arc = Arc::new(
            crate::services::settings_service::LiveSettingsService::new(db.clone()),
        );
        let import_io_arc = Arc::new(crate::import_io_service::ImportIoServiceImpl::new(
            db.clone(),
        ));
        let import_workflow_arc =
            Arc::new(livrarr_library::import_workflow::ImportWorkflowImpl::new(
                db.clone(),
                import_semaphore.clone(),
                data_dir_arc.clone(),
                Arc::new(crate::chapter_extractor::ChapterExtractorImpl),
            ));
        let tag_service_arc = Arc::new(crate::tag_service::LiveTagService::new(
            import_io_arc.clone(),
            data_dir_arc.clone(),
            db.clone(),
        ));
        let import_svc_arc = Arc::new(crate::import_service::LiveImportService::new(
            import_io_arc.clone(),
            import_workflow_arc.clone(),
            tag_service_arc.clone(),
            settings_service_arc.clone(),
            http_client_safe.clone(),
        ));

        let trusted_origins_arc = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());

        let readarr_import_service_arc =
            Arc::new(crate::readarr_import_service::LiveReadarrImportService::new(db.clone()));
        let readarr_import_progress_arc = Arc::new(tokio::sync::Mutex::new(
            crate::readarr_import_service::ReadarrImportProgress::default(),
        ));

        // No providers registered anywhere below (empty maps / empty queue) —
        // this test never exercises enrichment, so there is nothing for a
        // real Hardcover/OpenLibrary/etc. client to do.
        let identity_resolver_arc =
            Arc::new(m::english_identity_resolver::LiveEnglishIdentityResolver {
                clients: std::collections::HashMap::new(),
                cache: transport_cache.clone(),
                config: m::english_identity_resolver::ResolverConfig::default(),
            });

        let db_arc = Arc::new(db.clone());
        let queue = Arc::new(m::DefaultProviderQueueBuilder::new().build(db_arc.clone()));
        let merge_engine = Arc::new(m::DefaultMergeEngine::new(m::PriorityModel::english()));
        let enrichment_service = Arc::new(m::EnrichmentServiceImpl::new(
            db_arc.clone(),
            queue.clone(),
            merge_engine,
            false,
        ));

        let work_service_arc: Arc<crate::state::LiveWorkService> = {
            let ew = m::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                enrichment_service.clone(),
            );
            Arc::new(
                m::work_service::WorkServiceImpl::new(
                    db.clone(),
                    ew,
                    http_fetcher.clone(),
                    data_dir.clone(),
                )
                .with_resolver(identity_resolver_arc.clone()),
            )
        };

        let discovery_service_arc = Arc::new(
            m::discovery_service::DiscoveryServiceImpl::new(
                db.clone(),
                http_fetcher.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    llm_http_client.clone(),
                ),
            )
            .with_resolver(identity_resolver_arc.clone()),
        );

        let hmac_key = crate::cover_service::generate_hmac_key();
        let cover_service = Arc::new(crate::cover_service::LiveCoverService::new(
            db.clone(),
            http_fetcher.clone(),
            std::collections::HashMap::new(),
            hmac_key.clone(),
            data_dir_arc.clone(),
        ));
        let identity_road_arc = Arc::new(crate::identity_layer::build_live_identity_road(
            db.clone(),
            http_fetcher.clone(),
            http_client.clone(),
            live_metadata_config.clone(),
        ));

        let state = AppState {
            db: db.clone(),
            auth_service,
            http_client: http_client.clone(),
            http_client_safe,
            http_fetcher: http_fetcher.clone(),
            config: Arc::new(crate::config::AppConfig::default()),
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
            identity_road: identity_road_arc.clone(),

            author_service: Arc::new(m::author_service::AuthorServiceImpl::new(
                db.clone(),
                http_fetcher.clone(),
                livrarr_external_data::llm_caller_service::LlmCallerImpl::new(
                    live_metadata_config.clone(),
                    llm_http_client.clone(),
                ),
            )),
            author_link_service: Arc::new(
                crate::services::author_linking_service::LiveAuthorLinkingService,
            ),
            series_service: Arc::new(m::series_service::SeriesServiceImpl::new(db.clone())),
            series_query_service: Arc::new(
                m::series_query_service::SeriesQueryServiceImpl::new(
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
            discovery_service: discovery_service_arc.clone(),
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
            bookmark_service: Arc::new(
                livrarr_library::bookmark_service::BookmarkServiceImpl::new(db.clone()),
            ),
            cross_format_service: Arc::new(
                livrarr_library::cross_format_service::CrossFormatServiceImpl::new(
                    db.clone(),
                    livrarr_library::file_service::FileServiceImpl::new(db.clone()),
                ),
            ),
            import_workflow: import_workflow_arc.clone(),
            rss_sync_workflow: {
                let rs = Arc::new(livrarr_download::release_service::ReleaseServiceImpl::new(
                    db.clone(),
                    http_fetcher.clone(),
                    trusted_origins_arc.clone(),
                ));
                Arc::new(m::rss_sync_workflow::RssSyncWorkflowImpl::new(
                    Arc::new(db.clone()),
                    Arc::new(http_fetcher.clone()),
                    rs,
                ))
            },
            list_service: {
                let ew = m::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                    enrichment_service.clone(),
                );
                let ws = m::work_service::WorkServiceImpl::new(
                    db.clone(),
                    ew,
                    http_fetcher.clone(),
                    data_dir.clone(),
                );
                Arc::new(m::list_service::ListServiceImpl::with_identity_road(
                    db.clone(),
                    ws,
                    http_fetcher.clone(),
                    m::list_service::NoOpBibliographyTrigger,
                    identity_road_arc.clone(),
                ))
            },
            identity_conflict_service: Arc::new(
                crate::services::identity_conflict_service::LiveIdentityConflictService::new(
                    db.clone(),
                ),
            ),
            identity_resolver: identity_resolver_arc,
            enrichment_workflow: Arc::new(
                m::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                    enrichment_service.clone(),
                ),
            ),
            author_monitor_workflow: {
                let ew = m::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
                    enrichment_service.clone(),
                );
                let ws = m::work_service::WorkServiceImpl::new(
                    db.clone(),
                    ew,
                    http_fetcher.clone(),
                    data_dir.clone(),
                );
                Arc::new(
                    m::author_monitor_workflow::AuthorMonitorWorkflowImpl::with_identity_road(
                        Arc::new(db.clone()),
                        Arc::new(ws),
                        Arc::new(http_fetcher.clone()),
                        identity_road_arc.clone(),
                    ),
                )
            },
            readarr_import_service: readarr_import_service_arc.clone(),
            settings_service: settings_service_arc.clone(),
            notification_service: Arc::new(
                crate::notification_service::NotificationServiceImpl::new(db.clone()),
            ),
            history_service: Arc::new(crate::history_service::HistoryServiceImpl::new(db.clone())),
            queue_service: Arc::new(crate::queue_service::QueueServiceImpl::new(
                db.clone(),
                http_client.clone(),
            )),
            import_io_service: import_io_arc.clone(),
            manual_import_db_service: Arc::new(
                crate::manual_import_service::ManualImportServiceImpl::new(db.clone()),
            ),

            rss_sync_state: crate::state::RssSyncState {
                running: rss_sync_running.clone(),
                last_run: rss_last_run.clone(),
            },
            system_state: crate::state::SystemState {
                log_buffer: log_buffer.clone(),
                log_level_handle: log_level_handle.clone(),
            },
            provider_stats_service: Arc::new(crate::state::LiveProviderStatsService::new(
                db.clone(),
            )),
            log_surface_accessor: crate::state::LogSurfaceAccessorImpl {
                log_dir: data_dir.join("logs"),
                init_error: None,
            },
            live_metadata_config_accessor: crate::state::LiveMetadataConfigAccessorImpl(
                live_metadata_config.clone(),
            ),
            cover_proxy_cache_accessor: crate::state::CoverProxyCacheAccessorImpl(
                cover_proxy_cache.clone(),
            ),
            tag_service: tag_service_arc.clone(),
            email_svc: Arc::new(crate::email_service::LiveEmailService::new(
                settings_service_arc.clone(),
            )),
            import_svc: import_svc_arc,
            matching_svc: crate::matching_service::LiveMatchingService,
            manual_import_scan_svc:
                crate::manual_import_scan_service::LiveManualImportScanService {
                    scans: manual_import_scans_shared.clone(),
                },
            readarr_import_wf: Arc::new(
                crate::readarr_import_workflow::LiveReadarrImportWorkflow::new(
                    http_fetcher.clone(),
                    readarr_import_service_arc,
                    readarr_import_progress_arc,
                    data_dir_arc.clone(),
                    work_service_arc.clone(),
                    db.clone(),
                    import_workflow_arc.clone(),
                )
                .with_identity_road(identity_road_arc.clone()),
            ),
            cover_service,
            preadd_cover_service: Arc::new(m::preadd_cover_service::LivePreaddCoverService::new(
                std::collections::HashMap::new(),
            )),
            hmac_key,
            trusted_origins_rebuilder: crate::state::TrustedOriginsRebuilderImpl(
                trusted_origins_arc.clone(),
            ),
        };

        (state, tmp)
    }

    /// Drives the REAL production router (`build_router`, not a hand-rolled
    /// stand-in) so that deleting the `.layer(GovernorLayer::new(setup_governor))`
    /// line from the `/setup` route in `build_router` turns this test red.
    ///
    /// Only asserts the `burst_size(1)` behavior (2nd immediate request is
    /// rate-limited) — a full 60-second-window assertion (proving the refill
    /// rate holds a real client to 5/min rather than more) would require the
    /// test to either sleep ~48 real seconds or mock the governor's clock,
    /// neither of which this suite currently has infrastructure for. Noted
    /// as a gap rather than faked.
    #[tokio::test]
    async fn setup_route_burst_of_one_blocks_a_second_immediate_request() {
        let (state, _tmp) = test_app_state().await;
        let ui_dir = state.data_dir.join("ui-not-present-in-test");
        let app = build_router(state, ui_dir);

        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 12345);
        let body = || Body::from(r#"{"username":"admin","password":"correct-horse-battery"}"#);

        let mut first = Request::builder()
            .method("POST")
            .uri("/api/v1/setup")
            .header("content-type", "application/json")
            .body(body())
            .unwrap();
        first.extensions_mut().insert(ConnectInfo(peer));
        let resp = app.clone().oneshot(first).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "the first request must reach the real setup handler and succeed"
        );

        let mut second = Request::builder()
            .method("POST")
            .uri("/api/v1/setup")
            .header("content-type", "application/json")
            .body(body())
            .unwrap();
        second.extensions_mut().insert(ConnectInfo(peer));
        let resp = app.clone().oneshot(second).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "burst_size(1) means a 2nd immediate request from the same IP must be rate-limited"
        );
    }
}
