#![allow(dead_code)]
//! Behavioral tests for enrichment_service::EnrichmentService::enrich_work.
//!
//! Covers orchestration contracts for R-02 and R-22:
//! - 9-step enrich_work flow
//! - deferred merge behavior
//! - TOCTOU re-read after dispatch
//! - CAS retry / exhaustion
//! - per-work lock invariant [I-12]
//! - conflict status-only merge path
//! - corrupt retry payload handling
//! - background vs manual coercion of non-mergeable outcomes
//! - DB postconditions and returned provider outcome classes

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use assert_matches::assert_matches;
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use futures::future::BoxFuture;
use livrarr_db::{
    create_test_db, ApplyEnrichmentMergeRequest, CreateUserDbRequest, CreateWorkDbRequest, DbError,
    ExternalIdDb, ProvenanceDb, ProviderRetryStateDb, SetFieldProvenanceRequest,
    UpdateWorkEnrichmentDbRequest, UpdateWorkUserFieldsDbRequest, UpsertExternalIdRequest, UserDb,
    WorkDb,
};
use livrarr_domain::{
    ApplyMergeOutcome, EnrichmentStatus, FieldProvenance, MergeResolved, MetadataProvider,
    NarrationType, OutcomeClass, PermanentFailureReason, ProvenanceSetter, UserId, UserRole, Work,
    WorkField, WorkId,
};
use livrarr_metadata::{
    CircuitState, EnrichmentContext, EnrichmentError, EnrichmentMode, EnrichmentService,
    MergeEngine, MergeError, MergeInput, MergeOutput, NormalizedWorkDetail, ProviderOutcome,
    ProviderQueue, ProviderQueueError, ScatterGatherResult,
};
use tokio::sync::{Mutex, Notify};

#[async_trait]
pub trait DbTestHarness: Send + Sync {
    type Db: WorkDb + ProvenanceDb + ProviderRetryStateDb + ExternalIdDb + UserDb + Send + Sync;
    async fn setup() -> Self;
    fn db(&self) -> &Self::Db;
    fn user_id(&self) -> UserId;
}

pub struct SqliteHarness {
    db: livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
}

#[async_trait]
impl DbTestHarness for SqliteHarness {
    type Db = livrarr_db::sqlite::SqliteDb;

    async fn setup() -> Self {
        let db = create_test_db().await;
        let user = db
            .create_user(CreateUserDbRequest {
                username: "enrichment-test-user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "api".to_string(),
            })
            .await
            .unwrap();

        Self {
            db,
            user_id: user.id,
        }
    }

    fn db(&self) -> &Self::Db {
        &self.db
    }

    fn user_id(&self) -> UserId {
        self.user_id
    }
}

#[derive(Clone)]
struct DispatchPlan {
    result: ScatterGatherResult,
    persist_outcomes:
        Option<Arc<dyn Fn(ScatterGatherResult) -> BoxFuture<'static, ()> + Send + Sync>>,
    before_return: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,
    hold_until: Option<Arc<Notify>>,
}

#[derive(Clone, Default)]
struct StubProviderQueue {
    plans: Arc<Mutex<VecDeque<DispatchPlan>>>,
    dispatch_started: Arc<Notify>,
    dispatch_count: Arc<Mutex<u32>>,
    seen_contexts: Arc<Mutex<Vec<EnrichmentContext>>>,
}

impl StubProviderQueue {
    fn with_plan(plan: DispatchPlan) -> Self {
        let mut q = VecDeque::new();
        q.push_back(plan);
        Self {
            plans: Arc::new(Mutex::new(q)),
            dispatch_started: Arc::new(Notify::new()),
            dispatch_count: Arc::new(Mutex::new(0)),
            seen_contexts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_plans(plans: Vec<DispatchPlan>) -> Self {
        Self {
            plans: Arc::new(Mutex::new(plans.into())),
            dispatch_started: Arc::new(Notify::new()),
            dispatch_count: Arc::new(Mutex::new(0)),
            seen_contexts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn dispatch_count(&self) -> u32 {
        *self.dispatch_count.lock().await
    }

    async fn contexts(&self) -> Vec<EnrichmentContext> {
        self.seen_contexts.lock().await.clone()
    }
}

impl ProviderQueue for StubProviderQueue {
    async fn dispatch_enrichment(
        &self,
        _work: &Work,
        context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        {
            let mut count = self.dispatch_count.lock().await;
            *count += 1;
        }
        self.seen_contexts.lock().await.push(context);
        self.dispatch_started.notify_waiters();

        let plan = self
            .plans
            .lock()
            .await
            .pop_front()
            .expect("test queue plan missing");

        if let Some(persist) = plan.persist_outcomes {
            persist(plan.result.clone()).await;
        }

        if let Some(cb) = plan.before_return {
            cb().await;
        }

        if let Some(notify) = plan.hold_until {
            notify.notified().await;
        }

        Ok(plan.result)
    }

    fn circuit_state(&self, _provider: MetadataProvider) -> CircuitState {
        CircuitState::Closed
    }
}

type MockProviderQueue = StubProviderQueue;

#[derive(Clone, Default)]
struct StubMergeEngine {
    outputs: Arc<Mutex<VecDeque<Result<MergeOutput, MergeError>>>>,
    seen_inputs: Arc<Mutex<Vec<MergeInput>>>,
}

impl StubMergeEngine {
    fn with_output(output: MergeOutput) -> Self {
        let mut q = VecDeque::new();
        q.push_back(Ok(output));
        Self {
            outputs: Arc::new(Mutex::new(q)),
            seen_inputs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_outputs(outputs: Vec<Result<MergeOutput, MergeError>>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
            seen_inputs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn inputs(&self) -> Vec<MergeInput> {
        self.seen_inputs.lock().await.clone()
    }
}

impl MergeEngine for StubMergeEngine {
    fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError> {
        self.seen_inputs.blocking_lock().push(inputs);
        self.outputs
            .blocking_lock()
            .pop_front()
            .expect("test merge output missing")
    }
}

#[derive(Clone)]
struct SequencedApplyDb {
    inner: livrarr_db::sqlite::SqliteDb,
    sequence: Arc<Mutex<VecDeque<ApplyMergeOutcome>>>,
}

impl SequencedApplyDb {
    fn new(inner: livrarr_db::sqlite::SqliteDb, sequence: Vec<ApplyMergeOutcome>) -> Self {
        Self {
            inner,
            sequence: Arc::new(Mutex::new(sequence.into())),
        }
    }
}

impl WorkDb for SequencedApplyDb {
    async fn create_work(&self, req: CreateWorkDbRequest) -> Result<Work, DbError> {
        self.inner.create_work(req).await
    }

    async fn get_work(&self, user_id: UserId, work_id: WorkId) -> Result<Work, DbError> {
        self.inner.get_work(user_id, work_id).await
    }

    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        self.inner.list_works(user_id).await
    }

    async fn list_works_by_author(
        &self,
        user_id: UserId,
        author_id: livrarr_db::AuthorId,
    ) -> Result<Vec<Work>, DbError> {
        self.inner.list_works_by_author(user_id, author_id).await
    }

    async fn list_works_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
        sort_by: &str,
        sort_dir: &str,
    ) -> Result<(Vec<Work>, i64), DbError> {
        self.inner
            .list_works_paginated(user_id, page, per_page, sort_by, sort_dir)
            .await
    }

    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError> {
        self.inner.update_work_enrichment(user_id, id, req).await
    }

    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError> {
        self.inner
            .update_work_user_fields(user_id, work_id, req)
            .await
    }

    async fn set_cover_manual(
        &self,
        user_id: UserId,
        id: WorkId,
        manual: bool,
    ) -> Result<(), DbError> {
        self.inner.set_cover_manual(user_id, id, manual).await
    }

    async fn delete_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError> {
        self.inner.delete_work(user_id, id).await
    }

    async fn work_exists_by_ol_key(&self, user_id: UserId, ol_key: &str) -> Result<bool, DbError> {
        self.inner.work_exists_by_ol_key(user_id, ol_key).await
    }

    async fn list_works_for_enrichment(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        self.inner.list_works_for_enrichment(user_id).await
    }

    async fn list_works_by_author_ol_keys(
        &self,
        user_id: UserId,
        author_ol_key: &str,
    ) -> Result<Vec<String>, DbError> {
        self.inner
            .list_works_by_author_ol_keys(user_id, author_ol_key)
            .await
    }

    async fn find_by_normalized_match(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
    ) -> Result<Vec<Work>, DbError> {
        self.inner
            .find_by_normalized_match(user_id, title, author)
            .await
    }

    async fn reset_pending_enrichments(&self) -> Result<u64, DbError> {
        self.inner.reset_pending_enrichments().await
    }

    async fn list_monitored_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        self.inner.list_monitored_works_all_users().await
    }

    async fn set_enrichment_status_skipped(&self, id: WorkId) -> Result<(), DbError> {
        self.inner.set_enrichment_status_skipped(id).await
    }

    async fn apply_enrichment_merge(
        &self,
        req: ApplyEnrichmentMergeRequest,
    ) -> Result<ApplyMergeOutcome, DbError> {
        let queued = self.sequence.lock().await.pop_front();
        match queued {
            Some(ApplyMergeOutcome::Superseded) => Ok(ApplyMergeOutcome::Superseded),
            Some(ApplyMergeOutcome::Applied)
            | Some(ApplyMergeOutcome::NoChange)
            | Some(ApplyMergeOutcome::Deferred)
            | None => self.inner.apply_enrichment_merge(req).await,
        }
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        self.inner.reset_for_manual_refresh(user_id, work_id).await
    }

    async fn get_merge_generation(&self, user_id: UserId, work_id: WorkId) -> Result<i64, DbError> {
        self.inner.get_merge_generation(user_id, work_id).await
    }

    async fn list_conflict_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        self.inner.list_conflict_works(user_id).await
    }

    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError> {
        self.inner
            .search_works(user_id, query, page, per_page)
            .await
    }

    async fn list_work_provider_keys_by_author(
        &self,
        user_id: UserId,
        author_id: i64,
    ) -> Result<Vec<(Option<String>, Option<String>)>, DbError> {
        self.inner
            .list_work_provider_keys_by_author(user_id, author_id)
            .await
    }
}

#[async_trait]
impl ProvenanceDb for SequencedApplyDb {
    async fn get_field_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
        field: WorkField,
    ) -> Result<Option<FieldProvenance>, DbError> {
        self.inner
            .get_field_provenance(user_id, work_id, field)
            .await
    }

    async fn list_work_provenance(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<FieldProvenance>, DbError> {
        self.inner.list_work_provenance(user_id, work_id).await
    }

    async fn set_field_provenance(&self, req: SetFieldProvenanceRequest) -> Result<(), DbError> {
        self.inner.set_field_provenance(req).await
    }

    async fn set_field_provenance_batch(
        &self,
        reqs: Vec<SetFieldProvenanceRequest>,
    ) -> Result<(), DbError> {
        self.inner.set_field_provenance_batch(reqs).await
    }

    async fn delete_field_provenance_batch(
        &self,
        user_id: UserId,
        work_id: WorkId,
        fields: Vec<WorkField>,
    ) -> Result<(), DbError> {
        self.inner
            .delete_field_provenance_batch(user_id, work_id, fields)
            .await
    }

    async fn clear_work_provenance(&self, user_id: UserId, work_id: WorkId) -> Result<(), DbError> {
        self.inner.clear_work_provenance(user_id, work_id).await
    }
}

#[async_trait]
impl ProviderRetryStateDb for SequencedApplyDb {
    async fn get_retry_state(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
    ) -> Result<Option<livrarr_db::ProviderRetryState>, DbError> {
        self.inner.get_retry_state(user_id, work_id, provider).await
    }

    async fn list_retry_states(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<livrarr_db::ProviderRetryState>, DbError> {
        self.inner.list_retry_states(user_id, work_id).await
    }

    async fn record_will_retry(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        next_attempt_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<livrarr_db::ProviderRetryState, DbError> {
        self.inner
            .record_will_retry(user_id, work_id, provider, next_attempt_at)
            .await
    }

    async fn record_suppressed(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<livrarr_db::ProviderRetryState, DbError> {
        self.inner
            .record_suppressed(user_id, work_id, provider, until)
            .await
    }

    async fn record_terminal_outcome(
        &self,
        user_id: UserId,
        work_id: WorkId,
        provider: MetadataProvider,
        outcome: OutcomeClass,
        normalized_payload_json: Option<String>,
    ) -> Result<(), DbError> {
        self.inner
            .record_terminal_outcome(user_id, work_id, provider, outcome, normalized_payload_json)
            .await
    }

    async fn reset_all_retry_states(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        self.inner.reset_all_retry_states(user_id, work_id).await
    }

    async fn list_works_due_for_retry(
        &self,
        user_id: UserId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(WorkId, MetadataProvider)>, DbError> {
        self.inner.list_works_due_for_retry(user_id, now).await
    }

    async fn list_works_with_terminal_provider_rows(
        &self,
        user_id: UserId,
    ) -> Result<Vec<(WorkId, Vec<MetadataProvider>)>, DbError> {
        self.inner
            .list_works_with_terminal_provider_rows(user_id)
            .await
    }

    async fn reset_not_configured_outcomes(
        &self,
        provider: MetadataProvider,
    ) -> Result<u64, DbError> {
        self.inner.reset_not_configured_outcomes(provider).await
    }
}

#[async_trait]
impl ExternalIdDb for SequencedApplyDb {
    async fn upsert_external_id(
        &self,
        user_id: UserId,
        req: UpsertExternalIdRequest,
    ) -> Result<(), DbError> {
        self.inner.upsert_external_id(user_id, req).await
    }

    async fn upsert_external_ids_batch(
        &self,
        user_id: UserId,
        reqs: Vec<UpsertExternalIdRequest>,
    ) -> Result<(), DbError> {
        self.inner.upsert_external_ids_batch(user_id, reqs).await
    }

    async fn list_external_ids(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<livrarr_db::ExternalId>, DbError> {
        self.inner.list_external_ids(user_id, work_id).await
    }
}

fn make_work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        author_id: None,
        ol_key: None,
        year: Some(2024),
        cover_url: Some("https://example.test/original.jpg".to_string()),
        ..Default::default()
    }
}

async fn seed_work<DB: WorkDb>(db: &DB, user_id: UserId) -> Work {
    db.create_work(make_work_req(user_id, "Original Title", "Original Author"))
        .await
        .unwrap()
}

fn normalized_payload(
    title: &str,
    description: Option<&str>,
    cover_url: Option<&str>,
) -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some(title.to_string()),
        subtitle: None,
        original_title: None,
        author_name: Some("Provider Author".to_string()),
        description: description.map(ToString::to_string),
        year: Some(2024),
        series_name: None,
        series_position: None,
        genres: None,
        language: Some("en".to_string()),
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        hc_key: None,
        gr_key: Some("gr/work/123".to_string()),
        ol_key: Some("OL999W".to_string()),
        isbn_13: Some("9781234567890".to_string()),
        asin: Some("B00TEST123".to_string()),
        narrator: None,
        narration_type: Some(NarrationType::Human),
        abridged: Some(false),
        rating: Some(4.5),
        rating_count: Some(10),
        cover_url: cover_url.map(ToString::to_string),
        additional_isbns: vec!["9780000000001".to_string()],
        additional_asins: vec!["B00EXTRA1".to_string()],
    }
}

fn provider_outcomes_map(
    pairs: &[(MetadataProvider, ProviderOutcome<NormalizedWorkDetail>)],
) -> HashMap<MetadataProvider, ProviderOutcome<NormalizedWorkDetail>> {
    pairs.iter().cloned().collect()
}

fn outcome_classes(
    pairs: &[(MetadataProvider, OutcomeClass)],
) -> HashMap<MetadataProvider, OutcomeClass> {
    pairs.iter().copied().collect()
}

fn scatter_result(
    work_id: WorkId,
    outcomes: HashMap<MetadataProvider, ProviderOutcome<NormalizedWorkDetail>>,
    merge_eligible: bool,
    deferred: bool,
) -> ScatterGatherResult {
    ScatterGatherResult {
        work_id,
        outcomes,
        merge_eligible,
        deferred,
    }
}

fn merge_output_success(title: &str) -> MergeOutput {
    MergeOutput {
        conflict_detected: false,
        work_update: Some(MergeResolved::new(UpdateWorkEnrichmentDbRequest {
            title: Some(title.to_string()),
            subtitle: None,
            original_title: None,
            author_name: Some("Provider Author".to_string()),
            description: Some("Merged description".to_string()),
            year: Some(2024),
            series_name: None,
            series_position: None,
            genres: None,
            language: Some("en".to_string()),
            page_count: None,
            duration_seconds: None,
            publisher: None,
            publish_date: None,
            hc_key: None,
            gr_key: Some("gr/work/123".to_string()),
            ol_key: Some("OL999W".to_string()),
            isbn_13: Some("9781234567890".to_string()),
            asin: Some("B00TEST123".to_string()),
            narrator: None,
            narration_type: None,
            abridged: Some(false),
            rating: Some(4.5),
            rating_count: Some(10),
            cover_url: Some("https://example.test/provider.jpg".to_string()),
            ..Default::default()
        })),
        provenance_upserts: vec![SetFieldProvenanceRequest {
            user_id: 0,
            work_id: 0,
            field: WorkField::Title,
            source: Some(MetadataProvider::Goodreads),
            setter: ProvenanceSetter::Provider,
            cleared: false,
        }],
        provenance_deletes: vec![],
        external_id_updates: vec![
            UpsertExternalIdRequest {
                work_id: 0,
                id_type: livrarr_domain::ExternalIdType::Isbn13,
                id_value: "9780000000001".to_string(),
            },
            UpsertExternalIdRequest {
                work_id: 0,
                id_type: livrarr_domain::ExternalIdType::Asin,
                id_value: "B00EXTRA1".to_string(),
            },
        ],
        enrichment_status: EnrichmentStatus::Enriched,
        enrichment_source: Some("goodreads".to_string()),
    }
}

fn merge_output_conflict() -> MergeOutput {
    MergeOutput {
        conflict_detected: true,
        work_update: None,
        provenance_upserts: vec![],
        provenance_deletes: vec![],
        external_id_updates: vec![],
        enrichment_status: EnrichmentStatus::Conflict,
        enrichment_source: None,
    }
}

fn make_service<DB>(
    db: Arc<DB>,
    queue: StubProviderQueue,
    merge_engine: StubMergeEngine,
) -> impl EnrichmentService
where
    DB: WorkDb + ProvenanceDb + ProviderRetryStateDb + ExternalIdDb + Clone + Send + Sync + 'static,
{
    livrarr_metadata::EnrichmentServiceImpl::new(
        db,
        Arc::new(queue),
        Arc::new(merge_engine),
        Arc::new(livrarr_metadata::llm_validator::NoOpLlmValidator::new()),
    )
}

fn persist_scatter_result_hook<DB>(
    db: Arc<DB>,
    user_id: UserId,
) -> Arc<dyn Fn(ScatterGatherResult) -> BoxFuture<'static, ()> + Send + Sync>
where
    DB: ProviderRetryStateDb + Send + Sync + 'static,
{
    Arc::new(move |result: ScatterGatherResult| {
        let db = db.clone();
        Box::pin(async move {
            for (provider, outcome) in result.outcomes {
                match outcome {
                    ProviderOutcome::Success(payload) => {
                        db.record_terminal_outcome(
                            user_id,
                            result.work_id,
                            provider,
                            OutcomeClass::Success,
                            Some(serde_json::to_string(&*payload).unwrap()),
                        )
                        .await
                        .unwrap();
                    }
                    ProviderOutcome::NotFound => {
                        db.record_terminal_outcome(
                            user_id,
                            result.work_id,
                            provider,
                            OutcomeClass::NotFound,
                            None,
                        )
                        .await
                        .unwrap();
                    }
                    ProviderOutcome::PermanentFailure { .. } => {
                        db.record_terminal_outcome(
                            user_id,
                            result.work_id,
                            provider,
                            OutcomeClass::PermanentFailure,
                            None,
                        )
                        .await
                        .unwrap();
                    }
                    ProviderOutcome::Conflict { .. } => {
                        db.record_terminal_outcome(
                            user_id,
                            result.work_id,
                            provider,
                            OutcomeClass::Conflict,
                            None,
                        )
                        .await
                        .unwrap();
                    }
                    ProviderOutcome::WillRetry {
                        next_attempt_at, ..
                    } => {
                        db.record_will_retry(user_id, result.work_id, provider, next_attempt_at)
                            .await
                            .unwrap();
                    }
                    ProviderOutcome::Suppressed { until } => {
                        db.record_suppressed(user_id, result.work_id, provider, until)
                            .await
                            .unwrap();
                    }
                    ProviderOutcome::NotConfigured => {
                        db.record_terminal_outcome(
                            user_id,
                            result.work_id,
                            provider,
                            OutcomeClass::NotConfigured,
                            None,
                        )
                        .await
                        .unwrap();
                    }
                }
            }
        })
    })
}

fn coercion_test_will_retry_at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 3, 4, 5).unwrap()
}

fn coercion_test_suppressed_until() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 3, 14, 5).unwrap()
}

fn make_coercion_test_queue<DB>(db: Arc<DB>, user_id: UserId, work_id: WorkId) -> MockProviderQueue
where
    DB: ProviderRetryStateDb + Send + Sync + 'static,
{
    StubProviderQueue::with_plan(DispatchPlan {
        result: scatter_result(
            work_id,
            provider_outcomes_map(&[
                (
                    MetadataProvider::Hardcover,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: coercion_test_will_retry_at(),
                    },
                ),
                (
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::Suppressed {
                        until: coercion_test_suppressed_until(),
                    },
                ),
                (
                    MetadataProvider::Goodreads,
                    ProviderOutcome::Success(Box::new(normalized_payload(
                        "Manual Merge Title",
                        Some("Desc"),
                        Some("https://example.test/provider.jpg"),
                    ))),
                ),
            ]),
            false,
            true,
        ),
        persist_outcomes: Some(persist_scatter_result_hook(db, user_id)),
        before_return: None,
        hold_until: None,
    })
}

async fn set_work_language<DB: WorkDb>(db: &DB, work: &Work, language: &str) -> Work {
    let expected_generation = db
        .get_merge_generation(work.user_id, work.id)
        .await
        .unwrap();

    let outcome = db
        .apply_enrichment_merge(ApplyEnrichmentMergeRequest {
            user_id: work.user_id,
            work_id: work.id,
            expected_merge_generation: expected_generation,
            work_update: Some(MergeResolved::new(UpdateWorkEnrichmentDbRequest {
                title: Some(work.title.clone()),
                subtitle: work.subtitle.clone(),
                original_title: work.original_title.clone(),
                author_name: Some(work.author_name.clone()),
                description: work.description.clone(),
                year: work.year,
                series_name: work.series_name.clone(),
                series_position: work.series_position,
                genres: work.genres.clone(),
                language: Some(language.to_string()),
                page_count: work.page_count,
                duration_seconds: work.duration_seconds,
                publisher: work.publisher.clone(),
                publish_date: work.publish_date.clone(),
                hc_key: work.hc_key.clone(),
                gr_key: work.gr_key.clone(),
                ol_key: work.ol_key.clone(),
                isbn_13: work.isbn_13.clone(),
                asin: work.asin.clone(),
                narrator: work.narrator.clone(),
                narration_type: work.narration_type,
                abridged: Some(work.abridged),
                rating: work.rating,
                rating_count: work.rating_count,
                cover_url: work.cover_url.clone(),
                ..Default::default()
            })),
            new_enrichment_status: work.enrichment_status,
            provenance_upserts: vec![],
            provenance_deletes: vec![],
            external_id_updates: vec![],
        })
        .await
        .unwrap();

    assert_eq!(outcome, ApplyMergeOutcome::Applied);
    db.get_work(work.user_id, work.id).await.unwrap()
}

async fn mark_work_enriched<DB: WorkDb>(db: &DB, work: &Work) -> Work {
    let expected_generation = db
        .get_merge_generation(work.user_id, work.id)
        .await
        .unwrap();

    let outcome = db
        .apply_enrichment_merge(ApplyEnrichmentMergeRequest {
            user_id: work.user_id,
            work_id: work.id,
            expected_merge_generation: expected_generation,
            work_update: Some(MergeResolved::new(UpdateWorkEnrichmentDbRequest {
                title: Some(work.title.clone()),
                subtitle: work.subtitle.clone(),
                original_title: work.original_title.clone(),
                author_name: Some(work.author_name.clone()),
                description: Some("Already enriched".to_string()),
                year: work.year,
                series_name: work.series_name.clone(),
                series_position: work.series_position,
                genres: work.genres.clone(),
                language: work.language.clone(),
                page_count: work.page_count,
                duration_seconds: work.duration_seconds,
                publisher: work.publisher.clone(),
                publish_date: work.publish_date.clone(),
                hc_key: work.hc_key.clone(),
                gr_key: work.gr_key.clone(),
                ol_key: work.ol_key.clone(),
                isbn_13: work.isbn_13.clone(),
                asin: work.asin.clone(),
                narrator: work.narrator.clone(),
                narration_type: work.narration_type,
                abridged: Some(work.abridged),
                rating: work.rating,
                rating_count: work.rating_count,
                cover_url: Some("https://example.test/enriched.jpg".to_string()),
                ..Default::default()
            })),
            new_enrichment_status: EnrichmentStatus::Enriched,
            provenance_upserts: vec![],
            provenance_deletes: vec![],
            external_id_updates: vec![],
        })
        .await
        .unwrap();

    assert_eq!(outcome, ApplyMergeOutcome::Applied);
    db.get_work(work.user_id, work.id).await.unwrap()
}

#[macro_export]
macro_rules! enrichment_service_tests {
    ($harness:ty) => {
        #[tokio::test]
        async fn test_enrichment_service_enrich_work_nominal_success() {
            // REQ-ID: R-02, R-22 | Contract: EnrichmentService::enrich_work | Behavior: successful enrichment updates the work in DB and returns EnrichmentResult
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[
                        (
                            MetadataProvider::Goodreads,
                            ProviderOutcome::Success(Box::new(normalized_payload(
                                "Provider Title",
                                Some("Provider description"),
                                Some("https://example.test/provider.jpg"),
                            ))),
                        ),
                        (MetadataProvider::OpenLibrary, ProviderOutcome::NotFound),
                    ]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: None,
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::with_output(merge_output_success("Provider Title"));
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(db.clone(), queue, merge_engine);

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert_eq!(persisted.title, "Provider Title");
            assert_eq!(persisted.enrichment_status, EnrichmentStatus::Enriched);
            assert_eq!(result.work.id, work.id);
            assert_eq!(result.work.title, "Provider Title");
            assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
            assert!(!result.merge_deferred);
            assert_eq!(
                result.provider_outcomes,
                outcome_classes(&[
                    (MetadataProvider::Goodreads, OutcomeClass::Success),
                    (MetadataProvider::OpenLibrary, OutcomeClass::NotFound),
                ])
            );

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 1);
            assert_eq!(
                seen[0]
                    .provider_results
                    .get(&MetadataProvider::Goodreads)
                    .unwrap()
                    .class,
                OutcomeClass::Success
            );
            assert!(seen[0]
                .provider_results
                .get(&MetadataProvider::Goodreads)
                .unwrap()
                .payload
                .is_some());
            assert_eq!(
                seen[0]
                    .provider_results
                    .get(&MetadataProvider::OpenLibrary)
                    .unwrap()
                    .class,
                OutcomeClass::NotFound
            );
            assert!(seen[0]
                .provider_results
                .get(&MetadataProvider::OpenLibrary)
                .unwrap()
                .payload
                .is_none());
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_nominal_deferred_persists_phase1_retry_state() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: deferred enrichment returns merge_deferred=true and leaves queue-persisted phase-1 outcomes durable and unchanged
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let will_retry_at = Utc::now() + ChronoDuration::minutes(10);
            let suppressed_until = Utc::now() + ChronoDuration::minutes(20);

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[
                        (
                            MetadataProvider::Goodreads,
                            ProviderOutcome::WillRetry {
                                reason: livrarr_domain::WillRetryReason::Timeout,
                                next_attempt_at: will_retry_at,
                            },
                        ),
                        (
                            MetadataProvider::OpenLibrary,
                            ProviderOutcome::Suppressed {
                                until: suppressed_until,
                            },
                        ),
                        (
                            MetadataProvider::Hardcover,
                            ProviderOutcome::Success(Box::new(normalized_payload(
                                "HC Title",
                                Some("HC desc"),
                                Some("https://example.test/hc.jpg"),
                            ))),
                        ),
                    ]),
                    false,
                    true,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: None,
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::default();
            let service = make_service(db.clone(), queue, merge_engine);

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            assert!(result.merge_deferred);
            assert_eq!(
                result.provider_outcomes,
                outcome_classes(&[
                    (MetadataProvider::Goodreads, OutcomeClass::WillRetry),
                    (MetadataProvider::OpenLibrary, OutcomeClass::Suppressed),
                    (MetadataProvider::Hardcover, OutcomeClass::Success),
                ])
            );

            let gr = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
                .await
                .unwrap()
                .expect("goodreads retry state must exist");
            assert_eq!(gr.last_outcome, Some(OutcomeClass::WillRetry));
            assert_eq!(gr.next_attempt_at, Some(will_retry_at));

            let ol = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::OpenLibrary)
                .await
                .unwrap()
                .expect("openlibrary retry state must exist");
            assert_eq!(ol.last_outcome, Some(OutcomeClass::Suppressed));
            assert_eq!(ol.next_attempt_at, Some(suppressed_until));

            let hc = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::Hardcover)
                .await
                .unwrap()
                .expect("hardcover retry state must exist");
            assert_eq!(hc.last_outcome, Some(OutcomeClass::Success));
            assert!(hc.normalized_payload_json.is_some());
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_rereads_current_work_after_dispatch() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: merge input current_work is re-read from DB after dispatch returns, not taken from the pre-dispatch snapshot
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let db_for_update = db.clone();
            let work_for_closure = work.clone();
            let before_return = Arc::new(move || {
                let db_for_update = db_for_update.clone();
                let work = work_for_closure.clone();
                Box::pin(async move {
                    db_for_update
                        .update_work_user_fields(
                            user_id,
                            work.id,
                            UpdateWorkUserFieldsDbRequest {
                                title: Some("Changed During Dispatch".to_string()),
                                author_name: None,
                                series_name: None,
                                series_position: None,
                                ..Default::default()
                            },
                        )
                        .await
                        .unwrap();
                }) as BoxFuture<'static, ()>
            });

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Provider Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: Some(before_return),
                hold_until: None,
            });
            // The before_return hook calls update_work_user_fields(title=...), which
            // bumps merge_generation per [I-10]. The first apply_enrichment_merge
            // therefore returns Superseded and the service retries — so the merge
            // engine is invoked twice. Both invocations re-read current_work from
            // DB, both must see the updated title.
            let merge_engine = StubMergeEngine::with_outputs(vec![
                Ok(merge_output_success("Merged Title")),
                Ok(merge_output_success("Merged Title")),
            ]);
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(db.clone(), queue, merge_engine);

            let _ = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 2);
            assert_eq!(seen[0].current_work.title, "Changed During Dispatch");
            assert_eq!(seen[1].current_work.title, "Changed During Dispatch");
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_rereads_current_provenance_after_dispatch() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: merge input current_provenance is re-read from DB after dispatch returns
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let db_for_update = db.clone();
            let work_for_closure = work.clone();
            let before_return = Arc::new(move || {
                let db_for_update = db_for_update.clone();
                let work = work_for_closure.clone();
                Box::pin(async move {
                    db_for_update
                        .set_field_provenance(SetFieldProvenanceRequest {
                            user_id,
                            work_id: work.id,
                            field: WorkField::Title,
                            source: Some(MetadataProvider::Goodreads),
                            setter: ProvenanceSetter::Provider,
                            cleared: false,
                        })
                        .await
                        .unwrap();
                }) as BoxFuture<'static, ()>
            });

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Provider Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: Some(before_return),
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::with_output(merge_output_success("Merged Title"));
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(db.clone(), queue, merge_engine);

            let _ = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 1);
            assert!(seen[0]
                .current_provenance
                .iter()
                .any(|p| p.field == WorkField::Title
                    && p.setter == ProvenanceSetter::Provider
                    && p.source == Some(MetadataProvider::Goodreads)));
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_retries_once_after_superseded() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: one CAS superseded outcome causes exactly one retry and then succeeds
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Provider Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(Arc::new(h.db().clone()), user_id)),
                before_return: None,
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::with_outputs(vec![
                Ok(merge_output_success("Title After Retry")),
                Ok(merge_output_success("Title After Retry")),
            ]);

            let seq_db = SequencedApplyDb::new(
                h.db().clone(),
                vec![ApplyMergeOutcome::Superseded, ApplyMergeOutcome::Applied],
            );

            let service = livrarr_metadata::EnrichmentServiceImpl::new(
                Arc::new(seq_db),
                Arc::new(queue),
                Arc::new(merge_engine),
                Arc::new(livrarr_metadata::llm_validator::NoOpLlmValidator::new()),
            );

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
            assert_eq!(persisted.title, "Title After Retry");
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_returns_merge_superseded_after_three_superseded_attempts() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: three CAS superseded outcomes exhaust retries and return MergeSuperseded
            let h = <$harness as DbTestHarness>::setup().await;
            let work = seed_work(h.db(), h.user_id()).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Provider Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(
                    Arc::new(h.db().clone()),
                    h.user_id(),
                )),
                before_return: None,
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::with_outputs(vec![
                Ok(merge_output_success("Never Applied")),
                Ok(merge_output_success("Never Applied")),
                Ok(merge_output_success("Never Applied")),
            ]);

            let seq_db = SequencedApplyDb::new(
                h.db().clone(),
                vec![
                    ApplyMergeOutcome::Superseded,
                    ApplyMergeOutcome::Superseded,
                    ApplyMergeOutcome::Superseded,
                ],
            );

            let service = livrarr_metadata::EnrichmentServiceImpl::new(
                Arc::new(seq_db),
                Arc::new(queue),
                Arc::new(merge_engine),
                Arc::new(livrarr_metadata::llm_validator::NoOpLlmValidator::new()),
            );

            let err = service
                .enrich_work(h.user_id(), work.id, EnrichmentMode::Background)
                .await
                .unwrap_err();

            assert_matches!(err, EnrichmentError::MergeSuperseded);
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_serializes_same_work_calls_with_lock() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: concurrent enrichments for the same (user_id, work_id) are mutually exclusive for the full call duration
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let work = seed_work(h.db(), h.user_id()).await;

            let release_first = Arc::new(Notify::new());
            let queue = StubProviderQueue::with_plans(vec![
                DispatchPlan {
                    result: scatter_result(
                        work.id,
                        provider_outcomes_map(&[(
                            MetadataProvider::Goodreads,
                            ProviderOutcome::Success(Box::new(normalized_payload(
                                "First",
                                Some("Desc"),
                                Some("https://example.test/first.jpg"),
                            ))),
                        )]),
                        true,
                        false,
                    ),
                    persist_outcomes: Some(persist_scatter_result_hook(db.clone(), h.user_id())),
                    before_return: None,
                    hold_until: Some(release_first.clone()),
                },
                DispatchPlan {
                    result: scatter_result(
                        work.id,
                        provider_outcomes_map(&[(
                            MetadataProvider::Goodreads,
                            ProviderOutcome::Success(Box::new(normalized_payload(
                                "Second",
                                Some("Desc"),
                                Some("https://example.test/second.jpg"),
                            ))),
                        )]),
                        true,
                        false,
                    ),
                    persist_outcomes: Some(persist_scatter_result_hook(db.clone(), h.user_id())),
                    before_return: None,
                    hold_until: None,
                },
            ]);
            let queue_observer = queue.clone();

            let merge_engine = StubMergeEngine::with_outputs(vec![
                Ok(merge_output_success("First")),
                Ok(merge_output_success("Second")),
            ]);

            let service = Arc::new(make_service(db.clone(), queue, merge_engine));

            let first_service = service.clone();
            let first_work = work.clone();
            let first = tokio::spawn(async move {
                first_service
                    .enrich_work(first_work.user_id, first_work.id, EnrichmentMode::Background)
                    .await
                    .unwrap()
            });

            queue_observer.dispatch_started.notified().await;

            let second_service = service.clone();
            let second_work = work.clone();
            let (second_started_tx, second_started_rx) = tokio::sync::oneshot::channel();
            let second = tokio::spawn(async move {
                let _ = second_started_tx.send(());
                second_service
                    .enrich_work(second_work.user_id, second_work.id, EnrichmentMode::Background)
                    .await
                    .unwrap()
            });

            second_started_rx.await.unwrap();

            assert_eq!(
                queue_observer.dispatch_count().await,
                1,
                "second call must not enter provider dispatch while first call holds the per-work lock"
            );
            assert!(
                !second.is_finished(),
                "second call must still be blocked behind the first call's same-work lock"
            );

            // notify_one() rather than notify_waiters() so the permit persists
            // even if the first task hasn't reached hold_until.notified().await
            // by the time we observe dispatch_count==1. notify_waiters() has no
            // permit memory and races with the first task's progression past
            // notify/persist/before_return.
            release_first.notify_one();

            let _ = first.await.unwrap();
            let _ = second.await.unwrap();

            assert_eq!(queue_observer.dispatch_count().await, 2);
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_conflict_path_writes_status_only_conflict() {
            // REQ-ID: R-02, R-22 | Contract: EnrichmentService::enrich_work | Behavior: merge conflict causes status-only apply with work.enrichment_status set to Conflict
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;
            let original_title = work.title.clone();

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Conflict {
                            detail: "gr_key mismatch".to_string(),
                        },
                    )]),
                    false,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(
                    Arc::new(h.db().clone()),
                    user_id,
                )),
                before_return: None,
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::with_output(merge_output_conflict());
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert_eq!(persisted.title, original_title);
            assert_eq!(persisted.enrichment_status, EnrichmentStatus::Conflict);
            assert_eq!(result.enrichment_status, EnrichmentStatus::Conflict);
            assert_eq!(
                result.provider_outcomes,
                outcome_classes(&[(MetadataProvider::Goodreads, OutcomeClass::Conflict)])
            );

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 1);
            assert_eq!(
                seen[0]
                    .provider_results
                    .get(&MetadataProvider::Goodreads)
                    .unwrap()
                    .class,
                OutcomeClass::Conflict
            );
            assert!(seen[0]
                .provider_results
                .get(&MetadataProvider::Goodreads)
                .unwrap()
                .payload
                .is_none());
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_returns_corrupt_retry_payload_when_success_payload_is_unparseable() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: invalid normalized_payload_json for a Success retry-state row returns CorruptRetryPayload
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let bad_db = Arc::new(h.db().clone());
            let work_for_closure = work.clone();
            let before_return = Arc::new(move || {
                let bad_db = bad_db.clone();
                let work = work_for_closure.clone();
                Box::pin(async move {
                    bad_db
                        .record_terminal_outcome(
                            user_id,
                            work.id,
                            MetadataProvider::Goodreads,
                            OutcomeClass::Success,
                            Some("{ not valid json".to_string()),
                        )
                        .await
                        .unwrap();
                }) as BoxFuture<'static, ()>
            });

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Provider Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: None,
                before_return: Some(before_return),
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::default();
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let err = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap_err();

            assert_matches!(
                err,
                EnrichmentError::CorruptRetryPayload {
                    work_id,
                    provider: MetadataProvider::Goodreads
                } if work_id == work.id
            );
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_background_will_retry_and_suppressed_defer_merge() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: background mode defers merge when any provider outcome is WillRetry or Suppressed
            let h = <$harness as DbTestHarness>::setup().await;
            let work = seed_work(h.db(), h.user_id()).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[
                        (
                            MetadataProvider::Goodreads,
                            ProviderOutcome::WillRetry {
                                reason: livrarr_domain::WillRetryReason::RateLimit,
                                next_attempt_at: Utc::now() + ChronoDuration::minutes(5),
                            },
                        ),
                        (
                            MetadataProvider::OpenLibrary,
                            ProviderOutcome::Suppressed {
                                until: Utc::now() + ChronoDuration::minutes(10),
                            },
                        ),
                    ]),
                    false,
                    true,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(
                    Arc::new(h.db().clone()),
                    h.user_id(),
                )),
                before_return: None,
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::default();
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let result = service
                .enrich_work(h.user_id(), work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            assert!(result.merge_deferred);
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_manual_coerces_will_retry_and_suppressed_to_immediate_merge() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: with the same queue outcomes that would defer in background mode, manual mode still returns non-deferred and merges immediately, proving service-level coercion
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;
            let db = Arc::new(h.db().clone());

            let queue = make_coercion_test_queue(db.clone(), user_id, work.id);
            let queue_observer = queue.clone();
            let merge_engine = StubMergeEngine::with_output(merge_output_success("Manual Merge Title"));
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Manual)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert!(
                !result.merge_deferred,
                "queue reported deferred=true, so a non-deferred manual result proves the service coerced the outcome"
            );
            assert_eq!(
                result.provider_outcomes,
                outcome_classes(&[
                    (MetadataProvider::Hardcover, OutcomeClass::WillRetry),
                    (MetadataProvider::OpenLibrary, OutcomeClass::Suppressed),
                    (MetadataProvider::Goodreads, OutcomeClass::Success),
                ])
            );
            assert_eq!(persisted.title, "Manual Merge Title");
            assert_eq!(persisted.enrichment_status, EnrichmentStatus::Enriched);

            let seen_contexts = queue_observer.contexts().await;
            assert_eq!(seen_contexts.len(), 1);
            assert_eq!(seen_contexts[0].mode, EnrichmentMode::Manual);

            let seen_inputs = merge_engine_observer.inputs().await;
            assert_eq!(seen_inputs.len(), 1);
            assert_eq!(
                seen_inputs[0]
                    .provider_results
                    .get(&MetadataProvider::Hardcover)
                    .unwrap()
                    .class,
                OutcomeClass::WillRetry
            );
            assert_eq!(
                seen_inputs[0]
                    .provider_results
                    .get(&MetadataProvider::OpenLibrary)
                    .unwrap()
                    .class,
                OutcomeClass::Suppressed
            );
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_background_does_not_coerce_non_merge_eligible_will_retry_and_suppressed() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: with identical queue outcomes to the manual coercion case, background mode preserves deferred=true and does not merge
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;
            let db = Arc::new(h.db().clone());

            let queue = make_coercion_test_queue(db.clone(), user_id, work.id);
            let merge_engine = StubMergeEngine::default();
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert!(
                result.merge_deferred,
                "with the same queue deferred=true outcome used by the manual test, background mode must remain deferred"
            );
            assert_eq!(
                result.provider_outcomes,
                outcome_classes(&[
                    (MetadataProvider::Hardcover, OutcomeClass::WillRetry),
                    (MetadataProvider::OpenLibrary, OutcomeClass::Suppressed),
                    (MetadataProvider::Goodreads, OutcomeClass::Success),
                ])
            );
            assert_eq!(persisted.title, work.title);
            assert!(merge_engine_observer.inputs().await.is_empty());
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_updates_db_status_when_merge_not_deferred() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: if merge is not deferred, enrichment_status is updated in DB
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Applied Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(
                    Arc::new(h.db().clone()),
                    user_id,
                )),
                before_return: None,
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::with_output(merge_output_success("Applied Title"));
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert!(!result.merge_deferred);
            assert_eq!(persisted.enrichment_status, result.enrichment_status);
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_provider_outcomes_reflect_actual_outcome_class_per_provider() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: EnrichmentResult.provider_outcomes reports the actual OutcomeClass for each provider
            let h = <$harness as DbTestHarness>::setup().await;
            let work = seed_work(h.db(), h.user_id()).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[
                        (
                            MetadataProvider::Goodreads,
                            ProviderOutcome::Success(Box::new(normalized_payload(
                                "T",
                                Some("D"),
                                Some("https://example.test/gr.jpg"),
                            ))),
                        ),
                        (MetadataProvider::OpenLibrary, ProviderOutcome::NotFound),
                        (
                            MetadataProvider::Hardcover,
                            ProviderOutcome::PermanentFailure {
                                reason: PermanentFailureReason::InvalidResponse,
                            },
                        ),
                        (
                            MetadataProvider::Audnexus,
                            ProviderOutcome::Suppressed {
                                until: Utc::now() + ChronoDuration::minutes(10),
                            },
                        ),
                    ]),
                    false,
                    true,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(
                    Arc::new(h.db().clone()),
                    h.user_id(),
                )),
                before_return: None,
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::default();
            let service = make_service(Arc::new(h.db().clone()), queue, merge_engine);

            let result = service
                .enrich_work(h.user_id(), work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            assert_eq!(
                result.provider_outcomes,
                outcome_classes(&[
                    (MetadataProvider::Goodreads, OutcomeClass::Success),
                    (MetadataProvider::OpenLibrary, OutcomeClass::NotFound),
                    (MetadataProvider::Hardcover, OutcomeClass::PermanentFailure),
                    (MetadataProvider::Audnexus, OutcomeClass::Suppressed),
                ])
            );
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_deferred_does_not_modify_queue_written_retry_rows() {
            // REQ-ID: R-22 | Contract: EnrichmentService::enrich_work | Behavior: when dispatch already durably wrote phase-1 retry rows, deferred return leaves those rows unchanged
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let will_retry_at = Utc::now() + ChronoDuration::minutes(7);
            let suppressed_until = Utc::now() + ChronoDuration::minutes(13);

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[
                        (
                            MetadataProvider::Goodreads,
                            ProviderOutcome::WillRetry {
                                reason: livrarr_domain::WillRetryReason::Timeout,
                                next_attempt_at: will_retry_at,
                            },
                        ),
                        (
                            MetadataProvider::OpenLibrary,
                            ProviderOutcome::Suppressed {
                                until: suppressed_until,
                            },
                        ),
                    ]),
                    false,
                    true,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: None,
                hold_until: None,
            });

            let service = make_service(db.clone(), queue, StubMergeEngine::default());

            let _ = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let gr = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(gr.last_outcome, Some(OutcomeClass::WillRetry));
            assert_eq!(gr.next_attempt_at, Some(will_retry_at));

            let ol = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::OpenLibrary)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ol.last_outcome, Some(OutcomeClass::Suppressed));
            assert_eq!(ol.next_attempt_at, Some(suppressed_until));
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_retries_after_real_db_generation_change_during_dispatch() {
            // REQ-ID: R-02, R-22 | Contract: EnrichmentService::enrich_work | Behavior: a real merge_generation change during dispatch causes first apply_enrichment_merge to be Superseded and the service retries using a fresh generation
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let db_for_update = db.clone();
            let work_for_closure = work.clone();
            let before_return = Arc::new(move || {
                let db_for_update = db_for_update.clone();
                let work = work_for_closure.clone();
                Box::pin(async move {
                    db_for_update
                        .update_work_user_fields(
                            user_id,
                            work.id,
                            UpdateWorkUserFieldsDbRequest {
                                title: Some("Concurrent Edit".to_string()),
                                author_name: None,
                                series_name: None,
                                series_position: None,
                                ..Default::default()
                            },
                        )
                        .await
                        .unwrap();
                }) as BoxFuture<'static, ()>
            });

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Provider Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: Some(before_return),
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::with_outputs(vec![
                Ok(merge_output_success("After CAS Retry")),
                Ok(merge_output_success("After CAS Retry")),
            ]);
            let merge_engine_observer = merge_engine.clone();

            let service = livrarr_metadata::EnrichmentServiceImpl::new(
                db.clone(),
                Arc::new(queue),
                Arc::new(merge_engine),
                Arc::new(livrarr_metadata::llm_validator::NoOpLlmValidator::new()),
            );

            let result = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let persisted = h.db().get_work(user_id, work.id).await.unwrap();
            assert_eq!(persisted.title, "After CAS Retry");
            assert_eq!(result.work.title, "After CAS Retry");

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 2);
            assert_eq!(seen[0].current_work.title, "Concurrent Edit");
            assert_eq!(seen[1].current_work.title, "Concurrent Edit");
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_hard_refresh_passes_hard_refresh_mode_to_merge_engine() {
            // REQ-ID: R-02, R-22 | Contract: EnrichmentService::enrich_work | Behavior: HardRefresh mode is forwarded to MergeEngine in MergeInput.mode
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Hard Refresh Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: None,
                hold_until: None,
            });
            let merge_engine = StubMergeEngine::with_output(merge_output_success("Hard Refresh Title"));
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(db.clone(), queue, merge_engine);

            let _ = service
                .enrich_work(user_id, work.id, EnrichmentMode::HardRefresh)
                .await
                .unwrap();

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].mode, EnrichmentMode::HardRefresh);
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_uses_english_priority_model_for_english_work() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: english-language work selects PriorityModel::english for merge input
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let seeded = seed_work(h.db(), user_id).await;
            let work = set_work_language(h.db(), &seeded, "en").await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "English Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: None,
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::with_output(merge_output_success("English Title"));
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(db.clone(), queue, merge_engine);

            let _ = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 1);
            let pm = &seen[0].priority_model;
            assert_eq!(
                pm.content,
                vec![
                    MetadataProvider::Hardcover,
                    MetadataProvider::Goodreads,
                    MetadataProvider::OpenLibrary
                ]
            );
            assert_eq!(
                pm.description,
                vec![
                    MetadataProvider::Hardcover,
                    MetadataProvider::OpenLibrary,
                    MetadataProvider::Goodreads
                ]
            );
            assert_eq!(
                pm.cover,
                vec![
                    MetadataProvider::Hardcover,
                    MetadataProvider::Goodreads,
                    MetadataProvider::OpenLibrary
                ]
            );
        }

        #[tokio::test]
        async fn test_enrichment_service_enrich_work_uses_foreign_priority_model_for_foreign_work() {
            // REQ-ID: R-02 | Contract: EnrichmentService::enrich_work | Behavior: foreign-language work selects PriorityModel::foreign for merge input
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let seeded = seed_work(h.db(), user_id).await;
            let work = set_work_language(h.db(), &seeded, "fr").await;

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[(
                        MetadataProvider::Goodreads,
                        ProviderOutcome::Success(Box::new(normalized_payload(
                            "Foreign Title",
                            Some("Desc"),
                            Some("https://example.test/provider.jpg"),
                        ))),
                    )]),
                    true,
                    false,
                ),
                persist_outcomes: Some(persist_scatter_result_hook(db.clone(), user_id)),
                before_return: None,
                hold_until: None,
            });

            let merge_engine = StubMergeEngine::with_output(merge_output_success("Foreign Title"));
            let merge_engine_observer = merge_engine.clone();
            let service = make_service(db.clone(), queue, merge_engine);

            let _ = service
                .enrich_work(user_id, work.id, EnrichmentMode::Background)
                .await
                .unwrap();

            let seen = merge_engine_observer.inputs().await;
            assert_eq!(seen.len(), 1);
            let pm = &seen[0].priority_model;
            assert_eq!(pm.content, vec![MetadataProvider::Goodreads]);
            assert_eq!(pm.description, vec![MetadataProvider::Goodreads]);
            assert_eq!(pm.cover, vec![MetadataProvider::Goodreads]);
        }

        #[tokio::test]
        async fn test_enrichment_service_reset_for_manual_refresh_sets_pending_clears_enriched_at_and_bumps_generation() {
            // REQ-ID: R-20, R-21 | Contract: EnrichmentService::reset_for_manual_refresh | Behavior: reset sets status=Pending, clears enriched_at, increments merge_generation, clears all provider_retry_state rows, and preserves provenance rows
            let h = <$harness as DbTestHarness>::setup().await;
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;
            let enriched = mark_work_enriched(h.db(), &work).await;

            assert_eq!(enriched.enrichment_status, EnrichmentStatus::Enriched);
            assert!(enriched.enriched_at.is_some());

            h.db()
                .set_field_provenance(SetFieldProvenanceRequest {
                    user_id,
                    work_id: work.id,
                    field: WorkField::Title,
                    source: None,
                    setter: ProvenanceSetter::User,
                    cleared: false,
                })
                .await
                .unwrap();

            h.db()
                .record_will_retry(
                    user_id,
                    work.id,
                    MetadataProvider::Goodreads,
                    Utc::now() + ChronoDuration::minutes(5),
                )
                .await
                .unwrap();
            h.db()
                .record_suppressed(
                    user_id,
                    work.id,
                    MetadataProvider::OpenLibrary,
                    Utc::now() + ChronoDuration::minutes(10),
                )
                .await
                .unwrap();

            let provenance_before = h.db().list_work_provenance(user_id, work.id).await.unwrap();
            assert!(provenance_before
                .iter()
                .any(|p| p.field == WorkField::Title
                    && p.setter == ProvenanceSetter::User
                    && p.source.is_none()));

            let generation_before = h.db().get_merge_generation(user_id, work.id).await.unwrap();

            let service = make_service(
                Arc::new(h.db().clone()),
                StubProviderQueue::default(),
                StubMergeEngine::default(),
            );

            service
                .reset_for_manual_refresh(user_id, work.id)
                .await
                .unwrap();

            let reset = h.db().get_work(user_id, work.id).await.unwrap();
            let generation_after = h.db().get_merge_generation(user_id, work.id).await.unwrap();

            assert_eq!(reset.enrichment_status, EnrichmentStatus::Pending);
            assert!(reset.enriched_at.is_none());
            assert_eq!(generation_after, generation_before + 1);

            let gr_after = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
                .await
                .unwrap();
            let ol_after = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::OpenLibrary)
                .await
                .unwrap();
            assert!(gr_after.is_none());
            assert!(ol_after.is_none());

            let provenance_after = h.db().list_work_provenance(user_id, work.id).await.unwrap();
            assert!(provenance_after
                .iter()
                .any(|p| p.field == WorkField::Title
                    && p.setter == ProvenanceSetter::User
                    && p.source.is_none()));
        }

        #[tokio::test]
        async fn test_enrichment_service_reset_for_manual_refresh_serializes_with_enrich_work_for_same_work() {
            // REQ-ID: R-20, R-21, R-22 | Contract: EnrichmentService::reset_for_manual_refresh + EnrichmentService::enrich_work | Behavior: concurrent reset and enrich for the same (user_id, work_id) are serialized so reset cannot clear provider_retry_state rows until the in-flight enrich releases the same-work lock
            let h = <$harness as DbTestHarness>::setup().await;
            let db = Arc::new(h.db().clone());
            let user_id = h.user_id();
            let work = seed_work(h.db(), user_id).await;

            let release_enrich = Arc::new(Notify::new());
            let phase1_persisted = Arc::new(Notify::new());
            let persist_outcomes = {
                let db = db.clone();
                let phase1_persisted = phase1_persisted.clone();
                Arc::new(move |result: ScatterGatherResult| {
                    let db = db.clone();
                    let phase1_persisted = phase1_persisted.clone();
                    Box::pin(async move {
                        for (provider, outcome) in result.outcomes {
                            match outcome {
                                ProviderOutcome::WillRetry {
                                    next_attempt_at, ..
                                } => {
                                    db.record_will_retry(
                                        user_id,
                                        result.work_id,
                                        provider,
                                        next_attempt_at,
                                    )
                                    .await
                                    .unwrap();
                                }
                                ProviderOutcome::Suppressed { until } => {
                                    db.record_suppressed(user_id, result.work_id, provider, until)
                                        .await
                                        .unwrap();
                                }
                                ProviderOutcome::Success(payload) => {
                                    db.record_terminal_outcome(
                                        user_id,
                                        result.work_id,
                                        provider,
                                        OutcomeClass::Success,
                                        Some(serde_json::to_string(&*payload).unwrap()),
                                    )
                                    .await
                                    .unwrap();
                                }
                                ProviderOutcome::NotFound => {
                                    db.record_terminal_outcome(
                                        user_id,
                                        result.work_id,
                                        provider,
                                        OutcomeClass::NotFound,
                                        None,
                                    )
                                    .await
                                    .unwrap();
                                }
                                ProviderOutcome::PermanentFailure { .. } => {
                                    db.record_terminal_outcome(
                                        user_id,
                                        result.work_id,
                                        provider,
                                        OutcomeClass::PermanentFailure,
                                        None,
                                    )
                                    .await
                                    .unwrap();
                                }
                                ProviderOutcome::Conflict { .. } => {
                                    db.record_terminal_outcome(
                                        user_id,
                                        result.work_id,
                                        provider,
                                        OutcomeClass::Conflict,
                                        None,
                                    )
                                    .await
                                    .unwrap();
                                }
                                ProviderOutcome::NotConfigured => {
                                    db.record_terminal_outcome(
                                        user_id,
                                        result.work_id,
                                        provider,
                                        OutcomeClass::NotConfigured,
                                        None,
                                    )
                                    .await
                                    .unwrap();
                                }
                            }
                        }
                        phase1_persisted.notify_waiters();
                    }) as BoxFuture<'static, ()>
                }) as Arc<dyn Fn(ScatterGatherResult) -> BoxFuture<'static, ()> + Send + Sync>
            };

            let queue = StubProviderQueue::with_plan(DispatchPlan {
                result: scatter_result(
                    work.id,
                    provider_outcomes_map(&[
                        (
                            MetadataProvider::Goodreads,
                            ProviderOutcome::WillRetry {
                                reason: livrarr_domain::WillRetryReason::Timeout,
                                next_attempt_at: Utc::now() + ChronoDuration::minutes(5),
                            },
                        ),
                        (
                            MetadataProvider::OpenLibrary,
                            ProviderOutcome::Suppressed {
                                until: Utc::now() + ChronoDuration::minutes(10),
                            },
                        ),
                    ]),
                    false,
                    true,
                ),
                persist_outcomes: Some(persist_outcomes),
                before_return: None,
                hold_until: Some(release_enrich.clone()),
            });
            let queue_observer = queue.clone();

            let service = Arc::new(make_service(
                db.clone(),
                queue,
                StubMergeEngine::default(),
            ));

            let enrich_service = service.clone();
            let enrich = tokio::spawn(async move {
                enrich_service
                    .enrich_work(user_id, work.id, EnrichmentMode::Background)
                    .await
                    .unwrap()
            });

            queue_observer.dispatch_started.notified().await;
            phase1_persisted.notified().await;

            let reset_service = service.clone();
            let (reset_started_tx, reset_started_rx) = tokio::sync::oneshot::channel();
            let reset = tokio::spawn(async move {
                let _ = reset_started_tx.send(());
                reset_service
                    .reset_for_manual_refresh(user_id, work.id)
                    .await
                    .unwrap()
            });

            reset_started_rx.await.unwrap();

            let gr_during_enrich = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
                .await
                .unwrap();
            assert!(
                gr_during_enrich.is_some(),
                "phase-1 rows written by enrich_work must remain present while reset is blocked on the same-work lock"
            );
            assert!(
                !reset.is_finished(),
                "reset_for_manual_refresh must still be waiting for the same-work lock while enrich_work is in flight"
            );
            assert_eq!(
                queue_observer.dispatch_count().await,
                1,
                "reset_for_manual_refresh must not trigger a second provider dispatch"
            );

            release_enrich.notify_waiters();

            let enrich_result = enrich.await.unwrap();
            assert!(enrich_result.merge_deferred);
            reset.await.unwrap();

            let gr_after = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::Goodreads)
                .await
                .unwrap();
            let ol_after = h
                .db()
                .get_retry_state(user_id, work.id, MetadataProvider::OpenLibrary)
                .await
                .unwrap();
            let reset_work = h.db().get_work(user_id, work.id).await.unwrap();

            assert!(gr_after.is_none());
            assert!(ol_after.is_none());
            assert_eq!(reset_work.enrichment_status, EnrichmentStatus::Pending);
            assert_eq!(queue_observer.dispatch_count().await, 1);
        }

        #[tokio::test]
        async fn test_enrichment_service_reset_for_manual_refresh_does_not_affect_other_users_work() {
            // REQ-ID: R-20, R-21 | Contract: EnrichmentService::reset_for_manual_refresh | Behavior: resetting one user's work does not affect another user's work
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();

            let user_a = h.user_id();
            let user_b = db
                .create_user(CreateUserDbRequest {
                    username: "enrichment-test-user-b".to_string(),
                    password_hash: "hash".to_string(),
                    role: UserRole::Admin,
                    api_key_hash: "api-b".to_string(),
                })
                .await
                .unwrap()
                .id;

            let work_a = seed_work(db, user_a).await;
            let work_b = seed_work(db, user_b).await;

            let enriched_a = mark_work_enriched(db, &work_a).await;
            let enriched_b = mark_work_enriched(db, &work_b).await;

            assert_eq!(enriched_a.enrichment_status, EnrichmentStatus::Enriched);
            assert_eq!(enriched_b.enrichment_status, EnrichmentStatus::Enriched);

            let gen_a_before = db.get_merge_generation(user_a, work_a.id).await.unwrap();
            let gen_b_before = db.get_merge_generation(user_b, work_b.id).await.unwrap();

            let service = make_service(
                Arc::new(db.clone()),
                StubProviderQueue::default(),
                StubMergeEngine::default(),
            );

            service
                .reset_for_manual_refresh(user_a, work_a.id)
                .await
                .unwrap();

            let after_a = db.get_work(user_a, work_a.id).await.unwrap();
            let after_b = db.get_work(user_b, work_b.id).await.unwrap();
            let gen_a_after = db.get_merge_generation(user_a, work_a.id).await.unwrap();
            let gen_b_after = db.get_merge_generation(user_b, work_b.id).await.unwrap();

            assert_eq!(after_a.enrichment_status, EnrichmentStatus::Pending);
            assert!(after_a.enriched_at.is_none());
            assert_eq!(gen_a_after, gen_a_before + 1);

            assert_eq!(after_b.enrichment_status, EnrichmentStatus::Enriched);
            assert!(after_b.enriched_at.is_some());
            assert_eq!(gen_b_after, gen_b_before);
        }
    };
}

enrichment_service_tests!(SqliteHarness);
