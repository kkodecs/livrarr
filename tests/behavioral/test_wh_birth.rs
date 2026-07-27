mod common;

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateImportDbRequest, CreateSeriesDbRequest, HistoryDb,
    ImportDb, SeriesDb,
};
use livrarr_domain::history_events::WorkAddSource;
use livrarr_domain::identity::{
    CandidateId, CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate,
};
use livrarr_domain::seed::{
    seed_add_box, seed_author_monitor, seed_list_import, seed_manual_import, seed_readarr_import,
    seed_series_monitor, SeedInput, SeedLanguage,
};
use livrarr_domain::services::{SourceProviderData, WorkService};
use livrarr_domain::{EventType, HistoryFilter, UserId, WorkId};
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService =
    WorkServiceImpl<SqliteDb, livrarr_metadata::work_service::StubNoEnrichment, StubHttpFetcher>;

fn service(db: SqliteDb) -> TestWorkService {
    WorkServiceImpl::without_enrichment(
        db,
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    )
}

fn seed_input(title: &str, author: &str) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: author.to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: None,
        year: Some(2024),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn identity(title: &str, author: &str, ol_key: &str) -> IdentityState {
    IdentityState::Confirmed {
        anchors: CapturedIdentity {
            ol_key: Some(ol_key.to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: title.to_string(),
            author_name: author.to_string(),
            language: Some("en".to_string()),
        },
        method: IdentityMethod::UserSelected,
        score: None,
    }
}

fn added_filter(work_id: WorkId) -> HistoryFilter {
    HistoryFilter {
        event_type: Some(EventType::Added),
        work_id: Some(work_id),
        start_date: None,
        end_date: None,
    }
}

async fn added_count(db: &SqliteDb, user_id: UserId, work_id: WorkId) -> usize {
    db.list_history(user_id, added_filter(work_id))
        .await
        .expect("history should list")
        .len()
}

/// Door-specific FK targets, seeded before the add. Green-integration fixture
/// fix: the authored fixtures passed dangling references (series id 77, an
/// imports id with no row) and died on the FK constraint inside the driven
/// add before the assertion under test was ever reached.
struct DoorSeeds {
    series_id: i64,
    import_id: String,
}

async fn assert_creation_door(
    door_name: &str,
    expected_source: &str,
    make_candidate: impl Fn(&str, &str, &str, &DoorSeeds) -> WorkCandidate,
) {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = service(db.clone());
    // The added event snapshots the STORED work title, which rides through
    // clean_title (title-casing) at add — the fixture title must be a
    // clean_title fixed point or the raw-vs-stored comparison below fails on
    // a lowercase door name (green-integration fixture fix; assertions
    // unchanged).
    let door_title = {
        let mut cs = door_name.chars();
        match cs.next() {
            Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
            None => String::new(),
        }
    };
    let title = format!("Work History Birth {door_title}");
    let author = format!("Author {door_name}");
    let ol_key = format!("/works/WH-{door_name}");

    let (seed_author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: format!("Series Author {door_name}"),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed author for series FK");
    let series = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: seed_author.id,
            name: format!("Series {door_title}"),
            gr_key: format!("wh-birth-{door_name}"),
            monitor_ebook: true,
            monitor_audiobook: true,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("seed series row");
    let import_id = format!("import-{ol_key}");
    db.create_import(CreateImportDbRequest {
        id: import_id.clone(),
        user_id,
        source: "readarr".to_string(),
        source_url: None,
        target_root_folder_id: None,
    })
    .await
    .expect("seed imports row");
    let seeds = DoorSeeds {
        series_id: series.id,
        import_id,
    };

    let first = svc
        .add(user_id, make_candidate(&title, &author, &ol_key, &seeds))
        .await
        .expect("first add should create");
    assert!(first.created, "{door_name} should create the first work");

    let rows = db
        .list_history(user_id, added_filter(first.work.id))
        .await
        .expect("history should list");
    assert_eq!(rows.len(), 1, "{door_name} should write one added event");
    assert_eq!(
        rows[0].data.get("source").and_then(|v| v.as_str()),
        Some(expected_source),
        "{door_name} should preserve its true creation source"
    );
    assert_eq!(
        rows[0].data.get("work_title").and_then(|v| v.as_str()),
        Some(title.as_str()),
        "{door_name} added payload must include work_title"
    );

    let before = added_count(&db, user_id, first.work.id).await;
    let second = svc
        .add(user_id, make_candidate(&title, &author, &ol_key, &seeds))
        .await
        .expect("second add should dedup");
    let after = added_count(&db, user_id, first.work.id).await;

    assert!(!second.created, "{door_name} re-add should be a dedup hit");
    assert_eq!(
        after, before,
        "{door_name} dedup hit must not write a new added event"
    );
}

#[tokio::test]
async fn wh_added_event_records_search_source_and_dedup_writes_no_second_event() {
    assert_creation_door(
        "search",
        WorkAddSource::Search.as_str(),
        |title, author, ol_key, _seeds: &DoorSeeds| {
            seed_add_box(
                seed_input(title, author),
                identity(title, author, ol_key),
                Some(CandidateId(format!("candidate-{ol_key}"))),
                false,
            )
        },
    )
    .await;
}

#[tokio::test]
async fn wh_added_event_records_list_import_source_and_dedup_writes_no_second_event() {
    assert_creation_door(
        "list-import",
        WorkAddSource::ListImport.as_str(),
        |title, author, ol_key, _seeds: &DoorSeeds| {
            seed_list_import(
                seed_input(title, author),
                identity(title, author, ol_key),
                Some(CandidateId(format!("candidate-{ol_key}"))),
            )
        },
    )
    .await;
}

#[tokio::test]
async fn wh_added_event_records_readarr_source_and_dedup_writes_no_second_event() {
    assert_creation_door(
        "readarr",
        WorkAddSource::Readarr.as_str(),
        |title, author, ol_key, seeds: &DoorSeeds| {
            seed_readarr_import(
                seed_input(title, author),
                identity(title, author, ol_key),
                SourceProviderData {
                    description: Some("Readarr source payload".to_string()),
                    ..SourceProviderData::default()
                },
                true,
                true,
                seeds.import_id.clone(),
            )
        },
    )
    .await;
}

#[tokio::test]
async fn wh_added_event_records_author_monitor_source_and_dedup_writes_no_second_event() {
    assert_creation_door(
        "author-monitor",
        WorkAddSource::AuthorMonitor.as_str(),
        |title, author, ol_key, _seeds: &DoorSeeds| {
            seed_author_monitor(seed_input(title, author), identity(title, author, ol_key))
        },
    )
    .await;
}

#[tokio::test]
async fn wh_added_event_records_series_monitor_source_and_dedup_writes_no_second_event() {
    assert_creation_door(
        "series-monitor",
        WorkAddSource::SeriesMonitor.as_str(),
        |title, author, ol_key, seeds: &DoorSeeds| {
            seed_series_monitor(
                seed_input(title, author),
                identity(title, author, ol_key),
                seeds.series_id,
                true,
                true,
            )
        },
    )
    .await;
}

#[tokio::test]
async fn wh_added_event_records_file_import_source_and_dedup_writes_no_second_event() {
    assert_creation_door(
        "file-import",
        WorkAddSource::FileImport.as_str(),
        |title, author, ol_key, _seeds: &DoorSeeds| {
            seed_manual_import(
                seed_input(title, author),
                identity(title, author, ol_key),
                Some(CandidateId(format!("candidate-{ol_key}"))),
            )
        },
    )
    .await;
}
