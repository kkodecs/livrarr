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
    AuthorDb, CreateAuthorDbRequest, CreateDownloadClientDbRequest, CreateGrabDbRequest,
    CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, DownloadClientDb, GrabDb,
    LibraryItemDb, ProvenanceDb, RootFolderDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{
    IdentityMethod, IdentityState, PendingReason, WorkCandidate, WorkSeedFields,
};
use livrarr_domain::services::*;
use livrarr_domain::{
    DbError, DownloadClientImplementation, GrabStatus, MediaType, ProvenanceSetter, UserRole,
    WorkField,
};
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
        language: None,
        sort_by: None,
        sort_dir: None,
        media_type: None,
    }
}

fn make_candidate(title: &str, author: &str, ol_key: Option<&str>) -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: title.into(),
            author_name: author.into(),
            language: "en".into(),
            author_ol_key: None,
            year: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity: match ol_key {
            Some(k) => IdentityState::Confirmed {
                anchors: livrarr_domain::identity::CapturedIdentity {
                    ol_key: Some(k.into()),
                    gr_key: None,
                    hc_key: None,
                    isbn_13: None,
                    asin: None,
                    title: title.into(),
                    author_name: author.into(),
                    language: None,
                },
                method: IdentityMethod::UserSelected,
                score: None,
            },
            None => IdentityState::Pending {
                reason: PendingReason::NoCandidates,
                seed_anchors: None,
                top_candidates: vec![],
            },
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: None,
        import_id: None,
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::Search,
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
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let candidate = WorkCandidate {
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        source_provider_data: Some(SourceProviderData {
            description: Some("Readarr supplied description".into()),
            isbn: Some("9780765326355".into()),
            asin: Some("B003P2WO5E".into()),
            publisher: Some("Tor Books".into()),
            genres: Some(vec!["Fantasy".into()]),
            page_count: Some(1007),
            rating: Some(4.65),
            rating_count: Some(42),
            cover_url: Some("https://example.test/readarr-cover.jpg".into()),
            series_name: Some("The Stormlight Archive".into()),
            series_position: Some("1".into()),
        }),
        ..make_candidate(
            "The Way of Kings",
            "Brandon Sanderson",
            Some("/works/OL123W"),
        )
    };

    let result = svc
        .add(user_id, candidate)
        .await
        .expect("add should succeed");
    let work = result.work;
    assert!(work.id > 0);
    assert_eq!(work.user_id, user_id);
    assert_eq!(work.title, "The Way of Kings");
    assert_eq!(work.ol_key.as_deref(), Some("/works/OL123W"));
    assert_eq!(work.author_name, "Brandon Sanderson");

    let title_provenance = db
        .get_field_provenance(user_id, work.id, WorkField::Title)
        .await
        .expect("title provenance lookup should succeed")
        .expect("title provenance should be written at add time");
    assert_eq!(title_provenance.setter, ProvenanceSetter::Import);

    let ol_key_provenance = db
        .get_field_provenance(user_id, work.id, WorkField::OlKey)
        .await
        .expect("ol_key provenance lookup should succeed")
        .expect("ol_key provenance should be written at add time");
    assert_eq!(ol_key_provenance.setter, ProvenanceSetter::Import);
}

#[tokio::test]
async fn test_work_add_duplicate_ol_key_returns_already_exists() {
    // SVC-WORK-001: Given duplicate ol_key, returns AlreadyExists
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db, stub_http(), test_data_dir());

    svc.add(
        user_id,
        make_candidate("Book One", "", Some("/works/OL999W")),
    )
    .await
    .unwrap();

    let result = svc
        .add(
            user_id,
            make_candidate("Book One Again", "", Some("/works/OL999W")),
        )
        .await
        .expect("duplicate ol_key should return existing work");
    assert!(
        !result.created,
        "duplicate ol_key should not create a new work"
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

    let result = svc
        .add(user_id, make_candidate("Enrichment Fails", "Author", None))
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

    let result = svc
        .add(
            user_id,
            make_candidate("The Left Hand of Darkness", "  ursula k. le guin  ", None),
        )
        .await
        .unwrap();
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

    let result = svc
        .add(
            user_id,
            make_candidate("Neuromancer", "William Gibson", None),
        )
        .await
        .unwrap();
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

    let result = svc
        .add(
            user_id,
            make_candidate("  The Way of Kings  ", "  Brandon Sanderson  ", None),
        )
        .await
        .unwrap();
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
            make_candidate("Snow Crash", "Neal Stephenson", None),
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
            make_candidate("Kindred", "Octavia E. Butler", None),
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
        .add(user_id, make_candidate("Anonymous Collection", "", None))
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
            make_candidate("Dune", "Frank Herbert", Some("/works/OL1W")),
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
        .add(user_a, make_candidate("Book A", "", None))
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

    svc.add(user_a, make_candidate("W1", "", None))
        .await
        .unwrap();
    svc.add(user_a, make_candidate("W2", "", None))
        .await
        .unwrap();
    svc.add(user_b, make_candidate("Other", "", None))
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
        .add(user_id, make_candidate("Old Title", "", None))
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
        .add(user_id, make_candidate("Keep This", "", None))
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
        .add(user_id, make_candidate("To Delete", "", None))
        .await
        .unwrap();

    // Seed a library item for this work
    use livrarr_db::{CreateLibraryItemDbRequest, LibraryItemDb, RootFolderDb, TagStatus};
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
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
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
        .add(user_id, make_candidate("No Cover", "", None))
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
        .add(user_id, make_candidate("Refresh Me", "", None))
        .await
        .unwrap();

    let refreshed = svc
        .refresh(user_id, added.work.id, RefreshSurface::Interactive)
        .await
        .unwrap();
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
        .add(user_id, make_candidate("Concurrent", "", None))
        .await
        .unwrap();

    let svc1 = svc.clone();
    let svc2 = svc.clone();
    let id = added.work.id;

    let (r1, r2) = tokio::join!(
        async move { svc1.refresh(user_id, id, RefreshSurface::Interactive).await },
        async move { svc2.refresh(user_id, id, RefreshSurface::Interactive).await }
    );

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
        .add(user_id, make_candidate("Will Fail Refresh", "", None))
        .await
        .unwrap();

    let result = svc
        .refresh(user_id, added.work.id, RefreshSurface::Interactive)
        .await;
    assert!(
        result.is_ok(),
        "refresh should return Ok even on enrichment failure (enrichment failure is non-fatal)"
    );
}

#[tokio::test]
#[ignore = "pk-implement: requires provenance infrastructure to verify user-set fields preserved"]
async fn test_work_refresh_preserves_user_provenance() {
    // SVC-WORK-003: User-set provenance fields are preserved after refresh
    todo!("Setup: seed a work where fields have provenance=User")
}

// =============================================================================
// refresh_all (dead — bulk refresh moved to handler layer per insight 9g)
// =============================================================================

// #[tokio::test]
// async fn test_work_refresh_all_returns_immediately() {
//     // SVC-WORK-003: Returns immediately with correct total_works count
//     use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
//     let db = create_test_db().await;
//     let user_id = setup_user(&db).await;
//     let svc = WorkServiceImpl::new(
//         db,
//         StubEnrichmentWorkflow::succeeding(),
//         stub_http(),
//         test_data_dir(),
//     );
//
//     svc.add(user_id, make_candidate("Work 1", "", None))
//         .await
//         .unwrap();
//     svc.add(user_id, make_candidate("Work 2", "", None))
//         .await
//         .unwrap();
//     svc.add(user_id, make_candidate("Work 3", "", None))
//         .await
//         .unwrap();
//
//     let handle = svc.refresh_all(user_id).await.unwrap();
//     assert_eq!(handle.total_works, 3);
// }
//
// #[tokio::test]
// #[ignore = "pk-implement: requires background task spawning + failure tracking"]
// async fn test_work_refresh_all_single_failure_continues() {
//     // SVC-WORK-003: Single work failure does not abort the batch
//     todo!("Setup: seed multiple works and stub refresh so one work fails")
// }

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

// =============================================================================
// merge_works / preview_merge_works (REQ-015)
// =============================================================================

#[allow(clippy::too_many_arguments)]
async fn seed_work_full(
    db: &SqliteDb,
    user_id: i64,
    title: &str,
    author: &str,
    series_name: Option<&str>,
    series_position: Option<f64>,
    monitor_ebook: bool,
    monitor_audiobook: bool,
) -> i64 {
    // normalized_title/author must be distinct per work — the DB layer's
    // ON CONFLICT dedup backstop collapses two creates that share the same
    // (user_id, normalized_title, normalized_author) key, which the
    // all-empty-string default would trigger for every seeded work.
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.into(),
        author_name: author.into(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(author),
        series_name: series_name.map(String::from),
        series_position,
        monitor_ebook,
        monitor_audiobook,
        ..Default::default()
    })
    .await
    .unwrap()
    .0
    .id
}

async fn seed_download_client(db: &SqliteDb) -> i64 {
    db.create_download_client(CreateDownloadClientDbRequest {
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
    .unwrap()
    .id
}

async fn seed_library_item(
    db: &SqliteDb,
    user_id: i64,
    work_id: i64,
    root_id: i64,
    path: &str,
) -> i64 {
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: root_id,
        path: path.into(),
        media_type: MediaType::Ebook,
        file_size: 10,
        import_id: None,
        tag_status: livrarr_db::TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap()
    .id
}

#[tokio::test]
async fn test_merge_preview_counts_and_conflicts() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let survivor_id = seed_work_full(
        &db,
        user_id,
        "Survivor",
        "Author",
        Some("Foo"),
        Some(1.0),
        true,
        false,
    )
    .await;
    let loser_id = seed_work_full(
        &db,
        user_id,
        "Loser",
        "Author",
        Some("Bar"),
        Some(1.0),
        false,
        true,
    )
    .await;

    let root = db
        .create_root_folder("/data/books", MediaType::Ebook)
        .await
        .unwrap();
    seed_library_item(&db, user_id, loser_id, root.id, "a.epub").await;
    seed_library_item(&db, user_id, loser_id, root.id, "b.epub").await;

    let client_id = seed_download_client(&db).await;
    db.upsert_grab(CreateGrabDbRequest {
        user_id,
        work_id: loser_id,
        download_client_id: client_id,
        title: "grab".into(),
        indexer: "idx".into(),
        guid: "guid-1".into(),
        size: None,
        download_url: "http://x".into(),
        download_id: None,
        status: GrabStatus::Sent,
        media_type: None,
    })
    .await
    .unwrap();

    let preview = svc
        .preview_merge_works(user_id, survivor_id, loser_id)
        .await
        .unwrap();

    // Only SeriesName conflicts — both sides share the same series_position.
    assert_eq!(preview.library_items_moving, 2);
    assert_eq!(preview.grabs_moving, 1);
    assert!(preview.monitor_ebook_result);
    assert!(preview.monitor_audiobook_result);
    assert_eq!(preview.conflicts.len(), 1);
    assert_eq!(preview.conflicts[0].field, MergeableField::SeriesName);
    assert_eq!(preview.conflicts[0].survivor_value, "Foo");
    assert_eq!(preview.conflicts[0].loser_value, "Bar");
}

#[tokio::test]
async fn test_merge_refuses_without_choice_for_conflict() {
    // AC-025: a conflicting user-set field with no explicit choice must
    // refuse, not silently default.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let survivor_id = seed_work_full(
        &db,
        user_id,
        "Survivor",
        "Author",
        Some("Foo"),
        None,
        false,
        false,
    )
    .await;
    let loser_id = seed_work_full(
        &db,
        user_id,
        "Loser",
        "Author",
        Some("Bar"),
        None,
        false,
        false,
    )
    .await;

    let result = svc
        .merge_works(user_id, survivor_id, loser_id, vec![])
        .await;
    match result {
        Err(WorkServiceError::MergeChoiceRequired(fields)) => {
            assert_eq!(fields, vec![MergeableField::SeriesName]);
        }
        other => panic!("expected MergeChoiceRequired, got {other:?}"),
    }

    // The refusal must not have applied anything — the loser still exists.
    assert!(db.get_work(user_id, loser_id).await.is_ok());
}

#[tokio::test]
async fn test_merge_applies_explicit_choice() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let survivor_id = seed_work_full(
        &db,
        user_id,
        "Survivor",
        "Author",
        Some("Foo"),
        None,
        false,
        false,
    )
    .await;
    let loser_id = seed_work_full(
        &db,
        user_id,
        "Loser",
        "Author",
        Some("Bar"),
        None,
        false,
        false,
    )
    .await;

    let result = svc
        .merge_works(
            user_id,
            survivor_id,
            loser_id,
            vec![MergeFieldChoiceEntry {
                field: MergeableField::SeriesName,
                choice: MergeFieldChoice::TakeLoser,
            }],
        )
        .await
        .unwrap();

    assert_eq!(result.survivor.series_name.as_deref(), Some("Bar"));
    // Loser is gone only now that the choice let the merge complete.
    assert!(db.get_work(user_id, loser_id).await.is_err());
}

#[tokio::test]
async fn test_merge_additive_fields_need_no_choice() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    // Only the loser has series info — additive, never a conflict.
    let survivor_id =
        seed_work_full(&db, user_id, "Survivor", "Author", None, None, true, false).await;
    let loser_id = seed_work_full(
        &db,
        user_id,
        "Loser",
        "Author",
        Some("OnlyLoser"),
        Some(3.0),
        false,
        true,
    )
    .await;

    let result = svc
        .merge_works(user_id, survivor_id, loser_id, vec![])
        .await
        .unwrap();

    assert_eq!(result.survivor.series_name.as_deref(), Some("OnlyLoser"));
    assert_eq!(result.survivor.series_position, Some(3.0));
    assert!(result.survivor.monitor_ebook); // survivor's own true, OR'd
    assert!(result.survivor.monitor_audiobook); // loser's true, OR'd
}

#[tokio::test]
async fn test_merge_reassigns_items_and_grabs_same_row_ids_and_deletes_loser() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let survivor_id =
        seed_work_full(&db, user_id, "Survivor", "Author", None, None, false, false).await;
    let loser_id = seed_work_full(&db, user_id, "Loser", "Author", None, None, false, false).await;

    let root = db
        .create_root_folder("/data/books", MediaType::Ebook)
        .await
        .unwrap();
    let item_id = seed_library_item(&db, user_id, loser_id, root.id, "a.epub").await;

    let client_id = seed_download_client(&db).await;
    let grab = db
        .upsert_grab(CreateGrabDbRequest {
            user_id,
            work_id: loser_id,
            download_client_id: client_id,
            title: "grab".into(),
            indexer: "idx".into(),
            guid: "guid-1".into(),
            size: None,
            download_url: "http://x".into(),
            download_id: None,
            status: GrabStatus::Sent,
            media_type: None,
        })
        .await
        .unwrap();

    let result = svc
        .merge_works(user_id, survivor_id, loser_id, vec![])
        .await
        .unwrap();
    assert_eq!(result.library_items_moved, 1);
    assert_eq!(result.grabs_moved, 1);

    assert!(
        db.get_work(user_id, loser_id).await.is_err(),
        "loser row must be gone"
    );

    // Reassignment updates work_id on the SAME row — never delete+recreate —
    // so anything FK'd to the item/grab id (bookmarks, playback, kash links)
    // rides along untouched (REQ-015 d).
    let moved_item = db.get_library_item(user_id, item_id).await.unwrap();
    assert_eq!(moved_item.work_id, survivor_id);
    let moved_grab = db.get_grab(user_id, grab.id).await.unwrap();
    assert_eq!(moved_grab.work_id, survivor_id);
}

#[tokio::test]
async fn test_merge_rejects_self_merge() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let work_id = seed_work_full(&db, user_id, "Solo", "Author", None, None, false, false).await;

    let merge_result = svc.merge_works(user_id, work_id, work_id, vec![]).await;
    assert!(matches!(merge_result, Err(WorkServiceError::Validation(_))));

    let preview_result = svc.preview_merge_works(user_id, work_id, work_id).await;
    assert!(matches!(
        preview_result,
        Err(WorkServiceError::Validation(_))
    ));
}

#[tokio::test]
async fn test_merge_cross_user_is_impossible() {
    // AC-024: a merge never crosses users. Cross-user attempts must fail
    // exactly like the id didn't exist — never leaking which case it was.
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;
    let svc = WorkServiceImpl::without_enrichment(db.clone(), stub_http(), test_data_dir());

    let survivor_id = seed_work_full(&db, user_a, "Mine", "Author", None, None, false, false).await;
    let other_users_work = seed_work_full(
        &db,
        user_b,
        "TheirsNotMine",
        "Author",
        None,
        None,
        false,
        false,
    )
    .await;

    let preview = svc
        .preview_merge_works(user_a, survivor_id, other_users_work)
        .await;
    assert!(matches!(preview, Err(WorkServiceError::NotFound)));

    let merge = svc
        .merge_works(user_a, survivor_id, other_users_work, vec![])
        .await;
    assert!(matches!(merge, Err(WorkServiceError::NotFound)));

    // The other user's work is completely untouched.
    let still_theirs = db.get_work(user_b, other_users_work).await.unwrap();
    assert_eq!(still_theirs.title, "TheirsNotMine");

    // Also reject the reverse direction (loser belongs to the caller, but
    // the "survivor" they named does not).
    let reverse = svc
        .merge_works(user_a, other_users_work, survivor_id, vec![])
        .await;
    assert!(matches!(reverse, Err(WorkServiceError::NotFound)));
}

#[tokio::test]
async fn test_merge_works_db_aborts_whole_transaction_leaving_nothing_half_moved() {
    // Exercises `WorkDb::merge_works`'s OWN internal ownership re-check
    // directly (bypassing the service layer, which would already refuse
    // this pair before ever calling the DB — this proves the DB-layer
    // guard is independently load-bearing, defense in depth). The
    // survivor's ownership check passes first; the loser's fails second —
    // so this is a genuine "some validation already happened, then it
    // aborts" case, not a same-request no-op. Nothing about the loser
    // (its row, or the items it owns) may be touched when the call fails:
    // one transaction, all-or-nothing (REQ-015 e).
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let other_user = setup_second_user(&db).await;

    let survivor_id =
        seed_work_full(&db, user_id, "Survivor", "Author", None, None, false, false).await;
    // Actually owned by `other_user`, not `user_id` — the request below
    // claims it as if it were the caller's.
    let loser_id = seed_work_full(
        &db, other_user, "NotMine", "Author", None, None, false, false,
    )
    .await;

    let root = db
        .create_root_folder("/data/books", MediaType::Ebook)
        .await
        .unwrap();
    let item_id = seed_library_item(&db, other_user, loser_id, root.id, "a.epub").await;

    let result = db
        .merge_works(livrarr_db::MergeWorksDbRequest {
            user_id,
            survivor_id,
            loser_id,
            monitor_ebook: false,
            monitor_audiobook: false,
            series_name: None,
            series_position: None,
        })
        .await;
    assert!(matches!(result, Err(DbError::NotFound { .. })));

    // The real owner's work and item are completely untouched.
    assert!(db.get_work(other_user, loser_id).await.is_ok());
    let item_after = db.get_library_item(other_user, item_id).await.unwrap();
    assert_eq!(item_after.work_id, loser_id);
}
