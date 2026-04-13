//! Goodreads HTML parsing — regex extraction from search pages, JSON-LD + regex from detail pages.
//!
//! Replaces LLM-based scraping with direct HTML parsing for foreign language works.
//! LLM is kept as fallback only (see fallback chain in design doc).

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
            .map(|s| s.to_string());

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
// URL validation
// =============================================================================

/// Validate that a detail URL points to Goodreads (SSRF protection).
pub fn validate_detail_url(url: &str) -> bool {
    url.starts_with("https://www.goodreads.com/") || url.starts_with("/book/show/")
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
    Regex::new(r#"(?si)<a[^>]*href="/series/(\d+)-[^"]*"[^>]*>([^<]+)</a>"#).unwrap()
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
    use std::path::Path;

    fn fixture_search(filename: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-html")
            .join(filename);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path.display(), e))
    }

    fn fixture_detail(filename: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-detail")
            .join(filename);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path.display(), e))
    }

    // =========================================================================
    // Helper to find a result by expected title substring
    // =========================================================================

    fn find_result<'a>(
        results: &'a [GoodreadsSearchResult],
        title_contains: &str,
    ) -> &'a GoodreadsSearchResult {
        results
            .iter()
            .find(|r| r.title.contains(title_contains))
            .unwrap_or_else(|| {
                panic!(
                    "No result containing '{}' in titles: {:?}",
                    title_contains,
                    results.iter().map(|r| &r.title).collect::<Vec<_>>()
                )
            })
    }

    // =========================================================================
    // Search page tests — German (de)
    // =========================================================================

    #[test]
    fn search_de_31_das_parfum() {
        let html = fixture_search("de_31_Das_Parfum__Die_Geschichte_eines_M\u{00f6}rders.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 5);

        let book = find_result(&results, "Das Parfum: Die Geschichte");
        assert_eq!(
            book.title,
            "Das Parfum: Die Geschichte eines M\u{00f6}rders"
        );
        assert_eq!(book.author.as_deref(), Some("Patrick S\u{00fc}skind"));
        assert!(book.detail_url.starts_with("/book/show/"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_de_39_die_krone_der_sterne() {
        let html = fixture_search("de_39_Die_Krone_der_Sterne.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 6);

        let book = find_result(&results, "Die Krone der Sterne");
        assert_eq!(book.author.as_deref(), Some("Kai Meyer"));
        assert!(book.detail_url.starts_with("/book/show/"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_de_49_qualityland() {
        let html = fixture_search("de_49_QualityLand__QualityLand___1_.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 1);

        assert_eq!(results[0].title, "QualityLand");
        assert_eq!(results[0].series_name.as_deref(), Some("QualityLand"));
        assert_eq!(results[0].series_position, Some(1.0));
        assert_eq!(results[0].author.as_deref(), Some("Marc-Uwe Kling"));
        assert!(results[0].detail_url.starts_with("/book/show/"));
        assert!(results[0].cover_url.is_some());
    }

    #[test]
    fn search_de_50_qualityland_2() {
        let html = fixture_search("de_50_QualityLand_2_0__QualityLand___2_.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 1);

        assert_eq!(results[0].title, "QualityLand 2.0");
        assert_eq!(results[0].series_name.as_deref(), Some("QualityLand"));
        assert_eq!(results[0].series_position, Some(2.0));
        assert_eq!(results[0].author.as_deref(), Some("Marc-Uwe Kling"));
        assert!(results[0].cover_url.is_some());
    }

    #[test]
    fn search_de_51_tintenherz() {
        let html = fixture_search("de_51_Tintenherz__Tintenwelt___1_.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 1);

        assert_eq!(results[0].title, "Tintenherz");
        assert_eq!(results[0].series_name.as_deref(), Some("Tintenwelt"));
        assert_eq!(results[0].series_position, Some(1.0));
        assert_eq!(results[0].author.as_deref(), Some("Cornelia Funke"));
        assert!(results[0].cover_url.is_some());
    }

    // =========================================================================
    // Search page tests — Spanish (es)
    // =========================================================================

    #[test]
    fn search_es_29_el_ojo_del_mundo() {
        let html = fixture_search("es_29_El_ojo_del_mundo__La_rueda_del_tiempo___.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 2);

        let book = find_result(&results, "El ojo del mundo");
        assert_eq!(book.author.as_deref(), Some("Robert Jordan"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_es_45_tres_cuerpos() {
        let html = fixture_search("es_45_El_problema_de_los_tres_cuerpos.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 6);

        let book = find_result(&results, "El problema de los tres cuerpos");
        assert_eq!(book.author.as_deref(), Some("Liu Cixin"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_es_46_lagrimas() {
        let html = fixture_search("es_46_L\u{00e1}grimas_en_la_lluvia.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 3);

        let book = find_result(&results, "L\u{00e1}grimas en la lluvia");
        assert_eq!(book.author.as_deref(), Some("Rosa Montero"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_es_47_sin_noticias() {
        let html = fixture_search("es_47_Sin_noticias_de_Gurb.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 2);

        let book = find_result(&results, "Sin noticias de Gurb");
        assert_eq!(book.author.as_deref(), Some("Eduardo Mendoza"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_es_48_klara() {
        let html = fixture_search("es_48_Klara_y_el_Sol.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 2);

        let book = find_result(&results, "Klara y el Sol");
        assert_eq!(book.author.as_deref(), Some("Kazuo Ishiguro"));
        assert!(book.cover_url.is_some());
    }

    // =========================================================================
    // Search page tests — French (fr)
    // =========================================================================

    #[test]
    fn search_fr_30_la_nuit() {
        let html = fixture_search("fr_30_La_Nuit_des_temps.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 8);

        let book = find_result(&results, "La Nuit des temps");
        assert!(book.detail_url.starts_with("/book/show/"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_fr_34_le_petit_prince() {
        let html = fixture_search("fr_34_Le_Petit_Prince.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 19);

        // Many results — verify some have covers and some don't (nophoto filtering)
        let with_covers = results.iter().filter(|r| r.cover_url.is_some()).count();
        assert_eq!(with_covers, 13);
    }

    #[test]
    fn search_fr_36_letranger() {
        let html = fixture_search("fr_36_L_\u{00c9}tranger.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 14);

        let with_covers = results.iter().filter(|r| r.cover_url.is_some()).count();
        assert_eq!(with_covers, 11);
    }

    #[test]
    fn search_fr_37_horde() {
        let html = fixture_search("fr_37_La_Horde_du_Contrevent.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 3);

        let book = find_result(&results, "La Horde du Contrevent");
        assert_eq!(book.author.as_deref(), Some("Alain Damasio"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_fr_38_les_furtifs() {
        let html = fixture_search("fr_38_Les_Furtifs.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 1);

        assert_eq!(results[0].title, "Les Furtifs");
        assert_eq!(results[0].author.as_deref(), Some("Alain Damasio"));
        assert!(results[0].cover_url.is_some());
    }

    // =========================================================================
    // Search page tests — Polish (pl)
    // =========================================================================

    #[test]
    fn search_pl_32_solaris() {
        let html = fixture_search("pl_32_Solaris.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 17);

        let book = find_result(&results, "Solaris");
        // Author should be found for the main Solaris entry
        assert!(book.detail_url.starts_with("/book/show/"));
        assert!(book.cover_url.is_some());
    }

    #[test]
    fn search_pl_35_pan_tadeusz() {
        let html = fixture_search("pl_35_Pan_Tadeusz.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 19);

        // Pan Tadeusz has many editions, some without covers
        let with_covers = results.iter().filter(|r| r.cover_url.is_some()).count();
        assert_eq!(with_covers, 10);
    }

    #[test]
    fn search_pl_40_lod() {
        let html = fixture_search("pl_40_L\u{00f3}d.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 1);

        assert_eq!(results[0].title, "L\u{00f3}d");
        assert_eq!(results[0].author.as_deref(), Some("Jacek Dukaj"));
        assert!(results[0].cover_url.is_some());
    }

    #[test]
    fn search_pl_42_perfekcyjna() {
        let html = fixture_search("pl_42_Perfekcyjna_niedoskona\u{0142}o\u{015b}\u{0107}.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 1);

        assert_eq!(
            results[0].title,
            "Perfekcyjna niedoskona\u{0142}o\u{015b}\u{0107}"
        );
        assert_eq!(results[0].author.as_deref(), Some("Jacek Dukaj"));
    }

    #[test]
    fn search_pl_44_wiedzmin() {
        let html = fixture_search("pl_44_Wiedźmin.html");
        let results = parse_search_html(&html);
        assert_eq!(results.len(), 19);

        let book = find_result(&results, "Wiedźmin");
        assert_eq!(book.author.as_deref(), Some("Andrzej Sapkowski"));
        assert!(book.cover_url.is_some());

        // Most Witcher results should have covers
        let with_covers = results.iter().filter(|r| r.cover_url.is_some()).count();
        assert_eq!(with_covers, 18);
    }

    // =========================================================================
    // Search page invariants — all fixtures
    // =========================================================================

    #[test]
    fn search_all_results_have_title_and_detail_url() {
        let search_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-html");

        for entry in std::fs::read_dir(&search_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_none_or(|e| e != "html") {
                continue;
            }
            let html = std::fs::read_to_string(entry.path()).unwrap();
            let results = parse_search_html(&html);

            assert!(
                !results.is_empty(),
                "No results from {}",
                entry.file_name().to_string_lossy()
            );

            for (i, r) in results.iter().enumerate() {
                assert!(
                    !r.title.is_empty(),
                    "Empty title in result {} of {}",
                    i,
                    entry.file_name().to_string_lossy()
                );
                assert!(
                    r.detail_url.starts_with("/book/show/"),
                    "detail_url '{}' doesn't start with /book/show/ in result {} of {}",
                    r.detail_url,
                    i,
                    entry.file_name().to_string_lossy()
                );
            }
        }
    }

    #[test]
    fn search_cover_urls_exclude_placeholders() {
        let search_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-html");

        for entry in std::fs::read_dir(&search_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_none_or(|e| e != "html") {
                continue;
            }
            let html = std::fs::read_to_string(entry.path()).unwrap();
            let results = parse_search_html(&html);

            for r in &results {
                if let Some(url) = &r.cover_url {
                    assert!(
                        !url.contains("nophoto"),
                        "Cover URL contains 'nophoto': {} in {}",
                        url,
                        entry.file_name().to_string_lossy()
                    );
                    assert!(
                        !url.contains("loading-trans"),
                        "Cover URL contains 'loading-trans': {} in {}",
                        url,
                        entry.file_name().to_string_lossy()
                    );
                }
            }
        }
    }

    // =========================================================================
    // Detail page tests — German (de)
    // =========================================================================

    #[test]
    fn detail_de_31_das_parfum() {
        let html = fixture_detail("de_31_Das_Parfum__Die_Geschichte_eines_M_rders.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert_eq!(
            result.title.as_deref(),
            Some("Das Parfum: Die Geschichte eines M\u{00f6}rders")
        );
        assert_eq!(result.author.as_deref(), Some("Patrick S\u{00fc}skind"));
        assert_eq!(result.isbn.as_deref(), Some("9783257228007"));
        assert!((result.rating.unwrap() - 4.04).abs() < 0.01);
        assert_eq!(result.page_count, Some(320));
        assert_eq!(result.language.as_deref(), Some("German"));
        assert!(result.cover_url.is_some());
        assert!(result.description.is_some());
        assert!(result.description.as_ref().unwrap().contains("Grenouille"));
        assert!(result.genres.contains(&"classics".to_string()));
        assert_eq!(result.publish_date.as_deref(), Some("February 26, 1985"));
    }

    #[test]
    fn detail_de_39_die_krone() {
        let html = fixture_detail("de_39_Die_Krone_der_Sterne.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result
            .title
            .as_ref()
            .unwrap()
            .contains("Die Krone der Sterne"));
        assert_eq!(result.author.as_deref(), Some("Kai Meyer"));
        assert_eq!(result.isbn.as_deref(), Some("9783596035854"));
        assert!((result.rating.unwrap() - 3.92).abs() < 0.01);
        assert_eq!(result.page_count, Some(448));
        assert_eq!(result.language.as_deref(), Some("German"));
    }

    #[test]
    fn detail_de_49_qualityland() {
        let html = fixture_detail("de_49_QualityLand__QualityLand___1_.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("QualityLand"));
        assert_eq!(result.author.as_deref(), Some("Marc-Uwe Kling"));
        assert_eq!(result.isbn.as_deref(), Some("9783550050237"));
        assert!((result.rating.unwrap() - 4.11).abs() < 0.01);
        assert!(result.genres.contains(&"science-fiction".to_string()));
    }

    #[test]
    fn detail_de_50_qualityland_2() {
        let html = fixture_detail("de_50_QualityLand_2_0__QualityLand___2_.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("QualityLand 2.0"));
        assert_eq!(result.author.as_deref(), Some("Marc-Uwe Kling"));
        // No ISBN for this edition
        assert!(result.isbn.is_none());
        assert!((result.rating.unwrap() - 4.19).abs() < 0.01);
    }

    #[test]
    fn detail_de_51_tintenherz() {
        let html = fixture_detail("de_51_Tintenherz__Tintenwelt___1_.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("Tintenherz"));
        assert_eq!(result.author.as_deref(), Some("Cornelia Funke"));
        assert_eq!(result.isbn.as_deref(), Some("9783791504650"));
        assert!((result.rating.unwrap() - 3.93).abs() < 0.01);
    }

    // =========================================================================
    // Detail page tests — Spanish (es)
    // =========================================================================

    #[test]
    fn detail_es_29_el_ojo() {
        let html = fixture_detail("es_29_El_ojo_del_mundo__La_rueda_del_tiempo___.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("El ojo del mundo"));
        assert_eq!(result.author.as_deref(), Some("Robert Jordan"));
        assert_eq!(result.isbn.as_deref(), Some("9788448031183"));
        assert!((result.rating.unwrap() - 4.19).abs() < 0.01);
        assert!(result.description.is_some());
    }

    #[test]
    fn detail_es_45_tres_cuerpos() {
        let html = fixture_detail("es_45_El_problema_de_los_tres_cuerpos.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result
            .title
            .as_ref()
            .unwrap()
            .contains("El problema de los tres cuerpos"));
        assert_eq!(result.author.as_deref(), Some("Liu Cixin"));
        assert_eq!(result.isbn.as_deref(), Some("9788466659734"));
        assert!((result.rating.unwrap() - 4.08).abs() < 0.01);
    }

    #[test]
    fn detail_es_46_lagrimas() {
        let html = fixture_detail("es_46_L_grimas_en_la_lluvia.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result
            .title
            .as_ref()
            .unwrap()
            .contains("L\u{00e1}grimas en la lluvia"));
        assert_eq!(result.author.as_deref(), Some("Rosa Montero"));
        assert_eq!(result.isbn.as_deref(), Some("9788432296987"));
        assert!((result.rating.unwrap() - 3.77).abs() < 0.01);
    }

    #[test]
    fn detail_es_47_sin_noticias() {
        let html = fixture_detail("es_47_Sin_noticias_de_Gurb.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result
            .title
            .as_ref()
            .unwrap()
            .contains("Sin noticias de Gurb"));
        assert_eq!(result.author.as_deref(), Some("Eduardo Mendoza"));
        assert_eq!(result.isbn.as_deref(), Some("9788432207822"));
        assert!((result.rating.unwrap() - 3.76).abs() < 0.01);
    }

    #[test]
    fn detail_es_48_klara() {
        let html = fixture_detail("es_48_Klara_y_el_Sol.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("Klara y el Sol"));
        assert_eq!(result.author.as_deref(), Some("Kazuo Ishiguro"));
        assert_eq!(result.isbn.as_deref(), Some("9788433980878"));
        assert!((result.rating.unwrap() - 3.74).abs() < 0.01);
    }

    // =========================================================================
    // Detail page tests — French (fr)
    // =========================================================================

    #[test]
    fn detail_fr_30_la_nuit() {
        let html = fixture_detail("fr_30_La_Nuit_des_temps.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("La Nuit des temps"));
        assert_eq!(result.author.as_deref(), Some("Ren\u{00e9} Barjavel"));
        assert!(result.isbn.is_none()); // No ISBN for this edition
        assert!((result.rating.unwrap() - 4.02).abs() < 0.01);
        assert!(result.description.is_some());
    }

    #[test]
    fn detail_fr_34_le_petit_prince() {
        let html = fixture_detail("fr_34_Le_Petit_Prince.html");
        let result = parse_detail_html(&html).expect("should parse");

        // The detail page fixture might be for a different edition
        assert!(result.title.is_some());
        assert_eq!(
            result.author.as_deref(),
            Some("Antoine de Saint-Exup\u{00e9}ry")
        );
        assert!((result.rating.unwrap() - 4.58).abs() < 0.01);
    }

    #[test]
    fn detail_fr_36_letranger() {
        let html = fixture_detail("fr_36_L__tranger.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.is_some());
        assert_eq!(result.author.as_deref(), Some("Albert Camus"));
        assert!(result.isbn.is_none()); // No ISBN for this edition
        assert!((result.rating.unwrap() - 4.03).abs() < 0.01);
    }

    #[test]
    fn detail_fr_37_horde() {
        let html = fixture_detail("fr_37_La_Horde_du_Contrevent.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result
            .title
            .as_ref()
            .unwrap()
            .contains("La Horde du Contrevent"));
        assert_eq!(result.author.as_deref(), Some("Alain Damasio"));
        assert!(result.isbn.is_none());
        assert!((result.rating.unwrap() - 4.42).abs() < 0.01);
        assert_eq!(result.page_count, Some(736));
    }

    #[test]
    fn detail_fr_38_les_furtifs() {
        let html = fixture_detail("fr_38_Les_Furtifs.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("Les Furtifs"));
        assert_eq!(result.author.as_deref(), Some("Alain Damasio"));
        assert_eq!(result.isbn.as_deref(), Some("9782370490742"));
        assert!((result.rating.unwrap() - 3.96).abs() < 0.01);
    }

    // =========================================================================
    // Detail page tests — Polish (pl)
    // =========================================================================

    #[test]
    fn detail_pl_32_solaris() {
        let html = fixture_detail("pl_32_Solaris.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("Solaris"));
        assert_eq!(result.author.as_deref(), Some("Stanis\u{0142}aw Lem"));
        assert!(result.isbn.is_none());
        assert!((result.rating.unwrap() - 3.98).abs() < 0.01);
        assert!(result.description.is_some());
    }

    #[test]
    fn detail_pl_35_pan_tadeusz() {
        let html = fixture_detail("pl_35_Pan_Tadeusz.html");
        let result = parse_detail_html(&html);

        // Pan Tadeusz has minimal data — may or may not parse
        // If it parses, verify what we can
        if let Some(r) = result {
            if let Some(title) = &r.title {
                assert!(title.contains("Pan Tadeusz"));
            }
            if let Some(author) = &r.author {
                assert!(author.contains("Mickiewicz"));
            }
            // No rating for this edition
            // No description available
        }
    }

    #[test]
    fn detail_pl_40_lod() {
        let html = fixture_detail("pl_40_L_d.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("L\u{00f3}d"));
        assert_eq!(result.author.as_deref(), Some("Jacek Dukaj"));
        assert_eq!(result.isbn.as_deref(), Some("9788308039854"));
        assert!((result.rating.unwrap() - 4.16).abs() < 0.01);
        assert_eq!(result.page_count, Some(1051));
    }

    #[test]
    fn detail_pl_42_perfekcyjna() {
        let html = fixture_detail("pl_42_Perfekcyjna_niedoskona_o__.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result
            .title
            .as_ref()
            .unwrap()
            .contains("Perfekcyjna niedoskona\u{0142}o\u{015b}\u{0107}"));
        assert_eq!(result.author.as_deref(), Some("Jacek Dukaj"));
        assert_eq!(result.isbn.as_deref(), Some("9788308036471"));
        assert!((result.rating.unwrap() - 4.22).abs() < 0.01);
    }

    #[test]
    fn detail_pl_44_wiedzmin() {
        let html = fixture_detail("pl_44_Wied_min.html");
        let result = parse_detail_html(&html).expect("should parse");

        assert!(result.title.as_ref().unwrap().contains("Wiedźmin"));
        assert_eq!(result.author.as_deref(), Some("Andrzej Sapkowski"));
        assert_eq!(result.isbn.as_deref(), Some("9788370541507"));
        assert!((result.rating.unwrap() - 4.68).abs() < 0.01);
    }

    // =========================================================================
    // Detail page invariants — all fixtures
    // =========================================================================

    #[test]
    fn detail_all_have_jsonld_book() {
        let detail_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-detail");

        let mut parsed = 0;
        let mut with_title = 0;
        let mut with_author = 0;
        let mut with_cover = 0;

        for entry in std::fs::read_dir(&detail_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_none_or(|e| e != "html") {
                continue;
            }
            let html = std::fs::read_to_string(entry.path()).unwrap();
            if let Some(result) = parse_detail_html(&html) {
                parsed += 1;
                if result.title.is_some() {
                    with_title += 1;
                }
                if result.author.is_some() {
                    with_author += 1;
                }
                if result.cover_url.is_some() {
                    with_cover += 1;
                }
            }
        }

        // At least 19 of 20 should parse (Pan Tadeusz might be minimal)
        assert!(parsed >= 19, "Only {parsed}/20 detail pages parsed");
        assert!(with_title >= 19, "Only {with_title}/20 have titles");
        assert!(with_author >= 19, "Only {with_author}/20 have authors");
        // Covers: 19 of 20 have covers (Pan Tadeusz: NO in the handoff table)
        assert!(with_cover >= 19, "Only {with_cover}/20 have covers");
    }

    #[test]
    fn detail_descriptions_are_plain_text() {
        let detail_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-detail");

        for entry in std::fs::read_dir(&detail_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_none_or(|e| e != "html") {
                continue;
            }
            let html = std::fs::read_to_string(entry.path()).unwrap();
            if let Some(result) = parse_detail_html(&html) {
                if let Some(desc) = &result.description {
                    // No HTML tags should remain
                    assert!(
                        !desc.contains("<br"),
                        "Description contains HTML in {}",
                        entry.file_name().to_string_lossy()
                    );
                    assert!(
                        !desc.contains("<p"),
                        "Description contains HTML in {}",
                        entry.file_name().to_string_lossy()
                    );
                    assert!(
                        !desc.contains("<span"),
                        "Description contains HTML in {}",
                        entry.file_name().to_string_lossy()
                    );
                    assert!(
                        !desc.contains("<div"),
                        "Description contains HTML in {}",
                        entry.file_name().to_string_lossy()
                    );
                    assert!(
                        !desc.contains("<a "),
                        "Description contains HTML in {}",
                        entry.file_name().to_string_lossy()
                    );
                }
            }
        }
    }

    #[test]
    fn detail_all_authors_are_arrays() {
        // Based on fixture analysis, all authors in our fixtures use array form
        let detail_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("build/tmp-goodreads-detail");

        for entry in std::fs::read_dir(&detail_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_none_or(|e| e != "html") {
                continue;
            }
            let html = std::fs::read_to_string(entry.path()).unwrap();
            let book = find_book_jsonld(&html);
            if let Some(book) = book {
                if let Some(author) = book.get("author") {
                    assert!(
                        author.is_array(),
                        "Author is not array in {}: {:?}",
                        entry.file_name().to_string_lossy(),
                        author
                    );
                }
            }
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
        assert_eq!(result.language.as_deref(), Some("English"));
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
