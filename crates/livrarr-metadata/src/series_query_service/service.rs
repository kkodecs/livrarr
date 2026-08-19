use std::sync::Arc;

use livrarr_db::{
    AuthorDb, ConfigDb, CreateSeriesDbRequest, LibraryItemDb, LinkWorkToSeriesRequest,
    SeriesCacheDb, SeriesCacheEntry, SeriesDb, SeriesRosterDb, SeriesRosterEntry, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

use super::gr_candidates::resolve_gr_candidates_json;
use super::gr_fetch::{fetch_author_series_pages, fetch_series_roster_pages, to_roster_entries};
use super::llm_clean::llm_clean_series_list;
use super::roster_merge::merge_roster_with_works;
use super::series_list_merge::build_merged_series_list;

pub struct NoSeriesIdentityRoad;

pub struct ConfiguredSeriesIdentityRoad<R>(Arc<R>);

#[trait_variant::make(SeriesIdentityRoad: Send)]
pub trait LocalSeriesIdentityRoad: Send + Sync {
    async fn settle(
        &self,
        request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Option<
        Result<
            livrarr_domain::identity_layer::IdentityRoadOutcome,
            livrarr_domain::identity_layer::IdentityRoadError,
        >,
    >;
}

impl SeriesIdentityRoad for NoSeriesIdentityRoad {
    async fn settle(
        &self,
        _request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Option<
        Result<
            livrarr_domain::identity_layer::IdentityRoadOutcome,
            livrarr_domain::identity_layer::IdentityRoadError,
        >,
    > {
        None
    }
}

impl<R> SeriesIdentityRoad for ConfiguredSeriesIdentityRoad<R>
where
    R: livrarr_domain::identity_layer::IdentityRoadService + Send + Sync,
{
    async fn settle(
        &self,
        request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Option<
        Result<
            livrarr_domain::identity_layer::IdentityRoadOutcome,
            livrarr_domain::identity_layer::IdentityRoadError,
        >,
    > {
        Some(self.0.settle(request).await)
    }
}

pub struct SeriesQueryServiceImpl<
    D,
    F,
    W,
    L = livrarr_external_data::llm_caller_service::LlmCallerImpl,
    R = NoSeriesIdentityRoad,
> {
    db: D,
    fetcher: F,
    work_service: Arc<W>,
    llm: L,
    identity_road: R,
}

impl<D, F, W, L> SeriesQueryServiceImpl<D, F, W, L, NoSeriesIdentityRoad> {
    pub fn new(db: D, fetcher: F, work_service: Arc<W>, llm: L) -> Self {
        Self {
            db,
            fetcher,
            work_service,
            llm,
            identity_road: NoSeriesIdentityRoad,
        }
    }
}

impl<D, F, W, L, R> SeriesQueryServiceImpl<D, F, W, L, R> {
    pub fn with_identity_road<R2>(
        self,
        identity_road: Arc<R2>,
    ) -> SeriesQueryServiceImpl<D, F, W, L, ConfiguredSeriesIdentityRoad<R2>> {
        SeriesQueryServiceImpl {
            db: self.db,
            fetcher: self.fetcher,
            work_service: self.work_service,
            llm: self.llm,
            identity_road: ConfiguredSeriesIdentityRoad(identity_road),
        }
    }
}

impl<D, F, W, L, R> SeriesQueryServiceImpl<D, F, W, L, R>
where
    D: SeriesDb
        + AuthorDb
        + WorkDb
        + LibraryItemDb
        + SeriesCacheDb
        + SeriesRosterDb
        + ConfigDb
        + livrarr_db::AuthorLinkDb
        + Clone
        + Send
        + Sync
        + 'static,
    F: HttpFetcher + Clone + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    R: SeriesIdentityRoad + Send + Sync,
{
    /// REQ-010 amendment 2: resolve a stub's GR identity on expand WITHOUT
    /// monitoring — the REQ-009 exact-match road only (no picker, no author
    /// resolution). Returns roster entries on success; `None` means the
    /// caller degrades to linked works and the stub row stays unchanged.
    /// Adoption (gr_key + real roster size, sentinel never leaks) happens
    /// only with a non-empty roster in hand.
    /// Load a series' FK-linked works with their library items, sorted by
    /// position — the shared road of `get_detail` and `series_books`.
    ///
    /// #112: classify each series entry's language via Google Books. GR gives
    /// no language signal, but (unlike the bibliography path) the series name
    /// GR hands us is already in whatever language the series actually is —
    /// so a straight title search has no translation title-mismatch problem.
    /// Confidence gate reuses the project's one matching authority
    /// (`identity_matching::title_verdict`/`author_verdict`, wiki insight
    /// #59) instead of inventing new fuzzy-match logic: only trust a GB
    /// volume's language if its title is Same/Grey against the series name
    /// and its author doesn't Disagree.
    ///
    /// No GB key configured, no qualifying match, or any fetch error all
    /// default to `target_language` rather than Unknown — absence of
    /// evidence isn't evidence of a foreign series (same rule as the
    /// bibliography path's `classify_ol_language`; a real PO-reported
    /// "language unknown" showing on an author's own well-known series was
    /// confusing, not cautious). A confidently-detected OTHER language still
    /// overrides this and is flagged normally.
    /// "Author's language" for #112 classification — same resolution order
    /// as `author_service.rs`'s `fetch_bibliography_entries`.
    async fn effective_target_language(&self, author: &Author) -> String {
        match &author.monitor_language {
            Some(lang) => lang.clone(),
            None => self
                .db
                .get_default_language()
                .await
                .unwrap_or_else(|_| "en".to_string()),
        }
    }

    async fn classify_series_languages(
        &self,
        author_name: &str,
        target_language: &str,
        entries: Vec<SeriesCacheEntry>,
    ) -> Vec<SeriesCacheEntry> {
        let api_key = match self.db.get_metadata_config().await {
            Ok(cfg) => cfg.google_books_api_key.filter(|k| !k.is_empty()),
            Err(_) => None,
        };
        let Some(api_key) = api_key else {
            return entries
                .into_iter()
                .map(|mut e| {
                    e.language = Some(target_language.to_string());
                    e
                })
                .collect();
        };

        let mut out = Vec::with_capacity(entries.len());
        for mut entry in entries {
            entry.language = Some(
                self.classify_one_series_language(&api_key, author_name, &entry.name)
                    .await
                    .unwrap_or_else(|| target_language.to_string()),
            );
            out.push(entry);
        }
        out
    }

    async fn classify_one_series_language(
        &self,
        api_key: &str,
        author_name: &str,
        series_name: &str,
    ) -> Option<String> {
        let query = format!("intitle:\"{series_name}\" inauthor:\"{author_name}\"");
        let url = format!(
            "https://www.googleapis.com/books/v1/volumes?q={}&maxResults=5&fields=items(volumeInfo(title,authors,language))",
            urlencoding::encode(&query),
        );

        // Interactive: both callers (list_author_series, refresh_author_series)
        // are synchronous, user-facing series-tab loads, not background scans.
        let volumes = livrarr_external_data::google_books::fetch_gb_volumes(
            &self.fetcher,
            api_key,
            url,
            RequestPriority::Interactive,
        )
        .await
        .ok()?;

        let series_parsed = livrarr_domain::identity_matching::parse_title(series_name);
        let author_list = vec![author_name.to_string()];

        volumes.iter().find_map(|vol| {
            let vi = vol.volume_info.as_ref()?;
            let title = vi.title.as_ref()?;
            let lang = vi.language.as_ref()?;

            let vol_parsed = livrarr_domain::identity_matching::parse_title(title);
            let title_ok = matches!(
                livrarr_domain::identity_matching::title_verdict(&series_parsed, &vol_parsed),
                livrarr_domain::identity_matching::TitleVerdict::Same
                    | livrarr_domain::identity_matching::TitleVerdict::Grey { .. }
            );
            if !title_ok {
                return None;
            }

            let vol_authors = vi.authors.clone().unwrap_or_default();
            let author_ok = !matches!(
                livrarr_domain::identity_matching::author_verdict(&author_list, &vol_authors),
                livrarr_domain::identity_matching::AuthorVerdict::Disagree
            );
            if !author_ok {
                return None;
            }

            Some(livrarr_domain::normalize_language(lang))
        })
    }

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

    /// The author's active Goodreads routes, in stable order.
    ///
    /// The one authority for "which Goodreads feeds belong to this person" —
    /// never `authors.gr_key` (FP-036) and never a name guess (FP-013). An empty
    /// answer means the author is not linked to Goodreads; the caller degrades or
    /// shows the picker rather than adopting a lookalike.
    async fn active_goodreads_routes(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<String>, SeriesServiceError> {
        let routes = self
            .db
            .list_active_routes(user_id, author_id, Some(AuthorProvider::Goodreads))
            .await
            .map_err(SeriesServiceError::Db)?;
        Ok(routes.iter().map(|route| route.key.value()).collect())
    }

    /// Fetch every active Goodreads feed and union the series by canonical
    /// Goodreads series key.
    ///
    /// A person with two Goodreads author pages has one set of series, not two
    /// lists (FP-039). One feed failing is isolated: the siblings' results
    /// survive, and only a total failure is reported as an error, because an
    /// empty list would be cached and read as "this author has no series".
    async fn union_author_series_feeds(
        &self,
        gr_routes: &[String],
    ) -> Result<Vec<SeriesCacheEntry>, SeriesServiceError> {
        let mut union: Vec<SeriesCacheEntry> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut last_error: Option<SeriesServiceError> = None;
        for gr_key in gr_routes {
            match fetch_author_series_pages(&self.fetcher, gr_key, None).await {
                Ok(entries) => {
                    for entry in entries {
                        let identity = if entry.gr_key.is_empty() {
                            format!(
                                "name:{}",
                                identity_matching::identity_key(&entry.name, "").0
                            )
                        } else {
                            format!("gr:{}", entry.gr_key)
                        };
                        if seen.contains(&identity) {
                            continue;
                        }
                        seen.push(identity);
                        union.push(entry);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        %gr_key,
                        "author series: Goodreads feed unavailable, keeping sibling results: {e}"
                    );
                    last_error = Some(e);
                }
            }
        }
        match last_error {
            Some(e) if union.is_empty() => Err(e),
            _ => Ok(union),
        }
    }

    async fn silently_resolve_stub_roster(
        &self,
        user_id: UserId,
        series: &Series,
    ) -> Option<Vec<SeriesRosterEntry>> {
        let author = self.db.get_author(user_id, series.author_id).await.ok()?;
        if self
            .active_goodreads_routes(user_id, series.author_id)
            .await
            .ok()?
            .is_empty()
        {
            tracing::debug!(series = %series.name, author = %author.name,
                "silent resolution: author has no active Goodreads route");
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
            let books = fetch_series_roster_pages(&self.fetcher, &gr_key, None)
                .await
                .ok()?;
            let entries = to_roster_entries(&books);
            if entries.is_empty() {
                tracing::debug!(series = %series.name, gr_key = %gr_key,
                    "silent resolution: collided row roster fetch parsed empty — degrading");
                return None;
            }
            if let Err(e) = self.db.save_series_roster(other.id, &entries).await {
                tracing::warn!(
                    series_id = other.id,
                    "silent resolution: roster save failed: {e}"
                );
            }
            // Same pairing rule as every roster save: the count follows.
            if let Err(e) = self
                .db
                .update_series_work_count(user_id, other.id, entries.len() as i32)
                .await
            {
                tracing::warn!(
                    series_id = other.id,
                    "silent resolution: work-count update failed: {e}"
                );
            }
            return Some(entries);
        }

        // Fetch first; adopt only with a non-empty roster in hand — an empty
        // parse would write work_count = 0, which wins the ST-007 guard.
        let books = match fetch_series_roster_pages(&self.fetcher, &gr_key, None).await {
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

impl<D, F, W, L, R> SeriesQueryService for SeriesQueryServiceImpl<D, F, W, L, R>
where
    D: SeriesDb
        + AuthorDb
        + WorkDb
        + LibraryItemDb
        + SeriesCacheDb
        + SeriesRosterDb
        + ConfigDb
        + livrarr_db::AuthorLinkDb
        + Clone
        + Send
        + Sync
        + 'static,
    F: HttpFetcher + Clone + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
    L: LlmCaller + Send + Sync,
    R: SeriesIdentityRoad + Send + Sync,
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
            language: None,
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

        // REQ-003 degraded mode: an author with no active Goodreads route cannot
        // reach GR, but their DB-backed series (stubs included) must still be
        // served — the GR cache/fetch leg is skipped, never an error.
        let gr_routes = self.active_goodreads_routes(user_id, author_id).await?;
        let (filtered_entries, raw_entries_opt, fetched_at) = if gr_routes.is_empty() {
            (Vec::new(), None, None)
        } else {
            let cache = self.db.get_series_cache(author_id).await.unwrap_or(None);
            if let Some(cached) = cache {
                (cached.entries, cached.raw_entries, Some(cached.fetched_at))
            } else {
                let raw_entries = self.union_author_series_feeds(&gr_routes).await?;
                let target_language = self.effective_target_language(&author).await;
                let raw_entries = self
                    .classify_series_languages(&author.name, &target_language, raw_entries)
                    .await;
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

        // No active Goodreads route means there is nothing to refresh from. The
        // typed answer sends the user to the picker; nothing here guesses at a
        // Goodreads author by name (FP-013).
        let gr_routes = self.active_goodreads_routes(user_id, author_id).await?;
        if gr_routes.is_empty() {
            return Err(SeriesServiceError::MissingGoodreadsRoute);
        }

        let _ = self.db.delete_series_cache(author_id).await;
        let raw_entries = self.union_author_series_feeds(&gr_routes).await?;
        let target_language = self.effective_target_language(&author).await;
        let raw_entries = self
            .classify_series_languages(&author.name, &target_language, raw_entries)
            .await;
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

        // The series' own detected content language (from GB, computed at
        // fetch/refresh time — see fetch_author_series_pages) is ground
        // truth when known; it overrides whatever the section-wide dropdown
        // sent, so a foreign-only series can't be mis-stamped with the
        // author's default language regardless of what the UI sent (#112).
        let detected_language = cache_entry.language.clone();
        let effective_language = detected_language.clone().or_else(|| req.language.clone());

        let series = self
            .db
            .upsert_series(CreateSeriesDbRequest {
                user_id,
                author_id,
                name: cache_entry.name.clone(),
                gr_key: req.gr_key.clone(),
                monitor_ebook: req.monitor_ebook,
                monitor_audiobook: req.monitor_audiobook,
                monitor_language: effective_language
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
            language: detected_language,
        })
    }

    async fn run_series_monitor_worker(
        &self,
        params: SeriesMonitorWorkerParams,
    ) -> Result<(), SeriesServiceError> {
        let SeriesMonitorWorkerParams {
            cancel,
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

        let all_books =
            fetch_series_roster_pages(&self.fetcher, &series_gr_key, Some(&cancel)).await?;

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
            if let Err(e) = self
                .db
                .update_series_work_count(user_id, series_id, all_books.len() as i32)
                .await
            {
                tracing::warn!(
                    series_id = series_id,
                    count = all_books.len(),
                    "series worker: failed to persist work count: {e}"
                );
            }
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
                match self
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
                    .await
                {
                    Ok(_) => linked += 1,
                    Err(e) => tracing::warn!(
                        work_id = existing.id,
                        series_id = series_id,
                        "series worker: failed to link existing work to series: {e}"
                    ),
                }
                continue;
            }

            let provider_route = (!book.gr_key.trim().is_empty()).then(|| {
                livrarr_domain::identity_layer::ProviderIdentityEvidence {
                    provider: livrarr_domain::identity_layer::IdentityProvider::Goodreads,
                    route: livrarr_domain::identity_layer::RouteKey {
                        provider: livrarr_domain::identity_layer::IdentityProvider::Goodreads,
                        kind: livrarr_domain::identity_layer::RouteKind::GoodreadsBookEdition,
                        value: book.gr_key.clone(),
                    },
                    work_core: livrarr_domain::identity_layer::title_parts_from_provider(
                        book.title.clone(),
                        None,
                    )
                    .ok()
                    .map(|identity_title| {
                        livrarr_domain::identity_layer::ProviderWorkIdentityCore {
                            identity_title,
                            primary_author_id: author.id,
                        }
                    }),
                    provenance: Default::default(),
                }
            });
            let minimum = provider_route.is_none().then(|| {
                livrarr_domain::identity_layer::MinimumWorkEvidence {
                    title: book.title.clone(),
                    authors: vec![author.id],
                }
            });
            let road_request = livrarr_domain::identity_layer::IdentityRoadRequest {
                user_id: author.user_id,
                origin: livrarr_domain::identity_layer::IdentityRoadOrigin::CreationDoor(
                    livrarr_domain::identity_layer::DoorKind::SeriesMonitor,
                ),
                evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                    user_choice: None,
                    owned_files: Vec::new(),
                    provider_identity: provider_route.into_iter().collect(),
                    minimum,
                },
                interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::MachineAlone,
                existing_work_id: None,
            };
            if let Some(road_result) = self.identity_road.settle(road_request).await {
                match road_result {
                    Ok(livrarr_domain::identity_layer::IdentityRoadOutcome::Settled {
                        work_id,
                        created: road_created,
                        ..
                    }) => {
                        if let Err(error) = self
                            .db
                            .link_work_to_series(
                                user_id,
                                LinkWorkToSeriesRequest {
                                    work_id,
                                    series_id,
                                    series_work_count: series.work_count,
                                    series_name: series_name.clone(),
                                    series_position: book.position,
                                    monitor_ebook,
                                    monitor_audiobook,
                                },
                            )
                            .await
                        {
                            tracing::warn!(work_id, series_id, %error, "series worker: failed to link road-settled work");
                        }
                        if road_created {
                            created += 1;
                        } else {
                            linked += 1;
                        }
                        let work_service = self.work_service.clone();
                        tokio::spawn(async move {
                            let _ = work_service.converge_work(user_id, work_id, 3).await;
                        });
                    }
                    Ok(other) => {
                        tracing::warn!(title = %book.title, outcome = ?other, "series identity road did not settle work");
                    }
                    Err(error) => {
                        tracing::warn!(title = %book.title, %error, "series identity road failed");
                    }
                }
                continue;
            }

            // Compatibility construction for narrow legacy unit fixtures. The
            // production composition always injects the shared identity road.
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
                        let work_service = self.work_service.clone();
                        let (uid, wid) = (author.user_id, result.work.id);
                        tokio::spawn(async move {
                            let _ = work_service.converge_work(uid, wid, 3).await;
                        });
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

        // REQ-009: the series-list fetch needs the author's Goodreads link. There
        // is no silent name-similarity adoption any more (FP-013/FP-014) — an
        // author with no active Goodreads route goes to the author-candidate flow,
        // where the user picks.
        if self
            .active_goodreads_routes(user_id, author_id)
            .await?
            .is_empty()
        {
            tracing::debug!(author = %author.name, "promote stub: author has no active Goodreads route");
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
                let books = fetch_series_roster_pages(&self.fetcher, &series.gr_key, None).await?;
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
                if let Err(e) = self
                    .db
                    .update_series_work_count(user_id, series_id, entries.len() as i32)
                    .await
                {
                    tracing::warn!(
                        series_id = series_id,
                        "series books: work-count update failed after roster heal: {e}"
                    );
                }
                entries
            }
        };

        Ok(SeriesBooksView {
            roster_available: true,
            rows: merge_roster_with_works(&entries, linked, &author.name),
        })
    }
}
