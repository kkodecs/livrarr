//! Inline enrichment — Hardcover, OpenLibrary, Audnexus.
//!
//! Called synchronously at work-add and refresh time.
//! Per spec: ENRICH-001 through ENRICH-010.

use std::time::Duration;

use librarr_db::{ConfigDb, EnrichmentStatus, NarrationType, UpdateWorkEnrichmentDbRequest};
use librarr_domain::Work;
use tracing::warn;

use librarr_db::MetadataConfig;

use crate::state::AppState;

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
                warn!("Hardcover enrichment failed for '{title}': {e}");
                messages.push(format!("Hardcover: {e}"));
            }
            Err(_) => {
                warn!("Hardcover enrichment timed out for '{title}'");
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
                    warn!("OL detail failed for '{title}': {e}");
                }
                Err(_) => {
                    warn!("OL detail timed out for '{title}'");
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
                warn!("Audnexus failed for '{title}': {e}");
            }
            Err(_) => {
                warn!("Audnexus timed out for '{title}'");
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

    // --- Download cover if we got a new URL ---
    if let Some(ref url) = req.cover_url {
        let covers_dir = state.data_dir.join("covers");
        let _ = tokio::fs::create_dir_all(&covers_dir).await;
        if let Ok(resp) = state.http_client.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    let path = covers_dir.join(format!("{}.jpg", work.id));
                    let _ = tokio::fs::write(&path, &bytes).await;
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
    http: &librarr_http::HttpClient,
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

    // Quote the title for exact matching — without quotes, Hardcover
    // returns partial matches (e.g., comic adaptations) that flood results.
    let search_term = format!("\"{title}\"");
    let body = serde_json::json!({
        "query": query,
        "variables": {"query": search_term}
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
    http: &librarr_http::HttpClient,
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
        .post("https://api.hardcover.app/v1/graphql")
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
    http: &librarr_http::HttpClient,
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
    http: &librarr_http::HttpClient,
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
    http: &librarr_http::HttpClient,
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
