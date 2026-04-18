use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateWorkDbRequest, UpdateWorkUserFieldsDbRequest, WorkDb,
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
    D: WorkDb + AuthorDb + Send + Sync,
    E: EnrichmentWorkflow + Send + Sync,
{
    async fn add(&self, user_id: UserId, req: AddWorkRequest) -> Result<Work, WorkServiceError> {
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

        let author_name = req.author_name.as_deref().unwrap_or("").trim().to_string();
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
                            ol_key: None,
                            gr_key: None,
                            hc_key: None,
                            import_id: None,
                        })
                        .await
                        .map_err(WorkServiceError::Db)?;
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
                ..Default::default()
            })
            .await
            .map_err(WorkServiceError::Db)?;

        // Enrichment is best-effort — failure doesn't fail the add
        let _ = self
            .enrichment
            .enrich_work(user_id, work.id, EnrichmentMode::Manual)
            .await;

        // Re-read work to pick up any enrichment changes
        match self.db.get_work(user_id, work.id).await {
            Ok(enriched) => Ok(enriched),
            Err(_) => Ok(work),
        }
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
            title: req.title,
            author_name: req.author_name,
            monitor_ebook: req.monitored,
            ..Default::default()
        };

        self.db
            .update_work_user_fields(user_id, work_id, db_req)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })
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

    async fn refresh(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError> {
        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
            })?;

        // Per-work lock — concurrent refreshes wait, not reject
        let lock = {
            let mut locks = self.refresh_locks.lock().await;
            locks
                .entry((user_id, work_id))
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        self.enrichment
            .enrich_work(user_id, work_id, EnrichmentMode::HardRefresh)
            .await
            .map_err(|e| WorkServiceError::Enrichment(e.to_string()))?;

        self.db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => WorkServiceError::NotFound,
                other => WorkServiceError::Db(other),
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
