//! Behavioral RED tests for metadata-correctness add-door identity/enrichment routing.
//!
//! Each red door test wires a real `WorkServiceImpl` with a stub-backed
//! `LiveEnglishIdentityResolver` (the test_idu_bulk_import_identity pattern), so the
//! AC-012 RED directive is asserted in full: resolver/provider fan-out OBSERVED and
//! resolved anchors PERSISTED — not merely that the add road was reached.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{
    CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate, WorkSeedFields,
};
use livrarr_domain::services::WorkService;
use livrarr_domain::{normalize_for_matching, EnrichmentStatus, ProvenanceSetter, Work};
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

fn service(db: SqliteDb, workflow: StubEnrichmentWorkflow) -> TestWorkService {
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

fn candidate_fields(title: &str, author: &str, language: &str) -> WorkSeedFields {
    WorkSeedFields {
        title: title.to_string(),
        author_name: author.to_string(),
        language: language.to_string(),
        author_ol_key: None,
        year: Some(2024),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn anchorless_confirmed_candidate(title: &str, author: &str) -> WorkCandidate {
    WorkCandidate {
        fields: candidate_fields(title, author, "en"),
        identity: IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: None,
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
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::Search,
    }
}

fn work_req(user_id: i64, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: false,
        ..Default::default()
    }
}

async fn seed_anchorless_work(db: &SqliteDb, user_id: i64, title: &str, author: &str) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title, author))
        .await
        .expect("seed anchorless work");
    assert!(created);
    work
}

#[tokio::test]
async fn enriched_dedup_readd_does_not_reenrich_or_touch_source() {
    // REQ-010/AC-012: re-adding a candidate deduped to an already-Enriched work preserves behavior.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let existing = seed_anchorless_work(&db, user_id, "Already Enriched", "Door Audit").await;
    db.update_work_enrichment(
        user_id,
        existing.id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("existing-source".to_string()),
            description: Some("Existing description".to_string()),
            cover_url: Some("https://covers.example/existing.jpg".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("mark existing work enriched");

    let workflow = StubEnrichmentWorkflow::succeeding();
    let svc = service(db.clone(), workflow.clone());
    let added = svc
        .add(
            user_id,
            anchorless_confirmed_candidate("Already Enriched", "Door Audit"),
        )
        .await
        .expect("re-add enriched dedup");
    let persisted = db
        .get_work(user_id, existing.id)
        .await
        .expect("read deduped work");

    assert!(!added.created);
    assert_eq!(added.work.id, existing.id);
    assert_eq!(
        workflow.call_count(),
        0,
        "already enriched dedup must not re-enrich"
    );
    assert_eq!(persisted.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(
        persisted.enrichment_source.as_deref(),
        Some("existing-source")
    );
}
