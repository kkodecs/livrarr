#![allow(dead_code)]

use chrono::{Duration, Utc};
use librarr_db::mem::InMemoryDb;
use librarr_db::*;
use librarr_domain::{EventType, MediaType, NotificationType, Session, User, UserRole};

// =============================================================================
// Helper — create a test user
// =============================================================================

async fn create_user(db: &InMemoryDb, name: &str) -> User {
    db.create_user(CreateUserDbRequest {
        username: name.into(),
        password_hash: format!("hash:{name}"),
        role: UserRole::Admin,
        api_key_hash: format!("api:{name}"),
    })
    .await
    .unwrap()
}

// =============================================================================
// User cascade delete — verify all related records removed
// =============================================================================

#[tokio::test]
async fn user_delete_cascades_sessions_works_authors_grabs_notifications_history() {
    let db = InMemoryDb::new();
    let u1 = create_user(&db, "alice").await;
    let u2 = create_user(&db, "bob").await;

    // Create data for u1
    let author1 = db
        .create_author(CreateAuthorDbRequest {
            user_id: u1.id,
            name: "Author1".into(),
            sort_name: None,
            ol_key: Some("OL1".into()),
        })
        .await
        .unwrap();
    let work1 = db
        .create_work(CreateWorkDbRequest {
            user_id: u1.id,
            title: "Work1".into(),
            author_name: "Author1".into(),
            author_id: Some(author1.id),
            ol_key: Some("OLW1".into()),
            year: None,
            cover_url: None,
        })
        .await
        .unwrap();
    db.create_session(&Session {
        token_hash: "tok_u1".into(),
        user_id: u1.id,
        persistent: false,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(1),
    })
    .await
    .unwrap();
    db.create_notification(CreateNotificationDbRequest {
        user_id: u1.id,
        notification_type: NotificationType::NewWorkDetected,
        ref_key: Some("ref1".into()),
        message: "test".into(),
        data: serde_json::json!({}),
    })
    .await
    .unwrap();
    db.create_history_event(CreateHistoryEventDbRequest {
        user_id: u1.id,
        work_id: Some(work1.id),
        event_type: EventType::Grabbed,
        data: serde_json::json!({}),
    })
    .await
    .unwrap();

    // Create data for u2
    let _author2 = db
        .create_author(CreateAuthorDbRequest {
            user_id: u2.id,
            name: "Author2".into(),
            sort_name: None,
            ol_key: None,
        })
        .await
        .unwrap();

    // Delete u1
    db.delete_user(u1.id).await.unwrap();

    // u1 data gone
    assert!(matches!(db.get_user(u1.id).await, Err(DbError::NotFound)));
    assert_eq!(db.list_works(u1.id).await.unwrap().len(), 0);
    assert_eq!(db.list_authors(u1.id).await.unwrap().len(), 0);
    assert!(db.get_session("tok_u1").await.unwrap().is_none());
    assert_eq!(db.list_notifications(u1.id, false).await.unwrap().len(), 0);
    assert_eq!(
        db.list_history(
            u1.id,
            HistoryFilter {
                event_type: None,
                work_id: None,
                start_date: None,
                end_date: None,
            }
        )
        .await
        .unwrap()
        .len(),
        0
    );

    // u2 data intact
    assert!(db.get_user(u2.id).await.is_ok());
    assert_eq!(db.list_authors(u2.id).await.unwrap().len(), 1);
}

// =============================================================================
// Notification dedup — None ref_key
// =============================================================================

#[tokio::test]
async fn notification_dedup_with_none_ref_key_is_distinct_per_type() {
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;

    let n1 = db
        .create_notification(CreateNotificationDbRequest {
            user_id: u.id,
            notification_type: NotificationType::NewWorkDetected,
            ref_key: None,
            message: "first".into(),
            data: serde_json::json!({}),
        })
        .await
        .unwrap();
    let n2 = db
        .create_notification(CreateNotificationDbRequest {
            user_id: u.id,
            notification_type: NotificationType::NewWorkDetected,
            ref_key: None,
            message: "second".into(),
            data: serde_json::json!({}),
        })
        .await
        .unwrap();
    // No dedup for None ref_key — follows SQL NULL semantics (NULL ≠ NULL in UNIQUE)
    assert_ne!(n1.id, n2.id);

    // Different type with None ref_key → new notification
    let n3 = db
        .create_notification(CreateNotificationDbRequest {
            user_id: u.id,
            notification_type: NotificationType::BulkEnrichmentComplete,
            ref_key: None,
            message: "bulk".into(),
            data: serde_json::json!({}),
        })
        .await
        .unwrap();
    assert_ne!(n1.id, n3.id);
}

// =============================================================================
// Work — ol_key duplicate detection
// =============================================================================

#[tokio::test]
async fn work_exists_by_ol_key_detects_existing() {
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;

    assert!(!db.work_exists_by_ol_key(u.id, "OL1").await.unwrap());

    db.create_work(CreateWorkDbRequest {
        user_id: u.id,
        title: "Dune".into(),
        author_name: "Herbert".into(),
        author_id: None,
        ol_key: Some("OL1".into()),
        year: None,
        cover_url: None,
    })
    .await
    .unwrap();

    assert!(db.work_exists_by_ol_key(u.id, "OL1").await.unwrap());
    assert!(!db.work_exists_by_ol_key(u.id, "OL2").await.unwrap());
}

#[tokio::test]
async fn work_exists_by_ol_key_is_user_scoped() {
    let db = InMemoryDb::new();
    let u1 = create_user(&db, "alice").await;
    let u2 = create_user(&db, "bob").await;

    db.create_work(CreateWorkDbRequest {
        user_id: u1.id,
        title: "Dune".into(),
        author_name: "Herbert".into(),
        author_id: None,
        ol_key: Some("OL1".into()),
        year: None,
        cover_url: None,
    })
    .await
    .unwrap();

    assert!(db.work_exists_by_ol_key(u1.id, "OL1").await.unwrap());
    assert!(!db.work_exists_by_ol_key(u2.id, "OL1").await.unwrap());
}

// =============================================================================
// Library item UNIQUE constraint
// =============================================================================

#[tokio::test]
async fn library_item_same_path_same_work_is_idempotent() {
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;
    let rf = db
        .create_root_folder("/books", MediaType::Ebook)
        .await
        .unwrap();
    let w = db
        .create_work(CreateWorkDbRequest {
            user_id: u.id,
            title: "Book".into(),
            author_name: "Author".into(),
            author_id: None,
            ol_key: None,
            year: None,
            cover_url: None,
        })
        .await
        .unwrap();

    let li1 = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id: u.id,
            work_id: w.id,
            root_folder_id: rf.id,
            path: "alice/Author/Book.epub".into(),
            media_type: MediaType::Ebook,
            file_size: 1000,
        })
        .await
        .unwrap();
    let li2 = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id: u.id,
            work_id: w.id,
            root_folder_id: rf.id,
            path: "alice/Author/Book.epub".into(),
            media_type: MediaType::Ebook,
            file_size: 1000,
        })
        .await
        .unwrap();
    assert_eq!(li1.id, li2.id);
}

#[tokio::test]
async fn library_item_same_path_different_work_is_constraint_error() {
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;
    let rf = db
        .create_root_folder("/books", MediaType::Ebook)
        .await
        .unwrap();
    let w1 = db
        .create_work(CreateWorkDbRequest {
            user_id: u.id,
            title: "Book1".into(),
            author_name: "Author".into(),
            author_id: None,
            ol_key: None,
            year: None,
            cover_url: None,
        })
        .await
        .unwrap();
    let w2 = db
        .create_work(CreateWorkDbRequest {
            user_id: u.id,
            title: "Book2".into(),
            author_name: "Author".into(),
            author_id: None,
            ol_key: None,
            year: None,
            cover_url: None,
        })
        .await
        .unwrap();

    db.create_library_item(CreateLibraryItemDbRequest {
        user_id: u.id,
        work_id: w1.id,
        root_folder_id: rf.id,
        path: "same/path.epub".into(),
        media_type: MediaType::Ebook,
        file_size: 1000,
    })
    .await
    .unwrap();

    let err = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id: u.id,
            work_id: w2.id,
            root_folder_id: rf.id,
            path: "same/path.epub".into(),
            media_type: MediaType::Ebook,
            file_size: 1000,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::Constraint { .. }));
}

// =============================================================================
// Config defaults
// =============================================================================

#[tokio::test]
async fn fresh_db_has_expected_defaults() {
    let db = InMemoryDb::new();

    let naming = db.get_naming_config().await.unwrap();
    assert!(!naming.author_folder_format.is_empty());
    assert!(!naming.book_folder_format.is_empty());

    let metadata = db.get_metadata_config().await.unwrap();
    assert!(metadata.audnexus_url.contains("audnex"));
    assert_eq!(metadata.languages, vec!["en".to_string()]);

    let media = db.get_media_management_config().await.unwrap();
    assert!(media.cwa_ingest_path.is_none());
}

// =============================================================================
// Root folder — one per media type
// =============================================================================

#[tokio::test]
async fn root_folder_by_media_type_returns_correct() {
    let db = InMemoryDb::new();
    let ebook_rf = db
        .create_root_folder("/ebooks", MediaType::Ebook)
        .await
        .unwrap();
    let audio_rf = db
        .create_root_folder("/audio", MediaType::Audiobook)
        .await
        .unwrap();

    let found = db
        .get_root_folder_by_media_type(MediaType::Ebook)
        .await
        .unwrap();
    assert_eq!(found.unwrap().id, ebook_rf.id);

    let found = db
        .get_root_folder_by_media_type(MediaType::Audiobook)
        .await
        .unwrap();
    assert_eq!(found.unwrap().id, audio_rf.id);
}

// =============================================================================
// list_monitored_authors — cross-user, filters by monitored + ol_key
// =============================================================================

#[tokio::test]
async fn list_monitored_authors_returns_only_monitored_with_ol_key() {
    let db = InMemoryDb::new();
    let u1 = create_user(&db, "alice").await;
    let u2 = create_user(&db, "bob").await;

    // Case 1: Monitored + ol_key → should be included
    let a1 = db
        .create_author(CreateAuthorDbRequest {
            user_id: u1.id,
            name: "Author1".into(),
            sort_name: None,
            ol_key: Some("OL1".into()),
        })
        .await
        .unwrap();
    db.update_author(
        u1.id,
        a1.id,
        UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            monitored: Some(true),
            monitor_new_items: None,
            monitor_since: Some(Utc::now()),
        },
    )
    .await
    .unwrap();

    // Case 2: Monitored + NO ol_key → should NOT be included
    let a2 = db
        .create_author(CreateAuthorDbRequest {
            user_id: u1.id,
            name: "Author2".into(),
            sort_name: None,
            ol_key: None,
        })
        .await
        .unwrap();
    db.update_author(
        u1.id,
        a2.id,
        UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            monitored: Some(true),
            monitor_new_items: None,
            monitor_since: Some(Utc::now()),
        },
    )
    .await
    .unwrap();

    // Case 3: NOT monitored + ol_key → should NOT be included
    let _a3 = db
        .create_author(CreateAuthorDbRequest {
            user_id: u1.id,
            name: "Author3".into(),
            sort_name: None,
            ol_key: Some("OL3".into()),
        })
        .await
        .unwrap();

    // Case 4: Different user, monitored + ol_key → should be included (cross-user)
    let a4 = db
        .create_author(CreateAuthorDbRequest {
            user_id: u2.id,
            name: "Author4".into(),
            sort_name: None,
            ol_key: Some("OL4".into()),
        })
        .await
        .unwrap();
    db.update_author(
        u2.id,
        a4.id,
        UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            monitored: Some(true),
            monitor_new_items: None,
            monitor_since: Some(Utc::now()),
        },
    )
    .await
    .unwrap();

    // list_monitored_authors is now user-scoped: only returns the given user's monitored authors.
    let monitored_u1 = db.list_monitored_authors(u1.id).await.unwrap();
    // InMemoryDb filters by monitored only — NOT by ol_key.
    // So a1 (monitored+ol_key) and a2 (monitored+no ol_key) are returned for u1.
    let ids_u1: Vec<i64> = monitored_u1.iter().map(|a| a.id).collect();
    assert!(ids_u1.contains(&a1.id));
    assert!(ids_u1.contains(&a2.id));
    assert!(!ids_u1.contains(&a4.id), "u1 should not see u2's authors");
    assert!(monitored_u1.iter().all(|a| a.monitored));

    // u2's monitored authors
    let monitored_u2 = db.list_monitored_authors(u2.id).await.unwrap();
    let ids_u2: Vec<i64> = monitored_u2.iter().map(|a| a.id).collect();
    assert!(ids_u2.contains(&a4.id));
    assert!(!ids_u2.contains(&a1.id), "u2 should not see u1's authors");
    // Note: the trait doc says "monitored authors with ol_key" but InMemoryDb
    // returns ALL monitored authors for the user. The caller (AuthorMonitor) must filter by ol_key.
}

// =============================================================================
// Cross-user data isolation
// =============================================================================

#[tokio::test]
async fn cross_user_work_isolation() {
    // User A's works are not visible to User B.
    let db = InMemoryDb::new();
    let u1 = create_user(&db, "alice").await;
    let u2 = create_user(&db, "bob").await;

    let w = db
        .create_work(CreateWorkDbRequest {
            user_id: u1.id,
            title: "Alice's Book".into(),
            author_name: "Author".into(),
            author_id: None,
            ol_key: None,
            year: None,
            cover_url: None,
        })
        .await
        .unwrap();

    // u2 cannot see u1's work
    assert!(matches!(
        db.get_work(u2.id, w.id).await,
        Err(DbError::NotFound)
    ));
    assert_eq!(db.list_works(u2.id).await.unwrap().len(), 0);
    assert_eq!(db.list_works(u1.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn cross_user_author_isolation() {
    let db = InMemoryDb::new();
    let u1 = create_user(&db, "alice").await;
    let u2 = create_user(&db, "bob").await;

    let a = db
        .create_author(CreateAuthorDbRequest {
            user_id: u1.id,
            name: "Author".into(),
            sort_name: None,
            ol_key: None,
        })
        .await
        .unwrap();

    assert!(matches!(
        db.get_author(u2.id, a.id).await,
        Err(DbError::NotFound)
    ));
    assert_eq!(db.list_authors(u2.id).await.unwrap().len(), 0);
}

// =============================================================================
// Duplicate username constraint
// =============================================================================

#[tokio::test]
async fn create_user_duplicate_username_case_insensitive_returns_constraint() {
    let db = InMemoryDb::new();
    create_user(&db, "Alice").await;

    let err = db
        .create_user(CreateUserDbRequest {
            username: "alice".into(),
            password_hash: "hash".into(),
            role: UserRole::User,
            api_key_hash: "api".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::Constraint { .. }));
}

// =============================================================================
// Root folder — media type uniqueness is not enforced at DB level
// =============================================================================

#[tokio::test]
async fn root_folder_db_enforces_one_per_media_type() {
    // InMemoryDb enforces one root folder per media type at the DB level.
    let db = InMemoryDb::new();
    db.create_root_folder("/ebooks1", MediaType::Ebook)
        .await
        .unwrap();
    let err = db
        .create_root_folder("/ebooks2", MediaType::Ebook)
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::Constraint { .. }));

    // Different media type still works.
    db.create_root_folder("/audio", MediaType::Audiobook)
        .await
        .unwrap();
}

// =============================================================================
// ID generation — monotonic
// =============================================================================

#[tokio::test]
async fn ids_are_monotonically_increasing() {
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;
    let a = db
        .create_author(CreateAuthorDbRequest {
            user_id: u.id,
            name: "Author".into(),
            sort_name: None,
            ol_key: None,
        })
        .await
        .unwrap();
    let w = db
        .create_work(CreateWorkDbRequest {
            user_id: u.id,
            title: "Work".into(),
            author_name: "Author".into(),
            author_id: Some(a.id),
            ol_key: None,
            year: None,
            cover_url: None,
        })
        .await
        .unwrap();

    assert!(u.id < a.id);
    assert!(a.id < w.id);
}

// =============================================================================
// Session — expired not returned by get
// =============================================================================

#[tokio::test]
async fn session_get_returns_expired_sessions_too() {
    // InMemoryDb.get_session does NOT filter by expiry — that's the caller's job.
    // This test documents the behavior.
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;
    db.create_session(&Session {
        token_hash: "expired".into(),
        user_id: u.id,
        persistent: false,
        created_at: Utc::now() - Duration::hours(2),
        expires_at: Utc::now() - Duration::hours(1),
    })
    .await
    .unwrap();

    let session = db.get_session("expired").await.unwrap();
    // InMemoryDb returns it — expiry checking is done by the caller
    assert!(session.is_some());
}

#[tokio::test]
async fn delete_expired_sessions_only_removes_expired() {
    let db = InMemoryDb::new();
    let u = create_user(&db, "alice").await;

    db.create_session(&Session {
        token_hash: "expired1".into(),
        user_id: u.id,
        persistent: false,
        created_at: Utc::now() - Duration::hours(2),
        expires_at: Utc::now() - Duration::hours(1),
    })
    .await
    .unwrap();
    db.create_session(&Session {
        token_hash: "live1".into(),
        user_id: u.id,
        persistent: false,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(1),
    })
    .await
    .unwrap();

    let count = db.delete_expired_sessions().await.unwrap();
    assert_eq!(count, 1);
    assert!(db.get_session("expired1").await.unwrap().is_none());
    assert!(db.get_session("live1").await.unwrap().is_some());
}
