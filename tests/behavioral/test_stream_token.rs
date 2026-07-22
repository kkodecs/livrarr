//! Behavioral suite for Unit C — scoped, user-bound, expiring stream token.
//!
//! The pure claims/signature logic (tamper, wrong-purpose, wrong-item,
//! expiry, replay-within-TTL, domain separation from the cover-proxy
//! signer) is unit-tested in `livrarr-handlers/src/stream_token.rs`
//! itself. This suite covers the door-to-road wiring those unit tests
//! can't: the mint route's authorization (an item must belong to the
//! caller before a token is signed), the stream route's cross-user
//! defense-in-depth (a token whose claims are internally consistent but
//! whose user doesn't actually own the item is still rejected via
//! `resolve_path`), and that HTTP range requests keep working under a
//! freshly minted token.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use tower::ServiceExt;

use livrarr_behavioral::stubs::{create_second_test_user, create_test_user};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateLibraryItemDbRequest, CreateWorkDbRequest, LibraryItemDb, RootFolderDb, TagStatus,
    UserDb, WorkDbCreate,
};
use livrarr_domain::{AuthType, MediaType, User};
use livrarr_handlers::context::{HasFileService, HasHmacKey};
use livrarr_handlers::stream_token::mint_stream_token;
use livrarr_handlers::{work, workfile, AuthContext};
use livrarr_library::file_service::FileServiceImpl;

const KEY: &[u8] = b"behavioral-test-hmac-key-32-byte";

#[derive(Clone)]
struct RouteState {
    file_service: Arc<FileServiceImpl<SqliteDb>>,
    hmac_key: Arc<Vec<u8>>,
}

impl HasFileService for RouteState {
    type FileSvc = FileServiceImpl<SqliteDb>;
    fn file_service(&self) -> &Self::FileSvc {
        &self.file_service
    }
}

impl HasHmacKey for RouteState {
    fn hmac_key(&self) -> &[u8] {
        &self.hmac_key
    }
}

fn route_state(db: SqliteDb) -> RouteState {
    RouteState {
        file_service: Arc::new(FileServiceImpl::new(db)),
        hmac_key: Arc::new(KEY.to_vec()),
    }
}

async fn auth_context(db: &SqliteDb, user_id: i64) -> AuthContext {
    let user: User = db.get_user(user_id).await.expect("seeded user exists");
    AuthContext {
        user,
        auth_type: AuthType::Session,
        session_token_hash: None,
    }
}

fn stream_token_app(state: RouteState) -> Router {
    Router::new()
        .route(
            "/workfile/{id}/stream-token",
            post(workfile::mint_stream_token_route::<RouteState>),
        )
        .route("/stream/{id}", get(work::stream::<RouteState>))
        .with_state(state)
}

/// Create (once per test) the root folder backing `root_dir`. A DB-level
/// uniqueness rule allows only one root folder per media type, so callers
/// seeding multiple items under the same directory must create the root
/// once and pass its id to every `seed_playable_item` call.
async fn ensure_root(db: &SqliteDb, root_dir: &std::path::Path) -> i64 {
    db.create_root_folder(root_dir.to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap()
        .id
}

/// Seed a work + library item under `root_folder_id`, with a real file on
/// disk at `root_dir.join(relative_path)`, so `resolve_path`'s canonicalize
/// and containment check succeeds exactly like production. Returns the
/// library item id.
async fn seed_playable_item(
    db: &SqliteDb,
    user_id: i64,
    root_folder_id: i64,
    root_dir: &std::path::Path,
    relative_path: &str,
    contents: &[u8],
) -> i64 {
    std::fs::create_dir_all(root_dir.join(relative_path).parent().unwrap()).unwrap();
    std::fs::write(root_dir.join(relative_path), contents).unwrap();

    let (work_row, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Stream Token Test Book".into(),
            author_name: "Test Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work_row.id,
            root_folder_id,
            path: relative_path.into(),
            media_type: MediaType::Audiobook,
            file_size: contents.len() as i64,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();
    item.id
}

// ---------------------------------------------------------------------------
// Mint route: authorization before signing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mint_route_rejects_item_not_owned_by_caller() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let tmp = tempfile::tempdir().unwrap();
    let root = ensure_root(&db, tmp.path()).await;
    let item_a =
        seed_playable_item(&db, user_a, root, tmp.path(), "book.m4b", b"audio-bytes").await;

    let app = stream_token_app(route_state(db.clone()));
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/workfile/{item_a}/stream-token"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(auth_context(&db, user_b).await);

    let resp = app.oneshot(req).await.unwrap();
    // B does not own A's item — mint must refuse, never sign a token B could
    // use (or hand to anyone else) to stream A's book.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mint_route_returns_a_token_and_a_24h_expiry() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let tmp = tempfile::tempdir().unwrap();
    let root = ensure_root(&db, tmp.path()).await;
    let item_a =
        seed_playable_item(&db, user_a, root, tmp.path(), "book.m4b", b"audio-bytes").await;

    let app = stream_token_app(route_state(db.clone()));
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/workfile/{item_a}/stream-token"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(auth_context(&db, user_a).await);

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["token"].as_str().is_some_and(|t| !t.is_empty()));
    let exp = json["exp"].as_i64().expect("exp is a number");
    let now = Utc::now().timestamp();
    // ~24h out; generous tolerance for test wall-clock slop.
    assert!(exp > now + 23 * 3600 && exp < now + 25 * 3600);
}

// ---------------------------------------------------------------------------
// Stream route: cross-user, wrong-item, range requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_route_serves_a_freshly_minted_token_including_range_requests() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let tmp = tempfile::tempdir().unwrap();
    let contents = b"0123456789abcdefghij".to_vec();
    let root = ensure_root(&db, tmp.path()).await;
    let item_a = seed_playable_item(&db, user_a, root, tmp.path(), "book.m4b", &contents).await;

    let (token, _exp) = mint_stream_token(KEY, user_a, item_a, Utc::now());
    let app = stream_token_app(route_state(db.clone()));

    // Full-body request.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/stream/{item_a}?token={token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Range request under the SAME token — must keep working (replay within
    // TTL, across the byte-range serving the audio element actually uses).
    let req = Request::builder()
        .method("GET")
        .uri(format!("/stream/{item_a}?token={token}"))
        .header(header::RANGE, "bytes=0-4")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let content_range = resp
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_range.starts_with("bytes 0-4/"), "{content_range}");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], &contents[0..=4]);
}

#[tokio::test]
async fn stream_route_rejects_a_token_for_a_different_item_in_the_url() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let tmp = tempfile::tempdir().unwrap();
    let root = ensure_root(&db, tmp.path()).await;
    let item_1 = seed_playable_item(&db, user_a, root, tmp.path(), "one.m4b", b"one").await;
    let item_2 = seed_playable_item(&db, user_a, root, tmp.path(), "two.m4b", b"two").await;

    // A token minted for item_1 must not stream item_2, even though the
    // same user owns both — the token's own item_id claim is checked, not
    // just ownership.
    let (token, _exp) = mint_stream_token(KEY, user_a, item_1, Utc::now());
    let app = stream_token_app(route_state(db));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/stream/{item_2}?token={token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stream_route_rejects_cross_user_token_via_resolve_path() {
    // "B's token for A's item": the token's own claims are internally
    // consistent (constructed directly here, since the real mint endpoint
    // would refuse to issue it — see mint_route_rejects_item_not_owned_by_caller
    // above) but resolve_path's real, DB-backed ownership check is a
    // second, independent line of defense and must still reject it.
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let tmp = tempfile::tempdir().unwrap();
    let root = ensure_root(&db, tmp.path()).await;
    let item_a =
        seed_playable_item(&db, user_a, root, tmp.path(), "book.m4b", b"audio-bytes").await;

    let (token, _exp) = mint_stream_token(KEY, user_b, item_a, Utc::now());
    let app = stream_token_app(route_state(db));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/stream/{item_a}?token={token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stream_route_rejects_missing_token() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let tmp = tempfile::tempdir().unwrap();
    let root = ensure_root(&db, tmp.path()).await;
    let item_a =
        seed_playable_item(&db, user_a, root, tmp.path(), "book.m4b", b"audio-bytes").await;

    let app = stream_token_app(route_state(db));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/stream/{item_a}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
