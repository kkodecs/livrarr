//! Inline enrichment — Hardcover, OpenLibrary, Audnexus.
//!
//! Called synchronously at work-add and refresh time.
//! Per spec: ENRICH-001 through ENRICH-010.

use std::time::Duration;

use livrarr_db::{ConfigDb, EnrichmentStatus, NarrationType, UpdateWorkEnrichmentDbRequest};
use livrarr_domain::Work;
use tracing::warn;

use livrarr_metadata::goodreads::{
    self, fetch_goodreads_detail, fetch_goodreads_html, GoodreadsFetchError, GOODREADS_BASE_URL,
};
use livrarr_metadata::hardcover::{fetch_hardcover_editions, query_hardcover};
use livrarr_metadata::openlibrary::query_ol_detail;

use crate::state::AppState;

/// Atomically write a cover file: `path.tmp` → fsync → rename over `path`.
/// On any failure the `.tmp` is cleaned up so no partial file leaks onto disk.
async fn atomic_write_cover(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
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

/// Result of enrichment — data to write + messages for the user.
pub struct EnrichmentOutcome {
    pub request: UpdateWorkEnrichmentDbRequest,
    pub messages: Vec<String>,
}

/// Run enrichment for a work. Returns the update request and user-facing messages.
/// Never fails — provider errors are logged and result in partial/failed status.
pub async fn enrich_work(state: &AppState, work: &Work) -> EnrichmentOutcome {
    let cfg = match state.db.get_metadata_config().await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read metadata config: {e}");
            return empty_outcome("Enrichment skipped: config unavailable");
        }
    };

    let work_id = work.id;
    let title = &work.title;
    let author = &work.author_name;
    let mut messages = Vec::new();
    let mut req = UpdateWorkEnrichmentDbRequest {
        title: None,
        subtitle: None,
        original_title: None,
        author_name: None,
        description: None,
        year: None,
        series_name: None,
        series_position: None,
        genres: None,
        language: None,
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        enrichment_status: EnrichmentStatus::Failed,
        enrichment_source: None,
        cover_url: None,
    };

    let mut sources: Vec<&str> = Vec::new();
    let budget = tokio::time::Instant::now();
    let max_budget = Duration::from_secs(30);
    let per_provider = Duration::from_secs(10);

    // Per-provider tracking for accurate status derivation (F8).
    let mut hc_attempted = false;
    let mut hc_succeeded = false;
    let mut ol_attempted = false;
    let mut ol_succeeded = false;
    let mut audnexus_attempted = false;
    let mut audnexus_succeeded = false;

    // --- 1. Hardcover ---
    let hc_token = cfg.hardcover_api_token.as_deref().map(|t| t.trim());
    if let Some(token) = hc_token.filter(|t| !t.is_empty() && cfg.hardcover_enabled) {
        hc_attempted = true;
        let clean_token = token
            .strip_prefix("Bearer ")
            .or_else(|| token.strip_prefix("bearer "))
            .unwrap_or(token);

        match tokio::time::timeout(
            per_provider,
            query_hardcover(&state.http_client, title, author, clean_token, &cfg),
        )
        .await
        {
            Ok(Ok(hc)) => {
                // Use HC title to fix capitalization from OL/search.
                req.title = hc.title;
                // Subtitle intentionally not used — Hardcover data quality is unreliable
                // (e.g., Chinese subtitles on English books). Series info from
                // featured_series is more reliable for disambiguation.
                req.original_title = hc.original_title;
                req.description = hc.description;
                // F5: Only populate series fields if the work doesn't already have user-set values.
                if work.series_name.is_none() {
                    req.series_name = hc.series_name;
                }
                if work.series_position.is_none() {
                    req.series_position = hc.series_position;
                }
                req.genres = hc.genres;
                req.page_count = hc.page_count;
                req.publisher = hc.publisher;
                req.publish_date = hc.publish_date;
                // Derive year from publish_date (e.g. "1965-08-01" → 1965).
                if req.year.is_none() {
                    req.year = req
                        .publish_date
                        .as_deref()
                        .and_then(|d| d.get(..4))
                        .and_then(|y| y.parse::<i32>().ok());
                }
                req.hc_key = hc.hc_key.clone();
                req.isbn_13 = hc.isbn_13;
                req.rating = hc.rating;
                req.rating_count = hc.rating_count;
                if hc.cover_url.is_some() && !work.cover_manual {
                    req.cover_url = hc.cover_url;
                }
                // F7: Fetch edition detail with language filtering for better ISBN.
                if let Some(ref hc_id) = hc.hc_key {
                    if let Ok(Ok(Some(isbn))) = tokio::time::timeout(
                        per_provider,
                        fetch_hardcover_editions(&state.http_client, hc_id, clean_token, "en"),
                    )
                    .await
                    {
                        req.isbn_13 = Some(isbn);
                    }
                }
                sources.push("hardcover");
                hc_succeeded = true;
            }
            Ok(Err(e)) => {
                warn!("Hardcover enrichment failed for <work {work_id}>: {e}");
                messages.push(format!("Hardcover: {e}"));
            }
            Err(_) => {
                warn!("Hardcover enrichment timed out for <work {work_id}>");
                messages.push("Hardcover: timed out".into());
            }
        }
    }

    // --- 2. OpenLibrary fallback ---
    // F6: Run OL when Hardcover failed/timed out/not configured, OR when description still missing.
    if (!hc_succeeded || req.description.is_none()) && budget.elapsed() < max_budget {
        if let Some(ol_key) = &work.ol_key {
            ol_attempted = true;
            match tokio::time::timeout(per_provider, query_ol_detail(&state.http_client, ol_key))
                .await
            {
                Ok(Ok(ol)) => {
                    if ol.description.is_some() {
                        req.description = ol.description;
                    }
                    if req.isbn_13.is_none() && ol.isbn_13.is_some() {
                        req.isbn_13 = ol.isbn_13;
                    }
                    sources.push("openlibrary");
                    ol_succeeded = true;
                }
                Ok(Err(e)) => {
                    warn!("OL detail failed for <work {work_id}>: {e}");
                }
                Err(_) => {
                    warn!("OL detail timed out for <work {work_id}>");
                }
            }
        }
    }

    // --- 3. Audnexus (always, if budget allows) ---
    if budget.elapsed() < max_budget {
        audnexus_attempted = true;
        match tokio::time::timeout(
            per_provider,
            query_audnexus(
                &state.http_client,
                &cfg.audnexus_url,
                work.asin.as_deref(),
                title,
                author,
            ),
        )
        .await
        {
            Ok(Ok(Some(audio))) => {
                req.narrator = Some(audio.narrators);
                req.duration_seconds = audio.duration_seconds;
                req.asin = audio.asin.or(work.asin.clone());
                if !audio.narrators_empty {
                    req.narration_type = Some(NarrationType::Human);
                }
                sources.push("audnexus");
                audnexus_succeeded = true;
            }
            Ok(Ok(None)) => {
                // No audiobook data — normal, not an error. Count as success.
                audnexus_succeeded = true;
            }
            Ok(Err(e)) => {
                warn!("Audnexus failed for <work {work_id}>: {e}");
            }
            Err(_) => {
                warn!("Audnexus timed out for <work {work_id}>");
            }
        }
    }

    // --- Set status and source (F8: per-provider derivation) ---
    let attempted = [hc_attempted, ol_attempted, audnexus_attempted];
    let succeeded = [hc_succeeded, ol_succeeded, audnexus_succeeded];
    let attempted_count = attempted.iter().filter(|&&x| x).count();
    let succeeded_count = succeeded.iter().filter(|&&x| x).count();

    if attempted_count == 0 {
        req.enrichment_status = EnrichmentStatus::Failed;
        if messages.is_empty() {
            messages.push("Enrichment failed: no providers available".into());
        }
    } else if hc_succeeded {
        // "Enriched" means Hardcover succeeded — it's the primary source.
        req.enrichment_status = EnrichmentStatus::Enriched;
        req.enrichment_source = Some(sources.join("+"));
        messages.insert(0, format!("Enriched from {}", sources.join(" + ")));
    } else if succeeded_count > 0 {
        // OL/Audnexus only = partial (Hardcover not available or failed).
        req.enrichment_status = EnrichmentStatus::Partial;
        req.enrichment_source = Some(sources.join("+"));
        messages.insert(
            0,
            format!("Partially enriched from {}", sources.join(" + ")),
        );
    } else {
        req.enrichment_status = EnrichmentStatus::Failed;
        if messages.is_empty() {
            messages.push("Enrichment failed: all providers failed".into());
        }
    }

    // Default language from metadata config if not set by any provider.
    if req.language.is_none() {
        req.language = cfg.languages.first().cloned();
    }

    // --- 4. Goodreads fallback for cover ---
    // If no cover from Hardcover/OL (or cover is a tiny thumbnail < 50KB),
    // search GR by title+author and fetch hi-res cover from the detail page.
    let cover_too_small = if let Some(ref url) = req.cover_url {
        // Probe the URL — if it's under 50KB it's a thumbnail, not a real cover.
        // URL came from an external metadata provider — use SSRF-safe client.
        match state.http_client_safe.get(url).send().await {
            Ok(resp) => resp
                .content_length()
                .map(|len| len < 50_000)
                .unwrap_or(false),
            Err(_) => false,
        }
    } else {
        false
    };
    let gr_eligible = (req.cover_url.is_none() || cover_too_small) && !work.cover_manual;
    let budget_ok = budget.elapsed() < max_budget;
    tracing::info!(
        work_id = work.id,
        cover_url = ?req.cover_url,
        cover_too_small,
        cover_manual = work.cover_manual,
        gr_eligible,
        budget_ok,
        budget_elapsed_ms = budget.elapsed().as_millis() as u64,
        "GR cover fallback check"
    );
    if gr_eligible && budget_ok {
        // Resolve a GR detail URL via one of three strategies (in priority order):
        //   1. Direct lookup by gr_key (most reliable, skips search entirely)
        //   2. Search by title+author
        //   3. Search with ASCII-stripped title (fallback for titles with diacritics)
        let detail_url_opt: Option<String> = if let Some(gr_key) =
            work.gr_key.as_deref().filter(|k| !k.is_empty())
        {
            tracing::info!(work_id = work.id, gr_key, "GR direct lookup by gr_key");
            Some(goodreads::detail_url_for_gr_key(GOODREADS_BASE_URL, gr_key))
        } else {
            state.goodreads_rate_limiter.acquire().await;
            let mut results = match tokio::time::timeout(
                per_provider,
                goodreads::search_goodreads(&state.http_client, GOODREADS_BASE_URL, title, author),
            )
            .await
            {
                Ok(Ok(hits)) => {
                    tracing::info!(work_id = work.id, count = hits.len(), "GR search parsed");
                    hits
                }
                Ok(Err(e)) => {
                    tracing::warn!(work_id = work.id, error = ?e, "GR search failed");
                    Vec::new()
                }
                Err(_) => {
                    tracing::warn!(work_id = work.id, "GR search timed out");
                    Vec::new()
                }
            };

            if results.is_empty() && !title.is_ascii() {
                let ascii_title: String = title.chars().filter(|c| c.is_ascii()).collect();
                tracing::info!(
                    work_id = work.id,
                    ascii_title = %ascii_title,
                    "GR search retry with ASCII title"
                );
                state.goodreads_rate_limiter.acquire().await;
                if let Ok(Ok(hits)) = tokio::time::timeout(
                    per_provider,
                    goodreads::search_goodreads(
                        &state.http_client,
                        GOODREADS_BASE_URL,
                        &ascii_title,
                        author,
                    ),
                )
                .await
                {
                    results = hits;
                }
            }

            results
                .into_iter()
                .next()
                .map(|top| goodreads::resolve_detail_url(GOODREADS_BASE_URL, &top.detail_url))
        };

        if let Some(detail_url) = detail_url_opt {
            if livrarr_metadata::goodreads::validate_detail_url(&detail_url) {
                state.goodreads_rate_limiter.acquire().await;
                let detail_result = tokio::time::timeout(
                    per_provider,
                    fetch_goodreads_detail(&state.http_client, &detail_url),
                )
                .await;

                match &detail_result {
                    Ok(Ok(_)) => {
                        tracing::info!(work_id = work.id, url = %detail_url, "GR detail page fetched")
                    }
                    Ok(Err(GoodreadsFetchError::AntiBot)) => {
                        // R-13: anti-bot challenge in the English cover-fallback path
                        // is now treated as a hard skip rather than a silent miss.
                        tracing::warn!(work_id = work.id, "GR detail blocked by anti-bot");
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(work_id = work.id, error = ?e, "GR detail fetch failed")
                    }
                    Err(_) => tracing::warn!(work_id = work.id, "GR detail fetch timed out"),
                }
                if let Ok(Ok(detail)) = detail_result {
                    tracing::info!(
                        work_id = work.id,
                        cover = ?detail.cover_url,
                        "GR detail parsed"
                    );
                    if let Some(ref cover) = detail.cover_url {
                        if livrarr_metadata::goodreads::validate_cover_url(cover) {
                            req.cover_url = Some(cover.clone());
                            if !sources.contains(&"goodreads") {
                                sources.push("goodreads");
                            }
                        }
                    }
                    if req.description.is_none() {
                        req.description = detail.description;
                    }
                }
            }
        }

        // GR provided some data → don't leave status as Failed (stops the retry loop).
        if req.enrichment_status == EnrichmentStatus::Failed
            && (req.cover_url.is_some() || req.description.is_some())
        {
            req.enrichment_status = EnrichmentStatus::Partial;
            req.enrichment_source = Some(sources.join("+"));
        }
    }

    // --- Download cover if we got a new URL ---
    if let Some(ref url) = req.cover_url {
        let covers_dir = state.data_dir.join("covers");
        if let Err(e) = tokio::fs::create_dir_all(&covers_dir).await {
            tracing::warn!("create_dir_all for covers failed: {e}");
        }
        // Cover URL came from an external metadata provider — use SSRF-safe client.
        if let Ok(resp) = state.http_client_safe.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    let path = covers_dir.join(format!("{}.jpg", work.id));
                    if let Err(e) = atomic_write_cover(&path, &bytes).await {
                        tracing::warn!("write cover file failed: {e}");
                    }
                    // Delete stale thumbnail so it gets regenerated from the new cover.
                    let thumb_path = covers_dir.join(format!("{}_thumb.jpg", work.id));
                    let _ = tokio::fs::remove_file(&thumb_path).await;
                }
            }
        }
    }

    EnrichmentOutcome {
        request: req,
        messages,
    }
}

fn empty_outcome(msg: &str) -> EnrichmentOutcome {
    EnrichmentOutcome {
        request: UpdateWorkEnrichmentDbRequest {
            title: None,
            subtitle: None,
            original_title: None,
            author_name: None,
            description: None,
            year: None,
            series_name: None,
            series_position: None,
            genres: None,
            language: None,
            page_count: None,
            duration_seconds: None,
            publisher: None,
            publish_date: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            narrator: None,
            narration_type: None,
            abridged: None,
            rating: None,
            rating_count: None,
            enrichment_status: EnrichmentStatus::Failed,
            enrichment_source: None,
            cover_url: None,
        },
        messages: vec![msg.to_string()],
    }
}

// =============================================================================
// Hardcover, OpenLibrary, LLM disambiguation
// =============================================================================
// `query_hardcover`, `fetch_hardcover_editions`, `HardcoverResult`,
// `query_ol_detail`, `OlDetailResult`, and `llm_disambiguate` were lifted to
// `livrarr_metadata::{hardcover, openlibrary}` in Phase 1.5. Behavior unchanged.

// =============================================================================
// Audnexus
// =============================================================================
// `query_audnexus`, `parse_audnexus`, and `AudnexusResult` were lifted to
// `livrarr_metadata::audnexus` in Phase 1.5 so the same code can serve both the
// inline pipeline (still on the legacy direct path) and `ProviderClient::Audnexus`
// behind `DefaultProviderQueue`. Behavior unchanged.

use livrarr_metadata::audnexus::query_audnexus;

// =============================================================================
// Foreign work enrichment — Goodreads / LLM scraper detail pages
// =============================================================================

/// LLM extraction result for a book detail page.
#[derive(serde::Deserialize)]
struct LlmDetailResult {
    description: Option<String>,
    series_name: Option<String>,
    series_position: Option<f64>,
    genres: Option<Vec<String>>,
    page_count: Option<i32>,
    publisher: Option<String>,
    publish_date: Option<String>,
    cover_url: Option<String>,
    rating: Option<f64>,
    rating_count: Option<i32>,
    isbn: Option<String>,
}

/// System prompt for detail page enrichment extraction.
const DETAIL_EXTRACTION_PROMPT: &str = r#"You are a metadata extraction tool. Extract book details from the provided book detail page HTML.

Return ONLY a JSON object with exactly these fields:
- "description": string or null (book description/synopsis, plain text, no HTML)
- "series_name": string or null (series name if this book is part of a series)
- "series_position": number or null (position in the series, e.g. 1, 2, 3)
- "genres": array of strings or null (genre/shelf tags, max 10)
- "page_count": integer or null
- "publisher": string or null
- "publish_date": string or null (in YYYY-MM-DD or YYYY format)
- "cover_url": string or null (full URL of the largest/highest resolution cover image)
- "rating": number or null (average rating, typically 1-5 scale)
- "rating_count": integer or null (number of ratings)
- "isbn": string or null (ISBN-13 if visible)

Rules:
- Return ONLY the JSON object, no markdown fences, no explanation
- If a field is not visible on the page, use null
- Do NOT invent or guess missing data
- For cover_url, prefer the largest image version available
- For description, extract only the book synopsis, not reviews or ads
- For genres, use the most specific applicable tags"#;

/// Enrich a foreign work from its detail page URL.
/// Primary: JSON-LD + regex parsing. Fallback: LLM extraction.
pub async fn enrich_foreign_work(state: &AppState, work: &Work) -> EnrichmentOutcome {
    let detail_url = match &work.detail_url {
        Some(url) if !url.is_empty() => url.clone(),
        _ => {
            return empty_outcome("Foreign enrichment skipped: no detail URL");
        }
    };

    // SSRF validation on the stored detail URL.
    if !livrarr_metadata::goodreads::validate_detail_url(&detail_url) {
        warn!(url = %detail_url, "Foreign enrichment: detail URL failed SSRF validation");
        return empty_outcome("Foreign enrichment skipped: invalid detail URL");
    }

    let mut messages = Vec::new();
    let mut req = UpdateWorkEnrichmentDbRequest {
        title: None,
        subtitle: None,
        original_title: None,
        author_name: None,
        description: None,
        year: None,
        series_name: None,
        series_position: None,
        genres: None,
        language: None,
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        enrichment_status: EnrichmentStatus::Failed,
        enrichment_source: None,
        cover_url: None,
    };

    // Rate limit outbound Goodreads requests.
    state.goodreads_rate_limiter.acquire().await;

    // --- Fetch detail page ---
    // The lifted helper handles the UA / Accept-Language headers, status code
    // check, and anti-bot detection that used to live inline here.
    let page_result = tokio::time::timeout(
        Duration::from_secs(15),
        fetch_goodreads_html(&state.http_client, &detail_url),
    )
    .await;

    let raw_html = match page_result {
        Ok(Ok(html)) => html,
        Ok(Err(GoodreadsFetchError::AntiBot)) => {
            warn!("Anti-bot page detected for work {}", work.id);
            messages.push("Detail page blocked by anti-bot protection".into());
            return EnrichmentOutcome {
                request: req,
                messages,
            };
        }
        Ok(Err(e)) => {
            warn!(
                "Foreign enrichment fetch failed for '{}': {:?}",
                work.title, e
            );
            messages.push(format!("Detail page fetch failed: {e:?}"));
            return EnrichmentOutcome {
                request: req,
                messages,
            };
        }
        Err(_) => {
            warn!("Foreign enrichment fetch timed out for work {}", work.id);
            messages.push("Detail page fetch timed out".into());
            return EnrichmentOutcome {
                request: req,
                messages,
            };
        }
    };

    // --- Primary: Direct JSON-LD + regex parsing ---
    let nfc = livrarr_metadata::normalize::nfc;
    let direct_result = livrarr_metadata::goodreads::parse_detail_html(&raw_html);

    let mut direct_success = false;

    if let Some(detail) = direct_result {
        let has_title = detail.title.is_some();
        let has_cover = detail.cover_url.is_some();
        let has_description = detail.description.is_some();

        if has_title || has_cover || has_description {
            direct_success = true;

            req.description = detail.description.map(|s| nfc(&s));

            if work.series_name.is_none() {
                req.series_name = detail.series_name.map(|s| nfc(&s));
            }
            if work.series_position.is_none() {
                req.series_position = detail.series_position;
            }

            req.genres = if detail.genres.is_empty() {
                None
            } else {
                Some(detail.genres.into_iter().map(|s| nfc(&s)).collect())
            };
            req.page_count = detail.page_count.filter(|&p| p > 0);
            req.publish_date = detail.publish_date;
            req.language = detail.language.map(|s| nfc(&s));
            req.rating = detail.rating;
            req.rating_count = detail.rating_count;
            req.isbn_13 = detail.isbn.filter(|s| s.len() >= 10);

            // Derive year from publish_date if not already set.
            if work.year.is_none() {
                req.year = req
                    .publish_date
                    .as_deref()
                    .and_then(|d| d.get(..4))
                    .and_then(|y| y.parse::<i32>().ok());
            }

            // --- Cover handling ---
            if let Some(ref cover_url_str) = detail.cover_url {
                if livrarr_metadata::goodreads::validate_cover_url(cover_url_str) {
                    req.cover_url = Some(cover_url_str.clone());

                    // Rate limit cover download.
                    state.goodreads_rate_limiter.acquire().await;

                    let covers_dir = state.data_dir.join("covers");
                    if let Err(e) = tokio::fs::create_dir_all(&covers_dir).await {
                        tracing::warn!("create_dir_all for covers failed: {e}");
                    }
                    if let Ok(resp) = state.http_client_safe.get(cover_url_str).send().await {
                        if resp.status().is_success() {
                            if let Ok(bytes) = resp.bytes().await {
                                let path = covers_dir.join(format!("{}.jpg", work.id));
                                if let Err(e) = atomic_write_cover(&path, &bytes).await {
                                    tracing::warn!("write cover file failed: {e}");
                                }
                                // Keep thumbnail — useful for list views.
                            }
                        }
                    }
                }
            }
        }
    }

    // --- Fallback: LLM extraction (only if direct parsing failed) ---
    if !direct_success {
        tracing::info!(
            work_id = work.id,
            "Direct parsing returned no useful data, attempting LLM fallback"
        );

        let cfg = match state.db.get_metadata_config().await {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read metadata config for LLM fallback: {e}");
                messages.push("Direct parsing failed; LLM fallback unavailable".into());
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
        };

        if !cfg.llm_enabled {
            messages.push("Direct parsing failed; LLM not configured for fallback".into());
            return EnrichmentOutcome {
                request: req,
                messages,
            };
        }

        let endpoint = match cfg.llm_endpoint.as_deref().filter(|s| !s.is_empty()) {
            Some(e) => e,
            None => {
                messages.push("Direct parsing failed; LLM endpoint not set".into());
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
        };
        let api_key = match cfg.llm_api_key.as_deref().filter(|s| !s.is_empty()) {
            Some(k) => k,
            None => {
                messages.push("Direct parsing failed; LLM API key not set".into());
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
        };
        let model = match cfg.llm_model.as_deref().filter(|s| !s.is_empty()) {
            Some(m) => m,
            None => {
                messages.push("Direct parsing failed; LLM model not set".into());
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
        };

        let cleaned = livrarr_metadata::llm_scraper::clean_html_for_llm(&raw_html);
        if cleaned.is_empty() {
            messages.push("Detail page empty after cleaning (LLM fallback)".into());
            return EnrichmentOutcome {
                request: req,
                messages,
            };
        }

        let llm_url = format!(
            "{}chat/completions",
            endpoint.trim_end_matches('/').to_owned() + "/"
        );

        let lang_hint = work
            .language
            .as_deref()
            .and_then(livrarr_metadata::language::get_language_info)
            .map(|info| info.english_name)
            .unwrap_or("the original");

        let user_prompt = format!(
            "This book is in {lang_hint}. Extract book details from this page. \
             For the description, use ONLY text in {lang_hint} or English. \
             If the description is in a different language, return null for description.\n\n{}",
            cleaned
        );

        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": DETAIL_EXTRACTION_PROMPT},
                {"role": "user", "content": user_prompt},
            ],
            "max_tokens": 4000,
            "temperature": 0.0,
        });

        let llm_result = tokio::time::timeout(Duration::from_secs(30), async {
            let resp = state
                .http_client
                .post(&llm_url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("LLM request failed: {e}"))?;

            if !resp.status().is_success() {
                return Err(format!("LLM HTTP {}", resp.status()));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| format!("LLM parse error: {e}"))?;

            data.pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| "LLM response missing content".to_string())
        })
        .await;

        let llm_response = match llm_result {
            Ok(Ok(content)) => content,
            Ok(Err(e)) => {
                warn!("Foreign enrichment LLM failed for '{}': {e}", work.title);
                messages.push(format!("LLM fallback failed: {e}"));
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
            Err(_) => {
                warn!("Foreign enrichment LLM timed out for work {}", work.id);
                messages.push("LLM fallback timed out".into());
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
        };

        // Parse LLM response.
        let trimmed = llm_response.trim();
        let json_str = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed)
            .strip_suffix("```")
            .unwrap_or(trimmed)
            .trim();
        let json_str = json_str
            .find('{')
            .and_then(|start| json_str.rfind('}').map(|end| &json_str[start..=end]))
            .unwrap_or(json_str);

        let detail: LlmDetailResult = match serde_json::from_str(json_str) {
            Ok(d) => d,
            Err(e) => {
                let snippet: String = llm_response.chars().take(500).collect();
                warn!(
                    error = %e,
                    response_snippet = %snippet,
                    "Foreign enrichment: LLM returned malformed JSON"
                );
                messages.push("LLM returned unparseable response".into());
                return EnrichmentOutcome {
                    request: req,
                    messages,
                };
            }
        };

        req.description = detail.description.map(|s| nfc(&s));
        if work.series_name.is_none() {
            req.series_name = detail.series_name.map(|s| nfc(&s));
        }
        if work.series_position.is_none() {
            req.series_position = detail.series_position;
        }
        req.genres = detail
            .genres
            .map(|g| g.into_iter().map(|s| nfc(&s)).collect());
        req.page_count = detail.page_count.filter(|&p| p > 0);
        req.publisher = detail.publisher.map(|s| nfc(&s));
        req.publish_date = detail.publish_date;
        req.rating = detail.rating;
        req.rating_count = detail.rating_count;
        req.isbn_13 = detail.isbn.filter(|s| s.len() >= 10);

        if work.year.is_none() {
            req.year = req
                .publish_date
                .as_deref()
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse::<i32>().ok());
        }

        if let Some(ref cover_url_str) = detail.cover_url {
            if let Some(validated) =
                livrarr_metadata::llm_scraper::validate_cover_url(cover_url_str, "")
            {
                req.cover_url = Some(validated.clone());

                let covers_dir = state.data_dir.join("covers");
                if let Err(e) = tokio::fs::create_dir_all(&covers_dir).await {
                    tracing::warn!("create_dir_all for covers failed: {e}");
                }
                if let Ok(resp) = state.http_client_safe.get(&validated).send().await {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            let path = covers_dir.join(format!("{}.jpg", work.id));
                            if let Err(e) = atomic_write_cover(&path, &bytes).await {
                                tracing::warn!("write cover file failed: {e}");
                            }
                        }
                    }
                }
            }
        }

        messages.push("Enriched via LLM fallback (direct parsing failed)".into());
    }

    // --- Set status ---
    let has_description = req.description.is_some();
    let has_cover = req.cover_url.is_some();

    if has_description || has_cover {
        req.enrichment_status = if has_description && has_cover {
            EnrichmentStatus::Enriched
        } else {
            EnrichmentStatus::Partial
        };
        req.enrichment_source = Some(if direct_success {
            "goodreads_direct".to_string()
        } else {
            "web_search".to_string()
        });
        messages.insert(
            0,
            format!(
                "Enriched from detail page ({})",
                if has_description && has_cover {
                    "description + cover"
                } else if has_description {
                    "description"
                } else {
                    "cover"
                }
            ),
        );
    } else {
        req.enrichment_status = EnrichmentStatus::Partial;
        req.enrichment_source = Some("web_search".to_string());
        messages.push("Detail page enrichment: no description or cover extracted".into());
    }

    // Set language from the work's existing metadata.
    if req.language.is_none() {
        req.language = work.language.clone();
    }

    EnrichmentOutcome {
        request: req,
        messages,
    }
}
