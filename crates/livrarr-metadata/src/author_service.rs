use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use livrarr_db::{
    AuthorBibliographyDb, AuthorDb, ConfigDb, CreateAuthorDbRequest, UpdateAuthorDbRequest, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

/// Classify a bibliography entry's language from the batched OL author-search
/// map (#112): a presence check, not a single classification.
///
/// Absence of a language tag is NOT evidence of a foreign language — measured
/// live, only ~67-69% of even a well-known English author's own OL Works
/// carry a language tag at all (PO-reported: a real "language unknown" label
/// on the majority of an English author's own catalog reads as broken, not
/// cautious). So a Work with no signal defaults to the target language,
/// matching what the page already assumes; a Work that DOES carry language
/// data, none of which is the target, is genuine foreign-language evidence
/// and stays flagged.
fn classify_ol_language(
    ol_key: Option<&str>,
    lang_map: &HashMap<String, Vec<String>>,
    target_language: &str,
) -> Option<String> {
    let Some(key) = ol_key else {
        return Some(target_language.to_string());
    };
    let Some(raw_langs) = lang_map.get(key) else {
        return Some(target_language.to_string());
    };
    let normalized: Vec<String> = raw_langs
        .iter()
        .map(|l| livrarr_domain::normalize_language(l))
        .collect();
    if normalized.iter().any(|l| l == target_language) {
        Some(target_language.to_string())
    } else {
        normalized.into_iter().next()
    }
}

pub struct AuthorServiceImpl<D, F, L> {
    db: D,
    fetcher: F,
    llm: L,
}

impl<D, F, L> AuthorServiceImpl<D, F, L> {
    pub fn new(db: D, fetcher: F, llm: L) -> Self {
        Self { db, fetcher, llm }
    }
}

impl<D, F, L> AuthorService for AuthorServiceImpl<D, F, L>
where
    D: AuthorDb + WorkDb + AuthorBibliographyDb + ConfigDb + Send + Sync,
    F: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    async fn add(
        &self,
        user_id: UserId,
        req: AddAuthorRequest,
    ) -> Result<AddAuthorResult, AuthorServiceError> {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(AuthorServiceError::Validation {
                field: "name".into(),
                message: "name must not be empty".into(),
            });
        }

        if let Some(existing) = self
            .db
            .find_author_by_name(user_id, &name)
            .await
            .map_err(AuthorServiceError::Db)?
        {
            let updated = self
                .db
                .update_author(
                    user_id,
                    existing.id,
                    UpdateAuthorDbRequest {
                        name: None,
                        sort_name: req.sort_name.map(Some),
                        ol_key: req.ol_key.map(Some),
                        gr_key: None,
                        monitored: None,
                        monitor_new_items: None,
                        monitor_since: None,
                        monitor_language: None,
                    },
                )
                .await
                .map_err(AuthorServiceError::Db)?;
            return Ok(AddAuthorResult::Updated(updated));
        }

        let db_req = CreateAuthorDbRequest {
            user_id,
            name,
            sort_name: req.sort_name,
            ol_key: req.ol_key,
            gr_key: None,
            hc_key: None,
            import_id: None,
        };

        let author = self
            .db
            .create_author(db_req)
            .await
            .map_err(AuthorServiceError::Db)?;
        Ok(AddAuthorResult::Created(author))
    }

    async fn get(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Author, AuthorServiceError> {
        self.db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })
    }

    async fn list(&self, user_id: UserId) -> Result<Vec<Author>, AuthorServiceError> {
        self.db
            .list_authors(user_id)
            .await
            .map_err(AuthorServiceError::Db)
    }

    async fn update(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        req: UpdateAuthorRequest,
    ) -> Result<Author, AuthorServiceError> {
        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })?;

        let will_have_ol_key = req.ol_key.is_some() || author.ol_key.is_some();
        if req.monitored == Some(true) && !will_have_ol_key {
            return Err(AuthorServiceError::Validation {
                field: "monitored".into(),
                message: "cannot monitor author without OL linkage".into(),
            });
        }

        let monitored = req.monitored;
        let monitor_new_items = req.monitor_new_items;
        let mut monitor_since = None;

        if req.monitored == Some(true) && !author.monitored {
            monitor_since = Some(Utc::now());
        }

        let db_req = UpdateAuthorDbRequest {
            name: req.name,
            sort_name: req.sort_name,
            ol_key: req.ol_key,
            gr_key: req.gr_key,
            monitored,
            monitor_new_items,
            monitor_since,
            monitor_language: req.monitor_language,
        };

        self.db
            .update_author(user_id, author_id, db_req)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })
    }

    async fn delete(&self, user_id: UserId, author_id: AuthorId) -> Result<(), AuthorServiceError> {
        self.db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })?;

        self.db
            .delete_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })
    }

    async fn lookup(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError> {
        let url = format!(
            "https://openlibrary.org/search/authors.json?q={}&limit={}",
            urlencoding::encode(query),
            limit
        );
        let req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(10),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 512 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        };
        let resp = self
            .fetcher
            .fetch(req)
            .await
            .map_err(|e| AuthorServiceError::Provider(e.to_string()))?;

        if resp.status != 200 {
            return Err(AuthorServiceError::Provider(format!(
                "OpenLibrary returned {}",
                resp.status
            )));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| AuthorServiceError::Provider(format!("OpenLibrary parse error: {e}")))?;

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(docs
            .iter()
            .filter_map(|doc| {
                let key = doc.get("key")?.as_str()?;
                let name = doc.get("name")?.as_str()?;
                let ol_key = key.trim_start_matches("/authors/").to_string();
                Some(AuthorLookupResult {
                    ol_key,
                    name: name.to_string(),
                    sort_name: None,
                })
            })
            .collect())
    }

    async fn search(
        &self,
        _user_id: UserId,
        query: &str,
    ) -> Result<Vec<Author>, AuthorServiceError> {
        let url = format!(
            "https://openlibrary.org/search/authors.json?q={}&limit=20",
            urlencoding::encode(query)
        );
        let req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(10),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 512 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        };
        let _resp = self
            .fetcher
            .fetch(req)
            .await
            .map_err(|e| AuthorServiceError::Provider(e.to_string()))?;
        // OL search returns JSON with author docs — but this method returns Vec<Author>
        // which doesn't match OL search results. The handler currently uses a separate
        // lookup_ol_authors function that returns AuthorSearchResult, not Author.
        // This trait method signature needs revision in a future IR pass.
        // For now, return empty — the handler still uses the standalone lookup function.
        Ok(vec![])
    }

    async fn bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        raw: bool,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        let author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })?;

        let cached = self.db.get_bibliography(author.id).await.ok().flatten();

        if cached.as_ref().is_none_or(|c| c.entries.is_empty()) {
            let raw_entries = self.fetch_bibliography_entries(&author, user_id).await;

            if !raw_entries.is_empty() {
                let cleaned = self
                    .llm_clean_bibliography(&author.name, &raw_entries)
                    .await
                    .unwrap_or_else(|| raw_entries.clone());
                let llm_changed = cleaned.len() != raw_entries.len();
                let saved = self
                    .db
                    .save_bibliography(
                        author_id,
                        &cleaned,
                        if llm_changed {
                            Some(&raw_entries)
                        } else {
                            None
                        },
                    )
                    .await
                    .map_err(AuthorServiceError::Db)?;
                return self
                    .build_bibliography_result(user_id, author_id, &saved, raw)
                    .await;
            }

            let saved = self
                .db
                .save_bibliography(author_id, &[], None)
                .await
                .map_err(AuthorServiceError::Db)?;
            return self
                .build_bibliography_result(user_id, author_id, &saved, raw)
                .await;
        }

        let cached = cached.unwrap();
        self.build_bibliography_result(user_id, author_id, &cached, raw)
            .await
    }

    async fn refresh_bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        let _author = self
            .db
            .get_author(user_id, author_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthorServiceError::NotFound,
                other => AuthorServiceError::Db(other),
            })?;

        if let Err(e) = self.db.delete_bibliography(author_id).await {
            tracing::warn!("delete_bibliography failed: {e}");
        }

        self.bibliography(user_id, author_id, false).await
    }

    fn spawn_bibliography_refresh(&self, _author_id: i64, _user_id: i64) {
        // Stub — server wires this up via the concrete AppState spawn
    }

    async fn lookup_authors(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<
        Vec<livrarr_domain::services::AuthorLookupResult>,
        livrarr_domain::services::AuthorServiceError,
    > {
        self.lookup(query, limit).await
    }
}

// Private helper methods
impl<D, F, L> AuthorServiceImpl<D, F, L>
where
    D: AuthorDb + WorkDb + AuthorBibliographyDb + ConfigDb + Send + Sync,
    F: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    async fn resolve_ol_key(
        &self,
        user_id: UserId,
        author: &Author,
    ) -> Result<String, AuthorServiceError> {
        let results = self.lookup(&author.name, 5).await?;
        let best = results.first().ok_or_else(|| {
            AuthorServiceError::Provider(format!(
                "No OpenLibrary match for author '{}'",
                author.name
            ))
        })?;
        let ol_key = best.ol_key.clone();
        let _ = self
            .db
            .update_author(
                user_id,
                author.id,
                UpdateAuthorDbRequest {
                    name: None,
                    sort_name: None,
                    ol_key: Some(Some(ol_key.clone())),
                    gr_key: None,
                    monitored: None,
                    monitor_new_items: None,
                    monitor_since: None,
                    monitor_language: None,
                },
            )
            .await;
        tracing::info!(
            author_id = author.id,
            %ol_key,
            "auto-resolved OL key for '{}'", author.name
        );
        Ok(ol_key)
    }

    async fn fetch_bibliography_entries(
        &self,
        author: &Author,
        user_id: UserId,
    ) -> Vec<livrarr_db::BibliographyEntry> {
        // "Author's language" for #112 classification: their own monitor
        // setting if configured, else the install default — same resolution
        // order as insight #53's dominant_language/suggested_language.
        let target_language = match &author.monitor_language {
            Some(lang) => lang.clone(),
            None => self
                .db
                .get_default_language()
                .await
                .unwrap_or_else(|_| "en".to_string()),
        };

        // Try OL first
        let ol_result = async {
            let ol_key = match author.ol_key.as_deref() {
                Some(k) => k.to_string(),
                None => self.resolve_ol_key(user_id, author).await?,
            };
            self.fetch_ol_bibliography(&ol_key, &target_language).await
        }
        .await;

        match ol_result {
            Ok(entries) if !entries.is_empty() => return entries,
            Ok(_) => {
                tracing::info!(author = %author.name, "OL bibliography empty, trying GB");
            }
            Err(e) => {
                tracing::warn!(author = %author.name, "OL bibliography failed ({e}), trying GB");
            }
        }

        // Fallback to Google Books
        match self
            .fetch_gb_bibliography(&author.name, &target_language)
            .await
        {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(author = %author.name, "GB bibliography also failed: {e}");
                vec![]
            }
        }
    }

    async fn fetch_ol_bibliography(
        &self,
        ol_key: &str,
        target_language: &str,
    ) -> Result<Vec<livrarr_db::BibliographyEntry>, AuthorServiceError> {
        let url = format!("https://openlibrary.org/authors/{ol_key}/works.json?limit=100");
        let req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(10),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        };

        let resp = self
            .fetcher
            .fetch(req)
            .await
            .map_err(|e| AuthorServiceError::Provider(format!("OL request failed: {e}")))?;

        if resp.status != 200 {
            return Err(AuthorServiceError::Provider(format!(
                "OL returned {}",
                resp.status
            )));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| AuthorServiceError::Provider(format!("OL parse: {e}")))?;

        let mut entries: Vec<livrarr_db::BibliographyEntry> = data
            .get("entries")
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|doc| {
                        let title = doc.get("title")?.as_str()?;
                        let key = doc.get("key")?.as_str()?;
                        let ol_key = key.trim_start_matches("/works/").to_string();
                        let year = doc
                            .get("first_publish_date")
                            .and_then(|d| d.as_str())
                            .and_then(|s| s.get(..4))
                            .and_then(|y| y.parse().ok());
                        Some(livrarr_db::BibliographyEntry {
                            ol_key: Some(ol_key),
                            title: title.to_string(),
                            year,
                            series_name: None,
                            series_position: None,
                            language: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // #112: classify each entry's language via a batched OL search — a
        // presence check ("does this Work have a target-language edition"),
        // not a single classification, so a merged multi-language OL Work
        // stays visible as long as one of its editions is in the target
        // language. Best-effort: any failure just leaves entries Unknown
        // (never blocks the bibliography from displaying).
        let lang_map = self.fetch_ol_author_languages(ol_key).await;
        if !lang_map.is_empty() {
            for entry in &mut entries {
                entry.language =
                    classify_ol_language(entry.ol_key.as_deref(), &lang_map, target_language);
            }
        }

        Ok(entries)
    }

    /// Batched OL search — returns bare work key -> raw OL language codes
    /// (3-letter, e.g. "eng","spa") for every work by this OL author key.
    /// Paginates via `offset` until `numFound` is exhausted. The page cap is
    /// a defensive error guard, not a normal path: hitting it logs a warning
    /// rather than silently treating overflow entries as ordinary Unknown
    /// (#112 review round 1 — a fixed single page silently re-creates the
    /// leak for any author with 100+ works).
    async fn fetch_ol_author_languages(&self, ol_author_key: &str) -> HashMap<String, Vec<String>> {
        const PAGE_SIZE: usize = 100;
        const MAX_PAGES: usize = 10;

        let mut map = HashMap::new();
        let mut offset = 0usize;
        let mut num_found: Option<usize> = None;

        for page in 0..MAX_PAGES {
            let url = format!(
                "https://openlibrary.org/search.json?q=author_key:{ol_author_key}&fields=key,title,language&limit={PAGE_SIZE}&offset={offset}"
            );
            let req = FetchRequest {
                url,
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(10),
                rate_bucket: RateBucket::OpenLibrary,
                max_body_bytes: 2 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
                priority: RequestPriority::Normal,
            };

            let resp = match self.fetcher.fetch(req).await {
                Ok(r) if r.status == 200 => r,
                Ok(r) => {
                    tracing::warn!(
                        ol_author_key,
                        status = r.status,
                        "OL author-language search returned non-200; stopping pagination"
                    );
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        ol_author_key,
                        "OL author-language search failed ({e}); stopping pagination"
                    );
                    break;
                }
            };

            let data: serde_json::Value = match serde_json::from_slice(&resp.body) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(ol_author_key, "OL author-language search parse failed: {e}");
                    break;
                }
            };

            if num_found.is_none() {
                num_found = data
                    .get("numFound")
                    .and_then(|n| n.as_u64())
                    .map(|n| n as usize);
            }

            let docs = data
                .get("docs")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            let page_len = docs.len();

            for doc in &docs {
                let Some(key) = doc.get("key").and_then(|k| k.as_str()) else {
                    continue;
                };
                let bare_key = key.trim_start_matches("/works/").to_string();
                let langs: Vec<String> = doc
                    .get("language")
                    .and_then(|l| l.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if !langs.is_empty() {
                    map.insert(bare_key, langs);
                }
            }

            offset += PAGE_SIZE;
            let exhausted = page_len < PAGE_SIZE || num_found.is_some_and(|n| offset >= n);
            if exhausted {
                break;
            }
            if page == MAX_PAGES - 1 {
                tracing::warn!(
                    ol_author_key,
                    num_found = ?num_found,
                    pages_fetched = MAX_PAGES,
                    "OL author-language search hit the defensive page cap; \
                     some works may be unclassified (Unknown)"
                );
            }
        }

        map
    }

    async fn fetch_gb_bibliography(
        &self,
        author_name: &str,
        target_language: &str,
    ) -> Result<Vec<livrarr_db::BibliographyEntry>, AuthorServiceError> {
        let api_key = match self.db.get_metadata_config().await {
            Ok(cfg) => match cfg
                .google_books_api_key
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                Some(k) => k.to_string(),
                None => {
                    return Err(AuthorServiceError::Provider(
                        "no Google Books API key configured".into(),
                    ));
                }
            },
            Err(e) => {
                return Err(AuthorServiceError::Db(e));
            }
        };

        let query = format!("inauthor:\"{}\"", author_name);
        let url = format!(
            "https://www.googleapis.com/books/v1/volumes?q={}&maxResults=40",
            urlencoding::encode(&query),
        );

        // Interactive: `bibliography()` is a synchronous, user-facing lookup
        // (the author bibliography page), not a background scan.
        let volumes = livrarr_external_data::google_books::fetch_gb_volumes(
            &self.fetcher,
            &api_key,
            url,
            RequestPriority::Interactive,
        )
        .await
        .map_err(AuthorServiceError::Provider)?;

        let entries: Vec<livrarr_db::BibliographyEntry> = volumes
            .iter()
            .filter_map(|vol| {
                let vi = vol.volume_info.as_ref()?;
                let title = vi.title.as_ref()?.clone();
                let year = vi
                    .published_date
                    .as_deref()
                    .and_then(|d| d.get(..4))
                    .and_then(|y| y.parse::<i32>().ok());

                Some(livrarr_db::BibliographyEntry {
                    ol_key: None,
                    title,
                    year,
                    series_name: None,
                    series_position: None,
                    // No language on this volume isn't evidence of foreign —
                    // same "no signal ⇒ assume target" rule as the OL path.
                    language: Some(
                        vi.language
                            .as_deref()
                            .map(livrarr_domain::normalize_language)
                            .unwrap_or_else(|| target_language.to_string()),
                    ),
                })
            })
            .collect();

        tracing::info!(
            author = %author_name,
            count = entries.len(),
            "GB bibliography fetched"
        );

        Ok(entries)
    }

    async fn build_bibliography_result(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        cached: &livrarr_db::AuthorBibliography,
        raw: bool,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        let source = if raw {
            cached.raw_entries.as_deref().unwrap_or(&cached.entries)
        } else {
            &cached.entries
        };
        let entries = self
            .enrich_bibliography(user_id, author_id, source.to_vec())
            .await;
        Ok(BibliographyResult {
            filtered_count: cached.entries.len(),
            raw_count: cached
                .raw_entries
                .as_ref()
                .map_or(cached.entries.len(), |r| r.len()),
            raw_available: cached.raw_entries.is_some(),
            fetched_at: cached.fetched_at.clone(),
            entries,
        })
    }

    async fn enrich_bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        db_entries: Vec<livrarr_db::BibliographyEntry>,
    ) -> Vec<BibliographyEntry> {
        let works = self
            .db
            .list_works_by_author(user_id, author_id)
            .await
            .unwrap_or_default();

        db_entries
            .into_iter()
            .map(|b| {
                let bib_norm = livrarr_matching::work_dedup::normalize_title_for_match(&b.title);
                let already_in_library = works.iter().any(|w| {
                    (b.ol_key.is_some() && w.ol_key.as_deref() == b.ol_key.as_deref())
                        || livrarr_matching::work_dedup::normalize_title_for_match(&w.title)
                            == bib_norm
                });
                BibliographyEntry {
                    title: b.title,
                    year: b.year,
                    ol_key: b.ol_key,
                    series_name: b.series_name,
                    series_position: b.series_position,
                    already_in_library,
                    language: b.language,
                }
            })
            .collect()
    }

    async fn llm_clean_bibliography(
        &self,
        author_name: &str,
        entries: &[livrarr_db::BibliographyEntry],
    ) -> Option<Vec<livrarr_db::BibliographyEntry>> {
        use std::collections::HashMap;

        let mut listing = String::new();
        for (i, e) in entries.iter().enumerate() {
            listing.push_str(&format!(
                "{}: \"{}\" ({})\n",
                i,
                e.title,
                e.year.map(|y| y.to_string()).unwrap_or_default(),
            ));
        }

        let system = "You are a librarian assistant. Clean up book bibliography lists.";
        let user_template = format!(
            "These are works attributed to \"{author_name}\" from a book database:\n\n\
             {listing}\n\
             Clean up this list:\n\
             1. REMOVE works by a different person who shares the same name (e.g. a 16th-century playwright vs a modern author)\n\
             2. Remove duplicates, foreign-language editions of the same work, comic adaptations, anthologies, and compilations\n\
             3. Fix spelling and capitalization\n\
             4. Add series name and position if you know it\n\
             5. Order by series first (in reading order), then standalone works by publication year\n\n\
             Return a JSON array. Each entry: {{\"idx\": <original index>, \"title\": \"<cleaned title>\", \
             \"series\": \"<series name or null>\", \"position\": <number or null>}}\n\
             Return ONLY the JSON array, no other text."
        );

        let mut context = HashMap::new();
        context.insert(LlmField::AuthorName, LlmValue::Text(author_name.into()));
        context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing.clone()));

        let req = LlmCallRequest {
            system_template: system.to_string(),
            user_template,
            context,
            allowed_fields: &[LlmField::AuthorName, LlmField::BibliographyHtml],
            timeout: Duration::from_secs(30),
            purpose: LlmPurpose::BibliographyCleanup,
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

        let llm_entries: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

        let cleaned: Vec<livrarr_db::BibliographyEntry> = llm_entries
            .iter()
            .filter_map(|entry| {
                let idx = entry.get("idx")?.as_u64()? as usize;
                if idx >= entries.len() {
                    return None;
                }
                let mut e = entries[idx].clone();
                // Fidelity guard (#53): the LLM is asked only to fix spelling
                // and capitalization (instruction 3 above), not rewrite the
                // work. A garbled or wrong-book replacement (e.g. mixing up
                // two same-author titles) scores low here and the original
                // scraped title is kept instead of the LLM's output. 0.75
                // mirrors `identity_matching::TITLE_GREY_FLOOR`, the
                // established "still plausibly the same title" bar used
                // elsewhere in the codebase.
                if let Some(t) = entry.get("title").and_then(|v| v.as_str()) {
                    let candidate = t.trim();
                    if !candidate.is_empty()
                        && livrarr_matching::string_similarity(&e.title, candidate) >= 0.75
                    {
                        e.title = candidate.to_string();
                    }
                }
                e.series_name = entry
                    .get("series")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                e.series_position = entry.get("position").and_then(|v| v.as_f64());
                Some(e)
            })
            .collect();

        if cleaned.is_empty() {
            return None;
        }

        Some(cleaned)
    }
}
