use livrarr_db::{
    AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateWorkDbRequest, EnrichmentRetryDb,
    LibraryItemDb, ProvenanceDb, SetFieldProvenanceRequest, UpdateWorkUserFieldsDbRequest, WorkDb,
};
use livrarr_domain::keyed_mutex::KeyedMutex;
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct StubNoLlm;

impl LlmCaller for StubNoLlm {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        Err(LlmError::NotConfigured)
    }
}

struct CachedLookup {
    filtered: Vec<LookupResult>,
    raw: Vec<LookupResult>,
    raw_available: bool,
    created_at: Instant,
}

pub struct WorkServiceImpl<D, E, H, L = StubNoLlm> {
    db: D,
    enrichment: E,
    http: H,
    llm: L,
    hc_client: livrarr_http::HttpClient,
    data_dir: PathBuf,
    refresh_locks: KeyedMutex<(UserId, WorkId)>,
    bulk_refresh_users: Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
    lookup_cache: Arc<std::sync::Mutex<HashMap<(String, String), CachedLookup>>>,
}

impl<D, E, H> WorkServiceImpl<D, E, H, StubNoLlm> {
    pub fn new(db: D, enrichment: E, http: H, data_dir: PathBuf) -> Self {
        Self {
            db,
            enrichment,
            http,
            llm: StubNoLlm,
            hc_client: livrarr_http::HttpClient::builder()
                .build()
                .expect("HC HttpClient"),
            data_dir,
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }
}

impl<D, E, H, L> WorkServiceImpl<D, E, H, L> {
    pub fn new_with_llm(db: D, enrichment: E, http: H, llm: L, data_dir: PathBuf) -> Self {
        Self {
            db,
            enrichment,
            http,
            llm,
            hc_client: livrarr_http::HttpClient::builder()
                .build()
                .expect("HC HttpClient"),
            data_dir,
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }
}

impl<D, H> WorkServiceImpl<D, (), H> {
    pub fn without_enrichment(
        db: D,
        http: H,
        data_dir: PathBuf,
    ) -> WorkServiceImpl<D, StubNoEnrichment, H, StubNoLlm> {
        WorkServiceImpl {
            db,
            enrichment: StubNoEnrichment,
            http,
            llm: StubNoLlm,
            hc_client: livrarr_http::HttpClient::builder()
                .build()
                .expect("HC HttpClient"),
            data_dir,
            refresh_locks: KeyedMutex::new(),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            lookup_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
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

impl<D, E, H, L> WorkService for WorkServiceImpl<D, E, H, L>
where
    D: WorkDb
        + AuthorDb
        + LibraryItemDb
        + ProvenanceDb
        + EnrichmentRetryDb
        + ConfigDb
        + Send
        + Sync,
    E: EnrichmentWorkflow + Send + Sync,
    H: HttpFetcher + Clone + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
{
    async fn add(
        &self,
        user_id: UserId,
        req: AddWorkRequest,
    ) -> Result<AddWorkResult, WorkServiceError> {
        let cleaned_title = crate::title_cleanup::clean_title(&req.title);
        if cleaned_title.is_empty() {
            return Err(WorkServiceError::Enrichment(
                "title must not be empty".into(),
            ));
        }
        let cleaned_author = crate::title_cleanup::clean_author(&req.author_name);

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

        let mut author_created = false;
        let author_id = if !cleaned_author.is_empty() {
            let normalized = cleaned_author.to_lowercase();
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
                            name: cleaned_author.clone(),
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

        let cover_url = req.cover_url.clone();

        let work = self
            .db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: cleaned_title,
                author_name: cleaned_author,
                author_id,
                ol_key: req.ol_key,
                gr_key: req.gr_key,
                year: req.year,
                cover_url: req.cover_url,
                metadata_source: req.metadata_source,
                detail_url: req.detail_url,
                language: livrarr_domain::normalize_language_opt(req.language.as_deref()),
                series_name: req.series_name,
                series_position: req.series_position,
                monitor_ebook: true,
                monitor_audiobook: true,
                ..Default::default()
            })
            .await
            .map_err(WorkServiceError::Db)?;

        let setter = req.provenance_setter.unwrap_or(ProvenanceSetter::User);
        write_addtime_provenance(&self.db, user_id, &work, setter).await;

        let _is_foreign = crate::language::is_foreign_source(work.metadata_source.as_deref());
        let cover_url = cover_url.map(|u| unproxy_cover_url(&u));

        if req.defer_enrichment {
            let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
            let hc_token = self
                .db
                .get_metadata_config()
                .await
                .ok()
                .and_then(|c| c.hardcover_api_token);

            let cover_mtime = crate::cover::fetch_phase1_cover(
                &self.http,
                &self.hc_client,
                &work.title,
                &work.author_name,
                cover_url.as_deref(),
                hc_token.as_deref(),
                &covers_dir,
                work.id,
            )
            .await;

            // Delete stale thumbnail if cover was written
            if cover_mtime.is_some() {
                let thumb = covers_dir.join(format!("{}_thumb.jpg", work.id));
                let _ = tokio::fs::remove_file(&thumb).await;
            }

            return Ok(AddWorkResult {
                work,
                author_created,
                author_id,
                messages: vec![],
                cover_mtime,
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

        if let Some(ref cover_url) = enriched_work.cover_url {
            let covers_dir = self.data_dir.join("covers");
            let work_id = enriched_work.id;
            if let Err(e) =
                download_cover_to_disk(&self.http, cover_url, &covers_dir, work_id, "").await
            {
                tracing::warn!(work_id, "cover download failed: {e}");
            }
            let thumb = covers_dir.join(format!("{work_id}_thumb.jpg"));
            let _ = tokio::fs::remove_file(&thumb).await;
        }

        let covers_dir_for_mtime = self.data_dir.join("covers").join(user_id.to_string());
        let cover_mtime = crate::cover::cover_file_mtime(&covers_dir_for_mtime, enriched_work.id)
            .or_else(|| {
                crate::cover::cover_file_mtime(&self.data_dir.join("covers"), enriched_work.id)
            });

        Ok(AddWorkResult {
            work: enriched_work,
            author_created,
            author_id,
            messages,
            cover_mtime,
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
        let mut works = if let Some(author_id) = filter.author_id {
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

        if let Some(monitored) = filter.monitored {
            works.retain(|w| (w.monitor_ebook || w.monitor_audiobook) == monitored);
        }
        if let Some(ref status) = filter.enrichment_status {
            works.retain(|w| w.enrichment_status == *status);
        }
        if let Some(media_type) = filter.media_type {
            works.retain(|w| match media_type {
                MediaType::Ebook => w.monitor_ebook,
                MediaType::Audiobook => w.monitor_audiobook,
            });
        }
        if let Some(sort_by) = filter.sort_by {
            let dir = filter.sort_dir.unwrap_or(SortDirection::Asc);
            works.sort_by(|a, b| {
                let cmp = match sort_by {
                    WorkSortField::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                    WorkSortField::DateAdded => a.added_at.cmp(&b.added_at),
                    WorkSortField::Year => a.year.cmp(&b.year),
                    WorkSortField::Author => a.author_name.cmp(&b.author_name),
                };
                match dir {
                    SortDirection::Asc => cmp,
                    SortDirection::Desc => cmp.reverse(),
                }
            });
        }

        Ok(works)
    }

    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
        sort_by: WorkSortField,
        sort_dir: SortDirection,
    ) -> Result<PaginatedWorksView, WorkServiceError> {
        let sort_col = match sort_by {
            WorkSortField::Title => "title",
            WorkSortField::DateAdded => "date_added",
            WorkSortField::Year => "year",
            WorkSortField::Author => "author",
        };
        let dir = match sort_dir {
            SortDirection::Asc => "asc",
            SortDirection::Desc => "desc",
        };
        let (works, total) = self
            .db
            .list_works_paginated(user_id, page, page_size, sort_col, dir)
            .await
            .map_err(WorkServiceError::Db)?;

        let work_ids: Vec<i64> = works.iter().map(|w| w.id).collect();
        let items = self
            .db
            .list_library_items_by_work_ids(user_id, &work_ids)
            .await
            .map_err(WorkServiceError::Db)?;

        // Pre-index items by work_id to avoid O(works×items) filtering.
        let mut items_by_work: HashMap<WorkId, Vec<LibraryItem>> =
            HashMap::with_capacity(work_ids.len());
        for item in items {
            items_by_work.entry(item.work_id).or_default().push(item);
        }

        let work_views = works
            .into_iter()
            .map(|w| {
                let work_items = items_by_work.remove(&w.id).unwrap_or_default();
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

        let has_title = req.title.is_some();
        let has_author = req.author_name.is_some();
        let series_name_cleared = matches!(req.series_name, Some(None));
        let series_position_cleared = matches!(req.series_position, Some(None));
        let has_series_name = req.series_name.is_some();
        let has_series_position = req.series_position.is_some();
        let db_req = UpdateWorkUserFieldsDbRequest {
            title: req.title.map(|t| crate::title_cleanup::clean_title(&t)),
            author_name: req
                .author_name
                .map(|a| crate::title_cleanup::clean_author(&a)),
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

        let mut prov_reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
        if has_title {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::Title,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if has_author {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::AuthorName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: false,
            });
        }
        if has_series_name {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::SeriesName,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: series_name_cleared,
            });
        }
        if has_series_position {
            prov_reqs.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field: WorkField::SeriesPosition,
                source: None,
                setter: ProvenanceSetter::User,
                cleared: series_position_cleared,
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

        let _guard = self.refresh_locks.lock((user_id, work_id)).await;

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

        if !self.try_start_bulk_refresh(user_id) {
            return Err(WorkServiceError::Enrichment(
                "bulk refresh already in progress".into(),
            ));
        }

        Ok(RefreshAllHandle { total_works })
    }

    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        bytes: &[u8],
    ) -> Result<(), WorkServiceError> {
        const MAX_COVER_BYTES: usize = 1_024 * 1_024;

        if bytes.len() > MAX_COVER_BYTES {
            return Err(WorkServiceError::Enrichment(format!(
                "cover too large: {} bytes (max {})",
                bytes.len(),
                MAX_COVER_BYTES
            )));
        }
        if bytes.is_empty() {
            return Err(WorkServiceError::Enrichment("empty image data".into()));
        }

        let _work = self.get(user_id, work_id).await?;

        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        tokio::fs::create_dir_all(&covers_dir)
            .await
            .map_err(|e| WorkServiceError::Enrichment(format!("create covers dir: {e}")))?;

        let cover_path = covers_dir.join(format!("{work_id}.jpg"));
        let tmp_path = cover_path.with_extension("jpg.tmp");
        let tmp_clone = tmp_path.clone();
        let target = cover_path.clone();
        let bytes_vec = bytes.to_vec();
        let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_clone)?;
            f.write_all(&bytes_vec)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp_clone, &target)
        })
        .await
        .map_err(|e| WorkServiceError::Enrichment(format!("spawn error: {e}")))?;

        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(WorkServiceError::Enrichment(format!("write cover: {e}")));
        }

        let thumb_path = covers_dir.join(format!("{work_id}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb_path).await;

        self.db
            .set_cover_manual(user_id, work_id, true)
            .await
            .map_err(WorkServiceError::Db)?;

        Ok(())
    }

    async fn download_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError> {
        let _work = self.get(user_id, work_id).await?;

        // Try new tenant-aware path first, fall back to old flat layout.
        let new_path = self
            .data_dir
            .join("covers")
            .join(user_id.to_string())
            .join(format!("{work_id}.jpg"));
        let cover_path = if new_path.exists() {
            new_path
        } else {
            self.data_dir.join("covers").join(format!("{work_id}.jpg"))
        };
        let bytes = tokio::fs::read(&cover_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                WorkServiceError::NotFound
            } else {
                WorkServiceError::Enrichment(format!("read cover: {e}"))
            }
        })?;
        Ok(bytes)
    }

    async fn lookup(&self, req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError> {
        let term = req.term.trim().to_string();
        if term.is_empty() {
            return Ok(vec![]);
        }

        let cfg = self.db.get_metadata_config().await.ok();
        let default_lang = cfg
            .as_ref()
            .and_then(|c| c.languages.first().cloned())
            .unwrap_or_else(|| "en".to_string());
        let lang = req.lang_override.as_deref().unwrap_or(&default_lang);

        if lang != "en" && !crate::language::is_supported_language(lang) {
            return Err(WorkServiceError::Enrichment(format!(
                "unsupported language: {lang}"
            )));
        }

        // Non-English: Goodreads search with regex HTML parsing.
        if lang != "en" {
            return self.lookup_goodreads(&term, lang).await;
        }

        // English: OpenLibrary search.
        let results = self.lookup_openlibrary(&term).await?;
        if !results.is_empty() {
            return Ok(results);
        }

        Ok(vec![])
    }

    async fn lookup_filtered(
        &self,
        req: LookupRequest,
        raw: bool,
    ) -> Result<LookupResponse, WorkServiceError> {
        let term = req.term.trim().to_lowercase();
        if term.is_empty() {
            return Ok(LookupResponse {
                results: vec![],
                filtered_count: 0,
                raw_count: 0,
                raw_available: false,
            });
        }

        let lang = req
            .lang_override
            .clone()
            .unwrap_or_else(|| "en".to_string());
        let cache_key = (term.clone(), lang.clone());

        // Check cache (15 min TTL)
        {
            let cache = self.lookup_cache.lock().unwrap();
            if let Some(cached) = cache.get(&cache_key) {
                if cached.created_at.elapsed() < Duration::from_secs(900) {
                    let results = if raw || !cached.raw_available {
                        cached.raw.clone()
                    } else {
                        cached.filtered.clone()
                    };
                    return Ok(LookupResponse {
                        filtered_count: cached.filtered.len(),
                        raw_count: cached.raw.len(),
                        raw_available: cached.raw_available,
                        results,
                    });
                }
            }
        }

        let raw_results = self.lookup(req).await?;
        if raw_results.is_empty() {
            return Ok(LookupResponse {
                results: vec![],
                filtered_count: 0,
                raw_count: 0,
                raw_available: false,
            });
        }

        let raw_count = raw_results.len();

        // Attempt LLM filtering
        let (filtered, raw_available) = match self.llm_filter_search(&raw_results).await {
            Some(indices) if indices.len() < raw_count => {
                let filtered: Vec<LookupResult> = indices
                    .into_iter()
                    .filter_map(|i| raw_results.get(i).cloned())
                    .collect();
                (filtered, true)
            }
            _ => (raw_results.clone(), false),
        };

        let filtered_count = filtered.len();

        // Cache both
        {
            let mut cache = self.lookup_cache.lock().unwrap();
            // Evict stale entries
            cache.retain(|_, v| v.created_at.elapsed() < Duration::from_secs(900));
            cache.insert(
                cache_key,
                CachedLookup {
                    filtered: filtered.clone(),
                    raw: raw_results.clone(),
                    raw_available,
                    created_at: Instant::now(),
                },
            );
        }

        let results = if raw || !raw_available {
            raw_results
        } else {
            filtered
        };

        Ok(LookupResponse {
            results,
            filtered_count,
            raw_count,
            raw_available,
        })
    }

    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError> {
        WorkDb::search_works(&self.db, user_id, query, page, page_size)
            .await
            .map_err(WorkServiceError::Db)
    }

    async fn download_cover_from_url(
        &self,
        user_id: i64,
        work_id: i64,
        cover_url: &str,
    ) -> Result<(), WorkServiceError> {
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        download_cover_to_disk(&self.http, cover_url, &covers_dir, work_id, "")
            .await
            .map_err(|e| WorkServiceError::Cover(e.to_string()))?;
        let thumb = covers_dir.join(format!("{work_id}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb).await;
        Ok(())
    }

    fn try_start_bulk_refresh(&self, user_id: i64) -> bool {
        let mut guard = self.bulk_refresh_users.lock().unwrap();
        guard.insert(user_id)
    }

    fn finish_bulk_refresh(&self, user_id: i64) {
        let mut guard = self.bulk_refresh_users.lock().unwrap();
        guard.remove(&user_id);
    }
}

impl<D, E, H, L> WorkServiceImpl<D, E, H, L>
where
    D: WorkDb + ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    async fn llm_filter_search(&self, results: &[LookupResult]) -> Option<Vec<usize>> {
        let mut listing = String::new();
        for (i, r) in results.iter().enumerate() {
            listing.push_str(&format!(
                "{}: \"{}\" by {} ({})\n",
                i,
                r.title,
                r.author_name,
                r.year.map(|y| y.to_string()).unwrap_or_default(),
            ));
        }

        let system = "You are a librarian assistant. Clean up book search results.";
        let user_prompt = format!(
            "These are search results from a book database:\n\n\
             {listing}\n\
             Clean up this list:\n\
             1. Remove non-book items (study guides, journals, blank notebooks, merchandise, board games)\n\
             2. Remove duplicate editions of the same work — keep the one with the best metadata\n\
             3. Remove comic/manga adaptations, movie tie-in editions, and abridged versions\n\
             4. Remove anthologies and compilations unless they are a well-known standalone work\n\
             5. Keep results that are legitimate different works even if titles are similar\n\n\
             Return a JSON array of the original indices to keep, e.g. [0, 2, 5].\n\
             Return ONLY the JSON array, no other text."
        );

        let mut context = HashMap::new();
        context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing));

        let req = LlmCallRequest {
            system_template: Box::leak(system.to_string().into_boxed_str()),
            user_template: Box::leak(user_prompt.into_boxed_str()),
            context,
            allowed_fields: &[LlmField::BibliographyHtml],
            timeout: Duration::from_secs(30),
            purpose: LlmPurpose::SearchResultCleanup,
        };

        let resp = self.llm.call(req).await.ok()?;

        let json_str = resp
            .content
            .trim()
            .strip_prefix("```json")
            .or_else(|| resp.content.trim().strip_prefix("```"))
            .unwrap_or(resp.content.trim())
            .strip_suffix("```")
            .unwrap_or(resp.content.trim())
            .trim();

        let indices: Vec<usize> = serde_json::from_str(json_str).ok()?;
        let max_idx = results.len();
        let valid: Vec<usize> = indices.into_iter().filter(|&i| i < max_idx).collect();

        if valid.is_empty() {
            return None;
        }

        Some(valid)
    }

    async fn lookup_goodreads(
        &self,
        term: &str,
        lang: &str,
    ) -> Result<Vec<LookupResult>, WorkServiceError> {
        let search_url = format!(
            "https://www.goodreads.com/search?q={}",
            urlencoding::encode(term)
        );

        let fetch_req = FetchRequest {
            url: search_url,
            method: HttpMethod::Get,
            headers: vec![("Accept-Language".into(), "en-US,en;q=0.9".into())],
            body: None,
            timeout: std::time::Duration::from_secs(10),
            rate_bucket: RateBucket::Goodreads,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: true,
            user_agent: UserAgentProfile::Browser,
        };

        let resp = match self.http.fetch(fetch_req).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Goodreads search fetch failed: {e}");
                return Ok(vec![]);
            }
        };

        if resp.status >= 400 {
            tracing::warn!(
                status = resp.status,
                "Goodreads search returned non-success"
            );
            return Ok(vec![]);
        }

        let raw_html = String::from_utf8_lossy(&resp.body);

        if crate::llm_scraper::is_anti_bot_page(&raw_html) {
            tracing::warn!("Goodreads search: anti-bot page detected");
            return Ok(vec![]);
        }

        let parsed = crate::goodreads::parse_search_html(&raw_html);

        if parsed.is_empty() && raw_html.contains("itemtype=\"http") {
            tracing::warn!(
                "Goodreads parser drift: HTML contains schema.org Book rows but 0 passed \
                 validation. HTML structure may have changed."
            );
        }

        let lang_owned = lang.to_string();
        let results = parsed
            .into_iter()
            .map(|r| {
                let full_url = if r.detail_url.starts_with('/') {
                    format!("https://www.goodreads.com{}", r.detail_url)
                } else {
                    r.detail_url.clone()
                };
                let validated_url = if crate::goodreads::validate_detail_url(&full_url) {
                    Some(full_url)
                } else {
                    None
                };
                LookupResult {
                    ol_key: None,
                    title: r.title,
                    author_name: r.author.unwrap_or_default(),
                    author_ol_key: None,
                    year: r.year,
                    cover_url: r.cover_url,
                    description: None,
                    series_name: r.series_name,
                    series_position: r.series_position,
                    source: Some("Goodreads".to_string()),
                    source_type: Some("goodreads".to_string()),
                    language: Some(lang_owned.clone()),
                    detail_url: validated_url,
                    rating: r.rating,
                }
            })
            .collect();

        Ok(results)
    }

    async fn lookup_openlibrary(&self, term: &str) -> Result<Vec<LookupResult>, WorkServiceError> {
        let url = format!(
            "https://openlibrary.org/search.json?q={}&limit=50&fields=key,title,author_name,author_key,first_publish_year,cover_i",
            urlencoding::encode(term)
        );

        let fetch_req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: std::time::Duration::from_secs(10),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
        };

        let resp = match self.http.fetch(fetch_req).await {
            Ok(r) => r,
            Err(e) => {
                return Err(WorkServiceError::Enrichment(format!(
                    "OpenLibrary request failed: {e}"
                )));
            }
        };

        if resp.status >= 400 {
            return Err(WorkServiceError::Enrichment(format!(
                "OpenLibrary returned {}",
                resp.status
            )));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| WorkServiceError::Enrichment(format!("OpenLibrary parse error: {e}")))?;

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let results = docs
            .iter()
            .filter_map(|doc| {
                let key = doc.get("key")?.as_str()?;
                let title = doc.get("title")?.as_str()?;
                let ol_key = key.trim_start_matches("/works/").to_string();

                let author_name = doc
                    .get("author_name")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

                let author_ol_key = doc
                    .get("author_key")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .map(|k| k.trim_start_matches("/authors/").to_string());

                let year = doc
                    .get("first_publish_year")
                    .and_then(|y| y.as_i64())
                    .map(|y| y as i32);

                let cover_url = doc
                    .get("cover_i")
                    .and_then(|c| c.as_i64())
                    .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-M.jpg"));

                Some(LookupResult {
                    ol_key: Some(ol_key),
                    title: title.to_string(),
                    author_name,
                    author_ol_key,
                    year,
                    cover_url,
                    description: None,
                    series_name: None,
                    series_position: None,
                    source: None,
                    source_type: None,
                    language: Some("en".to_string()),
                    detail_url: None,
                    rating: None,
                })
            })
            .collect();

        Ok(results)
    }
}

async fn write_addtime_provenance<D: ProvenanceDb>(
    db: &D,
    user_id: i64,
    work: &Work,
    setter: ProvenanceSetter,
) {
    crate::provenance::write_addtime_provenance(db, user_id, work, setter).await;
}

pub fn unproxy_cover_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("/api/v1/coverproxy?url=") {
        urlencoding::decode(rest)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| url.to_string())
    } else {
        url.to_string()
    }
}

pub async fn download_cover_to_disk<H: HttpFetcher>(
    http: &H,
    url: &str,
    covers_dir: &std::path::Path,
    work_id: i64,
    suffix: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::fs::create_dir_all(covers_dir).await?;

    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::None,
        max_body_bytes: 10 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
    };

    let resp = http
        .fetch_ssrf_safe(req)
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    if resp.status >= 400 {
        return Err(format!("cover download returned {}", resp.status).into());
    }

    let cover_path = covers_dir.join(format!("{work_id}{suffix}.jpg"));
    let tmp_path = cover_path.with_extension("jpg.tmp");
    let tmp_clone = tmp_path.clone();
    let target = cover_path.clone();
    let bytes = resp.body;
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_clone)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_clone, &target)
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(Box::new(e))
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(format!("spawn error: {e}").into())
        }
    }
}
