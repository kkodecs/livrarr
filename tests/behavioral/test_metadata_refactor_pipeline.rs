use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use livrarr_db::{
    create_test_db, CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    identity::CandidateId, normalize_for_matching, EnrichmentStatus, MetadataProvider,
    OutcomeClass, RequestPriority, UserId, UserRole, Work, WorkId,
};
use livrarr_external_data::{
    transport_cache::TransportCache, NormalizedWorkDetail, ProviderOutcome,
};
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentContext, EnrichmentMode, EnrichmentService,
    EnrichmentServiceImpl, PriorityModel, ProviderQueue, ProviderQueueError, ScatterGatherResult,
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
    gr_key: Option<&str>,
) -> Work {
    db.create_work(CreateWorkDbRequest {
        gr_key: gr_key.map(str::to_string),
        ..work_req(user_id, title, "Contract Author")
    })
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
        provider_chase_attempted: true,
        search_leg_fired: false,
        search_ledger_burnable: false,
        search_provider_identity: Vec::new(),
        search_route_proposals: Vec::new(),
    }
}

fn service(
    db: livrarr_db::sqlite::SqliteDb,
    queue: StubProviderQueue,
    cache: TransportCache,
) -> impl EnrichmentService {
    EnrichmentServiceImpl::new(
        Arc::new(db),
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    )
    .with_transport_cache(Arc::new(cache.clone()))
}

#[tokio::test]
async fn add_box_and_author_page_paths_converge_on_same_metadata_and_covers() {
    // AC-001
    let db = create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "add-box".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api-add-box".to_string(),
        })
        .await
        .expect("test user should be created");
    let user_id = user.id;
    // REQ-007: anchors arrive at creation (identity capture), never from the
    // merge — the first-created row carries the gr anchor; the add-box door's
    // create dedups onto the same row.
    let author_page_work =
        seed_work_for_user(&db, user_id, "Add Box Title", Some("gr-refactor")).await;
    let (add_box_work, _) = db
        .create_work(work_req(user_id, "Add Box Title", "Contract Author"))
        .await
        .expect("test work should be created");

    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            add_box_work.id,
            HashMap::from([(MetadataProvider::Hardcover, payload_with_cover())]),
        )],
    );
    let queue_probe = queue.clone();
    let cache = TransportCache::new(Duration::from_secs(60));
    let service = service(db.clone(), queue, cache.clone());

    service
        .enrich_work(
            user_id,
            add_box_work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("network path should enrich");

    let cached_payload = match payload_with_cover() {
        ProviderOutcome::Success(payload) => *payload,
        _ => unreachable!("payload_with_cover always returns a successful payload"),
    };
    cache.cache_put(
        user_id,
        CandidateId("cached-candidate-with-cover".to_string()),
        HashMap::from([(MetadataProvider::Hardcover, cached_payload)]),
    );

    service
        .enrich_work(
            user_id,
            author_page_work.id,
            EnrichmentMode::Manual,
            Some(CandidateId("cached-candidate-with-cover".to_string())),
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
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
async fn failed_enrichment_sets_failed() {
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
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
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
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("unconfigured provider must not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
    let saved = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        saved.description,
        Some("A provider description".to_string())
    );
    // The remaining provider still saves its non-cover data. Round 15 keeps
    // Goodreads payload parsing intact but excludes its cover at candidate
    // assembly, so a GR-only scatter has no in-memory cover resolution.
    assert!(result.cover_resolution.is_none());
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
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("empty provider results must not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Thin);
    let saved = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(saved.title, "Thin Title");
    assert_eq!(saved.author_name, "Contract Author");
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
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("mixed empty-success plus errors should complete");

    assert_eq!(
        result.enrichment_status,
        EnrichmentStatus::Thin,
        "a successful empty provider response makes the work Thin, not Failed"
    );
}
