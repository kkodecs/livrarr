use axum::http::{HeaderValue, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::handlers;
use crate::middleware::auth_middleware;
use crate::state::AppState;

/// Build the Axum router.
///
/// Satisfies: RUNTIME-SERVER-005, RUNTIME-COMPOSE-003, RUNTIME-COMPOSE-004
pub fn build_router(state: AppState, ui_dir: std::path::PathBuf) -> Router {
    // Public API routes (no auth required).
    let public = Router::new()
        .route("/setup/status", get(handlers::setup::setup_status))
        .route("/setup", post(handlers::setup::setup))
        .route("/auth/login", post(handlers::auth::login));

    // Protected API routes (auth middleware applied).
    let protected = Router::new()
        // Auth
        .route("/auth/logout", post(handlers::auth::logout))
        .route("/auth/me", get(handlers::auth::me))
        .route("/auth/profile", put(handlers::profile::update_profile))
        .route("/auth/apikey", post(handlers::profile::regenerate_api_key))
        // Users (admin)
        .route(
            "/user",
            get(handlers::user::list).post(handlers::user::create),
        )
        .route(
            "/user/{id}",
            get(handlers::user::get)
                .put(handlers::user::update)
                .delete(handlers::user::delete),
        )
        .route(
            "/user/{id}/apikey",
            post(handlers::user::regenerate_user_api_key),
        )
        // Root folders
        .route(
            "/rootfolder",
            get(handlers::root_folder::list).post(handlers::root_folder::create),
        )
        .route("/rootfolder/{id}", delete(handlers::root_folder::delete))
        .route("/rootfolder/{id}/scan", post(handlers::root_folder::scan))
        // Unmapped file scan (arbitrary path)
        .route("/unmapped/scan", post(handlers::root_folder::scan_path))
        // Download clients
        .route(
            "/downloadclient",
            get(handlers::download_client::list).post(handlers::download_client::create),
        )
        .route(
            "/downloadclient/test",
            post(handlers::download_client::test),
        )
        .route(
            "/downloadclient/{id}",
            get(handlers::download_client::get)
                .put(handlers::download_client::update)
                .delete(handlers::download_client::delete),
        )
        .route(
            "/downloadclient/{id}/test",
            post(handlers::download_client::test_saved),
        )
        // Remote path mappings
        .route(
            "/remotepathmapping",
            get(handlers::remote_path_mapping::list).post(handlers::remote_path_mapping::create),
        )
        .route(
            "/remotepathmapping/{id}",
            get(handlers::remote_path_mapping::get)
                .put(handlers::remote_path_mapping::update)
                .delete(handlers::remote_path_mapping::delete),
        )
        // Config
        .route("/config/naming", get(handlers::config::get_naming))
        .route(
            "/config/mediamanagement",
            get(handlers::config::get_media_management)
                .put(handlers::config::update_media_management),
        )
        // Indexers (replaces /config/prowlarr — DEFERRED-001)
        .route(
            "/indexer",
            get(handlers::indexer::list).post(handlers::indexer::create),
        )
        .route("/indexer/test", post(handlers::indexer::test))
        .route(
            "/indexer/{id}",
            get(handlers::indexer::get)
                .put(handlers::indexer::update)
                .delete(handlers::indexer::delete),
        )
        .route("/indexer/{id}/test", post(handlers::indexer::test_saved))
        .route(
            "/config/metadata",
            get(handlers::config::get_metadata).put(handlers::config::update_metadata),
        )
        .route(
            "/config/metadata/test/hardcover",
            post(handlers::config::test_hardcover),
        )
        .route(
            "/config/metadata/test/audnexus",
            post(handlers::config::test_audnexus),
        )
        .route(
            "/config/metadata/test/llm",
            post(handlers::config::test_llm),
        )
        // Works
        .route("/work/lookup", get(handlers::work::lookup))
        .route("/work/refresh", post(handlers::work::refresh_all))
        .route("/work", get(handlers::work::list).post(handlers::work::add))
        .route(
            "/work/{id}",
            get(handlers::work::get)
                .put(handlers::work::update)
                .delete(handlers::work::delete),
        )
        .route("/work/{id}/cover", post(handlers::work::upload_cover))
        .route("/work/{id}/refresh", post(handlers::work::refresh))
        // Authors
        .route("/author/lookup", get(handlers::author::lookup))
        .route("/author/search", post(handlers::author::search))
        .route(
            "/author",
            get(handlers::author::list).post(handlers::author::add),
        )
        .route(
            "/author/{id}",
            get(handlers::author::get)
                .put(handlers::author::update)
                .delete(handlers::author::delete),
        )
        .route(
            "/author/{id}/bibliography",
            get(handlers::author::bibliography),
        )
        .route(
            "/author/{id}/bibliography/refresh",
            post(handlers::author::refresh_bibliography),
        )
        // Queue
        .route("/queue", get(handlers::queue::list))
        .route("/queue/{id}", delete(handlers::queue::remove))
        // Grabs
        .route("/grab/{id}/retry", post(handlers::queue::retry_import))
        // Releases
        .route("/release", get(handlers::release::search))
        .route("/release/grab", post(handlers::release::grab))
        // Notifications
        .route(
            "/notification",
            get(handlers::notification::list).delete(handlers::notification::dismiss_all),
        )
        .route(
            "/notification/{id}",
            put(handlers::notification::mark_read).delete(handlers::notification::dismiss),
        )
        // History
        .route("/history", get(handlers::history::list))
        // System
        .route("/health", get(handlers::system::health))
        .route("/system/status", get(handlers::system::status))
        // Filesystem browse
        .route("/filesystem", get(handlers::filesystem::browse))
        // Manual import
        .route("/manualimport/scan", post(handlers::manual_import::scan))
        .route(
            "/manualimport/import",
            post(handlers::manual_import::import),
        )
        .route(
            "/manualimport/search",
            post(handlers::manual_import::search),
        )
        // Library files
        .route("/workfile", get(handlers::workfile::list))
        .route(
            "/workfile/{id}",
            get(handlers::workfile::get).delete(handlers::workfile::delete),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Media cover serving (no auth — images loaded by browser directly).
    let mediacover = Router::new()
        .route(
            "/mediacover/{id}/cover.jpg",
            get(handlers::mediacover::get_cover),
        )
        .route(
            "/mediacover/{id}/thumb.jpg",
            get(handlers::mediacover::get_thumb),
        );

    // Combine API routes. Unmatched API paths return 404.
    let api = Router::new()
        .merge(public)
        .merge(protected)
        .merge(mediacover)
        .fallback(|| async { StatusCode::NOT_FOUND });

    let app = Router::new().nest("/api/v1", api);

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
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'; \
             base-uri 'self'; object-src 'none'; form-action 'self'",
        ),
    ))
    .with_state(state)
}
