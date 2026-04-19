use livrarr_db::{
    AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateWorkDbRequest, EnrichmentRetryDb,
    LibraryItemDb, ProvenanceDb, SetFieldProvenanceRequest, UpdateWorkUserFieldsDbRequest, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

type RefreshLockMap = Arc<Mutex<HashMap<(UserId, WorkId), Arc<Mutex<()>>>>>;

pub struct WorkServiceImpl<D, E, H> {
    db: D,
    enrichment: E,
    http: H,
    data_dir: PathBuf,
    refresh_locks: RefreshLockMap,
    bulk_refresh_users: Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
}

impl<D, E, H> WorkServiceImpl<D, E, H> {
    pub fn new(db: D, enrichment: E, http: H, data_dir: PathBuf) -> Self {
        Self {
            db,
            enrichment,
            http,
            data_dir,
            refresh_locks: Arc::new(Mutex::new(HashMap::new())),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }
}

impl<D, H> WorkServiceImpl<D, (), H> {
    pub fn without_enrichment(
        db: D,
        http: H,
        data_dir: PathBuf,
    ) -> WorkServiceImpl<D, StubNoEnrichment, H> {
        WorkServiceImpl {
            db,
            enrichment: StubNoEnrichment,
            http,
            data_dir,
            refresh_locks: Arc::new(Mutex::new(HashMap::new())),
            bulk_refresh_users: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
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

impl<D, E, H> WorkService for WorkServiceImpl<D, E, H>
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
                language: req.language,
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

        let is_foreign = crate::language::is_foreign_source(work.metadata_source.as_deref());
        let cover_url = cover_url.map(|u| unproxy_cover_url(&u));

        if req.defer_enrichment {
            // Download cover now since enrichment won't run to provide a better one.
            if let Some(ref url) = cover_url {
                if crate::llm_scraper::validate_cover_url(url, "").is_some() {
                    let covers_dir = self.data_dir.join("covers");
                    let work_id = work.id;
                    let suffix = if is_foreign { "_thumb" } else { "" };
                    let url = url.clone();
                    let http = self.http.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            download_cover_to_disk(&http, &url, &covers_dir, work_id, suffix).await
                        {
                            tracing::warn!(work_id, %url, "background cover download failed: {e}");
                        }
                    });
                }
            }
            return Ok(AddWorkResult {
                work,
                author_created,
                author_id,
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

        // Post-enrichment cover download (enrichment may have found a better cover URL).
        if let Some(ref cover_url) = enriched_work.cover_url {
            let covers_dir = self.data_dir.join("covers");
            let work_id = enriched_work.id;
            let url = cover_url.clone();
            let http = self.http.clone();
            tokio::spawn(async move {
                if let Err(e) = download_cover_to_disk(&http, &url, &covers_dir, work_id, "").await
                {
                    tracing::warn!(work_id, "post-enrich cover download failed: {e}");
                }
                // Delete stale thumbnail.
                let thumb = covers_dir.join(format!("{work_id}_thumb.jpg"));
                let _ = tokio::fs::remove_file(&thumb).await;
            });
        }

        Ok(AddWorkResult {
            work: enriched_work,
            author_created,
            author_id,
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

        let has_title = req.title.is_some();
        let has_author = req.author_name.is_some();
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

        // Write provenance for edited fields (re-lock as setter=User).
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

        let covers_dir = self.data_dir.join("covers");
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

        let cover_path = self.data_dir.join("covers").join(format!("{work_id}.jpg"));
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

    async fn download_cover_from_url(&self, work_id: i64, cover_url: &str) {
        let covers_dir = self.data_dir.join("covers");
        if let Err(e) =
            download_cover_to_disk(&self.http, cover_url, &covers_dir, work_id, "").await
        {
            tracing::warn!(work_id, cover_url, "download_cover_from_url failed: {e}");
        }
        let thumb = covers_dir.join(format!("{work_id}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb).await;
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

impl<D, E, H> WorkServiceImpl<D, E, H>
where
    D: ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
{
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
                    language: None,
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

fn unproxy_cover_url(url: &str) -> String {
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
