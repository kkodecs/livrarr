#![allow(dead_code, unused_imports)]

//! Behavioral tests for GrabService trait (SVC-GRAB-001..002).
//! Covers: fn.grab_service.{list, get, remove}

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateDownloadClientDbRequest, CreateGrabDbRequest, CreateUserDbRequest, CreateWorkDbRequest,
    DownloadClientDb, GrabDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_download::grab_service::GrabServiceImpl;

async fn setup_user(db: &SqliteDb) -> i64 {
    let user = db
        .create_user(CreateUserDbRequest {
            username: "testuser".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "testhash".into(),
        })
        .await
        .unwrap();
    user.id
}

async fn setup_second_user(db: &SqliteDb) -> i64 {
    let user = db
        .create_user(CreateUserDbRequest {
            username: "otheruser".into(),
            password_hash: "hash".into(),
            role: UserRole::User,
            api_key_hash: "testhash2".into(),
        })
        .await
        .unwrap();
    user.id
}

/// Create a download client and work, return (client_id, work_id).
async fn setup_grab_prereqs(db: &SqliteDb, user_id: i64) -> (i64, i64) {
    let client = db
        .create_download_client(CreateDownloadClientDbRequest {
            name: "test-qbit".into(),
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost".into(),
            port: 8080,
            use_ssl: false,
            skip_ssl_validation: false,
            url_base: None,
            username: None,
            password: None,
            category: "livrarr".into(),
            download_dir: None,
            enabled: true,
            api_key: None,
        })
        .await
        .unwrap();

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Test Book".into(),
            author_name: "Test Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    (client.id, work.id)
}

/// Seed a grab into the DB, return the Grab.
async fn seed_grab(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
    download_client_id: i64,
    status: GrabStatus,
    guid_suffix: &str,
) -> Grab {
    db.upsert_grab(CreateGrabDbRequest {
        user_id,
        work_id,
        download_client_id,
        title: format!("Grab {guid_suffix}"),
        indexer: "test-indexer".into(),
        guid: format!("guid-{guid_suffix}"),
        size: Some(1024),
        download_url: format!("magnet:?xt=urn:btih:abc{guid_suffix}"),
        download_id: Some(format!("hash-{guid_suffix}")),
        status,
        media_type: None,
    })
    .await
    .unwrap()
}

// =============================================================================
// list
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: requires download client integration for progress"]
async fn test_grab_list_active_with_reachable_client_returns_progress() {
    // SVC-GRAB-002: Given active grabs with reachable client, returns progress
    todo!("Requires download client stub that returns progress for active grabs")
}

#[tokio::test]
#[ignore = "pk-implement: requires download client integration for progress"]
async fn test_grab_list_unreachable_client_returns_grabs_without_progress() {
    // SVC-GRAB-002: Given unreachable client, returns grabs with progress=None
    todo!("Requires download client stub that fails on progress lookup")
}

#[tokio::test]
async fn test_grab_list_filter_by_status() {
    // SVC-GRAB-001: Given filter by status, returns only matching grabs
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_grab_prereqs(&db, user_id).await;

    // Seed grabs with different statuses
    seed_grab(&db, user_id, work_id, client_id, GrabStatus::Sent, "a").await;
    seed_grab(&db, user_id, work_id, client_id, GrabStatus::Confirmed, "b").await;
    seed_grab(&db, user_id, work_id, client_id, GrabStatus::Imported, "c").await;

    // Also seed a grab for another user
    let user_b = setup_second_user(&db).await;
    let (work_b, _) = db
        .create_work(CreateWorkDbRequest {
            user_id: user_b,
            title: "Other Book".into(),
            author_name: "Other Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    seed_grab(&db, user_b, work_b.id, client_id, GrabStatus::Sent, "d").await;

    let svc = GrabServiceImpl::new(db);

    // Filter by Sent status
    let result = svc
        .list(
            user_id,
            GrabFilter {
                status: Some(GrabStatus::Sent),
                page: None,
                per_page: None,
            },
        )
        .await;

    let items = result.expect("list should succeed");
    assert_eq!(items.len(), 1, "should return only Sent grabs for user");
    assert_eq!(items[0].grab.status, GrabStatus::Sent);
    assert_eq!(items[0].grab.user_id, user_id);
    assert!(
        items[0].progress.is_none(),
        "no client integration, progress should be None"
    );
}

#[tokio::test]
async fn test_grab_list_no_filter_returns_all_for_user() {
    // SVC-GRAB-001: No filter returns all grabs for user
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_grab_prereqs(&db, user_id).await;

    seed_grab(&db, user_id, work_id, client_id, GrabStatus::Sent, "a").await;
    seed_grab(&db, user_id, work_id, client_id, GrabStatus::Confirmed, "b").await;

    let svc = GrabServiceImpl::new(db);

    let result = svc
        .list(
            user_id,
            GrabFilter {
                status: None,
                page: None,
                per_page: None,
            },
        )
        .await;

    let items = result.expect("list should succeed");
    assert_eq!(items.len(), 2, "should return all grabs for user");
}

// =============================================================================
// get
// =============================================================================

#[tokio::test]
async fn test_grab_get_existing_returns_with_progress() {
    // SVC-GRAB-001: Given existing grab, returns it (progress=None without client)
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_grab_prereqs(&db, user_id).await;

    let grab = seed_grab(&db, user_id, work_id, client_id, GrabStatus::Sent, "x").await;

    let svc = GrabServiceImpl::new(db);
    let result = svc.get(user_id, grab.id).await;

    let item = result.expect("get should succeed");
    assert_eq!(item.grab.id, grab.id);
    assert_eq!(item.grab.user_id, user_id);
    assert_eq!(item.grab.title, "Grab x");
    assert!(
        item.progress.is_none(),
        "no client integration, progress should be None"
    );
}

#[tokio::test]
async fn test_grab_get_nonexistent_returns_not_found() {
    // SVC-GRAB-001: Given nonexistent grab, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let svc = GrabServiceImpl::new(db);
    let result = svc.get(user_id, 99999).await;

    assert!(
        matches!(result, Err(GrabServiceError::NotFound)),
        "expected NotFound"
    );
}

#[tokio::test]
async fn test_grab_get_wrong_user_returns_not_found() {
    // SVC-GRAB-001: Grab belonging to another user returns NotFound
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;
    let (client_id, work_id) = setup_grab_prereqs(&db, user_a).await;

    let grab = seed_grab(&db, user_a, work_id, client_id, GrabStatus::Sent, "y").await;

    let svc = GrabServiceImpl::new(db);
    let result = svc.get(user_b, grab.id).await;

    assert!(
        matches!(result, Err(GrabServiceError::NotFound)),
        "expected NotFound for wrong user"
    );
}

// =============================================================================
// remove
// =============================================================================

#[tokio::test]
async fn test_grab_remove_deletes_from_db() {
    // SVC-GRAB-001: Given existing grab, marks as removed in DB
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (client_id, work_id) = setup_grab_prereqs(&db, user_id).await;

    let grab = seed_grab(&db, user_id, work_id, client_id, GrabStatus::Sent, "r").await;
    let db2 = db.clone();

    let svc = GrabServiceImpl::new(db);
    let result = svc.remove(user_id, grab.id).await;
    assert!(result.is_ok(), "remove should succeed");

    // Verify the grab is now marked as Removed
    let after = db2.get_grab(user_id, grab.id).await.unwrap();
    assert_eq!(after.status, GrabStatus::Removed);
}

#[tokio::test]
#[ignore = "pk-implement: requires download client integration for client removal"]
async fn test_grab_remove_client_failure_still_removes_from_db() {
    // SVC-GRAB-001: Given client removal failure, still removes from DB
    todo!("Requires download client stub that fails on removal")
}

#[tokio::test]
async fn test_grab_remove_nonexistent_returns_not_found() {
    // SVC-GRAB-001: Given nonexistent grab, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let svc = GrabServiceImpl::new(db);
    let result = svc.remove(user_id, 99999).await;

    assert!(
        matches!(result, Err(GrabServiceError::NotFound)),
        "expected NotFound, got {result:?}"
    );
}
