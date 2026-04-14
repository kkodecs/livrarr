//! List import handler — CSV imports from Goodreads and Hardcover.
//!
//! Two-step flow: preview (parse + local check) → confirm (OL lookup + add, batched).
//! Undo deletes works created by the import.

use axum::extract::{Multipart, Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tracing::{info, warn};

use crate::handlers::work::add_work_internal;
use crate::parsers::{self, CsvSource, ImportStatus, ParseError};
use crate::state::AppState;
use crate::{AddWorkRequest, ApiError, AuthContext};

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResponse {
    pub preview_id: String,
    pub source: String,
    pub total_rows: usize,
    pub rows: Vec<PreviewRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    pub row_index: usize,
    pub title: String,
    pub author: String,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub year: Option<i32>,
    pub source_status: Option<ImportStatus>,
    pub source_rating: Option<f32>,
    pub preview_status: String, // "new", "already_exists", "parse_error"
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmRequest {
    pub preview_id: String,
    pub row_indices: Vec<usize>,
    pub import_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmResponse {
    pub import_id: String,
    pub results: Vec<ConfirmRowResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmRowResult {
    pub row_index: usize,
    pub status: String, // "added", "already_exists", "add_failed", "lookup_error"
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResponse {
    pub works_removed: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub id: String,
    pub source: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub works_created: i64,
}

// ---------------------------------------------------------------------------
// POST /api/v1/listimport/preview
// ---------------------------------------------------------------------------

pub async fn preview(
    State(state): State<AppState>,
    ctx: AuthContext,
    mut multipart: Multipart,
) -> Result<Json<PreviewResponse>, ApiError> {
    let user_id = ctx.user.id;

    // Read the first uploaded field.
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| ApiError::BadRequest("no file uploaded".into()))?;

    let bytes = field
        .bytes()
        .await
        .map_err(|e| ApiError::BadRequest(format!("failed to read upload: {e}")))?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(ApiError::BadRequest("file too large (max 20MB)".into()));
    }
    let bytes = bytes.to_vec();
    if bytes.is_empty() {
        return Err(ApiError::BadRequest("uploaded file is empty".into()));
    }

    // Auto-detect source and parse.
    let stripped = parsers::strip_bom_pub(&bytes);
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(stripped);

    let headers = rdr
        .headers()
        .map_err(|e| ApiError::BadRequest(format!("invalid CSV: {e}")))?
        .clone();

    let source = parsers::detect_csv_source(&headers).map_err(|e| match e {
        ParseError::UnknownFormat {
            detected_headers, ..
        } => ApiError::BadRequest(format!(
            "unrecognized CSV format. Detected headers: {}",
            detected_headers.join(", ")
        )),
        other => ApiError::BadRequest(other.to_string()),
    })?;

    let rows = match source {
        CsvSource::Goodreads => parsers::parse_goodreads_csv(&bytes),
        CsvSource::Hardcover => parsers::parse_hardcover_csv(&bytes),
    }
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let source_str = match source {
        CsvSource::Goodreads => "goodreads",
        CsvSource::Hardcover => "hardcover",
    };

    // Generate preview_id.
    let preview_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Check local DB for existing works by ISBN.
    let mut preview_rows = Vec::with_capacity(rows.len());

    for row in &rows {
        let status = if row.title.is_empty() {
            "parse_error"
        } else {
            // Check if work already exists by ISBN.
            let exists = check_work_exists_by_isbn(
                &state,
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
        sqlx::query(
            "INSERT INTO list_import_previews \
             (preview_id, user_id, row_index, title, author, isbn_13, isbn_10, year, \
              source_status, source_rating, preview_status, source, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&preview_id)
        .bind(user_id)
        .bind(row.row_index as i64)
        .bind(&row.title)
        .bind(&row.author)
        .bind(&row.isbn_13)
        .bind(&row.isbn_10)
        .bind(row.year)
        .bind(row.status.map(|s| format!("{s:?}")))
        .bind(row.rating)
        .bind(status)
        .bind(source_str)
        .bind(&now)
        .execute(state.db.pool())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to persist preview: {e}")))?;

        preview_rows.push(PreviewRow {
            row_index: row.row_index,
            title: row.title.clone(),
            author: row.author.clone(),
            isbn_13: row.isbn_13.clone(),
            isbn_10: row.isbn_10.clone(),
            year: row.year,
            source_status: row.status,
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

    Ok(Json(PreviewResponse {
        preview_id,
        source: source_str.to_string(),
        total_rows: preview_rows.len(),
        rows: preview_rows,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/listimport/confirm
// ---------------------------------------------------------------------------

pub async fn confirm(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<ConfirmResponse>, ApiError> {
    let user_id = ctx.user.id;

    // Validate preview exists for this user.
    let preview_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM list_import_previews WHERE preview_id = ? AND user_id = ?",
    )
    .bind(&req.preview_id)
    .bind(user_id)
    .fetch_one(state.db.pool())
    .await
    .map_err(|e| ApiError::Internal(format!("preview lookup failed: {e}")))?;

    if preview_count == 0 {
        return Err(ApiError::BadRequest("preview not found or expired".into()));
    }

    // Get or create import record.
    let import_id = if let Some(ref id) = req.import_id {
        // Validate ownership and status.
        let row = sqlx::query("SELECT user_id, status FROM imports WHERE id = ?")
            .bind(id)
            .fetch_optional(state.db.pool())
            .await
            .map_err(|e| ApiError::Internal(format!("import lookup failed: {e}")))?
            .ok_or_else(|| ApiError::NotFound)?;

        let owner: i64 = row.try_get("user_id").unwrap_or(0);
        let status: String = row.try_get("status").unwrap_or_default();
        if owner != user_id {
            return Err(ApiError::Forbidden);
        }
        if status != "running" {
            return Err(ApiError::Conflict {
                reason: format!("import is {status}, not running"),
            });
        }
        id.clone()
    } else {
        // Get source from preview.
        let source: String = sqlx::query_scalar(
            "SELECT source FROM list_import_previews WHERE preview_id = ? AND user_id = ? LIMIT 1",
        )
        .bind(&req.preview_id)
        .bind(user_id)
        .fetch_one(state.db.pool())
        .await
        .map_err(|e| ApiError::Internal(format!("source lookup failed: {e}")))?;

        // Create new import record.
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO imports (id, user_id, source, status, started_at) VALUES (?, ?, ?, 'running', ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(&source)
        .bind(&now)
        .execute(state.db.pool())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to create import: {e}")))?;

        id
    };

    // Process each requested row.
    let mut results = Vec::with_capacity(req.row_indices.len());
    let mut works_created: i64 = 0;

    for &row_idx in &req.row_indices {
        let row = sqlx::query(
            "SELECT * FROM list_import_previews \
             WHERE preview_id = ? AND user_id = ? AND row_index = ?",
        )
        .bind(&req.preview_id)
        .bind(user_id)
        .bind(row_idx as i64)
        .fetch_optional(state.db.pool())
        .await
        .map_err(|e| ApiError::Internal(format!("row fetch failed: {e}")))?;

        let row = match row {
            Some(r) => r,
            None => {
                results.push(ConfirmRowResult {
                    row_index: row_idx,
                    status: "add_failed".into(),
                    message: Some("row not found in preview".into()),
                });
                continue;
            }
        };

        let title: String = row.try_get("title").unwrap_or_default();
        let author: String = row.try_get("author").unwrap_or_default();
        let isbn_13: Option<String> = row.try_get("isbn_13").ok();
        let isbn_10: Option<String> = row.try_get("isbn_10").ok();
        let year: Option<i32> = row.try_get("year").ok().flatten();

        // OL lookup: ISBN first, fallback to title+author search.
        let lookup_result = ol_lookup(
            &state,
            isbn_13.as_deref(),
            isbn_10.as_deref(),
            &title,
            &author,
            year,
        )
        .await;

        let add_req = match lookup_result {
            Ok(req) => req,
            Err(msg) => {
                results.push(ConfirmRowResult {
                    row_index: row_idx,
                    status: "lookup_error".into(),
                    message: Some(msg),
                });
                continue;
            }
        };

        // Try to add via existing pipeline.
        match add_work_internal(&state, user_id, add_req).await {
            Ok(_response) => {
                // Tag the newly created work with import_id.
                // The work was just created — find it by ol_key.
                let _ = sqlx::query(
                    "UPDATE works SET import_id = ? WHERE user_id = ? AND id = \
                     (SELECT id FROM works WHERE user_id = ? ORDER BY id DESC LIMIT 1)",
                )
                .bind(&import_id)
                .bind(user_id)
                .bind(user_id)
                .execute(state.db.pool())
                .await;

                works_created += 1;
                results.push(ConfirmRowResult {
                    row_index: row_idx,
                    status: "added".into(),
                    message: None,
                });
            }
            Err(ApiError::Conflict { .. }) => {
                results.push(ConfirmRowResult {
                    row_index: row_idx,
                    status: "already_exists".into(),
                    message: None,
                });
            }
            Err(e) => {
                warn!(row_idx, error = %e, "list import: add_work failed");
                results.push(ConfirmRowResult {
                    row_index: row_idx,
                    status: "add_failed".into(),
                    message: Some(format!("{e}")),
                });
            }
        }
    }

    // Update import counters.
    let _ = sqlx::query("UPDATE imports SET works_created = works_created + ? WHERE id = ?")
        .bind(works_created)
        .bind(&import_id)
        .execute(state.db.pool())
        .await;

    info!(
        user_id,
        import_id = %import_id,
        batch_size = req.row_indices.len(),
        works_created,
        "list import confirm batch processed"
    );

    Ok(Json(ConfirmResponse { import_id, results }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/listimport/{import_id}/complete
// ---------------------------------------------------------------------------

pub async fn complete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = chrono::Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE imports SET status = 'completed', completed_at = ? \
         WHERE id = ? AND user_id = ? AND status = 'running'",
    )
    .bind(&now)
    .bind(&import_id)
    .bind(ctx.user.id)
    .execute(state.db.pool())
    .await
    .map_err(|e| ApiError::Internal(format!("complete failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    info!(user_id = ctx.user.id, import_id = %import_id, "list import completed");

    Ok(Json(serde_json::json!({ "status": "completed" })))
}

// ---------------------------------------------------------------------------
// DELETE /api/v1/listimport/{import_id}
// ---------------------------------------------------------------------------

pub async fn undo(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(import_id): Path<String>,
) -> Result<Json<UndoResponse>, ApiError> {
    let user_id = ctx.user.id;

    // Validate import exists and belongs to user.
    let row = sqlx::query("SELECT status FROM imports WHERE id = ? AND user_id = ?")
        .bind(&import_id)
        .bind(user_id)
        .fetch_optional(state.db.pool())
        .await
        .map_err(|e| ApiError::Internal(format!("import lookup failed: {e}")))?
        .ok_or_else(|| ApiError::NotFound)?;

    let status: String = row.try_get("status").unwrap_or_default();
    if status == "undone" {
        return Err(ApiError::Conflict {
            reason: "import already undone".into(),
        });
    }

    // Delete works created by this import for this user.
    // Also delete associated library_items, grabs, history, etc. via cascading
    // or explicit cleanup. For alpha3, works is sufficient — imported works
    // won't have library items (they're metadata-only, no files).
    let deleted = sqlx::query("DELETE FROM works WHERE import_id = ? AND user_id = ?")
        .bind(&import_id)
        .bind(user_id)
        .execute(state.db.pool())
        .await
        .map_err(|e| ApiError::Internal(format!("undo delete failed: {e}")))?
        .rows_affected() as i64;

    // Mark import as undone.
    let _ = sqlx::query("UPDATE imports SET status = 'undone' WHERE id = ?")
        .bind(&import_id)
        .execute(state.db.pool())
        .await;

    info!(user_id, import_id = %import_id, works_removed = deleted, "list import undone");

    Ok(Json(UndoResponse {
        works_removed: deleted,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/v1/listimport
// ---------------------------------------------------------------------------

pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<ImportSummary>>, ApiError> {
    let rows = sqlx::query(
        "SELECT id, source, status, started_at, completed_at, works_created \
         FROM imports WHERE user_id = ? AND source IN ('goodreads', 'hardcover') \
         ORDER BY started_at DESC LIMIT 50",
    )
    .bind(ctx.user.id)
    .fetch_all(state.db.pool())
    .await
    .map_err(|e| ApiError::Internal(format!("list imports failed: {e}")))?;

    let imports: Vec<ImportSummary> = rows
        .iter()
        .map(|r| ImportSummary {
            id: r.try_get("id").unwrap_or_default(),
            source: r.try_get("source").unwrap_or_default(),
            status: r.try_get("status").unwrap_or_default(),
            started_at: r.try_get("started_at").unwrap_or_default(),
            completed_at: r.try_get("completed_at").ok().flatten(),
            works_created: r.try_get("works_created").unwrap_or(0),
        })
        .collect();

    Ok(Json(imports))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a work already exists for this user by ISBN-13 or ISBN-10.
async fn check_work_exists_by_isbn(
    state: &AppState,
    user_id: i64,
    isbn_13: Option<&str>,
    isbn_10: Option<&str>,
) -> bool {
    // Check ISBN-13 via works table.
    if let Some(isbn) = isbn_13 {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ? AND isbn_13 = ?")
                .bind(user_id)
                .bind(isbn)
                .fetch_one(state.db.pool())
                .await
                .unwrap_or(0);
        if count > 0 {
            return true;
        }

        // Also check external_ids table.
        let ext_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_ids ei \
             JOIN works w ON ei.work_id = w.id \
             WHERE w.user_id = ? AND ei.id_type = 'isbn_13' AND ei.id_value = ?",
        )
        .bind(user_id)
        .bind(isbn)
        .fetch_one(state.db.pool())
        .await
        .unwrap_or(0);
        if ext_count > 0 {
            return true;
        }
    }

    // Check ISBN-10 via external_ids.
    if let Some(isbn) = isbn_10 {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_ids ei \
             JOIN works w ON ei.work_id = w.id \
             WHERE w.user_id = ? AND ei.id_type = 'isbn_10' AND ei.id_value = ?",
        )
        .bind(user_id)
        .bind(isbn)
        .fetch_one(state.db.pool())
        .await
        .unwrap_or(0);
        if count > 0 {
            return true;
        }
    }

    false
}

/// Look up a book on OpenLibrary by ISBN (preferred) or title+author search.
/// Returns an AddWorkRequest on success, or an error message.
async fn ol_lookup(
    state: &AppState,
    isbn_13: Option<&str>,
    isbn_10: Option<&str>,
    title: &str,
    author: &str,
    year: Option<i32>,
) -> Result<AddWorkRequest, String> {
    // Try ISBN lookup first (more precise).
    let isbn = isbn_13.or(isbn_10);
    if let Some(isbn) = isbn {
        if let Some(req) = ol_isbn_lookup(state, isbn).await {
            return Ok(req);
        }
    }

    // Fallback: title + author search.
    ol_search(state, title, author, year).await
}

/// OpenLibrary ISBN lookup → AddWorkRequest.
async fn ol_isbn_lookup(state: &AppState, isbn: &str) -> Option<AddWorkRequest> {
    let url = format!("https://openlibrary.org/isbn/{isbn}.json");
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state.http_client.get(&url).send(),
    )
    .await
    .ok()?
    .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;

    // ISBN endpoint returns an edition — we need to follow the works link.
    let works_key = data
        .get("works")
        .and_then(|w| w.as_array())
        .and_then(|a| a.first())
        .and_then(|w| w.get("key"))
        .and_then(|k| k.as_str())?;

    let ol_key = works_key.trim_start_matches("/works/").to_string();

    // Fetch the work record for title/author.
    let work_url = format!("https://openlibrary.org{works_key}.json");
    let work_resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state.http_client.get(&work_url).send(),
    )
    .await
    .ok()?
    .ok()?;

    let work_data: serde_json::Value = work_resp.json().await.ok()?;

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
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                state.http_client.get(&author_url).send(),
            )
            .await
            {
                Ok(Ok(resp)) => resp
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("name")?.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Unknown".to_string()),
                _ => "Unknown".to_string(),
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
            // Extract 4-digit year from publish_date string.
            d.chars()
                .collect::<String>()
                .split_whitespace()
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
        defer_enrichment: false,
    })
}

/// OpenLibrary search by title + author → AddWorkRequest.
async fn ol_search(
    state: &AppState,
    title: &str,
    author: &str,
    csv_year: Option<i32>,
) -> Result<AddWorkRequest, String> {
    let search_term = format!("{title} {author}");

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state
            .http_client
            .get("https://openlibrary.org/search.json")
            .query(&[
                ("q", search_term.as_str()),
                ("limit", "5"),
                (
                    "fields",
                    "key,title,author_name,author_key,first_publish_year,cover_i",
                ),
            ])
            .send(),
    )
    .await
    .map_err(|_| "OpenLibrary search timed out".to_string())?
    .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("OpenLibrary returned {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
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
        defer_enrichment: false,
    })
}
