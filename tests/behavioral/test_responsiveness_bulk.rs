use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use livrarr_domain::services::{
    AddWorkResult, BulkRefreshGuard, ConvergeOutcome, MergeFieldChoiceEntry, MergePreview,
    MergeWorksResult, PaginatedWorksView, RefreshSurface, RefreshWorkResult, RetrySummary,
    SortDirection, SourceProviderData, UpdateWorkRequest, WorkDetailView, WorkFilter, WorkService,
    WorkServiceError, WorkSortField,
};
use livrarr_domain::{MediaType, UserId, Work, WorkId};

#[derive(Clone)]
struct BulkRefreshStub {
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
    calls: Arc<Mutex<Vec<WorkId>>>,
    fail_work_id: WorkId,
}

impl BulkRefreshStub {
    fn new(fail_work_id: WorkId) -> Self {
        Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_in_flight: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_work_id,
        }
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    fn calls(&self) -> Vec<WorkId> {
        self.calls.lock().unwrap().clone()
    }

    fn record_max(&self, current: usize) {
        let mut observed = self.max_in_flight.load(Ordering::SeqCst);
        while current > observed {
            match self.max_in_flight.compare_exchange(
                observed,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(next) => observed = next,
            }
        }
    }
}

impl WorkService for BulkRefreshStub {
    async fn add(
        &self,
        _user_id: UserId,
        _candidate: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn resolve_identity(
        &self,
        _user_id: UserId,
        _harvest: livrarr_domain::identity::RawHarvest,
        _tier: livrarr_domain::identity::LatencyTier,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        unimplemented!("not exercised")
    }

    fn resolve_identity_local(
        &self,
        _harvest: livrarr_domain::identity::RawHarvest,
    ) -> Result<livrarr_domain::identity::ResolvedIdentity, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn add_fast(
        &self,
        _user_id: UserId,
        _candidate: livrarr_domain::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn complete_add(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _source_provider_data: Option<SourceProviderData>,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _mode: livrarr_domain::identity::IdentityMode,
        _source: livrarr_domain::identity::ConflictSource,
    ) {
        unimplemented!("not exercised")
    }

    fn is_enriching(&self, _user_id: UserId, _work_id: WorkId) -> bool {
        unimplemented!("not exercised")
    }

    async fn get(&self, _user_id: UserId, _work_id: WorkId) -> Result<Work, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn get_detail(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn list(
        &self,
        _user_id: UserId,
        _filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn list_paginated(
        &self,
        _user_id: UserId,
        _page: u32,
        _page_size: u32,
        _sort_by: WorkSortField,
        _sort_dir: SortDirection,
        _media_type: Option<MediaType>,
        _language: Option<&str>,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn update(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn delete(&self, _user_id: UserId, _work_id: WorkId) -> Result<(), WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
        surface: RefreshSurface,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        assert_eq!(
            surface,
            RefreshSurface::Bulk,
            "AC-019: bulk sweep must refresh each work through the Bulk surface"
        );

        self.calls.lock().unwrap().push(work_id);

        let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_max(current);
        tokio::time::sleep(Duration::from_millis(40)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);

        if work_id == self.fail_work_id {
            Err(WorkServiceError::Enrichment("rigged failure".into()))
        } else {
            Ok(RefreshWorkResult {
                work: Work {
                    id: work_id,
                    user_id,
                    ..Default::default()
                },
                messages: vec![],
                taggable_items: vec![],
                merge_deferred: false,
            })
        }
    }

    async fn retry_all_incomplete(
        &self,
        _user_id: UserId,
    ) -> Result<RetrySummary, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn upload_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn download_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn search_works(
        &self,
        _user_id: UserId,
        _query: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        unimplemented!("not exercised")
    }

    fn try_start_bulk_refresh(&self, _user_id: i64) -> Option<BulkRefreshGuard> {
        unimplemented!("not exercised")
    }

    async fn converge_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _threshold: u32,
    ) -> Result<ConvergeOutcome, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn preview_merge_works(
        &self,
        _user_id: UserId,
        _survivor_id: WorkId,
        _loser_id: WorkId,
    ) -> Result<MergePreview, WorkServiceError> {
        unimplemented!("not exercised")
    }

    async fn merge_works(
        &self,
        _user_id: UserId,
        _survivor_id: WorkId,
        _loser_id: WorkId,
        _choices: Vec<MergeFieldChoiceEntry>,
    ) -> Result<MergeWorksResult, WorkServiceError> {
        unimplemented!("not exercised")
    }
}

#[tokio::test]
async fn bulk_sweep_is_bounded_concurrent_and_isolates_failures() {
    let user_id = 42;
    let works: Vec<Work> = (1..=8)
        .map(|id| Work {
            id,
            user_id,
            ..Default::default()
        })
        .collect();
    let expected_ids: Vec<WorkId> = works.iter().map(|work| work.id).collect();
    let stub = BulkRefreshStub::new(expected_ids[3]);

    let (enriched, failed) =
        livrarr_handlers::work::bulk_refresh_sweep(&stub, user_id, works).await;

    assert_eq!(
        (enriched, failed),
        (7, 1),
        "AC-019: one per-work refresh failure must be counted without aborting the sweep"
    );

    let mut calls = stub.calls();
    calls.sort_unstable();

    let mut expected_counts = HashMap::new();
    for id in expected_ids {
        *expected_counts.entry(id).or_insert(0usize) += 1;
    }

    let mut actual_counts = HashMap::new();
    for id in calls {
        *actual_counts.entry(id).or_insert(0usize) += 1;
    }

    assert_eq!(
        actual_counts, expected_counts,
        "AC-019: bulk sweep must refresh every seeded work exactly once despite isolated failures"
    );

    let max_in_flight = stub.max_in_flight();
    assert!(
        max_in_flight >= 2,
        "AC-019: bulk sweep must be genuinely concurrent; observed max in-flight was {max_in_flight}"
    );
    assert!(
        max_in_flight <= 3,
        "AC-019: bulk sweep concurrency must be bounded at 3; observed max in-flight was {max_in_flight}"
    );
}
