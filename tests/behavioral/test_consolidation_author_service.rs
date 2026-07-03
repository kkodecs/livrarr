#![allow(dead_code, unused_imports)]

//! Behavioral tests for AuthorService trait (SVC-AUTHOR-001..003).
//! Covers: fn.author_service.{add, get, list, update, delete, search, bibliography, refresh_bibliography}

use livrarr_behavioral::stubs::{StubHttpFetcher, StubLlmCaller};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::*;
use livrarr_domain::UserRole;
use livrarr_metadata::author_service::AuthorServiceImpl;

fn make_svc(db: SqliteDb) -> AuthorServiceImpl<SqliteDb, StubHttpFetcher, StubLlmCaller> {
    AuthorServiceImpl::new(db, StubHttpFetcher::new(), StubLlmCaller::not_configured())
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

// =============================================================================
// add
// =============================================================================

#[tokio::test]
async fn test_author_add_creates_and_returns() {
    // SVC-AUTHOR-001: Given new author, creates and returns it
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let before_count = db2.list_authors(user_id).await.unwrap().len();

    let req = AddAuthorRequest {
        name: "  Brandon Sanderson  ".into(),
        ol_key: Some("/authors/OL123A".into()),
        monitored: true,
        sort_name: None,
    };
    let result = svc.add(user_id, req).await.expect("add should succeed");
    assert!(result.is_created());
    let author = result.author();

    assert!(author.id > 0);
    assert_eq!(author.user_id, user_id);
    assert_eq!(author.name, "Brandon Sanderson");
    assert_eq!(author.ol_key.as_deref(), Some("/authors/OL123A"));

    let after_count = db2.list_authors(user_id).await.unwrap().len();
    assert_eq!(
        after_count,
        before_count + 1,
        "author count should increase by 1"
    );
}

#[tokio::test]
async fn test_author_add_duplicate_upserts_existing() {
    // SVC-AUTHOR-001: Given duplicate name, upserts and returns Updated
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let first = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Brandon Sanderson".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();
    assert!(first.is_created());

    let count_before = db2.list_authors(user_id).await.unwrap().len();

    let result = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Brandon Sanderson".into(),
                ol_key: Some("/authors/OL999A".into()),
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    assert!(!result.is_created(), "expected Updated, got Created");
    assert_eq!(
        result.author().ol_key.as_deref(),
        Some("/authors/OL999A"),
        "ol_key should be updated on upsert"
    );

    let count_after = db2.list_authors(user_id).await.unwrap().len();
    assert_eq!(
        count_after, count_before,
        "author count should remain unchanged"
    );
}

// =============================================================================
// get
// =============================================================================

#[tokio::test]
async fn test_author_get_existing_returns_author() {
    // SVC-AUTHOR-001: Given existing author, returns it
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = make_svc(db);

    let added = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "N.K. Jemisin".into(),
                ol_key: Some("/authors/OL456A".into()),
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    let author = svc
        .get(user_id, added.author().id)
        .await
        .expect("get should succeed");
    assert_eq!(author.id, added.author().id);
    assert_eq!(author.user_id, user_id);
    assert_eq!(author.name, "N.K. Jemisin");
    assert_eq!(author.ol_key.as_deref(), Some("/authors/OL456A"));
}

#[tokio::test]
async fn test_author_get_wrong_user_returns_not_found() {
    // SVC-AUTHOR-001: Given nonexistent or wrong-user author, returns NotFound
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;
    let svc = make_svc(db);

    let added = svc
        .add(
            user_a,
            AddAuthorRequest {
                name: "Author A".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    let result = svc.get(user_b, added.author().id).await;
    assert!(
        matches!(result, Err(AuthorServiceError::NotFound)),
        "expected NotFound for wrong user, got {result:?}"
    );
}

// =============================================================================
// list
// =============================================================================

#[tokio::test]
async fn test_author_list_returns_all_for_user() {
    // SVC-AUTHOR-001: Returns all authors for user, empty vec if none
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;
    let svc = make_svc(db);

    let empty = svc.list(user_a).await.unwrap();
    assert!(empty.is_empty());

    let a1 = svc
        .add(
            user_a,
            AddAuthorRequest {
                name: "Author One".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();
    let a2 = svc
        .add(
            user_a,
            AddAuthorRequest {
                name: "Author Two".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();
    svc.add(
        user_b,
        AddAuthorRequest {
            name: "Other Author".into(),
            ol_key: None,
            monitored: false,
            sort_name: None,
        },
    )
    .await
    .unwrap();

    let list_a = svc.list(user_a).await.unwrap();
    assert_eq!(list_a.len(), 2);
    assert!(list_a.iter().all(|a| a.user_id == user_a));
    let ids: Vec<_> = list_a.iter().map(|a| a.id).collect();
    assert!(ids.contains(&a1.author().id), "should contain Author One");
    assert!(ids.contains(&a2.author().id), "should contain Author Two");
}

// =============================================================================
// update
// =============================================================================

#[tokio::test]
async fn test_author_update_name_changes() {
    // SVC-AUTHOR-001: Given name update, name changes
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = make_svc(db);

    let added = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Old Name".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    let updated = svc
        .update(
            user_id,
            added.author().id,
            UpdateAuthorRequest {
                name: Some("New Name".into()),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                monitor_new_items: None,
                monitored: None,
                monitor_language: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.id, added.author().id);
    assert_eq!(updated.name, "New Name");

    // Verify persisted via re-read
    let persisted = svc.get(user_id, added.author().id).await.unwrap();
    assert_eq!(persisted.name, "New Name");
}

#[tokio::test]
async fn test_author_update_none_name_unchanged() {
    // SVC-AUTHOR-001: Given None name, name unchanged
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = make_svc(db);

    let added = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Keep This Name".into(),
                ol_key: Some("/authors/OL789A".into()),
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    let updated = svc
        .update(
            user_id,
            added.author().id,
            UpdateAuthorRequest {
                name: None,
                sort_name: None,
                ol_key: None,
                gr_key: None,
                monitor_new_items: None,
                monitored: Some(true),
                monitor_language: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.name, "Keep This Name");

    // Verify persisted
    let persisted = svc.get(user_id, added.author().id).await.unwrap();
    assert_eq!(persisted.name, "Keep This Name");
}

#[tokio::test]
async fn test_author_update_nonexistent_returns_not_found() {
    // SVC-AUTHOR-001: Given nonexistent author, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let count_before = db2.list_authors(user_id).await.unwrap().len();

    let result = svc
        .update(
            user_id,
            99999,
            UpdateAuthorRequest {
                name: Some("X".into()),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                monitor_new_items: None,
                monitored: None,
                monitor_language: None,
            },
        )
        .await;

    assert!(matches!(result, Err(AuthorServiceError::NotFound)));

    let count_after = db2.list_authors(user_id).await.unwrap().len();
    assert_eq!(count_after, count_before, "no rows should be modified");
}

// =============================================================================
// delete
// =============================================================================

#[tokio::test]
async fn test_author_delete_existing() {
    // SVC-AUTHOR-001: Given existing author, deletes it; works remain
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let author = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "To Delete".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    // Seed a work referencing this author
    let _ = db2
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Orphan Work".into(),
            author_name: "To Delete".into(),
            author_id: Some(author.author().id),
            ..Default::default()
        })
        .await
        .unwrap();

    svc.delete(user_id, author.author().id)
        .await
        .expect("delete should succeed");

    // Author is gone
    assert!(matches!(
        svc.get(user_id, author.author().id).await,
        Err(AuthorServiceError::NotFound)
    ));

    // Work still exists (orphaned)
    let works = db2.list_works(user_id).await.unwrap();
    assert_eq!(works.len(), 1, "work should remain after author deletion");
}

#[tokio::test]
async fn test_author_delete_nonexistent_returns_not_found() {
    // SVC-AUTHOR-001: Given nonexistent author, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let count_before = db2.list_authors(user_id).await.unwrap().len();

    let result = svc.delete(user_id, 99999).await;
    assert!(matches!(result, Err(AuthorServiceError::NotFound)));

    let count_after = db2.list_authors(user_id).await.unwrap().len();
    assert_eq!(count_after, count_before, "DB state should be unchanged");
}

// =============================================================================
// search
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: requires HttpFetcher integration (Wave 2)"]
async fn test_author_search_returns_ol_results() {
    // SVC-AUTHOR-002: Given valid query, returns parsed OL results
    todo!("Setup: stub OpenLibrary author search HTTP response with valid payload for a query. Call AuthorService::search(user_id, query). Assert: result.is_ok(); returned vector length and parsed fields (name, ol_key/id, maybe work_count) match stub payload; outgoing request uses expected query; no DB muta...")
}

#[tokio::test]
#[ignore = "pk-implement: requires HttpFetcher integration (Wave 2)"]
async fn test_author_search_ol_429_returns_rate_limited() {
    // SVC-AUTHOR-002: Given OL 429, returns OlRateLimited
    todo!("Setup: stub OpenLibrary search endpoint to return HTTP 429. Call AuthorService::search(user_id, query). Assert: result is Err(OlRateLimited); error is mapped specifically from 429 rather than generic provider failure.")
}

// =============================================================================
// bibliography
// =============================================================================

#[tokio::test]
async fn test_author_bibliography_returns_entries() {
    // SVC-AUTHOR-002: Given author with cached bibliography, returns entries
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let author = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Test Author".into(),
                ol_key: Some("/authors/OL1A".into()),
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    use livrarr_db::AuthorBibliographyDb;
    let db_entries = vec![
        livrarr_db::BibliographyEntry {
            ol_key: Some("/works/OL10W".to_string()),
            title: "Book One".into(),
            year: Some(2020),
            series_name: None,
            series_position: None,
        },
        livrarr_db::BibliographyEntry {
            ol_key: Some("/works/OL20W".to_string()),
            title: "Book Two".into(),
            year: Some(2021),
            series_name: None,
            series_position: None,
        },
    ];
    db2.save_bibliography(author.author().id, &db_entries, None)
        .await
        .unwrap();

    let result = svc
        .bibliography(user_id, author.author().id, false)
        .await
        .unwrap();
    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.entries[0].title, "Book One");
    assert_eq!(result.entries[0].year, Some(2020));
    assert_eq!(result.entries[0].ol_key.as_deref(), Some("/works/OL10W"));
    assert_eq!(result.entries[1].title, "Book Two");
    assert_eq!(result.entries[1].year, Some(2021));
    assert_eq!(result.entries[1].ol_key.as_deref(), Some("/works/OL20W"));
}

#[tokio::test]
#[ignore = "pk-implement: requires HttpFetcher + LlmCaller integration"]
async fn test_author_bibliography_llm_failure_returns_unclean() {
    // SVC-AUTHOR-002: Given LLM failure, returns unclean bibliography
    todo!("Setup: stub bibliography provider success and LLM cleanup failure. Call AuthorService::bibliography(user_id, author_id). Assert: result.is_ok(); bibliography entries are still returned from raw/provider parsing; LLM failure does not become Err")
}

#[tokio::test]
async fn test_author_bibliography_already_in_library_flag() {
    // SVC-AUTHOR-002: already_in_library is true for entries matching existing works
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let author = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Test Author".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    let _ = db2
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Book One".into(),
            author_name: "Test Author".into(),
            author_id: Some(author.author().id),
            ..Default::default()
        })
        .await
        .unwrap();

    use livrarr_db::AuthorBibliographyDb;
    db2.save_bibliography(
        author.author().id,
        &[
            livrarr_db::BibliographyEntry {
                ol_key: Some("/works/OL10W".to_string()),
                title: "Book One".into(),
                year: Some(2020),
                series_name: None,
                series_position: None,
            },
            livrarr_db::BibliographyEntry {
                ol_key: Some("/works/OL20W".to_string()),
                title: "Unknown Book".into(),
                year: Some(2021),
                series_name: None,
                series_position: None,
            },
        ],
        None,
    )
    .await
    .unwrap();

    let result = svc
        .bibliography(user_id, author.author().id, false)
        .await
        .unwrap();
    assert_eq!(result.entries.len(), 2);

    let book_one = result
        .entries
        .iter()
        .find(|e| e.title == "Book One")
        .unwrap();
    assert!(book_one.already_in_library, "Book One should be in library");

    let unknown = result
        .entries
        .iter()
        .find(|e| e.title == "Unknown Book")
        .unwrap();
    assert!(
        !unknown.already_in_library,
        "Unknown Book should not be in library"
    );
}

// Phase 5 Unit E: pins the already-in-library seat under the identity
// authority (REQ-014, site 7 — author_service.rs's enrich_bibliography now
// routes normalize_title_for_match through identity_matching::parse_title).
#[tokio::test]
async fn test_author_bibliography_already_in_library_flag_across_a_subtitle() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let db2 = db.clone();
    let svc = make_svc(db);

    let author = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "Test Author".into(),
                ol_key: None,
                monitored: false,
                sort_name: None,
            },
        )
        .await
        .unwrap();

    // The library holds the bare title; the bibliography entry carries a
    // subtitle. Both must still resolve to the same main title.
    let _ = db2
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Storm Front".into(),
            author_name: "Test Author".into(),
            author_id: Some(author.author().id),
            ..Default::default()
        })
        .await
        .unwrap();

    use livrarr_db::AuthorBibliographyDb;
    db2.save_bibliography(
        author.author().id,
        &[livrarr_db::BibliographyEntry {
            ol_key: Some("/works/OL30W".to_string()),
            title: "Storm Front: The Dresden Files, Book 1".into(),
            year: Some(2000),
            series_name: None,
            series_position: None,
        }],
        None,
    )
    .await
    .unwrap();

    let result = svc
        .bibliography(user_id, author.author().id, false)
        .await
        .unwrap();
    let entry = result
        .entries
        .iter()
        .find(|e| e.title == "Storm Front: The Dresden Files, Book 1")
        .unwrap();
    assert!(
        entry.already_in_library,
        "a subtitled bibliography entry must still match the bare-title library work"
    );
}

// =============================================================================
// refresh_bibliography
// =============================================================================

#[tokio::test]
#[ignore = "pk-implement: requires provider integration for diff logic"]
async fn test_author_refresh_bibliography_returns_only_new() {
    // SVC-AUTHOR-003: Returns only entries not in previous bibliography
    todo!("Setup: seed stored previous bibliography snapshot/session for author and stub current provider bibliography containing old + newly added entries. Call AuthorService::refresh_bibliography(user_id, author_id). Assert: result.is_ok(); returned entries contain only items not present in previous bibli...")
}
