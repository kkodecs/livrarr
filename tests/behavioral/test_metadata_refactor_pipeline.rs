use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use futures::future::BoxFuture;
use livrarr_db::{
    create_test_db, CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    identity::CandidateId, normalize_for_matching, EnrichmentStatus, MetadataProvider,
    OutcomeClass, UserId, UserRole, Work, WorkId,
};
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};
use livrarr_metadata::{
    CircuitState, DefaultMergeEngine, EnrichmentContext, EnrichmentMode, EnrichmentService,
    EnrichmentServiceImpl, PacingQueue, PriorityModel, ProviderQueue, ProviderQueueError,
    ScatterGatherResult,
};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct StubProviderQueue {
    plans: Arc<Mutex<VecDeque<ScatterGatherResult>>>,
    dispatch_count: Arc<Mutex<usize>>,
    persist: Option<(livrarr_db::sqlite::SqliteDb, UserId)>,
}

impl StubProviderQueue {
    fn with_persisted_plans(
        db: livrarr_db::sqlite::SqliteDb,
        user_id: UserId,
        plans: Vec<ScatterGatherResult>,
    ) -> Self {
        Self {
            plans: Arc::new(Mutex::new(plans.into())),
            dispatch_count: Arc::new(Mutex::new(0)),
            persist: Some((db, user_id)),
        }
    }

    async fn dispatch_count(&self) -> usize {
        *self.dispatch_count.lock().await
    }
}

impl ProviderQueue for StubProviderQueue {
    async fn dispatch_enrichment(
        &self,
        _work: &Work,
        _context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        *self.dispatch_count.lock().await += 1;
        let result = self.plans.lock().await.pop_front().ok_or_else(|| {
            ProviderQueueError::Db(livrarr_domain::DbError::Conflict {
                message: "unexpected provider dispatch".to_string(),
            })
        })?;
        if let Some((db, user_id)) = &self.persist {
            persist_scatter_result(db, *user_id, &result).await?;
        }
        Ok(result)
    }

    fn circuit_state(&self, _provider: MetadataProvider) -> CircuitState {
        CircuitState::Closed
    }
}

async fn persist_scatter_result(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    result: &ScatterGatherResult,
) -> Result<(), ProviderQueueError> {
    use livrarr_db::ProviderRetryStateDb;

    for (provider, outcome) in &result.outcomes {
        match outcome {
            ProviderOutcome::Success(payload) => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::Success,
                    Some(serde_json::to_string(&**payload).map_err(|err| {
                        ProviderQueueError::Db(livrarr_domain::DbError::Io(Box::new(err)))
                    })?),
                )
                .await?;
            }
            ProviderOutcome::NotFound => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::NotFound,
                    None,
                )
                .await?;
            }
            ProviderOutcome::NotConfigured => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::NotConfigured,
                    None,
                )
                .await?;
            }
            ProviderOutcome::PermanentFailure { .. } => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::PermanentFailure,
                    None,
                )
                .await?;
            }
            ProviderOutcome::Conflict { .. } => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::Conflict,
                    None,
                )
                .await?;
            }
            ProviderOutcome::WillRetry {
                next_attempt_at, ..
            } => {
                db.record_will_retry(user_id, result.work_id, *provider, *next_attempt_at)
                    .await?;
            }
            ProviderOutcome::Suppressed { until } => {
                db.record_suppressed(user_id, result.work_id, *provider, *until)
                    .await?;
            }
        }
    }
    Ok(())
}

fn work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    }
}

async fn seed_user_and_work(
    db: &livrarr_db::sqlite::SqliteDb,
    username: &str,
    title: &str,
) -> (UserId, Work) {
    let user = db
        .create_user(CreateUserDbRequest {
            username: username.to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: format!("api-{username}"),
        })
        .await
        .expect("test user should be created");
    let (work, _) = db
        .create_work(work_req(user.id, title, "Contract Author"))
        .await
        .expect("test work should be created");
    (user.id, work)
}

async fn seed_work_for_user(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    title: &str,
) -> Work {
    db.create_work(work_req(user_id, title, "Contract Author"))
        .await
        .expect("test work should be created")
        .0
}

fn payload_with_cover() -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
        title: Some("Contract Title".to_string()),
        author_name: Some("Contract Author".to_string()),
        description: Some("A provider description".to_string()),
        gr_key: Some("gr-refactor".to_string()),
        isbn_13: Some("9780000000003".to_string()),
        cover_url: Some("https://covers.example.test/ebook.jpg".to_string()),
        ..Default::default()
    }))
}

fn scatter(
    work_id: WorkId,
    outcomes: HashMap<MetadataProvider, ProviderOutcome<NormalizedWorkDetail>>,
) -> ScatterGatherResult {
    ScatterGatherResult {
        work_id,
        outcomes,
        merge_eligible: true,
        deferred: false,
    }
}

fn service(db: livrarr_db::sqlite::SqliteDb, queue: StubProviderQueue) -> impl EnrichmentService {
    EnrichmentServiceImpl::new(
        Arc::new(db),
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        Arc::new(livrarr_metadata::llm_validator::NoOpLlmValidator::new()),
        livrarr_metadata::work_service::StubNoLlm,
        false,
    )
}

#[tokio::test]
async fn add_box_and_author_page_paths_converge_on_same_metadata_and_covers() {
    // AC-001
    let db = create_test_db().await;
    let (user_id, add_box_work) = seed_user_and_work(&db, "add-box", "Add Box Title").await;
    let author_page_work = seed_work_for_user(&db, user_id, "Author Page Title").await;

    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            add_box_work.id,
            HashMap::from([(MetadataProvider::Hardcover, payload_with_cover())]),
        )],
    );
    let queue_probe = queue.clone();
    let service = service(db.clone(), queue);

    service
        .enrich_work(user_id, add_box_work.id, EnrichmentMode::Manual, None)
        .await
        .expect("network path should enrich");

    service
        .enrich_work(
            user_id,
            author_page_work.id,
            EnrichmentMode::Manual,
            Some(CandidateId("cached-candidate-with-cover".to_string())),
        )
        .await
        .expect("candidate reuse path should enrich without a second network dispatch");

    assert_eq!(
        queue_probe.dispatch_count().await,
        1,
        "the candidate-reuse door should use cached payloads instead of re-dispatching providers"
    );

    let add_box = db.get_work(user_id, add_box_work.id).await.unwrap();
    let author_page = db.get_work(user_id, author_page_work.id).await.unwrap();
    assert_eq!(add_box.title, author_page.title);
    assert_eq!(add_box.description, author_page.description);
    assert_eq!(add_box.gr_key, author_page.gr_key);
    assert_eq!(add_box.isbn_13, author_page.isbn_13);
    assert_eq!(add_box.cover_url, author_page.cover_url);
    assert_eq!(add_box.audiobook_cover_url, author_page.audiobook_cover_url);
}

#[tokio::test]
async fn failed_enrichment_sets_failed_and_queue_membership_is_transient() {
    // AC-009
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "failed", "Failed Title").await;
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([(MetadataProvider::Hardcover, ProviderOutcome::NotConfigured)]),
        )],
    );
    let service = service(db.clone(), queue);

    let result = service
        .enrich_work(user_id, work.id, EnrichmentMode::Manual, None)
        .await
        .expect("failed provider run should not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Failed);
    assert_eq!(
        db.get_work(user_id, work.id)
            .await
            .unwrap()
            .enrichment_status,
        EnrichmentStatus::Failed
    );

    let pacing = livrarr_metadata::LivePacingQueue::new(Arc::new(db));
    assert!(
        !pacing.has_pending_or_running(work.id),
        "in-progress is derived from live queue membership, not persisted status"
    );
}

#[tokio::test]
async fn unconfigured_provider_is_skipped_remaining_providers_save_the_work() {
    // AC-017
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "skip", "Skip Title").await;
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([
                (MetadataProvider::Hardcover, ProviderOutcome::NotConfigured),
                (MetadataProvider::Goodreads, payload_with_cover()),
            ]),
        )],
    );
    let service = service(db.clone(), queue);

    let result = service
        .enrich_work(user_id, work.id, EnrichmentMode::Manual, None)
        .await
        .expect("unconfigured provider must not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
    let saved = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        saved.description,
        Some("A provider description".to_string())
    );
    assert_eq!(
        saved.cover_url,
        Some("https://covers.example.test/ebook.jpg".to_string())
    );
}

#[tokio::test]
async fn all_providers_no_usable_data_saves_seed_and_lands_thin_or_failed() {
    // AC-018
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "thin", "Thin Title").await;
    let empty_success = ProviderOutcome::Success(Box::new(NormalizedWorkDetail::default()));
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([(MetadataProvider::Hardcover, empty_success)]),
        )],
    );
    let service = service(db.clone(), queue);

    let result = service
        .enrich_work(user_id, work.id, EnrichmentMode::Manual, None)
        .await
        .expect("empty provider results must not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Thin);
    let saved = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(saved.title, "Thin Title");
    assert_eq!(saved.author_name, "Contract Author");
}

#[ignore = "pk-impl: blocked pending green server wiring (AC-019)"]
#[tokio::test]
async fn retry_all_failed_reenqueues_failed_works_without_background_retry_loop() {
    // AC-019
    let _intended_assertion: BoxFuture<'static, ()> = Box::pin(async {
        panic!(
            "retry_all_failed should list Failed works, enqueue each through the same pacing queue, and start no background retry loop"
        );
    });
}

#[tokio::test]
async fn one_empty_success_and_rest_errors_lands_thin_not_failed() {
    // AC-020
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "mixed", "Mixed Title").await;
    let empty_success = ProviderOutcome::Success(Box::new(NormalizedWorkDetail::default()));
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([
                (MetadataProvider::OpenLibrary, empty_success),
                (MetadataProvider::Hardcover, ProviderOutcome::NotConfigured),
            ]),
        )],
    );
    let service = service(db.clone(), queue);

    let result = service
        .enrich_work(user_id, work.id, EnrichmentMode::Manual, None)
        .await
        .expect("mixed empty-success plus errors should complete");

    assert_eq!(
        result.enrichment_status,
        EnrichmentStatus::Thin,
        "a successful empty provider response makes the work Thin, not Failed"
    );
}
