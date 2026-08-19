//! Hardcover GraphQL client, consumed via `ProviderClient::Hardcover` (queue
//! dispatch and the identity-resolution fan-out).

use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
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

fn hardcover_response_error(message: impl Into<String>) -> HardcoverError {
    outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Failure);
    HardcoverError::Http(message.into())
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
        // An expired or revoked token answers 401/403 forever; reporting only
        // 5xx meant the breaker never learned and the queue kept dispatching.
        //
        // No 404/410 exemption: this is one fixed GraphQL POST route. GraphQL
        // signals "no such record" with a 200 carrying null data, never with a
        // 404 — so a 404 here means the ROUTE is gone or blocked, which is a
        // provider failure, not an absent book.
        outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Failure);
        return Err(HardcoverError::Http(format!("HTTP {}", resp.status)));
    }

    // No success report here. A Hardcover operation can carry a second leg
    // (`fetch_hardcover_editions`) and `record_success` clears every
    // accumulated failure, so reporting this leg's success up front meant a
    // permanently refused editions endpoint produced Success, Failure, Success,
    // Failure… and never reached the threshold. The operation boundary reports
    // once, via `report_hardcover_success`.
    let parsed: Value = serde_json::from_slice(&resp.body)
        .map_err(|e| hardcover_response_error(format!("parse error: {e}")))?;

    if let Some(errors) = parsed.get("errors") {
        let errors = errors
            .as_array()
            .ok_or_else(|| hardcover_response_error("malformed GraphQL errors field"))?;
        if !errors.is_empty() {
            return Err(hardcover_response_error(format!(
                "GraphQL response contained {} errors",
                errors.len()
            )));
        }
    }

    if !parsed.get("data").is_some_and(Value::is_object) {
        return Err(hardcover_response_error(
            "GraphQL response missing object data",
        ));
    }

    Ok(parsed)
}

/// Report a completed Hardcover operation as healthy.
///
/// Called by the operation boundary — the caller that knows every leg it was
/// going to run has run and succeeded — never by an individual request helper.
/// See the note in [`hc_post`].
pub fn report_hardcover_success() {
    outbound_queue::shared().report_outcome(RateBucket::Hardcover, BreakerSignal::Success);
}

/// Extract the `hits` array from a Hardcover search response value.
pub fn hc_extract_hits(data: &Value) -> Result<Vec<Value>, HardcoverError> {
    data.pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .ok_or_else(|| hardcover_response_error("GraphQL search response missing hits array"))
}

/// Pure Hardcover discovery transport lifted out of livrarr-metadata so both
/// interactive lookup and REQ-027 use one DEP-004-legal authority.
pub async fn fetch_hardcover_discovery_hits<F: HttpFetcher>(
    fetcher: &F,
    term: &str,
    token: &str,
    priority: RequestPriority,
) -> Result<Vec<Value>, HardcoverError> {
    let body = hc_search_body(15, term);
    let data = hc_post(fetcher, body, token, priority).await?;
    hc_extract_hits(&data)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HardcoverIdentityIds {
    pub isbns: Vec<String>,
    pub asins: Vec<String>,
}

/// One corroboration probe for a picked Hardcover work. The editions relation
/// carries every ISBN-13/ASIN needed by REQ-027 in this single `hc_post` call.
pub async fn probe_hardcover_identity_ids<F: HttpFetcher>(
    fetcher: &F,
    work_id: i64,
    token: &str,
    priority: RequestPriority,
) -> Result<HardcoverIdentityIds, HardcoverError> {
    let body = serde_json::json!({
        "query": r#"query IdentityEditions($bookId: Int!) {
            editions(where: {book_id: {_eq: $bookId}}, limit: 50) {
                isbn_13
                asin
            }
        }"#,
        "variables": {"bookId": work_id}
    });
    let data = hc_post(fetcher, body, token, priority).await?;
    let editions = data
        .pointer("/data/editions")
        .and_then(Value::as_array)
        .ok_or_else(|| hardcover_response_error("identity probe missing editions array"))?;
    let mut ids = HardcoverIdentityIds::default();
    for edition in editions {
        if let Some(value) = edition
            .get("isbn_13")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !ids.isbns.iter().any(|existing| existing == value) {
                ids.isbns.push(value.to_string());
            }
        }
        if let Some(value) = edition
            .get("asin")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !ids.asins.iter().any(|existing| existing == value) {
                ids.asins.push(value.to_string());
            }
        }
    }
    Ok(ids)
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
/// A Tier-1 miss falls through to the same deterministic title+author picker
/// every other provider uses (REQ-016/D10) — a near-miss that doesn't clear
/// the bar rides the standard grey-candidate flow at the identity layer, like
/// any other provider, rather than asking an LLM to choose.
pub async fn query_hardcover<F: HttpFetcher>(
    fetcher: &F,
    title: &str,
    author: &str,
    token: &str,
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
    let hits = hc_extract_hits(&data)?;

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

    // Tier 2: deterministic picker when the exact match fails (REQ-016/D10).
    // Mirrors the Goodreads/Audible tier (`gr_best_match`): score every hit's
    // title+author against the query with the shared scorer and take the
    // best match that clears the bar. Nothing clearing it means Hardcover
    // abstains, same as any other provider — the near-miss rides the
    // standard grey-candidate flow at the identity layer instead of an LLM
    // pick.
    let doc_idx = match best_idx {
        Some(i) => i,
        None => {
            let kept: Vec<(usize, (String, String))> = hits
                .iter()
                .enumerate()
                .filter_map(|(i, hit)| {
                    let doc = hit.get("document")?;
                    let t = doc.get("title").and_then(|v| v.as_str())?.trim();
                    if t.is_empty() {
                        return None;
                    }
                    let a = doc_author_name(doc).unwrap_or_default();
                    Some((i, (t.to_string(), a)))
                })
                .collect();
            let scored: Vec<(String, String)> = kept.iter().map(|(_, c)| c.clone()).collect();
            match livrarr_domain::identity_matching::pick_best_candidate(
                title, author, &scored, false,
            ) {
                Some(pick) => kept[pick].0,
                None => {
                    return Err(HardcoverError::NoMatch(
                        "no confident deterministic match".into(),
                    ))
                }
            }
        }
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

    let data = hc_post(fetcher, body, token, priority)
        .await
        .map_err(|e| format!("edition request failed: {e}"))?;
    // Editions are a child leg. `HardcoverClient::build_success` reports the
    // operation's one Success only after this parse and every earlier leg pass.

    let editions = data
        .pointer("/data/editions")
        .and_then(|e| e.as_array())
        .cloned()
        .ok_or_else(|| hardcover_response_error("GraphQL editions response missing editions array"))
        .map_err(|e| format!("edition response invalid: {e}"))?;

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

pub async fn query_hardcover_by_isbn<F: HttpFetcher>(
    fetcher: &F,
    isbn: &str,
    token: &str,
    _metadata_cfg: &livrarr_domain::settings::MetadataConfig,
    priority: RequestPriority,
) -> Result<Option<HardcoverResult>, HardcoverError> {
    let body = hc_search_body(10, isbn);
    let data = hc_post(fetcher, body, token, priority).await?;
    let hits = hc_extract_hits(&data)?;

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

/// GraphQL query for fetching a single Hardcover book by its numeric `id`
/// (the `hc_key` anchor). Depth-limited: no author/series/edition joins, so
/// callers get title/subtitle/description/pages/rating/cover only.
fn hc_book_by_key_graphql() -> &'static str {
    r#"query GetBookByKey($id: Int!) {
        books_by_pk(id: $id) {
            title
            subtitle
            description
            release_date
            pages
            rating
            ratings_count
            image { url }
        }
    }"#
}

/// Fetch a Hardcover book directly by its known `hc_key` (numeric book id).
///
/// This is a high-confidence exact match (unlike the fuzzy `SearchBooks`
/// path), so `subtitle` is populated here. Author, series, publisher,
/// genres, and ISBN are not available at this query depth — callers needing
/// an ISBN already make a separate `fetch_hardcover_editions` call
/// (see `build_success`).
pub async fn query_hardcover_by_key<F: HttpFetcher>(
    fetcher: &F,
    book_id: i64,
    token: &str,
    priority: RequestPriority,
) -> Result<Option<HardcoverResult>, HardcoverError> {
    let body = serde_json::json!({
        "query": hc_book_by_key_graphql(),
        "variables": {"id": book_id}
    });
    let data = hc_post(fetcher, body, token, priority).await?;

    let doc = match data.pointer("/data/books_by_pk") {
        Some(v) if v.is_null() => return Ok(None),
        Some(v) if v.is_object() => v,
        _ => {
            return Err(hardcover_response_error(
                "GraphQL key response missing object-or-null books_by_pk",
            ))
        }
    };

    let title = doc
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let subtitle = doc
        .get("subtitle")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let description = doc
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let page_count = doc.get("pages").and_then(|v| v.as_i64()).map(|v| v as i32);

    let publish_date = doc
        .get("release_date")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let cover_url = doc
        .pointer("/image/url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let rating = doc.get("rating").and_then(|v| v.as_f64());
    let rating_count = doc
        .get("ratings_count")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    Ok(Some(HardcoverResult {
        title,
        author_name: None,
        subtitle,
        original_title: None,
        description,
        series_name: None,
        series_position: None,
        genres: None,
        page_count,
        publisher: None,
        publish_date,
        hc_key: Some(book_id.to_string()),
        isbn_13: None,
        cover_url,
        rating,
        rating_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata_config() -> livrarr_domain::settings::MetadataConfig {
        livrarr_domain::settings::MetadataConfig {
            hardcover_enabled: true,
            hardcover_api_token: Some("tok".to_string()),
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: None,
        }
    }

    /// Hardcover speaks one fixed GraphQL POST route. "No such record" arrives
    /// as a 200 carrying null data, never as a 404 — so a 404/410 here means the
    /// route is gone or blocked, and exempting it left a dead route invisible to
    /// the breaker while every request errored.
    #[tokio::test]
    async fn a_hardcover_route_404_reports_a_breaker_failure() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Hardcover,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);
        let err = hc_post(
            &fetcher,
            serde_json::json!({"query": "{ books { id } }"}),
            "tok",
            RequestPriority::Normal,
        )
        .await;
        assert!(err.is_err(), "a 404 route must not yield data");

        let tripped = {
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "a 404 on the fixed GraphQL route is a provider failure, not an absent book"
        );
    }

    #[tokio::test]
    async fn hc_post_rejects_unreadable_or_invalid_graphql_envelopes() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        let one_strike = || CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        };
        let cases = vec![
            ("malformed JSON", b"not json".to_vec()),
            (
                "non-empty errors",
                serde_json::json!({"errors": [{"message": "denied"}], "data": {}})
                    .to_string()
                    .into_bytes(),
            ),
            (
                "partial data plus errors",
                serde_json::json!({
                    "errors": [{"message": "partial"}],
                    "data": {"search": {"results": {"hits": []}}}
                })
                .to_string()
                .into_bytes(),
            ),
            (
                "non-array errors",
                serde_json::json!({"errors": {"message": "bad"}, "data": {}})
                    .to_string()
                    .into_bytes(),
            ),
            (
                "null errors",
                serde_json::json!({"errors": null, "data": {}})
                    .to_string()
                    .into_bytes(),
            ),
            (
                "missing data",
                serde_json::json!({"errors": []}).to_string().into_bytes(),
            ),
            (
                "non-object data",
                serde_json::json!({"errors": [], "data": []})
                    .to_string()
                    .into_bytes(),
            ),
        ];

        for (label, response_body) in cases {
            queue.reset_breaker_for_tests(RateBucket::Hardcover);
            queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, response_body);
            let result = hc_post(
                &fetcher,
                serde_json::json!({"query": "{ books { id } }"}),
                "tok",
                RequestPriority::Normal,
            )
            .await;
            assert!(
                matches!(result, Err(HardcoverError::Http(_))),
                "{label}: the invalid response must preserve the Http error variant"
            );

            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            assert!(
                matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
                "{label}: the completed invalid response must open a threshold-one breaker"
            );
        }

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            serde_json::json!({"errors": [], "data": {}})
                .to_string()
                .into_bytes(),
        );
        let result = hc_post(
            &fetcher,
            serde_json::json!({"query": "{ books { id } }"}),
            "tok",
            RequestPriority::Normal,
        )
        .await;
        assert!(result.is_ok(), "an empty errors array is a valid envelope");
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            !matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "a valid empty-errors envelope must remain healthy"
        );
    }

    #[tokio::test]
    async fn hardcover_search_paths_reject_missing_or_wrong_typed_hits() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        let one_strike = || CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        };
        let cases = [
            (
                "missing hits",
                serde_json::json!({"data": {"search": {"results": {}}}}),
            ),
            (
                "wrong-typed hits",
                serde_json::json!({"data": {"search": {"results": {"hits": {}}}}}),
            ),
        ];

        for (label, response) in &cases {
            queue.reset_breaker_for_tests(RateBucket::Hardcover);
            queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
                200,
                response.to_string().into_bytes(),
            );
            let result = query_hardcover(
                &fetcher,
                "Dune",
                "Frank Herbert",
                "tok",
                RequestPriority::Normal,
            )
            .await;
            assert!(
                matches!(result, Err(HardcoverError::Http(_))),
                "title search with {label} must be a response error"
            );
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            assert!(
                matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
                "title search with {label} must open a threshold-one breaker"
            );

            queue.reset_breaker_for_tests(RateBucket::Hardcover);
            queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
                200,
                response.to_string().into_bytes(),
            );
            let result = query_hardcover_by_isbn(
                &fetcher,
                "9780441172719",
                "tok",
                &test_metadata_config(),
                RequestPriority::Normal,
            )
            .await;
            assert!(
                matches!(result, Err(HardcoverError::Http(_))),
                "ISBN search with {label} must be a response error"
            );
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            assert!(
                matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
                "ISBN search with {label} must open a threshold-one breaker"
            );
        }

        let empty_hits = serde_json::json!({"data": {"search": {"results": {"hits": []}}}});
        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            empty_hits.to_string().into_bytes(),
        );
        let result = query_hardcover(
            &fetcher,
            "Dune",
            "Frank Herbert",
            "tok",
            RequestPriority::Normal,
        )
        .await;
        assert!(matches!(result, Err(HardcoverError::NoResults)));
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            !matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "an explicitly empty title-search hits array is a healthy miss"
        );

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            empty_hits.to_string().into_bytes(),
        );
        let result = query_hardcover_by_isbn(
            &fetcher,
            "9780441172719",
            "tok",
            &test_metadata_config(),
            RequestPriority::Normal,
        )
        .await;
        assert!(matches!(result, Ok(None)));
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            !matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "an explicitly empty ISBN-search hits array is a healthy miss"
        );
    }

    #[tokio::test]
    async fn hardcover_key_lookup_rejects_missing_or_wrong_typed_book_field() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        let one_strike = || CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        };
        let cases = [
            ("missing books_by_pk", serde_json::json!({"data": {}})),
            (
                "wrong-typed books_by_pk",
                serde_json::json!({"data": {"books_by_pk": []}}),
            ),
        ];

        for (label, response) in cases {
            queue.reset_breaker_for_tests(RateBucket::Hardcover);
            queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
                200,
                response.to_string().into_bytes(),
            );
            let result = query_hardcover_by_key(&fetcher, 42, "tok", RequestPriority::Normal).await;
            assert!(
                matches!(result, Err(HardcoverError::Http(_))),
                "{label} must be a response error"
            );
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            assert!(
                matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
                "{label} must open a threshold-one breaker"
            );
        }

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            serde_json::json!({"data": {"books_by_pk": null}})
                .to_string()
                .into_bytes(),
        );
        let result = query_hardcover_by_key(&fetcher, 42, "tok", RequestPriority::Normal).await;
        assert!(
            matches!(result, Ok(None)),
            "an explicit null book is a healthy miss"
        );
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            !matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "an explicit null book must not report Failure"
        );
    }

    #[tokio::test]
    async fn every_hardcover_query_path_uses_the_common_graphql_envelope_check() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        let one_strike = || CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        };
        let response = serde_json::json!({
            "errors": [{"message": "denied"}],
            "data": {}
        })
        .to_string()
        .into_bytes();

        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, response.clone());
        let result = query_hardcover(
            &fetcher,
            "Dune",
            "Frank Herbert",
            "tok",
            RequestPriority::Normal,
        )
        .await;
        assert!(matches!(result, Err(HardcoverError::Http(_))));
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(matches!(admission, Err(AdmissionError::CircuitOpen { .. })));

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, response.clone());
        let result = query_hardcover_by_isbn(
            &fetcher,
            "9780441172719",
            "tok",
            &test_metadata_config(),
            RequestPriority::Normal,
        )
        .await;
        assert!(matches!(result, Err(HardcoverError::Http(_))));
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(matches!(admission, Err(AdmissionError::CircuitOpen { .. })));

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, response.clone());
        let result = query_hardcover_by_key(&fetcher, 42, "tok", RequestPriority::Normal).await;
        assert!(matches!(result, Err(HardcoverError::Http(_))));
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(matches!(admission, Err(AdmissionError::CircuitOpen { .. })));

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, response);
        let result =
            fetch_hardcover_editions(&fetcher, "42", "tok", "en", RequestPriority::Normal).await;
        assert!(
            result.is_err(),
            "editions must reject the common GraphQL error envelope"
        );
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "editions must report the common envelope failure"
        );
    }

    #[tokio::test]
    async fn hardcover_editions_uses_common_envelope_failure_signal() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Hardcover,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            serde_json::json!({
                "errors": [{"message": "denied"}],
                "data": {"editions": []}
            })
            .to_string()
            .into_bytes(),
        );

        let result =
            fetch_hardcover_editions(&fetcher, "42", "tok", "en", RequestPriority::Normal).await;
        assert!(result.is_err());
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "the editions path must retain hc_post's response-derived Failure"
        );
    }

    #[tokio::test]
    async fn hardcover_editions_rejects_missing_or_wrong_typed_editions_field() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        let one_strike = || CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        };
        let cases = [
            ("missing editions", serde_json::json!({"data": {}})),
            (
                "wrong-typed editions",
                serde_json::json!({"data": {"editions": {}}}),
            ),
        ];

        for (label, response) in cases {
            queue.reset_breaker_for_tests(RateBucket::Hardcover);
            queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
                200,
                response.to_string().into_bytes(),
            );
            let result =
                fetch_hardcover_editions(&fetcher, "42", "tok", "en", RequestPriority::Normal)
                    .await;
            assert!(result.is_err(), "{label} must be a response error");
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            assert!(
                matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
                "{label} must open a threshold-one breaker"
            );
        }

        queue.reset_breaker_for_tests(RateBucket::Hardcover);
        queue.set_breaker_config_for_tests(RateBucket::Hardcover, one_strike());
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            serde_json::json!({"data": {"editions": []}})
                .to_string()
                .into_bytes(),
        );
        let result =
            fetch_hardcover_editions(&fetcher, "42", "tok", "en", RequestPriority::Normal).await;
        assert!(
            matches!(result, Ok(None)),
            "an explicit empty editions array is a healthy miss"
        );
        let admission = queue
            .acquire(RateBucket::Hardcover, RequestPriority::Normal)
            .await;
        assert!(
            !matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "an explicit empty editions array must not report Failure"
        );
    }

    /// A success clears every accumulated failure, so a query leg reporting
    /// success before the editions leg ran meant `query=200, editions=403`
    /// repeating forever produced Success, Failure, Success, Failure… and the
    /// count never reached the production threshold. The refused editions
    /// endpoint was re-requested indefinitely with the breaker closed.
    ///
    /// Runs at the PRODUCTION threshold deliberately — forcing the threshold to
    /// 1 cannot observe this shape at all.
    ///
    /// This lower-level two-leg regression remains useful alongside B2's
    /// `HardcoverClient<RecordingHttpFetcher>` composition tests: both exercise
    /// the same process-global breaker, while the client test additionally
    /// proves `build_success` still owns the final signal.
    #[tokio::test]
    async fn a_refused_editions_endpoint_eventually_opens_the_hardcover_breaker() {
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();

        let ok_body = serde_json::json!({"data": {}}).to_string();
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, ok_body.clone().into_bytes());
        for _ in 0..12 {
            fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
                status: 403,
                headers: vec![],
                body: vec![],
            }));
            fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
                status: 200,
                headers: vec![],
                body: ok_body.clone().into_bytes(),
            }));
        }

        for _ in 0..12 {
            // Leg 1: the query, exactly as `query_hardcover*` issues it.
            let _ = hc_post(
                &fetcher,
                serde_json::json!({"query": "{ books { id } }"}),
                "tok",
                RequestPriority::Normal,
            )
            .await;
            // Leg 2: the editions follow-up `build_success` issues on a hc_key.
            let editions =
                fetch_hardcover_editions(&fetcher, "123", "tok", "en", RequestPriority::Normal)
                    .await;
            // The operation boundary reports success only when every leg was ok.
            if editions.is_ok() {
                report_hardcover_success();
            }
        }

        let tripped = {
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "a permanently refused editions endpoint must eventually open the \
             Hardcover breaker — a sibling leg's success must not erase it"
        );
    }

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
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
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

    /// Editions are a child leg of a Hardcover operation. A healthy child
    /// response must not clear failures before the caller reaches the final
    /// boundary and decides whether every provider leg succeeded.
    // Bug reproduction: subtitle-matching finding 1 — the editions helper and
    // `build_success` both reported Success for one operation.
    #[tokio::test]
    async fn hardcover_editions_defers_success_to_the_operation_boundary() {
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let queue = outbound_queue::shared();
        for _ in 0..4 {
            queue.report_outcome(RateBucket::Hardcover, BreakerSignal::Failure);
        }

        let canned = serde_json::json!({"data": {"editions": []}}).to_string();
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, canned.into_bytes());
        let result =
            fetch_hardcover_editions(&fetcher, "42", "tok", "en", RequestPriority::Normal).await;
        assert_eq!(
            result.expect("a parseable editions response is healthy"),
            None
        );

        queue.report_outcome(RateBucket::Hardcover, BreakerSignal::Failure);
        let tripped = {
            let admission = queue
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "the editions helper must not report Success before its caller boundary"
        );
    }

    // -------------------------------------------------------------------
    // Error mapping: HttpFetcher failures map onto the HardcoverError
    // shapes callers match on (provider_client.rs WillRetry{ServerError}).
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn hc_post_maps_http_500_to_hardcover_error_http() {
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
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

    // -------------------------------------------------------------------
    // query_hardcover_by_key: hc_key anchor fetch (GetBookByKey), used by
    // provider_client.rs's AnchorQuery::HcKey arm.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_hardcover_by_key_sends_hardcover_bucket_post_bearer_auth_and_id_variable() {
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let canned = serde_json::json!({"data": {"books_by_pk": null}});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        query_hardcover_by_key(&fetcher, 10257, "test-token", RequestPriority::High)
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
        let sent_body: Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
        assert_eq!(sent_body["variables"], serde_json::json!({"id": 10257}));
        assert_transport_params(req, RequestPriority::High);
    }

    #[tokio::test]
    async fn query_hardcover_by_key_maps_full_response_into_hardcover_result() {
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        // Shape matches the live-prototyped GetBookByKey response
        // (2026-07-05, book id 1): rating is a float, release_date is
        // "YYYY-MM-DD", pages/ratings_count are ints.
        let canned = serde_json::json!({
            "data": {
                "books_by_pk": {
                    "title": "I Am Legend",
                    "subtitle": "And Other Stories",
                    "description": "A vampire novel.",
                    "release_date": "1954-01-01",
                    "release_year": 1954,
                    "pages": 161,
                    "rating": 3.9038461538461538,
                    "ratings_count": 26,
                    "image": {"url": "https://assets.hardcover.app/edition/x.jpeg"}
                }
            }
        });
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let result = query_hardcover_by_key(&fetcher, 10257, "tok", RequestPriority::Normal)
            .await
            .unwrap()
            .expect("expected Some(HardcoverResult)");

        assert_eq!(result.title.as_deref(), Some("I Am Legend"));
        assert_eq!(result.subtitle.as_deref(), Some("And Other Stories"));
        assert_eq!(result.description.as_deref(), Some("A vampire novel."));
        assert_eq!(result.publish_date.as_deref(), Some("1954-01-01"));
        assert_eq!(result.page_count, Some(161));
        assert_eq!(result.rating, Some(3.9038461538461538));
        assert_eq!(result.rating_count, Some(26));
        assert_eq!(
            result.cover_url.as_deref(),
            Some("https://assets.hardcover.app/edition/x.jpeg")
        );
        assert_eq!(result.hc_key.as_deref(), Some("10257"));
        // Not available at this query depth — must stay None per design.
        assert_eq!(result.author_name, None);
        assert_eq!(result.original_title, None);
        assert_eq!(result.publisher, None);
        assert_eq!(result.series_name, None);
        assert_eq!(result.series_position, None);
        assert_eq!(result.genres, None);
        assert_eq!(result.isbn_13, None);
    }

    #[tokio::test]
    async fn query_hardcover_by_key_returns_none_for_null_books_by_pk() {
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let canned = serde_json::json!({"data": {"books_by_pk": null}});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let result = query_hardcover_by_key(&fetcher, 999999999, "tok", RequestPriority::Normal)
            .await
            .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn provider_picker_conformance_hardcover_abstains_on_grey_author_hit() {
        let _guard = crate::test_support::lock_breaker(RateBucket::Hardcover).await;
        let canned = serde_json::json!({
            "data": {
                "search": {
                    "results": {
                        "hits": [{
                            "document": {
                                "id": 42,
                                "title": "Storm Front",
                                "author_names": ["Jane Smith"],
                                "users_read_count": 10
                            }
                        }]
                    }
                }
            }
        });
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let err = query_hardcover(
            &fetcher,
            "Storm Front",
            "John Smith",
            "tok",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        match err {
            HardcoverError::NoMatch(msg) => {
                assert_eq!(msg, "no confident deterministic match");
            }
            other => panic!("expected NoMatch for grey author hit, got {other:?}"),
        }
    }
}
