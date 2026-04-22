//! Goodreads HTML parsing — regex extraction from search pages, JSON-LD + regex from detail pages.
//!
//! Replaces LLM-based scraping with direct HTML parsing for foreign language works.
//! LLM is kept as fallback only (see fallback chain in design doc).

use livrarr_http::HttpClient;
use regex::Regex;
use std::sync::LazyLock;

// =============================================================================
// Types
// =============================================================================

/// A single book result extracted from a Goodreads search results page.
#[derive(Debug, Clone)]
pub struct GoodreadsSearchResult {
    pub title: String,
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

        // Cover URL (optional, filter placeholders)
        let cover_url = RE_COVER.captures(row).and_then(|c| {
            let url = c[1].to_string();
            if url.contains("nophoto") || url.contains("loading-trans") {
                None
            } else {
                Some(url)
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
            title: clean_title,
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
/// (legacy English / foreign paths in `livrarr-server`).
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

/// Fetch a Goodreads HTML page. Adds the Chrome UA header, treats
/// non-success status and anti-bot challenge pages as errors.
///
/// Used by both the queue path (`GoodreadsClient` in `provider_client`) and
/// the legacy English/foreign paths in `livrarr-server`. Pacing is the
/// caller's responsibility — queue dispatch goes through the per-provider
/// `TokenBucket`; legacy paths still call `state.goodreads_rate_limiter`
/// before invoking this.
pub async fn fetch_goodreads_html(
    http: &HttpClient,
    url: &str,
) -> Result<String, GoodreadsFetchError> {
    let resp = http
        .get(url)
        .header("User-Agent", GOODREADS_USER_AGENT)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| GoodreadsFetchError::Network(format!("GR request: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(GoodreadsFetchError::HttpStatus(status.as_u16()));
    }
    let html = resp
        .text()
        .await
        .map_err(|e| GoodreadsFetchError::Network(format!("GR body: {e}")))?;
    if crate::llm_scraper::is_anti_bot_page(&html) {
        return Err(GoodreadsFetchError::AntiBot);
    }
    Ok(html)
}

/// Search Goodreads by `title author` and parse the results page.
pub async fn search_goodreads(
    http: &HttpClient,
    base_url: &str,
    title: &str,
    author: &str,
) -> Result<Vec<GoodreadsSearchResult>, GoodreadsFetchError> {
    let base = base_url.trim_end_matches('/');
    let raw_query = format!("{title} {author}");
    let query = urlencoding::encode(&raw_query);
    let url = format!("{base}/search?q={query}");
    let html = fetch_goodreads_html(http, &url).await?;
    Ok(parse_search_html(&html))
}

/// Fetch and parse a Goodreads detail page. Returns `Err(Parse)` if the page
/// loads but yields no useful fields.
pub async fn fetch_goodreads_detail(
    http: &HttpClient,
    detail_url: &str,
) -> Result<GoodreadsDetailResult, GoodreadsFetchError> {
    let html = fetch_goodreads_html(http, detail_url).await?;
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
/// regex parsing returns nothing useful. Lifts the legacy
/// `enrich_foreign_work` LLM-extraction logic out of livrarr-server.
///
/// Privacy: the prompt body contains only the cleaned page HTML and a
/// language hint. NO Pete-private fields (filenames, work IDs, etc.).
///
/// `endpoint` is the OpenAI-compat base URL the user configured (e.g.
/// `https://api.groq.com/openai/v1`,
/// `https://generativelanguage.googleapis.com/v1beta/openai`,
/// `https://api.openai.com/v1`). The function appends `/chat/completions`.
///
/// `language_hint` should be the work's expected language English-name (e.g.
/// "French", "Japanese") or "the original" if unknown — used to tell the LLM
/// which-language description to keep / drop.
pub async fn extract_with_llm(
    http: &HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    raw_html: &str,
    language_hint: &str,
) -> Result<crate::NormalizedWorkDetail, GoodreadsFetchError> {
    let cleaned = crate::llm_scraper::clean_html_for_llm(raw_html);
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
        .and_then(|u| crate::llm_scraper::validate_cover_url(u, ""));

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
}

/// Matches position headers: <h3...>Book 1</h3>, <h3...>Book 2.5</h3>
static RE_SERIES_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)<h3[^>]*>\s*Book\s+(\d+(?:\.\d+)?)\s*</h3>"#).unwrap());

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
fn decode_html_entities(s: &str) -> String {
    s.replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

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

        results.push(GoodreadsSeriesBook {
            title: clean_title,
            gr_key,
            position: Some(position),
            year,
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
