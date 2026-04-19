// tests/behavioral/test_consolidation_work_service.rs
#![allow(dead_code, unused_imports)]

//! Behavioral tests for WorkService trait (SVC-WORK-001..004).
//! Covers: fn.work_service.{add, get, list, update, delete, refresh, refresh_all, upload_cover, download_cover}
//! Test obligations: test.work.add.*, test.work.refresh.*, test.work.refresh_all.*
//! Added for redesign phase:
//! - AddWorkRequest.provenance_setter behavioral contracts
//! - AddWorkResult.author_id behavioral contracts
//! - WorkService::lookup() future behavioral contracts (ignored until trait lands)

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::UserRole;
use livrarr_metadata::work_service::WorkServiceImpl;
use std::sync::Arc;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-test-{}", std::process::id()))
}

fn stub_http() -> StubHttpFetcher {
    StubHttpFetcher::new()
}

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "testuser".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "testhash".into(),
    })
    .await
    .unwrap()
    .id
}

async fn setup_second_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "otheruser".into(),
        password_hash: "hash".into(),
        role: UserRole::User,
        api_key_hash: "testhash2".into(),
    })
    .await
    .unwrap()
    .id
}

fn no_filter() -> WorkFilter {
    WorkFilter {
        author_id: None,
        monitored: None,
        enrichment_status: None,
        sort_by: None,
        sort_dir: None,
        media_type: None,
    }
}

// =============================================================================
// add
// =============================================================================

#[tokio::test]
async fn test_work_add_happy_path_creates_with_provenance() {
    // SVC-WORK-001, SVC-WORK-002: Given a new work with ol_key, work is created
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let req = AddWorkRequest {
        title: "The Way of Kings".into(),
        author_name: "Brandon Sanderson".into(),
        ol_key: Some("/works/OL123W".into()),
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };

    let result = svc.add(user_id, req).await.expect("add should succeed");
    let work = result.work;
    assert!(work.id > 0);
    assert_eq!(work.user_id, user_id);
    assert_eq!(work.title, "The Way of Kings");
    assert_eq!(work.ol_key.as_deref(), Some("/works/OL123W"));
    assert_eq!(work.author_name, "Brandon Sanderson");
}

#[tokio::test]
async fn test_work_add_duplicate_ol_key_returns_already_exists() {
    // SVC-WORK-001: Given duplicate ol_key, returns AlreadyExists
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let req1 = AddWorkRequest {
        title: "Book One".into(),
        author_name: "".into(),
        ol_key: Some("/works/OL999W".into()),
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };
    svc.add(user_id, req1).await.unwrap();

    let req2 = AddWorkRequest {
        title: "Book One Again".into(),
        author_name: "".into(),
        ol_key: Some("/works/OL999W".into()),
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };
    let result = svc.add(user_id, req2).await;
    assert!(
        matches!(result, Err(WorkServiceError::AlreadyExists)),
        "expected AlreadyExists, got {result:?}"
    );
}

#[tokio::test]
async fn test_work_add_enrichment_failure_returns_ok_unenriched() {
    // SVC-WORK-002: Given enrichment failure, returns Ok with unenriched work
    use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::failing(),
        stub_http(),
        test_data_dir(),
    );

    let req = AddWorkRequest {
        title: "Enrichment Fails".into(),
        author_name: "Author".into(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };

    let result = svc
        .add(user_id, req)
        .await
        .expect("add should succeed even when enrichment fails");
    assert_eq!(result.work.title, "Enrichment Fails");
    assert_eq!(result.work.user_id, user_id);
}

#[tokio::test]
#[ignore = "pk-implement: requires cover download stub"]
async fn test_work_add_cover_download_failure_returns_ok() {
    // SVC-WORK-002: Given cover download failure, returns Ok (cover is best-effort)
    todo!("Setup: create add request whose enrichment returns metadata including cover URL/path, but stub cover download/storage to fail. Assert: result.is_ok(); work row is created in DB")
}

#[tokio::test]
async fn test_work_add_finds_existing_author_by_normalized_name() {
    // SVC-WORK-002: Author is found by normalized name when existing
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    // Seed an author with mixed case
    db.create_author(CreateAuthorDbRequest {
        user_id,
        name: "Ursula K. Le Guin".into(),
        sort_name: None,
        ol_key: None,
        gr_key: None,
        hc_key: None,
        import_id: None,
    })
    .await
    .unwrap();

    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let req = AddWorkRequest {
        title: "The Left Hand of Darkness".into(),
        author_name: "  ursula k. le guin  ".into(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };

    let result = svc.add(user_id, req).await.unwrap();
    let work = result.work;
    assert!(work.author_id.is_some());

    // Should not have created a second author
    let authors = db.list_authors(user_id).await.unwrap();
    assert_eq!(
        authors.len(),
        1,
        "should reuse existing author, not create a new one"
    );
}

#[tokio::test]
async fn test_work_add_creates_author_when_not_found() {
    // SVC-WORK-002: Author is created when not found
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let req = AddWorkRequest {
        title: "Neuromancer".into(),
        author_name: "William Gibson".into(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };

    let result = svc.add(user_id, req).await.unwrap();
    let work = result.work;
    assert!(work.author_id.is_some());

    let authors = db.list_authors(user_id).await.unwrap();
    assert_eq!(authors.len(), 1);
    assert_eq!(authors[0].name, "William Gibson");
}

#[tokio::test]
async fn test_work_add_cleans_title_and_author() {
    // SVC-WORK-002: Title and author are cleaned before persistence
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let req = AddWorkRequest {
        title: "  The Way of Kings  ".into(),
        author_name: "  Brandon Sanderson  ".into(),
        ol_key: None,
        detail_url: None,
        cover_url: None,
        author_ol_key: None,
        gr_key: None,
        year: None,
        metadata_source: None,
        language: None,
        series_name: None,
        series_position: None,
        defer_enrichment: false,
        provenance_setter: None,
    };

    let result = svc.add(user_id, req).await.unwrap();
    let work = result.work;
    assert_eq!(work.title, "The Way of Kings");
    assert_eq!(work.author_name, "Brandon Sanderson");
}

#[tokio::test]
async fn test_work_add_result_author_id_when_new_author_created() {
    // Redesign contract: new author created => author_created=true, author_id=Some(id)
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let result = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Snow Crash".into(),
                author_name: "Neal Stephenson".into(),
                author_ol_key: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                language: None,
                detail_url: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    assert!(result.author_created, "expected new author to be created");
    let author_id = result.author_id.expect("expected author_id for new author");
    assert!(author_id > 0);

    let authors = db.list_authors(user_id).await.unwrap();
    assert_eq!(authors.len(), 1);
    assert_eq!(authors[0].id, author_id);
    assert_eq!(result.work.author_id, Some(author_id));
}

#[tokio::test]
async fn test_work_add_result_author_id_when_existing_author_reused() {
    // Redesign contract: existing author reused => author_created=false, author_id=Some(existing_id)
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let existing = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Octavia E. Butler".into(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .unwrap();

    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let result = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Kindred".into(),
                author_name: "Octavia E. Butler".into(),
                author_ol_key: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                language: None,
                detail_url: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    assert!(
        !result.author_created,
        "expected existing author to be reused, not created"
    );
    assert_eq!(result.author_id, Some(existing.id));
    assert_eq!(result.work.author_id, Some(existing.id));
}

#[tokio::test]
async fn test_work_add_result_author_id_none_when_no_author_name() {
    // Redesign contract: no author name => author_created=false, author_id=None
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let result = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Anonymous Collection".into(),
                author_name: "".into(),
                author_ol_key: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                language: None,
                detail_url: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    assert!(!result.author_created);
    assert_eq!(result.author_id, None);
    assert_eq!(result.work.author_id, None);

    let authors = db.list_authors(user_id).await.unwrap();
    assert!(
        authors.is_empty(),
        "no author row should be created when author name is empty"
    );
}

#[tokio::test]
#[ignore = "pk-implement: provenance fields are not yet exposed/verifiable through current service or DB API"]
async fn test_work_add_provenance_setter_none_defaults_to_user() {
    todo!("Setup: add work with provenance_setter=None. Assert persisted field provenance for user-editable metadata defaults to ProvenanceSetter::User via detail or DB-backed provenance inspection.")
}

#[tokio::test]
#[ignore = "pk-implement: provenance fields are not yet exposed/verifiable through current service or DB API"]
async fn test_work_add_provenance_setter_auto_added_written() {
    todo!("Setup: add work with provenance_setter=Some(ProvenanceSetter::AutoAdded). Assert persisted provenance is AutoAdded for created fields.")
}

#[tokio::test]
#[ignore = "pk-implement: provenance fields are not yet exposed/verifiable through current service or DB API"]
async fn test_work_add_provenance_setter_imported_written() {
    todo!("Setup: add work with provenance_setter=Some(ProvenanceSetter::Imported). Assert persisted provenance is Imported for created fields.")
}

// =============================================================================
// get
// =============================================================================

#[tokio::test]
async fn test_work_get_existing_returns_work() {
    // SVC-WORK-001: Given existing work for user, returns it
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Dune".into(),
                author_name: "Frank Herbert".into(),
                ol_key: Some("/works/OL1W".into()),
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let work = svc.get(user_id, added.work.id).await.unwrap();
    assert_eq!(work.id, added.work.id);
    assert_eq!(work.user_id, user_id);
    assert_eq!(work.title, "Dune");
}

#[tokio::test]
async fn test_work_get_nonexistent_returns_not_found() {
    // SVC-WORK-001: Given nonexistent work_id, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let result = svc.get(user_id, 99999).await;
    assert!(matches!(result, Err(WorkServiceError::NotFound)));
}

#[tokio::test]
async fn test_work_get_wrong_user_returns_not_found() {
    // SVC-WORK-001: Given work_id belonging to different user, returns NotFound
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let added = svc
        .add(
            user_a,
            AddWorkRequest {
                title: "Book A".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let result = svc.get(user_b, added.work.id).await;
    assert!(matches!(result, Err(WorkServiceError::NotFound)));
}

// =============================================================================
// list
// =============================================================================

#[tokio::test]
async fn test_work_list_no_filter_returns_all() {
    // SVC-WORK-001: Given no filter, returns all works for user
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    svc.add(
        user_a,
        AddWorkRequest {
            title: "W1".into(),
            author_name: "".into(),
            ol_key: None,
            detail_url: None,
            cover_url: None,
            author_ol_key: None,
            gr_key: None,
            year: None,
            metadata_source: None,
            language: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: None,
        },
    )
    .await
    .unwrap();
    svc.add(
        user_a,
        AddWorkRequest {
            title: "W2".into(),
            author_name: "".into(),
            ol_key: None,
            detail_url: None,
            cover_url: None,
            author_ol_key: None,
            gr_key: None,
            year: None,
            metadata_source: None,
            language: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: None,
        },
    )
    .await
    .unwrap();
    svc.add(
        user_b,
        AddWorkRequest {
            title: "Other".into(),
            author_name: "".into(),
            ol_key: None,
            detail_url: None,
            cover_url: None,
            author_ol_key: None,
            gr_key: None,
            year: None,
            metadata_source: None,
            language: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: None,
        },
    )
    .await
    .unwrap();

    let works = svc.list(user_a, no_filter()).await.unwrap();
    assert_eq!(works.len(), 2);
    assert!(works.iter().all(|w| w.user_id == user_a));
}

#[tokio::test]
#[ignore = "pk-implement: list filtering requires DB query changes"]
async fn test_work_list_monitored_filter() {
    // SVC-WORK-001: Given monitored=true filter, returns only monitored works
    todo!("Setup: seed works with monitored=true and false. Call list with filter. Assert only monitored works returned.")
}

#[tokio::test]
#[ignore = "pk-implement: list sorting requires DB query changes"]
async fn test_work_list_sort_by_year() {
    // SVC-WORK-001: Given sort_by=Year, results are sorted by year
    todo!(
        "Setup: seed works with distinct years. Call list with sort_by=Year. Assert sorted order."
    )
}

// =============================================================================
// update
// =============================================================================

#[tokio::test]
async fn test_work_update_title_changes() {
    // SVC-WORK-001: Given title update, title changes
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Old Title".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let updated = svc
        .update(
            user_id,
            added.work.id,
            UpdateWorkRequest {
                title: Some("New Title".into()),
                author_name: None,
                series_name: None,
                series_position: None,
                monitor_ebook: None,
                monitor_audiobook: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.title, "New Title");

    // Verify persisted
    let persisted = svc.get(user_id, added.work.id).await.unwrap();
    assert_eq!(persisted.title, "New Title");
}

#[tokio::test]
#[ignore = "pk-implement: provenance infrastructure not yet integrated"]
async fn test_work_update_title_provenance_set_to_user() {
    // SVC-WORK-001: Given title update, provenance is set to User
    todo!("Verify provenance for title field is set to User after update")
}

#[tokio::test]
async fn test_work_update_none_title_unchanged() {
    // SVC-WORK-001: Given None title, title is unchanged
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Keep This".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let updated = svc
        .update(
            user_id,
            added.work.id,
            UpdateWorkRequest {
                title: None,
                author_name: None,
                series_name: None,
                series_position: None,
                monitor_ebook: Some(true),
                monitor_audiobook: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.title, "Keep This");
}

#[tokio::test]
async fn test_work_update_nonexistent_returns_not_found() {
    // SVC-WORK-001: Given nonexistent work, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let result = svc
        .update(
            user_id,
            99999,
            UpdateWorkRequest {
                title: Some("X".into()),
                author_name: None,
                series_name: None,
                series_position: None,
                monitor_ebook: None,
                monitor_audiobook: None,
            },
        )
        .await;
    assert!(matches!(result, Err(WorkServiceError::NotFound)));
}

// =============================================================================
// delete
// =============================================================================

#[tokio::test]
async fn test_work_delete_removes_work_and_library_items() {
    // SVC-WORK-001: Given existing work with library items, deletes work
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "To Delete".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    // Seed a library item for this work
    use livrarr_db::{CreateLibraryItemDbRequest, LibraryItemDb, RootFolderDb};
    let rf = db2
        .create_root_folder("/tmp/test-library", livrarr_domain::MediaType::Ebook)
        .await
        .unwrap();
    db2.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id: added.work.id,
        root_folder_id: rf.id,
        path: "test/book.epub".into(),
        media_type: livrarr_domain::MediaType::Ebook,
        file_size: 1024,
        import_id: None,
    })
    .await
    .unwrap();

    // Verify library item exists before delete
    let items_before = db2
        .list_library_items_by_work(user_id, added.work.id)
        .await
        .unwrap();
    assert_eq!(items_before.len(), 1);

    svc.delete(user_id, added.work.id).await.unwrap();

    // Work is gone
    assert!(matches!(
        svc.get(user_id, added.work.id).await,
        Err(WorkServiceError::NotFound)
    ));

    // Library items are cascade-deleted by FK
    let items_after = db2
        .list_library_items_by_work(user_id, added.work.id)
        .await
        .unwrap();
    assert!(
        items_after.is_empty(),
        "library items should be deleted with work"
    );
}

#[tokio::test]
async fn test_work_delete_nonexistent_returns_not_found() {
    // SVC-WORK-001: Given nonexistent work, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let result = svc.delete(user_id, 99999).await;
    assert!(matches!(result, Err(WorkServiceError::NotFound)));
}

#[tokio::test]
async fn test_work_delete_missing_cover_still_ok() {
    // SVC-WORK-001: Given missing cover file, still returns Ok
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "No Cover".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    // Delete should succeed even without a cover file
    let result = svc.delete(user_id, added.work.id).await;
    assert!(result.is_ok());
}

// =============================================================================
// refresh
// =============================================================================

#[tokio::test]
async fn test_work_refresh_returns_updated_metadata() {
    // SVC-WORK-003: Given existing work, returns refreshed work
    use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        stub_http(),
        test_data_dir(),
    );

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Refresh Me".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let refreshed = svc.refresh(user_id, added.work.id).await.unwrap();
    assert_eq!(refreshed.work.id, added.work.id);
    assert_eq!(refreshed.work.user_id, user_id);
}

#[tokio::test]
async fn test_work_refresh_concurrent_waits_not_rejects() {
    // SVC-WORK-003: Given concurrent refresh, second caller waits and succeeds
    use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = Arc::new(WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        stub_http(),
        test_data_dir(),
    ));

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Concurrent".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let svc1 = svc.clone();
    let svc2 = svc.clone();
    let id = added.work.id;

    let (r1, r2) = tokio::join!(async move { svc1.refresh(user_id, id).await }, async move {
        svc2.refresh(user_id, id).await
    });

    assert!(r1.is_ok(), "first refresh should succeed");
    assert!(
        r2.is_ok(),
        "second concurrent refresh should also succeed (wait, not reject)"
    );
}

#[tokio::test]
async fn test_work_refresh_enrichment_failure_returns_error() {
    // SVC-WORK-003: Given enrichment failure, returns Enrichment error
    use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::failing(),
        stub_http(),
        test_data_dir(),
    );

    let added = svc
        .add(
            user_id,
            AddWorkRequest {
                title: "Will Fail Refresh".into(),
                author_name: "".into(),
                ol_key: None,
                detail_url: None,
                cover_url: None,
                author_ol_key: None,
                gr_key: None,
                year: None,
                metadata_source: None,
                language: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
                provenance_setter: None,
            },
        )
        .await
        .unwrap();

    let result = svc.refresh(user_id, added.work.id).await.unwrap();
    assert!(
        !result.messages.is_empty(),
        "expected enrichment failure message"
    );
    assert!(
        result.messages[0].contains("enrichment failed"),
        "expected enrichment failure in messages, got {:?}",
        result.messages
    );
}

#[tokio::test]
#[ignore = "pk-implement: requires provenance infrastructure to verify user-set fields preserved"]
async fn test_work_refresh_preserves_user_provenance() {
    // SVC-WORK-003: User-set provenance fields are preserved after refresh
    todo!("Setup: seed a work where fields have provenance=User")
}

// =============================================================================
// refresh_all
// =============================================================================

#[tokio::test]
async fn test_work_refresh_all_returns_immediately() {
    // SVC-WORK-003: Returns immediately with correct total_works count
    use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        stub_http(),
        test_data_dir(),
    );

    svc.add(
        user_id,
        AddWorkRequest {
            title: "Work 1".into(),
            author_name: "".into(),
            ol_key: None,
            detail_url: None,
            cover_url: None,
            author_ol_key: None,
            gr_key: None,
            year: None,
            metadata_source: None,
            language: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: None,
        },
    )
    .await
    .unwrap();
    svc.add(
        user_id,
        AddWorkRequest {
            title: "Work 2".into(),
            author_name: "".into(),
            ol_key: None,
            detail_url: None,
            cover_url: None,
            author_ol_key: None,
            gr_key: None,
            year: None,
            metadata_source: None,
            language: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: None,
        },
    )
    .await
    .unwrap();
    svc.add(
        user_id,
        AddWorkRequest {
            title: "Work 3".into(),
            author_name: "".into(),
            ol_key: None,
            detail_url: None,
            cover_url: None,
            author_ol_key: None,
            gr_key: None,
            year: None,
            metadata_source: None,
            language: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: None,
        },
    )
    .await
    .unwrap();

    let handle = svc.refresh_all(user_id).await.unwrap();
    assert_eq!(handle.total_works, 3);
}

#[tokio::test]
#[ignore = "pk-implement: requires background task spawning + failure tracking"]
async fn test_work_refresh_all_single_failure_continues() {
    // SVC-WORK-003: Single work failure does not abort the batch
    todo!("Setup: seed multiple works and stub refresh so one work fails")
}

// =============================================================================
// upload_cover
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: requires filesystem integration"]
async fn test_work_upload_cover_writes_and_sets_manual() {
    // SVC-WORK-001: Given valid bytes, cover is written and cover_manual set
    todo!("Setup: seed an existing work, provide valid image bytes")
}

#[tokio::test]
#[ignore = "pk-implement: requires filesystem integration"]
async fn test_work_upload_cover_oversized_returns_error() {
    // SVC-WORK-001: Given oversized bytes, returns CoverTooLarge
    todo!("Setup: seed an existing work and create oversized bytes")
}

#[tokio::test]
#[ignore = "pk-implement: requires filesystem integration"]
async fn test_work_upload_cover_nonexistent_returns_not_found() {
    // SVC-WORK-001: Given nonexistent work, returns NotFound
    todo!("Setup: ensure work_id does not exist for user_id")
}

// =============================================================================
// download_cover
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: requires filesystem integration"]
async fn test_work_download_cover_returns_bytes() {
    // SVC-WORK-001: Given existing cover, returns bytes
    todo!("Setup: seed an existing work with a valid stored cover path")
}

#[tokio::test]
#[ignore = "pk-implement: requires filesystem integration"]
async fn test_work_download_cover_no_file_returns_not_found() {
    // SVC-WORK-001: Given no cover file, returns NotFound
    todo!("Setup: seed an existing work with no cover path")
}

// =============================================================================
// lookup (future trait method)
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: WorkService::lookup() not yet added to domain trait"]
async fn test_work_lookup_empty_term_returns_empty_results() {
    todo!("Call lookup with LookupRequest {{ term: \"\".into(), lang_override: None }}. Assert Ok(vec![]) and no provider error.")
}

#[tokio::test]
#[ignore = "pk-implement: WorkService::lookup() not yet added to domain trait"]
async fn test_work_lookup_english_default_uses_openlibrary_and_parses_results() {
    todo!("Setup real service with stub HTTP returning OpenLibrary search JSON. Call lookup with English/default language. Assert parsed WorkSearchResult values.")
}

#[tokio::test]
#[ignore = "pk-implement: WorkService::lookup() not yet added to domain trait"]
async fn test_work_lookup_non_english_uses_goodreads_html_parse() {
    todo!("Setup service config or lang_override for non-English, stub Goodreads HTML response, assert regex-parsed results are returned. Ensure OpenLibrary is not used for foreign language.")
}

#[tokio::test]
#[ignore = "pk-implement: WorkService::lookup() not yet added to domain trait"]
async fn test_work_lookup_lang_override_takes_precedence_over_config_primary_language() {
    todo!("Configure primary language English, pass lang_override non-English, assert Goodreads branch used. Also cover inverse case if implementation supports both.")
}

#[tokio::test]
#[ignore = "pk-implement: WorkService::lookup() not yet added to domain trait"]
async fn test_work_lookup_openlibrary_empty_with_llm_fallback_returns_llm_results() {
    todo!("Stub OpenLibrary empty result + configured LLM stub success. Assert fallback results returned.")
}

#[tokio::test]
#[ignore = "pk-implement: WorkService::lookup() not yet added to domain trait"]
async fn test_work_lookup_degraded_provider_returns_empty_not_error() {
    todo!("Stub provider failure/degraded HTTP path. Assert lookup returns Ok(empty vec) rather than error, per graceful degradation contract.")
}
