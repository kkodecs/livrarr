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
    // Trusted proxies from config (empty = direct exposure, peer IP only).
    // Rejected entries trust nothing; `config::config_warnings` reports them.
    let (trusted_proxies, _rejected) =
        crate::rate_limit::parse_trusted_proxies(&state.config.server.trusted_proxies);
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
            "/config/languages",
            get(livrarr_handlers::config::get_languages::<AppState>),
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
            "/system/health",
            get(livrarr_handlers::system::admin_health::<AppState>),
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
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
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
    async fn app_state_with_auth(
        make_auth: impl FnOnce(
            livrarr_db::sqlite::SqliteDb,
        ) -> crate::auth_service::ServerAuthService<
            crate::auth_crypto::RealAuthCrypto,
        >,
    ) -> (AppState, tempfile::TempDir) {
        let db = livrarr_db::test_helpers::create_test_db().await;
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let data_dir_arc = Arc::new(data_dir.clone());

        let auth_service = Arc::new(make_auth(db.clone()));

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
            http_client.clone(),
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

    /// Harness state whose auth service is built without a setup token.
    async fn test_app_state_without_setup_token() -> (AppState, tempfile::TempDir) {
        app_state_with_auth(|db| {
            crate::auth_service::ServerAuthService::new(db, crate::auth_crypto::RealAuthCrypto)
        })
        .await
    }

    /// Harness state whose auth service holds `setup_token`, as `main` gives it
    /// the token from `{data}/setup-token`.
    async fn test_app_state_with_setup_token(
        setup_token: &'static str,
    ) -> (AppState, tempfile::TempDir) {
        app_state_with_auth(move |db| {
            // The token file lives under a folder that never exists, so the
            // best-effort removal after a successful setup finds nothing.
            let file = std::env::temp_dir()
                .join("livrarr-router-tests-no-such-folder")
                .join(crate::setup_token::SETUP_TOKEN_FILE_NAME);
            crate::auth_service::ServerAuthService::new(db, crate::auth_crypto::RealAuthCrypto)
                .with_setup_token(crate::setup_token::SetupToken::new(setup_token, file))
        })
        .await
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
        let (state, _tmp) = test_app_state_with_setup_token(KNOWN_SETUP_TOKEN).await;
        let ui_dir = state.data_dir.join("ui-not-present-in-test");
        let app = build_router(state, ui_dir);

        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 12345);
        let body = || {
            Body::from(
                serde_json::json!({
                    "username": "admin",
                    "password": "correct-horse-battery",
                    "setupToken": KNOWN_SETUP_TOKEN,
                })
                .to_string(),
            )
        };

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

    /// The setup token given to `test_app_state_with_setup_token` and sent as
    /// the right proof.
    const KNOWN_SETUP_TOKEN: &str = "6f1c2b9e0d4a7f3851c6e2b0a9d8f714";

    const SETUP_TOKEN_MESSAGE: &str = "The setup token is missing or wrong. Find it in \
        Livrarr's startup output, or in the file setup-token in its data folder \
        (/config/setup-token in Docker).";

    fn peer(last_octet: u8) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, last_octet)), 40000)
    }

    /// Sends one request through the real router from `peer`, returning the
    /// status and the serialized body.
    async fn call(
        app: &Router,
        from: SocketAddr,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = match body {
            Some(json) => builder
                .header("content-type", "application/json")
                .body(Body::from(json.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        request.extensions_mut().insert(ConnectInfo(from));
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn setup_status_body(app: &Router, from: SocketAddr) -> serde_json::Value {
        let (status, body) = call(app, from, "GET", "/api/v1/setup/status", &[], None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    fn assert_token_refusal(status: StatusCode, body: &str, case: &str) {
        assert_eq!(status, StatusCode::FORBIDDEN, "{case}: {body}");
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["message"], SETUP_TOKEN_MESSAGE, "{case}: {body}");
        assert!(
            !body.contains(KNOWN_SETUP_TOKEN),
            "{case}: body carries the token"
        );
    }

    #[tokio::test]
    async fn setup_is_refused_when_the_auth_service_holds_no_token() {
        let (state, _tmp) = test_app_state_without_setup_token().await;
        let ui_dir = state.data_dir.join("ui-not-present-in-test");
        let app = build_router(state, ui_dir);

        let (status, text) = call(
            &app,
            peer(40),
            "POST",
            "/api/v1/setup",
            &[],
            Some(serde_json::json!({
                "username": "owner",
                "password": "owner-password-1",
                "setupToken": KNOWN_SETUP_TOKEN,
            })),
        )
        .await;
        assert_token_refusal(status, &text, "service without a token");
        assert_eq!(
            setup_status_body(&app, peer(41)).await,
            serde_json::json!({"setupRequired": true})
        );
    }

    #[tokio::test]
    async fn two_simultaneous_right_token_setups_have_one_winner() {
        let (state, _tmp) = test_app_state_with_setup_token(KNOWN_SETUP_TOKEN).await;
        let ui_dir = state.data_dir.join("ui-not-present-in-test");
        let app = build_router(state, ui_dir);

        let body = |name: &str| {
            serde_json::json!({
                "username": name,
                "password": "owner-password-1",
                "setupToken": KNOWN_SETUP_TOKEN,
            })
        };
        let (first, second) = tokio::join!(
            call(
                &app,
                peer(50),
                "POST",
                "/api/v1/setup",
                &[],
                Some(body("owner-a"))
            ),
            call(
                &app,
                peer(51),
                "POST",
                "/api/v1/setup",
                &[],
                Some(body("owner-b"))
            ),
        );
        let mut statuses = [first.0, second.0];
        statuses.sort();
        assert_eq!(
            statuses,
            [StatusCode::OK, StatusCode::CONFLICT],
            "first={first:?} second={second:?}"
        );
    }

    // Language list route and the password rule, through the real router.

    static NEXT_PEER: AtomicU32 = AtomicU32::new(1);

    /// A peer address no other request in this module uses, so no
    /// per-address rate limit is shared between calls.
    fn fresh_peer() -> SocketAddr {
        let [_, b, c, d] = NEXT_PEER.fetch_add(1, Ordering::Relaxed).to_be_bytes();
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, b, c, d)), 41000)
    }

    #[derive(Clone, Copy)]
    enum Caller<'a> {
        SignedOut,
        Session(&'a str),
        ApiKey(&'a str),
    }

    /// One request through the real router as `caller`, from a fresh peer.
    async fn api(
        app: &Router,
        caller: Caller<'_>,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        match caller {
            Caller::SignedOut => call(app, fresh_peer(), method, uri, &[], body).await,
            Caller::Session(token) => {
                let bearer = format!("Bearer {token}");
                call(
                    app,
                    fresh_peer(),
                    method,
                    uri,
                    &[("authorization", bearer.as_str())],
                    body,
                )
                .await
            }
            Caller::ApiKey(key) => {
                call(app, fresh_peer(), method, uri, &[("x-api-key", key)], body).await
            }
        }
    }

    fn json_of(body: &str) -> serde_json::Value {
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
    }

    fn message_of(body: &str) -> String {
        json_of(body)["message"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    const OWNER: &str = "owner";
    const OWNER_PASSWORD: &str = "owner-password-1";

    /// The real router after first-run setup through `POST /setup`, with the
    /// owner's session token and API key from its reply.
    struct OwnedApp {
        app: Router,
        db: livrarr_db::sqlite::SqliteDb,
        owner_session: String,
        owner_api_key: String,
        _tmp: tempfile::TempDir,
    }

    async fn router_before_setup() -> (Router, livrarr_db::sqlite::SqliteDb, tempfile::TempDir) {
        let (state, tmp) = test_app_state_with_setup_token(KNOWN_SETUP_TOKEN).await;
        let db = state.db.clone();
        let ui_dir = state.data_dir.join("ui-not-present-in-test");
        (build_router(state, ui_dir), db, tmp)
    }

    async fn router_with_owner() -> OwnedApp {
        let (app, db, tmp) = router_before_setup().await;
        let (status, body) = api(
            &app,
            Caller::SignedOut,
            "POST",
            "/api/v1/setup",
            Some(serde_json::json!({
                "username": OWNER,
                "password": OWNER_PASSWORD,
                "setupToken": KNOWN_SETUP_TOKEN,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "owner setup: {body}");
        let reply = json_of(&body);
        OwnedApp {
            app,
            db,
            owner_session: reply["token"].as_str().unwrap().to_string(),
            owner_api_key: reply["apiKey"].as_str().unwrap().to_string(),
            _tmp: tmp,
        }
    }

    /// A normal user created by the owner through `POST /user`; returns its id.
    async fn create_normal_user(owned: &OwnedApp, username: &str, password: &str) -> i64 {
        let (status, body) = api(
            &owned.app,
            Caller::Session(&owned.owner_session),
            "POST",
            "/api/v1/user",
            Some(serde_json::json!({
                "username": username,
                "password": password,
                "role": "user",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create {username}: {body}");
        json_of(&body)["id"].as_i64().unwrap()
    }

    /// Signs in through `POST /auth/login`; the session token on success.
    async fn sign_in(app: &Router, username: &str, password: &str) -> Option<String> {
        let (status, body) = api(
            app,
            Caller::SignedOut,
            "POST",
            "/api/v1/auth/login",
            Some(serde_json::json!({
                "username": username,
                "password": password,
                "rememberMe": false,
            })),
        )
        .await;
        (status == StatusCode::OK).then(|| json_of(&body)["token"].as_str().unwrap().to_string())
    }

    /// `GET /auth/me` with `session`: the signed-in user's role, or None when refused.
    async fn role_of_session(app: &Router, session: &str) -> Option<String> {
        let (status, body) = api(
            app,
            Caller::Session(session),
            "GET",
            "/api/v1/auth/me",
            None,
        )
        .await;
        (status == StatusCode::OK)
            .then(|| json_of(&body)["user"]["role"].as_str().unwrap().to_string())
    }

    const HARDCOVER_TOKEN: &str = "hc-token-6a1f0c";
    const AI_ENDPOINT: &str = "https://llm-host-77d2.example.com/v1";
    const AI_KEY: &str = "ai-key-90be41";
    const GOOGLE_BOOKS_KEY: &str = "gb-key-3c58aa";

    /// The owner saves every metadata secret and English, French and German
    /// through `PUT /config/metadata`, then creates a normal user and signs it in.
    async fn router_with_saved_languages() -> (OwnedApp, String) {
        let owned = router_with_owner().await;
        let (status, body) = api(
            &owned.app,
            Caller::Session(&owned.owner_session),
            "PUT",
            "/api/v1/config/metadata",
            Some(serde_json::json!({
                "hardcoverEnabled": true,
                "hardcoverApiToken": HARDCOVER_TOKEN,
                "llmEnabled": true,
                "llmProvider": "openai",
                "llmEndpoint": AI_ENDPOINT,
                "llmApiKey": AI_KEY,
                "llmModel": "model-name-1",
                "googleBooksApiKey": GOOGLE_BOOKS_KEY,
                "languages": ["en", "fr", "de"],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "save metadata settings: {body}");
        assert_eq!(
            json_of(&body)["languages"],
            serde_json::json!(["en", "fr", "de"]),
            "the saved language list"
        );

        create_normal_user(&owned, "reader", "reader-password-1").await;
        let reader = sign_in(&owned.app, "reader", "reader-password-1")
            .await
            .expect("the normal user signs in");
        assert_eq!(
            role_of_session(&owned.app, &reader).await.as_deref(),
            Some("user"),
            "the reader is a normal user"
        );
        (owned, reader)
    }

    #[tokio::test]
    async fn language_route_gives_every_signed_in_user_only_the_saved_list() {
        let (owned, reader) = router_with_saved_languages().await;
        let expected = serde_json::json!({ "languages": ["en", "fr", "de"] });

        let mut wrong = Vec::new();
        for (who, caller) in [
            ("normal user", Caller::Session(&reader)),
            ("admin", Caller::Session(&owned.owner_session)),
            ("admin by API key", Caller::ApiKey(&owned.owner_api_key)),
        ] {
            let (status, body) =
                api(&owned.app, caller, "GET", "/api/v1/config/languages", None).await;
            if status != StatusCode::OK {
                wrong.push(format!("{who}: status {status}, expected 200"));
            }
            if json_of(&body) != expected {
                wrong.push(format!("{who}: body {body:?}, expected {expected}"));
            }
            for secret in [HARDCOVER_TOKEN, AI_ENDPOINT, AI_KEY, GOOGLE_BOOKS_KEY] {
                if body.contains(secret) {
                    wrong.push(format!("{who}: body carries {secret}"));
                }
            }
        }
        let (status, _) = api(
            &owned.app,
            Caller::SignedOut,
            "GET",
            "/api/v1/config/languages",
            None,
        )
        .await;
        if status != StatusCode::UNAUTHORIZED {
            wrong.push(format!("signed out: status {status}, expected 401"));
        }

        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn metadata_settings_route_still_refuses_a_normal_user() {
        let (owned, reader) = router_with_saved_languages().await;

        let (admin_status, admin_body) = api(
            &owned.app,
            Caller::Session(&owned.owner_session),
            "GET",
            "/api/v1/config/metadata",
            None,
        )
        .await;
        assert_eq!(admin_status, StatusCode::OK, "control, admin: {admin_body}");

        let (status, body) = api(
            &owned.app,
            Caller::Session(&reader),
            "GET",
            "/api/v1/config/metadata",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }

    #[derive(Clone, Copy, Debug)]
    enum PasswordDoor {
        Setup,
        OwnProfile,
        AdminCreate,
        AdminEdit,
    }

    const PASSWORD_DOORS: [PasswordDoor; 4] = [
        PasswordDoor::Setup,
        PasswordDoor::OwnProfile,
        PasswordDoor::AdminCreate,
        PasswordDoor::AdminEdit,
    ];

    /// What one attempt to set a password through a door did.
    struct PasswordAttempt {
        status: StatusCode,
        message: String,
        /// The account and the attempted password sign in afterwards.
        new_password_signs_in: bool,
        /// Setup and admin create: an account with the attempted name exists afterwards.
        account_exists: Option<bool>,
        /// Profile and admin edit: the password held before the attempt still signs in.
        old_password_signs_in: Option<bool>,
        /// Profile and admin edit: the user's session issued before the attempt still works.
        earlier_session_works: Option<bool>,
    }

    static NEXT_ACCOUNT: AtomicU32 = AtomicU32::new(1);

    /// Sets `password` through `door`. Setup gets a router of its own, as it
    /// can run once; the other doors act on a new normal user in `owned`.
    async fn set_password_through(
        owned: &OwnedApp,
        door: PasswordDoor,
        password: &str,
    ) -> PasswordAttempt {
        let n = NEXT_ACCOUNT.fetch_add(1, Ordering::Relaxed);
        match door {
            PasswordDoor::Setup => {
                let (app, _db, _tmp) = router_before_setup().await;
                let (status, body) = api(
                    &app,
                    Caller::SignedOut,
                    "POST",
                    "/api/v1/setup",
                    Some(serde_json::json!({
                        "username": OWNER,
                        "password": password,
                        "setupToken": KNOWN_SETUP_TOKEN,
                    })),
                )
                .await;
                let setup_required = setup_status_body(&app, fresh_peer()).await["setupRequired"]
                    .as_bool()
                    .unwrap();
                PasswordAttempt {
                    status,
                    message: message_of(&body),
                    new_password_signs_in: sign_in(&app, OWNER, password).await.is_some(),
                    account_exists: Some(!setup_required),
                    old_password_signs_in: None,
                    earlier_session_works: None,
                }
            }
            PasswordDoor::AdminCreate => {
                let username = format!("created-{n}");
                let (status, body) = api(
                    &owned.app,
                    Caller::Session(&owned.owner_session),
                    "POST",
                    "/api/v1/user",
                    Some(serde_json::json!({
                        "username": username,
                        "password": password,
                        "role": "user",
                    })),
                )
                .await;
                let (list_status, list) = api(
                    &owned.app,
                    Caller::Session(&owned.owner_session),
                    "GET",
                    "/api/v1/user",
                    None,
                )
                .await;
                assert_eq!(list_status, StatusCode::OK, "user list: {list}");
                let exists = json_of(&list)
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|u| u["username"] == username.as_str());
                PasswordAttempt {
                    status,
                    message: message_of(&body),
                    new_password_signs_in: sign_in(&owned.app, &username, password).await.is_some(),
                    account_exists: Some(exists),
                    old_password_signs_in: None,
                    earlier_session_works: None,
                }
            }
            PasswordDoor::OwnProfile | PasswordDoor::AdminEdit => {
                let username = format!("member-{n}");
                let old_password = format!("old-password-{n}");
                let id = create_normal_user(owned, &username, &old_password).await;
                let session = sign_in(&owned.app, &username, &old_password)
                    .await
                    .expect("control: the user signs in with its first password");
                assert_eq!(
                    role_of_session(&owned.app, &session).await.as_deref(),
                    Some("user"),
                    "control: the session works before the change"
                );
                let change = serde_json::json!({ "password": password });
                let (status, body) = match door {
                    PasswordDoor::OwnProfile => {
                        api(
                            &owned.app,
                            Caller::Session(&session),
                            "PUT",
                            "/api/v1/auth/profile",
                            Some(change),
                        )
                        .await
                    }
                    _ => {
                        api(
                            &owned.app,
                            Caller::Session(&owned.owner_session),
                            "PUT",
                            &format!("/api/v1/user/{id}"),
                            Some(change),
                        )
                        .await
                    }
                };
                let earlier_session_works = role_of_session(&owned.app, &session).await.is_some();
                PasswordAttempt {
                    status,
                    message: message_of(&body),
                    new_password_signs_in: sign_in(&owned.app, &username, password).await.is_some(),
                    account_exists: None,
                    old_password_signs_in: Some(
                        sign_in(&owned.app, &username, &old_password)
                            .await
                            .is_some(),
                    ),
                    earlier_session_works: Some(earlier_session_works),
                }
            }
        }
    }

    /// Checks that `password` is refused at the minimum on every door, with
    /// nothing written; returns one line per difference.
    async fn refused_below_minimum(owned: &OwnedApp, label: &str, password: &str) -> Vec<String> {
        let mut wrong = Vec::new();
        for door in PASSWORD_DOORS {
            let attempt = set_password_through(owned, door, password).await;
            if attempt.status != StatusCode::UNPROCESSABLE_ENTITY {
                wrong.push(format!(
                    "{door:?} {label}: status {}, expected 422",
                    attempt.status
                ));
            }
            if attempt.message != "invalid password: minimum 8 characters" {
                wrong.push(format!("{door:?} {label}: message {:?}", attempt.message));
            }
            if attempt.account_exists == Some(true) {
                wrong.push(format!("{door:?} {label}: an account was made"));
            }
            if attempt.old_password_signs_in == Some(false) {
                wrong.push(format!(
                    "{door:?} {label}: the old password no longer signs in"
                ));
            }
        }
        wrong
    }

    /// Checks each of `accepted` is accepted on every door and signs in, and
    /// `refused` is answered 422; returns one line per difference.
    async fn accepted_and_refused(
        owned: &OwnedApp,
        accepted: &[(&str, String)],
        refused: (&str, String),
    ) -> Vec<String> {
        let mut wrong = Vec::new();
        for door in PASSWORD_DOORS {
            for (label, password) in accepted {
                let attempt = set_password_through(owned, door, password).await;
                if !attempt.status.is_success() {
                    wrong.push(format!(
                        "{door:?} {label}: status {} {:?}, expected success",
                        attempt.status, attempt.message
                    ));
                }
                if !attempt.new_password_signs_in {
                    wrong.push(format!(
                        "{door:?} {label}: the new password does not sign in"
                    ));
                }
            }
            let (label, password) = &refused;
            let attempt = set_password_through(owned, door, password).await;
            if attempt.status != StatusCode::UNPROCESSABLE_ENTITY {
                wrong.push(format!(
                    "{door:?} {label}: status {}, expected 422",
                    attempt.status
                ));
            }
        }
        wrong
    }

    #[tokio::test]
    async fn password_under_eight_characters_is_refused_on_every_door() {
        let owned = router_with_owner().await;
        let mut wrong = refused_below_minimum(&owned, "ASCII 7", "abcdefg").await;
        wrong.extend(refused_below_minimum(&owned, "7 é", &"é".repeat(7)).await);
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn ascii_password_from_eight_to_1024_characters_is_accepted_on_every_door() {
        let owned = router_with_owner().await;
        let wrong = accepted_and_refused(
            &owned,
            &[
                ("ASCII 8", "abcdefgh".to_string()),
                ("ASCII 1,024", "a".repeat(1024)),
            ],
            ("ASCII 1,025", "a".repeat(1025)),
        )
        .await;
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn multibyte_password_minimum_counts_characters_and_maximum_counts_bytes() {
        let owned = router_with_owner().await;
        let wrong = accepted_and_refused(
            &owned,
            &[
                ("8 é", "é".repeat(8)),
                ("512 é (1,024 bytes)", "é".repeat(512)),
            ],
            ("513 é (1,026 bytes)", "é".repeat(513)),
        )
        .await;
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn profile_and_admin_edit_without_a_password_still_succeed() {
        let owned = router_with_owner().await;
        let id = create_normal_user(&owned, "renamer", "renamer-password-1").await;
        let session = sign_in(&owned.app, "renamer", "renamer-password-1")
            .await
            .expect("control: the user signs in");

        let (status, body) = api(
            &owned.app,
            Caller::Session(&session),
            "PUT",
            "/api/v1/auth/profile",
            Some(serde_json::json!({ "username": "renamed-self" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "own profile: {body}");

        let (status, body) = api(
            &owned.app,
            Caller::Session(&owned.owner_session),
            "PUT",
            &format!("/api/v1/user/{id}"),
            Some(serde_json::json!({ "username": "renamed-by-admin" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "admin edit: {body}");
    }

    #[tokio::test]
    async fn existing_six_character_password_still_signs_in() {
        use crate::auth_crypto::AuthCryptoService;
        use livrarr_db::UserDb;

        let owned = router_with_owner().await;
        let crypto = crate::auth_crypto::RealAuthCrypto;
        let api_key = crypto.generate_token().await.unwrap();
        owned
            .db
            .create_user(livrarr_db::CreateUserDbRequest {
                username: "veteran".to_string(),
                password_hash: crypto.hash_password("six6ch").await.unwrap(),
                role: livrarr_db::UserRole::User,
                api_key_hash: crypto.hash_token(&api_key).await.unwrap(),
            })
            .await
            .expect("production user writer");

        let session = sign_in(&owned.app, "veteran", "six6ch").await;
        assert!(session.is_some(), "the six-character password signs in");
        assert_eq!(
            role_of_session(&owned.app, &session.unwrap())
                .await
                .as_deref(),
            Some("user")
        );
    }

    #[tokio::test]
    async fn refused_password_change_keeps_earlier_sessions_and_the_old_password() {
        let owned = router_with_owner().await;
        let mut wrong = Vec::new();
        for door in [PasswordDoor::OwnProfile, PasswordDoor::AdminEdit] {
            let attempt = set_password_through(&owned, door, "abcdefg").await;
            if attempt.earlier_session_works != Some(true) {
                wrong.push(format!(
                    "{door:?}: the session issued before the refused change stopped working (status {})",
                    attempt.status
                ));
            }
            if attempt.old_password_signs_in != Some(true) {
                wrong.push(format!("{door:?}: the old password no longer signs in"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    // Health, the admin health list, config warnings and trusted proxies,
    // through the real router with `config.toml` read from a temporary folder.

    use crate::config::AppConfig;
    use std::net::Ipv6Addr;

    /// Reads `{dir}/config.toml` with the startup loader's steps: an absent or
    /// blank file is the default config; otherwise the TOML is read into
    /// `AppConfig` and then checked by `validate_config`.
    fn load_config_from(dir: &std::path::Path) -> Result<AppConfig, String> {
        crate::config::load_config(dir).map(|(config, _unknown_keys)| config)
    }

    /// Writes `text` as `config.toml` into a new temporary folder (no file
    /// when `None`) and loads it.
    fn loaded_config(text: Option<&str>) -> AppConfig {
        let dir = tempfile::tempdir().unwrap();
        if let Some(text) = text {
            std::fs::write(dir.path().join("config.toml"), text).unwrap();
        }
        load_config_from(dir.path()).expect("config.toml loads")
    }

    /// The real router over a fresh real database, with `config` as the
    /// state's loaded config. No account exists yet.
    async fn router_with_config(
        config: AppConfig,
    ) -> (Router, livrarr_db::sqlite::SqliteDb, tempfile::TempDir) {
        let (mut state, tmp) = test_app_state_with_setup_token(KNOWN_SETUP_TOKEN).await;
        state.config = Arc::new(config);
        let db = state.db.clone();
        let ui_dir = state.data_dir.join("ui-not-present-in-test");
        (build_router(state, ui_dir), db, tmp)
    }

    static NEXT_QUIET_PEER: AtomicU32 = AtomicU32::new(1);

    /// Where an app's own requests (setup, sign-in, admin reads) come from.
    #[derive(Clone, Copy)]
    enum ControlPeers {
        /// A new address in 100.64.0.0/10 for each request.
        V4,
        /// A new address in 2001:db8:ffff::/48 for each request.
        V6,
        /// One address for every request.
        Fixed(SocketAddr),
    }

    impl ControlPeers {
        fn next(self) -> SocketAddr {
            let n = NEXT_QUIET_PEER.fetch_add(1, Ordering::Relaxed);
            match self {
                ControlPeers::V4 => {
                    let [_, _, c, d] = n.to_be_bytes();
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(100, 64 + (c % 64), d, 1)), 42000)
                }
                ControlPeers::V6 => SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::new(
                        0x2001,
                        0xdb8,
                        0xffff,
                        0,
                        0,
                        0,
                        (n >> 16) as u16,
                        n as u16,
                    )),
                    42000,
                ),
                ControlPeers::Fixed(addr) => addr,
            }
        }
    }

    fn caller_headers(caller: Caller<'_>) -> Vec<(&'static str, String)> {
        match caller {
            Caller::SignedOut => Vec::new(),
            Caller::Session(token) => vec![("authorization", format!("Bearer {token}"))],
            Caller::ApiKey(key) => vec![("x-api-key", key.to_string())],
        }
    }

    /// The real router after first-run setup, with `config` loaded, an admin
    /// (the owner) and a signed-in normal user.
    struct ConfiguredApp {
        app: Router,
        db: livrarr_db::sqlite::SqliteDb,
        peers: ControlPeers,
        owner_session: String,
        owner_api_key: String,
        reader_session: String,
        _tmp: tempfile::TempDir,
    }

    impl ConfiguredApp {
        async fn new(config: AppConfig, peers: ControlPeers) -> Self {
            let (app, db, tmp) = router_with_config(config).await;
            let (status, body) = call(
                &app,
                peers.next(),
                "POST",
                "/api/v1/setup",
                &[],
                Some(serde_json::json!({
                    "username": OWNER,
                    "password": OWNER_PASSWORD,
                    "setupToken": KNOWN_SETUP_TOKEN,
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "control, owner setup: {body}");
            let reply = json_of(&body);
            let owner_session = reply["token"].as_str().unwrap().to_string();
            let owner_api_key = reply["apiKey"].as_str().unwrap().to_string();

            let bearer = format!("Bearer {owner_session}");
            let (status, body) = call(
                &app,
                peers.next(),
                "POST",
                "/api/v1/user",
                &[("authorization", bearer.as_str())],
                Some(serde_json::json!({
                    "username": "reader",
                    "password": "reader-password-1",
                    "role": "user",
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "control, create reader: {body}");
            let (status, body) = call(
                &app,
                peers.next(),
                "POST",
                "/api/v1/auth/login",
                &[],
                Some(serde_json::json!({
                    "username": "reader",
                    "password": "reader-password-1",
                    "rememberMe": false,
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "control, reader signs in: {body}");
            let reader_session = json_of(&body)["token"].as_str().unwrap().to_string();

            Self {
                app,
                db,
                peers,
                owner_session,
                owner_api_key,
                reader_session,
                _tmp: tmp,
            }
        }

        async fn send(&self, caller: Caller<'_>, method: &str, uri: &str) -> (StatusCode, String) {
            let headers = caller_headers(caller);
            let headers: Vec<(&str, &str)> = headers
                .iter()
                .map(|(name, value)| (*name, value.as_str()))
                .collect();
            call(&self.app, self.peers.next(), method, uri, &headers, None).await
        }

        /// `GET /api/v1/system/health` as the admin, by session.
        async fn admin_health(&self) -> (StatusCode, String) {
            self.send(
                Caller::Session(&self.owner_session),
                "GET",
                "/api/v1/system/health",
            )
            .await
        }
    }

    fn items_of(body: &str) -> Vec<serde_json::Value> {
        json_of(body).as_array().cloned().unwrap_or_default()
    }

    /// The `(checkType, message)` of every item whose source is `config`.
    fn config_rows(body: &str) -> Vec<(String, String)> {
        items_of(body)
            .iter()
            .filter(|item| item["source"] == "config")
            .map(|item| {
                (
                    item["checkType"].as_str().unwrap_or_default().to_string(),
                    item["message"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    fn warning_row(message: &str) -> (String, String) {
        ("warning".to_string(), message.to_string())
    }

    fn proxy_warning(entry: &str) -> String {
        format!(
            "Ignored [server] trusted_proxies entry \"{entry}\": use an IP address or range \
             such as 172.18.0.0/16; host names and ports are not supported"
        )
    }

    /// The public reply when the database check fails.
    fn minimal_failure_body() -> serde_json::Value {
        serde_json::json!([{
            "source": "database",
            "checkType": "error",
            "message": "database check failed",
        }])
    }

    /// Records a finding unless the admin health reply is a 200 whose first
    /// item is the database check.
    fn check_admin_reply_shape(wrong: &mut Vec<String>, who: &str, status: StatusCode, body: &str) {
        if status != StatusCode::OK {
            wrong.push(format!(
                "{who}: GET /api/v1/system/health answered {status}, expected 200; body {body:?}"
            ));
            return;
        }
        let items = items_of(body);
        if items.first().map(|item| &item["source"]) != Some(&serde_json::json!("database")) {
            wrong.push(format!(
                "{who}: first item is not the database check: {body}"
            ));
        }
    }

    /// One request through the real router as its own task, so a panic in
    /// the request path is returned as `Err(panic message)`.
    async fn send_catching(
        app: &Router,
        from: SocketAddr,
        uri: &str,
        headers: &[(&str, &str)],
        body: serde_json::Value,
    ) -> Result<StatusCode, String> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(Body::from(body.to_string())).unwrap();
        request.extensions_mut().insert(ConnectInfo(from));
        let app = app.clone();
        let joined =
            tokio::spawn(async move { app.oneshot(request).await.unwrap().status() }).await;
        joined.map_err(|e| {
            if !e.is_panic() {
                return e.to_string();
            }
            let payload = e.into_panic();
            payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic".to_string())
        })
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Bucket {
        /// The sixth failed login, with client B, was not answered 429.
        Trusted,
        /// The sixth failed login, with client B, was answered 429.
        Untrusted,
    }

    /// Five failed logins from `peer` with `header` naming client `a`, then
    /// one naming client `b`. Each of the five must reach the login handler
    /// (401); the sixth decides whether the peer's header picked the key.
    async fn bucket_test(
        app: &Router,
        peer: SocketAddr,
        header: &str,
        a: &str,
        b: &str,
    ) -> Result<Bucket, String> {
        let login = serde_json::json!({
            "username": "bucket-probe",
            "password": "wrong-password-1",
            "rememberMe": false,
        });
        for attempt in 1..=5 {
            let status = send_catching(
                app,
                peer,
                "/api/v1/auth/login",
                &[(header, a)],
                login.clone(),
            )
            .await
            .map_err(|panic| format!("{header} from {peer}: login {attempt} panicked: {panic}"))?;
            if status != StatusCode::UNAUTHORIZED {
                return Err(format!(
                    "{header} from {peer}: control, login {attempt} answered {status}, expected 401"
                ));
            }
        }
        let status = send_catching(app, peer, "/api/v1/auth/login", &[(header, b)], login)
            .await
            .map_err(|panic| format!("{header} from {peer}: login 6 panicked: {panic}"))?;
        match status {
            StatusCode::TOO_MANY_REQUESTS => Ok(Bucket::Untrusted),
            StatusCode::UNAUTHORIZED => Ok(Bucket::Trusted),
            other => Err(format!(
                "{header} from {peer}: login 6 answered {other}, expected 401 or 429"
            )),
        }
    }

    fn v4(a: u8, b: u8, c: u8, d: u8) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d)), 43000)
    }

    fn v6(text: &str) -> SocketAddr {
        SocketAddr::new(text.parse::<IpAddr>().unwrap(), 43000)
    }

    /// Runs one bucket test on a fresh router built from `config` and records
    /// a finding unless it gives `expected`.
    #[allow(clippy::too_many_arguments)]
    async fn expect_bucket(
        wrong: &mut Vec<String>,
        label: &str,
        config: &AppConfig,
        peer: SocketAddr,
        header: &str,
        a: &str,
        b: &str,
        expected: Bucket,
    ) {
        let (app, _db, _tmp) = router_with_config(config.clone()).await;
        match bucket_test(&app, peer, header, a, b).await {
            Ok(outcome) if outcome == expected => {}
            Ok(outcome) => wrong.push(format!("{label}: {outcome:?}, expected {expected:?}")),
            Err(reason) => wrong.push(format!("{label}: expected {expected:?}; {reason}")),
        }
    }

    const DATABASE_FAILED_PREFIX: &str = "database check failed: ";

    // REQ-101, REQ-103: the public health route.

    #[tokio::test]
    async fn public_health_answers_database_ok_to_every_caller() {
        let owned = ConfiguredApp::new(loaded_config(None), ControlPeers::V4).await;
        let mut wrong = Vec::new();
        for (who, caller) in [
            ("signed out", Caller::SignedOut),
            ("normal user", Caller::Session(&owned.reader_session)),
            ("admin", Caller::Session(&owned.owner_session)),
        ] {
            let (status, body) = owned.send(caller, "GET", "/api/v1/health").await;
            let items = items_of(&body);
            if status != StatusCode::OK
                || items.len() != 1
                || items[0]["source"] != "database"
                || items[0]["checkType"] != "ok"
            {
                wrong.push(format!(
                    "{who}: {status} {body}, expected 200 with one database/ok item"
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn public_health_answers_503_when_the_pool_is_closed() {
        let (app, db, _tmp) = router_with_config(loaded_config(None)).await;
        db.pool().close().await;
        assert!(db.pool().is_closed(), "control: the pool is closed");

        let (status, body) = call(&app, fresh_peer(), "GET", "/api/v1/health", &[], None).await;
        let items = items_of(&body);
        let mut wrong = Vec::new();
        if status != StatusCode::SERVICE_UNAVAILABLE {
            wrong.push(format!("status {status}, expected 503; body {body}"));
        }
        if items.len() != 1
            || items[0]["checkType"] != "error"
            || items[0]["message"] != "database check failed"
        {
            wrong.push(format!(
                "body {body}, expected one error item \"database check failed\""
            ));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn public_health_answers_503_within_three_seconds_when_no_connection_is_free() {
        let (app, db, _tmp) = router_with_config(loaded_config(None)).await;
        let held = db
            .pool()
            .acquire()
            .await
            .expect("hold the pool's only connection");
        assert_eq!(
            db.pool().num_idle(),
            0,
            "control: no idle connection remains"
        );

        let started = std::time::Instant::now();
        let reply = tokio::time::timeout(
            Duration::from_secs(10),
            call(&app, fresh_peer(), "GET", "/api/v1/health", &[], None),
        )
        .await;
        let elapsed = started.elapsed();
        drop(held);

        let (status, body) = reply.expect("the health reply arrives within 10 seconds");
        let mut wrong = Vec::new();
        if status != StatusCode::SERVICE_UNAVAILABLE {
            wrong.push(format!("status {status}, expected 503; body {body}"));
        }
        if elapsed >= Duration::from_secs(3) {
            wrong.push(format!(
                "the reply took {elapsed:?}, expected under 3 seconds"
            ));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn public_health_head_answers_the_get_status_with_an_empty_body() {
        let (app, db, _tmp) = router_with_config(loaded_config(None)).await;
        let mut wrong = Vec::new();
        for (case, expected) in [
            ("healthy", StatusCode::OK),
            ("pool closed", StatusCode::SERVICE_UNAVAILABLE),
        ] {
            if expected == StatusCode::SERVICE_UNAVAILABLE {
                db.pool().close().await;
            }
            let (get_status, _) =
                call(&app, fresh_peer(), "GET", "/api/v1/health", &[], None).await;
            let (head_status, head_body) =
                call(&app, fresh_peer(), "HEAD", "/api/v1/health", &[], None).await;
            if get_status != expected {
                wrong.push(format!(
                    "{case}: GET answered {get_status}, expected {expected}"
                ));
            }
            if head_status != get_status || head_status != expected {
                wrong.push(format!(
                    "{case}: HEAD answered {head_status}, GET {get_status}, expected both {expected}"
                ));
            }
            if !head_body.is_empty() {
                wrong.push(format!("{case}: HEAD body {head_body:?}, expected empty"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn public_health_never_carries_config_warnings() {
        let config = loaded_config(Some(
            "zz_marker_615 = 1\n[server]\ntrusted_proxies = ['nginx-marker-615']\n",
        ));
        assert_eq!(
            config.server.trusted_proxies,
            vec!["nginx-marker-615".to_string()],
            "control: the config loaded"
        );
        let owned = ConfiguredApp::new(config, ControlPeers::V4).await;
        let mut wrong = Vec::new();
        for (who, caller) in [
            ("signed out", Caller::SignedOut),
            ("normal user", Caller::Session(&owned.reader_session)),
            ("admin", Caller::Session(&owned.owner_session)),
        ] {
            let (status, body) = owned.send(caller, "GET", "/api/v1/health").await;
            if status != StatusCode::OK {
                wrong.push(format!("{who}: control, status {status}, expected 200"));
            }
            for marker in ["nginx-marker-615", "zz_marker_615"] {
                if body.contains(marker) {
                    wrong.push(format!("{who}: body carries {marker}: {body}"));
                }
            }
            if items_of(&body).len() != 1 {
                wrong.push(format!("{who}: expected one item, got {body}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn public_health_answers_the_minimal_503_while_schema_reads_are_refused() {
        let (app, db, _tmp) = router_with_config(loaded_config(None)).await;

        livrarr_db::test_helpers::set_schema_reads_refused(&db, true).await;
        let refusal = schema_read_error(&db).await;
        let (status, body) = call(&app, fresh_peer(), "GET", "/api/v1/health", &[], None).await;
        livrarr_db::test_helpers::set_schema_reads_refused(&db, false).await;
        let (after_status, after_body) =
            call(&app, fresh_peer(), "GET", "/api/v1/health", &[], None).await;

        assert!(
            refusal
                .as_deref()
                .is_some_and(|e| e.contains("not authorized")),
            "control: SQLite refuses the schema read under the fixture, got {refusal:?}"
        );
        let mut wrong = Vec::new();
        if status != StatusCode::SERVICE_UNAVAILABLE || json_of(&body) != minimal_failure_body() {
            wrong.push(format!(
                "under refusal: {status} {body}, expected 503 {}",
                minimal_failure_body()
            ));
        }
        let after = items_of(&after_body);
        if after_status != StatusCode::OK
            || after.len() != 1
            || after[0]["source"] != "database"
            || after[0]["checkType"] != "ok"
        {
            wrong.push(format!(
                "after removal: {after_status} {after_body}, expected 200 with database/ok"
            ));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// The error SQLite gives for a read of the schema table, or `None` when
    /// the read succeeds.
    async fn schema_read_error(db: &livrarr_db::sqlite::SqliteDb) -> Option<String> {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sqlite_schema")
            .fetch_one(db.pool())
            .await
            .err()
            .map(|e| e.to_string())
    }

    // REQ-102: the admin health list.

    #[tokio::test]
    async fn admin_health_answers_the_admin_by_session_and_by_api_key() {
        let owned = ConfiguredApp::new(loaded_config(None), ControlPeers::V4).await;
        let mut wrong = Vec::new();
        for (who, caller) in [
            ("admin by session", Caller::Session(&owned.owner_session)),
            ("admin by API key", Caller::ApiKey(&owned.owner_api_key)),
        ] {
            let (status, body) = owned.send(caller, "GET", "/api/v1/system/health").await;
            let items = items_of(&body);
            if status != StatusCode::OK
                || items.first().map(|i| (&i["source"], &i["checkType"]))
                    != Some((&serde_json::json!("database"), &serde_json::json!("ok")))
            {
                wrong.push(format!(
                    "{who}: {status} {body:?}, expected 200 with database/ok first"
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn admin_health_refuses_a_normal_user_and_a_signed_out_caller() {
        let owned = ConfiguredApp::new(loaded_config(None), ControlPeers::V4).await;
        let mut wrong = Vec::new();
        for (who, caller, expected) in [
            (
                "normal user",
                Caller::Session(&owned.reader_session),
                StatusCode::FORBIDDEN,
            ),
            ("signed out", Caller::SignedOut, StatusCode::UNAUTHORIZED),
        ] {
            let (status, body) = owned.send(caller, "GET", "/api/v1/system/health").await;
            if status != expected {
                wrong.push(format!("{who}: {status} {body:?}, expected {expected}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn admin_health_shows_the_database_error_and_still_lists_config_warnings() {
        let config = loaded_config(Some("[server]\ntrusted_proxies = ['nginx']\n"));
        let owned = ConfiguredApp::new(config, ControlPeers::V4).await;
        let mut wrong = Vec::new();

        livrarr_db::test_helpers::set_schema_reads_refused(&owned.db, true).await;
        let refusal = schema_read_error(&owned.db).await;
        let (me_status, me_body) = owned
            .send(
                Caller::Session(&owned.owner_session),
                "GET",
                "/api/v1/auth/me",
            )
            .await;
        let (status, body) = owned.admin_health().await;
        let (public_status, public_body) =
            owned.send(Caller::SignedOut, "GET", "/api/v1/health").await;
        livrarr_db::test_helpers::set_schema_reads_refused(&owned.db, false).await;
        let restored = schema_read_error(&owned.db).await;
        let (after_status, after_body) = owned.admin_health().await;

        assert!(
            refusal
                .as_deref()
                .is_some_and(|e| e.contains("not authorized")),
            "control: SQLite refuses the schema read under the fixture, got {refusal:?}"
        );
        assert_eq!(
            me_status,
            StatusCode::OK,
            "control: the admin's session still passes the login check under the fixture: {me_body}"
        );
        assert_eq!(
            restored, None,
            "control: the schema read succeeds after removal"
        );

        let items = items_of(&body);
        if status != StatusCode::OK {
            wrong.push(format!(
                "admin under refusal: {status} {body:?}, expected 200"
            ));
        }
        match items.first() {
            Some(first)
                if first["source"] == "database"
                    && first["checkType"] == "error"
                    && first["message"].as_str().is_some_and(|m| {
                        m.starts_with(DATABASE_FAILED_PREFIX)
                            && m.len() > DATABASE_FAILED_PREFIX.len()
                    }) => {}
            other => wrong.push(format!(
                "admin under refusal: first item {other:?}, expected database/error \
                 \"{DATABASE_FAILED_PREFIX}<detail>\""
            )),
        }
        let expected_second = serde_json::json!({
            "source": "config",
            "checkType": "warning",
            "message": proxy_warning("nginx"),
        });
        if items.get(1) != Some(&expected_second) {
            wrong.push(format!(
                "admin under refusal: second item {:?}, expected {expected_second}",
                items.get(1)
            ));
        }
        if public_status != StatusCode::SERVICE_UNAVAILABLE
            || json_of(&public_body) != minimal_failure_body()
        {
            wrong.push(format!(
                "signed out under refusal: {public_status} {public_body}, expected 503 {}",
                minimal_failure_body()
            ));
        }
        let after = items_of(&after_body);
        if after_status != StatusCode::OK
            || after.first().map(|i| &i["checkType"]) != Some(&serde_json::json!("ok"))
        {
            wrong.push(format!(
                "admin after removal: {after_status} {after_body:?}, expected database/ok first"
            ));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    // REQ-601 to REQ-603, REQ-701: config warnings and trusted proxies.

    /// Every proxy entry shape. `10.0.0.1` comes before both `/0` ranges so a
    /// request from 10.0.0.1 is matched before either is tried.
    const EVERY_PROXY_SHAPE: &str = "[server]\ntrusted_proxies = ['', ' 10.0.0.5 ', '10.0.0.1', \
        '10.0.0.0/8', '::1', 'fd00::/8', '0.0.0.0/0', '::/0', '10.0.0.0/33', 'fd00::/129', \
        '10.0.0.0/', 'nginx', '10.0.0.1:443', '[::1]:443', '10.0.0.1']\n";

    const REJECTED_PROXY_SHAPES: [&str; 7] = [
        "",
        "10.0.0.0/33",
        "fd00::/129",
        "10.0.0.0/",
        "nginx",
        "10.0.0.1:443",
        "[::1]:443",
    ];

    #[tokio::test]
    async fn every_rejected_proxy_entry_gets_one_warning_row_and_no_accepted_one_does() {
        let mut expected: Vec<(String, String)> = REJECTED_PROXY_SHAPES
            .iter()
            .map(|entry| warning_row(&proxy_warning(entry)))
            .collect();
        for (_, message) in &expected {
            assert_eq!(
                &livrarr_domain::cleanse_log_line(message),
                message,
                "control: the cleanser leaves the warning text unchanged"
            );
        }
        expected.sort();

        let owned = ConfiguredApp::new(
            loaded_config(Some(EVERY_PROXY_SHAPE)),
            ControlPeers::Fixed(v4(10, 0, 0, 1)),
        )
        .await;
        let empty = ConfiguredApp::new(
            loaded_config(Some("[server]\ntrusted_proxies = []\n")),
            ControlPeers::V4,
        )
        .await;

        let mut wrong = Vec::new();
        let (status, body) = owned.admin_health().await;
        check_admin_reply_shape(&mut wrong, "every shape", status, &body);
        let mut rows = config_rows(&body);
        rows.sort();
        if rows != expected {
            wrong.push(format!(
                "every shape: config rows {rows:#?}\nexpected {expected:#?}"
            ));
        }
        let (status, body) = empty.admin_health().await;
        check_admin_reply_shape(&mut wrong, "empty list", status, &body);
        if !config_rows(&body).is_empty() {
            wrong.push(format!("empty list: config rows in {body}"));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn config_with_every_proxy_shape_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), EVERY_PROXY_SHAPE).unwrap();
        let loaded = load_config_from(dir.path());
        assert!(loaded.is_ok(), "{loaded:?}");
    }

    #[tokio::test]
    async fn padded_proxy_entry_is_trusted_and_a_host_name_trusts_nothing() {
        let config = loaded_config(Some(
            "[server]\ntrusted_proxies = [' 10.0.0.5 ', 'nginx']\n",
        ));
        let mut wrong = Vec::new();
        expect_bucket(
            &mut wrong,
            "peer 10.0.0.5 (padded entry)",
            &config,
            v4(10, 0, 0, 5),
            "x-real-ip",
            "203.0.113.1",
            "203.0.113.2",
            Bucket::Trusted,
        )
        .await;
        expect_bucket(
            &mut wrong,
            "peer 10.0.0.9 (control, listed nowhere)",
            &config,
            v4(10, 0, 0, 9),
            "x-real-ip",
            "203.0.113.1",
            "203.0.113.2",
            Bucket::Untrusted,
        )
        .await;
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn proxy_warning_stays_until_the_config_is_fixed_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\ntrusted_proxies = ['nginx']\n").unwrap();
        let before = ConfiguredApp::new(
            load_config_from(dir.path()).expect("config.toml loads"),
            ControlPeers::V4,
        )
        .await;
        let expected = vec![warning_row(&proxy_warning("nginx"))];

        let mut wrong = Vec::new();
        for reply in ["first reply", "second reply"] {
            let (status, body) = before.admin_health().await;
            check_admin_reply_shape(&mut wrong, reply, status, &body);
            let rows = config_rows(&body);
            if rows != expected {
                wrong.push(format!(
                    "{reply}: config rows {rows:?}, expected {expected:?}"
                ));
            }
        }

        std::fs::write(&path, "[server]\ntrusted_proxies = []\n").unwrap();
        let after = ConfiguredApp::new(
            load_config_from(dir.path()).expect("config.toml loads"),
            ControlPeers::V4,
        )
        .await;
        let (status, body) = after.admin_health().await;
        check_admin_reply_shape(&mut wrong, "after restart", status, &body);
        if !config_rows(&body).is_empty() {
            wrong.push(format!("after restart: config rows remain in {body}"));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn admin_health_refuses_a_normal_user_while_warnings_are_configured() {
        let owned = ConfiguredApp::new(
            loaded_config(Some(
                "zz_marker_616 = 1\n[server]\ntrusted_proxies = ['nginx']\n",
            )),
            ControlPeers::V4,
        )
        .await;
        let (status, body) = owned
            .send(
                Caller::Session(&owned.reader_session),
                "GET",
                "/api/v1/system/health",
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "body {body:?}");
        assert!(
            !body.contains("nginx") && !body.contains("zz_marker_616"),
            "the refusal carries a warning: {body:?}"
        );
    }

    #[tokio::test]
    async fn over_wide_proxy_ranges_are_warned_about_and_trust_nothing() {
        let mut wrong = Vec::new();

        let config = loaded_config(Some(
            "[server]\ntrusted_proxies = ['10.0.0.0/33', 'fd00::/129']\n",
        ));
        let owned = ConfiguredApp::new(config, ControlPeers::V4).await;
        let (status, body) = owned.admin_health().await;
        check_admin_reply_shape(&mut wrong, "over-wide", status, &body);
        let mut rows = config_rows(&body);
        rows.sort();
        let mut expected = vec![
            warning_row(&proxy_warning("10.0.0.0/33")),
            warning_row(&proxy_warning("fd00::/129")),
        ];
        expected.sort();
        if rows != expected {
            wrong.push(format!(
                "over-wide: config rows {rows:?}, expected {expected:?}"
            ));
        }
        for (label, peer, a, b) in [
            (
                "over-wide, peer 10.0.0.0",
                v4(10, 0, 0, 0),
                "203.0.113.1",
                "203.0.113.2",
            ),
            (
                "over-wide, peer fd00::",
                v6("fd00::"),
                "2001:db8::1",
                "2001:db8::2",
            ),
        ] {
            match bucket_test(&owned.app, peer, "x-real-ip", a, b).await {
                Ok(Bucket::Untrusted) => {}
                other => wrong.push(format!("{label}: {other:?}, expected Untrusted")),
            }
        }

        let control = ConfiguredApp::new(
            loaded_config(Some(
                "[server]\ntrusted_proxies = ['10.0.0.0/8', 'fd00::/8']\n",
            )),
            ControlPeers::V4,
        )
        .await;
        let (status, body) = control.admin_health().await;
        check_admin_reply_shape(&mut wrong, "control ranges", status, &body);
        if !config_rows(&body).is_empty() {
            wrong.push(format!("control ranges: config rows in {body}"));
        }
        for (label, peer, a, b) in [
            (
                "control ranges, peer 10.0.0.0",
                v4(10, 0, 0, 0),
                "203.0.113.1",
                "203.0.113.2",
            ),
            (
                "control ranges, peer fd00::",
                v6("fd00::"),
                "2001:db8::1",
                "2001:db8::2",
            ),
        ] {
            match bucket_test(&control.app, peer, "x-real-ip", a, b).await {
                Ok(Bucket::Trusted) => {}
                other => wrong.push(format!("{label}: {other:?}, expected Trusted")),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// The admin reply for `config` holds no `config` row.
    async fn expect_no_config_rows(
        wrong: &mut Vec<String>,
        label: &str,
        config: &AppConfig,
        peers: ControlPeers,
    ) {
        let owned = ConfiguredApp::new(config.clone(), peers).await;
        let (status, body) = owned.admin_health().await;
        check_admin_reply_shape(wrong, label, status, &body);
        if !config_rows(&body).is_empty() {
            wrong.push(format!("{label}: config rows in {body}"));
        }
    }

    #[tokio::test]
    async fn ipv4_zero_range_trusts_every_ipv4_peer_and_no_ipv6_peer() {
        let config = loaded_config(Some("[server]\ntrusted_proxies = ['0.0.0.0/0']\n"));
        let mut wrong = Vec::new();
        expect_bucket(
            &mut wrong,
            "0.0.0.0/0, peer 192.0.2.7",
            &config,
            v4(192, 0, 2, 7),
            "x-real-ip",
            "203.0.113.1",
            "203.0.113.2",
            Bucket::Trusted,
        )
        .await;
        expect_bucket(
            &mut wrong,
            "0.0.0.0/0, peer 2001:db8::7",
            &config,
            v6("2001:db8::7"),
            "x-real-ip",
            "2001:db8::1",
            "2001:db8::2",
            Bucket::Untrusted,
        )
        .await;
        expect_no_config_rows(&mut wrong, "0.0.0.0/0", &config, ControlPeers::V6).await;
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn ipv6_zero_range_trusts_every_ipv6_peer_and_no_ipv4_peer() {
        let config = loaded_config(Some("[server]\ntrusted_proxies = ['::/0']\n"));
        let mut wrong = Vec::new();
        expect_bucket(
            &mut wrong,
            "::/0, peer 2001:db8::7",
            &config,
            v6("2001:db8::7"),
            "x-real-ip",
            "2001:db8::1",
            "2001:db8::2",
            Bucket::Trusted,
        )
        .await;
        expect_bucket(
            &mut wrong,
            "::/0, peer 192.0.2.7",
            &config,
            v4(192, 0, 2, 7),
            "x-real-ip",
            "203.0.113.1",
            "203.0.113.2",
            Bucket::Untrusted,
        )
        .await;
        expect_no_config_rows(&mut wrong, "::/0", &config, ControlPeers::V4).await;
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn zero_range_with_only_forwarded_for_falls_back_to_the_peer() {
        let mut wrong = Vec::new();
        for (range, peer, a, b, peers) in [
            (
                "0.0.0.0/0",
                v4(192, 0, 2, 7),
                "203.0.113.1",
                "203.0.113.2",
                ControlPeers::V6,
            ),
            (
                "::/0",
                v6("2001:db8::7"),
                "2001:db8::1",
                "2001:db8::2",
                ControlPeers::V4,
            ),
        ] {
            let config = loaded_config(Some(&format!("[server]\ntrusted_proxies = ['{range}']\n")));
            expect_bucket(
                &mut wrong,
                &format!("{range}, forwarded-for from {peer}"),
                &config,
                peer,
                "x-forwarded-for",
                a,
                b,
                Bucket::Untrusted,
            )
            .await;
            expect_bucket(
                &mut wrong,
                &format!("{range}, X-Real-IP from {peer} (control)"),
                &config,
                peer,
                "x-real-ip",
                a,
                b,
                Bucket::Trusted,
            )
            .await;
            expect_no_config_rows(&mut wrong, range, &config, peers).await;
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn unknown_keys_get_rows_by_name_and_never_show_a_value() {
        let config = loaded_config(Some(
            "nginx = 1\n[foo]\n[server]\napi_kye = \"sk-live-ac711Marker\"\n",
        ));
        let owned = ConfiguredApp::new(config, ControlPeers::V4).await;
        let (status, body) = owned.admin_health().await;
        let mut wrong = Vec::new();
        check_admin_reply_shape(&mut wrong, "unknown keys", status, &body);
        let rows = config_rows(&body);
        for key in ["foo", "nginx", "server.api_kye"] {
            let row = warning_row(&format!("Unknown config key: {key}"));
            if !rows.contains(&row) {
                wrong.push(format!("no row {row:?} in {rows:?}"));
            }
        }
        if body.contains("ac711Marker") {
            wrong.push(format!("the reply carries the value: {body}"));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn secret_shaped_unknown_key_name_is_masked_in_its_row() {
        let key = "x?apikey=ac712Marker";
        assert!(
            livrarr_domain::cleanse_log_line(&format!("Unknown config key: {key}"))
                .contains("[REDACTED]"),
            "control: the cleanser masks this key name"
        );
        let config = loaded_config(Some(&format!("\"{key}\" = 1\n")));
        let owned = ConfiguredApp::new(config, ControlPeers::V4).await;
        let (status, body) = owned.admin_health().await;
        let mut wrong = Vec::new();
        check_admin_reply_shape(&mut wrong, "secret-shaped key", status, &body);
        let rows: Vec<_> = config_rows(&body)
            .into_iter()
            .filter(|(_, message)| message.starts_with("Unknown config key: x?apikey="))
            .collect();
        if rows.len() != 1 || rows[0].0 != "warning" || !rows[0].1.contains("[REDACTED]") {
            wrong.push(format!(
                "expected one warning row for the key, masked with [REDACTED]; got {rows:?}"
            ));
        }
        if body.contains("ac712Marker") {
            wrong.push(format!("the reply carries the secret part: {body}"));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[tokio::test]
    async fn without_config_toml_the_admin_list_holds_only_the_database_item() {
        let owned = ConfiguredApp::new(loaded_config(None), ControlPeers::V4).await;
        let (status, body) = owned.admin_health().await;
        let items = items_of(&body);
        assert_eq!(status, StatusCode::OK, "body {body:?}");
        assert_eq!(items.len(), 1, "only the database item: {body}");
        assert_eq!(items[0]["source"], "database", "{body}");
    }

    #[tokio::test]
    async fn shared_database_check_reports_a_closed_pool_with_its_detail() {
        let (state, _tmp) = test_app_state_with_setup_token(KNOWN_SETUP_TOKEN).await;
        state.db.pool().close().await;
        assert!(state.db.pool().is_closed(), "control: the pool is closed");

        let item = livrarr_handlers::system::database_check(&state).await;

        assert_eq!(item.source, "database");
        assert_eq!(item.check_type, livrarr_domain::HealthCheckType::Error);
        assert!(
            item.message.starts_with(DATABASE_FAILED_PREFIX)
                && item.message.len() > DATABASE_FAILED_PREFIX.len(),
            "expected \"{DATABASE_FAILED_PREFIX}<detail>\", got {:?}",
            item.message
        );
    }
}
