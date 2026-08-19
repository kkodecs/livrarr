use std::collections::HashSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateDownloadClientDbRequest, CreateGrabDbRequest, CreateHistoryEventDbRequest,
    CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, DownloadClientDb, GrabDb,
    HistoryDb, LibraryItemDb, RootFolderDb, TagStatus, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    normalize_for_matching, DownloadClientImplementation, EventType, Grab, GrabStatus,
    HistoryEvent, HistoryFilter, LibraryItem, MediaType, UserId, UserRole, Work, WorkId,
};
use livrarr_server::jobs::history_backfill::run_history_backfill;
use serde_json::{json, Value};

fn dt(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 2, 3, hour, minute, second)
        .single()
        .expect("valid fixture datetime")
}

fn empty_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn event_type_str(event_type: EventType) -> &'static str {
    match event_type {
        EventType::Grabbed => "grabbed",
        EventType::DownloadCompleted => "downloadCompleted",
        EventType::DownloadFailed => "downloadFailed",
        EventType::Imported => "imported",
        EventType::ImportFailed => "importFailed",
        EventType::Enriched => "enriched",
        EventType::EnrichmentFailed => "enrichmentFailed",
        EventType::TagWritten => "tagWritten",
        EventType::TagWriteFailed => "tagWriteFailed",
        EventType::FileDeleted => "fileDeleted",
        EventType::Added => "added",
        EventType::WorkDeleted => "workDeleted",
        EventType::WorksMerged => "worksMerged",
        EventType::IdentityResolved => "identityResolved",
    }
}

fn grab_status_str(status: GrabStatus) -> &'static str {
    match status {
        GrabStatus::Sent => "sent",
        GrabStatus::Confirmed => "confirmed",
        GrabStatus::Importing => "importing",
        GrabStatus::Imported => "imported",
        GrabStatus::ImportFailed => "importFailed",
        GrabStatus::Removed => "removed",
        GrabStatus::Failed => "failed",
    }
}

async fn seed_user(db: &SqliteDb, username: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: username.to_string(),
        password_hash: "hash".to_string(),
        role: UserRole::Admin,
        api_key_hash: format!("{username}-api-key"),
    })
    .await
    .expect("seed user")
    .id
}

async fn seed_work(
    db: &SqliteDb,
    user_id: UserId,
    title: &str,
    author: &str,
    added_at: DateTime<Utc>,
    enriched_at: Option<DateTime<Utc>>,
    enrichment_source: Option<&str>,
) -> Work {
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: title.to_string(),
            author_name: author.to_string(),
            normalized_title: normalize_for_matching(title),
            normalized_author: normalize_for_matching(author),
            language: Some("en".to_string()),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..Default::default()
        })
        .await
        .expect("seed work");
    assert!(created, "fixture work titles must be unique");

    sqlx::query(
        "UPDATE works SET added_at = ?, enriched_at = ?, enrichment_source = ? WHERE id = ?",
    )
    .bind(added_at.to_rfc3339())
    .bind(enriched_at.map(|d| d.to_rfc3339()))
    .bind(enrichment_source)
    .bind(work.id)
    .execute(db.pool())
    .await
    .expect("stamp work fact dates");

    db.get_work(user_id, work.id)
        .await
        .expect("read seeded work")
}

/// Grabs carry a NOT NULL foreign key to `download_clients`
/// (001_initial_schema.sql:195), so a fixture client must exist before any
/// grab row can be inserted. Get-or-create keeps repeated `seed_grab` calls
/// on the same DB cheap and collision-free.
async fn seed_download_client(db: &SqliteDb) -> i64 {
    if let Some(existing) = db
        .list_download_clients()
        .await
        .expect("list download clients")
        .into_iter()
        .next()
    {
        return existing.id;
    }
    db.create_download_client(CreateDownloadClientDbRequest {
        name: "fixture-client".to_string(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "localhost".to_string(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: None,
        password: None,
        category: "books".to_string(),
        download_dir: None,
        enabled: true,
        api_key: None,
    })
    .await
    .expect("seed download client")
    .id
}

#[allow(clippy::too_many_arguments)] // fixture helper mirrors the grab fact's full column set
async fn seed_grab(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    release_title: &str,
    guid: &str,
    status: GrabStatus,
    import_error: Option<&str>,
    grabbed_at: DateTime<Utc>,
) -> Grab {
    let grab = db
        .upsert_grab(CreateGrabDbRequest {
            user_id,
            work_id,
            download_client_id: seed_download_client(db).await,
            title: release_title.to_string(),
            indexer: "fixture-indexer".to_string(),
            guid: guid.to_string(),
            size: Some(42),
            download_url: format!("https://indexer.invalid/{guid}"),
            download_id: Some(format!("download-{guid}")),
            status,
            media_type: Some(MediaType::Ebook),
        })
        .await
        .expect("seed grab");

    sqlx::query("UPDATE grabs SET status = ?, import_error = ?, grabbed_at = ? WHERE id = ?")
        .bind(grab_status_str(status))
        .bind(import_error)
        .bind(grabbed_at.to_rfc3339())
        .bind(grab.id)
        .execute(db.pool())
        .await
        .expect("stamp grab fact date and status");

    db.get_grab(user_id, grab.id)
        .await
        .expect("read seeded grab")
}

async fn seed_item(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    root_folder_id: i64,
    path: &str,
    imported_at: DateTime<Utc>,
) -> LibraryItem {
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id,
            path: path.to_string(),
            media_type: MediaType::Ebook,
            file_size: 1234,
            import_id: None,
            tag_status: TagStatus::Synced,
            tagged_at_generation: 0,
        })
        .await
        .expect("seed library item");

    sqlx::query("UPDATE library_items SET imported_at = ? WHERE id = ?")
        .bind(imported_at.to_rfc3339())
        .bind(item.id)
        .execute(db.pool())
        .await
        .expect("stamp item import date");

    db.get_library_item(user_id, item.id)
        .await
        .expect("read seeded item")
}

async fn insert_history_raw(
    db: &SqliteDb,
    user_id: UserId,
    work_id: Option<WorkId>,
    event_type: EventType,
    data: Value,
    date: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO history (user_id, work_id, event_type, data, date) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(event_type_str(event_type))
    .bind(data.to_string())
    .bind(date.to_rfc3339())
    .execute(db.pool())
    .await
    .expect("seed pre-existing history row");
}

async fn marker_value(db: &SqliteDb) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = 'history_backfill_generation'")
        .fetch_optional(db.pool())
        .await
        .expect("read backfill marker")
}

async fn history(db: &SqliteDb, user_id: UserId) -> Vec<HistoryEvent> {
    db.list_history(user_id, empty_filter())
        .await
        .expect("list history")
}

fn backfilled(events: &[HistoryEvent], event_type: EventType) -> Vec<&HistoryEvent> {
    events
        .iter()
        .filter(|event| {
            event.event_type == event_type
                && event
                    .data
                    .get("backfilled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .collect()
}

fn backfilled_titles(events: &[HistoryEvent], event_type: EventType) -> HashSet<String> {
    backfilled(events, event_type)
        .into_iter()
        .map(|event| {
            event
                .data
                .get("title")
                .or_else(|| event.data.get("path"))
                .or_else(|| event.data.get("work_title"))
                .and_then(Value::as_str)
                .expect("backfilled payload identity")
                .to_string()
        })
        .collect()
}

fn assert_event_date(
    events: &[HistoryEvent],
    event_type: EventType,
    key: &str,
    value: &str,
    expected: DateTime<Utc>,
) {
    let event = events
        .iter()
        .find(|event| {
            event.event_type == event_type
                && event.data.get("backfilled").and_then(Value::as_bool) == Some(true)
                && event.data.get(key).and_then(Value::as_str) == Some(value)
        })
        .unwrap_or_else(|| panic!("missing {event_type:?} backfill row with {key}={value}"));
    assert_eq!(event.date, expected);
}

#[tokio::test]
async fn wh_backfill_full_fixture_synthesizes_only_uncovered_facts() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-full-user").await;
    let other_user = seed_user(&db, "wh-full-other-user").await;
    let root = db
        .create_root_folder("/tmp/wh-full-ebooks", MediaType::Ebook)
        .await
        .expect("seed root folder");

    let alpha = seed_work(
        &db,
        user,
        "WH Alpha",
        "Author A",
        dt(1, 0, 0),
        Some(dt(2, 0, 0)),
        Some("goodreads"),
    )
    .await;
    let beta = seed_work(&db, user, "WH Beta", "Author B", dt(3, 0, 0), None, None).await;
    let gamma = seed_work(
        &db,
        user,
        "WH Gamma",
        "Author C",
        dt(4, 0, 0),
        Some(dt(5, 0, 0)),
        Some("google-books"),
    )
    .await;
    let foreign = seed_work(
        &db,
        other_user,
        "WH Other User",
        "Author X",
        dt(6, 0, 0),
        Some(dt(7, 0, 0)),
        Some("other-source"),
    )
    .await;

    seed_grab(
        &db,
        user,
        alpha.id,
        "Alpha Imported Release",
        "guid-alpha-imported",
        GrabStatus::Imported,
        None,
        dt(8, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        alpha.id,
        "Alpha Download Failed Release",
        "guid-alpha-failed",
        GrabStatus::Failed,
        Some("download client reported failure"),
        dt(9, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        beta.id,
        "Beta Import Failed Release",
        "guid-beta-import-failed",
        GrabStatus::ImportFailed,
        Some("no supported files"),
        dt(10, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        beta.id,
        "Beta Removed Release",
        "guid-beta-removed",
        GrabStatus::Removed,
        None,
        dt(11, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        gamma.id,
        "Gamma In Flight Release",
        "guid-gamma-sent",
        GrabStatus::Sent,
        None,
        dt(12, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        gamma.id,
        "Gamma Real Covered Release",
        "guid-gamma-covered",
        GrabStatus::Imported,
        None,
        dt(13, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        other_user,
        foreign.id,
        "Other User Release",
        "guid-other-user",
        GrabStatus::Imported,
        None,
        dt(14, 0, 0),
    )
    .await;

    let covered_item = seed_item(
        &db,
        user,
        alpha.id,
        root.id,
        "WH Alpha/covered.epub",
        dt(15, 0, 0),
    )
    .await;
    let uncovered_item = seed_item(
        &db,
        user,
        beta.id,
        root.id,
        "WH Beta/uncovered.epub",
        dt(16, 0, 0),
    )
    .await;
    let manual_outside_item = seed_item(
        &db,
        user,
        gamma.id,
        root.id,
        "WH Gamma/manual-outside.epub",
        dt(18, 10, 0),
    )
    .await;
    seed_item(
        &db,
        other_user,
        foreign.id,
        root.id,
        "WH Other User/foreign.epub",
        dt(19, 0, 0),
    )
    .await;

    insert_history_raw(
        &db,
        user,
        Some(alpha.id),
        EventType::Imported,
        json!({
            "title": "Alpha Imported Release",
            "imported": 1,
            "failed": 0,
            "skipped": 0,
            "work_title": alpha.title,
        }),
        covered_item.imported_at + Duration::minutes(20),
    )
    .await;
    insert_history_raw(
        &db,
        user,
        Some(gamma.id),
        EventType::Grabbed,
        json!({
            "title": "Gamma Real Covered Release",
            "indexer": "fixture-indexer",
            "guid": "guid-gamma-covered",
            "download_client_id": 1,
            "work_title": gamma.title,
        }),
        dt(13, 0, 1),
    )
    .await;

    run_history_backfill(db.clone()).await;

    let rows = history(&db, user).await;
    let other_rows = history(&db, other_user).await;

    assert_eq!(marker_value(&db).await.as_deref(), Some("2"));
    assert!(rows.iter().all(|row| row.user_id == user));
    assert!(other_rows.iter().all(|row| row.user_id == other_user));
    assert_eq!(rows.len(), 14, "2 real rows plus 12 synthesized rows");
    assert_eq!(
        rows.iter()
            .filter(|event| event.data.get("backfilled").and_then(Value::as_bool) == Some(true))
            .count(),
        12
    );

    let added = backfilled(&rows, EventType::Added);
    assert_eq!(added.len(), 3);
    assert!(added.iter().all(|event| event.data.get("source").is_none()));
    assert!(added
        .iter()
        .all(|event| event.data.get("work_title").is_some()));

    let enriched = backfilled(&rows, EventType::Enriched);
    assert_eq!(enriched.len(), 2);
    assert_eq!(
        enriched
            .iter()
            .filter(|event| event.work_id == Some(alpha.id))
            .count(),
        1
    );
    assert_eq!(
        enriched
            .iter()
            .find(|event| event.work_id == Some(alpha.id))
            .and_then(|event| event.data.get("source"))
            .and_then(Value::as_str),
        Some("goodreads")
    );

    let grabbed_titles = backfilled_titles(&rows, EventType::Grabbed);
    assert_eq!(
        grabbed_titles,
        HashSet::from([
            "Alpha Imported Release".to_string(),
            "Alpha Download Failed Release".to_string(),
            "Beta Import Failed Release".to_string(),
        ])
    );
    assert!(!grabbed_titles.contains("Gamma Real Covered Release"));
    assert!(!grabbed_titles.contains("Beta Removed Release"));
    assert!(!grabbed_titles.contains("Gamma In Flight Release"));

    let imported_paths = backfilled_titles(&rows, EventType::Imported);
    assert_eq!(
        imported_paths,
        HashSet::from([
            uncovered_item.path.clone(),
            manual_outside_item.path.clone(),
        ])
    );

    let download_failed = backfilled(&rows, EventType::DownloadFailed);
    assert_eq!(download_failed.len(), 1);
    assert_eq!(
        download_failed[0].data.get("title").and_then(Value::as_str),
        Some("Alpha Download Failed Release")
    );
    assert_eq!(
        download_failed[0].data.get("error").and_then(Value::as_str),
        Some("download client reported failure")
    );

    let import_failed = backfilled(&rows, EventType::ImportFailed);
    assert_eq!(import_failed.len(), 1);
    assert_eq!(
        import_failed[0].data.get("title").and_then(Value::as_str),
        Some("Beta Import Failed Release")
    );
    assert_eq!(
        import_failed[0].data.get("error").and_then(Value::as_str),
        Some("no supported files")
    );

    assert_event_date(
        &rows,
        EventType::Added,
        "work_title",
        "WH Alpha",
        dt(1, 0, 0),
    );
    assert_event_date(
        &rows,
        EventType::Enriched,
        "work_title",
        "WH Alpha",
        dt(2, 0, 0),
    );
    assert_event_date(
        &rows,
        EventType::Grabbed,
        "title",
        "Alpha Imported Release",
        dt(8, 0, 0),
    );
    assert_event_date(
        &rows,
        EventType::Imported,
        "path",
        "WH Beta/uncovered.epub",
        dt(16, 0, 0),
    );
}

#[tokio::test]
async fn wh_backfill_imported_coverage_window_pins_inside_and_outside_edges() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-window-user").await;
    let root = db
        .create_root_folder("/tmp/wh-window-ebooks", MediaType::Ebook)
        .await
        .expect("seed root folder");
    let work = seed_work(&db, user, "WH Window", "Author W", dt(1, 0, 0), None, None).await;

    let real_batch_date = dt(9, 0, 0);
    let inside_before = seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Window/inside-before.epub",
        real_batch_date + Duration::seconds(5),
    )
    .await;
    let inside_after = seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Window/inside-after.epub",
        real_batch_date - Duration::hours(1),
    )
    .await;
    let outside_before = seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Window/outside-before.epub",
        real_batch_date + Duration::seconds(6),
    )
    .await;
    let outside_after = seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Window/outside-after.epub",
        real_batch_date - Duration::hours(1) - Duration::seconds(1),
    )
    .await;
    let manual_newer_outside = seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Window/manual-newer-outside.epub",
        real_batch_date + Duration::hours(1) + Duration::seconds(1),
    )
    .await;

    insert_history_raw(
        &db,
        user,
        Some(work.id),
        EventType::Imported,
        json!({
            "title": "Grab-road batch",
            "imported": 3,
            "failed": 0,
            "skipped": 0,
            "work_title": work.title,
        }),
        real_batch_date,
    )
    .await;

    run_history_backfill(db.clone()).await;

    let rows = history(&db, user).await;
    let imported_paths = backfilled_titles(&rows, EventType::Imported);

    assert!(!imported_paths.contains(&inside_before.path));
    assert!(!imported_paths.contains(&inside_after.path));
    assert!(imported_paths.contains(&outside_before.path));
    assert!(imported_paths.contains(&outside_after.path));
    assert!(imported_paths.contains(&manual_newer_outside.path));
}

#[tokio::test]
async fn wh_backfill_idempotency_is_non_destructive_and_marker_stops_clean_reruns() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-idempotent-user").await;
    let root = db
        .create_root_folder("/tmp/wh-idempotent-ebooks", MediaType::Ebook)
        .await
        .expect("seed root folder");
    let work = seed_work(
        &db,
        user,
        "WH Idempotent",
        "Author I",
        dt(1, 0, 0),
        Some(dt(2, 0, 0)),
        Some("goodreads"),
    )
    .await;
    seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Idempotent/file.epub",
        dt(3, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        work.id,
        "WH Idempotent Release",
        "guid-idempotent",
        GrabStatus::Imported,
        None,
        dt(4, 0, 0),
    )
    .await;

    run_history_backfill(db.clone()).await;
    let after_first = history(&db, user).await;
    let first_count = after_first.len();
    assert!(first_count > 0);
    assert_eq!(marker_value(&db).await.as_deref(), Some("2"));

    sqlx::query("DELETE FROM works WHERE id = ? AND user_id = ?")
        .bind(work.id)
        .bind(user)
        .execute(db.pool())
        .await
        .expect("delete work after backfill");

    run_history_backfill(db.clone()).await;
    let after_second = history(&db, user).await;
    assert_eq!(after_second.len(), first_count);
    assert!(
        after_second.iter().any(|event| event.work_id.is_none()
            && event.data.get("work_title").and_then(Value::as_str) == Some("WH Idempotent")),
        "history for the deleted work must survive and must not be rebuilt destructively"
    );

    run_history_backfill(db.clone()).await;
    let after_third = history(&db, user).await;
    assert_eq!(after_third.len(), first_count);
    assert_eq!(marker_value(&db).await.as_deref(), Some("2"));
}

#[tokio::test]
async fn wh_backfill_insert_failure_leaves_no_marker_and_rerun_is_additive() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-insert-failure-user").await;
    let root = db
        .create_root_folder("/tmp/wh-insert-failure-ebooks", MediaType::Ebook)
        .await
        .expect("seed root folder");
    let work = seed_work(&db, user, "WH Failure", "Author F", dt(1, 0, 0), None, None).await;
    seed_item(
        &db,
        user,
        work.id,
        root.id,
        "WH Failure/file.epub",
        dt(2, 0, 0),
    )
    .await;

    // BLOCKED-PENDING-IMPLEMENTATION: run_history_backfill currently accepts
    // concrete SqliteDb, so this behavioral harness cannot inject the
    // required HistoryDb double that fails exactly one create_history_event
    // call mid-pass. The observable contract pinned here is the deepest
    // reachable seam until run_history_backfill is generic over HistoryDb or
    // gains an explicit test hook: failed/partial pass => no marker; next run
    // is additive and creates only missing rows.
    sqlx::query(
        "CREATE TRIGGER wh_fail_one_history_insert \
         BEFORE INSERT ON history \
         WHEN NEW.event_type = 'imported' \
         BEGIN \
           SELECT RAISE(FAIL, 'simulated one-row history insert failure'); \
         END",
    )
    .execute(db.pool())
    .await
    .expect("install failure trigger");

    run_history_backfill(db.clone()).await;
    let partial_rows = history(&db, user).await;
    assert_eq!(marker_value(&db).await, None);
    assert_eq!(backfilled(&partial_rows, EventType::Imported).len(), 0);
    assert_eq!(backfilled(&partial_rows, EventType::Added).len(), 1);

    sqlx::query("DROP TRIGGER wh_fail_one_history_insert")
        .execute(db.pool())
        .await
        .expect("remove failure trigger");

    run_history_backfill(db.clone()).await;
    let completed_rows = history(&db, user).await;
    assert_eq!(marker_value(&db).await.as_deref(), Some("2"));
    assert_eq!(backfilled(&completed_rows, EventType::Added).len(), 1);
    assert_eq!(backfilled(&completed_rows, EventType::Imported).len(), 1);
}

#[tokio::test]
async fn wh_backfill_failure_dedup_is_per_guid_for_same_title_grabs() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-guid-user").await;
    let work = seed_work(&db, user, "WH Guid", "Author G", dt(1, 0, 0), None, None).await;

    // The same release title grabbed from two indexers: two Failed facts
    // sharing (work, title) but not guid — UNIQUE(user_id, guid, indexer)
    // allows the pair, and only guid distinguishes them.
    seed_grab(
        &db,
        user,
        work.id,
        "WH Guid Release",
        "guid-a",
        GrabStatus::Failed,
        Some("boom-a"),
        dt(2, 0, 0),
    )
    .await;
    seed_grab(
        &db,
        user,
        work.id,
        "WH Guid Release",
        "guid-b",
        GrabStatus::Failed,
        Some("boom-b"),
        dt(3, 0, 0),
    )
    .await;
    // A third failed grab whose fact is covered by a LIVE (guid-less) row.
    seed_grab(
        &db,
        user,
        work.id,
        "WH Guid Other",
        "guid-c",
        GrabStatus::Failed,
        Some("boom-c"),
        dt(4, 0, 0),
    )
    .await;

    // Simulated crashed prior pass: grab A's failure row already written
    // (backfilled, guid-carrying) before the completion marker could land.
    insert_history_raw(
        &db,
        user,
        Some(work.id),
        EventType::DownloadFailed,
        json!({
            "title": "WH Guid Release",
            "guid": "guid-a",
            "error": "boom-a",
            "work_title": work.title,
            "backfilled": true,
        }),
        dt(2, 0, 0),
    )
    .await;
    // Live-writer coverage for grab C: no guid key — matched by (work, title).
    insert_history_raw(
        &db,
        user,
        Some(work.id),
        EventType::DownloadFailed,
        json!({
            "title": "WH Guid Other",
            "error": "boom-c",
            "work_title": work.title,
        }),
        dt(4, 0, 1),
    )
    .await;

    run_history_backfill(db.clone()).await;

    let rows = history(&db, user).await;
    let dl_backfilled = backfilled(&rows, EventType::DownloadFailed);
    // Grab B's fact is the only uncovered failure: A is guid-covered, C is
    // title-covered by the live row. A (work, title)-only key would skip B —
    // the crash-rerun fact loss this pin exists to prevent.
    assert_eq!(dl_backfilled.len(), 2, "pre-seeded A + newly synthesized B");
    let guids: HashSet<&str> = dl_backfilled
        .iter()
        .filter_map(|event| event.data.get("guid").and_then(Value::as_str))
        .collect();
    assert_eq!(guids, HashSet::from(["guid-a", "guid-b"]));
    assert_eq!(marker_value(&db).await.as_deref(), Some("2"));
}

#[tokio::test]
async fn wh_create_history_event_date_some_persists_fact_date_and_none_uses_now() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-date-user").await;
    let work = seed_work(&db, user, "WH Date", "Author D", dt(1, 0, 0), None, None).await;
    let fact_date = dt(6, 30, 0);

    db.create_history_event(CreateHistoryEventDbRequest {
        user_id: user,
        work_id: Some(work.id),
        event_type: EventType::Added,
        data: json!({"work_title": work.title, "backfilled": true}),
        date: Some(fact_date),
    })
    .await
    .expect("insert fact-dated history row");

    let before = Utc::now();
    db.create_history_event(CreateHistoryEventDbRequest {
        user_id: user,
        work_id: Some(work.id),
        event_type: EventType::Grabbed,
        data: json!({
            "title": "WH Date Release",
            "indexer": "fixture-indexer",
            "guid": "guid-date-none",
            "download_client_id": 1,
            "work_title": "WH Date",
        }),
        date: None,
    })
    .await
    .expect("insert now-dated history row");
    let after = Utc::now();

    let rows = history(&db, user).await;
    let fact_row = rows
        .iter()
        .find(|event| event.event_type == EventType::Added)
        .expect("fact-dated row");
    assert_eq!(fact_row.date, fact_date);

    let now_row = rows
        .iter()
        .find(|event| event.event_type == EventType::Grabbed)
        .expect("now-dated row");
    assert!(
        now_row.date >= before - Duration::seconds(1)
            && now_row.date <= after + Duration::seconds(1),
        "date None should persist approximately now; got {} outside {}..{}",
        now_row.date,
        before,
        after
    );
}

// Bug reproduction: identity-layer-rewrite S-14 — installations that already
// completed history backfill generation 1 must receive the additive v2-birth
// repair once, fact-dated from works.added_at, and a rerun must write nothing.
#[tokio::test]
async fn wh_v2_missing_birth_backfills_once_after_generation_one_marker() {
    let db = create_test_db().await;
    let user = seed_user(&db, "wh-v2-birth-user").await;
    let added_at = dt(7, 45, 12);
    let work = seed_work(
        &db,
        user,
        "The Cider House Rules",
        "John Irving",
        added_at,
        None,
        None,
    )
    .await;
    sqlx::query(
        "UPDATE works SET identity_generation=1, identity_status_v2='connected' \
          WHERE user_id=?1 AND id=?2",
    )
    .bind(user)
    .bind(work.id)
    .execute(db.pool())
    .await
    .expect("mark fixture as a v2-created Work");
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('history_backfill_generation', '1') \
         ON CONFLICT(key) DO UPDATE SET value='1'",
    )
    .execute(db.pool())
    .await
    .expect("seed completed generation-one marker");

    run_history_backfill(db.clone()).await;
    let after_first = history(&db, user).await;
    let births = backfilled(&after_first, EventType::Added);
    assert_eq!(births.len(), 1);
    assert_eq!(births[0].work_id, Some(work.id));
    assert_eq!(births[0].date, added_at);
    assert_eq!(births[0].data["work_title"], "The Cider House Rules");
    assert_eq!(births[0].data["work_author"], "John Irving");
    assert_eq!(births[0].data["backfilled"], true);
    assert_eq!(marker_value(&db).await.as_deref(), Some("2"));

    run_history_backfill(db.clone()).await;
    let after_second = history(&db, user).await;
    assert_eq!(after_second.len(), after_first.len(), "rerun is zero-write");
    assert_eq!(backfilled(&after_second, EventType::Added).len(), 1);
}
