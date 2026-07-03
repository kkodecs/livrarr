//! Goodreads HTML parsing — regex extraction from search pages, JSON-LD + regex from detail pages.
//!
//! Replaces LLM-based scraping with direct HTML parsing for foreign language works.
//! LLM is kept as fallback only (see fallback chain in design doc).

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use livrarr_http::HttpClient;
use regex::Regex;
use std::sync::LazyLock;
use std::time::Duration;

// =============================================================================
// Types
// =============================================================================

/// A single book result extracted from a Goodreads search results page.
#[derive(Debug, Clone)]
pub struct GoodreadsSearchResult {
    pub title: String,
    /// GR's own undecorated title (`bookTitleBare`): "Pandora's Star" where
    /// `title` is "Pandora's Star (Commonwealth Saga, #1)". Provider data,
    /// not a cleaning step — preferred as the payload title so GR's answer
    /// compares like every other provider's instead of carrying search-card
    /// series decoration into matching and merge.
    pub title_bare: Option<String>,
    pub author: Option<String>,
    pub detail_url: String,
    pub cover_url: Option<String>,
    pub year: Option<i32>,
    pub rating: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

/// Detailed metadata extracted from a Goodreads book detail page.
#[derive(Debug, Clone)]
pub struct GoodreadsDetailResult {
    // JSON-LD fields (primary)
    pub title: Option<String>,
    pub author: Option<String>,
    pub isbn: Option<String>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub page_count: Option<i32>,
    pub language: Option<String>,
    pub cover_url: Option<String>,
    pub book_format: Option<String>,
    // Regex fields (secondary)
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub publish_date: Option<String>,
}

// =============================================================================
// Regex patterns (LazyLock for one-time compilation)
// =============================================================================

// Search page patterns
static RE_BOOK_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<tr[^>]*itemscope[^>]*itemtype="https?://schema\.org/Book"[^>]*>(.*?)</tr>"#)
        .unwrap()
});

static RE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?si)<a[^>]*class="bookTitle"[^>]*href="([^"]*)"[^>]*>.*?<span[^>]*>([^<]+)</span>"#,
    )
    .unwrap()
});

static RE_AUTHOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<a[^>]*class="authorName"[^>]*>.*?<span[^>]*>([^<]+)</span>"#).unwrap()
});

static RE_COVER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<img[^>]*class="bookCover"[^>]*(?:src|data-src)="([^"]+)""#).unwrap()
});

static RE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"published\s+(\d{4})"#).unwrap());

static RE_RATING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)class="minirating"[^>]*>(.*?)</span>"#).unwrap());

static RE_RATING_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\d+\.\d+)\s+avg"#).unwrap());

/// Matches series info in parentheses at end of title: "(Series Name, #1)"
static RE_TITLE_SERIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\s*\(([^,]+),\s*#(\d+(?:\.\d+)?)\)\s*$"#).unwrap());

// Detail page regex patterns
static RE_JSONLD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<script\s+type="application/ld\+json">(.*?)</script>"#).unwrap()
});

static RE_DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)<span\s+class="Formatted">(.*?)</span>"#).unwrap());

static RE_GENRES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href="[^"]*goodreads\.com/genres/([^"]+)""#).unwrap());

static RE_PUBLISHED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"First published\s+(.*?)(?:<|$)"#).unwrap());

static RE_SERIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"aria-label="Book (\d+) in the (.*?) series""#).unwrap());

static RE_HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<[^>]+>"#).unwrap());

// =============================================================================
// Search page parsing
// =============================================================================

/// Parse a Goodreads search results page into structured results.
///
/// Extracts book rows from the HTML using schema.org `<tr>` markup, then pulls
/// title, author, detail URL, cover URL, and year from each row.
///
/// Returns an empty vec if no valid rows found (caller decides on fallback).
pub fn parse_search_html(html: &str) -> Vec<GoodreadsSearchResult> {
    let mut results = Vec::new();

    for row_match in RE_BOOK_ROW.captures_iter(html) {
        let row = &row_match[1];

        // Title + detail URL (required)
        let Some(title_cap) = RE_TITLE.captures(row) else {
            continue;
        };
        let raw_url = title_cap[1].to_string();
        let title = title_cap[2].trim().to_string();

        if title.is_empty() {
            continue;
        }

        // Strip query params from detail URL
        let detail_url = raw_url.split('?').next().unwrap_or(&raw_url).to_string();

        // Unescape &amp; in URLs
        let detail_url = detail_url.replace("&amp;", "&");

        // Author (optional)
        let author = RE_AUTHOR
            .captures(row)
            .map(|c| c[1].trim().to_string())
            .filter(|a| !a.is_empty());

        // Cover URL (optional, filter placeholders). Resolve to an absolute URL
        // against the Goodreads base so a relative `src` never propagates.
        let cover_url = RE_COVER.captures(row).and_then(|c| {
            let url = &c[1];
            if url.contains("nophoto") || url.contains("loading-trans") {
                None
            } else {
                crate::provider_util::validate_cover_url(url, GOODREADS_BASE_URL)
            }
        });

        // Year (optional)
        let year = RE_YEAR.captures(row).and_then(|c| c[1].parse::<i32>().ok());

        // Rating (optional) — e.g. "3.92 avg rating"
        let rating = RE_RATING
            .captures(row)
            .and_then(|c| RE_RATING_VALUE.captures(&c[1]).map(|m| m[1].to_string()));

        // Extract series from title: "Book Title (Series Name, #1)" → strip from title
        let (clean_title, series_name, series_position) =
            if let Some(caps) = RE_TITLE_SERIES.captures(&title) {
                let sname = caps[1].trim().to_string();
                let spos = caps[2].parse::<f64>().ok();
                let clean = RE_TITLE_SERIES.replace(&title, "").trim().to_string();
                (clean, Some(sname), spos)
            } else {
                (title, None, None)
            };

        results.push(GoodreadsSearchResult {
            // Already stripped of its "(Series, #N)" decoration above.
            title: clean_title,
            title_bare: None,
            author,
            detail_url,
            cover_url,
            year,
            rating,
            series_name,
            series_position,
        });
    }

    results
}

// =============================================================================
// Autocomplete (discovery) parsing
// =============================================================================

/// One entry from the Goodreads `/book/auto_complete` JSON response. Only the
/// fields a search card needs are modeled; the rest (`workId`, `numPages`,
/// `ratingsCount`, `description`, …) is ignored.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutocompleteEntry {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    book_title_bare: Option<String>,
    #[serde(default)]
    book_url: Option<String>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    avg_rating: Option<StringOrNumber>,
    #[serde(default)]
    author: Option<AutocompleteAuthor>,
}

/// `avgRating` arrives as a string on most entries ("4.30") but as a bare JSON
/// number (0.0) on some unrated editions — one such entry must not fail the
/// batch. Numbers render to the same two-decimal form the strings use, so the
/// downstream "0.00" = unrated filter applies uniformly.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StringOrNumber {
    S(String),
    N(f64),
}

impl StringOrNumber {
    fn into_rating_string(self) -> String {
        match self {
            StringOrNumber::S(s) => s,
            StringOrNumber::N(n) => format!("{n:.2}"),
        }
    }
}

#[derive(serde::Deserialize)]
struct AutocompleteAuthor {
    #[serde(default)]
    name: Option<String>,
}

/// Parse the Goodreads `/book/auto_complete?format=json` response into the same
/// `GoodreadsSearchResult` shape the HTML `/search` parser produces, so the
/// discovery fan-out can treat Goodreads like any other provider.
///
/// `/search` is AWS-WAF 202-challenged (effectively dead); this WAF-free JSON
/// endpoint is the live discovery path (measured 2026-06-01). A non-array body
/// (a WAF interstitial or a format change) yields an empty list rather than an
/// error — the caller unions providers, so a Goodreads miss is not a failure.
/// Entries deserialize INDIVIDUALLY: one malformed entry drops alone (logged)
/// instead of failing the whole batch — a single rogue edition in the hit list
/// used to silently erase every result for that query.
pub fn parse_autocomplete_json(body: &str) -> Vec<GoodreadsSearchResult> {
    let values: Vec<serde_json::Value> = match serde_json::from_str(body) {
        Ok(values) => values,
        Err(e) => {
            tracing::warn!(error = %e, "GR autocomplete body is not a JSON array (WAF interstitial or format change) — treating as no results");
            return Vec::new();
        }
    };
    values
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<AutocompleteEntry>(v) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!(error = %e, "GR autocomplete entry failed to parse — dropping this entry only");
                None
            }
        })
        .filter_map(|e| {
            let title = e.title.filter(|t| !t.trim().is_empty())?;
            let detail_url = e.book_url.filter(|u| !u.trim().is_empty())?;
            Some(GoodreadsSearchResult {
                title,
                title_bare: e.book_title_bare.filter(|t| !t.trim().is_empty()),
                author: e
                    .author
                    .and_then(|a| a.name)
                    .filter(|n| !n.trim().is_empty()),
                detail_url,
                cover_url: e
                    .image_url
                    .filter(|u| !u.trim().is_empty())
                    .and_then(|u| crate::provider_util::validate_cover_url(&u, GOODREADS_BASE_URL))
                    .map(|u| crate::provider_util::upscale_cover_url(&u)),
                year: None,
                // `avgRating` normalizes to a two-decimal string (e.g. "4.30");
                // "0.00" means unrated.
                rating: e
                    .avg_rating
                    .map(StringOrNumber::into_rating_string)
                    .filter(|r| !r.trim().is_empty() && r != "0.00"),
                series_name: None,
                series_position: None,
            })
        })
        .collect()
}

// =============================================================================
// Detail page parsing
// =============================================================================

/// Parse a Goodreads book detail page for metadata.
///
/// Primary source: JSON-LD `<script type="application/ld+json">` blocks.
/// Secondary source: regex for description, genres, series, published date.
pub fn parse_detail_html(html: &str) -> Option<GoodreadsDetailResult> {
    // Find the Book JSON-LD block
    let book_json = find_book_jsonld(html);

    // Parse regex fields regardless of JSON-LD success
    let description = extract_description(html);
    let genres = extract_genres(html);
    let publish_date = RE_PUBLISHED.captures(html).map(|c| c[1].trim().to_string());
    let (series_name, series_position) = RE_SERIES
        .captures(html)
        .map(|c| {
            let pos = c[1].parse::<f64>().ok();
            let name = c[2]
                .replace("&#x27;", "'")
                .replace("&amp;", "&")
                .replace("&quot;", "\"");
            (Some(name), pos)
        })
        .unwrap_or((None, None));

    // If we have JSON-LD, use it as primary
    if let Some(book) = book_json {
        let title = book.get("name").and_then(|v| v.as_str()).map(|s| {
            // Decode HTML entities
            s.replace("&amp;", "&")
                .replace("&apos;", "'")
                .replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
        });

        let author = extract_author_name(&book);

        let isbn = book
            .get("isbn")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let rating = book
            .get("aggregateRating")
            .and_then(|r| r.get("ratingValue"))
            .and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_f64(),
                serde_json::Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            });

        let rating_count = book
            .get("aggregateRating")
            .and_then(|r| r.get("ratingCount"))
            .and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_i64().map(|n| n as i32),
                serde_json::Value::String(s) => s.parse::<i32>().ok(),
                _ => None,
            });

        let page_count = book.get("numberOfPages").and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_i64().map(|n| n as i32),
            serde_json::Value::String(s) => s.parse::<i32>().ok(),
            _ => None,
        });

        let language = book
            .get("inLanguage")
            .and_then(|v| v.as_str())
            .map(livrarr_domain::normalize_language);

        let cover_url = book
            .get("image")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let book_format = book
            .get("bookFormat")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(GoodreadsDetailResult {
            title,
            author,
            isbn,
            rating,
            rating_count,
            page_count,
            language,
            cover_url,
            book_format,
            description,
            genres,
            series_name,
            series_position,
            publish_date,
        })
    } else if description.is_some() || !genres.is_empty() {
        // No JSON-LD but we got something from regex
        Some(GoodreadsDetailResult {
            title: None,
            author: None,
            isbn: None,
            rating: None,
            rating_count: None,
            page_count: None,
            language: None,
            cover_url: None,
            book_format: None,
            description,
            genres,
            series_name,
            series_position,
            publish_date,
        })
    } else {
        None
    }
}

/// Scan all JSON-LD blocks and find the one with `@type: "Book"`.
fn find_book_jsonld(html: &str) -> Option<serde_json::Value> {
    for cap in RE_JSONLD.captures_iter(html) {
        let json_str = &cap[1];
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else {
            continue;
        };

        // Direct object with @type: Book
        if value.get("@type").and_then(|v| v.as_str()) == Some("Book") {
            return Some(value);
        }

        // Array of objects
        if let Some(arr) = value.as_array() {
            for item in arr {
                if item.get("@type").and_then(|v| v.as_str()) == Some("Book") {
                    return Some(item.clone());
                }
            }
        }

        // @graph wrapper
        if let Some(graph) = value.get("@graph").and_then(|v| v.as_array()) {
            for item in graph {
                if item.get("@type").and_then(|v| v.as_str()) == Some("Book") {
                    return Some(item.clone());
                }
            }
        }
    }

    None
}

/// Extract author name from JSON-LD, handling both object and array forms.
fn extract_author_name(book: &serde_json::Value) -> Option<String> {
    match book.get("author") {
        Some(serde_json::Value::Array(arr)) => arr
            .first()
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Some(serde_json::Value::Object(obj)) => obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Extract description from HTML, stripping all HTML tags to plain text.
fn extract_description(html: &str) -> Option<String> {
    let cap = RE_DESCRIPTION.captures(html)?;
    let raw = &cap[1];
    let plain = RE_HTML_TAG.replace_all(raw, "");
    let trimmed = plain.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Extract unique genre slugs from the page.
fn extract_genres(html: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut genres = Vec::new();
    for cap in RE_GENRES.captures_iter(html) {
        let slug = cap[1].to_string();
        if seen.insert(slug.clone()) {
            genres.push(slug);
        }
    }
    genres
}

// =============================================================================
// HTTP fetcher
// =============================================================================

/// Production base URL for Goodreads. Tests pass a local TcpListener URL instead.
pub const GOODREADS_BASE_URL: &str = "https://www.goodreads.com";

/// Browser-like UA — Goodreads serves a stripped page (or anti-bot challenge)
/// without it.
pub const GOODREADS_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Failure modes from a Goodreads HTTP fetch. Callers map these onto
/// `ProviderOutcome` (queue path) or per-call retry / fallback decisions
/// (identity and series surfaces).
#[derive(Debug, Clone)]
pub enum GoodreadsFetchError {
    /// Response body matched the anti-bot indicator heuristic.
    AntiBot,
    /// HTTP status was non-success. Caller can discriminate 429/5xx vs 4xx.
    HttpStatus(u16),
    /// Transport / DNS / body-read error from `reqwest`.
    Network(String),
    /// Detail page returned 200 OK but no JSON-LD or regex fields parsed out.
    Parse,
    /// The outbound queue's breaker was Open for the Goodreads bucket — no
    /// HTTP was attempted (R-11: the caller must map this to
    /// `WillRetryReason::CircuitOpen`, never burn retry budget on it).
    CircuitOpen(Duration),
}

/// Build the canonical detail URL for a `gr_key` against the configured base.
///
/// `gr_key` is the bare identifier (e.g. `"123"` or `"123.Title_Slug"`) — the
/// part after `/book/show/`.
pub fn detail_url_for_gr_key(base_url: &str, gr_key: &str) -> String {
    format!(
        "{}/book/show/{}",
        base_url.trim_end_matches('/'),
        gr_key.trim_start_matches('/'),
    )
}

/// Resolve a (possibly relative) detail URL from `parse_search_html` against
/// the configured base. Production: `base = https://www.goodreads.com`.
/// Tests: `base = http://127.0.0.1:NNNN` (the TcpListener URL).
pub fn resolve_detail_url(base_url: &str, detail_url: &str) -> String {
    if detail_url.starts_with("http://") || detail_url.starts_with("https://") {
        detail_url.to_string()
    } else {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            detail_url.trim_start_matches('/'),
        )
    }
}

/// Extract the `gr_key` (the `123.Title_Slug` segment) from a detail URL.
/// Returns None if the URL doesn't follow the `/book/show/{key}` shape.
pub fn extract_gr_key(detail_url: &str) -> Option<String> {
    let after = detail_url.split("/book/show/").nth(1)?;
    let key = after.split(['?', '#', '/']).next()?;
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

/// Map an `HttpFetcher` transport failure onto `GoodreadsFetchError`. The
/// fetcher intercepts HTTP 429 at the transport level (`FetchError::RateLimited`)
/// rather than surfacing it as a normal response status, so it is translated
/// back to `HttpStatus(429)` here — preserving the existing 429-vs-other-error
/// discrimination (`map_fetch_err` treats `HttpStatus(429)` as `RateLimit`, any
/// other `Network` failure as `ServerError`).
fn map_transport_err(context: &str, err: FetchError) -> GoodreadsFetchError {
    match err {
        FetchError::RateLimited => GoodreadsFetchError::HttpStatus(429),
        FetchError::CircuitOpen { retry_after } => GoodreadsFetchError::CircuitOpen(retry_after),
        other => GoodreadsFetchError::Network(format!("{context}: {other}")),
    }
}

/// Fetch a Goodreads HTML page. Adds the Chrome UA and treats non-success
/// status and anti-bot challenge pages as errors.
///
/// Used by the queue path (`GoodreadsClient` in `provider_client`). Pacing is
/// the outbound queue's responsibility, per the per-provider `RateBucket`.
pub async fn fetch_goodreads_html<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    priority: RequestPriority,
) -> Result<String, GoodreadsFetchError> {
    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![("Accept-Language".to_string(), "en-US,en;q=0.9".to_string())],
        body: None,
        timeout: Duration::from_secs(30),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 5 * 1024 * 1024,
        // The fetcher's marker scan is a different, Cloudflare-flavored check —
        // the GR-specific `is_anti_bot_page` body check below owns this instead.
        anti_bot_check: false,
        user_agent: UserAgentProfile::Custom(GOODREADS_USER_AGENT.to_string()),
        priority,
    };
    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| map_transport_err("GR request", e))?;
    if !(200..300).contains(&resp.status) {
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Goodreads, BreakerSignal::Failure);
        }
        return Err(GoodreadsFetchError::HttpStatus(resp.status));
    }
    let html = String::from_utf8_lossy(&resp.body).into_owned();
    if crate::provider_util::is_anti_bot_page(&html) {
        // A 200-but-soft-blocked interstitial is a hard block on the
        // breaker, not a threshold-counted failure (R-8).
        outbound_queue::shared().report_outcome(
            RateBucket::Goodreads,
            BreakerSignal::TripImmediately { open_for: None },
        );
        return Err(GoodreadsFetchError::AntiBot);
    }
    outbound_queue::shared().report_outcome(RateBucket::Goodreads, BreakerSignal::Success);
    Ok(html)
}

/// Search Goodreads by TITLE ONLY via the WAF-free autocomplete JSON endpoint.
///
/// The author deliberately stays OUT of the query string: autocomplete
/// prefix-matches the raw string, so "Ender's Game Orson Scott Card" ranks
/// study guides whose TITLES contain "by Orson Scott Card" above the real
/// record — famous books drown in such parasites and the real book drops out
/// of the hit list entirely (38/135 imports, live 2026-07-03). Author
/// agreement is the PICKER's job (`gr_best_match` requires author-token
/// overlap against each hit's own author field), which is both stricter and
/// immune to that ranking poison.
pub async fn search_goodreads<F: HttpFetcher>(
    fetcher: &F,
    base_url: &str,
    title: &str,
    priority: RequestPriority,
) -> Result<Vec<GoodreadsSearchResult>, GoodreadsFetchError> {
    let base = base_url.trim_end_matches('/');
    let query = urlencoding::encode(title);
    let url = format!("{base}/book/auto_complete?format=json&q={query}");
    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![("Accept-Language".to_string(), "en-US,en;q=0.9".to_string())],
        body: None,
        timeout: Duration::from_secs(30),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Custom(GOODREADS_USER_AGENT.to_string()),
        priority,
    };
    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| map_transport_err("GR autocomplete", e))?;
    if !(200..300).contains(&resp.status) {
        return Err(GoodreadsFetchError::HttpStatus(resp.status));
    }
    let body = String::from_utf8_lossy(&resp.body).into_owned();
    Ok(parse_autocomplete_json(&body))
}

/// Fetch and parse a Goodreads detail page. Returns `Err(Parse)` if the page
/// loads but yields no useful fields.
pub async fn fetch_goodreads_detail<F: HttpFetcher>(
    fetcher: &F,
    detail_url: &str,
    priority: RequestPriority,
) -> Result<GoodreadsDetailResult, GoodreadsFetchError> {
    let html = fetch_goodreads_html(fetcher, detail_url, priority).await?;
    parse_detail_html(&html).ok_or(GoodreadsFetchError::Parse)
}

// =============================================================================
// LLM extraction fallback (foreign-language path)
// =============================================================================

/// System prompt for LLM-driven extraction from a foreign-language Goodreads
/// detail page. Used when direct JSON-LD + regex parsing fails (often on
/// foreign locales where GR's HTML structure differs).
///
/// The prompt is language-aware: it instructs the model to filter out
/// descriptions in unexpected languages so the validator's language guard
/// has clean inputs to work with.
const FOREIGN_LLM_EXTRACTION_PROMPT: &str = r#"You are a metadata extraction tool. Extract book details from the provided book detail page HTML.

Return ONLY a JSON object with exactly these fields:
- "title": string or null (book title in the work's primary language)
- "author": string or null (author name)
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
- "language": string (ISO 639-1 code) or null

Rules:
- Return ONLY the JSON object, no markdown fences, no explanation
- If a field is not visible on the page, use null
- Do NOT invent or guess missing data
- For cover_url, prefer the largest image version available
- For description, use ONLY text in the work's expected language (the language hint provided in the user message) or English. If the description is in another language, return null.
- For genres, use the most specific applicable tags"#;

/// LLM extraction response shape from Gemini.
#[derive(Debug, serde::Deserialize)]
struct LlmExtractionResult {
    title: Option<String>,
    author: Option<String>,
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
    language: Option<String>,
}

/// Extract foreign-language metadata from raw HTML using the configured LLM
/// (provider-agnostic OpenAI-compat).
///
/// Used as a fallback inside `GoodreadsClient::fetch` when direct JSON-LD +
/// regex parsing returns nothing useful.
///
/// Privacy: the prompt body contains only the cleaned page HTML and a
/// language hint. NO user-private fields (filenames, work IDs, etc.).
///
/// `endpoint` is the OpenAI-compat base URL the user configured (e.g.
/// `https://api.groq.com/openai/v1`,
/// `https://generativelanguage.googleapis.com/v1beta/openai`,
/// `https://api.openai.com/v1`). The function appends `/chat/completions`.
///
/// `language_hint` should be the work's expected language English-name (e.g.
/// "French", "Japanese") or "the original" if unknown — used to tell the LLM
/// which-language description to keep / drop.
///
/// `page_url` is the absolute detail-page URL the HTML was fetched from; it is
/// the base against which a relative `cover_url` from the LLM is resolved to an
/// absolute URL. A relative cover that cannot be resolved is dropped.
pub async fn extract_with_llm(
    http: &HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    raw_html: &str,
    language_hint: &str,
    page_url: &str,
) -> Result<crate::NormalizedWorkDetail, GoodreadsFetchError> {
    let cleaned = crate::provider_util::clean_html_for_llm(raw_html);
    if cleaned.is_empty() {
        return Err(GoodreadsFetchError::Parse);
    }

    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let user_msg = format!(
        "This book is in {language_hint}. Extract book details from this page. \
         For the description, use ONLY text in {language_hint} or English. \
         If the description is in a different language, return null for description.\n\n{cleaned}"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": FOREIGN_LLM_EXTRACTION_PROMPT},
            {"role": "user",   "content": user_msg},
        ],
        "temperature": 0.0,
        "response_format": {"type": "json_object"},
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| GoodreadsFetchError::Network(format!("LLM extract: {e}")))?;
    if !resp.status().is_success() {
        return Err(GoodreadsFetchError::HttpStatus(resp.status().as_u16()));
    }
    let envelope: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GoodreadsFetchError::Network(format!("LLM envelope: {e}")))?;
    let content_raw = envelope
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or(GoodreadsFetchError::Parse)?;
    // Tolerate code-fence wrapping that some providers add.
    let trimmed = content_raw.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let unfenced = unfenced.strip_suffix("```").unwrap_or(unfenced).trim();
    let result: LlmExtractionResult =
        serde_json::from_str(unfenced).map_err(|_| GoodreadsFetchError::Parse)?;

    let nfc = crate::normalize::nfc;
    let year = result
        .publish_date
        .as_deref()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse::<i32>().ok());
    let cover_url = result
        .cover_url
        .as_deref()
        .and_then(|u| crate::provider_util::validate_cover_url(u, page_url));

    Ok(crate::NormalizedWorkDetail {
        title: result.title.map(|s| nfc(&s)),
        subtitle: None,
        original_title: None,
        author_name: result.author.map(|s| nfc(&s)),
        description: result.description.map(|s| nfc(&s)),
        year,
        series_name: result.series_name.map(|s| nfc(&s)),
        series_position: result.series_position,
        genres: result
            .genres
            .map(|g| g.into_iter().map(|s| nfc(&s)).collect()),
        language: result
            .language
            .as_deref()
            .map(livrarr_domain::normalize_language),
        page_count: result.page_count.filter(|&p| p > 0),
        duration_seconds: None,
        publisher: result.publisher.map(|s| nfc(&s)),
        publish_date: result.publish_date,
        hc_key: None,
        gr_key: None,
        ol_key: None,
        isbn_13: result.isbn.filter(|s| s.len() >= 10),
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: result.rating,
        rating_count: result.rating_count,
        cover_url,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    })
}

// =============================================================================
// URL validation
// =============================================================================

/// Validate that a detail URL points to Goodreads (SSRF protection).
/// Accepts relative paths (`/book/show/...`) and absolute Goodreads URLs.
pub fn validate_detail_url(url: &str) -> bool {
    // Allow relative paths for internal use
    if url.starts_with("/book/show/") {
        return true;
    }

    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    if parsed.scheme() != "https" {
        return false;
    }

    match parsed.host_str() {
        Some(host) => host == "www.goodreads.com" || host == "goodreads.com",
        None => false,
    }
}

/// Validate that a cover URL is from an allowed host (SSRF protection).
/// HTTPS only — all Goodreads/Amazon CDNs serve HTTPS.
pub fn validate_cover_url(url: &str) -> bool {
    const ALLOWED_HOSTS: &[&str] = &[
        "i.gr-assets.com",
        "s.gr-assets.com",
        "m.media-amazon.com",
        "images-na.ssl-images-amazon.com",
        "images.gr-assets.com",
        "compressed.photo.goodreads.com",
    ];

    if let Ok(parsed) = url::Url::parse(url) {
        if parsed.scheme() != "https" {
            return false;
        }
        if let Some(host) = parsed.host_str() {
            return ALLOWED_HOSTS.contains(&host);
        }
    }

    false
}

// =============================================================================
// Author search page parsing (for GR author ID resolution)
// =============================================================================

/// A candidate author from a GR author search page.
#[derive(Debug, Clone)]
pub struct GoodreadsAuthorCandidate {
    pub gr_key: String,
    pub name: String,
    pub profile_url: String,
}

static RE_AUTHOR_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<a[^>]*href="(?:https://www\.goodreads\.com)?(/author/show/(\d+)[^"]*)"[^>]*>\s*(?:<span[^>]*>)?([^<]+?)(?:</span>)?\s*</a>"#)
        .unwrap()
});

/// Parse a Goodreads author search results page into candidates.
pub fn parse_author_search_html(html: &str) -> Vec<GoodreadsAuthorCandidate> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cap in RE_AUTHOR_ROW.captures_iter(html) {
        let profile_url = cap[1].to_string();
        let gr_key = cap[2].to_string();
        let name = decode_html_entities(cap[3].trim());

        if name.is_empty() || !seen.insert(gr_key.clone()) {
            continue;
        }

        results.push(GoodreadsAuthorCandidate {
            gr_key,
            name,
            profile_url,
        });
    }

    results
}

// =============================================================================
// Series list page parsing (for author's series)
// =============================================================================

/// A series entry from a GR author series list page.
#[derive(Debug, Clone)]
pub struct GoodreadsSeriesEntry {
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
}

static RE_SERIES_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<a[^>]*href="/series/(\d+)(?:-[^"]*)?"[^>]*>([^<]+)</a>"#).unwrap()
});

static RE_SERIES_BOOK_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\d+)\s+(?:books?|primary works?)"#).unwrap());

static RE_NEXT_PAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]*class="next_page"[^>]*href="([^"]+)""#).unwrap());

/// Parse a Goodreads series list page into series entries.
/// Returns (entries, has_next_page).
pub fn parse_series_list_html(html: &str) -> (Vec<GoodreadsSeriesEntry>, bool) {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Split by series link to process each series block.
    // Each series block contains the link and nearby book count text.
    for cap in RE_SERIES_LINK.captures_iter(html) {
        let gr_key = cap[1].to_string();
        let name = decode_html_entities(cap[2].trim());

        if name.is_empty() || !seen.insert(gr_key.clone()) {
            continue;
        }

        // Find book count near this match — look at the surrounding HTML context.
        // The book count typically follows the series link in the same row/block.
        let match_end = cap.get(0).unwrap().end();
        let mut ctx_end = std::cmp::min(match_end + 500, html.len());
        while ctx_end < html.len() && !html.is_char_boundary(ctx_end) {
            ctx_end += 1;
        }
        let context = &html[match_end..ctx_end];
        let book_count = RE_SERIES_BOOK_COUNT
            .captures(context)
            .and_then(|c| c[1].parse::<i32>().ok())
            .unwrap_or(0);

        results.push(GoodreadsSeriesEntry {
            name,
            gr_key,
            book_count,
        });
    }

    let has_next = RE_NEXT_PAGE.is_match(html);
    (results, has_next)
}

// =============================================================================
// Series detail page parsing (books in a series)
// =============================================================================

/// A book entry from a GR series detail page.
#[derive(Debug, Clone)]
pub struct GoodreadsSeriesBook {
    pub title: String,
    pub gr_key: String,
    pub position: Option<f64>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
}

/// Matches position headers: <h3...>Book 1</h3>, <h3...>Book 2.5</h3>
static RE_SERIES_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)<h3[^>]*>\s*Book\s+(\d+(?:\.\d+)?)\s*</h3>"#).unwrap());

static RE_IMG_SRC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"src="(https?://[^"]+)""#).unwrap());

/// Matches book title links after a heading.
static RE_SERIES_BOOK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?si)<a[^>]*href="(?:https://www\.goodreads\.com)?/book/show/(\d+)[^"]*"[^>]*>\s*(?:<span[^>]*>)?([^<]+?)(?:</span>)?\s*</a>"#,
    )
    .unwrap()
});

static RE_SERIES_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"published\s+(\d{4})"#).unwrap());

/// Decode common HTML entities in a string.
use livrarr_domain::decode_xml_entities as decode_html_entities;

/// Returns true if a title looks like an omnibus/collection rather than a single work.
fn is_collection_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    // Match common omnibus/collection patterns.
    lower.contains("collection")
        || lower.contains("omnibus")
        || lower.contains("complete ")
        || lower.contains("books collection")
        || lower.contains(" set,")
        || lower.contains(" set ")
        || (lower.contains("vol.") && lower.contains('-'))
}

/// Parse a Goodreads series detail page into book entries.
/// Returns (books, has_next_page).
///
/// Strategy: find all `<h3>Book N</h3>` headings, then find the first book `<a>` link
/// after each heading. This pairs positions with titles reliably regardless of HTML structure.
pub fn parse_series_detail_html(html: &str) -> (Vec<GoodreadsSeriesBook>, bool) {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Collect all position headings with their byte offsets.
    let headings: Vec<(usize, f64)> = RE_SERIES_HEADING
        .captures_iter(html)
        .filter_map(|cap| {
            let pos = cap[1].parse::<f64>().ok()?;
            Some((cap.get(0).unwrap().end(), pos))
        })
        .collect();

    // For each heading, find the first book link after it.
    for (i, &(heading_end, position)) in headings.iter().enumerate() {
        // Search region: from this heading to the next heading (or end of doc).
        let search_end = headings.get(i + 1).map(|h| h.0).unwrap_or(html.len());
        let search_region = &html[heading_end..search_end];

        let Some(book_cap) = RE_SERIES_BOOK.captures(search_region) else {
            continue;
        };

        let gr_key = book_cap[1].to_string();
        let raw_title = book_cap[2].trim().to_string();
        let title = decode_html_entities(&raw_title);

        if title.is_empty() || !seen.insert(gr_key.clone()) {
            continue;
        }

        // Filter out omnibus/collection editions.
        if is_collection_title(&title) {
            continue;
        }

        // Look for year after the book link.
        let book_end = heading_end + book_cap.get(0).unwrap().end();
        let mut year_end = std::cmp::min(book_end + 500, html.len());
        while year_end < html.len() && !html.is_char_boundary(year_end) {
            year_end += 1;
        }
        let post_context = &html[book_end..year_end];
        let year = RE_SERIES_YEAR
            .captures(post_context)
            .and_then(|c| c[1].parse::<i32>().ok());

        // Strip series info from title: "Book Title (Series, #1)" → "Book Title"
        let clean_title = if RE_TITLE_SERIES.is_match(&title) {
            RE_TITLE_SERIES.replace(&title, "").trim().to_string()
        } else {
            title
        };

        let cover_url = RE_IMG_SRC.captures_iter(search_region).find_map(|c| {
            let url = c[1].to_string();
            if url.contains("nophoto") || url.contains("loading-trans") {
                return None;
            }
            if validate_cover_url(&url) {
                Some(crate::provider_util::upscale_cover_url(&url))
            } else {
                None
            }
        });

        results.push(GoodreadsSeriesBook {
            title: clean_title,
            gr_key,
            position: Some(position),
            year,
            cover_url,
        });
    }

    let has_next = RE_NEXT_PAGE.is_match(html);
    (results, has_next)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Door-routing: fetch_goodreads_html / search_goodreads go through the
    // HttpFetcher trait with the Goodreads rate bucket, GET, the exact
    // Chrome UA string via UserAgentProfile::Custom (not a header — the
    // fetcher sets UA from `user_agent`), and the Accept-Language header.
    // anti_bot_check stays false: the app-level `is_anti_bot_page` body scan
    // owns anti-bot detection for Goodreads, not the fetcher's generic scan.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_goodreads_html_sends_goodreads_bucket_get_custom_ua_and_accept_language() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            b"<html><body>ok</body></html>".to_vec(),
        );

        let html = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert_eq!(html, "<html><body>ok</body></html>");
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, "https://www.goodreads.com/book/show/1");
        assert_eq!(req.rate_bucket, RateBucket::Goodreads);
        assert_eq!(req.method, HttpMethod::Get);
        match &req.user_agent {
            UserAgentProfile::Custom(ua) => assert_eq!(ua.as_str(), GOODREADS_USER_AGENT),
            other => panic!("expected UserAgentProfile::Custom, got {other:?}"),
        }
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Accept-Language" && v == "en-US,en;q=0.9"));
        assert!(!req.headers.iter().any(|(k, _)| k == "User-Agent"));
        assert!(!req.anti_bot_check);
        assert_eq!(req.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_anti_bot_body_to_antibot_error() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            br#"<html><div class="cf-browser-verification">Checking...</div></html>"#.to_vec(),
        );

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::AntiBot));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_fetcher_rate_limited_to_http_status_429() {
        // The fetcher intercepts HTTP 429 as `FetchError::RateLimited` rather
        // than a normal response — this must still surface as
        // `HttpStatus(429)` so `map_fetch_err` (provider_client.rs) keeps
        // classifying it as `WillRetry { RateLimit }`, not `ServerError`.
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::HttpStatus(429)));
    }

    #[tokio::test]
    async fn fetch_goodreads_html_maps_http_500_to_http_status() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);

        let err = fetch_goodreads_html(
            &fetcher,
            "https://www.goodreads.com/book/show/1",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::HttpStatus(500)));
    }

    #[tokio::test]
    async fn search_goodreads_sends_goodreads_bucket_get_autocomplete_url() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"[]".to_vec());

        let hits = search_goodreads(
            &fetcher,
            "https://www.goodreads.com",
            "Dune",
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(hits.is_empty());
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(
            req.url, "https://www.goodreads.com/book/auto_complete?format=json&q=Dune",
            "the query is the TITLE ONLY — an appended author makes autocomplete \
             rank study-guide titles containing the author's name above the real \
             record; author agreement belongs to the picker"
        );
        assert_eq!(req.rate_bucket, RateBucket::Goodreads);
        assert_eq!(req.method, HttpMethod::Get);
        match &req.user_agent {
            UserAgentProfile::Custom(ua) => assert_eq!(ua.as_str(), GOODREADS_USER_AGENT),
            other => panic!("expected UserAgentProfile::Custom, got {other:?}"),
        }
        assert!(!req.anti_bot_check);
        assert_eq!(req.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn search_goodreads_parses_canned_autocomplete_response() {
        let body = r#"[{"title":"The Hobbit","bookUrl":"/book/show/5907","author":{"name":"J.R.R. Tolkien"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let hits = search_goodreads(
            &fetcher,
            "https://www.goodreads.com",
            "Hobbit",
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "The Hobbit");
    }

    #[tokio::test]
    async fn search_goodreads_maps_fetcher_rate_limited_to_http_status_429() {
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = search_goodreads(
            &fetcher,
            "https://www.goodreads.com",
            "x",
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, GoodreadsFetchError::HttpStatus(429)));
    }

    // =========================================================================
    // Live-fetch helper (requires network — tests are #[ignore])
    // =========================================================================

    fn fetch_goodreads_page(url: &str) -> Option<String> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (compatible; Livrarr/0.1 test)")
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()?;
        let resp = client.get(url).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.text().ok()
    }

    // =========================================================================
    // Live-fetch search tests
    // =========================================================================

    #[test]
    #[ignore]
    fn live_search_german() {
        let html =
            fetch_goodreads_page("https://www.goodreads.com/search?q=Das+Parfum+S%C3%BCskind")
                .expect("fetch failed");
        let results = parse_search_html(&html);
        assert!(!results.is_empty(), "no results parsed");
        let book = results
            .iter()
            .find(|r| r.title.contains("Parfum"))
            .expect("no Parfum result");
        assert!(book.author.is_some());
        assert!(book.detail_url.starts_with("/book/show/"));
    }

    #[test]
    #[ignore]
    fn live_search_french() {
        let html = fetch_goodreads_page(
            "https://www.goodreads.com/search?q=Le+Petit+Prince+Saint-Exup%C3%A9ry",
        )
        .expect("fetch failed");
        let results = parse_search_html(&html);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.title.contains("Petit Prince")));
    }

    #[test]
    #[ignore]
    fn live_search_spanish() {
        let html = fetch_goodreads_page(
            "https://www.goodreads.com/search?q=El+problema+de+los+tres+cuerpos",
        )
        .expect("fetch failed");
        let results = parse_search_html(&html);
        assert!(!results.is_empty());
    }

    #[test]
    #[ignore]
    fn live_search_polish() {
        let html = fetch_goodreads_page("https://www.goodreads.com/search?q=Solaris+Stanislaw+Lem")
            .expect("fetch failed");
        let results = parse_search_html(&html);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.title.contains("Solaris")));
    }

    #[test]
    #[ignore]
    fn live_search_results_valid() {
        let html = fetch_goodreads_page("https://www.goodreads.com/search?q=Das+Parfum")
            .expect("fetch failed");
        let results = parse_search_html(&html);
        for r in &results {
            assert!(!r.title.is_empty());
            assert!(r.detail_url.starts_with("/book/show/"));
            if let Some(url) = &r.cover_url {
                assert!(!url.contains("nophoto"));
                assert!(!url.contains("loading-trans"));
            }
        }
    }

    // =========================================================================
    // Live-fetch detail tests
    // =========================================================================

    #[test]
    #[ignore]
    fn live_detail_german() {
        let html = fetch_goodreads_page("https://www.goodreads.com/book/show/2896.Das_Parfum")
            .expect("fetch failed");
        let result = parse_detail_html(&html).expect("should parse");
        assert!(result.title.is_some());
        assert!(result.author.is_some());
    }

    #[test]
    #[ignore]
    fn live_detail_french() {
        let html =
            fetch_goodreads_page("https://www.goodreads.com/book/show/157993.Le_Petit_Prince")
                .expect("fetch failed");
        let result = parse_detail_html(&html).expect("should parse");
        assert!(result.title.is_some());
        assert!(result.author.is_some());
    }

    #[test]
    #[ignore]
    fn live_detail_polish() {
        let html = fetch_goodreads_page("https://www.goodreads.com/book/show/40603587-wied-min")
            .expect("fetch failed");
        let result = parse_detail_html(&html).expect("should parse");
        assert!(result.title.is_some());
        assert!(result.author.is_some());
    }

    #[test]
    #[ignore]
    fn live_detail_has_jsonld() {
        let html = fetch_goodreads_page("https://www.goodreads.com/book/show/2896.Das_Parfum")
            .expect("fetch failed");
        let result = parse_detail_html(&html).expect("should parse");
        assert!(result.title.is_some(), "missing title");
        assert!(result.author.is_some(), "missing author");
        assert!(result.rating.is_some(), "missing rating");
    }

    #[test]
    #[ignore]
    fn live_detail_description_is_plain_text() {
        let html = fetch_goodreads_page("https://www.goodreads.com/book/show/2896.Das_Parfum")
            .expect("fetch failed");
        let result = parse_detail_html(&html).expect("should parse");
        if let Some(desc) = &result.description {
            assert!(!desc.contains("<br"), "HTML in description");
            assert!(!desc.contains("<p"), "HTML in description");
            assert!(!desc.contains("<span"), "HTML in description");
        }
    }

    // =========================================================================
    // URL validation tests
    // =========================================================================

    #[test]
    fn validate_goodreads_detail_urls() {
        assert!(validate_detail_url(
            "https://www.goodreads.com/book/show/2896.Das_Parfum"
        ));
        assert!(validate_detail_url("/book/show/2896.Das_Parfum"));
        assert!(!validate_detail_url("https://evil.com/book/show/123"));
        assert!(!validate_detail_url(
            "https://www.goodreads.com.evil.com/book/show/123"
        ));
        assert!(!validate_detail_url("javascript:alert(1)"));
    }

    #[test]
    fn validate_goodreads_cover_urls() {
        assert!(validate_cover_url(
            "https://i.gr-assets.com/images/S/compressed.photo.goodreads.com/books/123.jpg"
        ));
        assert!(validate_cover_url(
            "https://m.media-amazon.com/images/I/123.jpg"
        ));
        assert!(validate_cover_url(
            "https://images-na.ssl-images-amazon.com/images/I/123.jpg"
        ));
        assert!(!validate_cover_url("https://evil.com/image.jpg"));
        assert!(!validate_cover_url("ftp://i.gr-assets.com/image.jpg"));
        assert!(!validate_cover_url("javascript:alert(1)"));
    }

    // =========================================================================
    // Empty / malformed input tests
    // =========================================================================

    #[test]
    fn search_empty_html_returns_empty() {
        assert!(parse_search_html("").is_empty());
        assert!(parse_search_html("<html></html>").is_empty());
    }

    #[test]
    fn search_relative_cover_resolves_to_absolute() {
        // A relative cover `src` on a search-results row is resolved against the
        // Goodreads base, never stored relative.
        let html = r#"<tr itemscope itemtype="https://schema.org/Book">
            <td><img class="bookCover" src="/images/cover/123.jpg"></td>
            <td><a class="bookTitle" href="/book/show/123"><span>A Book</span></a></td>
            </tr>"#;
        let results = parse_search_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].cover_url.as_deref(),
            Some("https://www.goodreads.com/images/cover/123.jpg")
        );
    }

    #[test]
    fn autocomplete_json_parses_card_fields() {
        // A trimmed real `/book/auto_complete` entry (measured 2026-06-01).
        let body = r#"[{"imageUrl":"https://i.gr-assets.com/images/S/compressed.photo.goodreads.com/books/1546071216i/5907._SY75_.jpg","bookId":"5907","workId":"1540236","bookUrl":"/book/show/5907.The_Hobbit_or_There_and_Back_Again","title":"The Hobbit, or There and Back Again","avgRating":"4.30","author":{"id":656983,"name":"J.R.R. Tolkien"}}]"#;
        let results = parse_autocomplete_json(body);
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.title, "The Hobbit, or There and Back Again");
        assert_eq!(r.author.as_deref(), Some("J.R.R. Tolkien"));
        assert_eq!(
            r.detail_url,
            "/book/show/5907.The_Hobbit_or_There_and_Back_Again"
        );
        assert_eq!(r.rating.as_deref(), Some("4.30"));
        // The `_SY75_` thumbnail size token is stripped to the full-size cover.
        let cover = r.cover_url.as_deref().expect("cover");
        assert!(cover.ends_with("5907.jpg"), "cover not upscaled: {cover}");
        assert!(
            !cover.contains("_SY75_"),
            "size token not stripped: {cover}"
        );
    }

    #[test]
    fn autocomplete_numeric_avg_rating_entry_does_not_poison_the_batch() {
        // Live regression shape (Pandora's Star, measured 2026-07-03): GR
        // returns avgRating as a STRING on most entries but as a bare JSON
        // NUMBER (0.0) on some unrated editions. The old whole-batch parse
        // failed on that one entry and silently erased every hit for the
        // query. All four entries must survive; the numeric-0.0 entry parses
        // with its rating filtered as unrated.
        let body = r#"[
            {"imageUrl":"https://i.gr-assets.com/images/S/x/45252._SX50_.jpg","bookId":"45252","workId":"987015","bookUrl":"/book/show/45252.Pandora_s_Star","from_search":true,"from_srp":true,"qid":"DBVOC7gZj0","rank":1,"title":"Pandora's Star (Commonwealth Saga, #1)","bookTitleBare":"Pandora's Star","numPages":988,"avgRating":"4.22","ratingsCount":88123,"author":{"id":25375,"name":"Peter F. Hamilton"},"kcrPreviewUrl":null,"description":{"html":"x","truncated":true,"fullContentUrl":"https://www.goodreads.com/book/show/45252"}},
            {"imageUrl":"https://i.gr-assets.com/images/S/x/nophoto.jpg","bookId":"138001619","workId":"1","bookUrl":"/book/show/138001619","from_search":true,"from_srp":true,"qid":"DBVOC7gZj0","rank":2,"title":"Pandora's Star by Hamilton, Peter F. [MassMarket(2005)]","bookTitleBare":"Pandora's Star","numPages":null,"avgRating":0.0,"ratingsCount":0,"author":{"id":2,"name":"Peter F. Hamilton"},"kcrPreviewUrl":null},
            {"imageUrl":"https://i.gr-assets.com/images/S/x/3.jpg","bookId":"219187841","workId":"3","bookUrl":"/book/show/219187841","rank":3,"title":"Pandora's Star (Commonwealth Saga) by Peter F. Hamilton","avgRating":"5.00","ratingsCount":1,"author":{"id":2,"name":"Peter F. Hamilton"}},
            {"imageUrl":"https://i.gr-assets.com/images/S/x/4.jpg","bookId":"226763120","workId":"4","bookUrl":"/book/show/226763120","rank":4,"title":"Pandora's Star: Commonwealth Saga 1","avgRating":"4.00","ratingsCount":2,"author":{"id":2,"name":"Peter F. Hamilton"}}
        ]"#;
        let results = parse_autocomplete_json(body);
        assert_eq!(
            results.len(),
            4,
            "one numeric-avgRating entry must never erase the batch"
        );
        assert_eq!(results[0].title, "Pandora's Star (Commonwealth Saga, #1)");
        assert_eq!(
            results[0].title_bare.as_deref(),
            Some("Pandora's Star"),
            "bookTitleBare rides along so matching sees GR's own undecorated title"
        );
        assert_eq!(results[0].rating.as_deref(), Some("4.22"));
        assert_eq!(
            results[1].rating, None,
            "numeric 0.0 normalizes to \"0.00\" and filters as unrated"
        );
        assert_eq!(results[1].author.as_deref(), Some("Peter F. Hamilton"));
    }

    #[test]
    fn autocomplete_structurally_broken_entry_drops_alone() {
        // An entry whose fields are the wrong SHAPE entirely (bookUrl as an
        // object) drops by itself; its neighbors survive.
        let body = r#"[
            {"bookUrl":{"nested":"garbage"},"title":"Broken Entry","avgRating":"4.00"},
            {"bookId":"5907","bookUrl":"/book/show/5907","title":"Survivor","avgRating":"4.30","author":{"name":"J.R.R. Tolkien"}}
        ]"#;
        let results = parse_autocomplete_json(body);
        assert_eq!(results.len(), 1, "the broken entry drops alone");
        assert_eq!(results[0].title, "Survivor");
    }

    #[test]
    fn autocomplete_relative_cover_resolves_to_absolute() {
        // A relative `imageUrl` must be resolved against the Goodreads base so
        // it never reaches the work as a non-downloadable relative URL.
        let body = r#"[{"imageUrl":"/assets/cover/5907.jpg","bookId":"5907","bookUrl":"/book/show/5907","title":"Some Book","author":{"name":"Author"}}]"#;
        let results = parse_autocomplete_json(body);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].cover_url.as_deref(),
            Some("https://www.goodreads.com/assets/cover/5907.jpg")
        );
    }

    #[test]
    fn autocomplete_non_http_cover_is_dropped() {
        // A non-http(s) cover (e.g. a data: or javascript: payload) is dropped
        // rather than stored.
        let body = r#"[{"imageUrl":"javascript:alert(1)","bookId":"1","bookUrl":"/book/show/1","title":"X","author":{"name":"A"}}]"#;
        let results = parse_autocomplete_json(body);
        assert_eq!(results.len(), 1);
        assert!(results[0].cover_url.is_none());
    }

    #[test]
    fn autocomplete_non_json_is_empty() {
        // A WAF interstitial / HTML challenge body is a miss, not an error.
        assert!(parse_autocomplete_json("<html>challenge</html>").is_empty());
        assert!(parse_autocomplete_json("").is_empty());
        // An entry with no title/url is skipped.
        assert!(parse_autocomplete_json(r#"[{"avgRating":"4.0"}]"#).is_empty());
    }

    #[test]
    fn extract_gr_key_normalizes_to_bare_numeric() {
        // A picked Goodreads result must persist its anchor in the domain canonical
        // form (bare numeric), not the slug — so matching/conflict stay consistent.
        let slug = extract_gr_key("/book/show/5907.The_Hobbit_or_There_and_Back_Again")
            .expect("gr_key from a /book/show url");
        assert_eq!(slug, "5907.The_Hobbit_or_There_and_Back_Again");
        let canonical = livrarr_domain::normalization::normalize_gr_key(&slug).expect("normalizes");
        assert_eq!(canonical, "5907");
    }

    #[test]
    fn detail_empty_html_returns_none() {
        assert!(parse_detail_html("").is_none());
        assert!(parse_detail_html("<html></html>").is_none());
    }

    // =========================================================================
    // Edge case / stress tests (Block 4)
    // =========================================================================

    #[test]
    fn search_http_vs_https_schema_url() {
        // Both http and https variants of schema.org should match.
        let html_http = r#"<tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/123"><span>Test Book HTTP</span></a>
            <a class="authorName"><span>Author A</span></a>
        </tr>"#;
        let html_https = r#"<tr itemscope itemtype="https://schema.org/Book">
            <a class="bookTitle" href="/book/show/456"><span>Test Book HTTPS</span></a>
            <a class="authorName"><span>Author B</span></a>
        </tr>"#;

        let r1 = parse_search_html(html_http);
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].title, "Test Book HTTP");

        let r2 = parse_search_html(html_https);
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].title, "Test Book HTTPS");
    }

    #[test]
    fn search_rows_with_missing_title_are_skipped() {
        let html = r#"
        <tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/123"><span></span></a>
        </tr>
        <tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/456"><span>Valid Title</span></a>
        </tr>"#;

        let results = parse_search_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Valid Title");
    }

    #[test]
    fn search_rows_without_title_anchor_are_skipped() {
        // Row with no bookTitle anchor at all.
        let html = r#"
        <tr itemscope itemtype="http://schema.org/Book">
            <span>Something else</span>
        </tr>
        <tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/456"><span>Real Book</span></a>
        </tr>"#;

        let results = parse_search_html(html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Real Book");
    }

    #[test]
    fn search_detail_url_query_params_stripped() {
        let html = r#"<tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/123.Title?from_search=true&amp;from_srp=true&amp;qid=abc">
                <span>My Book</span>
            </a>
        </tr>"#;

        let results = parse_search_html(html);
        assert_eq!(results[0].detail_url, "/book/show/123.Title");
    }

    #[test]
    fn search_nophoto_covers_filtered() {
        let html = r#"<tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/1"><span>Book A</span></a>
            <img class="bookCover" src="https://s.gr-assets.com/nophoto/book/111.jpg">
        </tr>"#;

        let results = parse_search_html(html);
        assert_eq!(results.len(), 1);
        assert!(results[0].cover_url.is_none());
    }

    #[test]
    fn search_loading_trans_covers_filtered() {
        let html = r#"<tr itemscope itemtype="http://schema.org/Book">
            <a class="bookTitle" href="/book/show/1"><span>Book B</span></a>
            <img class="bookCover" src="https://s.gr-assets.com/loading-trans.gif">
        </tr>"#;

        let results = parse_search_html(html);
        assert!(results[0].cover_url.is_none());
    }

    #[test]
    fn detail_jsonld_direct_book_object() {
        let html = r#"<html><script type="application/ld+json">
        {"@context":"https://schema.org","@type":"Book","name":"Direct Book","author":[{"@type":"Person","name":"Test Author"}],"isbn":"9781234567890","aggregateRating":{"ratingValue":4.5,"ratingCount":100},"numberOfPages":300,"inLanguage":"English","image":"https://m.media-amazon.com/images/I/test.jpg"}
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.title.as_deref(), Some("Direct Book"));
        assert_eq!(result.author.as_deref(), Some("Test Author"));
        assert_eq!(result.isbn.as_deref(), Some("9781234567890"));
        assert!((result.rating.unwrap() - 4.5).abs() < 0.01);
        assert_eq!(result.rating_count, Some(100));
        assert_eq!(result.page_count, Some(300));
        assert_eq!(result.language.as_deref(), Some("en"));
    }

    #[test]
    fn detail_jsonld_multiple_blocks_finds_book() {
        // Breadcrumbs + Organization + Book — should find the Book.
        let html = r#"<html>
        <script type="application/ld+json">
        {"@context":"https://schema.org","@type":"BreadcrumbList","itemListElement":[]}
        </script>
        <script type="application/ld+json">
        {"@context":"https://schema.org","@type":"Organization","name":"Goodreads"}
        </script>
        <script type="application/ld+json">
        {"@context":"https://schema.org","@type":"Book","name":"Found Me","author":{"@type":"Person","name":"Author X"}}
        </script>
        </html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.title.as_deref(), Some("Found Me"));
        assert_eq!(result.author.as_deref(), Some("Author X"));
    }

    #[test]
    fn detail_jsonld_graph_wrapper() {
        let html = r#"<html><script type="application/ld+json">
        {"@context":"https://schema.org","@graph":[
            {"@type":"WebPage","name":"A page"},
            {"@type":"Book","name":"Graph Book","author":[{"@type":"Person","name":"Graph Author"}],"isbn":"9780000000001"}
        ]}
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.title.as_deref(), Some("Graph Book"));
        assert_eq!(result.author.as_deref(), Some("Graph Author"));
        assert_eq!(result.isbn.as_deref(), Some("9780000000001"));
    }

    #[test]
    fn detail_jsonld_array_format() {
        let html = r#"<html><script type="application/ld+json">
        [
            {"@type":"WebPage","name":"A page"},
            {"@type":"Book","name":"Array Book","author":[{"@type":"Person","name":"Arr Author"}]}
        ]
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.title.as_deref(), Some("Array Book"));
    }

    #[test]
    fn detail_author_as_object() {
        let html = r#"<html><script type="application/ld+json">
        {"@type":"Book","name":"Obj Author Book","author":{"@type":"Person","name":"Singular Author"}}
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.author.as_deref(), Some("Singular Author"));
    }

    #[test]
    fn detail_author_as_string() {
        let html = r#"<html><script type="application/ld+json">
        {"@type":"Book","name":"Str Author Book","author":"Plain String Author"}
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.author.as_deref(), Some("Plain String Author"));
    }

    #[test]
    fn detail_string_vs_number_tolerance() {
        // Rating as string, page count as string.
        let html = r#"<html><script type="application/ld+json">
        {"@type":"Book","name":"Tolerant Book","aggregateRating":{"ratingValue":"3.99","ratingCount":"500"},"numberOfPages":"256"}
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert!((result.rating.unwrap() - 3.99).abs() < 0.01);
        assert_eq!(result.rating_count, Some(500));
        assert_eq!(result.page_count, Some(256));
    }

    #[test]
    fn detail_description_html_stripped() {
        let html = r#"<html>
        <script type="application/ld+json">{"@type":"Book","name":"HTML Desc"}</script>
        <span class="Formatted">This is <b>bold</b> and <i>italic</i> and <br/>a newline and <a href="http://example.com">a link</a>.</span>
        </html>"#;

        let result = parse_detail_html(html).unwrap();
        let desc = result.description.unwrap();
        assert!(!desc.contains("<b>"));
        assert!(!desc.contains("<i>"));
        assert!(!desc.contains("<br"));
        assert!(!desc.contains("<a "));
        assert!(desc.contains("bold"));
        assert!(desc.contains("italic"));
    }

    #[test]
    fn detail_genres_deduplicated() {
        let html = r#"<html>
        <script type="application/ld+json">{"@type":"Book","name":"Genre Book"}</script>
        <a href="https://www.goodreads.com/genres/fantasy">Fantasy</a>
        <a href="https://www.goodreads.com/genres/fantasy">Fantasy</a>
        <a href="https://www.goodreads.com/genres/sci-fi">Sci-Fi</a>
        <a href="https://www.goodreads.com/genres/fantasy">Fantasy</a>
        </html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.genres.len(), 2);
        assert_eq!(result.genres[0], "fantasy");
        assert_eq!(result.genres[1], "sci-fi");
    }

    #[test]
    fn detail_html_entities_in_title() {
        let html = r#"<html><script type="application/ld+json">
        {"@type":"Book","name":"L&apos;étranger &amp; Other Stories"}
        </script></html>"#;

        let result = parse_detail_html(html).unwrap();
        assert_eq!(result.title.as_deref(), Some("L'étranger & Other Stories"));
    }

    #[test]
    fn detail_no_jsonld_but_has_description() {
        // Page with no JSON-LD at all, but has description via regex.
        let html = r#"<html>
        <span class="Formatted">A great book about testing edge cases.</span>
        </html>"#;

        let result = parse_detail_html(html).unwrap();
        assert!(result.title.is_none());
        assert!(result.author.is_none());
        assert!(result.description.is_some());
        assert!(result.description.as_ref().unwrap().contains("edge cases"));
    }

    #[test]
    fn detail_malformed_jsonld_ignored() {
        // Malformed JSON in ld+json block — should not crash, should return regex data.
        let html = r#"<html>
        <script type="application/ld+json">{this is not valid json}</script>
        <span class="Formatted">Fallback description here.</span>
        </html>"#;

        let result = parse_detail_html(html).unwrap();
        assert!(result.title.is_none()); // No JSON-LD parsed
        assert!(result.description.is_some()); // But regex found description
    }

    // =========================================================================
    // SSRF validation edge cases
    // =========================================================================

    #[test]
    fn ssrf_detail_url_rejects_non_goodreads() {
        assert!(!validate_detail_url("https://evil.com/book/show/123"));
        assert!(!validate_detail_url("http://localhost/book/show/123"));
        assert!(!validate_detail_url("file:///etc/passwd"));
        assert!(!validate_detail_url("data:text/html,<h1>XSS</h1>"));
    }

    #[test]
    fn ssrf_detail_url_accepts_goodreads() {
        assert!(validate_detail_url(
            "https://www.goodreads.com/book/show/123.Title"
        ));
        assert!(validate_detail_url("/book/show/123.Title"));
    }

    #[test]
    fn ssrf_cover_url_rejects_private_hosts() {
        assert!(!validate_cover_url("https://192.168.1.1/image.jpg"));
        assert!(!validate_cover_url("https://10.0.0.1/image.jpg"));
        assert!(!validate_cover_url("https://localhost/image.jpg"));
        assert!(!validate_cover_url("http://127.0.0.1/image.jpg"));
    }

    #[test]
    fn ssrf_cover_url_rejects_non_http_schemes() {
        assert!(!validate_cover_url("ftp://i.gr-assets.com/image.jpg"));
        assert!(!validate_cover_url("javascript:alert(document.cookie)"));
        assert!(!validate_cover_url("data:image/png;base64,abc"));
    }

    #[test]
    fn ssrf_cover_url_allows_known_cdns() {
        assert!(validate_cover_url(
            "https://i.gr-assets.com/images/S/compressed.photo.goodreads.com/books/123.jpg"
        ));
        assert!(validate_cover_url(
            "https://m.media-amazon.com/images/I/test.jpg"
        ));
        assert!(validate_cover_url(
            "https://images-na.ssl-images-amazon.com/images/I/test.jpg"
        ));
        assert!(validate_cover_url(
            "https://images.gr-assets.com/books/123.jpg"
        ));
    }
}
