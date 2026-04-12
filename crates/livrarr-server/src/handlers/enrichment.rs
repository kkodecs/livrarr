//! Inline enrichment — Hardcover, OpenLibrary, Audnexus.
//!
//! Called synchronously at work-add and refresh time.
//! Per spec: ENRICH-001 through ENRICH-010.

use std::time::Duration;

use livrarr_db::{ConfigDb, EnrichmentStatus, NarrationType, UpdateWorkEnrichmentDbRequest};
use livrarr_domain::Work;
use tracing::warn;

use livrarr_db::MetadataConfig;

use crate::state::AppState;

/// Hardcover GraphQL API endpoint.
const HARDCOVER_API_URL: &str = "https://api.hardcover.app/v1/graphql";

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
        hardcover_id: None,
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
                req.hardcover_id = hc.hardcover_id.clone();
                req.isbn_13 = hc.isbn_13;
                req.rating = hc.rating;
                req.rating_count = hc.rating_count;
                if hc.cover_url.is_some() && !work.cover_manual {
                    req.cover_url = hc.cover_url;
                }
                // F7: Fetch edition detail with language filtering for better ISBN.
                if let Some(ref hc_id) = hc.hardcover_id {
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

    // --- Download cover if we got a new URL ---
    if let Some(ref url) = req.cover_url {
        let covers_dir = state.data_dir.join("covers");
        if let Err(e) = tokio::fs::create_dir_all(&covers_dir).await {
            tracing::warn!("create_dir_all for covers failed: {e}");
        }
        if let Ok(resp) = state.http_client.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    let path = covers_dir.join(format!("{}.jpg", work.id));
                    if let Err(e) = tokio::fs::write(&path, &bytes).await {
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
            hardcover_id: None,
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
// Hardcover
// =============================================================================

struct HardcoverResult {
    title: Option<String>,
    #[allow(dead_code)] // used in test infrastructure
    subtitle: Option<String>,
    original_title: Option<String>,
    description: Option<String>,
    series_name: Option<String>,
    series_position: Option<f64>,
    genres: Option<Vec<String>>,
    page_count: Option<i32>,
    publisher: Option<String>,
    publish_date: Option<String>,
    hardcover_id: Option<String>,
    isbn_13: Option<String>,
    cover_url: Option<String>,
    rating: Option<f64>,
    rating_count: Option<i32>,
}

async fn query_hardcover(
    http: &livrarr_http::HttpClient,
    title: &str,
    author: &str,
    token: &str,
    metadata_cfg: &MetadataConfig,
) -> Result<HardcoverResult, String> {
    // Search by title only — gets the best results for short/common titles.
    let query = r#"query SearchBooks($query: String!) {
        search(query: $query, query_type: "books", per_page: 25) {
            results
        }
    }"#;

    // Strip trailing parenthetical before searching — OL titles often include
    // series info like "(The Wheel of Time Book 2)" which breaks Hardcover's
    // exact-match search. The enrichment result will supply the canonical title.
    let clean_title = title
        .rfind('(')
        .filter(|_| title.ends_with(')'))
        .map(|i| title[..i].trim())
        .unwrap_or(title);
    // Quote the title for exact matching — without quotes, Hardcover
    // returns partial matches (e.g., comic adaptations) that flood results.
    let search_term = format!("\"{clean_title}\"");
    let body = serde_json::json!({
        "query": query,
        "variables": {"query": search_term}
    });

    let resp = http
        .post(HARDCOVER_API_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    let hits = data
        .pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    if hits.is_empty() {
        return Err("no results".into());
    }

    // Tier 1: exact title + author match (case-insensitive), highest users_read_count wins.
    let title_lower = title.trim().to_lowercase();
    let author_lower = author.trim().to_lowercase();
    let mut best_idx: Option<usize> = None;
    let mut best_urc: i64 = -1;

    for (i, hit) in hits.iter().enumerate() {
        let doc = match hit.get("document") {
            Some(d) => d,
            None => continue,
        };
        let doc_title = doc
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if doc_title != title_lower {
            continue;
        }
        // Check author match.
        let doc_authors = doc
            .get("author_names")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_lowercase())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !doc_authors.iter().any(|a| a == &author_lower) {
            continue;
        }
        let urc = doc
            .get("users_read_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if urc > best_urc {
            best_idx = Some(i);
            best_urc = urc;
        }
    }

    // Tier 2: LLM disambiguation when exact match fails (SEARCH-007).
    let doc_idx = match best_idx {
        Some(i) => i,
        None => match llm_disambiguate(http, metadata_cfg, title, author, &hits).await {
            Ok(Some(idx)) => {
                tracing::info!(title = %title, chosen_idx = idx, "LLM selected Hardcover result");
                idx
            }
            Ok(None) => return Err("no exact match and LLM returned no selection".into()),
            Err(e) => {
                tracing::warn!(title = %title, error = %e, "LLM disambiguation failed");
                return Err(format!("no exact match (LLM: {e})"));
            }
        },
    };

    let doc = hits[doc_idx]
        .get("document")
        .ok_or("selected result has no document")?;

    // Extract fields.
    let hc_title = doc
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let hardcover_id = doc
        .get("id")
        .map(|v| v.to_string().trim_matches('"').to_string());

    // Subtitle intentionally skipped — Hardcover data quality is unreliable.

    let description = doc
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let series_name = doc
        .pointer("/featured_series/series/name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let series_position = doc
        .pointer("/featured_series/position")
        .and_then(|v| v.as_f64());

    let genres = doc.get("genres").and_then(|g| g.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.contains('|'))
            .take(5)
            .collect()
    });

    let page_count = doc.get("pages").and_then(|v| v.as_i64()).map(|v| v as i32);

    let publish_date = doc
        .get("release_date")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let isbn_13 = doc.get("isbns").and_then(|v| v.as_array()).and_then(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str())
            .find(|s| s.len() == 13)
            .map(|s| s.to_string())
    });

    let cover_url = doc
        .pointer("/image/url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let rating = doc.get("rating").and_then(|v| v.as_f64());
    let rating_count = doc
        .get("ratings_count")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    Ok(HardcoverResult {
        title: hc_title,
        subtitle: None,
        original_title: None,
        description,
        series_name,
        series_position,
        genres,
        page_count,
        publisher: None,
        publish_date,
        hardcover_id,
        isbn_13,
        cover_url,
        rating,
        rating_count,
    })
}

/// Fetch edition data from Hardcover with language filtering (F7: SEARCH-010).
/// Returns the best ISBN from editions matching the preferred language.
async fn fetch_hardcover_editions(
    http: &livrarr_http::HttpClient,
    book_id: &str,
    token: &str,
    preferred_language: &str,
) -> Result<Option<String>, String> {
    let book_id_int: i64 = book_id.parse().map_err(|_| "invalid book ID".to_string())?;

    let query = r#"query GetEditions($bookId: Int!) {
        editions(where: {book_id: {_eq: $bookId}}, order_by: [{users_read_count: desc}], limit: 50) {
            isbn_13
            language {
                language
            }
        }
    }"#;

    let body = serde_json::json!({
        "query": query,
        "variables": {"bookId": book_id_int}
    });

    let resp = http
        .post(HARDCOVER_API_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("edition request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("edition HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("edition parse: {e}"))?;

    let editions = data
        .pointer("/data/editions")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();

    let preferred = preferred_language.to_lowercase();

    // Prefer editions matching preferred language with a valid ISBN-13.
    for edition in &editions {
        let lang = edition
            .pointer("/language/language")
            .and_then(|l| l.as_str())
            .unwrap_or("")
            .to_lowercase();
        if lang == preferred || lang.starts_with(&preferred) {
            if let Some(isbn) = edition
                .get("isbn_13")
                .and_then(|v| v.as_str())
                .filter(|s| s.len() == 13)
            {
                return Ok(Some(isbn.to_string()));
            }
        }
    }

    // Fallback: any edition with ISBN (already sorted by users_read_count desc).
    for edition in &editions {
        if let Some(isbn) = edition
            .get("isbn_13")
            .and_then(|v| v.as_str())
            .filter(|s| s.len() == 13)
        {
            return Ok(Some(isbn.to_string()));
        }
    }

    Ok(None)
}

// =============================================================================
// LLM disambiguation (SEARCH-007 tier 2)
// =============================================================================

/// Ask an LLM to pick the best Hardcover result when exact title match fails.
/// Returns the index into `hits` of the best match, or None if LLM declines.
async fn llm_disambiguate(
    http: &livrarr_http::HttpClient,
    cfg: &MetadataConfig,
    title: &str,
    author: &str,
    hits: &[serde_json::Value],
) -> Result<Option<usize>, String> {
    if !cfg.llm_enabled {
        return Err("LLM disabled".into());
    }
    let endpoint = cfg
        .llm_endpoint
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM not configured")?;
    let api_key = cfg
        .llm_api_key
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM API key not configured")?;
    let model = cfg
        .llm_model
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM model not configured")?;

    // Build candidate descriptions for the prompt.
    let mut candidates = String::new();
    for (i, hit) in hits.iter().enumerate() {
        let doc = match hit.get("document") {
            Some(d) => d,
            None => continue,
        };
        let t = doc.get("title").and_then(|v| v.as_str()).unwrap_or("?");
        let a = doc
            .pointer("/contributions/0/author/name")
            .and_then(|v| v.as_str())
            .or_else(|| doc.get("author").and_then(|v| v.as_str()))
            .unwrap_or("?");
        let year = doc
            .get("release_date")
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(..4))
            .unwrap_or("?");
        let urc = doc
            .get("users_read_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        candidates.push_str(&format!("{i}: \"{t}\" by {a} ({year}, {urc} readers)\n"));
    }

    let prompt = format!(
        "I'm looking for the book \"{title}\" by {author}.\n\n\
         These are the search results from a book database:\n{candidates}\n\
         Which result (by number) is the correct match? \
         Reply with ONLY the number. If none match, reply \"none\"."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 10,
        "temperature": 0.0,
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM HTTP {status}: {text}"));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("LLM parse error: {e}"))?;

    let answer = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();

    tracing::debug!(
        candidates_count = candidates.lines().count(),
        raw_answer = %answer,
        "LLM disambiguation"
    );

    if answer == "none" || answer.is_empty() {
        return Ok(None);
    }

    match answer.parse::<usize>() {
        Ok(idx) if idx < hits.len() => Ok(Some(idx)),
        _ => {
            tracing::warn!(answer = %answer, "LLM returned unparseable disambiguation result");
            Ok(None)
        }
    }
}

// =============================================================================
// OpenLibrary detail
// =============================================================================

struct OlDetailResult {
    description: Option<String>,
    isbn_13: Option<String>,
}

async fn query_ol_detail(
    http: &livrarr_http::HttpClient,
    ol_key: &str,
) -> Result<OlDetailResult, String> {
    let key = ol_key.trim_start_matches("/works/").trim_start_matches('/');

    // Fetch work detail.
    let url = format!("https://openlibrary.org/works/{key}.json");
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    let description = data.get("description").and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    });

    // Fetch editions for ISBN.
    let mut isbn_13 = None;
    let editions_url = format!("https://openlibrary.org/works/{key}/editions.json?limit=10");
    if let Ok(ed_resp) = http.get(&editions_url).send().await {
        if let Ok(ed_data) = ed_resp.json::<serde_json::Value>().await {
            if let Some(entries) = ed_data.get("entries").and_then(|e| e.as_array()) {
                for entry in entries {
                    if let Some(isbns) = entry.get("isbn_13").and_then(|i| i.as_array()) {
                        if let Some(isbn) = isbns.first().and_then(|v| v.as_str()) {
                            isbn_13 = Some(isbn.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(OlDetailResult {
        description,
        isbn_13,
    })
}

// =============================================================================
// Audnexus
// =============================================================================

struct AudnexusResult {
    narrators: Vec<String>,
    narrators_empty: bool,
    duration_seconds: Option<i32>,
    asin: Option<String>,
}

async fn query_audnexus(
    http: &livrarr_http::HttpClient,
    base_url: &str,
    asin: Option<&str>,
    title: &str,
    author: &str,
) -> Result<Option<AudnexusResult>, String> {
    let base = base_url.trim_end_matches('/');

    // Try by ASIN first.
    if let Some(asin) = asin {
        let url = format!("{base}/books/{asin}");
        if let Ok(resp) = http.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    return Ok(Some(parse_audnexus(&data, Some(asin))));
                }
            }
        }
    }

    // Fallback: search by title + author.
    let url = format!(
        "{base}/books?title={}&author={}",
        urlencoding(title),
        urlencoding(author),
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    let book = if data.is_array() {
        data.as_array().and_then(|a| a.first()).cloned()
    } else {
        Some(data)
    };

    match book {
        Some(b) => Ok(Some(parse_audnexus(&b, None))),
        None => Ok(None),
    }
}

fn parse_audnexus(data: &serde_json::Value, asin_hint: Option<&str>) -> AudnexusResult {
    let narrators = data
        .get("narrators")
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    n.get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let duration_seconds = data
        .get("runtimeLengthSec")
        .or_else(|| data.get("runtime_length_sec"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let asin = data
        .get("asin")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| asin_hint.map(|s| s.to_string()));

    let narrators_empty = narrators.is_empty();

    AudnexusResult {
        narrators,
        narrators_empty,
        duration_seconds,
        asin,
    }
}

/// Simple URL encoding for query parameters.
fn urlencoding(s: &str) -> String {
    s.replace(' ', "+")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('?', "%3F")
        .replace('#', "%23")
}

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
        hardcover_id: None,
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
    let page_result = tokio::time::timeout(Duration::from_secs(15), async {
        let resp = state
            .http_client
            .get(&detail_url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| format!("HTTP fetch failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        resp.text()
            .await
            .map_err(|e| format!("failed to read body: {e}"))
    })
    .await;

    let raw_html = match page_result {
        Ok(Ok(html)) => html,
        Ok(Err(e)) => {
            warn!("Foreign enrichment fetch failed for '{}': {e}", work.title);
            messages.push(format!("Detail page fetch failed: {e}"));
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

    // Anti-bot detection — error, NO fallback.
    if livrarr_metadata::llm_scraper::is_anti_bot_page(&raw_html) {
        warn!("Anti-bot page detected for work {}", work.id);
        messages.push("Detail page blocked by anti-bot protection".into());
        return EnrichmentOutcome {
            request: req,
            messages,
        };
    }

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
                    if let Ok(resp) = state.http_client.get(cover_url_str).send().await {
                        if resp.status().is_success() {
                            if let Ok(bytes) = resp.bytes().await {
                                let path = covers_dir.join(format!("{}.jpg", work.id));
                                if let Err(e) = tokio::fs::write(&path, &bytes).await {
                                    tracing::warn!("write cover file failed: {e}");
                                }
                                // Delete stale thumbnail so it gets regenerated.
                                let thumb_path = covers_dir.join(format!("{}_thumb.jpg", work.id));
                                let _ = tokio::fs::remove_file(&thumb_path).await;
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
                if let Ok(resp) = state.http_client.get(&validated).send().await {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            let path = covers_dir.join(format!("{}.jpg", work.id));
                            if let Err(e) = tokio::fs::write(&path, &bytes).await {
                                tracing::warn!("write cover file failed: {e}");
                            }
                            let thumb_path = covers_dir.join(format!("{}_thumb.jpg", work.id));
                            let _ = tokio::fs::remove_file(&thumb_path).await;
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
