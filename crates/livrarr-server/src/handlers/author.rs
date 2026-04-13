use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    AddAuthorRequest, ApiError, AuthContext, AuthorDetailResponse, AuthorResponse,
    AuthorSearchResult, FieldError, UpdateAuthorApiRequest, WorkDetailResponse,
};
use livrarr_db::{
    AuthorBibliographyDb, AuthorDb, BibliographyEntry, ConfigDb, CreateAuthorDbRequest,
    UpdateAuthorDbRequest, WorkDb,
};
use livrarr_domain::{Author, Work};

fn author_to_response(a: &Author) -> AuthorResponse {
    AuthorResponse {
        id: a.id,
        name: a.name.clone(),
        sort_name: a.sort_name.clone(),
        ol_key: a.ol_key.clone(),
        monitored: a.monitored,
        monitor_new_items: a.monitor_new_items,
        added_at: a.added_at.to_rfc3339(),
    }
}

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
        hc_key: w.hc_key.clone(),
        gr_key: w.gr_key.clone(),
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
        monitor_ebook: w.monitor_ebook,
        monitor_audiobook: w.monitor_audiobook,
        added_at: w.added_at.to_rfc3339(),
        library_items: vec![],
        metadata_source: w.metadata_source.clone(),
    }
}

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
}

/// Search OpenLibrary for authors by name. Reusable by handlers and manual import.
pub async fn lookup_ol_authors(
    http: &livrarr_http::HttpClient,
    term: &str,
    limit: u32,
) -> Result<Vec<AuthorSearchResult>, String> {
    let resp = http
        .get("https://openlibrary.org/search/authors.json")
        .query(&[("q", term), ("limit", &limit.to_string())])
        .send()
        .await
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
        .cloned()
        .unwrap_or_default();

    Ok(docs
        .iter()
        .filter_map(|doc| {
            let key = doc.get("key")?.as_str()?;
            let name = doc.get("name")?.as_str()?;
            let ol_key = key.trim_start_matches("/authors/").to_string();

            Some(AuthorSearchResult {
                ol_key,
                name: name.to_string(),
                sort_name: None,
            })
        })
        .collect())
}

/// GET /api/v1/author/lookup?term=...  — searches OpenLibrary
pub async fn lookup(
    State(state): State<AppState>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<Vec<AuthorSearchResult>>, ApiError> {
    let term = q.term.unwrap_or_default();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }

    let results = lookup_ol_authors(&state.http_client, &term, 20)
        .await
        .map_err(|e| ApiError::BadGateway(e))?;

    Ok(Json(results))
}

/// POST /api/v1/author
pub async fn add(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<AddAuthorRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let user_id = ctx.user.id;

    // Check if author exists by name (dedup).
    if let Some(existing) = state.db.find_author_by_name(user_id, &req.name).await? {
        // Update existing with new ol_key.
        let updated = state
            .db
            .update_author(
                user_id,
                existing.id,
                UpdateAuthorDbRequest {
                    name: None,
                    sort_name: req.sort_name,
                    ol_key: Some(req.ol_key),
                    monitored: None,
                    monitor_new_items: None,
                    monitor_since: None,
                },
            )
            .await?;
        return Ok(Json(author_to_response(&updated)));
    }

    let author = state
        .db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: req.name,
            sort_name: req.sort_name,
            ol_key: Some(req.ol_key),
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await?;

    spawn_bibliography_fetch(state.clone(), author.id, user_id);

    Ok(Json(author_to_response(&author)))
}

/// GET /api/v1/author
pub async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<AuthorResponse>>, ApiError> {
    let authors = state.db.list_authors(ctx.user.id).await?;
    Ok(Json(authors.iter().map(author_to_response).collect()))
}

/// GET /api/v1/author/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<AuthorDetailResponse>, ApiError> {
    let user_id = ctx.user.id;
    let author = state.db.get_author(user_id, id).await?;
    let works = state.db.list_works(user_id).await?;
    let author_works: Vec<WorkDetailResponse> = works
        .iter()
        .filter(|w| w.author_id == Some(id))
        .map(work_to_detail)
        .collect();

    Ok(Json(AuthorDetailResponse {
        author: author_to_response(&author),
        works: author_works,
    }))
}

/// PUT /api/v1/author/:id
pub async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAuthorApiRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let user_id = ctx.user.id;
    let author = state.db.get_author(user_id, id).await?;

    // Validate: monitored=true requires ol_key.
    if req.monitored == Some(true) && author.ol_key.is_none() {
        return Err(ApiError::Validation {
            errors: vec![FieldError {
                field: "monitored".into(),
                message: "cannot monitor author without OL linkage".into(),
            }],
        });
    }

    // Validate: monitor_new_items=true requires monitored=true.
    if req.monitor_new_items == Some(true) {
        let will_be_monitored = req.monitored.unwrap_or(author.monitored);
        if !will_be_monitored {
            return Err(ApiError::Validation {
                errors: vec![FieldError {
                    field: "monitor_new_items".into(),
                    message: "monitor_new_items requires monitored=true".into(),
                }],
            });
        }
    }

    let mut db_req = UpdateAuthorDbRequest {
        name: None,
        sort_name: None,
        ol_key: None,
        monitored: req.monitored,
        monitor_new_items: req.monitor_new_items,
        monitor_since: None,
    };

    // Set monitor_since when first enabling monitoring.
    if req.monitored == Some(true) && !author.monitored {
        db_req.monitor_since = Some(Utc::now());
    }

    let updated = state.db.update_author(user_id, id, db_req).await?;

    Ok(Json(author_to_response(&updated)))
}

/// DELETE /api/v1/author/:id
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    state.db.delete_author(ctx.user.id, id).await?;
    Ok(())
}

/// POST /api/v1/author/search — trigger author monitor check for all monitored authors.
/// Returns 202 Accepted; work runs in the background.
pub async fn search(State(state): State<AppState>, _admin: RequireAdmin) -> StatusCode {
    tokio::spawn(async move {
        let cancel = CancellationToken::new();
        if let Err(e) = crate::jobs::author_monitor_tick(state, cancel).await {
            tracing::error!("manual author search failed: {e}");
        }
    });
    StatusCode::ACCEPTED
}

/// GET /api/v1/author/{id}/bibliography — cached author bibliography from OL + LLM cleanup.
pub async fn bibliography(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<livrarr_db::AuthorBibliography>, ApiError> {
    // Verify ownership BEFORE any cache read — prevents cross-user data leak.
    let author = state.db.get_author(ctx.user.id, id).await?;

    // Check cache — skip empty caches (failed previous fetch).
    if let Ok(Some(cached)) = state.db.get_bibliography(id).await {
        if !cached.entries.is_empty() {
            return Ok(Json(cached));
        }
    }
    let ol_key = author
        .ol_key
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("Author has no Open Library key".into()))?;

    let url = format!("https://openlibrary.org/authors/{ol_key}/works.json?limit=100");
    let resp = state
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("OL request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "OL returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("OL parse: {e}")))?;

    let entries_raw: Vec<BibliographyEntry> = data
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
                    Some(BibliographyEntry {
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

    if entries_raw.is_empty() {
        let bib = state.db.save_bibliography(id, &[]).await?;
        return Ok(Json(bib));
    }

    // LLM cleanup if configured.
    let cleaned = llm_clean_bibliography(&state, &author.name, &entries_raw).await;
    let final_entries = cleaned.as_deref().unwrap_or(&entries_raw);

    let bib = state.db.save_bibliography(id, final_entries).await?;
    Ok(Json(bib))
}

/// POST /api/v1/author/{id}/bibliography/refresh — force re-fetch.
pub async fn refresh_bibliography(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<livrarr_db::AuthorBibliography>, ApiError> {
    // Verify ownership BEFORE any cache mutation — prevents cross-user cache wipe.
    let _author = state.db.get_author(ctx.user.id, id).await?;

    // Clear cache so bibliography() re-fetches.
    if let Err(e) = sqlx::query("DELETE FROM author_bibliography WHERE author_id = ?")
        .bind(id)
        .execute(state.db.pool())
        .await
    {
        tracing::warn!("DELETE author_bibliography failed: {e}");
    }
    bibliography(State(state), ctx, Path(id)).await
}

/// Spawn a background task to fetch and cache an author's bibliography.
/// Fire-and-forget — errors are logged, never block the caller.
pub fn spawn_bibliography_fetch(state: AppState, author_id: i64, user_id: i64) {
    tokio::spawn(async move {
        // Small delay to let the author creation transaction commit.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let author = match state.db.get_author(user_id, author_id).await {
            Ok(a) => a,
            Err(_) => return,
        };
        let ol_key = match author.ol_key.as_deref() {
            Some(k) => k,
            None => return,
        };

        let url = format!("https://openlibrary.org/authors/{ol_key}/works.json?limit=100");
        let resp = match state.http_client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };
        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            Err(_) => return,
        };

        let entries_raw: Vec<BibliographyEntry> = data
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
                        Some(BibliographyEntry {
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

        if entries_raw.is_empty() {
            return;
        }

        let cleaned = llm_clean_bibliography(&state, &author.name, &entries_raw).await;
        let final_entries = cleaned.as_deref().unwrap_or(&entries_raw);
        if let Err(e) = state.db.save_bibliography(author_id, final_entries).await {
            tracing::warn!("save_bibliography failed: {e}");
        }

        tracing::info!(
            author = %author.name,
            entries = final_entries.len(),
            "background bibliography fetch complete"
        );
    });
}

async fn llm_clean_bibliography(
    state: &AppState,
    author_name: &str,
    entries: &[BibliographyEntry],
) -> Option<Vec<BibliographyEntry>> {
    let cfg = state.db.get_metadata_config().await.ok()?;
    if !cfg.llm_enabled {
        return None;
    }
    let endpoint = cfg.llm_endpoint.as_deref().filter(|s| !s.is_empty())?;
    let api_key = cfg.llm_api_key.as_deref().filter(|s| !s.is_empty())?;
    let model = cfg.llm_model.as_deref().filter(|s| !s.is_empty())?;

    let mut listing = String::new();
    for (i, e) in entries.iter().enumerate() {
        listing.push_str(&format!(
            "{}: \"{}\" ({})\n",
            i,
            e.title,
            e.year.map(|y| y.to_string()).unwrap_or_default(),
        ));
    }

    let prompt = format!(
        "These are works by {author_name} from a book database:\n\n\
         {listing}\n\
         Clean up this list:\n\
         1. Remove duplicates, foreign editions, comic adaptations, anthologies, and compilations\n\
         2. Fix spelling and capitalization\n\
         3. Add series name and position if you know it\n\
         4. Order results in the most logical way for a reader\n\n\
         Return a JSON array. Each entry: {{\"idx\": <original index>, \"title\": \"<cleaned title>\", \
         \"series\": \"<series name or null>\", \"position\": <number or null>}}\n\
         Return ONLY the JSON array, no other text."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 4000,
        "temperature": 0.0,
    });

    let resp = state
        .http_client
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

    let json_str = answer
        .strip_prefix("```json")
        .or_else(|| answer.strip_prefix("```"))
        .unwrap_or(answer)
        .strip_suffix("```")
        .unwrap_or(answer)
        .trim();

    let llm_entries: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

    let cleaned: Vec<BibliographyEntry> = llm_entries
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
