use std::time::Duration;

use chrono::Utc;
use livrarr_db::{
    AuthorBibliographyDb, AuthorDb, CreateAuthorDbRequest, UpdateAuthorDbRequest, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

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
    D: AuthorDb + WorkDb + AuthorBibliographyDb + Send + Sync,
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
            let ol_key = match author.ol_key.as_deref() {
                Some(k) => k.to_string(),
                None => {
                    let resolved = self.resolve_ol_key(user_id, &author).await?;
                    resolved
                }
            };

            let raw_entries = self.fetch_ol_bibliography(&ol_key).await?;
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
    D: AuthorDb + WorkDb + AuthorBibliographyDb + Send + Sync,
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

    async fn fetch_ol_bibliography(
        &self,
        ol_key: &str,
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

        let entries = data
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
                            ol_key,
                            title: title.to_string(),
                            year,
                            series_name: None,
                            series_position: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

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
                let ol = if b.ol_key.is_empty() {
                    None
                } else {
                    Some(b.ol_key.clone())
                };
                let already_in_library = works.iter().any(|w| {
                    (!b.ol_key.is_empty() && w.ol_key.as_deref() == Some(&b.ol_key))
                        || b.title.to_lowercase() == w.title.to_lowercase()
                });
                BibliographyEntry {
                    title: b.title,
                    year: b.year,
                    ol_key: ol,
                    series_name: b.series_name,
                    series_position: b.series_position,
                    already_in_library,
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
            system_template: Box::leak(system.to_string().into_boxed_str()),
            user_template: Box::leak(user_template.into_boxed_str()),
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
                if let Some(t) = entry.get("title").and_then(|v| v.as_str()) {
                    e.title = t.to_string();
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
