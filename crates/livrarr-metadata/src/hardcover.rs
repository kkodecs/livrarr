//! Hardcover GraphQL client.
//!
//! Lifted out of `livrarr-server/src/handlers/enrichment.rs` so the same code
//! can serve the legacy direct path AND `ProviderClient::Hardcover` behind
//! `DefaultProviderQueue`. Behavior unchanged from the original.

use livrarr_db::MetadataConfig;
use livrarr_http::HttpClient;
use serde_json::Value;

#[derive(Debug)]
pub enum HardcoverError {
    NoResults,
    NoMatch(String),
    Http(String),
}

impl std::fmt::Display for HardcoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoResults => write!(f, "no results"),
            Self::NoMatch(detail) => write!(f, "no match: {detail}"),
            Self::Http(msg) => write!(f, "{msg}"),
        }
    }
}

/// Hardcover GraphQL API endpoint.
pub const HARDCOVER_API_URL: &str = "https://api.hardcover.app/v1/graphql";

/// Parsed subset of a Hardcover search hit.
#[derive(Debug, Clone)]
pub struct HardcoverResult {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub page_count: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub hc_key: Option<String>,
    pub isbn_13: Option<String>,
    pub cover_url: Option<String>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
}

/// Search Hardcover for a book matching `title` + `author`. Tier 1 = exact
/// case-insensitive title + author match (highest `users_read_count` wins).
/// Tier 2 = LLM disambiguation when no exact match.
pub async fn query_hardcover(
    http: &HttpClient,
    title: &str,
    author: &str,
    token: &str,
    metadata_cfg: &MetadataConfig,
) -> Result<HardcoverResult, HardcoverError> {
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
        .map_err(|e| HardcoverError::Http(format!("request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(HardcoverError::Http(format!("HTTP {}", resp.status())));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| HardcoverError::Http(format!("parse error: {e}")))?;

    let hits = data
        .pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    if hits.is_empty() {
        return Err(HardcoverError::NoResults);
    }

    // Tier 1: exact title + author match (case-insensitive), highest users_read_count wins.
    // Use `clean_title` (the same value we searched with) so we match against what
    // we actually asked Hardcover for, not the unstripped original.
    let title_lower = clean_title.trim().to_lowercase();
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
    // The early-return on `hits.is_empty()` above prevents wasted LLM calls
    // for genuine HC misses; once HC returned candidates we always ask the
    // LLM to disambiguate (matches alpha2 behavior).
    let doc_idx = match best_idx {
        Some(i) => i,
        None => match llm_disambiguate(http, metadata_cfg, title, author, &hits).await {
            Ok(Some(idx)) => {
                tracing::info!(title = %title, chosen_idx = idx, "LLM selected Hardcover result");
                idx
            }
            Ok(None) => return Err(HardcoverError::NoMatch("LLM returned no selection".into())),
            Err(e) => {
                tracing::warn!(title = %title, error = %e, "LLM disambiguation failed");
                return Err(HardcoverError::NoMatch(format!("LLM: {e}")));
            }
        },
    };

    let doc = hits[doc_idx].get("document").ok_or(HardcoverError::Http(
        "selected result has no document".into(),
    ))?;

    let hc_title = doc
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let hc_key = doc
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
        hc_key,
        isbn_13,
        cover_url,
        rating,
        rating_count,
    })
}

/// Fetch edition data from Hardcover with language filtering (F7: SEARCH-010).
/// Returns the best ISBN from editions matching the preferred language.
pub async fn fetch_hardcover_editions(
    http: &HttpClient,
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

    let data: Value = resp
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

/// Ask an LLM to pick the best Hardcover result when exact title match fails.
/// Returns the index into `hits` of the best match, or None if LLM declines.
async fn llm_disambiguate(
    http: &HttpClient,
    cfg: &MetadataConfig,
    title: &str,
    author: &str,
    hits: &[Value],
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

    let data: Value = resp
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
