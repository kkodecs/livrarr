//! Hardcover GraphQL client, consumed via `ProviderClient::Hardcover` (queue
//! dispatch and the identity-resolution fan-out).

use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::settings::MetadataConfig;
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use livrarr_http::HttpClient;
use serde_json::Value;

#[derive(Debug)]
pub enum HardcoverError {
    NoResults,
    NoMatch(String),
    Http(String),
    /// The outbound queue's breaker was Open for the Hardcover bucket — no
    /// HTTP was attempted. Carries the retry-after duration (R-11: the
    /// enrichment-surface caller must map this to `WillRetryReason::CircuitOpen`).
    CircuitOpen(Duration),
}

impl std::fmt::Display for HardcoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoResults => write!(f, "no results"),
            Self::NoMatch(detail) => write!(f, "no match: {detail}"),
            Self::Http(msg) => write!(f, "{msg}"),
            Self::CircuitOpen(d) => write!(f, "circuit open, retry after {d:?}"),
        }
    }
}

impl std::error::Error for HardcoverError {}

/// Format the GraphQL `SearchBooks` query string with the given `per_page` limit.
fn hc_search_graphql(per_page: u32) -> String {
    format!(
        r#"query SearchBooks($query: String!) {{
        search(query: $query, query_type: "books", per_page: {per_page}) {{
            results
        }}
    }}"#
    )
}

/// Build the JSON body for a Hardcover `SearchBooks` request.
///
/// `term` is used verbatim — callers must pre-format it (e.g. `"\"title\""` for
/// exact-match, bare value for ISBN/broad lookup).
pub fn hc_search_body(per_page: u32, term: &str) -> Value {
    serde_json::json!({
        "query": hc_search_graphql(per_page),
        "variables": {"query": term}
    })
}

/// POST a pre-built body to the Hardcover GraphQL endpoint and return the parsed
/// JSON response. Handles auth header, Content-Type, and HTTP status check.
pub async fn hc_post<F: HttpFetcher>(
    fetcher: &F,
    body: Value,
    token: &str,
    priority: RequestPriority,
) -> Result<Value, HardcoverError> {
    let req = FetchRequest {
        url: HARDCOVER_API_URL.to_string(),
        method: HttpMethod::Post,
        headers: vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: Some(
            serde_json::to_vec(&body)
                .map_err(|e| HardcoverError::Http(format!("body encode error: {e}")))?,
        ),
        timeout: Duration::from_secs(10),
        rate_bucket: RateBucket::Hardcover,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };

    let resp = match fetcher.fetch(req).await {
        Ok(r) => r,
        Err(FetchError::CircuitOpen { retry_after }) => {
            return Err(HardcoverError::CircuitOpen(retry_after));
        }
        Err(e) => return Err(HardcoverError::Http(e.to_string())),
    };

    if !(200..300).contains(&resp.status) {
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Failure);
        }
        return Err(HardcoverError::Http(format!("HTTP {}", resp.status)));
    }

    let parsed = serde_json::from_slice(&resp.body)
        .map_err(|e| HardcoverError::Http(format!("parse error: {e}")))?;
    outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Success);
    Ok(parsed)
}

/// Extract the `hits` array from a Hardcover search response value.
pub fn hc_extract_hits(data: &Value) -> Vec<Value> {
    data.pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Hardcover GraphQL API endpoint.
pub const HARDCOVER_API_URL: &str = "https://api.hardcover.app/v1/graphql";

/// Parsed subset of a Hardcover search hit.
#[derive(Debug, Clone)]
pub struct HardcoverResult {
    pub title: Option<String>,
    pub author_name: Option<String>,
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

/// First non-empty credited author from a search hit's `author_names` array.
/// Identity arbitration compares provider answers by title + author — an
/// authorless payload can manufacture a false quorum split (#148).
fn doc_author_name(doc: &Value) -> Option<String> {
    doc.get("author_names")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Search Hardcover for a book matching `title` + `author`. Tier 1 = exact
/// case-insensitive title + author match (highest `users_read_count` wins).
/// Tier 2 = LLM disambiguation when no exact match.
pub async fn query_hardcover<F: HttpFetcher>(
    fetcher: &F,
    http: &HttpClient,
    title: &str,
    author: &str,
    token: &str,
    metadata_cfg: &MetadataConfig,
    priority: RequestPriority,
) -> Result<HardcoverResult, HardcoverError> {
    // Search by title only — gets the best results for short/common titles.
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
    let body = hc_search_body(25, &search_term);
    let data = hc_post(fetcher, body, token, priority).await?;
    let hits = hc_extract_hits(&data);

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
        author_name: doc_author_name(doc),
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
///
/// Best-effort: the sole caller (`HardcoverClient::build_success`) swallows
/// every `Err` behind `if let Ok(Some(..))` while already holding a Success
/// payload — no error from here (including a breaker-open pause) can reach
/// outcome mapping or retry-budget accounting. The opaque `String` error is
/// therefore deliberate; a caller that ever propagates these errors must
/// first switch this to a typed error preserving `FetchError::CircuitOpen`.
pub async fn fetch_hardcover_editions<F: HttpFetcher>(
    fetcher: &F,
    book_id: &str,
    token: &str,
    preferred_language: &str,
    priority: RequestPriority,
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

    let req = FetchRequest {
        url: HARDCOVER_API_URL.to_string(),
        method: HttpMethod::Post,
        headers: vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: Some(
            serde_json::to_vec(&body).map_err(|e| format!("edition body encode error: {e}"))?,
        ),
        timeout: Duration::from_secs(10),
        rate_bucket: RateBucket::Hardcover,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };

    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| format!("edition request failed: {e}"))?;

    if !(200..300).contains(&resp.status) {
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Failure);
        }
        return Err(format!("edition HTTP {}", resp.status));
    }

    let data: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("edition parse: {e}"))?;
    outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Success);

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
        .timeout(std::time::Duration::from_secs(30))
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

pub async fn query_hardcover_by_isbn<F: HttpFetcher>(
    fetcher: &F,
    isbn: &str,
    token: &str,
    _metadata_cfg: &livrarr_domain::settings::MetadataConfig,
    priority: RequestPriority,
) -> Result<Option<HardcoverResult>, HardcoverError> {
    let body = hc_search_body(10, isbn);
    let data = hc_post(fetcher, body, token, priority).await?;
    let hits = hc_extract_hits(&data);

    for hit in &hits {
        let doc = match hit.get("document") {
            Some(d) => d,
            None => continue,
        };

        let hit_isbns = doc
            .get("isbns")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        if !hit_isbns.iter().any(|i| i == &isbn) {
            continue;
        }

        let hc_title = doc
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let hc_key = doc
            .get("id")
            .map(|v| v.to_string().trim_matches('"').to_string());

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

        return Ok(Some(HardcoverResult {
            title: hc_title,
            author_name: doc_author_name(doc),
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
        }));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_author_name_takes_first_credited_author() {
        let doc = serde_json::json!({"author_names": ["Jim Butcher", "Co Writer"]});
        assert_eq!(doc_author_name(&doc).as_deref(), Some("Jim Butcher"));
    }

    #[test]
    fn doc_author_name_absent_empty_or_blank_is_none() {
        assert_eq!(doc_author_name(&serde_json::json!({})), None);
        assert_eq!(
            doc_author_name(&serde_json::json!({"author_names": []})),
            None
        );
        assert_eq!(
            doc_author_name(&serde_json::json!({"author_names": ["  "]})),
            None
        );
    }

    // -------------------------------------------------------------------
    // Door-routing: hc_post / fetch_hardcover_editions go through the
    // HttpFetcher trait with the Hardcover rate bucket, POST, Bearer auth,
    // and a JSON body.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn hc_post_sends_hardcover_bucket_post_bearer_auth_and_json_body() {
        let canned = serde_json::json!({"data": {"search": {"results": {"hits": []}}}});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );
        let body = hc_search_body(25, "\"Test Title\"");

        hc_post(&fetcher, body.clone(), "test-token", RequestPriority::High)
            .await
            .unwrap();

        assert_eq!(fetcher.call_count(), 1);
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, HARDCOVER_API_URL);
        assert_eq!(req.rate_bucket, RateBucket::Hardcover);
        assert_eq!(req.method, HttpMethod::Post);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer test-token"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        let sent_body: Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
        assert_eq!(sent_body, body);
        assert_transport_params(req, RequestPriority::High);
    }

    /// The fixed transport parameters every Hardcover API request carries.
    /// `expected_priority` asserts the caller-supplied `RequestPriority` was
    /// threaded through to the request unchanged (B4) rather than the prior
    /// hardcoded `Normal`.
    fn assert_transport_params(req: &FetchRequest, expected_priority: RequestPriority) {
        assert_eq!(req.timeout, Duration::from_secs(10));
        assert_eq!(req.max_body_bytes, 2 * 1024 * 1024);
        assert!(!req.anti_bot_check);
        assert!(matches!(req.user_agent, UserAgentProfile::Server));
        assert_eq!(req.priority, expected_priority);
    }

    #[tokio::test]
    async fn fetch_hardcover_editions_sends_hardcover_bucket_post_bearer_auth_and_json_body() {
        let canned = serde_json::json!({"data": {"editions": []}});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        fetch_hardcover_editions(&fetcher, "123", "test-token", "en", RequestPriority::Low)
            .await
            .unwrap();

        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, HARDCOVER_API_URL);
        assert_eq!(req.rate_bucket, RateBucket::Hardcover);
        assert_eq!(req.method, HttpMethod::Post);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer test-token"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        assert!(req.body.is_some());
        assert_transport_params(req, RequestPriority::Low);
    }

    #[tokio::test]
    async fn hc_post_parses_canned_success_response_into_value() {
        let canned = serde_json::json!({
            "data": {"search": {"results": {"hits": [{"document": {"id": 42}}]}}}
        });
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );
        let body = hc_search_body(25, "\"Test Title\"");

        let result = hc_post(&fetcher, body, "tok", RequestPriority::Normal)
            .await
            .unwrap();

        assert_eq!(result, canned);
    }

    #[tokio::test]
    async fn fetch_hardcover_editions_returns_isbn_for_preferred_language() {
        let canned = serde_json::json!({
            "data": {
                "editions": [
                    {"isbn_13": "9780000000002", "language": {"language": "french"}},
                    {"isbn_13": "9780000000001", "language": {"language": "english"}}
                ]
            }
        });
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let result = fetch_hardcover_editions(&fetcher, "42", "tok", "en", RequestPriority::Normal)
            .await
            .unwrap();

        assert_eq!(result, Some("9780000000001".to_string()));
    }

    // -------------------------------------------------------------------
    // Error mapping: HttpFetcher failures map onto the HardcoverError
    // shapes callers match on (provider_client.rs WillRetry{ServerError}).
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn hc_post_maps_http_500_to_hardcover_error_http() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);
        let body = hc_search_body(25, "\"x\"");

        let err = hc_post(&fetcher, body, "tok", RequestPriority::Normal)
            .await
            .unwrap_err();

        match err {
            HardcoverError::Http(msg) => assert_eq!(msg, "HTTP 500"),
            other => panic!("expected Http variant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn hc_post_maps_fetch_timeout_to_hardcover_error_http() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::Timeout(std::time::Duration::from_secs(10)),
        );
        let body = hc_search_body(25, "\"x\"");

        let err = hc_post(&fetcher, body, "tok", RequestPriority::Normal)
            .await
            .unwrap_err();

        match err {
            HardcoverError::Http(msg) => assert!(msg.contains("timeout")),
            other => panic!("expected Http variant, got {other:?}"),
        }
    }
}
