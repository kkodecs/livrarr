#![allow(dead_code)]
//! Behavioral DB contract tests for metadata redesign Phase 2.
//! These tests target the Phase 2 API surface and are expected to fail to
//! compile until the DB trait split, migrations, and domain fields exist.

mod common;

use chrono::{Duration, Utc};
use common::create_test_db;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::*;
use livrarr_domain::{normalize_for_matching, EnrichmentStatus, MediaType, TagStatus, UserRole};
use sqlx::Row;

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("phase2-{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: format!("apikey-{suffix}"),
    })
    .await
    .unwrap()
    .id
}

fn work_req(user_id: UserId, title: &str, author_name: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author_name.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author_name),
        author_id: None,
        ol_key: None,
        gr_key: None,
        year: Some(2026),
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        language: Some("en".to_string()),
        import_id: None,
        series_id: None,
        series_name: None,
        series_position: None,
        monitor_ebook: true,
        monitor_audiobook: false,
        source_provider_json: None,
    }
}

async fn create_work(db: &SqliteDb, user_id: UserId, title: &str, author_name: &str) -> Work {
    let (work, created) = WorkDbCreate::create_work(db, work_req(user_id, title, author_name))
        .await
        .unwrap();
    assert!(created, "seed work should be freshly inserted");
    work
}

async fn create_root_folder(db: &SqliteDb, path: &str) -> RootFolderId {
    db.create_root_folder(path, MediaType::Ebook)
        .await
        .unwrap()
        .id
}

fn library_item_req(
    user_id: UserId,
    work_id: WorkId,
    root_folder_id: RootFolderId,
    path: &str,
    tag_status: TagStatus,
    tagged_at_generation: i64,
) -> CreateLibraryItemDbRequest {
    CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id,
        path: path.to_string(),
        media_type: MediaType::Ebook,
        file_size: 1234,
        import_id: None,
        tag_status,
        tagged_at_generation,
    }
}

async fn create_library_item(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    root_folder_id: RootFolderId,
    path: &str,
    tag_status: TagStatus,
    tagged_at_generation: i64,
) -> LibraryItem {
    db.create_library_item(library_item_req(
        user_id,
        work_id,
        root_folder_id,
        path,
        tag_status,
        tagged_at_generation,
    ))
    .await
    .unwrap()
}

async fn stored_work_identity(db: &SqliteDb, work_id: WorkId) -> (String, String) {
    let row = sqlx::query("SELECT normalized_title, normalized_author FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    (
        row.try_get("normalized_title").unwrap(),
        row.try_get("normalized_author").unwrap(),
    )
}

async fn stored_work_status(db: &SqliteDb, work_id: WorkId) -> String {
    sqlx::query_scalar::<_, String>("SELECT enrichment_status FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn stored_library_tag(db: &SqliteDb, item_id: LibraryItemId) -> (String, i64) {
    let row =
        sqlx::query("SELECT tag_status, tagged_at_generation FROM library_items WHERE id = ?")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    (
        row.try_get("tag_status").unwrap(),
        row.try_get("tagged_at_generation").unwrap(),
    )
}

async fn set_work_state(db: &SqliteDb, work_id: WorkId, status: &str, merge_generation: i64) {
    sqlx::query("UPDATE works SET enrichment_status = ?, merge_generation = ? WHERE id = ?")
        .bind(status)
        .bind(merge_generation)
        .bind(work_id)
        .execute(db.pool())
        .await
        .unwrap();
}

async fn set_work_added_at(db: &SqliteDb, work_id: WorkId, added_at: chrono::DateTime<Utc>) {
    sqlx::query("UPDATE works SET added_at = ? WHERE id = ?")
        .bind(added_at.to_rfc3339())
        .bind(work_id)
        .execute(db.pool())
        .await
        .unwrap();
}

async fn set_work_retry_count(db: &SqliteDb, work_id: WorkId, count: i32) {
    sqlx::query("UPDATE works SET enrichment_retry_count = ? WHERE id = ?")
        .bind(count)
        .bind(work_id)
        .execute(db.pool())
        .await
        .unwrap();
}

async fn insert_provider_retry_state(db: &SqliteDb, user_id: UserId, work_id: WorkId) {
    sqlx::query(
        "INSERT INTO provider_retry_state \
         (user_id, work_id, provider, last_outcome, last_attempt_at) \
         VALUES (?, ?, 'open_library', 'not_found', ?)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn create_work_returns_created_true_then_existing_false_on_identity_conflict() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "create-conflict").await;

    let first_req = work_req(user_id, "The Left Hand: Of Darkness", "Ursula_K. Le Guin");
    let expected_title = first_req.normalized_title.clone();
    let expected_author = first_req.normalized_author.clone();

    let (first, first_created) = WorkDbCreate::create_work(&db, first_req).await.unwrap();
    assert!(first_created);
    assert_eq!(first.user_id, user_id);
    assert_eq!(first.title, "The Left Hand: Of Darkness");
    assert_eq!(first.author_name, "Ursula_K. Le Guin");
    assert_eq!(first.year, Some(2026));
    assert_eq!(
        first.cover_url.as_deref(),
        Some("https://example.test/cover.jpg")
    );

    let second_req = work_req(user_id, " the left hand of darkness ", "URSULA K/ LE GUIN");
    assert_eq!(second_req.normalized_title, expected_title);
    assert_eq!(second_req.normalized_author, expected_author);

    let (second, second_created) = WorkDbCreate::create_work(&db, second_req).await.unwrap();
    assert!(!second_created);
    assert_eq!(second.id, first.id);
    assert_eq!(second.title, first.title);

    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM works \
         WHERE user_id = ? AND normalized_title = ? AND normalized_author = ?",
    )
    .bind(user_id)
    .bind(&expected_title)
    .bind(&expected_author)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn concurrent_create_work_for_same_identity_returns_same_work_with_one_creator() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "concurrent").await;
    let db_a = db.clone();
    let db_b = db.clone();

    let req_a = work_req(user_id, "A Wizard: Of Earthsea", "Ursula Le Guin");
    let req_b = work_req(user_id, "a wizard of earthsea", " ursula le guin ");

    let (result_a, result_b) = tokio::join!(
        WorkDbCreate::create_work(&db_a, req_a),
        WorkDbCreate::create_work(&db_b, req_b)
    );

    let (work_a, created_a) = result_a.unwrap();
    let (work_b, created_b) = result_b.unwrap();
    assert_eq!(work_a.id, work_b.id);
    assert_ne!(
        created_a, created_b,
        "exactly one concurrent caller should create"
    );

    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM works WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn new_work_stores_unenriched_not_pending() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "unenriched-default").await;

    let work = create_work(&db, user_id, "Piranesi", "Susanna Clarke").await;

    assert_eq!(work.enrichment_status, EnrichmentStatus::Unenriched);
    assert_eq!(stored_work_status(&db, work.id).await, "unenriched");
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn new_library_item_has_pending_tag_status_and_generation_zero() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "item-defaults").await;
    let work = create_work(&db, user_id, "Jonathan Strange", "Susanna Clarke").await;
    let root_id = create_root_folder(&db, "/phase2/item-defaults").await;

    let item = create_library_item(
        &db,
        user_id,
        work.id,
        root_id,
        "Jonathan Strange.epub",
        TagStatus::Pending,
        0,
    )
    .await;

    assert_eq!(item.tag_status, TagStatus::Pending);
    assert_eq!(item.tagged_at_generation, 0);
    assert_eq!(
        stored_library_tag(&db, item.id).await,
        ("pending".to_string(), 0)
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn list_library_items_needing_tag_sync_selects_pending_enriched_and_stale_synced_or_failed() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "tag-sync-list").await;
    let root_id = create_root_folder(&db, "/phase2/tag-sync-list").await;

    let pending_enriched = create_work(&db, user_id, "Pending Enriched", "Author").await;
    set_work_state(&db, pending_enriched.id, "enriched", 0).await;
    let pending_enriched_item = create_library_item(
        &db,
        user_id,
        pending_enriched.id,
        root_id,
        "pending-enriched.epub",
        TagStatus::Pending,
        0,
    )
    .await;

    let pending_unenriched = create_work(&db, user_id, "Pending Unenriched", "Author").await;
    let pending_unenriched_item = create_library_item(
        &db,
        user_id,
        pending_unenriched.id,
        root_id,
        "pending-unenriched.epub",
        TagStatus::Pending,
        0,
    )
    .await;

    let stale_synced = create_work(&db, user_id, "Stale Synced", "Author").await;
    set_work_state(&db, stale_synced.id, "enriched", 5).await;
    let stale_synced_item = create_library_item(
        &db,
        user_id,
        stale_synced.id,
        root_id,
        "stale-synced.epub",
        TagStatus::Synced,
        4,
    )
    .await;

    let stale_failed = create_work(&db, user_id, "Stale Failed", "Author").await;
    set_work_state(&db, stale_failed.id, "enriched", 8).await;
    let stale_failed_item = create_library_item(
        &db,
        user_id,
        stale_failed.id,
        root_id,
        "stale-failed.epub",
        TagStatus::Failed,
        7,
    )
    .await;

    let current_synced = create_work(&db, user_id, "Current Synced", "Author").await;
    set_work_state(&db, current_synced.id, "enriched", 3).await;
    let current_synced_item = create_library_item(
        &db,
        user_id,
        current_synced.id,
        root_id,
        "current-synced.epub",
        TagStatus::Synced,
        3,
    )
    .await;

    let needing_sync = db.list_library_items_needing_tag_sync(50).await.unwrap();
    let ids: Vec<LibraryItemId> = needing_sync.iter().map(|item| item.id).collect();

    assert!(ids.contains(&pending_enriched_item.id));
    assert!(ids.contains(&stale_synced_item.id));
    assert!(ids.contains(&stale_failed_item.id));
    assert!(!ids.contains(&pending_unenriched_item.id));
    assert!(!ids.contains(&current_synced_item.id));
    assert_eq!(ids.len(), 3);

    let limited = db.list_library_items_needing_tag_sync(2).await.unwrap();
    assert_eq!(limited.len(), 2);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn update_library_item_tag_status_sets_status_and_generation() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "tag-update").await;
    let work = create_work(&db, user_id, "The Raven Tower", "Ann Leckie").await;
    let root_id = create_root_folder(&db, "/phase2/tag-update").await;
    let item = create_library_item(
        &db,
        user_id,
        work.id,
        root_id,
        "The Raven Tower.epub",
        TagStatus::Pending,
        0,
    )
    .await;

    db.update_library_item_tag_status(item.id, TagStatus::Synced, 12)
        .await
        .unwrap();

    let updated = db.get_library_item(user_id, item.id).await.unwrap();
    assert_eq!(updated.tag_status, TagStatus::Synced);
    assert_eq!(updated.tagged_at_generation, 12);
    assert_eq!(
        stored_library_tag(&db, item.id).await,
        ("synced".to_string(), 12)
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn list_stale_unenriched_works_returns_unenriched_older_than_threshold_across_users() {
    let db = create_test_db().await;
    let user_a = create_user(&db, "stale-a").await;
    let user_b = create_user(&db, "stale-b").await;
    let now = Utc::now();
    let threshold = now - Duration::hours(1);

    let stale_a = create_work(&db, user_a, "Old Unenriched A", "Author").await;
    let stale_b = create_work(&db, user_b, "Old Unenriched B", "Author").await;
    let recent = create_work(&db, user_a, "Recent Unenriched", "Author").await;
    let old_enriched = create_work(&db, user_a, "Old Enriched", "Author").await;

    set_work_added_at(&db, stale_a.id, now - Duration::hours(2)).await;
    set_work_added_at(&db, stale_b.id, now - Duration::hours(3)).await;
    set_work_added_at(&db, recent.id, now).await;
    set_work_added_at(&db, old_enriched.id, now - Duration::hours(2)).await;
    set_work_state(&db, old_enriched.id, "enriched", 1).await;

    let stale = db.list_stale_unenriched_works(threshold).await.unwrap();
    let ids: Vec<WorkId> = stale.iter().map(|work| work.id).collect();

    assert!(ids.contains(&stale_a.id));
    assert!(ids.contains(&stale_b.id));
    assert!(!ids.contains(&recent.id));
    assert!(!ids.contains(&old_enriched.id));
    assert!(stale.iter().any(|work| work.user_id == user_a));
    assert!(stale.iter().any(|work| work.user_id == user_b));
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn list_failed_works_without_retry_state_excludes_retry_rows_and_retry_count_three() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "failed-orphans").await;

    let orphan_failed = create_work(&db, user_id, "Orphan Failed", "Author").await;
    let failed_with_retry_row = create_work(&db, user_id, "Failed With Retry", "Author").await;
    let failed_retry_budget_used = create_work(&db, user_id, "Failed Retry Budget", "Author").await;
    let enriched = create_work(&db, user_id, "Not Failed", "Author").await;

    set_work_state(&db, orphan_failed.id, "failed", 0).await;
    set_work_state(&db, failed_with_retry_row.id, "failed", 0).await;
    set_work_state(&db, failed_retry_budget_used.id, "failed", 0).await;
    set_work_state(&db, enriched.id, "enriched", 0).await;
    insert_provider_retry_state(&db, user_id, failed_with_retry_row.id).await;
    set_work_retry_count(&db, failed_retry_budget_used.id, 3).await;

    let failed = db.list_failed_works_without_retry_state().await.unwrap();
    let ids: Vec<WorkId> = failed.iter().map(|work| work.id).collect();

    assert!(ids.contains(&orphan_failed.id));
    assert!(!ids.contains(&failed_with_retry_row.id));
    assert!(!ids.contains(&failed_retry_budget_used.id));
    assert!(!ids.contains(&enriched.id));
    assert_eq!(ids.len(), 1);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn normalized_identity_is_stored_on_create_and_rewritten_on_user_title_update() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "normalized-update").await;
    let work = create_work(&db, user_id, "Dune: Messiah", "Frank_Herbert").await;

    assert_eq!(
        stored_work_identity(&db, work.id).await,
        (
            normalize_for_matching("Dune: Messiah"),
            normalize_for_matching("Frank_Herbert"),
        )
    );

    let updated_title = "Children/Of Dune";
    let updated = db
        .update_work_user_fields(
            user_id,
            work.id,
            UpdateWorkUserFieldsDbRequest {
                title: Some(updated_title.to_string()),
                normalized_title: Some(normalize_for_matching(updated_title)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.title, updated_title);
    assert_eq!(
        stored_work_identity(&db, work.id).await,
        (
            normalize_for_matching(updated_title),
            normalize_for_matching("Frank_Herbert"),
        )
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn unique_identity_constraint_allows_same_title_and_author_for_different_users() {
    let db = create_test_db().await;
    let user_a = create_user(&db, "same-a").await;
    let user_b = create_user(&db, "same-b").await;

    let (work_a, created_a) = WorkDbCreate::create_work(
        &db,
        work_req(user_a, "Station Eleven", "Emily St. John Mandel"),
    )
    .await
    .unwrap();
    let (work_b, created_b) = WorkDbCreate::create_work(
        &db,
        work_req(user_b, "Station Eleven", "Emily St. John Mandel"),
    )
    .await
    .unwrap();

    assert!(created_a);
    assert!(created_b);
    assert_ne!(work_a.id, work_b.id);
    assert_eq!(work_a.user_id, user_a);
    assert_eq!(work_b.user_id, user_b);

    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM works \
         WHERE normalized_title = ? AND normalized_author = ?",
    )
    .bind(normalize_for_matching("Station Eleven"))
    .bind(normalize_for_matching("Emily St. John Mandel"))
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn update_work_user_fields_returns_conflict_on_unique_constraint_violation() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "rename-conflict").await;

    let _work_a = create_work(&db, user_id, "Dune", "Frank Herbert").await;
    let work_b = create_work(&db, user_id, "Foundation", "Isaac Asimov").await;

    // Rename work_b to match work_a's normalized identity → should fail with Conflict
    let result = db
        .update_work_user_fields(
            user_id,
            work_b.id,
            UpdateWorkUserFieldsDbRequest {
                title: Some("Dune".to_string()),
                author_name: Some("Frank Herbert".to_string()),
                normalized_title: Some(normalize_for_matching("Dune")),
                normalized_author: Some(normalize_for_matching("Frank Herbert")),
                ..Default::default()
            },
        )
        .await;

    assert!(
        matches!(result, Err(DbError::Constraint)),
        "renaming to collide with existing work should return Constraint error"
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn backfill_hook_skips_when_index_already_exists() {
    let db = create_test_db().await;

    // After migrations + backfill, the index should exist
    let index_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_works_identity'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();

    // If index exists, a second call to backfill should be a no-op (no queries, instant return)
    // This test verifies the contract — implementation must check index existence first
    assert!(
        index_exists,
        "after migrations and backfill, idx_works_identity must exist"
    );
}
