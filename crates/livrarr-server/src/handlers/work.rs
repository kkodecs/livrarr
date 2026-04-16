use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;

use crate::state::AppState;
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse,
    RefreshWorkResponse, UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use livrarr_db::{
    AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateWorkDbRequest, LibraryItemDb, NotificationDb,
    UpdateWorkUserFieldsDbRequest, WorkDb,
};
use livrarr_domain::Work;

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
    let term = q.term.unwrap_or_default();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }

    // Determine target language (default to primary from metadata config).
    let cfg = state.db.get_metadata_config().await.ok();
    let default_lang = cfg
        .as_ref()
        .and_then(|c| c.languages.first().cloned())
        .unwrap_or_else(|| "en".to_string());
    let lang = q.lang.as_deref().unwrap_or(&default_lang);

    // Validate language code.
    if lang != "en" && !livrarr_metadata::language::is_supported_language(lang) {
        return Err(ApiError::BadRequest(format!(
            "unsupported language: {lang}"
        )));
    }

    // Non-English: direct Goodreads search with regex parsing.
    if lang != "en" {
        let lang_owned = lang.to_string();

        // Rate limit outbound Goodreads requests.
        state.goodreads_rate_limiter.acquire().await;

        let search_url = format!(
            "https://www.goodreads.com/search?q={}",
            urlencoding::encode(&term)
        );

        let fetch_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            state
                .http_client
                .get(&search_url)
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
                )
                .header("Accept-Language", "en-US,en;q=0.9")
                .send(),
        )
        .await;

        let response = match fetch_result {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                state
                    .provider_health
                    .set_error("goodreads", format!("HTTP error: {e}"))
                    .await;
                tracing::warn!("Goodreads search fetch failed: {e}");
                return Ok(Json(vec![]));
            }
            Err(_) => {
                state
                    .provider_health
                    .set_error("goodreads", "timeout".into())
                    .await;
                tracing::warn!("Goodreads search fetch timed out");
                return Ok(Json(vec![]));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            state
                .provider_health
                .set_error("goodreads", format!("HTTP {status}"))
                .await;
            tracing::warn!(%status, "Goodreads search returned non-success status");
            return Ok(Json(vec![]));
        }

        let raw_html = match response.text().await {
            Ok(html) => html,
            Err(e) => {
                tracing::warn!("Goodreads search body read failed: {e}");
                return Ok(Json(vec![]));
            }
        };

        // Anti-bot detection — error, no fallback.
        if livrarr_metadata::llm_scraper::is_anti_bot_page(&raw_html) {
            state
                .provider_health
                .set_error("goodreads", "anti-bot challenge detected".into())
                .await;
            tracing::warn!("Goodreads search: anti-bot page detected");
            return Ok(Json(vec![]));
        }

        state.provider_health.clear_error("goodreads").await;

        // Direct regex parsing of search results.
        let parsed = livrarr_metadata::goodreads::parse_search_html(&raw_html);

        // Parser drift detection: if the HTML had book rows but none passed
        // validation, Goodreads likely changed their markup.
        if parsed.is_empty() && raw_html.contains("itemtype=\"http") {
            tracing::warn!(
                "Goodreads parser drift: HTML contains schema.org Book rows but 0 passed \
                 validation. HTML structure may have changed — please report this at \
                 https://github.com/kkodecs/livrarr/issues"
            );
        }

        // Include detail_url directly in each result for pass-through to add.
        let api_results: Vec<WorkSearchResult> = parsed
            .into_iter()
            .map(|r| {
                let full_url = if r.detail_url.starts_with('/') {
                    format!("https://www.goodreads.com{}", r.detail_url)
                } else {
                    r.detail_url.clone()
                };
                let validated_url = if livrarr_metadata::goodreads::validate_detail_url(&full_url) {
                    Some(full_url)
                } else {
                    None
                };
                WorkSearchResult {
                    ol_key: None,
                    title: r.title,
                    author_name: r.author.unwrap_or_default(),
                    author_ol_key: None,
                    year: r.year,
                    cover_url: r.cover_url,
                    description: None,
                    series_name: r.series_name,
                    series_position: r.series_position,
                    source: Some("Goodreads".to_string()),
                    source_type: Some("goodreads".to_string()),
                    language: Some(lang_owned.clone()),
                    detail_url: validated_url,
                    rating: r.rating,
                }
            })
            .collect();

        return Ok(Json(api_results));
    }

    // English: existing OpenLibrary flow.
    let results = lookup_openlibrary(&state.http_client, &term).await?;
    let results = results.0; // unwrap Json

    if !results.is_empty() {
        return Ok(Json(results));
    }

    // Primary source returned nothing — try LLM as fallback if configured.
    let cfg = state.db.get_metadata_config().await.ok();
    if let Some(cleaned) =
        llm_clean_search_results(&state.http_client, cfg.as_ref(), &term, &[], false).await
    {
        return Ok(Json(cleaned));
    }

    Ok(Json(vec![]))
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
    // Check duplicate by ol_key (only when provided).
    if let Some(ref ol_key) = req.ol_key {
        if state.db.work_exists_by_ol_key(user_id, ol_key).await? {
            return Err(ApiError::Conflict {
                reason: "work already exists".into(),
            });
        }
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
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await?;
            crate::handlers::author::spawn_bibliography_fetch((*state).clone(), author.id, user_id);
            (Some(author.id), true)
        }
    };

    let cover_url = req.cover_url.clone();

    // Detail URL comes from the frontend (passed through from search results).
    let detail_url = req.detail_url.clone();

    let work = state
        .db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: req.title,
            author_name: req.author_name,
            author_id,
            ol_key: req.ol_key.clone(),
            gr_key: None,
            year: req.year,
            cover_url: req.cover_url,
            metadata_source: req.metadata_source,
            detail_url,
            language: req.language,
            import_id: None,
            series_id: None,
            series_name: None,
            series_position: None,
            monitor_ebook: false,
            monitor_audiobook: false,
        })
        .await?;

    // Download cover image in background (best-effort, don't fail the add).
    // Unwrap proxy URLs back to the original external URL before downloading.
    // Foreign works: save as thumbnail ({id}_thumb.jpg) — enrichment saves hi-res later.
    // English works: save as main cover ({id}.jpg).
    let is_foreign = livrarr_metadata::language::is_foreign_source(work.metadata_source.as_deref());
    let cover_url = cover_url.map(|u| unproxy_cover_url(&u));
    if let Some(url) = cover_url {
        if livrarr_metadata::llm_scraper::validate_cover_url(&url, "").is_some() {
            // Cover URL came from a remote/user-supplied source — use SSRF-safe client.
            let http = state.http_client_safe.clone();
            let covers_dir = state.data_dir.join("covers");
            let work_id = work.id;
            let save_as_thumb = is_foreign;
            tokio::spawn(async move {
                if save_as_thumb {
                    let _ = download_cover_as(&http, &url, &covers_dir, work_id, "_thumb").await;
                } else {
                    let _ = download_cover(&http, &url, &covers_dir, work_id).await;
                }
            });
        } else {
            tracing::warn!(url = %url, "cover URL rejected by SSRF validation");
        }
    }

    // Deferred enrichment: skip inline enrichment, leave as pending for background job.
    if req.defer_enrichment {
        return Ok(AddWorkResponse {
            work: work_to_detail(&work),
            author_created,
            messages: vec![],
        });
    }

    // Foreign-language works: enrich from detail page (Goodreads etc.) if available.
    // English works: enrich from Hardcover + OL + Audnexus.
    let outcome = if livrarr_metadata::language::is_foreign_source(work.metadata_source.as_deref())
    {
        if work.detail_url.is_some() {
            super::enrichment::enrich_foreign_work(state, &work).await
        } else {
            // No detail URL — skip enrichment, mark as skipped.
            let _ = state.db.set_enrichment_status_skipped(work.id).await;
            let mut skipped_work = work;
            skipped_work.enrichment_status = livrarr_domain::EnrichmentStatus::Skipped;
            return Ok(AddWorkResponse {
                work: work_to_detail(&skipped_work),
                author_created,
                messages: vec![],
            });
        }
    } else {
        super::enrichment::enrich_work(state, &work).await
    };
    let enriched_work = match state
        .db
        .update_work_enrichment(user_id, work.id, outcome.request)
        .await
    {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(work_id = work.id, "update_work_enrichment failed: {e}");
            work
        }
    };

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
    Query(pq): Query<crate::PaginationQuery>,
) -> Result<Json<crate::PaginatedResponse<WorkDetailResponse>>, ApiError> {
    let user_id = ctx.user.id;
    let page = pq.page();
    let page_size = pq.page_size();
    let (works, total) = state
        .db
        .list_works_paginated(user_id, page, page_size)
        .await?;
    let work_ids: Vec<i64> = works.iter().map(|w| w.id).collect();
    let page_items = state
        .db
        .list_library_items_by_work_ids(user_id, &work_ids)
        .await?;

    let mut results: Vec<WorkDetailResponse> = works.iter().map(work_to_detail).collect();
    for detail in &mut results {
        detail.library_items = page_items
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
    Ok(Json(crate::PaginatedResponse {
        items: results,
        total,
        page,
        page_size,
    }))
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
    let user_id = ctx.user.id;

    // Reject oversized uploads (413 Payload Too Large).
    if body.len() > MAX_COVER_BYTES {
        return Err(ApiError::PayloadTooLarge {
            max_bytes: MAX_COVER_BYTES,
        });
    }

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
    // Atomic write: .tmp → fsync → rename.
    let tmp_path = cover_path.with_extension("jpg.tmp");
    let tmp_for_blocking = tmp_path.clone();
    let target = cover_path.clone();
    let body_vec = body.to_vec();
    let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_for_blocking)?;
        f.write_all(&body_vec)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_for_blocking, &target)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn error writing cover: {e}")))?;
    if let Err(e) = write_result {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(ApiError::Internal(format!("failed to write cover: {e}")));
    }

    // Delete stale thumbnail so it gets regenerated from the new cover.
    let thumb_path = covers_dir.join(format!("{id}_thumb.jpg"));
    let _ = tokio::fs::remove_file(&thumb_path).await;

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

    // Reset legacy enrichment_retry table state. The new queue path manages its
    // own retry state via provider_retry_state (cleared by
    // enrichment_service.reset_for_manual_refresh below); the legacy reset is
    // still needed for foreign works and for any in-flight legacy retry timer.
    if let Err(e) =
        livrarr_db::EnrichmentRetryDb::reset_enrichment_for_refresh(&state.db, user_id, id).await
    {
        tracing::warn!("reset_enrichment_for_refresh failed: {e}");
    }

    // Re-enrich — route to the correct enrichment function.
    let is_foreign = livrarr_metadata::language::is_foreign_source(work.metadata_source.as_deref());
    let (enriched, mut messages) = if is_foreign && work.detail_url.is_some() {
        // Foreign-language works still use the legacy direct path. Migrating
        // those alongside cover anti-bot detection (R-13) and dependent-step
        // LLM orchestration is a follow-on cutover step.
        let outcome = super::enrichment::enrich_foreign_work(&state, &work).await;
        let enriched = match state
            .db
            .update_work_enrichment(user_id, id, outcome.request)
            .await
        {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    work_id = id,
                    "update_work_enrichment failed during foreign refresh: {e}"
                );
                work.clone()
            }
        };
        (enriched, outcome.messages)
    } else {
        // English-language works: route through DefaultProviderQueue +
        // EnrichmentServiceImpl. First call-site cutover (Phase 1.5).
        // Known regression vs. legacy until Phase 2: GR cover fallback for
        // works whose HC/OL covers are missing or sub-50KB (R-13 — scheduled).
        if let Err(e) = livrarr_metadata::EnrichmentService::reset_for_manual_refresh(
            state.enrichment_service.as_ref(),
            user_id,
            id,
        )
        .await
        {
            tracing::warn!("enrichment_service.reset_for_manual_refresh failed: {e}");
        }

        let result = match livrarr_metadata::EnrichmentService::enrich_work(
            state.enrichment_service.as_ref(),
            user_id,
            id,
            livrarr_metadata::EnrichmentMode::HardRefresh,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("enrichment_service.enrich_work failed: {e}");
                return Ok(Json(RefreshWorkResponse {
                    work: work_to_detail(&work),
                    messages: vec![format!("enrichment failed: {e}")],
                }));
            }
        };

        let enriched = result.work.clone();

        // Download cover to disk if the queue produced a cover_url. Mirrors
        // the legacy enrich_work cover-download behavior so on-disk
        // covers/{id}.jpg stays in sync with the new path.
        if let Some(ref cover_url) = enriched.cover_url {
            let covers_dir = state.data_dir.join("covers");
            if let Err(e) = tokio::fs::create_dir_all(&covers_dir).await {
                tracing::warn!("create_dir_all for covers failed: {e}");
            }
            if let Ok(resp) = state.http_client_safe.get(cover_url).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        let path = covers_dir.join(format!("{id}.jpg"));
                        let tmp = path.with_extension("jpg.tmp");
                        if tokio::fs::write(&tmp, &bytes).await.is_ok()
                            && tokio::fs::rename(&tmp, &path).await.is_ok()
                        {
                            let thumb_path = covers_dir.join(format!("{id}_thumb.jpg"));
                            let _ = tokio::fs::remove_file(&thumb_path).await;
                        }
                    }
                }
            }
        }

        let mut messages: Vec<String> = result
            .provider_outcomes
            .iter()
            .filter(|(_, oc)| {
                !matches!(
                    oc,
                    livrarr_domain::OutcomeClass::Success | livrarr_domain::OutcomeClass::NotFound
                )
            })
            .map(|(p, oc)| format!("{p:?}: {oc:?}"))
            .collect();
        if result.merge_deferred {
            messages.push("Merge deferred — retry pending".to_string());
        }
        (enriched, messages)
    };

    // TAG-V21-004: rewrite tags on existing library items after re-enrichment.
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

    // Deduplication: reject if a refresh is already running for this user.
    {
        let mut guard = state.refresh_in_progress.lock().unwrap();
        if !guard.insert(user_id) {
            return Err(ApiError::Conflict {
                reason: "Refresh already in progress".to_string(),
            });
        }
    }

    // RAII guard — handles cleanup on all paths (empty, error, panic, completion).
    let refresh_guard = RefreshGuard {
        user_id,
        set: state.refresh_in_progress.clone(),
    };

    let works = state.db.list_works(user_id).await?;

    if works.is_empty() {
        return Ok(axum::http::StatusCode::ACCEPTED);
        // refresh_guard dropped here
    }

    let total = works.len();
    tokio::spawn(async move {
        let _guard = refresh_guard; // ensure guard lives for the task's lifetime
        let mut enriched = 0usize;
        let mut failed = 0usize;

        for work in &works {
            let is_foreign =
                livrarr_metadata::language::is_foreign_source(work.metadata_source.as_deref());
            let enrich_fut = if is_foreign && work.detail_url.is_some() {
                futures::future::Either::Left(super::enrichment::enrich_foreign_work(&state, work))
            } else {
                futures::future::Either::Right(super::enrichment::enrich_work(&state, work))
            };
            let outcome =
                tokio::time::timeout(std::time::Duration::from_secs(30), enrich_fut).await;

            match outcome {
                Ok(o) => {
                    match state
                        .db
                        .update_work_enrichment(user_id, work.id, o.request)
                        .await
                    {
                        Ok(enriched_work) => {
                            enriched += 1;
                            // Retag library files with updated metadata.
                            if let Ok(taggable) =
                                state.db.list_taggable_items_by_work(user_id, work.id).await
                            {
                                if !taggable.is_empty() {
                                    let _ = super::import::retag_library_items(
                                        &state,
                                        &enriched_work,
                                        &taggable,
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(_) => {
                            failed += 1;
                        }
                    }
                }
                Err(_) => {
                    failed += 1;
                }
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

        // _guard dropped here — RefreshGuard::drop removes user_id from set.
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
