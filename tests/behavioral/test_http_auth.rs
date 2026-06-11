use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

use librarr_db::pool::{create_sqlite_pool, run_migrations};
use librarr_db::sqlite::SqliteDb;
use librarr_server::auth_service::ServerAuthService;
use librarr_server::config::AppConfig;
use librarr_server::router::build_router;
use librarr_server::state::AppState;

async fn setup_app() -> (axum::Router, TempDir) {
    let dir = TempDir::new().unwrap();
    let pool = create_sqlite_pool(dir.path()).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let db = SqliteDb::new(pool);
    let auth_service = Arc::new(ServerAuthService::new(db.clone()));
    let http_client = librarr_http::HttpClient::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let state = AppState {
        db,
        auth_service,
        http_client,
        config: Arc::new(AppConfig::default()),
        data_dir: Arc::new(dir.path().to_path_buf()),
        startup_time: chrono::Utc::now(),
        job_runner: None,
    };
    (build_router(state, dir.path().join("ui")), dir)
}

async fn json_body(response: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn assert_error_shape(body: &Value, expected_status: u16) {
    assert_eq!(body["status"], json!(expected_status));
    assert!(body["error"].is_string(), "error should be a string");
    assert!(body["message"].is_string(), "message should be a string");
    // fieldErrors is optional — only present for validation errors
}

fn assert_hex_64(value: &Value) {
    let s = value.as_str().expect("expected string");
    assert_eq!(s.len(), 64, "expected 64-char hex string, got {}", s.len());
    assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
}

async fn post_setup(
    app: &axum::Router,
    username: &str,
    password: &str,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "username": username, "password": password }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn post_login(
    app: &axum::Router,
    username: &str,
    password: &str,
    remember_me: bool,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "username": username, "password": password, "rememberMe": remember_me })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn get_me(app: &axum::Router, token: Option<&str>) -> axum::http::Response<Body> {
    let mut builder = Request::builder().method("GET").uri("/api/v1/auth/me");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn post_logout(app: &axum::Router, token: Option<&str>) -> axum::http::Response<Body> {
    let mut builder = Request::builder().method("POST").uri("/api/v1/auth/logout");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// REQ-ID: RUNTIME-SERVER-005
#[tokio::test]
async fn setup_status_returns_true_on_fresh_db() {
    let (app, _dir) = setup_app().await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/setup/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body, json!({ "setupRequired": true }));
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn setup_status_returns_false_after_setup() {
    let (app, _dir) = setup_app().await;

    let setup_response = post_setup(&app, "admin", "password123").await;
    assert_eq!(setup_response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/setup/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body, json!({ "setupRequired": false }));
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn setup_returns_api_key_and_token() {
    let (app, _dir) = setup_app().await;

    let response = post_setup(&app, "admin", "password123").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert!(body.get("apiKey").is_some());
    assert!(body.get("token").is_some());
    assert_hex_64(&body["apiKey"]);
    assert_hex_64(&body["token"]);
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn setup_returns_409_if_called_again() {
    let (app, _dir) = setup_app().await;

    let first = post_setup(&app, "admin", "password123").await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = post_setup(&app, "admin2", "anotherpass").await;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let body = json_body(second).await;
    assert_error_shape(&body, 409);
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn setup_returns_422_for_short_username() {
    let (app, _dir) = setup_app().await;

    let response = post_setup(&app, "ab", "password123").await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = json_body(response).await;
    assert_error_shape(&body, 422);
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn setup_returns_422_for_short_password() {
    let (app, _dir) = setup_app().await;

    let response = post_setup(&app, "admin", "12345").await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = json_body(response).await;
    assert_error_shape(&body, 422);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn login_returns_token_for_valid_credentials() {
    let (app, _dir) = setup_app().await;
    post_setup(&app, "admin", "password123").await;

    let response = post_login(&app, "admin", "password123", false).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert!(body.get("token").is_some());
    assert_hex_64(&body["token"]);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn login_returns_401_for_wrong_password() {
    let (app, _dir) = setup_app().await;
    post_setup(&app, "admin", "password123").await;

    let response = post_login(&app, "admin", "wrongpassword", false).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn login_returns_401_for_nonexistent_user() {
    let (app, _dir) = setup_app().await;
    post_setup(&app, "admin", "password123").await;

    let response = post_login(&app, "nobody", "password123", false).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn auth_me_returns_user_for_valid_bearer() {
    let (app, _dir) = setup_app().await;
    post_setup(&app, "admin", "password123").await;

    let login_resp = post_login(&app, "admin", "password123", false).await;
    let login_body = json_body(login_resp).await;
    let token = login_body["token"].as_str().unwrap();

    let response = get_me(&app, Some(token)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["authType"], json!("session"));
    assert!(body["user"].is_object());
    assert!(body["user"]["id"].is_number());
    assert_eq!(body["user"]["username"], json!("admin"));
    assert_eq!(body["user"]["role"], json!("admin"));
    assert!(body["user"]["createdAt"].is_string());
    assert!(body["user"]["updatedAt"].is_string());
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn auth_me_returns_401_without_token() {
    let (app, _dir) = setup_app().await;

    let response = get_me(&app, None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// REQ-ID: AUTH-006
#[tokio::test]
async fn logout_invalidates_token() {
    let (app, _dir) = setup_app().await;
    post_setup(&app, "admin", "password123").await;

    let login_resp = post_login(&app, "admin", "password123", false).await;
    let login_body = json_body(login_resp).await;
    let token = login_body["token"].as_str().unwrap();

    // Token works before logout
    let me_before = get_me(&app, Some(token)).await;
    assert_eq!(me_before.status(), StatusCode::OK);

    // Logout
    let logout_resp = post_logout(&app, Some(token)).await;
    assert_eq!(logout_resp.status(), StatusCode::OK);

    // Token invalid after logout
    let me_after = get_me(&app, Some(token)).await;
    assert_eq!(me_after.status(), StatusCode::UNAUTHORIZED);
}
