use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateWorkDbRequest, EnrichmentRetryDb, LibraryItemDb,
    ProvenanceDb, SetFieldProvenanceRequest, UpdateWorkUserFieldsDbRequest, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct WorkServiceImpl<D, E> {
    db: D,
    enrichment: E,
    refresh_locks: Arc<Mutex<HashMap<(UserId, WorkId), Arc<Mutex<()>>>>>,
}

impl<D, E> WorkServiceImpl<D, E> {
    pub fn new(db: D, enrichment: E) -> Self {
        Self {
            db,
            enrichment,
            refresh_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<D> WorkServiceImpl<D, ()> {
    pub fn without_enrichment(db: D) -> WorkServiceImpl<D, StubNoEnrichment> {
        WorkServiceImpl {
            db,
            enrichment: StubNoEnrichment,
            refresh_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub struct StubNoEnrichment;

impl EnrichmentWorkflow for StubNoEnrichment {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Pending,
            enrichment_source: None,
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
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

impl<D, E> WorkService for WorkServiceImpl<D, E>
where
    D: WorkDb + AuthorDb + LibraryItemDb + ProvenanceDb + EnrichmentRetryDb + Send + Sync,
    E: EnrichmentWorkflow + Send + Sync,
{
    async fn add(
        &self,
        user_id: UserId,
        req: AddWorkRequest,
    ) -> Result<AddWorkResult, WorkServiceError> {
        let title = req.title.trim().to_string();
        if title.is_empty() {
            return Err(WorkServiceError::Enrichment(
                "title must not be empty".into(),
            ));
        }

        if let Some(ref ol_key) = req.ol_key {
            if self
                .db
                .work_exists_by_ol_key(user_id, ol_key)
                .await
                .map_err(WorkServiceError::Db)?
            {
                return Err(WorkServiceError::AlreadyExists);
            }
        }

        let author_name = req.author_name.trim().to_string();
        let mut author_created = false;
        let author_id = if !author_name.is_empty() {
            let normalized = author_name.to_lowercase();
            match self
                .db
                .find_author_by_name(user_id, &normalized)
                .await
                .map_err(WorkServiceError::Db)?
            {
                Some(existing) => Some(existing.id),
                None => {
                    let author = self
                        .db
                        .create_author(CreateAuthorDbRequest {
                            user_id,
                            name: author_name.clone(),
                            sort_name: None,
                            ol_key: req.author_ol_key,
                            gr_key: None,
                            hc_key: None,
                            import_id: None,
                        })
                        .await
                        .map_err(WorkServiceError::Db)?;
                    author_created = true;
                    Some(author.id)
                }
            }
        } else {
            None
        };

        let work = self
            .db
            .create_work(CreateWorkDbRequest {
                user_id,
                title,
                author_name,
                author_id,
                ol_key: req.ol_key,
                gr_key: req.gr_key,
                year: req.year,
                cover_url: req.cover_url,
                metadata_source: req.metadata_source,
                detail_url: req.detail_url,
                language: req.language,
                series_name: req.series_name,
                series_position: req.series_position,
                ..Default::default()
            })
            .await
            .map_err(WorkServiceError::Db)?;

        write_addtime_provenance(&self.db, user_id, &work, ProvenanceSetter::User).await;

        if req.defer_enrichment {
            return Ok(AddWorkResult {
                work,
                author_created,
                messages: vec![],
            });
        }

        let messages = match self
            .enrichment
            .enrich_work(user_id, work.id, EnrichmentMode::Background)
            .await
        {
            Ok(result) => result
                .provider_outcomes
                .iter()
                .filter(|(_, oc)| !matches!(oc, OutcomeClass::Success | OutcomeClass::NotFound))
                .map(|(p, oc)| format!("{p:?}: {oc:?}"))
                .collect(),
            Err(e) => {
                tracing::warn!(work_id = work.id, "add_work: enrichment failed: {e}");
                vec![format!("enrichment failed: {e}")]
            }
        };

        let enriched_work = match self.db.get_work(user_id, work.id).await {
            Ok(w) => w,
            Err(_) => work,
        };

        Ok(AddWorkResult {
            work: enriched_work,
            author_created,
            messages,
        })
    }

    async fn get(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })
    }

    async fn get_detail(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError> {
        let work = self.get(user_id, work_id).await?;
        let library_items = self
            .db
            .list_library_items_by_work(user_id, work_id)
            .await
            .map_err(WorkServiceError::Db)?;
        Ok(WorkDetailView {
            work,
            library_items,
        })
    }

    async fn list(
        &self,
        user_id: UserId,
        filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError> {
        let works = if let Some(author_id) = filter.author_id {
            self.db
                .list_works_by_author(user_id, author_id)
                .await
                .map_err(WorkServiceError::Db)?
        } else {
            self.db
                .list_works(user_id)
                .await
                .map_err(WorkServiceError::Db)?
        };
        Ok(works)
    }

    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        let (works, total) = self
            .db
            .list_works_paginated(user_id, page, page_size)
            .await
            .map_err(WorkServiceError::Db)?;

        let work_ids: Vec<i64> = works.iter().map(|w| w.id).collect();
        let items = self
            .db
            .list_library_items_by_work_ids(user_id, &work_ids)
            .await
            .map_err(WorkServiceError::Db)?;

        let work_views = works
            .into_iter()
            .map(|w| {
                let work_items: Vec<LibraryItem> = items
                    .iter()
                    .filter(|li| li.work_id == w.id)
                    .cloned()
                    .collect();
                WorkDetailView {
                    work: w,
                    library_items: work_items,
                }
            })
            .collect();

        Ok(PaginatedWorksView {
            works: work_views,
            total,
            page,
            page_size,
        })
    }

    async fn update(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        let db_req = UpdateWorkUserFieldsDbRequest {
            title: req.title.clone(),
            author_name: req.author_name.clone(),
            series_name: req.series_name,
            series_position: req.series_position,
            monitor_ebook: req.monitor_ebook,
            monitor_audiobook: req.monitor_audiobook,
        };

        let work = self
            .db
            .update_work_user_fields(user_id, work_id, db_req)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        // Write provenance for edited fields (re-lock as setter=User).
        let mut prov_reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
        if req.title.is_some() {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::Title,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if req.author_name.is_some() {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::AuthorName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if !prov_reqs.is_empty() {
            if let Err(e) = self.db.set_field_provenance_batch(prov_reqs).await {
                tracing::warn!(work_id, "user-edit provenance write failed: {e}");
            }
        }

        Ok(work)
    }

    async fn delete(&self, user_id: UserId, work_id: WorkId) -> Result<(), WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        self.db
            .delete_work(user_id, work_id)
            .await
            .map(|_| ())
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })
    }

    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<RefreshWorkResult, WorkServiceError> {
        let work = self.get(user_id, work_id).await?;

        let lock = {
            let mut locks = self.refresh_locks.lock().await;
            locks
                .entry((user_id, work_id))
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        if let Err(e) = self.db.reset_enrichment_for_refresh(user_id, work_id).await {
            tracing::warn!("reset_enrichment_for_refresh failed: {e}");
        }

        if let Err(e) = self
            .enrichment
            .reset_for_manual_refresh(user_id, work_id)
            .await
        {
            tracing::warn!("enrichment reset_for_manual_refresh failed: {e}");
        }

        let (enriched_work, messages, merge_deferred) = match self
            .enrichment
            .enrich_work(user_id, work_id, EnrichmentMode::HardRefresh)
            .await
        {
            Ok(result) => {
                let msgs: Vec<String> = result
                    .provider_outcomes
                    .iter()
                    .filter(|(_, oc)| !matches!(oc, OutcomeClass::Success | OutcomeClass::NotFound))
                    .map(|(p, oc)| format!("{p:?}: {oc:?}"))
                    .collect();
                let w = match self.db.get_work(user_id, work_id).await {
                    Ok(w) => w,
                    Err(_) => result.work,
                };
                (w, msgs, result.merge_deferred)
            }
            Err(e) => {
                tracing::warn!(work_id, "enrichment failed: {e}");
                (work, vec![format!("enrichment failed: {e}")], false)
            }
        };

        let taggable_items = self
            .db
            .list_taggable_items_by_work(user_id, work_id)
            .await
            .unwrap_or_default();

        Ok(RefreshWorkResult {
            work: enriched_work,
            messages,
            taggable_items,
            merge_deferred,
        })
    }

    async fn refresh_all(&self, user_id: UserId) -> Result<RefreshAllHandle, WorkServiceError> {
        let works = self
            .db
            .list_works(user_id)
            .await
            .map_err(WorkServiceError::Db)?;

        let total_works = works.len();

        Ok(RefreshAllHandle { total_works })
    }

    async fn upload_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        todo!("upload_cover requires filesystem/cover cache integration")
    }

    async fn download_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        todo!("download_cover requires filesystem/cover cache integration")
    }
}

async fn write_addtime_provenance<D: ProvenanceDb>(
    db: &D,
    user_id: i64,
    work: &Work,
    setter: ProvenanceSetter,
) {
    let mut reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
    let push = |reqs: &mut Vec<SetFieldProvenanceRequest>, field: WorkField| {
        reqs.push(SetFieldProvenanceRequest {
            user_id,
            work_id: work.id,
            field,
            source: None,
            setter,
            cleared: false,
        });
    };
    if !work.title.is_empty() {
        push(&mut reqs, WorkField::Title);
    }
    if !work.author_name.is_empty() {
        push(&mut reqs, WorkField::AuthorName);
    }
    if work.ol_key.is_some() {
        push(&mut reqs, WorkField::OlKey);
    }
    if work.gr_key.is_some() {
        push(&mut reqs, WorkField::GrKey);
    }
    if work.language.is_some() {
        push(&mut reqs, WorkField::Language);
    }
    if work.year.is_some() {
        push(&mut reqs, WorkField::Year);
    }
    if work.series_name.is_some() {
        push(&mut reqs, WorkField::SeriesName);
    }
    if work.series_position.is_some() {
        push(&mut reqs, WorkField::SeriesPosition);
    }
    if reqs.is_empty() {
        return;
    }
    if let Err(e) = db.set_field_provenance_batch(reqs).await {
        tracing::warn!(
            work_id = work.id,
            ?setter,
            "write_addtime_provenance failed: {e}"
        );
    }
}
