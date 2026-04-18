use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;

use livrarr_db::ListImportDb;
use livrarr_domain::services::*;
use livrarr_domain::{UserId, WorkId};

// ---------------------------------------------------------------------------
// Internal preview row — stored in memory, separate from domain type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct InternalPreviewRow {
    title: String,
    author: Option<String>,
    isbn: Option<String>,
    matched_work_id: Option<WorkId>,
    match_status: ListMatchStatus,
}

#[derive(Debug, Clone)]
struct PreviewState {
    user_id: UserId,
    source: ListSource,
    rows: Vec<InternalPreviewRow>,
}

// ---------------------------------------------------------------------------
// Parsed row from CSV/JSON input
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ParsedRow {
    title: String,
    author: Option<String>,
    isbn: Option<String>,
}

// ---------------------------------------------------------------------------
// ListServiceImpl
// ---------------------------------------------------------------------------

pub struct ListServiceImpl<D, W> {
    pub db: D,
    pub work_service: W,
    previews: Arc<RwLock<HashMap<String, PreviewState>>>,
}

impl<D, W> ListServiceImpl<D, W> {
    pub fn new(db: D, work_service: W) -> Self {
        Self {
            db,
            work_service,
            previews: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<D, W> ListService for ListServiceImpl<D, W>
where
    D: ListImportDb + livrarr_db::WorkDb + Send + Sync,
    W: WorkService + Send + Sync,
{
    async fn preview(
        &self,
        user_id: UserId,
        req: ListPreviewRequest,
    ) -> Result<ListPreviewResponse, ListServiceError> {
        if req.content.trim().is_empty() {
            return Err(ListServiceError::Parse("content is empty".into()));
        }

        let parsed = match req.source {
            ListSource::GoodreadsCsv => parse_goodreads_csv(&req.content)?,
            ListSource::OpenLibrary => parse_openlibrary_json(&req.content)?,
        };

        let import_id = uuid::Uuid::new_v4().to_string();

        // Pre-fetch existing works for matching.
        let existing_works = self
            .db
            .list_works(user_id)
            .await
            .map_err(ListServiceError::Db)?;

        // Build preview rows by checking existing works.
        let mut internal_rows = Vec::with_capacity(parsed.len());
        for p in parsed {
            let (match_status, matched_work_id) = if p.title.is_empty() {
                (ListMatchStatus::NotFound, None)
            } else {
                // Check by ISBN first.
                let isbn_exists = if let Some(ref isbn) = p.isbn {
                    self.db
                        .work_exists_by_isbn_13(user_id, isbn)
                        .await
                        .unwrap_or(false)
                } else {
                    false
                };

                if isbn_exists {
                    (ListMatchStatus::AlreadyExists, None)
                } else {
                    // Fallback: check by normalized title.
                    let normalized_title = p.title.trim().to_lowercase();
                    let existing = existing_works
                        .iter()
                        .find(|w| w.title.trim().to_lowercase() == normalized_title);
                    if let Some(w) = existing {
                        (ListMatchStatus::AlreadyExists, Some(w.id))
                    } else if p.isbn.is_some() || !p.title.is_empty() {
                        (ListMatchStatus::Matched, None)
                    } else {
                        (ListMatchStatus::NotFound, None)
                    }
                }
            };

            internal_rows.push(InternalPreviewRow {
                title: p.title,
                author: p.author,
                isbn: p.isbn,
                matched_work_id,
                match_status,
            });
        }

        // Build response rows.
        let response_rows: Vec<ListPreviewRow> = internal_rows
            .iter()
            .map(|r| ListPreviewRow {
                title: r.title.clone(),
                author: r.author.clone(),
                isbn: r.isbn.clone(),
                matched_work_id: r.matched_work_id,
                match_status: r.match_status,
            })
            .collect();

        // Store preview state.
        let state = PreviewState {
            user_id,
            source: req.source,
            rows: internal_rows,
        };
        self.previews.write().await.insert(import_id.clone(), state);

        Ok(ListPreviewResponse {
            rows: response_rows,
            import_id,
        })
    }

    async fn confirm(
        &self,
        user_id: UserId,
        import_id: &str,
    ) -> Result<ListConfirmResponse, ListServiceError> {
        // Single-use: remove preview state.
        let state = self
            .previews
            .write()
            .await
            .remove(import_id)
            .ok_or(ListServiceError::NotFound)?;

        if state.user_id != user_id {
            return Err(ListServiceError::NotFound);
        }

        // Create import record in DB.
        let source_str = match state.source {
            ListSource::GoodreadsCsv => "goodreads",
            ListSource::OpenLibrary => "openlibrary",
        };
        let now = Utc::now().to_rfc3339();
        let _ = self
            .db
            .create_list_import_record(import_id, user_id, source_str, &now)
            .await;

        let mut added: usize = 0;
        let mut skipped: usize = 0;
        let mut failed: Vec<ListFailedRow> = Vec::new();

        for row in &state.rows {
            if row.match_status == ListMatchStatus::AlreadyExists {
                skipped += 1;
                continue;
            }

            let add_req = AddWorkRequest {
                title: row.title.clone(),
                author_name: row.author.clone().unwrap_or_default(),
                author_ol_key: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                language: None,
                detail_url: None,
                series_name: None,
                series_position: None,
                defer_enrichment: false,
            };

            match self.work_service.add(user_id, add_req).await {
                Ok(_work) => {
                    let _ = self.db.tag_last_work_with_import(import_id, user_id).await;
                    added += 1;
                }
                Err(WorkServiceError::AlreadyExists) => {
                    skipped += 1;
                }
                Err(e) => {
                    failed.push(ListFailedRow {
                        title: row.title.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Update import record.
        let _ = self
            .db
            .increment_list_import_works_created(import_id, added as i64)
            .await;
        let completed_at = Utc::now().to_rfc3339();
        let _ = self
            .db
            .complete_list_import(import_id, user_id, &completed_at)
            .await;

        Ok(ListConfirmResponse {
            added,
            skipped,
            failed,
        })
    }

    async fn undo(&self, user_id: UserId, import_id: &str) -> Result<usize, ListServiceError> {
        let status = self
            .db
            .get_list_import_status_for_user(import_id, user_id)
            .await
            .map_err(ListServiceError::Db)?
            .ok_or(ListServiceError::NotFound)?;

        if status == "undone" {
            return Err(ListServiceError::NotFound);
        }

        let deleted = self
            .db
            .delete_works_by_list_import(import_id, user_id)
            .await
            .map_err(ListServiceError::Db)?;

        if deleted == 0 {
            return Err(ListServiceError::NotFound);
        }

        let _ = self.db.mark_list_import_undone(import_id).await;

        Ok(deleted as usize)
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
            .map(|r| {
                let source = match r.source.as_str() {
                    "goodreads" => ListSource::GoodreadsCsv,
                    _ => ListSource::OpenLibrary,
                };
                ListImportSummary {
                    import_id: r.id,
                    source,
                    added_count: r.works_created as usize,
                    skipped_count: 0,
                    failed_count: 0,
                    created_at: chrono::DateTime::parse_from_rfc3339(&r.started_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                }
            })
            .collect();

        Ok(summaries)
    }
}

// ---------------------------------------------------------------------------
// CSV / JSON Parsers
// ---------------------------------------------------------------------------

fn parse_goodreads_csv(content: &str) -> Result<Vec<ParsedRow>, ListServiceError> {
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers = rdr
        .headers()
        .map_err(|e| ListServiceError::Parse(format!("invalid CSV headers: {e}")))?
        .clone();

    let find_col = |name: &str| -> Option<usize> {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };

    let title_idx = find_col("Title")
        .ok_or_else(|| ListServiceError::Parse("missing 'Title' column".into()))?;
    let author_idx = find_col("Author").or_else(|| find_col("Author l-f"));
    let isbn_idx = find_col("ISBN13").or_else(|| find_col("ISBN"));

    let mut rows = Vec::new();
    for result in rdr.records() {
        let record = result.map_err(|e| ListServiceError::Parse(format!("CSV row error: {e}")))?;
        let title = record
            .get(title_idx)
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        let author = author_idx.and_then(|i| {
            let v = record.get(i).unwrap_or("").trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        });
        let isbn = isbn_idx.and_then(|i| {
            let v = record
                .get(i)
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .trim_matches('=')
                .trim_matches('"')
                .to_string();
            if v.is_empty() || v == "\"\"" {
                None
            } else {
                Some(v)
            }
        });

        if !title.is_empty() {
            rows.push(ParsedRow {
                title,
                author,
                isbn,
            });
        }
    }

    if rows.is_empty() {
        return Err(ListServiceError::Parse("no valid rows found".into()));
    }

    Ok(rows)
}

fn parse_openlibrary_json(content: &str) -> Result<Vec<ParsedRow>, ListServiceError> {
    let items: Vec<serde_json::Value> = serde_json::from_str(content)
        .map_err(|e| ListServiceError::Parse(format!("invalid JSON: {e}")))?;

    let mut rows = Vec::new();
    for item in &items {
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let author = item
            .get("author")
            .or_else(|| item.get("author_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let isbn = item
            .get("isbn_13")
            .or_else(|| item.get("isbn"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if !title.is_empty() {
            rows.push(ParsedRow {
                title,
                author,
                isbn,
            });
        }
    }

    if rows.is_empty() {
        return Err(ListServiceError::Parse("no valid items found".into()));
    }

    Ok(rows)
}
