use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;

use crate::state::AppState;
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse,
    RefreshWorkResponse, UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use livrarr_db::{
    AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateWorkDbRequest, NotificationDb, ProvenanceDb,
    SetFieldProvenanceRequest, WorkDb,
};
use livrarr_domain::services::{WorkDetailView, WorkService};
use livrarr_domain::{ProvenanceSetter, Work, WorkField};

/// Write add-time provenance entries for a freshly-created work.
///
/// Tracks which identity fields originated at add-time and the setter
/// (User for user-driven adds, AutoAdded for author-monitor / series-add).
/// User-set fields act as the LLM identity-lock anchor at enrichment time;
/// AutoAdded fields are tracked honestly but do not anchor.
///
/// Best-effort: errors are logged but do not fail the add — at worst the
/// work runs unanchored and the LLM check falls back to no-anchor mode
/// (equivalent to legacy enrichment).
pub(crate) async fn write_addtime_provenance(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: i64,
    work: &Work,
    setter: ProvenanceSetter,
) {
    let mut reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
    let push = |reqs: &mut Vec<SetFieldProvenanceRequest>, field: WorkField| {
        reqs.push(SetFieldProvenanceRequest {
            user_id,
            work_id: work.id,
            field,
            source: None,
            setter,
            cleared: false,
        });
    };
    if !work.title.is_empty() {
        push(&mut reqs, WorkField::Title);
    }
    if !work.author_name.is_empty() {
        push(&mut reqs, WorkField::AuthorName);
    }
    if work.ol_key.is_some() {
        push(&mut reqs, WorkField::OlKey);
    }
    if work.gr_key.is_some() {
        push(&mut reqs, WorkField::GrKey);
    }
    if work.language.is_some() {
        push(&mut reqs, WorkField::Language);
    }
    if work.year.is_some() {
        push(&mut reqs, WorkField::Year);
    }
    if work.series_name.is_some() {
        push(&mut reqs, WorkField::SeriesName);
    }
    if work.series_position.is_some() {
        push(&mut reqs, WorkField::SeriesPosition);
    }
    if reqs.is_empty() {
        return;
    }
    if let Err(e) = db.set_field_provenance_batch(reqs).await {
        tracing::warn!(
            work_id = work.id,
            ?setter,
            "write_addtime_provenance failed: {e}"
        );
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
        series_id: w.series_id,
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

fn detail_from_view(view: WorkDetailView) -> WorkDetailResponse {
    let mut detail = work_to_detail(&view.work);
    detail.library_items = view
        .library_items
        .iter()
        .map(|li| crate::LibraryItemResponse {
            id: li.id,
            path: li.path.clone(),
            media_type: li.media_type,
            file_size: li.file_size,
            imported_at: li.imported_at.to_rfc3339(),
        })
        .collect();
    detail
}

/// Extract the original URL from a proxy cover URL.
/// Returns the input unchanged if it's not a proxy URL.
fn unproxy_cover_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("/api/v1/coverproxy?url=") {
        urlencoding::decode(rest)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| url.to_string())
    } else {
        url.to_string()
    }
}

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
    pub lang: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct DeleteQuery {
    #[serde(rename = "deleteFiles")]
    pub delete_files: Option<bool>,
}

/// GET /api/v1/work/lookup?term=...&lang=...  — searches metadata providers by language.
pub async fn lookup(
    State(state): State<AppState>,
    _ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<Vec<WorkSearchResult>>, ApiError> {
    use livrarr_domain::services::WorkService;

    let req = livrarr_domain::services::LookupRequest {
        term: q.term.unwrap_or_default(),
        lang_override: q.lang,
    };

    let results = state.work_service.lookup(req).await?;

    let api_results = results
        .into_iter()
        .map(|r| WorkSearchResult {
            ol_key: r.ol_key,
            title: r.title,
            author_name: r.author_name,
            author_ol_key: r.author_ol_key,
            year: r.year,
            cover_url: r.cover_url,
            description: r.description,
            series_name: r.series_name,
            series_position: r.series_position,
            source: r.source,
            source_type: r.source_type,
            language: r.language,
            detail_url: r.detail_url,
            rating: r.rating,
        })
        .collect();

    Ok(Json(api_results))
}

/// Use LLM to clean up search results — remove duplicates, foreign editions,
/// comics, anthologies, and misattributions. Returns None if LLM not configured or fails.
async fn llm_clean_search_results(
    http: &livrarr_http::HttpClient,
    cfg: Option<&livrarr_db::MetadataConfig>,
    search_term: &str,
    results: &[WorkSearchResult],
    foreign: bool,
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

    let instructions = if foreign {
        "Clean up this list:\n\
         1. Remove academic papers, theses, and literary criticism — keep only the actual books\n\
         2. Remove exact duplicates (same title + same author), but KEEP different editions of the same work\n\
         3. Do NOT change title capitalization — preserve the original casing exactly as shown. \
            Many languages (Spanish, French, German, etc.) do NOT capitalize every word in titles like English does.\n\
         4. Fix author names: remove translator/editor info, keep only the primary author (First Last format)\n\
         5. Add series name and position if you know it\n\n\
         Keep multiple editions — they may have different ISBNs needed for cover resolution.\n\
         Order: most relevant/popular edition first."
    } else {
        "Clean up this list:\n\
         1. Remove duplicates, foreign editions, comic adaptations, and anthologies\n\
         2. Fix spelling and capitalization of titles and author names\n\
         3. Remove series info from titles (e.g. \"The Great Hunt (The Wheel of Time Book 2)\" → \"The Great Hunt\")\n\
         4. Add series name and position if you know it\n\n\
         Order results in the most logical way for a reader."
    };

    let prompt = format!(
        "I searched a book database for \"{search_term}\". Here are the raw results:\n\n\
         {listing}\n\
         {instructions}\n\n\
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
    http: &livrarr_http::HttpClient,
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
                ol_key: Some(ol_key),
                title: title.to_string(),
                author_name,
                author_ol_key,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
                source: None,
                source_type: None,
                language: None,
                detail_url: None,
                rating: None,
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
    use livrarr_domain::services::WorkService;

    let svc_req = livrarr_domain::services::AddWorkRequest {
        title: req.title,
        author_name: req.author_name,
        author_ol_key: req.author_ol_key,
        ol_key: req.ol_key,
        gr_key: None,
        year: req.year,
        cover_url: req.cover_url,
        metadata_source: req.metadata_source,
        language: req.language,
        detail_url: req.detail_url,
        series_name: None,
        series_position: None,
        defer_enrichment: req.defer_enrichment,
        provenance_setter: None,
    };

    let result = state.work_service.add(user_id, svc_req).await?;

    if result.author_created {
        if let Some(author_id) = result.author_id {
            crate::handlers::author::spawn_bibliography_fetch((*state).clone(), author_id, user_id);
        }
    }

    Ok(AddWorkResponse {
        work: work_to_detail(&result.work),
        author_created: result.author_created,
        messages: result.messages,
    })
}

/// Fetch the merge-resolved cover URL and write it atomically to
/// `covers/{work_id}.jpg`. Best-effort — every failure is logged but
/// returns `()` so callers don't have to handle errors. Uses the
/// SSRF-safe HTTP client.
pub(crate) async fn download_post_enrich_cover(state: &AppState, work_id: i64, cover_url: &str) {
    let covers_dir = state.data_dir.join("covers");
    if let Err(e) = tokio::fs::create_dir_all(&covers_dir).await {
        tracing::warn!(work_id, "create_dir_all for covers failed: {e}");
        return;
    }
    let resp = match state.http_client_safe.get(cover_url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(work_id, cover_url, "cover download request failed: {e}");
            return;
        }
    };
    if !resp.status().is_success() {
        tracing::warn!(
            work_id,
            cover_url,
            status = %resp.status(),
            "cover download returned non-success status"
        );
        return;
    }
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(work_id, "cover body read failed: {e}");
            return;
        }
    };
    let path = covers_dir.join(format!("{work_id}.jpg"));
    if let Err(e) = atomic_write_cover_handler(&path, &bytes).await {
        tracing::warn!(work_id, "cover atomic write failed: {e}");
        return;
    }
    let thumb = covers_dir.join(format!("{work_id}_thumb.jpg"));
    let _ = tokio::fs::remove_file(&thumb).await;
}

/// Atomically write a cover file: `path.tmp` → fsync → rename.
async fn atomic_write_cover_handler(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("jpg.tmp");
    let tmp_for_blocking = tmp.clone();
    let bytes = bytes.to_vec();
    let target = path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_for_blocking)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_for_blocking, &target)
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(std::io::Error::other(format!("spawn error: {e}")))
        }
    }
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
    Query(pq): Query<crate::PaginationQuery>,
) -> Result<Json<crate::PaginatedResponse<WorkDetailResponse>>, ApiError> {
    let view = state
        .work_service
        .list_paginated(ctx.user.id, pq.page(), pq.page_size())
        .await?;

    let items = view
        .works
        .into_iter()
        .map(|wv| detail_from_view(wv))
        .collect();
    Ok(Json(crate::PaginatedResponse {
        items,
        total: view.total,
        page: view.page,
        page_size: view.page_size,
    }))
}

/// GET /api/v1/work/:id
pub async fn get(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let view = state.work_service.get_detail(ctx.user.id, id).await?;
    Ok(Json(detail_from_view(view)))
}

/// PUT /api/v1/work/:id
pub async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateWorkRequest>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    use livrarr_domain::services::{UpdateWorkRequest as DomainUpdateWorkRequest, WorkService};

    let cleaned_title = req
        .title
        .as_deref()
        .map(livrarr_metadata::title_cleanup::clean_title);
    let cleaned_author = req
        .author_name
        .as_deref()
        .map(livrarr_metadata::title_cleanup::clean_author);

    let work = state
        .work_service
        .update(
            ctx.user.id,
            id,
            DomainUpdateWorkRequest {
                title: cleaned_title,
                author_name: cleaned_author,
                series_name: req.series_name,
                series_position: req.series_position,
                monitor_ebook: req.monitor_ebook,
                monitor_audiobook: req.monitor_audiobook,
            },
        )
        .await?;

    Ok(Json(work_to_detail(&work)))
}

/// Maximum upload size for cover images (1 MB).
const MAX_COVER_BYTES: usize = 1_024 * 1_024;

/// POST /api/v1/work/:id/cover
pub async fn upload_cover(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<(), ApiError> {
    use livrarr_domain::services::WorkService;
    state
        .work_service
        .upload_cover(ctx.user.id, id, &body)
        .await?;
    Ok(())
}

/// DELETE /api/v1/work/:id?deleteFiles=...
pub async fn delete(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Query(_q): Query<DeleteQuery>,
) -> Result<Json<DeleteWorkResponse>, ApiError> {
    use livrarr_domain::services::WorkService;
    state.work_service.delete(ctx.user.id, id).await?;
    Ok(Json(DeleteWorkResponse { warnings: vec![] }))
}

/// POST /api/v1/work/:id/refresh — re-enrich from providers
pub async fn refresh(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<RefreshWorkResponse>, ApiError> {
    let result = state.work_service.refresh(ctx.user.id, id).await?;

    if let Some(ref cover_url) = result.work.cover_url {
        download_post_enrich_cover(&state, id, cover_url).await;
    }

    let mut messages = result.messages;
    if result.merge_deferred {
        messages.push("Merge deferred — retry pending".to_string());
    }

    if !result.taggable_items.is_empty() {
        let tag_warnings =
            super::import::retag_library_items(&state, &result.work, &result.taggable_items).await;
        for w in &tag_warnings {
            messages.push(format!("tag rewrite warning: {w}"));
        }
        if tag_warnings.is_empty() {
            messages.push(format!(
                "tags rewritten on {} file(s)",
                result.taggable_items.len()
            ));
        }
    }

    Ok(Json(RefreshWorkResponse {
        work: work_to_detail(&result.work),
        messages,
    }))
}

/// RAII guard that removes user_id from refresh_in_progress on drop.
/// Panic-safe: uses `lock().ok()` to avoid double-panic during unwind.
struct RefreshGuard {
    user_id: i64,
    set: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.set.lock() {
            set.remove(&self.user_id);
        }
    }
}

/// POST /api/v1/work/refresh — refresh metadata for all user works.
/// Returns 202 immediately; enrichment runs in background.
pub async fn refresh_all(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    {
        let mut guard = state.refresh_in_progress.lock().unwrap();
        if !guard.insert(user_id) {
            return Err(ApiError::Conflict {
                reason: "Refresh already in progress".to_string(),
            });
        }
    }

    let refresh_guard = RefreshGuard {
        user_id,
        set: state.refresh_in_progress.clone(),
    };

    let works = state
        .work_service
        .list(
            user_id,
            livrarr_domain::services::WorkFilter {
                author_id: None,
                monitored: None,
                enrichment_status: None,
                media_type: None,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await
        .map_err(ApiError::from)?;

    if works.is_empty() {
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    let total = works.len();
    tokio::spawn(async move {
        let _guard = refresh_guard;
        let mut enriched = 0usize;
        let mut failed = 0usize;

        for work in &works {
            let result = match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                state.work_service.refresh(user_id, work.id),
            )
            .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh failed: {e}");
                    failed += 1;
                    continue;
                }
                Err(_) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh timed out");
                    failed += 1;
                    continue;
                }
            };

            if let Some(ref cover_url) = result.work.cover_url {
                download_post_enrich_cover(&state, work.id, cover_url).await;
            }

            enriched += 1;
            if !result.taggable_items.is_empty() {
                let _ = super::import::retag_library_items(
                    &state,
                    &result.work,
                    &result.taggable_items,
                )
                .await;
            }
        }

        if let Err(e) = state
            .db
            .create_notification(livrarr_db::CreateNotificationDbRequest {
                user_id,
                notification_type: livrarr_domain::NotificationType::BulkEnrichmentComplete,
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
            .await
        {
            tracing::warn!("create_notification failed: {e}");
        }
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

/// Download a cover image from a URL and save to the covers directory.
async fn download_cover(
    http: &livrarr_http::HttpClient,
    url: &str,
    covers_dir: &std::path::Path,
    work_id: i64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    download_cover_as(http, url, covers_dir, work_id, "").await
}

async fn download_cover_as(
    http: &livrarr_http::HttpClient,
    url: &str,
    covers_dir: &std::path::Path,
    work_id: i64,
    suffix: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::fs::create_dir_all(covers_dir).await?;
    let resp = http.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("cover download returned {}", resp.status()).into());
    }
    let bytes = resp.bytes().await?;
    let cover_path = covers_dir.join(format!("{work_id}{suffix}.jpg"));
    // Atomic write: .tmp → fsync → rename over target.
    let tmp_path = cover_path.with_extension("jpg.tmp");
    let tmp_for_blocking = tmp_path.clone();
    let target = cover_path.clone();
    let bytes_vec = bytes.to_vec();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_for_blocking)?;
        f.write_all(&bytes_vec)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_for_blocking, &target)
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(Box::new(e))
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(format!("spawn error: {e}").into())
        }
    }
}
