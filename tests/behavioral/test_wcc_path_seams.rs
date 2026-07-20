#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency list, Readarr, and monitor path seams.
//!
//! blocked-pending-implementation: IR v2 names `list_confirm_row`, but the current public
//! surface only exposes batched `ListService::confirm`. These tests invoke that real seam and
//! assert the persisted DB row after confirmation.
//! blocked-pending-implementation: Readarr `process_works` is currently a private
//! `ImportRunner` method, so the Readarr test uses the real downstream `WorkService::add`
//! persistence seam with a Readarr-shaped candidate and asserts the returned/persisted identity.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{AuthorDb, CreateAuthorDbRequest, ListImportDb, WorkDb};
use livrarr_domain::identity::*;
use livrarr_domain::services::*;
use livrarr_domain::{EnrichmentStatus, ProvenanceSetter, UserId, UserRole, Work};
use livrarr_external_data::parsers::parse_goodreads_csv;
use livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl;
use livrarr_metadata::list_service::{ListServiceImpl, NoOpBibliographyTrigger};
use livrarr_metadata::work_service::WorkServiceImpl;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-wcc-test-{}", std::process::id()))
}

fn ol_edition_response() -> FetchResponse {
    FetchResponse {
        status: 200,
        headers: vec![],
        body: br#"{
            "works": [{"key": "/works/OL45883W"}],
            "publish_date": "1965",
            "covers": [1111]
        }"#
        .to_vec(),
    }
}

fn ol_work_response() -> FetchResponse {
    FetchResponse {
        status: 200,
        headers: vec![],
        body: br#"{
            "title": "Dune",
            "authors": [{"author": {"key": "/authors/OL123A"}}]
        }"#
        .to_vec(),
    }
}

fn ol_author_response() -> FetchResponse {
    FetchResponse {
        status: 200,
        headers: vec![],
        body: br#"{"name": "Frank Herbert"}"#.to_vec(),
    }
}

fn ol_author_works_response(entries: &[(&str, &str, &str)]) -> Vec<u8> {
    let entries_json: Vec<String> = entries
        .iter()
        .map(|(key, title, date)| {
            format!(
                r#"{{"key": "/works/{}", "title": "{}", "first_publish_date": "{}"}}"#,
                key, title, date
            )
        })
        .collect();
    format!(r#"{{"entries": [{}]}}"#, entries_json.join(",")).into_bytes()
}

type TestWorkService = WorkServiceImpl<
    livrarr_db::sqlite::SqliteDb,
    livrarr_metadata::work_service::StubNoEnrichment,
    StubHttpFetcher,
>;

type TestListService = ListServiceImpl<
    livrarr_db::sqlite::SqliteDb,
    TestWorkService,
    StubHttpFetcher,
    NoOpBibliographyTrigger,
>;

async fn make_list_service() -> (TestListService, UserId) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let http = StubHttpFetcher::new();
    http.push_response(Ok(ol_edition_response()));
    http.push_response(Ok(ol_work_response()));
    http.push_response(Ok(ol_author_response()));

    let work_service =
        WorkServiceImpl::without_enrichment(db.clone(), http.clone(), test_data_dir());
    (
        ListServiceImpl::new(db, work_service, http, NoOpBibliographyTrigger),
        user_id,
    )
}

fn readarr_candidate_with_foreign_book_id() -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            year: Some(1965),
            language: "en".to_string(),
            author_ol_key: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity: IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: Some("OL45883W".to_string()),
                gr_key: Some("12345".to_string()),
                hc_key: None,
                isbn_13: Some("9780441013593".to_string()),
                asin: Some("B000N2HCP6".to_string()),
                title: "Dune".to_string(),
                author_name: "Frank Herbert".to_string(),
                language: Some("en".to_string()),
            },
            method: IdentityMethod::IsbnDirect,
            score: None,
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: Some(true),
        monitor_audiobook: Some(true),
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: Some("readarr-import".to_string()),
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::Search,
    }
}

/// REQ-IDs: REQ-006
/// AC-IDs: AC-007
/// Directive: Goodreads CSV parser carries Book Id so the list confirm seam can seed GR identity.
#[test]
fn test_wcc_path_seams_ac_007_parse_goodreads_csv_carries_book_id_and_isbn() {
    let csv = b"Book Id,Title,Author,ISBN13,Original Publication Year,Exclusive Shelf\n234225,Dune,Frank Herbert,=\"9780441013593\",1965,to-read\n";

    let rows = parse_goodreads_csv(csv).expect("Goodreads CSV should parse");
    let row = rows.first().expect("one row");
    let json = serde_json::to_value(row).expect("ImportRow should serialize");

    assert_eq!(row.title, "Dune");
    assert_eq!(row.author, "Frank Herbert");
    assert_eq!(row.isbn_13.as_deref(), Some("9780441013593"));
    assert_eq!(
        json.get("goodreadsBookId").and_then(|v| v.as_str()),
        Some("234225"),
        "AC-007: the parsed row must carry Book Id forward for list_confirm_row/ListService::confirm"
    );
}

/// REQ-IDs: REQ-001, REQ-006, REQ-026
/// AC-IDs: AC-007, AC-026
/// Directive: list confirm calls the real add seam and persists GR Book Id + ISBN on the created Work.
#[tokio::test]
async fn test_wcc_path_seams_ac_007_026_list_confirm_persists_gr_key_and_isbn_from_row() {
    let (svc, user_id) = make_list_service().await;
    let csv = b"Book Id,Title,Author,ISBN13,Original Publication Year,Exclusive Shelf\n234225,Dune,Frank Herbert,=\"9780441013593\",1965,to-read\n";

    let preview = svc
        .preview(user_id, csv.to_vec())
        .await
        .expect("preview Goodreads CSV");
    let result = svc
        .confirm(user_id, &preview.preview_id, None, &[0], None)
        .await
        .expect("confirm selected row");
    let works = svc
        .db
        .list_works(user_id)
        .await
        .expect("fetch works after confirm");
    let work = works
        .iter()
        .find(|w| w.title == "Dune")
        .expect("confirmed list row should create Dune");

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].status, "added");
    assert_eq!(
        work.gr_key.as_deref(),
        Some("234225"),
        "AC-007/REQ-006: list confirm must persist the Goodreads Book Id as gr_key"
    );
    assert_eq!(
        work.isbn_13.as_deref(),
        Some("9780441013593"),
        "AC-007/REQ-001: list confirm must persist the row ISBN bridge"
    );
    assert_eq!(
        work.enrichment_status,
        EnrichmentStatus::Unenriched,
        "REQ-014: the enrichment track is enrichment-only"
    );
    assert_eq!(
        work.identity_status,
        livrarr_domain::IdentityStatus::Pending,
        "AC-026: unresolved non-interactive residue is identity-pending on the identity track"
    );
}

/// REQ-IDs: REQ-001, REQ-006
/// AC-IDs: AC-002, AC-031
/// Directive: Readarr-shaped creation persists foreign_book_id as GR key plus ISBN/ASIN bridges.
#[tokio::test]
async fn test_wcc_path_seams_ac_002_031_readarr_candidate_persists_foreign_book_id_isbn_and_asin() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_service =
        WorkServiceImpl::without_enrichment(db.clone(), StubHttpFetcher::new(), test_data_dir());

    // The real Readarr workflow creates the `imports` record (process_works ->
    // create_import) BEFORE tagging works with its id; works.import_id is FK-
    // constrained to imports(id) (migration 017). Mirror that here so the
    // Readarr-shaped candidate's import_id has a valid FK target.
    db.create_list_import_record(
        "readarr-import",
        user_id,
        "readarr",
        &chrono::Utc::now().to_rfc3339(),
    )
    .await
    .expect("seed Readarr import record (FK target for work.import_id)");

    let result = work_service
        .add(user_id, readarr_candidate_with_foreign_book_id())
        .await
        .expect("Readarr process_works downstream add seam should create work");
    let persisted = db
        .get_work(user_id, result.work.id)
        .await
        .expect("fetch Readarr-created work");

    assert!(result.created);
    assert_eq!(persisted.gr_key.as_deref(), Some("12345"));
    assert_eq!(persisted.isbn_13.as_deref(), Some("9780441013593"));
    assert_eq!(persisted.asin.as_deref(), Some("B000N2HCP6"));
}

/// REQ-IDs: REQ-006
/// AC-IDs: AC-025
/// Directive: author monitor create path persists the native OL anchor at creation time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_wcc_path_seams_ac_025_author_monitor_create_persists_native_ol_anchor() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Frank Herbert".to_string(),
            sort_name: None,
            ol_key: Some("OL79034A".to_string()),
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed monitored author");
    db.update_author(
        user_id,
        author.id,
        livrarr_db::UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            gr_key: None,
            monitored: Some(true),
            monitor_new_items: Some(true),
            monitor_since: None,
            monitor_language: None,
        },
    )
    .await
    .expect("enable author monitor");

    let http = Arc::new(StubHttpFetcher::with_ok(
        200,
        ol_author_works_response(&[("OL45883W", "Dune", "1965")]),
    ));
    let work_service = Arc::new(WorkServiceImpl::without_enrichment(
        db.clone(),
        StubHttpFetcher::new(),
        test_data_dir(),
    ));
    let db = Arc::new(db);
    let workflow = AuthorMonitorWorkflowImpl::new(db.clone(), work_service, http);

    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .expect("run author monitor create path");
    let works = db
        .list_works(user_id)
        .await
        .expect("fetch works after monitor run");
    let created = works
        .iter()
        .find(|w| w.title == "Dune")
        .expect("author monitor should create Dune");

    assert_eq!(report.works_added, 1);
    assert_eq!(
        created.ol_key.as_deref(),
        Some("OL45883W"),
        "AC-025/REQ-006: author monitor must persist the source native OL anchor at create time"
    );
}
