#![allow(dead_code, unused_imports)]

//! Behavioral tests for SeriesService trait (SVC-SERIES-001..002).
//! Covers: fn.series_service.{list, get, refresh, monitor, update}

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateSeriesDbRequest, CreateUserDbRequest, SeriesDb, UserDb,
};
use livrarr_domain::services::*;
use livrarr_domain::UserRole;
use livrarr_metadata::series_service::SeriesServiceImpl;

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

async fn seed_series(db: &SqliteDb, user_id: i64, name: &str) -> livrarr_domain::Series {
    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: format!("Author for {name}"),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .unwrap();

    db.upsert_series(CreateSeriesDbRequest {
        user_id,
        author_id: author.id,
        name: name.into(),
        gr_key: format!("gr_{name}"),
        monitor_ebook: false,
        monitor_audiobook: false,
        monitor_language: None,
        work_count: 5,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn test_series_monitor_language_change_persists_without_restamp() {
    // AC-007 (series half): changing a monitored series' language persists and
    // governs future adds only; None leaves the setting untouched; the upsert
    // road (monitor action) also updates it.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let series = seed_series(&db, user_id, "Lang Series").await;
    assert_eq!(series.monitor_language, None);

    // Monitor action persists a concrete language via the upsert road.
    let monitored = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: series.author_id,
            name: series.name.clone(),
            gr_key: series.gr_key.clone(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("de".into()),
            work_count: 5,
        })
        .await
        .unwrap();
    assert_eq!(
        monitored.id, series.id,
        "upsert keys on (user, author, gr_key)"
    );
    assert_eq!(monitored.monitor_language.as_deref(), Some("de"));

    // The flag-update road changes the language when Some...
    let updated = db
        .update_series_flags(user_id, series.id, true, false, Some("fr".into()))
        .await
        .unwrap();
    assert_eq!(updated.monitor_language.as_deref(), Some("fr"));

    // ...and leaves it untouched when None (flag-only toggles never clear it).
    let untouched = db
        .update_series_flags(user_id, series.id, true, true, None)
        .await
        .unwrap();
    assert_eq!(untouched.monitor_language.as_deref(), Some("fr"));
}

#[tokio::test]
async fn test_series_list_returns_all_for_user() {
    // SVC-SERIES-001: Returns all series for user
    let db = create_test_db().await;
    let user_a = setup_user(&db).await;
    let user_b = setup_second_user(&db).await;

    seed_series(&db, user_a, "Cosmere").await;
    seed_series(&db, user_a, "Stormlight").await;
    seed_series(&db, user_b, "Other Series").await;

    let svc = SeriesServiceImpl::new(db);

    let list_a = svc.list(user_a).await.unwrap();
    assert_eq!(list_a.len(), 2);
    assert!(list_a.iter().all(|s| s.user_id == user_a));
}

#[tokio::test]
async fn test_series_get_existing_returns_series() {
    // SVC-SERIES-001: Given existing series, returns it
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let seeded = seed_series(&db, user_id, "Cosmere").await;

    let svc = SeriesServiceImpl::new(db);

    let result = svc.get(user_id, seeded.id).await;
    let series = result.expect("get should succeed");
    assert_eq!(series.id, seeded.id);
    assert_eq!(series.name, "Cosmere");
    assert_eq!(series.user_id, user_id);
}

#[tokio::test]
async fn test_series_get_nonexistent_returns_not_found() {
    // SVC-SERIES-001: Given nonexistent, returns NotFound
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let svc = SeriesServiceImpl::new(db);

    let result = svc.get(user_id, 99999).await;
    assert!(matches!(result, Err(SeriesServiceError::NotFound)));
}

#[tokio::test]
#[ignore = "pk-implement: requires Goodreads provider integration"]
async fn test_series_refresh_updates_from_goodreads() {
    // SVC-SERIES-002: Given available Goodreads, updates series membership
    todo!("Setup: seed a series with stale membership, stub Goodreads series response with updated ordered members. Call SeriesService::refresh(user_id, series_id). Assert: result.is_ok(); Goodreads provider is called once; returned/persisted series membership is reconciled to provider response (new members...")
}

#[tokio::test]
#[ignore = "pk-implement: requires Goodreads provider integration"]
async fn test_series_refresh_goodreads_unavailable_returns_error() {
    // SVC-SERIES-002: Given unavailable Goodreads, returns GoodreadsUnavailable
    todo!("Setup: seed a series and stub Goodreads provider failure/unavailable response. Call SeriesService::refresh(user_id, series_id). Assert: result is Err(GoodreadsUnavailable); existing series membership remains unchanged in DB.")
}

#[tokio::test]
async fn test_series_monitor_updates_state() {
    // SVC-SERIES-001: Setting monitored=true updates the series
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let seeded = seed_series(&db, user_id, "Cosmere").await;
    assert!(!seeded.monitor_ebook);

    let svc = SeriesServiceImpl::new(db);

    let updated = svc.monitor(user_id, seeded.id, true).await.unwrap();
    assert!(updated.monitor_ebook);
    assert!(updated.monitor_audiobook);

    let toggled = svc.monitor(user_id, seeded.id, false).await.unwrap();
    assert!(!toggled.monitor_ebook);
}

#[tokio::test]
#[ignore = "SPEC-GAP: SeriesDb lacks title update method — series names come from Goodreads. IR fn.series_service.update specifies title update but DB layer doesn't support it."]
async fn test_series_update_title_changes() {
    // SVC-SERIES-001: Given title update, title changes
    todo!("Requires SeriesDb title update method to be added first")
}
