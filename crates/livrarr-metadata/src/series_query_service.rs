use std::sync::Arc;
use std::time::Duration;

use livrarr_db::{
    AuthorDb, ConfigDb, CreateSeriesDbRequest, LibraryItemDb, LinkWorkToSeriesRequest,
    SeriesCacheDb, SeriesCacheEntry, SeriesDb, SeriesRosterDb, SeriesRosterEntry, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

pub struct SeriesQueryServiceImpl<
    D,
    F,
    W,
    L = livrarr_external_data::llm_caller_service::LlmCallerImpl,
> {
    db: D,
    fetcher: F,
    work_service: Arc<W>,
    llm: L,
}

impl<D, F, W, L> SeriesQueryServiceImpl<D, F, W, L> {
    pub fn new(db: D, fetcher: F, work_service: Arc<W>, llm: L) -> Self {
        Self {
            db,
            fetcher,
            work_service,
            llm,
        }
    }
}

impl<D, F, W, L> SeriesQueryServiceImpl<D, F, W, L>
where
    D: SeriesDb
        + AuthorDb
        + WorkDb
        + LibraryItemDb
        + SeriesCacheDb
        + SeriesRosterDb
        + ConfigDb
        + Clone
        + Send
        + Sync
        + 'static,
    F: HttpFetcher + Clone + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
{
    /// REQ-010 amendment 2: resolve a stub's GR identity on expand WITHOUT
    /// monitoring — the REQ-009 exact-match road only (no picker, no author
    /// resolution). Returns roster entries on success; `None` means the
    /// caller degrades to linked works and the stub row stays unchanged.
    /// Adoption (gr_key + real roster size, sentinel never leaks) happens
    /// only with a non-empty roster in hand.
    /// Load a series' FK-linked works with their library items, sorted by
    /// position — the shared road of `get_detail` and `series_books`.
    async fn linked_series_works(
        &self,
        user_id: UserId,
        series_id: i64,
        author_id: AuthorId,
    ) -> Result<Vec<SeriesWorkView>, SeriesServiceError> {
        let all_works = self
            .db
            .list_works_by_author(user_id, author_id)
            .await
            .map_err(SeriesServiceError::Db)?;
        let mut series_works: Vec<&Work> = all_works
            .iter()
            .filter(|w| w.series_id == Some(series_id))
            .collect();
        series_works.sort_by(|a, b| {
            let pa = a.series_position.unwrap_or(f64::MAX);
            let pb = b.series_position.unwrap_or(f64::MAX);
            pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
        });

        let work_ids: Vec<i64> = series_works.iter().map(|w| w.id).collect();
        let items = self
            .db
            .list_library_items_by_work_ids(user_id, &work_ids)
            .await
            .map_err(SeriesServiceError::Db)?;
        let mut items_by_work: std::collections::HashMap<i64, Vec<LibraryItem>> =
            std::collections::HashMap::with_capacity(work_ids.len());
        for item in items {
            items_by_work.entry(item.work_id).or_default().push(item);
        }

        Ok(series_works
            .iter()
            .map(|w| SeriesWorkView {
                work: (*w).clone(),
                library_items: items_by_work.remove(&w.id).unwrap_or_default(),
            })
            .collect())
    }

    /// Silent author resolution — the same rule the resolve-gr handler
    /// auto-links with: first autocomplete candidate at name similarity
    /// ≥ 0.90 (livrarr_matching::author_similarity is the authority).
    /// Persists the adopted key. `None` = genuinely ambiguous; caller
    /// degrades (expansion) or surfaces the picker (promotion).
    async fn silently_resolve_author_key(
        &self,
        user_id: UserId,
        author: &livrarr_domain::Author,
    ) -> Option<String> {
        let candidates = SeriesQueryService::resolve_gr_candidates(self, user_id, author.id)
            .await
            .ok()?;
        let first = candidates.first()?;
        if livrarr_matching::author_similarity(&author.name, &first.name) < 0.90 {
            return None;
        }
        let gr_key = first.gr_key.clone();
        self.db
            .update_author(
                user_id,
                author.id,
                livrarr_db::UpdateAuthorDbRequest {
                    name: None,
                    sort_name: None,
                    ol_key: None,
                    gr_key: Some(Some(gr_key.clone())),
                    monitored: None,
                    monitor_new_items: None,
                    monitor_since: None,
                    monitor_language: None,
                },
            )
            .await
            .ok()?;
        tracing::info!(author = %author.name, gr_key = %gr_key, "author silently resolved");
        Some(gr_key)
    }

    async fn silently_resolve_stub_roster(
        &self,
        user_id: UserId,
        series: &Series,
    ) -> Option<Vec<SeriesRosterEntry>> {
        let author = self.db.get_author(user_id, series.author_id).await.ok()?;
        if author.gr_key.is_none()
            && self
                .silently_resolve_author_key(user_id, &author)
                .await
                .is_none()
        {
            tracing::debug!(series = %series.name, author = %author.name,
                "silent resolution: author has no GR key and auto-link was ambiguous");
            return None;
        }

        let view =
            match SeriesQueryService::list_author_series(self, user_id, series.author_id, false)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!(series = %series.name, error = %e,
                    "silent resolution: author series list unavailable");
                    return None;
                }
            };
        // REQ-014: series names are title-like (no author component); the
        // author half of identity_key is unused here.
        let normalized = identity_matching::identity_key(&series.name, "").0;
        let matches: Vec<&AuthorSeriesItemView> = view
            .series
            .iter()
            .filter(|e| {
                !e.gr_key.is_empty() && identity_matching::identity_key(&e.name, "").0 == normalized
            })
            .collect();
        let [single] = matches.as_slice() else {
            tracing::debug!(series = %series.name, candidates = matches.len(),
                "silent resolution: no single exact name match");
            return None;
        };
        let gr_key = single.gr_key.clone();

        // Collision: another row for this author already owns the key — use
        // its roster for display only; rows merge only via promotion (REQ-009).
        let existing = self
            .db
            .list_series_for_author(user_id, series.author_id)
            .await
            .ok()?
            .into_iter()
            .find(|s| s.id != series.id && s.gr_key == gr_key);
        if let Some(other) = existing {
            // A stored-empty roster (pre-N1 break window) reads as absent —
            // the same emptiness-is-never-truth rule series_books applies.
            if let Ok(Some(roster)) = self.db.get_series_roster(other.id).await {
                if !roster.entries.is_empty() {
                    return Some(roster.entries);
                }
            }
            let books = fetch_series_roster_pages(&self.fetcher, &gr_key)
                .await
                .ok()?;
            let entries = to_roster_entries(&books);
            if entries.is_empty() {
                tracing::debug!(series = %series.name, gr_key = %gr_key,
                    "silent resolution: collided row roster fetch parsed empty — degrading");
                return None;
            }
            let _ = self.db.save_series_roster(other.id, &entries).await;
            // Same pairing rule as every roster save: the count follows.
            let _ = self
                .db
                .update_series_work_count(user_id, other.id, entries.len() as i32)
                .await;
            return Some(entries);
        }

        // Fetch first; adopt only with a non-empty roster in hand — an empty
        // parse would write work_count = 0, which wins the ST-007 guard.
        let books = match fetch_series_roster_pages(&self.fetcher, &gr_key).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(series = %series.name, gr_key = %gr_key, error = %e,
                    "silent resolution: roster fetch failed");
                return None;
            }
        };
        let entries = to_roster_entries(&books);
        if entries.is_empty() {
            tracing::debug!(series = %series.name, gr_key = %gr_key,
                "silent resolution: roster parsed empty — not adopting");
            return None;
        }
        self.db.save_series_roster(series.id, &entries).await.ok()?;
        if let Err(e) = self
            .db
            .update_series_identity(user_id, series.id, &gr_key, Some(entries.len() as i32))
            .await
        {
            tracing::warn!(series = %series.name, error = %e, "silent stub adoption failed");
        }
        tracing::info!(series = %series.name, gr_key = %gr_key, "stub silently resolved on expand");
        Some(entries)
    }
}

impl<D, F, W, L> SeriesQueryService for SeriesQueryServiceImpl<D, F, W, L>
where
    D: SeriesDb
        + AuthorDb
        + WorkDb
        + LibraryItemDb
        + SeriesCacheDb
        + SeriesRosterDb
        + ConfigDb
        + Clone
        + Send
        + Sync
        + 'static,
    F: HttpFetcher + Clone + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
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
                            let pa = a.series_position.unwrap_or(f64::MAX);
                            let pb = b.series_position.unwrap_or(f64::MAX);
                            pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|w| w.id)
                });
                // Q-002 pre-fill: the dominant language among the series' linked
                // works; a tie or an all-language-less set yields None (the UI
                // defaults the selector). Shared rule with the monitor-enable guard.
                let suggested_language = series_works.and_then(|ws| {
                    livrarr_domain::seed::dominant_language(
                        ws.iter().map(|w| w.language.as_deref()),
                    )
                });
                SeriesListView {
                    id: s.id,
                    name: s.name.clone(),
                    gr_key: s.gr_key.clone(),
                    book_count: s.work_count,
                    monitor_ebook: s.monitor_ebook,
                    monitor_audiobook: s.monitor_audiobook,
                    monitor_language: s.monitor_language.clone(),
                    suggested_language,
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

        let works = self
            .linked_series_works(user_id, series_id, series.author_id)
            .await?;

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
        language: Option<String>,
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

        // REQ-009: monitoring is never enabled without a resolved gr_key — a
        // stub must go through the promotion road, not the flag toggle.
        if crate::series_link::is_stub_key(&series.gr_key) && (monitor_ebook || monitor_audiobook) {
            return Err(SeriesServiceError::Validation {
                field: "gr_key".into(),
                message: "Series has no Goodreads key — promote it to start monitoring".into(),
            });
        }

        let updated = self
            .db
            .update_series_flags(
                user_id,
                series_id,
                monitor_ebook,
                monitor_audiobook,
                language.as_deref().map(livrarr_domain::normalize_language),
            )
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

        // JSON autocomplete API is the only road (REQ-005/ST-012): no GR
        // `/search` HTML fallback. Empty means "author not found on
        // Goodreads" — an honest outcome, never a scrape.
        Ok(resolve_gr_candidates_json(&self.fetcher, &author.name).await)
    }

    async fn list_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        raw: bool,
    ) -> Result<AuthorSeriesListView, SeriesServiceError> {
        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        // REQ-003 degraded mode: an author with no gr_key cannot reach GR,
        // but their DB-backed series (stubs included) must still be served —
        // the GR cache/fetch leg is skipped, never an error.
        let (filtered_entries, raw_entries_opt, fetched_at) = match author.gr_key.as_deref() {
            None => (Vec::new(), None, None),
            Some(gr_key) => {
                let cache = self.db.get_series_cache(author_id).await.unwrap_or(None);
                if let Some(cached) = cache {
                    (cached.entries, cached.raw_entries, Some(cached.fetched_at))
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
                                Some(raw_entries.as_slice())
                            } else {
                                None
                            },
                        )
                        .await
                        .map_err(SeriesServiceError::Db)?;
                    (saved.entries, saved.raw_entries, Some(saved.fetched_at))
                }
            }
        };

        let raw_available = raw_entries_opt.is_some();
        let filtered_count = filtered_entries.len();
        let raw_count = raw_entries_opt
            .as_ref()
            .map(|r| r.len())
            .unwrap_or(filtered_count);

        let display_entries = if raw {
            raw_entries_opt.unwrap_or(filtered_entries)
        } else {
            filtered_entries
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

        let series = build_merged_series_list(&display_entries, &db_series, &works);
        Ok(AuthorSeriesListView {
            series,
            fetched_at,
            raw_available,
            filtered_count,
            raw_count,
        })
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
                    Some(raw_entries.as_slice())
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

        let raw_available = llm_changed;
        let filtered_count = saved.entries.len();
        let raw_count = if llm_changed {
            raw_entries.len()
        } else {
            filtered_count
        };

        let series = build_merged_series_list(&saved.entries, &db_series, &works);
        Ok(AuthorSeriesListView {
            series,
            fetched_at: Some(saved.fetched_at),
            raw_available,
            filtered_count,
            raw_count,
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
            .chain(cache.raw_entries.iter().flatten())
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
                monitor_language: req
                    .language
                    .as_deref()
                    .map(livrarr_domain::normalize_language),
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

        let all_books = fetch_series_roster_pages(&self.fetcher, &series_gr_key).await?;

        tracing::info!(
            series = %series_name,
            author = %author.name,
            books = all_books.len(),
            "series detail fetched (primary works only)"
        );

        // Roster write-through (REQ-010): persist the fetch this run already
        // paid for, so expansions never re-hit GR. Before the cancellation
        // check on purpose — a cancelled run still yields a roster. An EMPTY
        // fetch is drift, never truth: it must not erase stored data (N1).
        if all_books.is_empty() {
            tracing::warn!(
                series = %series_name,
                "series roster fetch parsed empty — leaving stored roster and work_count untouched"
            );
        } else if let Err(e) = self
            .db
            .save_series_roster(series_id, &to_roster_entries(&all_books))
            .await
        {
            tracing::warn!(series = %series_name, error = %e, "roster write-through failed");
        }

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

        // Same drift guard: an empty fetch must not zero the work count
        // (`work_count = 0` would also win ST-007's most-specific arbitration).
        if !all_books.is_empty() {
            let _ = self
                .db
                .update_series_work_count(user_id, series_id, all_books.len() as i32)
                .await;
        }

        let existing_works = self
            .db
            .list_works_by_author(user_id, author_id)
            .await
            .map_err(SeriesServiceError::Db)?;

        // The user's default language, read once per worker run: roster works
        // whose series has no monitor_language choice seed this value.
        let default_language = self
            .db
            .get_default_language()
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

            let matched = livrarr_matching::work_dedup::find_matching_work(
                &existing_works,
                &book.title,
                &author.name,
                &livrarr_matching::work_dedup::ProviderKeys {
                    gr_key: Some(&book.gr_key),
                    ..Default::default()
                },
            );

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

            // No match — create new work via WorkService::add() (M2 single creation gate).
            use livrarr_domain::identity::{IdentityState, PendingReason};
            use livrarr_domain::seed::{seed_series_monitor, SeedInput, SeedLanguage};
            match self
                .work_service
                .add(
                    author.user_id,
                    seed_series_monitor(
                        SeedInput {
                            title: book.title.clone(),
                            author_name: author.name.clone(),
                            language: SeedLanguage::resolve(
                                current.as_ref().and_then(|s| s.monitor_language.as_deref()),
                                &default_language,
                            ),
                            author_ol_key: None,
                            year: book.year,
                            cover_url: book.cover_url.clone(),
                            detail_url: None,
                            description: None,
                            series_name: Some(series_name.clone()),
                            series_position: book.position,
                        },
                        IdentityState::Pending {
                            reason: PendingReason::NoCandidates,
                            // Carry the source GR anchor (REQ-006) so the work persists its
                            // gr_key at create and the background resolver converges it — a
                            // work anchor resolves with no further network.
                            seed_anchors: livrarr_domain::normalization::normalize_gr_key(
                                &book.gr_key,
                            )
                            .map(|gr| {
                                livrarr_domain::identity::CapturedIdentity {
                                    ol_key: None,
                                    gr_key: Some(gr),
                                    hc_key: None,
                                    isbn_13: None,
                                    asin: None,
                                    title: book.title.clone(),
                                    author_name: author.name.clone(),
                                    language: None,
                                }
                            }),
                            top_candidates: vec![],
                        },
                        series_id,
                        monitor_ebook,
                        monitor_audiobook,
                    ),
                )
                .await
            {
                Ok(result) => {
                    if result.created {
                        created += 1;
                        tracing::debug!(
                            work_id = result.work.id,
                            title = %book.title,
                            "created work from series"
                        );
                    } else {
                        tracing::debug!(
                            work_id = result.work.id,
                            title = %book.title,
                            "dedup matched existing work in series monitor"
                        );
                    }
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

    async fn promote_stub(
        &self,
        user_id: UserId,
        series_id: i64,
        explicit_gr_key: Option<String>,
    ) -> Result<PromoteStubOutcome, SeriesServiceError> {
        let series = self
            .db
            .get_series(user_id, series_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .ok_or(SeriesServiceError::NotFound)?;

        let author_id = series.author_id;

        // Already GR-backed: nothing to resolve.
        if !crate::series_link::is_stub_key(&series.gr_key) {
            return Ok(PromoteStubOutcome::Resolved {
                author_id,
                series_id: series.id,
                gr_key: series.gr_key,
                name: series.name,
            });
        }

        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => SeriesServiceError::NotFound,
                other => SeriesServiceError::Db(other),
            })?;

        // REQ-009: the series-list fetch hard-requires an author gr_key.
        // Try the silent road first (same ≥0.90 similarity rule as the
        // resolve-gr auto-link); only genuine ambiguity surfaces the
        // author-candidate flow.
        if author.gr_key.is_none()
            && self
                .silently_resolve_author_key(user_id, &author)
                .await
                .is_none()
        {
            return Ok(PromoteStubOutcome::NeedsAuthorResolution { author_id });
        }

        let adopted_gr_key = match explicit_gr_key.filter(|k| !k.is_empty()) {
            Some(k) => k,
            None => {
                // Exact normalized-name match among the author's GR series.
                let view = self.list_author_series(user_id, author_id, false).await?;
                let normalized_stub = identity_matching::identity_key(&series.name, "").0;
                let matches: Vec<&AuthorSeriesItemView> = view
                    .series
                    .iter()
                    .filter(|e| {
                        !e.gr_key.is_empty()
                            && identity_matching::identity_key(&e.name, "").0 == normalized_stub
                    })
                    .collect();
                match matches.as_slice() {
                    [single] => single.gr_key.clone(),
                    _ => {
                        return Ok(PromoteStubOutcome::NeedsPicker {
                            author_id,
                            candidates: view
                                .series
                                .into_iter()
                                .filter(|e| !e.gr_key.is_empty())
                                .collect(),
                        });
                    }
                }
            }
        };

        // Collision (REQ-009/AC-018): the resolved gr_key already belongs to
        // another row for this author — merge the stub into it.
        let existing = self
            .db
            .list_series_for_author(user_id, author_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .into_iter()
            .find(|s| s.id != series.id && s.gr_key == adopted_gr_key);

        if let Some(survivor) = existing {
            self.db
                .relink_series_works(user_id, series.id, survivor.id)
                .await
                .map_err(SeriesServiceError::Db)?;
            self.db
                .delete_series(user_id, series.id)
                .await
                .map_err(SeriesServiceError::Db)?;
            return Ok(PromoteStubOutcome::Resolved {
                author_id,
                series_id: survivor.id,
                gr_key: adopted_gr_key,
                name: survivor.name,
            });
        }

        // Adopt in place — row id and work links survive (REQ-008). The
        // sentinel work_count stays until the monitor worker writes the real
        // GR roster size moments later.
        self.db
            .update_series_identity(user_id, series.id, &adopted_gr_key, None)
            .await
            .map_err(SeriesServiceError::Db)?;

        Ok(PromoteStubOutcome::Resolved {
            author_id,
            series_id: series.id,
            gr_key: adopted_gr_key,
            name: series.name,
        })
    }

    async fn series_books(
        &self,
        user_id: UserId,
        series_id: i64,
    ) -> Result<SeriesBooksView, SeriesServiceError> {
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

        // FK-linked works + their library items (shared road with get_detail).
        let linked = self
            .linked_series_works(user_id, series_id, series.author_id)
            .await?;

        // Stubs resolve silently on first expand (REQ-010 amendment 2): the
        // REQ-009 exact-match road, monitoring untouched. On any failure the
        // expansion degrades to linked works — never an error, no adoption.
        if crate::series_link::is_stub_key(&series.gr_key) {
            match self.silently_resolve_stub_roster(user_id, &series).await {
                Some(entries) => {
                    return Ok(SeriesBooksView {
                        roster_available: true,
                        rows: merge_roster_with_works(&entries, linked, &author.name),
                    });
                }
                None => {
                    let rows = linked
                        .into_iter()
                        .map(|sw| SeriesBookRow::InLibrary {
                            position: sw.work.series_position,
                            entry: Box::new(sw),
                        })
                        .collect();
                    return Ok(SeriesBooksView {
                        roster_available: false,
                        rows,
                    });
                }
            }
        }

        // Persisted roster: a stored NON-EMPTY roster serves without a
        // refetch (AC-022). Emptiness is never persisted (N1): an empty
        // parse means drift or an unreadable page, so the view degrades to
        // linked works and the next expansion retries — the store heals as
        // soon as GR yields books again. (Pre-N1 rows that stored an empty
        // roster during the 2026-07 layout break heal through the same
        // road: empty-stored reads as absent and triggers the refetch.)
        let stored = self
            .db
            .get_series_roster(series_id)
            .await
            .map_err(SeriesServiceError::Db)?
            .map(|roster| roster.entries)
            .filter(|entries| !entries.is_empty());
        let entries = match stored {
            Some(entries) => entries,
            None => {
                let books = fetch_series_roster_pages(&self.fetcher, &series.gr_key).await?;
                let entries = to_roster_entries(&books);
                if entries.is_empty() {
                    let rows = linked
                        .into_iter()
                        .map(|sw| SeriesBookRow::InLibrary {
                            position: sw.work.series_position,
                            entry: Box::new(sw),
                        })
                        .collect();
                    return Ok(SeriesBooksView {
                        roster_available: false,
                        rows,
                    });
                }
                self.db
                    .save_series_roster(series_id, &entries)
                    .await
                    .map_err(SeriesServiceError::Db)?;
                // work_count IS the GR roster size (ST-007): every roster
                // save pairs with a count update, or a healed roster would
                // sit beside a stale count (the broken-window rows carry 0,
                // which wins most-specific arbitration).
                let _ = self
                    .db
                    .update_series_work_count(user_id, series_id, entries.len() as i32)
                    .await;
                entries
            }
        };

        Ok(SeriesBooksView {
            roster_available: true,
            rows: merge_roster_with_works(&entries, linked, &author.name),
        })
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
        // Parked (B4 table): this one fn serves BOTH the background series
        // monitor AND the interactive roster-expand door — stays Normal
        // rather than picking a value that's wrong for one of the two.
        priority: RequestPriority::Normal,
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

/// Fetch + parse a GR series' detail pages (ST-008 road: paged, ≤10 pages,
/// 1s pacing) and keep the PRIMARY-works roster — the same set `work_count`
/// counts. On the 2026-07 React layout the page lists primaries FIRST and
/// the header states their count; omnibuses, split editions, and
/// translations follow (measured on series 108562 and 43318). No primary
/// count means the header drifted: return an empty roster (loud, never a
/// guess) rather than adopt GR's full 27-entry edition soup. Shared by the
/// monitor worker and the first-expand roster fetch (REQ-010).
async fn fetch_series_roster_pages<F: HttpFetcher>(
    fetcher: &F,
    series_gr_key: &str,
) -> Result<Vec<livrarr_external_data::goodreads::GoodreadsSeriesBook>, SeriesServiceError> {
    let mut collected = Vec::new();
    let mut primary_count: Option<usize> = None;
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

        let html = fetch_gr_html(fetcher, &url).await?;
        let parsed = livrarr_external_data::goodreads::parse_series_detail_html(&html);

        if page == 1 {
            primary_count = parsed.primary_count;
        }
        if parsed.books.is_empty() {
            break;
        }
        collected.extend(parsed.books);

        let Some(needed) = primary_count else { break };
        if collected.len() >= needed || !parsed.has_next || page >= 10 {
            break;
        }

        page += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let Some(needed) = primary_count else {
        if !collected.is_empty() {
            tracing::warn!(
                series_gr_key,
                books = collected.len(),
                "GR series page parsed books but carries no primary count — refusing to guess the roster"
            );
        }
        return Ok(Vec::new());
    };
    // Fewer books than the header declared means a later page was unreadable
    // (or the pagination walk stopped short): a PARTIAL roster is drift, not
    // truth — returning empty routes it into the same no-write guards, so a
    // stored full roster is never replaced by a partial one (review R-3).
    if collected.len() < needed {
        tracing::warn!(
            series_gr_key,
            collected = collected.len(),
            declared = needed,
            "GR roster: fewer books than the declared primary count — refusing a partial roster"
        );
        return Ok(Vec::new());
    }
    collected.truncate(needed);
    let before = collected.len();
    collected.retain(|b| !livrarr_external_data::goodreads::is_collection_title(&b.title));
    if collected.len() < before {
        tracing::warn!(
            series_gr_key,
            screened = before - collected.len(),
            "GR roster: screened collection-shaped titles inside the primary window"
        );
    }
    Ok(collected)
}

fn to_roster_entries(
    books: &[livrarr_external_data::goodreads::GoodreadsSeriesBook],
) -> Vec<SeriesRosterEntry> {
    books
        .iter()
        .map(|b| SeriesRosterEntry {
            title: b.title.clone(),
            gr_key: b.gr_key.clone(),
            position: b.position,
            year: b.year,
        })
        .collect()
}

/// REQ-010 merge: every roster entry becomes a row — in-library when a linked
/// work matches (normalized GR key first, then the shared work-matching
/// authority), missing otherwise. Linked works no roster entry claimed are
/// appended, never dropped. Pure — unit-tested below.
fn merge_roster_with_works(
    roster: &[SeriesRosterEntry],
    works: Vec<SeriesWorkView>,
    author_name: &str,
) -> Vec<SeriesBookRow> {
    use livrarr_domain::normalization::normalize_gr_key;

    let work_refs: Vec<Work> = works.iter().map(|sw| sw.work.clone()).collect();
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, sw) in works.iter().enumerate() {
        if let Some(k) = sw.work.gr_key.as_deref().and_then(normalize_gr_key) {
            by_key.entry(k).or_insert(i);
        }
    }

    let mut sorted: Vec<&SeriesRosterEntry> = roster.iter().collect();
    sorted.sort_by(|a, b| {
        let pa = a.position.unwrap_or(f64::MAX);
        let pb = b.position.unwrap_or(f64::MAX);
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut slots: Vec<Option<SeriesWorkView>> = works.into_iter().map(Some).collect();
    let mut rows = Vec::with_capacity(sorted.len());

    for entry in sorted {
        let idx = normalize_gr_key(&entry.gr_key)
            .and_then(|k| by_key.get(&k).copied())
            .or_else(|| {
                livrarr_matching::work_dedup::find_matching_work(
                    &work_refs,
                    &entry.title,
                    author_name,
                    &livrarr_matching::work_dedup::ProviderKeys {
                        gr_key: Some(&entry.gr_key),
                        ..Default::default()
                    },
                )
                .and_then(|m| work_refs.iter().position(|w| w.id == m.id))
            });

        match idx.and_then(|i| slots[i].take()) {
            Some(sw) => rows.push(SeriesBookRow::InLibrary {
                position: entry.position.or(sw.work.series_position),
                entry: Box::new(sw),
            }),
            None => rows.push(SeriesBookRow::Missing {
                position: entry.position,
                title: entry.title.clone(),
                year: entry.year,
            }),
        }
    }

    // Linked works the roster didn't claim — appended, never dropped.
    let mut leftovers: Vec<SeriesWorkView> = slots.into_iter().flatten().collect();
    leftovers.sort_by(|a, b| {
        let pa = a.work.series_position.unwrap_or(f64::MAX);
        let pb = b.work.series_position.unwrap_or(f64::MAX);
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    });
    for sw in leftovers {
        rows.push(SeriesBookRow::InLibrary {
            position: sw.work.series_position,
            entry: Box::new(sw),
        });
    }

    rows
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrAutocompleteBook {
    author: Option<GrAutocompleteAuthor>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrAutocompleteAuthor {
    #[serde(deserialize_with = "de_stringish_id")]
    id: String,
    name: String,
    #[serde(default)]
    profile_url: String,
    #[allow(dead_code)] // deserialized from Goodreads but not consumed
    #[serde(default)]
    is_goodreads_author: bool,
}

fn de_stringish_id<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Id {
        Num(i64),
        Str(String),
    }
    use serde::Deserialize as _;
    match Id::deserialize(de)? {
        Id::Num(n) => Ok(n.to_string()),
        Id::Str(s) => Ok(s),
    }
}

async fn resolve_gr_candidates_json<F: HttpFetcher>(
    fetcher: &F,
    author_name: &str,
) -> Vec<GrAuthorCandidateView> {
    let clean_name: String = author_name
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let url = format!(
        "https://www.goodreads.com/book/auto_complete?format=json&q={}",
        urlencoding::encode(clean_name.trim())
    );

    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: Duration::from_secs(10),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 512 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Browser,
        priority: RequestPriority::Normal,
    };

    let resp = match fetcher.fetch(req).await {
        Ok(r) if r.status == 200 => r,
        _ => return Vec::new(),
    };

    let items: Vec<GrAutocompleteBook> = match serde_json::from_slice(&resp.body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for item in items {
        let Some(author) = item.author else {
            continue;
        };
        if author.name.trim().is_empty() {
            continue;
        }
        if !seen.insert(author.id.clone()) {
            continue;
        }
        out.push(GrAuthorCandidateView {
            gr_key: author.id,
            name: author.name,
            profile_url: author.profile_url,
        });
    }

    out
}

async fn fetch_author_series_pages<F: HttpFetcher>(
    fetcher: &F,
    gr_author_id: &str,
) -> Result<Vec<SeriesCacheEntry>, SeriesServiceError> {
    // Primary: HTML series list page (has proper gr_keys for monitoring)
    let mut all_entries = Vec::new();
    let mut page = 1;

    loop {
        let url = format!(
            "https://www.goodreads.com/series/list?id={}&page={}",
            gr_author_id, page
        );

        let html = fetch_gr_html(fetcher, &url).await?;
        let (entries, has_next) = livrarr_external_data::goodreads::parse_series_list_html(&html);

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

    // No GR `/search` fallback (REQ-005/ST-012): the series-list pages are
    // the only road. An empty result is an honest empty result — series
    // names from work metadata surface via DB stubs instead.
    Ok(all_entries)
}

fn build_merged_series_list(
    cache_entries: &[SeriesCacheEntry],
    db_series: &[Series],
    works: &[Work],
) -> Vec<AuthorSeriesItemView> {
    let mut matched_db_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut views: Vec<AuthorSeriesItemView> = cache_entries
        .iter()
        .map(|ce| {
            let db_match = if ce.gr_key.is_empty() {
                db_series.iter().find(|s| s.name == ce.name)
            } else {
                db_series.iter().find(|s| s.gr_key == ce.gr_key)
            };

            let (id, monitor_ebook, monitor_audiobook) = if let Some(s) = db_match {
                matched_db_ids.insert(s.id);
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
        .collect();

    // REQ-003: DB rows (stubs included) that matched no cache entry are
    // appended, never dropped — FK-counted. A stub's gr_key is exposed as
    // empty: "stub:" keys are internal, and the UI hides GR links for
    // keyless series.
    for s in db_series {
        if matched_db_ids.contains(&s.id) {
            continue;
        }
        let works_in_library = works.iter().filter(|w| w.series_id == Some(s.id)).count() as i64;
        let is_stub = crate::series_link::is_stub_key(&s.gr_key);
        views.push(AuthorSeriesItemView {
            id: Some(s.id),
            name: s.name.clone(),
            gr_key: if is_stub {
                String::new()
            } else {
                s.gr_key.clone()
            },
            book_count: if is_stub { 0 } else { s.work_count },
            monitor_ebook: s.monitor_ebook,
            monitor_audiobook: s.monitor_audiobook,
            works_in_library,
        });
    }

    views
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
         2. REMOVE box sets and omnibus editions that repackage books from other series\n\
         3. REMOVE series where this author only contributed a foreword, introduction, or single story\n\
         4. Keep the author's own original series\n\
         5. Fix capitalization: use standard Title Case (e.g. \"night angel\" → \"Night Angel\")\n\n\
         Return a JSON array of objects for series to KEEP: [{{\"i\": 0, \"name\": \"Corrected Name\"}}, ...]\n\
         If the name is already correct, use the original name.\n\
         Return ONLY the JSON array, no other text."
    );

    let mut context = HashMap::new();
    context.insert(LlmField::AuthorName, LlmValue::Text(author_name.into()));
    context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing));

    let req = LlmCallRequest {
        system_template: "You are a librarian assistant. Clean up book series lists.".to_string(),
        user_template,
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

    #[derive(serde::Deserialize)]
    struct KeepEntry {
        i: usize,
        name: Option<String>,
    }

    let cleaned: Vec<SeriesCacheEntry> =
        if let Ok(entries_with_names) = serde_json::from_str::<Vec<KeepEntry>>(json_str) {
            entries_with_names
                .into_iter()
                .filter_map(|ke| {
                    entries.get(ke.i).map(|orig| SeriesCacheEntry {
                        name: ke.name.unwrap_or_else(|| orig.name.clone()),
                        gr_key: orig.gr_key.clone(),
                        book_count: orig.book_count,
                    })
                })
                .collect()
        } else if let Ok(indices) = serde_json::from_str::<Vec<usize>>(json_str) {
            indices
                .into_iter()
                .filter_map(|i| entries.get(i).cloned())
                .collect()
        } else {
            return None;
        };

    if cleaned.is_empty() {
        return None;
    }

    Some(cleaned)
}

#[cfg(test)]
mod roster_merge_tests {
    use super::*;

    fn entry(title: &str, gr_key: &str, position: Option<f64>) -> SeriesRosterEntry {
        SeriesRosterEntry {
            title: title.to_string(),
            gr_key: gr_key.to_string(),
            position,
            year: None,
        }
    }

    fn linked_work(
        id: i64,
        title: &str,
        gr_key: Option<&str>,
        position: Option<f64>,
    ) -> SeriesWorkView {
        SeriesWorkView {
            work: Work {
                id,
                title: title.to_string(),
                author_name: "Jim Butcher".to_string(),
                gr_key: gr_key.map(str::to_string),
                series_position: position,
                ..Default::default()
            },
            library_items: vec![],
        }
    }

    fn titles(rows: &[SeriesBookRow]) -> Vec<(String, bool)> {
        rows.iter()
            .map(|r| match r {
                SeriesBookRow::InLibrary { entry, .. } => (entry.work.title.clone(), true),
                SeriesBookRow::Missing { title, .. } => (title.clone(), false),
            })
            .collect()
    }

    #[test]
    fn matches_by_normalized_gr_key() {
        let roster = vec![entry("Storm Front", "12345.Storm_Front", Some(1.0))];
        let works = vec![linked_work(
            1,
            "Storm Front (different title casing)",
            Some("12345"),
            Some(1.0),
        )];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert!(matches!(rows[0], SeriesBookRow::InLibrary { .. }));
    }

    #[test]
    fn falls_back_to_title_match_when_no_key() {
        let roster = vec![entry("Storm Front", "99999", Some(1.0))];
        let works = vec![linked_work(1, "Storm Front", None, Some(1.0))];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert!(matches!(rows[0], SeriesBookRow::InLibrary { .. }));
    }

    #[test]
    fn unmatched_entry_is_missing() {
        let roster = vec![
            entry("Storm Front", "1", Some(1.0)),
            entry("Fool Moon", "2", Some(2.0)),
        ];
        let works = vec![linked_work(1, "Storm Front", Some("1"), Some(1.0))];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), true),
                ("Fool Moon".to_string(), false)
            ]
        );
    }

    #[test]
    fn linked_work_absent_from_roster_is_appended_never_dropped() {
        let roster = vec![entry("Storm Front", "1", Some(1.0))];
        let works = vec![
            linked_work(1, "Storm Front", Some("1"), Some(1.0)),
            linked_work(2, "Side Jobs", Some("777"), Some(12.5)),
        ];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), true),
                ("Side Jobs".to_string(), true)
            ]
        );
    }

    #[test]
    fn rows_follow_roster_position_order() {
        let roster = vec![
            entry("Fool Moon", "2", Some(2.0)),
            entry("Storm Front", "1", Some(1.0)),
        ];
        let rows = merge_roster_with_works(&roster, vec![], "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), false),
                ("Fool Moon".to_string(), false)
            ]
        );
    }

    #[test]
    fn one_work_claimed_once_second_entry_reads_missing() {
        let roster = vec![
            entry("Storm Front", "1", Some(1.0)),
            entry("Storm Front (Reissue)", "1", Some(2.0)),
        ];
        let works = vec![linked_work(1, "Storm Front", Some("1"), Some(1.0))];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), true),
                ("Storm Front (Reissue)".to_string(), false)
            ]
        );
    }
}
