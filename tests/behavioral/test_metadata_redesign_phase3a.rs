//! Behavioral contracts for metadata-consistency Phase 3a.
//!
//! These tests are written before the Phase 3a implementation. They pin the
//! WorkService::add() creation gate contract: cleanup, dedup, creation result,
//! provenance, monitor defaults, synchronous enrichment, and provider data
//! handoff.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateUserDbRequest, ProvenanceDb, UserDb};
use livrarr_domain::services::{
    AddWorkRequest, EnrichmentMode, EnrichmentResult, EnrichmentWorkflow, EnrichmentWorkflowError,
    WorkService, WorkServiceError,
};
use livrarr_domain::{
    EnrichmentStatus, MetadataProvider, OutcomeClass, ProvenanceSetter, SourceProviderData, UserId,
    UserRole, Work, WorkField, WorkId,
};
use livrarr_metadata::work_service::WorkServiceImpl;

fn data_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("test data dir")
}

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("phase3a-{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: format!("phase3a-key-{suffix}"),
    })
    .await
    .expect("test user should be created")
    .id
}

fn add_req(title: &str, author_name: &str) -> AddWorkRequest {
    AddWorkRequest {
        title: title.to_string(),
        author_name: author_name.to_string(),
        author_ol_key: None,
        ol_key: None,
        gr_key: None,
        year: Some(2026),
        cover_url: None,
        language: Some("eng".to_string()),
        detail_url: None,
        series_id: None,
        series_name: None,
        series_position: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: None,
        import_id: None,
        source_provider_data: None,
    }
}

fn readarr_source_data(description: &str) -> SourceProviderData {
    SourceProviderData {
        description: Some(description.to_string()),
        isbn: Some("9780000000001".to_string()),
        asin: Some("B000000001".to_string()),
        publisher: Some("Readarr Press".to_string()),
        genres: Some(vec!["fiction".to_string()]),
        page_count: Some(321),
        rating: Some(4.25),
        rating_count: Some(128),
        cover_url: Some("https://example.test/readarr-cover.jpg".to_string()),
        series_name: Some("Readarr Series".to_string()),
        series_position: Some("1-3".to_string()),
    }
}

#[derive(Clone)]
struct RecordingEnrichment {
    db: SqliteDb,
    calls: Arc<AtomicUsize>,
    fail: bool,
    expected_source_description: Option<String>,
}

impl RecordingEnrichment {
    fn succeeding(db: SqliteDb) -> Self {
        Self {
            db,
            calls: Arc::new(AtomicUsize::new(0)),
            fail: false,
            expected_source_description: None,
        }
    }

    fn failing(db: SqliteDb) -> Self {
        Self {
            db,
            calls: Arc::new(AtomicUsize::new(0)),
            fail: true,
            expected_source_description: None,
        }
    }

    fn expecting_source_data(db: SqliteDb, description: &str) -> Self {
        Self {
            db,
            calls: Arc::new(AtomicUsize::new(0)),
            fail: false,
            expected_source_description: Some(description.to_string()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl EnrichmentWorkflow for RecordingEnrichment {
    async fn enrich_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        assert_eq!(mode, EnrichmentMode::Background);
        self.calls.fetch_add(1, Ordering::SeqCst);

        if let Some(expected) = &self.expected_source_description {
            let json = stored_source_provider_json(&self.db, work_id)
                .await
                .expect("source_provider_json should be written before enrichment runs");
            assert!(
                json.contains(expected),
                "source_provider_data should be persisted for enrichment input, got {json}"
            );
        }

        let status = if self.fail {
            EnrichmentStatus::Failed
        } else {
            EnrichmentStatus::Enriched
        };
        sqlx::query("UPDATE works SET enrichment_status = ? WHERE user_id = ? AND id = ?")
            .bind(match status {
                EnrichmentStatus::Enriched => "enriched",
                EnrichmentStatus::Failed => "failed",
                _ => unreachable!("test stub only persists Enriched or Failed"),
            })
            .bind(user_id)
            .bind(work_id)
            .execute(self.db.pool())
            .await
            .expect("test enrichment should be able to persist status");

        if self.fail {
            return Err(EnrichmentWorkflowError::Queue(
                "stub enrichment failure".into(),
            ));
        }

        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("phase3a-test-stub".to_string()),
            work: Work {
                id: work_id,
                user_id,
                enrichment_status: EnrichmentStatus::Enriched,
                ..Default::default()
            },
            merge_deferred: false,
            provider_outcomes: HashMap::<MetadataProvider, OutcomeClass>::new(),
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        Ok(())
    }
}

async fn stored_source_provider_json(db: &SqliteDb, work_id: WorkId) -> Option<String> {
    sqlx::query_scalar("SELECT source_provider_json FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("work row should exist")
}

async fn stored_work_count(db: &SqliteDb, user_id: UserId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("work count query should succeed")
}

async fn title_provenance_setter(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
) -> ProvenanceSetter {
    db.get_field_provenance(user_id, work_id, WorkField::Title)
        .await
        .expect("provenance lookup should succeed")
        .expect("title provenance should be present")
        .setter
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_new_work_returns_created_true_cleans_identity_writes_provenance_and_enriches() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "new-work").await;
    let enrichment = RecordingEnrichment::succeeding(db.clone());
    let calls = enrichment.clone();
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        enrichment,
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let result = service
        .add(
            user_id,
            add_req("  The Left Hand of Darkness  ", "  Ursula K. Le Guin  "),
        )
        .await
        .expect("new work add should succeed");

    assert!(result.created);
    assert_eq!(result.work.title, "The Left Hand of Darkness");
    assert_eq!(result.work.author_name, "Ursula K. Le Guin");
    assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(calls.call_count(), 1);
    assert_eq!(
        title_provenance_setter(&db, user_id, result.work.id).await,
        ProvenanceSetter::User
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_dedups_by_normalized_title_author_and_does_not_apply_source_data_to_existing() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "normalized-dedup").await;
    let enrichment = RecordingEnrichment::succeeding(db.clone());
    let calls = enrichment.clone();
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        enrichment,
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let first = service
        .add(
            user_id,
            add_req("A Wizard of Earthsea", "Ursula K. Le Guin"),
        )
        .await
        .expect("first add should create");

    let mut duplicate = add_req("  a wizard: of earthsea  ", "URSULA K LE GUIN");
    duplicate.source_provider_data = Some(readarr_source_data("duplicate-only provider data"));
    let second = service
        .add(user_id, duplicate)
        .await
        .expect("duplicate add should return existing work");

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(second.work.id, first.work.id);
    assert_eq!(stored_work_count(&db, user_id).await, 1);
    assert_eq!(stored_source_provider_json(&db, first.work.id).await, None);
    assert_eq!(
        calls.call_count(),
        1,
        "dedup fast path should not enrich the existing work again"
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_dedups_by_ol_key_and_returns_existing_work_id() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "ol-dedup").await;
    let enrichment = RecordingEnrichment::succeeding(db.clone());
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        enrichment,
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let mut first_req = add_req("Book One", "Author One");
    first_req.ol_key = Some("OL123W".to_string());
    let first = service.add(user_id, first_req).await.unwrap();

    let mut duplicate = add_req("Completely Different Title", "Different Author");
    duplicate.ol_key = Some("OL123W".to_string());
    let second = service.add(user_id, duplicate).await.unwrap();

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(second.work.id, first.work.id);
    assert_eq!(stored_work_count(&db, user_id).await, 1);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn concurrent_add_calls_for_same_identity_create_once_and_return_same_work() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "concurrent").await;
    let tmp_a = data_dir();
    let tmp_b = data_dir();
    let service_a = WorkServiceImpl::new(
        db.clone(),
        RecordingEnrichment::succeeding(db.clone()),
        StubHttpFetcher::new(),
        tmp_a.path().to_path_buf(),
    );
    let service_b = WorkServiceImpl::new(
        db.clone(),
        RecordingEnrichment::succeeding(db.clone()),
        StubHttpFetcher::new(),
        tmp_b.path().to_path_buf(),
    );

    let (a, b) = tokio::join!(
        service_a.add(user_id, add_req("Piranesi", "Susanna Clarke")),
        service_b.add(user_id, add_req(" piranesi ", "SUSANNA CLARKE")),
    );

    let a = a.expect("first concurrent add should return Ok");
    let b = b.expect("second concurrent add should return Ok");
    assert_eq!(a.work.id, b.work.id);
    assert_ne!(
        a.created, b.created,
        "exactly one concurrent add should report created=true"
    );
    assert_eq!(stored_work_count(&db, user_id).await, 1);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_monitor_flags_default_to_true_and_explicit_false_is_honored() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "monitor-flags").await;
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        RecordingEnrichment::succeeding(db.clone()),
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let defaults = service
        .add(user_id, add_req("Default Monitors", "Author"))
        .await
        .unwrap();
    assert!(defaults.work.monitor_ebook);
    assert!(defaults.work.monitor_audiobook);

    let mut explicit = add_req("Explicit Monitors", "Author");
    explicit.monitor_ebook = Some(false);
    explicit.monitor_audiobook = Some(false);
    let explicit = service.add(user_id, explicit).await.unwrap();
    assert!(!explicit.work.monitor_ebook);
    assert!(!explicit.work.monitor_audiobook);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_enrichment_failure_returns_ok_with_failed_status() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "enrichment-failure").await;
    let enrichment = RecordingEnrichment::failing(db.clone());
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db,
        enrichment,
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let result = service
        .add(user_id, add_req("Failure Is Not Fatal", "Author"))
        .await
        .expect("enrichment failure should not escape as WorkServiceError");

    assert!(result.created);
    assert_eq!(result.enrichment_status, EnrichmentStatus::Failed);
    assert_eq!(result.work.enrichment_status, EnrichmentStatus::Failed);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_title_cleanup_strips_whitespace_and_empty_cleaned_title_is_validation_error() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "title-cleanup").await;
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        RecordingEnrichment::succeeding(db),
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let cleaned = service
        .add(user_id, add_req("\n\t Clean Title \t", "  Clean Author  "))
        .await
        .unwrap();
    assert_eq!(cleaned.work.title, "Clean Title");
    assert_eq!(cleaned.work.author_name, "Clean Author");

    let err = service
        .add(user_id, add_req(" \n\t ", "Author"))
        .await
        .expect_err("empty cleaned title should be rejected");
    assert!(
        matches!(err, WorkServiceError::Validation(msg) if msg.contains("title")),
        "expected validation error for empty title, got {err:?}"
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_provenance_uses_request_setter_and_defaults_to_user() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "provenance").await;
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        RecordingEnrichment::succeeding(db.clone()),
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let mut imported_req = add_req("Imported Work", "Author");
    imported_req.provenance_setter = Some(ProvenanceSetter::Import);
    let imported = service.add(user_id, imported_req).await.unwrap();
    assert_eq!(
        title_provenance_setter(&db, user_id, imported.work.id).await,
        ProvenanceSetter::Import
    );

    let defaulted = service
        .add(user_id, add_req("User Work", "Author"))
        .await
        .unwrap();
    assert_eq!(
        title_provenance_setter(&db, user_id, defaulted.work.id).await,
        ProvenanceSetter::User
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn add_source_provider_data_is_available_to_enrichment_pipeline() {
    let db = create_test_db().await;
    let user_id = create_user(&db, "source-provider-data").await;
    let description = "Readarr supplied description";
    let enrichment = RecordingEnrichment::expecting_source_data(db.clone(), description);
    let tmp = data_dir();
    let service = WorkServiceImpl::new(
        db.clone(),
        enrichment,
        StubHttpFetcher::new(),
        tmp.path().to_path_buf(),
    );

    let mut req = add_req("Readarr Work", "Readarr Author");
    req.source_provider_data = Some(readarr_source_data(description));
    let result = service.add(user_id, req).await.unwrap();

    assert!(result.created);
    let json = stored_source_provider_json(&db, result.work.id)
        .await
        .expect("source provider data should be persisted");
    assert!(json.contains(description));
}

#[test]
fn add_work_request_has_no_defer_enrichment_field_compile_time_contract() {
    // This constructor intentionally omits `defer_enrichment`. Because
    // AddWorkRequest is a public struct, the test file stops compiling if that
    // field still exists or is reintroduced.
    let req = add_req("No Deferred Enrichment", "Author");
    assert_eq!(req.title, "No Deferred Enrichment");
}
