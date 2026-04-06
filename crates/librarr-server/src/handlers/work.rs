use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;

use crate::state::AppState;
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse,
    RefreshWorkResponse, UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use librarr_db::{
    AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateWorkDbRequest, LibraryItemDb, NotificationDb,
    UpdateWorkUserFieldsDbRequest, WorkDb,
};
use librarr_domain::Work;

fn work_to_detail(w: &Work) -> WorkDetailResponse {
    WorkDetailResponse {
        id: w.id,
        title: w.title.clone(),
        sort_title: w.sort_title.clone(),
        subtitle: w.subtitle.clone(),
        original_title: w.original_title.clone(),
        author_name: w.author_name.clone(),
        author_id: w.author_id,
        description: w.description.clone(),
        year: w.year,
        series_name: w.series_name.clone(),
        series_position: w.series_position,
        genres: w.genres.clone(),
        language: w.language.clone(),
        page_count: w.page_count,
        duration_seconds: w.duration_seconds,
        publisher: w.publisher.clone(),
        publish_date: w.publish_date.clone(),
        ol_key: w.ol_key.clone(),
        hardcover_id: w.hardcover_id.clone(),
        isbn_13: w.isbn_13.clone(),
        asin: w.asin.clone(),
        narrator: w.narrator.clone(),
        narration_type: w.narration_type,
        abridged: w.abridged,
        rating: w.rating,
        rating_count: w.rating_count,
        enrichment_status: w.enrichment_status,
        enriched_at: w.enriched_at.map(|d| d.to_rfc3339()),
        enrichment_source: w.enrichment_source.clone(),
        cover_manual: w.cover_manual,
        monitored: w.monitored,
        added_at: w.added_at.to_rfc3339(),
        library_items: vec![],
    }
}

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct DeleteQuery {
    #[serde(rename = "deleteFiles")]
    pub delete_files: Option<bool>,
}

/// GET /api/v1/work/lookup?term=...  — searches OpenLibrary, optionally cleans with LLM.
pub async fn lookup(
    State(state): State<AppState>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<Vec<WorkSearchResult>>, ApiError> {
    let term = q.term.unwrap_or_default();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }

    let results = lookup_openlibrary(&state.http_client, &term).await?;
    let results = results.0; // unwrap Json

    // If LLM is configured, clean up the results.
    if results.len() > 1 {
        let cfg = state.db.get_metadata_config().await.ok();
        if let Some(cleaned) =
            llm_clean_search_results(&state.http_client, cfg.as_ref(), &term, &results).await
        {
            return Ok(Json(cleaned));
        }
    }

    Ok(Json(results))
}

async fn lookup_hardcover(
    http: &librarr_http::HttpClient,
    term: &str,
    token: &str,
) -> Result<Vec<WorkSearchResult>, String> {
    // Search books and authors in parallel.
    let (book_results, author_books) = tokio::join!(
        search_hardcover_books(http, term, token),
        search_hardcover_author_books(http, term, token),
    );

    let mut results = author_books.unwrap_or_default();
    let author_ids: std::collections::HashSet<String> =
        results.iter().map(|r| r.ol_key.clone()).collect();

    // Append book results that aren't already in author results.
    for r in book_results.unwrap_or_default() {
        if !author_ids.contains(&r.ol_key) {
            results.push(r);
        }
    }

    Ok(results)
}

/// Search Hardcover for books matching term.
async fn search_hardcover_books(
    http: &librarr_http::HttpClient,
    term: &str,
    token: &str,
) -> Result<Vec<WorkSearchResult>, String> {
    let query = r#"query SearchBooks($query: String!) {
        search(query: $query, query_type: "books", per_page: 20) {
            results
        }
    }"#;

    let body = serde_json::json!({
        "query": query,
        "variables": {"query": term}
    });

    let resp = http
        .post("https://api.hardcover.app/v1/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    let hits = data
        .pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let results: Vec<WorkSearchResult> = hits
        .iter()
        .filter_map(|hit| {
            let doc = hit.get("document")?;
            let title = doc.get("title")?.as_str()?;
            let id = doc.get("id")?.to_string().trim_matches('"').to_string();

            let author_name = doc
                .get("author_names")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let year = doc
                .get("release_year")
                .and_then(|y| y.as_i64())
                .or_else(|| {
                    doc.get("release_date")
                        .and_then(|d| d.as_str())
                        .and_then(|s| s.get(..4))
                        .and_then(|y| y.parse().ok())
                })
                .map(|y| y as i32);

            let cover_url = doc
                .pointer("/image/url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let description = doc
                .get("description")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            // Use hardcover:{id} as the olKey — the frontend uses olKey as a unique identifier.
            // When adding, the backend will need to handle this prefix.
            Some(WorkSearchResult {
                ol_key: format!("hardcover:{id}"),
                title: title.to_string(),
                author_name,
                author_ol_key: None,
                year,
                cover_url,
                description,
                series_name: None,
                series_position: None,
            })
        })
        .collect();

    Ok(results)
}

/// Extract an author ID from book search results, then fetch their top books.
async fn search_hardcover_author_books(
    http: &librarr_http::HttpClient,
    term: &str,
    token: &str,
) -> Result<Vec<WorkSearchResult>, String> {
    // Step 1: Search books to find an author matching the term in contributions.
    let probe = search_hardcover_books(http, term, token).await;
    let probe_hits = match &probe {
        Ok(results) if !results.is_empty() => results,
        _ => return Ok(vec![]),
    };

    // We need the raw hits to extract author IDs from contributions.
    // Re-fetch the raw data (the book search already ran, but we didn't save raw hits).
    let query = r#"query SearchBooks($query: String!) {
        search(query: $query, query_type: "books", per_page: 5) {
            results
        }
    }"#;
    let body = serde_json::json!({
        "query": query,
        "variables": {"query": term}
    });
    let resp = http
        .post("https://api.hardcover.app/v1/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;
    let hits = data
        .pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    // Find the author ID whose name best matches the search term.
    let term_lower = term.trim().to_lowercase();
    let mut author_id: Option<i64> = None;
    let mut author_name = String::new();

    for hit in &hits {
        let contribs = hit
            .pointer("/document/contributions")
            .and_then(|c| c.as_array());
        if let Some(contribs) = contribs {
            for contrib in contribs {
                let name = contrib
                    .pointer("/author/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if name.to_lowercase() == term_lower {
                    author_id = contrib.pointer("/author/id").and_then(|v| v.as_i64());
                    author_name = name.to_string();
                    break;
                }
            }
        }
        if author_id.is_some() {
            break;
        }
    }

    let author_id = match author_id {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    // Step 2: Fetch books by this author
    let books_query = r#"query AuthorBooks($authorId: Int!) {
        books(
            where: { contributions: { author_id: { _eq: $authorId } } }
            order_by: [{ users_read_count: desc }]
            limit: 20
        ) {
            id
            title
            release_date
            release_year
            description
            image { url }
        }
    }"#;

    let body = serde_json::json!({
        "query": books_query,
        "variables": {"authorId": author_id}
    });

    let resp = http
        .post("https://api.hardcover.app/v1/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("author books failed: {e}"))?;

    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;
    let books = data
        .pointer("/data/books")
        .and_then(|b| b.as_array())
        .cloned()
        .unwrap_or_default();

    let results: Vec<WorkSearchResult> = books
        .iter()
        .filter_map(|book| {
            let title = book.get("title")?.as_str()?;
            let id = book.get("id")?.to_string().trim_matches('"').to_string();

            let year = book
                .get("release_year")
                .and_then(|y| y.as_i64())
                .or_else(|| {
                    book.get("release_date")
                        .and_then(|d| d.as_str())
                        .and_then(|s| s.get(..4))
                        .and_then(|y| y.parse().ok())
                })
                .map(|y| y as i32);

            let cover_url = book
                .pointer("/image/url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let description = book
                .get("description")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            Some(WorkSearchResult {
                ol_key: format!("hardcover:{id}"),
                title: title.to_string(),
                author_name: author_name.clone(),
                author_ol_key: None,
                year,
                cover_url,
                description,
                series_name: None,
                series_position: None,
            })
        })
        .collect();

    Ok(results)
}

/// Use LLM to clean up search results — remove duplicates, foreign editions,
/// comics, anthologies, and misattributions. Returns None if LLM not configured or fails.
async fn llm_clean_search_results(
    http: &librarr_http::HttpClient,
    cfg: Option<&librarr_db::MetadataConfig>,
    search_term: &str,
    results: &[WorkSearchResult],
) -> Option<Vec<WorkSearchResult>> {
    let cfg = cfg?;
    if !cfg.llm_enabled {
        return None;
    }
    let endpoint = cfg.llm_endpoint.as_deref().filter(|s| !s.is_empty())?;
    let api_key = cfg.llm_api_key.as_deref().filter(|s| !s.is_empty())?;
    let model = cfg.llm_model.as_deref().filter(|s| !s.is_empty())?;

    // Build numbered list of results for the LLM.
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

    let prompt = format!(
        "I searched a book database for \"{search_term}\". Here are the raw results:\n\n\
         {listing}\n\
         Clean up this list:\n\
         1. Remove duplicates, foreign editions, comic adaptations, and anthologies\n\
         2. Fix spelling and capitalization of titles and author names\n\
         3. Remove series info from titles (e.g. \"The Great Hunt (The Wheel of Time Book 2)\" → \"The Great Hunt\")\n\
         4. Add series name and position if you know it\n\n\
         Order results in the most logical way for a reader.\n\n\
         Return a JSON array. Each entry: {{\"idx\": <original index>, \"title\": \"<cleaned title>\", \
         \"author\": \"<cleaned author>\", \"series\": \"<series name or null>\", \"position\": <number or null>}}\n\
         Return ONLY the JSON array, no other text."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 2000,
        "temperature": 0.0,
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let answer = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if answer.is_empty() {
        return None;
    }

    // Strip markdown code fences if present.
    let json_str = answer
        .strip_prefix("```json")
        .or_else(|| answer.strip_prefix("```"))
        .unwrap_or(answer)
        .strip_suffix("```")
        .unwrap_or(answer)
        .trim();

    let entries: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

    let cleaned: Vec<WorkSearchResult> = entries
        .iter()
        .filter_map(|entry| {
            let idx = entry.get("idx")?.as_u64()? as usize;
            if idx >= results.len() {
                return None;
            }
            let mut r = results[idx].clone();
            if let Some(t) = entry.get("title").and_then(|v| v.as_str()) {
                r.title = t.to_string();
            }
            if let Some(a) = entry.get("author").and_then(|v| v.as_str()) {
                r.author_name = a.to_string();
            }
            r.series_name = entry
                .get("series")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            r.series_position = entry.get("position").and_then(|v| v.as_f64());
            Some(r)
        })
        .collect();

    if cleaned.is_empty() {
        return None;
    }

    Some(cleaned)
}

async fn lookup_openlibrary(
    http: &librarr_http::HttpClient,
    term: &str,
) -> Result<Json<Vec<WorkSearchResult>>, ApiError> {
    let resp = http
        .get("https://openlibrary.org/search.json")
        .query(&[
            ("q", term),
            ("limit", "50"),
            (
                "fields",
                "key,title,author_name,author_key,first_publish_year,cover_i",
            ),
        ])
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("OpenLibrary request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "OpenLibrary returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("OpenLibrary parse error: {e}")))?;

    let docs = data
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let results: Vec<WorkSearchResult> = docs
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

            Some(WorkSearchResult {
                ol_key,
                title: title.to_string(),
                author_name,
                author_ol_key,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
            })
        })
        .collect();

    Ok(Json(results))
}

/// Internal work creation — shared by the HTTP handler and manual import.
pub async fn add_work_internal(
    state: &AppState,
    user_id: i64,
    req: AddWorkRequest,
) -> Result<AddWorkResponse, ApiError> {
    // Check duplicate by ol_key.
    if state.db.work_exists_by_ol_key(user_id, &req.ol_key).await? {
        return Err(ApiError::Conflict {
            reason: "work already exists".into(),
        });
    }

    // Find or create author.
    let author_name_normalized = req.author_name.trim().to_lowercase();
    let existing_author = state
        .db
        .find_author_by_name(user_id, &author_name_normalized)
        .await?;

    let (author_id, author_created) = match existing_author {
        Some(a) => (Some(a.id), false),
        None => {
            let author = state
                .db
                .create_author(CreateAuthorDbRequest {
                    user_id,
                    name: req.author_name.clone(),
                    sort_name: None,
                    ol_key: req.author_ol_key.clone(),
                })
                .await?;
            crate::handlers::author::spawn_bibliography_fetch((*state).clone(), author.id, user_id);
            (Some(author.id), true)
        }
    };

    let cover_url = req.cover_url.clone();

    let work = state
        .db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: req.title,
            author_name: req.author_name,
            author_id,
            ol_key: Some(req.ol_key),
            year: req.year,
            cover_url: req.cover_url,
        })
        .await?;

    // Download cover image in background (best-effort, don't fail the add).
    if let Some(url) = cover_url {
        let http = state.http_client.clone();
        let covers_dir = state.data_dir.join("covers");
        let work_id = work.id;
        tokio::spawn(async move {
            let _ = download_cover(&http, &url, &covers_dir, work_id).await;
        });
    }

    // Run enrichment (synchronous, best-effort).
    let outcome = super::enrichment::enrich_work(state, &work).await;
    let enriched_work = state
        .db
        .update_work_enrichment(user_id, work.id, outcome.request)
        .await
        .unwrap_or(work);

    Ok(AddWorkResponse {
        work: work_to_detail(&enriched_work),
        author_created,
        messages: outcome.messages,
    })
}

/// POST /api/v1/work
pub async fn add(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<AddWorkRequest>,
) -> Result<Json<AddWorkResponse>, ApiError> {
    let resp = add_work_internal(&state, ctx.user.id, req).await?;
    Ok(Json(resp))
}

/// GET /api/v1/work
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<WorkDetailResponse>>, ApiError> {
    let user_id = ctx.user.id;
    let works = state.db.list_works(user_id).await?;
    let all_items = state.db.list_library_items(user_id).await?;

    let mut results: Vec<WorkDetailResponse> = works.iter().map(work_to_detail).collect();
    for detail in &mut results {
        detail.library_items = all_items
            .iter()
            .filter(|li| li.work_id == detail.id)
            .map(|li| crate::LibraryItemResponse {
                id: li.id,
                path: li.path.clone(),
                media_type: li.media_type,
                file_size: li.file_size,
                imported_at: li.imported_at.to_rfc3339(),
            })
            .collect();
    }
    Ok(Json(results))
}

/// GET /api/v1/work/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let user_id = ctx.user.id;
    let work = state.db.get_work(user_id, id).await?;
    let items = state.db.list_library_items_by_work(user_id, id).await?;
    let mut detail = work_to_detail(&work);
    detail.library_items = items
        .into_iter()
        .map(|li| crate::LibraryItemResponse {
            id: li.id,
            path: li.path,
            media_type: li.media_type,
            file_size: li.file_size,
            imported_at: li.imported_at.to_rfc3339(),
        })
        .collect();
    Ok(Json(detail))
}

/// PUT /api/v1/work/:id
pub async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateWorkRequest>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let work = state
        .db
        .update_work_user_fields(
            ctx.user.id,
            id,
            UpdateWorkUserFieldsDbRequest {
                title: req.title,
                author_name: req.author_name,
                series_name: req.series_name,
                series_position: req.series_position,
            },
        )
        .await?;
    Ok(Json(work_to_detail(&work)))
}

/// POST /api/v1/work/:id/cover
pub async fn upload_cover(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<(), ApiError> {
    let user_id = ctx.user.id;

    // Verify work exists.
    let _work = state.db.get_work(user_id, id).await?;

    if body.is_empty() {
        return Err(ApiError::BadRequest("empty image data".into()));
    }

    // Store cover file on disk.
    let covers_dir = state.data_dir.join("covers");
    tokio::fs::create_dir_all(&covers_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to create covers dir: {e}")))?;

    let cover_path = covers_dir.join(format!("{id}.jpg"));
    tokio::fs::write(&cover_path, &body)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to write cover: {e}")))?;

    // Set cover_manual flag.
    state.db.set_cover_manual(user_id, id, true).await?;

    Ok(())
}

/// DELETE /api/v1/work/:id?deleteFiles=...
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(_q): Query<DeleteQuery>,
) -> Result<Json<DeleteWorkResponse>, ApiError> {
    let _work = state.db.delete_work(ctx.user.id, id).await?;
    Ok(Json(DeleteWorkResponse { warnings: vec![] }))
}

/// POST /api/v1/work/:id/refresh — re-enrich from providers
pub async fn refresh(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<RefreshWorkResponse>, ApiError> {
    let user_id = ctx.user.id;
    let work = state.db.get_work(user_id, id).await?;

    // Reset enrichment status.
    let _ =
        librarr_db::EnrichmentRetryDb::reset_enrichment_for_refresh(&state.db, user_id, id).await;

    // Re-enrich.
    let outcome = super::enrichment::enrich_work(&state, &work).await;
    let enriched = state
        .db
        .update_work_enrichment(user_id, id, outcome.request)
        .await
        .unwrap_or(work);

    // TAG-V21-004: rewrite tags on existing library items after re-enrichment.
    let mut messages = outcome.messages;
    let taggable = state.db.list_taggable_items_by_work(user_id, id).await?;
    if !taggable.is_empty() {
        let tag_warnings = super::import::retag_library_items(&state, &enriched, &taggable).await;
        for w in &tag_warnings {
            messages.push(format!("tag rewrite warning: {w}"));
        }
        if tag_warnings.is_empty() && !taggable.is_empty() {
            messages.push(format!("tags rewritten on {} file(s)", taggable.len()));
        }
    }

    Ok(Json(RefreshWorkResponse {
        work: work_to_detail(&enriched),
        messages,
    }))
}

/// POST /api/v1/work/refresh — refresh metadata for all user works.
/// Returns 202 immediately; enrichment runs in background.
pub async fn refresh_all(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;
    let works = state.db.list_works(user_id).await?;

    if works.is_empty() {
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    let total = works.len();
    tokio::spawn(async move {
        let mut enriched = 0usize;
        let mut failed = 0usize;

        for work in &works {
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                super::enrichment::enrich_work(&state, work),
            )
            .await;

            match outcome {
                Ok(o) => {
                    if state
                        .db
                        .update_work_enrichment(user_id, work.id, o.request)
                        .await
                        .is_ok()
                    {
                        enriched += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(_) => {
                    failed += 1;
                }
            }
        }

        let _ = state
            .db
            .create_notification(librarr_db::CreateNotificationDbRequest {
                user_id,
                notification_type: librarr_domain::NotificationType::BulkEnrichmentComplete,
                ref_key: None,
                message: format!(
                    "Bulk refresh complete: {enriched}/{total} enriched, {failed} failed"
                ),
                data: serde_json::json!({
                    "total": total,
                    "enriched": enriched,
                    "failed": failed,
                }),
            })
            .await;
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

/// Download a cover image from a URL and save to the covers directory.
async fn download_cover(
    http: &librarr_http::HttpClient,
    url: &str,
    covers_dir: &std::path::Path,
    work_id: i64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::fs::create_dir_all(covers_dir).await?;
    let resp = http.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("cover download returned {}", resp.status()).into());
    }
    let bytes = resp.bytes().await?;
    let cover_path = covers_dir.join(format!("{work_id}.jpg"));
    tokio::fs::write(&cover_path, &bytes).await?;
    Ok(())
}
