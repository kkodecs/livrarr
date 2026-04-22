use std::sync::Arc;
use std::time::Duration;

use livrarr_db::{
    AuthorDb, CreateSeriesDbRequest, CreateWorkDbRequest, LibraryItemDb, LinkWorkToSeriesRequest,
    ProvenanceDb, SeriesCacheDb, SeriesCacheEntry, SeriesDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

pub struct SeriesQueryServiceImpl<D, F, E, L = crate::llm_caller_service::LlmCallerImpl> {
    db: D,
    fetcher: F,
    enrichment: Arc<E>,
    data_dir: std::path::PathBuf,
    llm: L,
}

impl<D, F, E, L> SeriesQueryServiceImpl<D, F, E, L> {
    pub fn new(
        db: D,
        fetcher: F,
        enrichment: Arc<E>,
        data_dir: std::path::PathBuf,
        llm: L,
    ) -> Self {
        Self {
            db,
            fetcher,
            enrichment,
            data_dir,
            llm,
        }
    }
}

impl<D, F, E, L> SeriesQueryService for SeriesQueryServiceImpl<D, F, E, L>
where
    D: SeriesDb
        + AuthorDb
        + WorkDb
        + LibraryItemDb
        + SeriesCacheDb
        + ProvenanceDb
        + Clone
        + Send
        + Sync
        + 'static,
    F: HttpFetcher + Clone + Send + Sync + 'static,
    E: EnrichmentWorkflow + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
{
    async fn list_enriched(
        &self,
        user_id: UserId,
    ) -> Result<Vec<SeriesListView>, SeriesServiceError> {
        let all_series = self
            .db
            .list_all_series(user_id)
            .await
            .map_err(SeriesServiceError::Db)?;
        let authors = self
            .db
            .list_authors(user_id)
            .await
            .map_err(SeriesServiceError::Db)?;
        let works = self
            .db
            .list_works(user_id)
            .await
            .map_err(SeriesServiceError::Db)?;

        // Pre-index authors by id and works by series_id to avoid O(series×works).
        let author_map: std::collections::HashMap<i64, &str> =
            authors.iter().map(|a| (a.id, a.name.as_str())).collect();

        let mut works_by_series: std::collections::HashMap<i64, Vec<&Work>> =
            std::collections::HashMap::new();
        for w in &works {
            if let Some(sid) = w.series_id {
                works_by_series.entry(sid).or_default().push(w);
            }
        }

        let views = all_series
            .iter()
            .map(|s| {
                let author_name = author_map.get(&s.author_id).unwrap_or(&"").to_string();
                let series_works = works_by_series.get(&s.id);
                let works_in_library = series_works.map(|ws| ws.len() as i64).unwrap_or(0);
                let first_work_id = series_works.and_then(|ws| {
                    ws.iter()
                        .min_by(|a, b| {
                            a.series_position
                                .unwrap_or(f64::MAX)
                                .partial_cmp(&b.series_position.unwrap_or(f64::MAX))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|w| w.id)
                });
                SeriesListView {
                    id: s.id,
                    name: s.name.clone(),
                    gr_key: s.gr_key.clone(),
                    book_count: s.work_count,
                    monitor_ebook: s.monitor_ebook,
                    monitor_audiobook: s.monitor_audiobook,
                    works_in_library,
                    author_id: s.author_id,
                    author_name,
                    first_work_id,
                }
            })
            .collect();

        Ok(views)
    }

    async fn get_detail(
        &self,
        user_id: UserId,
        series_id: i64,
    ) -> Result<SeriesDetailView, SeriesServiceError> {
        let series = self
            .db
            .get_series(user_id, series_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .ok_or(SeriesServiceError::NotFound)?;

        let author = self
            .db
            .get_author(user_id, series.author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let all_works = self
            .db
            .list_works_by_author(user_id, series.author_id)
            .await
            .map_err(SeriesServiceError::Db)?;

        let mut series_works: Vec<&Work> = all_works
            .iter()
            .filter(|w| w.series_id == Some(series_id))
            .collect();
        series_works.sort_by(|a, b| {
            a.series_position
                .unwrap_or(f64::MAX)
                .partial_cmp(&b.series_position.unwrap_or(f64::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let work_ids: Vec<i64> = series_works.iter().map(|w| w.id).collect();
        let items = self
            .db
            .list_library_items_by_work_ids(user_id, &work_ids)
            .await
            .map_err(SeriesServiceError::Db)?;

        // Pre-index items by work_id to avoid O(works×items) filtering.
        let mut items_by_work: std::collections::HashMap<i64, Vec<LibraryItem>> =
            std::collections::HashMap::with_capacity(work_ids.len());
        for item in items {
            items_by_work.entry(item.work_id).or_default().push(item);
        }

        let works = series_works
            .iter()
            .map(|w| {
                let work_items = items_by_work.remove(&w.id).unwrap_or_default();
                SeriesWorkView {
                    work: (*w).clone(),
                    library_items: work_items,
                }
            })
            .collect();

        Ok(SeriesDetailView {
            id: series.id,
            name: series.name,
            gr_key: series.gr_key,
            book_count: series.work_count,
            monitor_ebook: series.monitor_ebook,
            monitor_audiobook: series.monitor_audiobook,
            author_id: author.id,
            author_name: author.name,
            works,
        })
    }

    async fn update_flags(
        &self,
        user_id: UserId,
        series_id: i64,
        monitor_ebook: bool,
        monitor_audiobook: bool,
    ) -> Result<UpdateSeriesView, SeriesServiceError> {
        let series = self
            .db
            .get_series(user_id, series_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .ok_or(SeriesServiceError::NotFound)?;

        self.db
            .get_author(user_id, series.author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let updated = self
            .db
            .update_series_flags(user_id, series_id, monitor_ebook, monitor_audiobook)
            .await
            .map_err(SeriesServiceError::Db)?;

        let works = self
            .db
            .list_works_by_author(user_id, series.author_id)
            .await
            .unwrap_or_default();
        let count = works
            .iter()
            .filter(|w| w.series_id == Some(series_id))
            .count() as i64;

        Ok(UpdateSeriesView {
            id: updated.id,
            name: updated.name,
            gr_key: updated.gr_key,
            book_count: updated.work_count,
            monitor_ebook: updated.monitor_ebook,
            monitor_audiobook: updated.monitor_audiobook,
            works_in_library: count,
        })
    }

    async fn resolve_gr_candidates(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<GrAuthorCandidateView>, SeriesServiceError> {
        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let url = format!(
            "https://www.goodreads.com/search?q={}&search_type=authors",
            urlencoding::encode(&author.name)
        );

        let html = fetch_gr_html(&self.fetcher, &url).await?;

        let candidates: Vec<GrAuthorCandidateView> =
            crate::goodreads::parse_author_search_html(&html)
                .into_iter()
                .map(|c| GrAuthorCandidateView {
                    gr_key: c.gr_key,
                    name: c.name,
                    profile_url: format!("https://www.goodreads.com{}", c.profile_url),
                })
                .collect();

        Ok(candidates)
    }

    async fn list_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let gr_key = author
            .gr_key
            .as_deref()
            .ok_or_else(|| SeriesServiceError::Validation {
                field: "gr_key".into(),
                message: "Author has no Goodreads key".into(),
            })?;

        let cache = self.db.get_series_cache(author_id).await.unwrap_or(None);
        let (cache_entries, fetched_at) = if let Some(cached) = cache {
            (cached.entries, Some(cached.fetched_at))
        } else {
            let raw_entries = fetch_author_series_pages(&self.fetcher, gr_key).await?;
            let entries = llm_clean_series_list(&self.llm, &author.name, &raw_entries)
                .await
                .unwrap_or_else(|| raw_entries.clone());
            let llm_changed = entries.len() != raw_entries.len();
            let saved = self
                .db
                .save_series_cache(
                    author_id,
                    &entries,
                    if llm_changed {
                        Some(&raw_entries)
                    } else {
                        None
                    },
                )
                .await
                .map_err(SeriesServiceError::Db)?;
            (saved.entries, Some(saved.fetched_at))
        };

        let db_series = self
            .db
            .list_series_for_author(user_id, author_id)
            .await
            .unwrap_or_default();

        let works = self
            .db
            .list_works_by_author(user_id, author_id)
            .await
            .unwrap_or_default();

        let series = build_merged_series_list(&cache_entries, &db_series, &works);
        Ok(AuthorSeriesListView { series, fetched_at })
    }

    async fn refresh_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let gr_key = author
            .gr_key
            .as_deref()
            .ok_or_else(|| SeriesServiceError::Validation {
                field: "gr_key".into(),
                message: "Author has no Goodreads key".into(),
            })?;

        let _ = self.db.delete_series_cache(author_id).await;
        let raw_entries = fetch_author_series_pages(&self.fetcher, gr_key).await?;
        let entries = llm_clean_series_list(&self.llm, &author.name, &raw_entries)
            .await
            .unwrap_or_else(|| raw_entries.clone());
        let llm_changed = entries.len() != raw_entries.len();
        let saved = self
            .db
            .save_series_cache(
                author_id,
                &entries,
                if llm_changed {
                    Some(&raw_entries)
                } else {
                    None
                },
            )
            .await
            .map_err(SeriesServiceError::Db)?;

        let db_series = self
            .db
            .list_series_for_author(user_id, author_id)
            .await
            .unwrap_or_default();

        let works = self
            .db
            .list_works_by_author(user_id, author_id)
            .await
            .unwrap_or_default();

        let series = build_merged_series_list(&saved.entries, &db_series, &works);
        Ok(AuthorSeriesListView {
            series,
            fetched_at: Some(saved.fetched_at),
        })
    }

    async fn monitor_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        req: MonitorSeriesServiceRequest,
    ) -> Result<MonitorSeriesView, SeriesServiceError> {
        let _author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let cache = self
            .db
            .get_series_cache(author_id)
            .await
            .unwrap_or(None)
            .ok_or_else(|| SeriesServiceError::Validation {
                field: "gr_key".into(),
                message: "Fetch series list first".into(),
            })?;

        let cache_entry = cache
            .entries
            .iter()
            .find(|e| e.gr_key == req.gr_key)
            .ok_or_else(|| {
                tracing::warn!(
                    author_id,
                    requested_gr_key = %req.gr_key,
                    available_gr_keys = ?cache.entries.iter().map(|e| format!("{}={}", e.gr_key, e.name)).collect::<Vec<_>>(),
                    "grKey not found in cache"
                );
                SeriesServiceError::Validation {
                    field: "gr_key".into(),
                    message: format!("Series {} not found in cache", req.gr_key),
                }
            })?;

        let series = self
            .db
            .upsert_series(CreateSeriesDbRequest {
                user_id,
                author_id,
                name: cache_entry.name.clone(),
                gr_key: req.gr_key.clone(),
                monitor_ebook: req.monitor_ebook,
                monitor_audiobook: req.monitor_audiobook,
                work_count: cache_entry.book_count,
            })
            .await
            .map_err(SeriesServiceError::Db)?;

        Ok(MonitorSeriesView {
            id: series.id,
            name: series.name,
            gr_key: series.gr_key,
            book_count: series.work_count,
            monitor_ebook: series.monitor_ebook,
            monitor_audiobook: series.monitor_audiobook,
            works_in_library: 0,
        })
    }

    async fn run_series_monitor_worker(
        &self,
        params: SeriesMonitorWorkerParams,
    ) -> Result<(), SeriesServiceError> {
        let SeriesMonitorWorkerParams {
            user_id,
            author_id,
            series_id,
            series_name,
            series_gr_key,
            monitor_ebook,
            monitor_audiobook,
        } = params;

        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        let mut all_books = Vec::new();
        let mut page = 1;

        loop {
            let url = if page == 1 {
                format!("https://www.goodreads.com/series/{}", series_gr_key)
            } else {
                format!(
                    "https://www.goodreads.com/series/{}?page={}",
                    series_gr_key, page
                )
            };

            let html = fetch_gr_html(&self.fetcher, &url).await?;
            let (books, has_next) = crate::goodreads::parse_series_detail_html(&html);

            if books.is_empty() {
                break;
            }

            all_books.extend(books);

            if !has_next || page >= 10 {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        // Filter to primary works: integer positions (1.0, 2.0, ...).
        let primary_books: Vec<_> = all_books
            .into_iter()
            .filter(|b| {
                b.position
                    .map(|p| p > 0.0 && p.fract() == 0.0)
                    .unwrap_or(false)
            })
            .collect();

        tracing::info!(
            series = %series_name,
            author = %author.name,
            books = primary_books.len(),
            "series detail fetched (primary works only)"
        );

        let all_books = primary_books;

        // Re-read current series flags (cancellation guard).
        let series = self
            .db
            .get_series(user_id, series_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .ok_or(SeriesServiceError::NotFound)?;

        if !series.monitor_ebook && !series.monitor_audiobook {
            tracing::info!(series = %series_name, "series unmonitored — skipping work creation");
            return Ok(());
        }

        let _ = self
            .db
            .update_series_work_count(user_id, series_id, all_books.len() as i32)
            .await;

        let existing_works = self
            .db
            .list_works_by_author(user_id, author_id)
            .await
            .map_err(SeriesServiceError::Db)?;

        let mut created = 0u32;
        let mut linked = 0u32;
        let max_works = 50;

        for book in &all_books {
            if created >= max_works {
                tracing::warn!(series = %series_name, "hit max works cap ({max_works})");
                break;
            }

            // Cancellation guard: re-read flags per work.
            let current = self
                .db
                .get_series(user_id, series_id)
                .await
                .map_err(SeriesServiceError::Db)?;
            if let Some(s) = &current {
                if !s.monitor_ebook && !s.monitor_audiobook {
                    tracing::info!(series = %series_name, "series unmonitored mid-task — stopping");
                    break;
                }
            }

            // Match rule 1: exact gr_key.
            let matched = existing_works
                .iter()
                .find(|w| w.gr_key.as_deref() == Some(&book.gr_key));

            if let Some(existing) = matched {
                let _ = self
                    .db
                    .link_work_to_series(
                        user_id,
                        LinkWorkToSeriesRequest {
                            work_id: existing.id,
                            series_id,
                            series_work_count: series.work_count,
                            series_name: series_name.clone(),
                            series_position: book.position,
                            monitor_ebook,
                            monitor_audiobook,
                        },
                    )
                    .await;
                linked += 1;
                continue;
            }

            // Match rule 2: normalized title.
            let norm_title = normalize_for_match(&book.title);
            let title_matched = existing_works
                .iter()
                .find(|w| normalize_for_match(&w.title) == norm_title);

            if let Some(existing) = title_matched {
                let _ = self
                    .db
                    .link_work_to_series(
                        user_id,
                        LinkWorkToSeriesRequest {
                            work_id: existing.id,
                            series_id,
                            series_work_count: series.work_count,
                            series_name: series_name.clone(),
                            series_position: book.position,
                            monitor_ebook,
                            monitor_audiobook,
                        },
                    )
                    .await;
                linked += 1;
                continue;
            }

            // No match — create new work.
            let cleaned_title = crate::title_cleanup::clean_title(&book.title);
            let cleaned_author = crate::title_cleanup::clean_author(&author.name);
            match self
                .db
                .create_work(CreateWorkDbRequest {
                    user_id: author.user_id,
                    title: cleaned_title,
                    author_name: cleaned_author,
                    author_id: Some(author.id),
                    ol_key: None,
                    gr_key: Some(book.gr_key.clone()),
                    year: book.year,
                    cover_url: None,
                    metadata_source: None,
                    detail_url: None,
                    language: None,
                    import_id: None,
                    series_id: Some(series_id),
                    series_name: Some(series_name.clone()),
                    series_position: book.position,
                    monitor_ebook,
                    monitor_audiobook,
                })
                .await
            {
                Ok(work) => {
                    created += 1;
                    write_addtime_provenance(&self.db, user_id, &work).await;
                    tracing::debug!(
                        work_id = work.id,
                        title = %book.title,
                        "created work from series"
                    );
                    let enrichment = self.enrichment.clone();
                    let work_id = work.id;
                    let covers_dir = self.data_dir.join("covers");
                    let fetcher = self.fetcher.clone();
                    tokio::spawn(async move {
                        let result = tokio::time::timeout(
                            Duration::from_secs(30),
                            enrichment.enrich_work(user_id, work_id, EnrichmentMode::Background),
                        )
                        .await;
                        match result {
                            Ok(Ok(r)) => {
                                if let Some(ref url) = r.work.cover_url {
                                    if let Err(e) = crate::work_service::download_cover_to_disk(
                                        &fetcher,
                                        url,
                                        &covers_dir,
                                        work_id,
                                        "",
                                    )
                                    .await
                                    {
                                        tracing::warn!(work_id, "cover download failed: {e}");
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::warn!(work_id, "series-add enrichment failed: {e}");
                            }
                            Err(_) => {
                                tracing::warn!(work_id, "enrichment timed out");
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(title = %book.title, "failed to create work: {e}");
                }
            }
        }

        tracing::info!(
            series = %series_name,
            author = %author.name,
            created,
            linked,
            "series monitor worker complete"
        );

        Ok(())
    }
}

// =============================================================================
// Helpers
// =============================================================================

async fn fetch_gr_html<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
) -> Result<String, SeriesServiceError> {
    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![("Accept-Language".into(), "en-US,en;q=0.9".into())],
        body: None,
        timeout: Duration::from_secs(15),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 5 * 1024 * 1024,
        anti_bot_check: true,
        user_agent: UserAgentProfile::Browser,
    };
    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|_| SeriesServiceError::GoodreadsUnavailable)?;
    if resp.status != 200 {
        return Err(SeriesServiceError::GoodreadsUnavailable);
    }
    String::from_utf8(resp.body).map_err(|_| SeriesServiceError::GoodreadsUnavailable)
}

async fn fetch_author_series_pages<F: HttpFetcher>(
    fetcher: &F,
    gr_author_id: &str,
) -> Result<Vec<SeriesCacheEntry>, SeriesServiceError> {
    let mut all_entries = Vec::new();
    let mut page = 1;

    loop {
        let url = format!(
            "https://www.goodreads.com/series/list?id={}&page={}",
            gr_author_id, page
        );

        let html = fetch_gr_html(fetcher, &url).await?;
        let (entries, has_next) = crate::goodreads::parse_series_list_html(&html);

        if entries.is_empty() {
            break;
        }

        all_entries.extend(entries.into_iter().map(|e| SeriesCacheEntry {
            name: e.name,
            gr_key: e.gr_key,
            book_count: e.book_count,
        }));

        if !has_next || page >= 10 {
            break;
        }

        page += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(all_entries)
}

fn build_merged_series_list(
    cache_entries: &[SeriesCacheEntry],
    db_series: &[Series],
    works: &[Work],
) -> Vec<AuthorSeriesItemView> {
    cache_entries
        .iter()
        .map(|ce| {
            let db_match = db_series.iter().find(|s| s.gr_key == ce.gr_key);

            let (id, monitor_ebook, monitor_audiobook) = if let Some(s) = db_match {
                (Some(s.id), s.monitor_ebook, s.monitor_audiobook)
            } else {
                (None, false, false)
            };

            let works_in_library = if let Some(s) = db_match {
                works.iter().filter(|w| w.series_id == Some(s.id)).count() as i64
            } else {
                works
                    .iter()
                    .filter(|w| w.series_name.as_deref() == Some(&ce.name))
                    .count() as i64
            };

            AuthorSeriesItemView {
                id,
                name: ce.name.clone(),
                gr_key: ce.gr_key.clone(),
                book_count: ce.book_count,
                monitor_ebook,
                monitor_audiobook,
                works_in_library,
            }
        })
        .collect()
}

async fn llm_clean_series_list<L: LlmCaller + Send + Sync>(
    llm: &L,
    author_name: &str,
    entries: &[SeriesCacheEntry],
) -> Option<Vec<SeriesCacheEntry>> {
    use std::collections::HashMap;

    if entries.is_empty() {
        return None;
    }

    let mut listing = String::new();
    for (i, e) in entries.iter().enumerate() {
        listing.push_str(&format!("{}: \"{}\" ({} books)\n", i, e.name, e.book_count));
    }

    let user_template = format!(
        "These are book series attributed to \"{author_name}\" from Goodreads:\n\n\
         {listing}\n\
         Clean up this list:\n\
         1. REMOVE series by a different person who shares the same name\n\
         2. REMOVE anthologies, compilations, box sets, and omnibus editions\n\
         3. REMOVE series where this author only contributed a foreword, introduction, or single story\n\
         4. Keep the author's own original series\n\n\
         Return a JSON array of indices to KEEP: [0, 2, 5, ...]\n\
         Return ONLY the JSON array, no other text."
    );

    let mut context = HashMap::new();
    context.insert(LlmField::AuthorName, LlmValue::Text(author_name.into()));
    context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing));

    let req = LlmCallRequest {
        system_template: Box::leak(
            "You are a librarian assistant. Clean up book series lists."
                .to_string()
                .into_boxed_str(),
        ),
        user_template: Box::leak(user_template.into_boxed_str()),
        context,
        allowed_fields: &[LlmField::AuthorName, LlmField::BibliographyHtml],
        timeout: Duration::from_secs(15),
        purpose: LlmPurpose::BibliographyCleanup,
    };

    let resp = llm.call(req).await.ok()?;

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

    let cleaned: Vec<SeriesCacheEntry> = indices
        .into_iter()
        .filter_map(|i| entries.get(i).cloned())
        .collect();

    if cleaned.is_empty() {
        return None;
    }

    Some(cleaned)
}

fn normalize_for_match(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

async fn write_addtime_provenance<D: ProvenanceDb>(db: &D, user_id: i64, work: &Work) {
    crate::provenance::write_addtime_provenance(db, user_id, work, ProvenanceSetter::AutoAdded)
        .await;
}
