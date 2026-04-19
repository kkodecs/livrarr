//! ListService implementation — CSV imports from Goodreads and Hardcover.
//!
//! 5-step workflow: preview -> confirm (batched) -> complete -> undo -> list.
//! All business logic lives here. Handlers: validate -> call ONE service -> map result.

use std::time::Duration;

use chrono::Utc;
use tracing::{info, warn};

use livrarr_db::ListImportDb;
use livrarr_domain::services::*;
use livrarr_domain::{ProvenanceSetter, UserId};

use crate::parsers::{self, CsvSource, ParseError};

// ---------------------------------------------------------------------------
// ListServiceImpl
// ---------------------------------------------------------------------------

pub struct ListServiceImpl<D, W, H, B> {
    pub db: D,
    pub work_service: W,
    pub http: H,
    pub bibliography_trigger: B,
}

impl<D, W, H, B> ListServiceImpl<D, W, H, B> {
    pub fn new(db: D, work_service: W, http: H, bibliography_trigger: B) -> Self {
        Self {
            db,
            work_service,
            http,
            bibliography_trigger,
        }
    }
}

// ---------------------------------------------------------------------------
// OL lookup helpers (private)
// ---------------------------------------------------------------------------

impl<D, W, H, B> ListServiceImpl<D, W, H, B>
where
    H: HttpFetcher + Send + Sync,
{
    /// Look up a book on OpenLibrary by ISBN (preferred) or title+author search.
    /// Returns an AddWorkRequest on success, or an error message.
    async fn ol_lookup(
        &self,
        isbn_13: Option<&str>,
        isbn_10: Option<&str>,
        title: &str,
        author: &str,
        year: Option<i32>,
    ) -> Result<AddWorkRequest, String> {
        // Try ISBN lookup first (more precise).
        let isbn = isbn_13.or(isbn_10);
        if let Some(isbn) = isbn {
            if let Some(req) = self.ol_isbn_lookup(isbn).await {
                return Ok(req);
            }
        }

        // Fallback: title + author search.
        self.ol_search(title, author, year).await
    }

    /// OpenLibrary ISBN lookup -> AddWorkRequest.
    async fn ol_isbn_lookup(&self, isbn: &str) -> Option<AddWorkRequest> {
        let url = format!("https://openlibrary.org/isbn/{isbn}.json");
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

        let resp = self.http.fetch(req).await.ok()?;
        if resp.status != 200 {
            return None;
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body).ok()?;

        // ISBN endpoint returns an edition — follow the works link.
        let works_key = data
            .get("works")
            .and_then(|w| w.as_array())
            .and_then(|a| a.first())
            .and_then(|w| w.get("key"))
            .and_then(|k| k.as_str())?;

        let ol_key = works_key.trim_start_matches("/works/").to_string();

        // Fetch the work record for title/author.
        let work_url = format!("https://openlibrary.org{works_key}.json");
        let work_req = FetchRequest {
            url: work_url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(10),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
        };

        let work_resp = self.http.fetch(work_req).await.ok()?;
        let work_data: serde_json::Value = serde_json::from_slice(&work_resp.body).ok()?;

        let title = work_data
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown")
            .to_string();

        // Get author from the work's authors array.
        let author_keys = work_data
            .get("authors")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();

        let (author_name, author_ol_key) = if let Some(first) = author_keys.first() {
            let author_key = first
                .get("author")
                .and_then(|a| a.get("key"))
                .or_else(|| first.get("key"))
                .and_then(|k| k.as_str())
                .unwrap_or("");

            let author_ol = author_key.trim_start_matches("/authors/").to_string();

            // Fetch author name from OL author endpoint.
            let name = if !author_key.is_empty() {
                let author_url = format!("https://openlibrary.org{author_key}.json");
                let author_req = FetchRequest {
                    url: author_url,
                    method: HttpMethod::Get,
                    headers: vec![],
                    body: None,
                    timeout: Duration::from_secs(5),
                    rate_bucket: RateBucket::OpenLibrary,
                    max_body_bytes: 1024 * 1024,
                    anti_bot_check: false,
                    user_agent: UserAgentProfile::Server,
                };
                match self.http.fetch(author_req).await {
                    Ok(resp) => serde_json::from_slice::<serde_json::Value>(&resp.body)
                        .ok()
                        .and_then(|v| v.get("name")?.as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "Unknown".to_string()),
                    Err(_) => "Unknown".to_string(),
                }
            } else {
                "Unknown".to_string()
            };

            (name, Some(author_ol).filter(|s| !s.is_empty()))
        } else {
            ("Unknown".to_string(), None)
        };

        let year = data
            .get("publish_date")
            .and_then(|d| d.as_str())
            .and_then(|d| {
                d.split_whitespace()
                    .find_map(|w| w.parse::<i32>().ok().filter(|&y| y > 1000 && y < 3000))
            });

        let cover_url = data
            .get("covers")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.as_i64())
            .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

        Some(AddWorkRequest {
            ol_key: Some(ol_key),
            title,
            author_name,
            author_ol_key,
            year,
            cover_url,
            metadata_source: None,
            language: None,
            detail_url: None,
            gr_key: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: Some(ProvenanceSetter::Imported),
        })
    }

    /// OpenLibrary search by title + author -> AddWorkRequest.
    async fn ol_search(
        &self,
        title: &str,
        author: &str,
        csv_year: Option<i32>,
    ) -> Result<AddWorkRequest, String> {
        let search_term = format!("{title} {author}");
        let encoded = urlencoding::encode(&search_term);
        let url = format!(
            "https://openlibrary.org/search.json?q={encoded}&limit=5&fields=key,title,author_name,author_key,first_publish_year,cover_i"
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
        };

        let resp = self
            .http
            .fetch(req)
            .await
            .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

        if resp.status != 200 {
            return Err(format!("OpenLibrary returned {}", resp.status));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| format!("OpenLibrary parse error: {e}"))?;

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .ok_or_else(|| "no results from OpenLibrary".to_string())?;

        let doc = docs
            .first()
            .ok_or_else(|| format!("no OpenLibrary results for '{title}' by '{author}'"))?;

        let key = doc
            .get("key")
            .and_then(|k| k.as_str())
            .ok_or_else(|| "missing key in OL result".to_string())?;
        let ol_key = key.trim_start_matches("/works/").to_string();

        let result_title = doc
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or(title)
            .to_string();

        let author_name = doc
            .get("author_name")
            .and_then(|a| a.as_array())
            .and_then(|a| a.first())
            .and_then(|a| a.as_str())
            .unwrap_or(author)
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
            .map(|y| y as i32)
            .or(csv_year);

        let cover_url = doc
            .get("cover_i")
            .and_then(|c| c.as_i64())
            .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

        Ok(AddWorkRequest {
            ol_key: Some(ol_key),
            title: result_title,
            author_name,
            author_ol_key,
            year,
            cover_url,
            metadata_source: None,
            language: None,
            detail_url: None,
            gr_key: None,
            series_name: None,
            series_position: None,
            defer_enrichment: false,
            provenance_setter: Some(ProvenanceSetter::Imported),
        })
    }
}

// ---------------------------------------------------------------------------
// ListService trait implementation
// ---------------------------------------------------------------------------

impl<D, W, H, B> ListService for ListServiceImpl<D, W, H, B>
where
    D: ListImportDb + livrarr_db::WorkDb + Send + Sync,
    W: WorkService + Send + Sync,
    H: HttpFetcher + Send + Sync,
    B: BibliographyTrigger + Send + Sync,
{
    async fn preview(
        &self,
        user_id: UserId,
        bytes: Vec<u8>,
    ) -> Result<ListPreviewResponse, ListServiceError> {
        if bytes.is_empty() {
            return Err(ListServiceError::Parse("uploaded file is empty".into()));
        }
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(ListServiceError::Parse("file too large (max 20MB)".into()));
        }

        // Auto-detect source and parse.
        let stripped = parsers::strip_bom_pub(&bytes);
        let mut rdr = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(stripped);

        let headers = rdr
            .headers()
            .map_err(|e| ListServiceError::Parse(format!("invalid CSV: {e}")))?
            .clone();

        let source = parsers::detect_csv_source(&headers).map_err(|e| match e {
            ParseError::UnknownFormat {
                detected_headers, ..
            } => ListServiceError::Parse(format!(
                "unrecognized CSV format. Detected headers: {}",
                detected_headers.join(", ")
            )),
            other => ListServiceError::Parse(other.to_string()),
        })?;

        let rows = match source {
            CsvSource::Goodreads => parsers::parse_goodreads_csv(&bytes),
            CsvSource::Hardcover => parsers::parse_hardcover_csv(&bytes),
        }
        .map_err(|e| ListServiceError::Parse(e.to_string()))?;

        let source_str = match source {
            CsvSource::Goodreads => "goodreads",
            CsvSource::Hardcover => "hardcover",
        };

        // Generate preview_id.
        let preview_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        // Check local DB for existing works by ISBN and persist preview rows.
        let mut preview_rows = Vec::with_capacity(rows.len());

        for row in &rows {
            let status = if row.title.is_empty() {
                "parse_error"
            } else {
                // Check if work already exists by ISBN.
                let exists = check_work_exists_by_isbn(
                    &self.db,
                    user_id,
                    row.isbn_13.as_deref(),
                    row.isbn_10.as_deref(),
                )
                .await;
                if exists {
                    "already_exists"
                } else {
                    "new"
                }
            };

            // Persist to preview table.
            if let Err(e) = self
                .db
                .insert_list_import_preview_row(
                    &preview_id,
                    user_id,
                    row.row_index as i64,
                    &row.title,
                    &row.author,
                    row.isbn_13.as_deref(),
                    row.isbn_10.as_deref(),
                    row.year,
                    row.status.map(|s| format!("{s:?}")).as_deref(),
                    row.rating,
                    status,
                    source_str,
                    &now,
                )
                .await
            {
                return Err(ListServiceError::Db(e));
            }

            preview_rows.push(ListPreviewRow {
                row_index: row.row_index,
                title: row.title.clone(),
                author: row.author.clone(),
                isbn_13: row.isbn_13.clone(),
                isbn_10: row.isbn_10.clone(),
                year: row.year,
                source_status: row.status.map(|s| format!("{s:?}")),
                source_rating: row.rating,
                preview_status: status.to_string(),
            });
        }

        info!(
            user_id,
            source = source_str,
            rows = preview_rows.len(),
            preview_id = %preview_id,
            "list import preview created"
        );

        Ok(ListPreviewResponse {
            preview_id,
            source: source_str.to_string(),
            total_rows: preview_rows.len(),
            rows: preview_rows,
        })
    }

    async fn confirm(
        &self,
        user_id: UserId,
        preview_id: &str,
        import_id: Option<&str>,
        row_indices: &[usize],
    ) -> Result<ListConfirmResponse, ListServiceError> {
        // Validate preview exists for this user.
        let preview_count = self
            .db
            .count_list_import_previews(preview_id, user_id)
            .await
            .map_err(ListServiceError::Db)?;

        if preview_count == 0 {
            return Err(ListServiceError::Parse(
                "preview not found or expired".into(),
            ));
        }

        // Get or create import record.
        let resolved_import_id = if let Some(id) = import_id {
            // Validate ownership and status.
            let record = self
                .db
                .get_list_import_record(id)
                .await
                .map_err(ListServiceError::Db)?
                .ok_or(ListServiceError::NotFound)?;

            if record.user_id != user_id {
                return Err(ListServiceError::NotFound);
            }
            if record.status != "running" {
                return Err(ListServiceError::Conflict(format!(
                    "import is {}, not running",
                    record.status
                )));
            }
            id.to_string()
        } else {
            // Get source from preview.
            let source = self
                .db
                .get_list_import_source(preview_id, user_id)
                .await
                .map_err(ListServiceError::Db)?;

            // Create new import record.
            let id = uuid::Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            self.db
                .create_list_import_record(&id, user_id, &source, &now)
                .await
                .map_err(ListServiceError::Db)?;
            id
        };

        // Process each requested row.
        let mut results = Vec::with_capacity(row_indices.len());
        let mut works_created: i64 = 0;
        let mut new_author_ids: Vec<i64> = Vec::new();

        for &row_idx in row_indices {
            let row = self
                .db
                .get_list_import_preview_row(preview_id, user_id, row_idx as i64)
                .await
                .map_err(ListServiceError::Db)?;

            let row = match row {
                Some(r) => r,
                None => {
                    results.push(ListConfirmRowResult {
                        row_index: row_idx,
                        status: "add_failed".into(),
                        message: Some("row not found in preview".into()),
                    });
                    continue;
                }
            };

            // OL lookup: ISBN first, fallback to title+author search.
            let lookup_result = self
                .ol_lookup(
                    row.isbn_13.as_deref(),
                    row.isbn_10.as_deref(),
                    &row.title,
                    &row.author,
                    row.year,
                )
                .await;

            let add_req = match lookup_result {
                Ok(req) => req,
                Err(msg) => {
                    results.push(ListConfirmRowResult {
                        row_index: row_idx,
                        status: "lookup_error".into(),
                        message: Some(msg),
                    });
                    continue;
                }
            };

            // Try to add via WorkService.
            match self.work_service.add(user_id, add_req).await {
                Ok(add_result) => {
                    // Tag the work with import_id (explicit, race-free).
                    if let Err(e) = self
                        .db
                        .tag_work_with_import(user_id, add_result.work.id, &resolved_import_id)
                        .await
                    {
                        warn!(
                            user_id,
                            work_id = add_result.work.id,
                            import_id = %resolved_import_id,
                            "tag_work_with_import failed (non-fatal): {e}"
                        );
                    }

                    // Track new authors for bibliography trigger.
                    if add_result.author_created {
                        if let Some(author_id) = add_result.author_id {
                            if !new_author_ids.contains(&author_id) {
                                new_author_ids.push(author_id);
                            }
                        }
                    }

                    works_created += 1;
                    results.push(ListConfirmRowResult {
                        row_index: row_idx,
                        status: "added".into(),
                        message: None,
                    });
                }
                Err(WorkServiceError::AlreadyExists) => {
                    results.push(ListConfirmRowResult {
                        row_index: row_idx,
                        status: "already_exists".into(),
                        message: None,
                    });
                }
                Err(e) => {
                    warn!(row_idx, error = %e, "list import: add_work failed");
                    results.push(ListConfirmRowResult {
                        row_index: row_idx,
                        status: "add_failed".into(),
                        message: Some(format!("{e}")),
                    });
                }
            }
        }

        // Update import counters (non-fatal if this fails).
        if let Err(e) = self
            .db
            .increment_list_import_works_created(&resolved_import_id, works_created)
            .await
        {
            warn!(
                import_id = %resolved_import_id,
                "increment_list_import_works_created failed (non-fatal): {e}"
            );
        }

        // Trigger bibliography for newly created authors.
        for author_id in new_author_ids {
            self.bibliography_trigger.trigger(author_id, user_id);
        }

        info!(
            user_id,
            import_id = %resolved_import_id,
            batch_size = row_indices.len(),
            works_created,
            "list import confirm batch processed"
        );

        Ok(ListConfirmResponse {
            import_id: resolved_import_id,
            results,
        })
    }

    async fn complete(&self, user_id: UserId, import_id: &str) -> Result<(), ListServiceError> {
        let now = Utc::now().to_rfc3339();
        let rows_affected = self
            .db
            .complete_list_import(import_id, user_id, &now)
            .await
            .map_err(ListServiceError::Db)?;

        if rows_affected == 0 {
            return Err(ListServiceError::NotFound);
        }

        info!(user_id, import_id = %import_id, "list import completed");
        Ok(())
    }

    async fn undo(
        &self,
        user_id: UserId,
        import_id: &str,
    ) -> Result<ListUndoResponse, ListServiceError> {
        // Validate import exists and belongs to user.
        let status = self
            .db
            .get_list_import_status_for_user(import_id, user_id)
            .await
            .map_err(ListServiceError::Db)?
            .ok_or(ListServiceError::NotFound)?;

        if status == "undone" {
            return Err(ListServiceError::Conflict("import already undone".into()));
        }

        // Enumerate works created by this import and delete via WorkService.
        let work_ids = self
            .db
            .list_works_by_import(import_id, user_id)
            .await
            .map_err(ListServiceError::Db)?;

        let mut works_removed: usize = 0;
        let mut works_skipped: usize = 0;

        for work_id in &work_ids {
            match self.work_service.delete(user_id, *work_id).await {
                Ok(()) => works_removed += 1,
                Err(e) => {
                    warn!(
                        user_id,
                        work_id,
                        import_id = %import_id,
                        "undo: work delete failed (skipping): {e}"
                    );
                    works_skipped += 1;
                }
            }
        }

        // Mark import as undone.
        if let Err(e) = self.db.mark_list_import_undone(import_id).await {
            warn!(import_id = %import_id, "mark_list_import_undone failed: {e}");
        }

        info!(
            user_id,
            import_id = %import_id,
            works_removed,
            works_skipped,
            "list import undone"
        );

        Ok(ListUndoResponse {
            works_removed,
            works_skipped,
        })
    }

    async fn list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummary>, ListServiceError> {
        let rows = self
            .db
            .list_list_imports(user_id)
            .await
            .map_err(ListServiceError::Db)?;

        let summaries = rows
            .into_iter()
            .map(|r| ListImportSummary {
                id: r.id,
                source: r.source,
                status: r.status,
                started_at: r.started_at,
                completed_at: r.completed_at,
                works_created: r.works_created,
            })
            .collect();

        Ok(summaries)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a work already exists for this user by ISBN-13 or ISBN-10.
async fn check_work_exists_by_isbn<D: ListImportDb>(
    db: &D,
    user_id: i64,
    isbn_13: Option<&str>,
    isbn_10: Option<&str>,
) -> bool {
    if let Some(isbn) = isbn_13 {
        if db
            .work_exists_by_isbn_13(user_id, isbn)
            .await
            .unwrap_or(false)
        {
            return true;
        }
    }
    if let Some(isbn) = isbn_10 {
        if db
            .work_exists_by_isbn_10(user_id, isbn)
            .await
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// No-op BibliographyTrigger for tests
// ---------------------------------------------------------------------------

/// No-op bibliography trigger for unit/behavioral tests.
pub struct NoOpBibliographyTrigger;

impl BibliographyTrigger for NoOpBibliographyTrigger {
    fn trigger(&self, _author_id: i64, _user_id: UserId) {
        // no-op
    }
}
