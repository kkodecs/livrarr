use chrono::Utc;

use librarr::db::{
    CreateDownloadClientDbRequest, CreateGrabDbRequest, DbError, DownloadClient, DownloadClientDb,
    DownloadClientImplementation, Grab, GrabDb, GrabStatus, RemotePathMapping, RemotePathMappingDb,
    UpdateDownloadClientDbRequest,
};

type UserId = i64;
type WorkId = i64;

fn build_create_download_client_req(name: &str, enabled: bool) -> CreateDownloadClientDbRequest {
    CreateDownloadClientDbRequest {
        name: name.to_string(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: format!("{name}.example.test"),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: Some("/qb".to_string()),
        username: Some("user".to_string()),
        password: Some("pass".to_string()),
        category: "books".to_string(),
        download_dir: None,
        enabled,
        api_key: None,
    }
}

fn build_update_download_client_req() -> UpdateDownloadClientDbRequest {
    UpdateDownloadClientDbRequest {
        name: Some("updated-client".to_string()),
        host: Some("updated.example.test".to_string()),
        port: Some(9090),
        use_ssl: Some(true),
        skip_ssl_validation: Some(true),
        url_base: Some("/updated".to_string()),
        username: Some("updated-user".to_string()),
        password: Some("updated-pass".to_string()),
        category: Some("updated-category".to_string()),
        download_dir: None,
        enabled: Some(false),
        api_key: None,
        is_default_for_protocol: None,
    }
}

fn build_create_grab_req(
    user_id: UserId,
    work_id: WorkId,
    download_client_id: i64,
    guid: &str,
    indexer: &str,
    status: GrabStatus,
) -> CreateGrabDbRequest {
    CreateGrabDbRequest {
        user_id,
        work_id,
        download_client_id,
        title: format!("Title {guid}"),
        indexer: indexer.to_string(),
        guid: guid.to_string(),
        size: Some(12345),
        download_url: format!("https://downloads.example.test/{guid}.torrent"),
        download_id: None,
        status,
        media_type: None,
    }
}

async fn create_download_client<D: DownloadClientDb + ?Sized>(
    db: &D,
    name: &str,
    enabled: bool,
) -> DownloadClient {
    db.create_download_client(build_create_download_client_req(name, enabled))
        .await
        .expect("create_download_client should succeed")
}

async fn create_grab<D: GrabDb + ?Sized>(
    db: &D,
    user_id: UserId,
    work_id: WorkId,
    download_client_id: i64,
    guid: &str,
    indexer: &str,
    status: GrabStatus,
) -> Grab {
    db.upsert_grab(build_create_grab_req(
        user_id,
        work_id,
        download_client_id,
        guid,
        indexer,
        status,
    ))
    .await
    .expect("upsert_grab should succeed")
}

async fn create_remote_path_mapping<D: RemotePathMappingDb + ?Sized>(
    db: &D,
    host: &str,
    remote_path: &str,
    local_path: &str,
) -> RemotePathMapping {
    db.create_remote_path_mapping(host, remote_path, local_path)
        .await
        .expect("create_remote_path_mapping should succeed")
}

// Phase 2: wired to real in-memory DB
async fn setup_test_db() -> impl GrabDb + DownloadClientDb + RemotePathMappingDb {
    librarr::db::test_helpers::new_test_db().await
}

// Satisfies: DLC-006; IR contract: GrabDb::upsert_grab, GrabDb::get_grab
async fn test_grabdb_upsert_creates_and_get_retrieves(db: &(impl GrabDb + DownloadClientDb)) {
    let client = create_download_client(db, "client-a", true).await;
    let created = create_grab(db, 1, 10, client.id, "guid-1", "idx-a", GrabStatus::Sent).await;

    let fetched = db
        .get_grab(created.user_id, created.id)
        .await
        .expect("get_grab should succeed");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.user_id, 1);
    assert_eq!(fetched.work_id, 10);
    assert_eq!(fetched.download_client_id, client.id);
    assert_eq!(fetched.guid, "guid-1");
    assert_eq!(fetched.indexer, "idx-a");
    assert_eq!(fetched.status, GrabStatus::Sent);
    assert_eq!(fetched.download_id, None);
    assert!(fetched.grabbed_at <= Utc::now());
}

// Satisfies: IMPORT-005; IR contract: GrabDb::list_active_grabs
async fn test_grabdb_list_active_grabs_returns_sent_and_confirmed(
    db: &(impl GrabDb + DownloadClientDb),
) {
    let client = create_download_client(db, "client-b", true).await;
    let sent = create_grab(db, 2, 20, client.id, "guid-sent", "idx", GrabStatus::Sent).await;
    let confirmed = create_grab(
        db,
        2,
        21,
        client.id,
        "guid-confirmed",
        "idx",
        GrabStatus::Confirmed,
    )
    .await;

    let active = db
        .list_active_grabs()
        .await
        .expect("list_active_grabs should succeed");

    assert!(active.iter().any(|g| g.id == sent.id));
    assert!(active.iter().any(|g| g.id == confirmed.id));
}

// Satisfies: DLC-015; IR contract: GrabDb::update_grab_status
async fn test_grabdb_update_status_changes_status(db: &(impl GrabDb + DownloadClientDb)) {
    let client = create_download_client(db, "client-c", true).await;
    let grab = create_grab(db, 3, 30, client.id, "guid-2", "idx", GrabStatus::Sent).await;

    db.update_grab_status(grab.user_id, grab.id, GrabStatus::Confirmed, None)
        .await
        .expect("update_grab_status should succeed");

    let fetched = db
        .get_grab(grab.user_id, grab.id)
        .await
        .expect("get_grab should succeed");

    assert_eq!(fetched.status, GrabStatus::Confirmed);
    assert_eq!(fetched.import_error, None);
}

// Satisfies: DLC-015; IR contract: GrabDb::update_grab_download_id, GrabDb::get_grab_by_download_id
async fn test_grabdb_update_download_id_and_lookup_by_download_id(
    db: &(impl GrabDb + DownloadClientDb),
) {
    let client = create_download_client(db, "client-d", true).await;
    let grab = create_grab(db, 4, 40, client.id, "guid-3", "idx", GrabStatus::Sent).await;

    db.update_grab_download_id(grab.user_id, grab.id, "torrent-hash-123")
        .await
        .expect("update_grab_download_id should succeed");

    let fetched = db
        .get_grab_by_download_id("torrent-hash-123")
        .await
        .expect("get_grab_by_download_id should succeed")
        .expect("grab should be found");

    assert_eq!(fetched.id, grab.id);
    assert_eq!(fetched.download_id.as_deref(), Some("torrent-hash-123"));
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: GrabDb::get_grab
async fn test_grabdb_get_nonexistent_returns_not_found(db: &impl GrabDb) {
    let result = db.get_grab(999, 999_999).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: DLC-009; IR contract: GrabDb::upsert_grab UNIQUE(user_id, guid, indexer) active duplicate => Constraint
async fn test_grabdb_upsert_duplicate_active_returns_constraint(
    db: &(impl GrabDb + DownloadClientDb),
) {
    let client = create_download_client(db, "client-e", true).await;
    let _ = create_grab(
        db,
        5,
        50,
        client.id,
        "dup-guid-active",
        "idx",
        GrabStatus::Sent,
    )
    .await;

    let result = db
        .upsert_grab(build_create_grab_req(
            5,
            51,
            client.id,
            "dup-guid-active",
            "idx",
            GrabStatus::Sent,
        ))
        .await;

    assert!(matches!(result, Err(DbError::Constraint { .. })));
}

// Satisfies: DLC-009; IR contract: GrabDb::upsert_grab replaces Failed duplicate
async fn test_grabdb_upsert_duplicate_failed_replaces(db: &(impl GrabDb + DownloadClientDb)) {
    let client = create_download_client(db, "client-f", true).await;
    let original = create_grab(
        db,
        6,
        60,
        client.id,
        "dup-guid-failed",
        "idx",
        GrabStatus::Failed,
    )
    .await;

    let replaced = db
        .upsert_grab(build_create_grab_req(
            6,
            61,
            client.id,
            "dup-guid-failed",
            "idx",
            GrabStatus::Sent,
        ))
        .await
        .expect("upsert_grab should replace failed grab");

    assert_ne!(replaced.id, original.id);
    assert_eq!(replaced.user_id, 6);
    assert_eq!(replaced.work_id, 61);
    assert_eq!(replaced.guid, "dup-guid-failed");
    assert_eq!(replaced.indexer, "idx");
    assert_eq!(replaced.status, GrabStatus::Sent);
}

// Satisfies: DLC-009; IR contract: GrabDb::upsert_grab replaces Removed duplicate
async fn test_grabdb_upsert_duplicate_removed_replaces(db: &(impl GrabDb + DownloadClientDb)) {
    let client = create_download_client(db, "client-g", true).await;
    let original = create_grab(
        db,
        7,
        70,
        client.id,
        "dup-guid-removed",
        "idx",
        GrabStatus::Removed,
    )
    .await;

    let replaced = db
        .upsert_grab(build_create_grab_req(
            7,
            71,
            client.id,
            "dup-guid-removed",
            "idx",
            GrabStatus::Sent,
        ))
        .await
        .expect("upsert_grab should replace removed grab");

    assert_ne!(replaced.id, original.id);
    assert_eq!(replaced.work_id, 71);
    assert_eq!(replaced.status, GrabStatus::Sent);
}

// Satisfies: IMPORT-005; IR contract: GrabDb::list_active_grabs excludes non-active statuses
async fn test_grabdb_list_active_grabs_excludes_non_active_statuses(
    db: &(impl GrabDb + DownloadClientDb),
) {
    let client = create_download_client(db, "client-h", true).await;
    let sent = create_grab(db, 8, 80, client.id, "ga", "idx", GrabStatus::Sent).await;
    let confirmed = create_grab(db, 8, 81, client.id, "gb", "idx", GrabStatus::Confirmed).await;
    let importing = create_grab(db, 8, 82, client.id, "gc", "idx", GrabStatus::Importing).await;
    let imported = create_grab(db, 8, 83, client.id, "gd", "idx", GrabStatus::Imported).await;
    let import_failed =
        create_grab(db, 8, 84, client.id, "ge", "idx", GrabStatus::ImportFailed).await;
    let removed = create_grab(db, 8, 85, client.id, "gf", "idx", GrabStatus::Removed).await;
    let failed = create_grab(db, 8, 86, client.id, "gg", "idx", GrabStatus::Failed).await;

    let active = db
        .list_active_grabs()
        .await
        .expect("list_active_grabs should succeed");

    assert!(active.iter().any(|g| g.id == sent.id));
    assert!(active.iter().any(|g| g.id == confirmed.id));
    assert!(!active.iter().any(|g| g.id == importing.id));
    assert!(!active.iter().any(|g| g.id == imported.id));
    assert!(!active.iter().any(|g| g.id == import_failed.id));
    assert!(!active.iter().any(|g| g.id == removed.id));
    assert!(!active.iter().any(|g| g.id == failed.id));
}

// Satisfies: DLC-012; IR contract: GrabDb::update_grab_status to Removed
async fn test_grabdb_update_status_to_removed_succeeds(db: &(impl GrabDb + DownloadClientDb)) {
    let client = create_download_client(db, "client-i", true).await;
    let grab = create_grab(
        db,
        9,
        90,
        client.id,
        "guid-removed",
        "idx",
        GrabStatus::Confirmed,
    )
    .await;

    db.update_grab_status(grab.user_id, grab.id, GrabStatus::Removed, None)
        .await
        .expect("update_grab_status to Removed should succeed");

    let fetched = db
        .get_grab(grab.user_id, grab.id)
        .await
        .expect("get_grab should succeed");

    assert_eq!(fetched.status, GrabStatus::Removed);
}

// Satisfies: REQ-ID none (boundary path from spec); IR contract: GrabDb::get_grab_by_download_id
async fn test_grabdb_get_by_download_id_returns_none_when_missing(db: &impl GrabDb) {
    let result = db
        .get_grab_by_download_id("missing-download-id")
        .await
        .expect("get_grab_by_download_id should succeed");
    assert_eq!(result, None);
}

// Satisfies: DLC-001; IR contract: DownloadClientDb::create_download_client, DownloadClientDb::get_download_client
async fn test_downloadclientdb_create_and_get(db: &impl DownloadClientDb) {
    let created = db
        .create_download_client(build_create_download_client_req("client-j", true))
        .await
        .expect("create_download_client should succeed");

    let fetched = db
        .get_download_client(created.id)
        .await
        .expect("get_download_client should succeed");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "client-j");
    assert_eq!(
        fetched.implementation,
        DownloadClientImplementation::QBittorrent
    );
    assert_eq!(fetched.host, "client-j.example.test");
    assert_eq!(fetched.port, 8080);
    assert!(!fetched.use_ssl);
    assert!(!fetched.skip_ssl_validation);
    assert_eq!(fetched.url_base.as_deref(), Some("/qb"));
    assert_eq!(fetched.username.as_deref(), Some("user"));
    assert_eq!(fetched.password.as_deref(), Some("pass"));
    assert_eq!(fetched.category, "books");
    assert!(fetched.enabled);
}

// Satisfies: DLC-003; IR contract: DownloadClientDb::list_download_clients
async fn test_downloadclientdb_list_returns_created_clients(db: &impl DownloadClientDb) {
    let a = create_download_client(db, "client-k1", true).await;
    let b = create_download_client(db, "client-k2", false).await;

    let listed = db
        .list_download_clients()
        .await
        .expect("list_download_clients should succeed");

    assert!(listed.iter().any(|c| c.id == a.id));
    assert!(listed.iter().any(|c| c.id == b.id));
}

// Satisfies: DLC-001; IR contract: DownloadClientDb::update_download_client
async fn test_downloadclientdb_update_changes_fields(db: &impl DownloadClientDb) {
    let created = create_download_client(db, "client-l", true).await;

    let updated = db
        .update_download_client(created.id, build_update_download_client_req())
        .await
        .expect("update_download_client should succeed");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.name, "updated-client");
    assert_eq!(updated.host, "updated.example.test");
    assert_eq!(updated.port, 9090);
    assert!(updated.use_ssl);
    assert!(updated.skip_ssl_validation);
    assert_eq!(updated.url_base.as_deref(), Some("/updated"));
    assert_eq!(updated.username.as_deref(), Some("updated-user"));
    assert_eq!(updated.password.as_deref(), Some("updated-pass"));
    assert_eq!(updated.category, "updated-category");
    assert!(!updated.enabled);
}

// Satisfies: DLC-001; IR contract: DownloadClientDb::delete_download_client
async fn test_downloadclientdb_delete_removes_client(db: &impl DownloadClientDb) {
    let created = create_download_client(db, "client-m", true).await;

    db.delete_download_client(created.id)
        .await
        .expect("delete_download_client should succeed");

    let result = db.get_download_client(created.id).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: DLC-005, USE-DLC-004; IR contract: DownloadClientDb::get_default_download_client
// With auto-promote, the first enabled client for a protocol becomes the default.
async fn test_downloadclientdb_get_default_returns_highest_enabled_id(db: &impl DownloadClientDb) {
    let _disabled = create_download_client(db, "client-n1", false).await;
    let enabled_first = create_download_client(db, "client-n2", true).await;
    let _disabled2 = create_download_client(db, "client-n3", false).await;
    let _enabled_second = create_download_client(db, "client-n4", true).await;

    let default = db
        .get_default_download_client("qbittorrent")
        .await
        .expect("get_default_download_client should succeed")
        .expect("default client should exist");

    // First enabled client was auto-promoted as default.
    assert_eq!(default.id, enabled_first.id);
    assert!(default.enabled);
    assert!(default.is_default_for_protocol);
}

// Satisfies: DLC-005; IR contract: DownloadClientDb::get_default_download_client returns None when no enabled clients
async fn test_downloadclientdb_get_default_returns_none_when_none_enabled(
    db: &impl DownloadClientDb,
) {
    let _ = create_download_client(db, "client-o1", false).await;
    let _ = create_download_client(db, "client-o2", false).await;

    let default = db
        .get_default_download_client("qbittorrent")
        .await
        .expect("get_default_download_client should succeed");

    assert_eq!(default, None);
}

// Satisfies: DLC-005; IR contract: DownloadClientDb::get_default_download_client returns None when no clients exist
async fn test_downloadclientdb_get_default_returns_none_when_no_clients(
    db: &impl DownloadClientDb,
) {
    let default = db
        .get_default_download_client("qbittorrent")
        .await
        .expect("get_default_download_client should succeed");

    assert_eq!(default, None);
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: DownloadClientDb::get_download_client
async fn test_downloadclientdb_get_nonexistent_returns_not_found(db: &impl DownloadClientDb) {
    let result = db.get_download_client(999_999).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: DownloadClientDb::delete_download_client
async fn test_downloadclientdb_delete_nonexistent_returns_not_found(db: &impl DownloadClientDb) {
    let result = db.delete_download_client(999_999).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: DownloadClientDb::update_download_client
async fn test_downloadclientdb_update_nonexistent_returns_not_found(db: &impl DownloadClientDb) {
    let result = db
        .update_download_client(999_999, build_update_download_client_req())
        .await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: DLC-013; IR contract: RemotePathMappingDb::create_remote_path_mapping, RemotePathMappingDb::get_remote_path_mapping
async fn test_remotepathmappingdb_create_and_get(db: &impl RemotePathMappingDb) {
    let created = create_remote_path_mapping(db, "host-a", "/remote/a", "/local/a").await;

    let fetched = db
        .get_remote_path_mapping(created.id)
        .await
        .expect("get_remote_path_mapping should succeed");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.host, "host-a");
    assert_eq!(fetched.remote_path, "/remote/a");
    assert_eq!(fetched.local_path, "/local/a");
}

// Satisfies: DLC-013; IR contract: RemotePathMappingDb::list_remote_path_mappings
async fn test_remotepathmappingdb_list_returns_created_mappings(db: &impl RemotePathMappingDb) {
    let a = create_remote_path_mapping(db, "host-b", "/remote/b", "/local/b").await;
    let b = create_remote_path_mapping(db, "host-c", "/remote/c", "/local/c").await;

    let listed = db
        .list_remote_path_mappings()
        .await
        .expect("list_remote_path_mappings should succeed");

    assert!(listed.iter().any(|m| m.id == a.id));
    assert!(listed.iter().any(|m| m.id == b.id));
}

// Satisfies: DLC-013; IR contract: RemotePathMappingDb::update_remote_path_mapping
async fn test_remotepathmappingdb_update_changes_fields(db: &impl RemotePathMappingDb) {
    let created = create_remote_path_mapping(db, "host-d", "/remote/d", "/local/d").await;

    let updated = db
        .update_remote_path_mapping(created.id, "host-d2", "/remote/d2", "/local/d2")
        .await
        .expect("update_remote_path_mapping should succeed");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.host, "host-d2");
    assert_eq!(updated.remote_path, "/remote/d2");
    assert_eq!(updated.local_path, "/local/d2");
}

// Satisfies: DLC-013; IR contract: RemotePathMappingDb::delete_remote_path_mapping
async fn test_remotepathmappingdb_delete_removes_mapping(db: &impl RemotePathMappingDb) {
    let created = create_remote_path_mapping(db, "host-e", "/remote/e", "/local/e").await;

    db.delete_remote_path_mapping(created.id)
        .await
        .expect("delete_remote_path_mapping should succeed");

    let result = db.get_remote_path_mapping(created.id).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: RemotePathMappingDb::get_remote_path_mapping
async fn test_remotepathmappingdb_get_nonexistent_returns_not_found(db: &impl RemotePathMappingDb) {
    let result = db.get_remote_path_mapping(999_999).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: RemotePathMappingDb::delete_remote_path_mapping
async fn test_remotepathmappingdb_delete_nonexistent_returns_not_found(
    db: &impl RemotePathMappingDb,
) {
    let result = db.delete_remote_path_mapping(999_999).await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

// Satisfies: REQ-ID none (failure path from spec); IR contract: RemotePathMappingDb::update_remote_path_mapping
async fn test_remotepathmappingdb_update_nonexistent_returns_not_found(
    db: &impl RemotePathMappingDb,
) {
    let result = db
        .update_remote_path_mapping(999_999, "host-z", "/remote/z", "/local/z")
        .await;
    assert!(matches!(result, Err(DbError::NotFound)));
}

#[tokio::test]
async fn test_grabdb_upsert_creates_and_get_retrieves_tokio() {
    let db = setup_test_db().await;
    test_grabdb_upsert_creates_and_get_retrieves(&db).await;
}

#[tokio::test]
async fn test_grabdb_list_active_grabs_returns_sent_and_confirmed_tokio() {
    let db = setup_test_db().await;
    test_grabdb_list_active_grabs_returns_sent_and_confirmed(&db).await;
}

#[tokio::test]
async fn test_grabdb_update_status_changes_status_tokio() {
    let db = setup_test_db().await;
    test_grabdb_update_status_changes_status(&db).await;
}

#[tokio::test]
async fn test_grabdb_update_download_id_and_lookup_by_download_id_tokio() {
    let db = setup_test_db().await;
    test_grabdb_update_download_id_and_lookup_by_download_id(&db).await;
}

#[tokio::test]
async fn test_grabdb_get_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_grabdb_get_nonexistent_returns_not_found(&db).await;
}

#[tokio::test]
async fn test_grabdb_upsert_duplicate_active_returns_constraint_tokio() {
    let db = setup_test_db().await;
    test_grabdb_upsert_duplicate_active_returns_constraint(&db).await;
}

#[tokio::test]
async fn test_grabdb_upsert_duplicate_failed_replaces_tokio() {
    let db = setup_test_db().await;
    test_grabdb_upsert_duplicate_failed_replaces(&db).await;
}

#[tokio::test]
async fn test_grabdb_upsert_duplicate_removed_replaces_tokio() {
    let db = setup_test_db().await;
    test_grabdb_upsert_duplicate_removed_replaces(&db).await;
}

#[tokio::test]
async fn test_grabdb_list_active_grabs_excludes_non_active_statuses_tokio() {
    let db = setup_test_db().await;
    test_grabdb_list_active_grabs_excludes_non_active_statuses(&db).await;
}

#[tokio::test]
async fn test_grabdb_update_status_to_removed_succeeds_tokio() {
    let db = setup_test_db().await;
    test_grabdb_update_status_to_removed_succeeds(&db).await;
}

#[tokio::test]
async fn test_grabdb_get_by_download_id_returns_none_when_missing_tokio() {
    let db = setup_test_db().await;
    test_grabdb_get_by_download_id_returns_none_when_missing(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_create_and_get_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_create_and_get(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_list_returns_created_clients_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_list_returns_created_clients(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_update_changes_fields_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_update_changes_fields(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_delete_removes_client_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_delete_removes_client(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_get_default_returns_highest_enabled_id_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_get_default_returns_highest_enabled_id(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_get_default_returns_none_when_none_enabled_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_get_default_returns_none_when_none_enabled(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_get_default_returns_none_when_no_clients_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_get_default_returns_none_when_no_clients(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_get_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_get_nonexistent_returns_not_found(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_delete_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_delete_nonexistent_returns_not_found(&db).await;
}

#[tokio::test]
async fn test_downloadclientdb_update_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_downloadclientdb_update_nonexistent_returns_not_found(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_create_and_get_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_create_and_get(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_list_returns_created_mappings_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_list_returns_created_mappings(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_update_changes_fields_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_update_changes_fields(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_delete_removes_mapping_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_delete_removes_mapping(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_get_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_get_nonexistent_returns_not_found(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_delete_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_delete_nonexistent_returns_not_found(&db).await;
}

#[tokio::test]
async fn test_remotepathmappingdb_update_nonexistent_returns_not_found_tokio() {
    let db = setup_test_db().await;
    test_remotepathmappingdb_update_nonexistent_returns_not_found(&db).await;
}
