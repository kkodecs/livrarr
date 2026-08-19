//! Goodreads HTML/JSON parsing: regex extraction from search-results pages,
//! JSON-LD + regex from detail pages, and dedicated parsers for author
//! search, series-list, and series-detail pages.
//!
//! Replaces LLM-based scraping with direct HTML parsing for foreign language
//! works. LLM is kept as fallback only (see `super::llm_repair`).

use regex::Regex;
use std::sync::LazyLock;

use super::client::{extract_gr_key, GOODREADS_BASE_URL};

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
    /// IDs carried directly by the autocomplete response. REQ-027 consumes
    /// these without issuing a request merely to learn them again.
    pub book_id: Option<String>,
    pub work_id: Option<String>,
}

/// Detailed metadata extracted from a Goodreads book detail page.
#[derive(Debug, Clone)]
pub struct GoodreadsDetailResult {
    // JSON-LD fields (primary)
    pub title: Option<String>,
    pub author: Option<String>,
    pub isbn: Option<String>,
    pub asin: Option<String>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub page_count: Option<i32>,
    pub language: Option<String>,
    pub cover_url: Option<String>,
    pub book_format: Option<String>,
    /// Goodreads Work legacy id from the Book -> Work Apollo reference.
    pub work_id: Option<String>,
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

/// Locates the Next.js hydration payload — the primary source since GR's
/// 2026-07 React/Next redesign of the book detail page. The book/work/series/
/// contributor data lives in this script's Apollo-cache-shaped JSON.
static RE_NEXT_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<script\s+id="__NEXT_DATA__"[^>]*>(.*?)</script>"#).unwrap()
});

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
            detail_url: detail_url.clone(),
            cover_url,
            year,
            rating,
            series_name,
            series_position,
            book_id: extract_gr_key(&detail_url)
                .and_then(|key| key.split('.').next().map(str::to_string)),
            work_id: None,
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
    #[serde(default)]
    book_id: Option<StringOrNumber>,
    #[serde(default)]
    work_id: Option<StringOrNumber>,
}

/// `avgRating` arrives as a string on most entries ("4.30") but as a bare JSON
/// number (0.0) on some unrated editions — one such entry must not fail the
/// batch. Numbers render to the same two-decimal form the strings use, so the
/// downstream "0.00" = unrated filter applies uniformly.
#[derive(serde::Deserialize, Clone)]
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

    /// Numeric value regardless of wire representation — used by the GR
    /// detail-page Apollo-state path (`averageRating`), which has shown the
    /// same string-or-number inconsistency as the autocomplete `avgRating`
    /// field above.
    fn into_f64(self) -> Option<f64> {
        match self {
            StringOrNumber::S(s) => s.parse().ok(),
            StringOrNumber::N(n) => Some(n),
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
/// endpoint is the live discovery path (measured 2026-06-01). The checked
/// parser preserves an invalid/non-array top-level response as an error so the
/// provider client can classify it honestly. Entries still deserialize
/// INDIVIDUALLY: one malformed entry drops alone (logged) instead of failing
/// the whole batch.
pub fn parse_autocomplete_json_checked(
    body: &str,
) -> Result<Vec<GoodreadsSearchResult>, serde_json::Error> {
    let values: Vec<serde_json::Value> = serde_json::from_str(body)?;
    Ok(values
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
            // The search-card decoration "(Series, #N)" is provider data: the
            // same series/volume evidence the HTML road extracts. It must ride
            // the result so downstream volume vetoes can see it.
            let (series_name, series_position) = match RE_TITLE_SERIES.captures(&title) {
                Some(caps) => (
                    Some(caps[1].trim().to_string()),
                    caps[2].parse::<f64>().ok(),
                ),
                None => (None, None),
            };
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
                series_name,
                series_position,
                book_id: e.book_id.map(|value| match value {
                    StringOrNumber::S(value) => value,
                    StringOrNumber::N(value) => value.to_string(),
                }),
                work_id: e.work_id.map(|value| match value {
                    StringOrNumber::S(value) => value,
                    StringOrNumber::N(value) => value.to_string(),
                }),
            })
        })
        .collect())
}

/// Lenient compatibility wrapper for callers where Goodreads is one optional
/// contribution to a wider provider union.
pub fn parse_autocomplete_json(body: &str) -> Vec<GoodreadsSearchResult> {
    match parse_autocomplete_json_checked(body) {
        Ok(results) => results,
        Err(e) => {
            tracing::warn!(error = %e, "GR autocomplete body is not a JSON array (WAF interstitial or format change) — treating as no results");
            Vec::new()
        }
    }
}
// =============================================================================
// Detail page parsing
// =============================================================================

/// Parse a Goodreads book detail page for metadata.
///
/// Primary source (2026-07 Next.js redesign): the `__NEXT_DATA__` script's
/// Apollo-cache JSON — the book/work/series/contributor objects backing the
/// page's React hydration (see [`parse_detail_next_data`]). Fallback source
/// (pre-redesign pages, or any page GR still serves in the old shape): JSON-LD
/// `<script type="application/ld+json">` blocks + regex for description,
/// genres, series, published date (see [`parse_detail_html_legacy`]).
///
/// When NEITHER path yields anything usable this warns with enough shape
/// detail to diagnose the next layout drift, rather than silently returning
/// an empty result — an empty parse is drift, not truth (insight 62).
pub fn parse_detail_html(html: &str) -> Option<GoodreadsDetailResult> {
    if let Some(result) = parse_detail_next_data(html) {
        return Some(result);
    }
    if let Some(result) = parse_detail_html_legacy(html) {
        return Some(result);
    }
    if !html.is_empty() {
        tracing::warn!(
            has_next_data_script = RE_NEXT_DATA.is_match(html),
            has_jsonld_script = RE_JSONLD.is_match(html),
            len = html.len(),
            "GR detail page: neither the Next.js data blob nor the legacy JSON-LD/regex path yielded anything usable — treating as unreadable, not empty"
        );
    }
    None
}

// =============================================================================
// Detail page parsing — Next.js / Apollo-state primary path (2026-07 redesign)
// =============================================================================

/// One `{"__ref": "Type:key"}` pointer into the Apollo cache.
#[derive(serde::Deserialize)]
struct ApolloRef {
    #[serde(rename = "__ref")]
    r#ref: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApolloLanguage {
    #[serde(default)]
    name: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloBookDetails {
    #[serde(default)]
    isbn: Option<String>,
    #[serde(default)]
    isbn13: Option<String>,
    #[serde(default)]
    asin: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    num_pages: Option<StringOrInt>,
    #[serde(default)]
    language: Option<ApolloLanguage>,
}

#[derive(serde::Deserialize)]
struct ApolloGenre {
    #[serde(default, rename = "webUrl")]
    web_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApolloGenreEdge {
    #[serde(default)]
    genre: Option<ApolloGenre>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloBookSeriesEntry {
    #[serde(default)]
    user_position: Option<StringOrInt>,
    #[serde(default)]
    series: Option<ApolloRef>,
}

#[derive(serde::Deserialize)]
struct ApolloContributorEdge {
    #[serde(default)]
    node: Option<ApolloRef>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloBook {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    details: Option<ApolloBookDetails>,
    #[serde(default)]
    primary_contributor_edge: Option<ApolloContributorEdge>,
    #[serde(default)]
    book_series: Vec<ApolloBookSeriesEntry>,
    #[serde(default)]
    book_genres: Vec<ApolloGenreEdge>,
    #[serde(default)]
    work: Option<ApolloRef>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloWorkDetails {
    #[serde(default)]
    publication_time: Option<StringOrInt>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloWorkStats {
    #[serde(default)]
    average_rating: Option<StringOrNumber>,
    #[serde(default)]
    ratings_count: Option<StringOrInt>,
}

#[derive(serde::Deserialize)]
struct ApolloWork {
    #[serde(default, rename = "legacyId")]
    legacy_id: Option<StringOrInt>,
    #[serde(default)]
    details: Option<ApolloWorkDetails>,
    #[serde(default)]
    stats: Option<ApolloWorkStats>,
}

#[derive(serde::Deserialize)]
struct ApolloSeries {
    #[serde(default)]
    title: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApolloContributor {
    #[serde(default)]
    name: Option<String>,
}

/// Find this page's primary Book entity in the Apollo cache. Prefers the
/// explicit `getBookByLegacyId(...)` `ROOT_QUERY` pointer — it names THIS
/// page's book even if a future layout embeds other `Book:` stubs (e.g. a
/// "readers also enjoyed" carousel) — and falls back to the sole `Book:`
/// entry when that pointer is absent. Zero or multiple candidates is
/// ambiguous, never a guess.
fn find_apollo_book(
    apollo: &serde_json::Map<String, serde_json::Value>,
) -> Option<&serde_json::Value> {
    if let Some(root) = apollo.get("ROOT_QUERY").and_then(|v| v.as_object()) {
        let mut pointers = root
            .iter()
            .filter(|(k, _)| k.starts_with("getBookByLegacyId("));
        if let Some((_, first)) = pointers.next() {
            if pointers.next().is_some() {
                // Two pointers = two preloaded books and no way to know which
                // one this page is about — same abstain policy as the bare
                // multi-Book scan below. A wrong book is worse than none.
                tracing::warn!(
                    "GR detail page: multiple getBookByLegacyId pointers in ROOT_QUERY — cannot pick unambiguously"
                );
                return None;
            }
            if let Some(book) = first
                .get("__ref")
                .and_then(|v| v.as_str())
                .and_then(|book_ref| apollo.get(book_ref))
            {
                return Some(book);
            }
        }
    }

    let mut books = apollo.iter().filter(|(k, _)| k.starts_with("Book:"));
    let first = books.next()?;
    if books.next().is_some() {
        tracing::warn!(
            "GR detail page: multiple Book entities in apolloState — cannot pick unambiguously"
        );
        return None;
    }
    Some(first.1)
}

/// Genre slugs from `bookGenres` edges, deduplicated, in document order. The
/// slug is read off each genre's own `webUrl` (last path segment) rather than
/// derived from its display name, so it matches the legacy path's slug shape
/// ("non-fiction", not "Nonfiction") without guessing GR's naming rule. Each
/// entry resolves independently — one entry missing a usable URL just drops
/// from the list rather than failing the batch.
fn genres_from_apollo(edges: &[ApolloGenreEdge]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut genres = Vec::new();
    for edge in edges {
        let Some(slug) = edge
            .genre
            .as_ref()
            .and_then(|g| g.web_url.as_deref())
            .and_then(|url| url.rsplit('/').next())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if seen.insert(slug.to_string()) {
            genres.push(slug.to_string());
        }
    }
    genres
}

/// Apollo text fields (descriptions, bios) can carry raw HTML the same way
/// the legacy `Formatted` span does — strip tags and decode entities so the
/// blob path never regresses the "plain text description" contract.
fn clean_apollo_text(raw: &str) -> Option<String> {
    let stripped = RE_HTML_TAG.replace_all(raw, "");
    let decoded = decode_html_entities(&stripped);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse a Goodreads book detail page via the `__NEXT_DATA__` Apollo-cache
/// blob (2026-07 Next.js redesign). Returns `None` when the marker script is
/// absent (an older-shaped page — not drift, just the other path's job) or
/// when it's present but unreadable (drift — logged so the next layout
/// change is diagnosable). Every cross-reference (work/series/author)
/// resolves independently: one missing satellite object degrades only the
/// fields it would have supplied, never the whole parse.
/// Build the Book model field-by-field: a PRESENT field whose shape drifted
/// drops alone (warned) instead of failing the whole Book — `#[serde(default)]`
/// only covers ABSENT fields, and one malformed nested member must not erase
/// an otherwise-readable page.
fn lenient_apollo_book(obj: &serde_json::Map<String, serde_json::Value>) -> ApolloBook {
    ApolloBook {
        title: lenient_field(obj, "title"),
        description: lenient_field(obj, "description"),
        image_url: lenient_field(obj, "imageUrl"),
        details: lenient_field(obj, "details"),
        primary_contributor_edge: lenient_field(obj, "primaryContributorEdge"),
        book_series: lenient_field::<Vec<ApolloBookSeriesEntry>>(obj, "bookSeries")
            .unwrap_or_default(),
        book_genres: lenient_field::<Vec<ApolloGenreEdge>>(obj, "bookGenres").unwrap_or_default(),
        work: lenient_field(obj, "work"),
    }
}

fn lenient_field<T: serde::de::DeserializeOwned>(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<T> {
    let v = obj.get(key)?;
    if v.is_null() {
        return None;
    }
    match serde_json::from_value(v.clone()) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(
                error = %e,
                field = key,
                "GR detail page: Book field shape drifted — dropping this field only"
            );
            None
        }
    }
}

fn parse_detail_next_data(html: &str) -> Option<GoodreadsDetailResult> {
    let cap = RE_NEXT_DATA.captures(html)?;
    let blob = &cap[1];

    let root: serde_json::Value = match serde_json::from_str(blob) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "GR detail page: __NEXT_DATA__ present but not valid JSON — layout drifted"
            );
            return None;
        }
    };

    let Some(apollo) = root
        .get("props")
        .and_then(|v| v.get("pageProps"))
        .and_then(|v| v.get("apolloState"))
        .and_then(|v| v.as_object())
    else {
        tracing::warn!(
            "GR detail page: __NEXT_DATA__ present but no props.pageProps.apolloState — layout drifted"
        );
        return None;
    };

    let Some(book_value) = find_apollo_book(apollo) else {
        tracing::warn!(
            "GR detail page: apolloState present but no resolvable Book entity — layout drifted"
        );
        return None;
    };

    let Some(book_obj) = book_value.as_object() else {
        tracing::warn!("GR detail page: Book entity is not a JSON object — layout drifted");
        return None;
    };
    let book = lenient_apollo_book(book_obj);

    if book.title.is_none() && book.primary_contributor_edge.is_none() {
        tracing::warn!(
            "GR detail page: Book entity carried neither title nor author — treating as drift, not truth"
        );
        return None;
    }

    let work: Option<ApolloWork> = book
        .work
        .as_ref()
        .and_then(|r| r.r#ref.as_deref())
        .and_then(|key| apollo.get(key))
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    if book.work.is_some() && work.is_none() {
        tracing::warn!(
            "GR detail page: Book has a work reference that did not resolve — rating/rating count/publish date will be missing"
        );
    }

    let author = book
        .primary_contributor_edge
        .as_ref()
        .and_then(|edge| edge.node.as_ref())
        .and_then(|r| r.r#ref.as_deref())
        .and_then(|key| apollo.get(key))
        .and_then(|v| serde_json::from_value::<ApolloContributor>(v.clone()).ok())
        .and_then(|c| c.name);

    let (series_name, series_position) = match book.book_series.first() {
        Some(entry) => {
            let position = entry
                .user_position
                .as_ref()
                .and_then(|p| p.clone().into_string().parse::<f64>().ok());
            let name = entry
                .series
                .as_ref()
                .and_then(|r| r.r#ref.as_deref())
                .and_then(|key| apollo.get(key))
                .and_then(|v| serde_json::from_value::<ApolloSeries>(v.clone()).ok())
                .and_then(|s| s.title);
            (name, position)
        }
        None => (None, None),
    };

    let details = book.details.as_ref();
    let isbn = details
        .and_then(|d| d.isbn13.clone())
        .or_else(|| details.and_then(|d| d.isbn.clone()));
    let asin = details.and_then(|d| d.asin.clone());
    let page_count = details
        .and_then(|d| d.num_pages.as_ref())
        .and_then(|p| p.clone().into_string().parse::<i32>().ok());
    let book_format = details.and_then(|d| d.format.clone());
    let language = details
        .and_then(|d| d.language.as_ref())
        .and_then(|l| l.name.as_deref())
        .map(livrarr_domain::normalize_language);

    let genres = genres_from_apollo(&book.book_genres);
    let description = book.description.as_deref().and_then(clean_apollo_text);

    let stats = work.as_ref().and_then(|w| w.stats.as_ref());
    let rating = stats
        .and_then(|s| s.average_rating.as_ref())
        .and_then(|r| r.clone().into_f64());
    let rating_count = stats
        .and_then(|s| s.ratings_count.as_ref())
        .and_then(|c| c.clone().into_string().parse::<i32>().ok());
    let publish_date = work
        .as_ref()
        .and_then(|w| w.details.as_ref())
        .and_then(|d| d.publication_time.as_ref())
        .and_then(|t| t.clone().into_string().parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.format("%Y-%m-%d").to_string());
    let work_id = work
        .as_ref()
        .and_then(|candidate| candidate.legacy_id.clone())
        .map(StringOrInt::into_string)
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()));

    Some(GoodreadsDetailResult {
        title: book.title,
        author,
        isbn,
        asin,
        rating,
        rating_count,
        page_count,
        language,
        cover_url: book.image_url,
        book_format,
        work_id,
        description,
        genres,
        series_name,
        series_position,
        publish_date,
    })
}

// =============================================================================
// Detail page parsing — legacy JSON-LD/regex path (fallback)
// =============================================================================

/// Legacy extraction: JSON-LD `<script type="application/ld+json">` as
/// primary, regex (description/genres/series/published date) as secondary.
/// Kept as the fallback for any page GR still serves in the pre-2026-07
/// shape (older cached responses, or a future partial rollback).
fn parse_detail_html_legacy(html: &str) -> Option<GoodreadsDetailResult> {
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
        let asin = book
            .get("asin")
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

        // `find_book_jsonld` accepts any block declaring `"@type":"Book"`, so a
        // stub shell that declares the type and carries nothing else reached
        // here and became an all-`None` payload — which the caller counts as a
        // successful parse and reports to the breaker as Success, clearing
        // every accumulated failure. Declaring the type is not the same as
        // carrying a book.
        //
        // The bar is "carried at least one field we actually extracted", not a
        // named subset: a sparse page offering only a rating, a page count and
        // a language is thin but real, and rejecting it would send a readable
        // page down the unreadable path and lose the fields with it.
        let carried_nothing = title.is_none()
            && author.is_none()
            && isbn.is_none()
            && asin.is_none()
            && rating.is_none()
            && rating_count.is_none()
            && page_count.is_none()
            && language.is_none()
            && cover_url.is_none()
            && book_format.is_none()
            && description.is_none()
            && genres.is_empty()
            && series_name.is_none()
            && series_position.is_none()
            && publish_date.is_none();
        if carried_nothing {
            return None;
        }
        Some(GoodreadsDetailResult {
            title,
            author,
            isbn,
            asin,
            rating,
            rating_count,
            page_count,
            language,
            cover_url,
            book_format,
            work_id: None,
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
            asin: None,
            rating: None,
            rating_count: None,
            page_count: None,
            language: None,
            cover_url: None,
            book_format: None,
            work_id: None,
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

/// One contributor credited on a specific Goodreads book page.
///
/// `raw_id` is the provider's own author identifier exactly as the page spells
/// it; canonicalization belongs to `AuthorRouteKey::parse`, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodreadsBookContributor {
    pub raw_id: String,
    pub name: String,
    pub role: Option<String>,
}

/// A contributor edge that also carries the credited role.
#[derive(serde::Deserialize)]
struct ApolloRoleContributorEdge {
    #[serde(default)]
    node: Option<ApolloRef>,
    #[serde(default)]
    role: Option<String>,
}

/// The contributor half of the selected Book's edges.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloBookContributors {
    #[serde(default)]
    primary_contributor_edge: Option<ApolloRoleContributorEdge>,
    #[serde(default)]
    secondary_contributor_edges: Option<Vec<ApolloRoleContributorEdge>>,
}

/// A referenced Contributor entity: the credited name plus the numeric author
/// id every Goodreads author URL is built from.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApolloContributorIdentity {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    legacy_id: Option<StringOrInt>,
}

/// Which of the two book→contributor shapes a page was read through.
///
/// The caller certifies roles, and it cannot do that without knowing which
/// shape it is looking at: an Apollo edge names its own credit, while the
/// JSON-LD field this falls back to *is* the author list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoodreadsContributorSource {
    ApolloEdges,
    JsonLdAuthors,
}

/// Every contributor credited on one book, and the shape they were read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodreadsBookContributors {
    pub source: GoodreadsContributorSource,
    pub contributors: Vec<GoodreadsBookContributor>,
}

/// Every contributor credited on the selected book, or `None` when the
/// book→contributor association shape is unreadable.
///
/// `None` is layout drift, never "this book credits nobody": a Goodreads book
/// page always names at least its primary contributor, so an empty read means
/// the shape moved. Only edges hanging off the selected Book are followed —
/// reviewer and user entities in the same Apollo cache also carry `legacyId`
/// and are not contributors of this book.
pub fn parse_book_contributors(html: &str) -> Option<GoodreadsBookContributors> {
    apollo_book_contributors(html)
        .map(|contributors| GoodreadsBookContributors {
            source: GoodreadsContributorSource::ApolloEdges,
            contributors,
        })
        .or_else(|| {
            jsonld_book_contributors(html).map(|contributors| GoodreadsBookContributors {
                source: GoodreadsContributorSource::JsonLdAuthors,
                contributors,
            })
        })
}

/// The current layout: `__NEXT_DATA__` → `apolloState` → this page's Book →
/// primary/secondary contributor edges → referenced Contributor entities.
fn apollo_book_contributors(html: &str) -> Option<Vec<GoodreadsBookContributor>> {
    let cap = RE_NEXT_DATA.captures(html)?;
    let root: serde_json::Value = serde_json::from_str(&cap[1]).ok()?;
    let apollo = root
        .get("props")
        .and_then(|v| v.get("pageProps"))
        .and_then(|v| v.get("apolloState"))
        .and_then(|v| v.as_object())?;
    let book = find_apollo_book(apollo)?;
    let edges: ApolloBookContributors = serde_json::from_value(book.clone()).ok()?;

    let mut contributors = Vec::new();
    let secondary = edges.secondary_contributor_edges.unwrap_or_default();
    for edge in edges
        .primary_contributor_edge
        .iter()
        .chain(secondary.iter())
    {
        let Some(identity) = edge
            .node
            .as_ref()
            .and_then(|node| node.r#ref.as_deref())
            .and_then(|key| apollo.get(key))
            .and_then(|value| {
                serde_json::from_value::<ApolloContributorIdentity>(value.clone()).ok()
            })
        else {
            continue;
        };
        let (Some(raw_id), Some(name)) = (
            identity.legacy_id.map(StringOrInt::into_string),
            identity.name,
        ) else {
            continue;
        };
        if raw_id.trim().is_empty() || name.trim().is_empty() {
            continue;
        }
        contributors.push(GoodreadsBookContributor {
            raw_id: raw_id.trim().to_string(),
            name: name.trim().to_string(),
            role: edge
                .role
                .as_deref()
                .map(str::trim)
                .filter(|role| !role.is_empty())
                .map(str::to_string),
        });
    }

    (!contributors.is_empty()).then_some(contributors)
}

/// The legacy layout: the page's JSON-LD Book entity, whose `author` entries
/// carry the credited name and a `/author/show/<id>` link.
fn jsonld_book_contributors(html: &str) -> Option<Vec<GoodreadsBookContributor>> {
    let book = find_book_jsonld(html).or_else(|| find_jsonld_crediting_authors(html))?;
    let entries = match book.get("author")? {
        serde_json::Value::Array(items) => items.clone(),
        object @ serde_json::Value::Object(_) => vec![object.clone()],
        _ => return None,
    };

    let mut contributors = Vec::new();
    for entry in &entries {
        let Some(name) = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(raw_id) = entry
            .get("url")
            .and_then(|v| v.as_str())
            .and_then(author_show_id)
        else {
            continue;
        };
        contributors.push(GoodreadsBookContributor {
            raw_id,
            name: name.to_string(),
            role: None,
        });
    }

    (!contributors.is_empty()).then_some(contributors)
}

/// A JSON-LD entity crediting authors, for pages whose block omits the `Book`
/// type marker.
///
/// Still scoped to this page's own JSON-LD, so the credits belong to this page's
/// book — the type marker is how the entity is labelled, not what makes the
/// credits associated.
fn find_jsonld_crediting_authors(html: &str) -> Option<serde_json::Value> {
    for cap in RE_JSONLD.captures_iter(html) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&cap[1]) else {
            continue;
        };
        if value.get("author").is_some() {
            return Some(value);
        }
        let nested = value
            .as_array()
            .or_else(|| value.get("@graph").and_then(|graph| graph.as_array()));
        if let Some(item) = nested.and_then(|items| {
            items
                .iter()
                .find(|item| item.get("author").is_some())
                .cloned()
        }) {
            return Some(item);
        }
    }
    None
}

/// The numeric id in a `/author/show/<id>[.Slug]` link, ignoring anything else.
fn author_show_id(url: &str) -> Option<String> {
    let after = url.split("/author/show/").nth(1)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
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

/// A parsed GR series detail page (2026-07 React layout).
///
/// The page ships books as HTML-attribute-encoded JSON inside
/// `data-react-props` mounts. Books arrive in document order with the
/// PRIMARY works first, followed by omnibuses, split editions, and
/// translations (measured on series 108562 and 43318, 2026-07-03); the
/// header states how many of the leading entries are primary. The page
/// renders NO per-book position labels — the only position signal is the
/// title decoration "(Series Name, #N)".
#[derive(Debug, Clone, Default)]
pub struct GoodreadsSeriesPage {
    /// All parsed book entries, in document order (primaries first).
    pub books: Vec<GoodreadsSeriesBook>,
    /// Whether GR reports another page (total-works arithmetic, not primary).
    pub has_next: bool,
    /// "N primary works" from the page header — the roster cutoff.
    /// `None` means the header was missing or unreadable (layout drift).
    pub primary_count: Option<usize>,
}

/// Matches "N primary works" in the series header subtitle.
static RE_PRIMARY_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\d+)\s+primary work"#).unwrap());

/// One React mount div on the series page; captures the component name. The
/// props attribute is extracted from the full tag separately — attribute
/// order inside the tag is not guaranteed.
static RE_REACT_MOUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)<div\s[^>]*data-react-class="ReactComponents\.([A-Za-z0-9_]+)"[^>]*>"#)
        .unwrap()
});

static RE_REACT_PROPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-react-props="([^"]*)""#).unwrap());

/// `bookId` arrives as a JSON string; tolerate a bare number (the autocomplete
/// avgRating lesson — one differently-typed field must not drop an entry).
/// Also reused by the detail-page Apollo-state path for `userPosition`,
/// `numPages`, `ratingsCount`, and `publicationTime` — the same defensive
/// posture against a GR wire-shape tweak.
#[derive(serde::Deserialize, Clone)]
#[serde(untagged)]
enum StringOrInt {
    S(String),
    N(i64),
}

impl StringOrInt {
    fn into_string(self) -> String {
        match self {
            StringOrInt::S(s) => s,
            StringOrInt::N(n) => n.to_string(),
        }
    }
}

#[derive(serde::Deserialize)]
struct SeriesPageEntry {
    book: SeriesPageBook,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesPageBook {
    #[serde(default)]
    book_id: Option<StringOrInt>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    book_title_bare: Option<String>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    publication_date: Option<StringOrInt>,
}

#[derive(serde::Deserialize)]
struct SeriesPageHeader {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    subtitle: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesPagePagination {
    #[serde(default)]
    num_works: Option<i64>,
    #[serde(default)]
    current_page_number: Option<i64>,
    #[serde(default)]
    per_page: Option<i64>,
}

/// Normalize a series name for the position rule: a decorated position
/// counts only when the decoration names THIS page's series (an umbrella
/// page like Confederation Universe lists books decorated with their
/// sub-series' numbers — those must never be borrowed).
fn normalize_series_name(name: &str) -> String {
    let lower = name.trim().to_lowercase().replace('\u{2019}', "'");
    let no_suffix = lower.strip_suffix(" series").unwrap_or(&lower).trim_end();
    let no_prefix = no_suffix.strip_prefix("the ").unwrap_or(no_suffix);
    no_prefix.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First 4-digit token of a date string ("1996", "January 1, 1996", …).
fn year_from_date_str(s: &str) -> Option<i32> {
    s.split(|c: char| !c.is_ascii_digit())
        .find(|tok| tok.len() == 4)
        .and_then(|tok| tok.parse().ok())
}

/// Decode common HTML entities in a string.
use livrarr_domain::decode_xml_entities as decode_html_entities;

/// Returns true if a title looks like an omnibus/collection rather than a single work.
pub fn is_collection_title(title: &str) -> bool {
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

/// Parse a Goodreads series detail page (2026-07 React layout).
///
/// Books live as HTML-attribute-encoded JSON inside `data-react-props`
/// mounts (the pre-2026-07 `<h3>Book N</h3>` layout is gone). Entries parse
/// INDIVIDUALLY so one rogue entry never erases the page, and every
/// unreadable shape WARNS — a silent empty is how the last layout drift went
/// unnoticed. The header mount precedes the list mounts in document order on
/// every measured page; if GR ever reorders them, positions degrade to None
/// while the books themselves still parse.
pub fn parse_series_detail_html(html: &str) -> GoodreadsSeriesPage {
    let mut page = GoodreadsSeriesPage::default();
    let mut page_series_name: Option<String> = None;
    let mut series_list_blobs = 0usize;
    let mut mounts = 0usize;

    for mount in RE_REACT_MOUNT.captures_iter(html) {
        mounts += 1;
        let class = mount[1].to_string();
        if !matches!(
            class.as_str(),
            "SeriesHeader" | "SeriesList" | "FullPagePaginationControls"
        ) {
            continue;
        }
        let tag = mount.get(0).unwrap().as_str();
        let Some(props) = RE_REACT_PROPS.captures(tag) else {
            tracing::warn!(component = %class, "GR series page: mount has no props attribute");
            continue;
        };
        let decoded = decode_html_entities(&props[1]);
        let value: serde_json::Value = match serde_json::from_str(&decoded) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(component = %class, error = %e,
                    "GR series page: props blob is not valid JSON — skipping this blob");
                continue;
            }
        };
        match class.as_str() {
            "SeriesHeader" => {
                if let Ok(header) = serde_json::from_value::<SeriesPageHeader>(value) {
                    page_series_name = header.title.as_deref().map(normalize_series_name);
                    page.primary_count = header
                        .subtitle
                        .as_deref()
                        .and_then(|s| RE_PRIMARY_COUNT.captures(s))
                        .and_then(|c| c[1].parse::<usize>().ok());
                    if page.primary_count.is_none() {
                        tracing::warn!(
                            subtitle = header.subtitle.as_deref().unwrap_or(""),
                            "GR series page: header has no 'N primary works' count — layout drifted"
                        );
                    }
                }
            }
            "SeriesList" => {
                series_list_blobs += 1;
                let Some(entries) = value.get("series").and_then(|s| s.as_array()) else {
                    tracing::warn!(
                        "GR series page: SeriesList blob has no `series` array — layout drifted"
                    );
                    continue;
                };
                for entry in entries {
                    match serde_json::from_value::<SeriesPageEntry>(entry.clone()) {
                        Ok(e) => {
                            if let Some(book) =
                                series_book_from_blob(e.book, page_series_name.as_deref())
                            {
                                page.books.push(book);
                            }
                        }
                        Err(e) => tracing::warn!(error = %e,
                            "GR series page: entry failed to parse — dropping this entry only"),
                    }
                }
            }
            "FullPagePaginationControls" => {
                if let Ok(p) = serde_json::from_value::<SeriesPagePagination>(value) {
                    if let (Some(total), Some(current), Some(per)) =
                        (p.num_works, p.current_page_number, p.per_page)
                    {
                        page.has_next = current.saturating_mul(per) < total;
                    }
                }
            }
            _ => unreachable!("class list is filtered above"),
        }
    }

    // Dedupe by GR key, preserving document order.
    let mut seen = std::collections::HashSet::new();
    page.books.retain(|b| seen.insert(b.gr_key.clone()));

    if mounts == 0 {
        tracing::warn!(
            "GR series page: no React mounts found — page unreadable or layout replaced again"
        );
    } else if series_list_blobs == 0 {
        tracing::warn!("GR series page: mounts present but no SeriesList blob — layout drifted");
    } else if page.books.is_empty() {
        tracing::warn!(
            "GR series page: SeriesList present but zero books parsed — treat as drift, not truth"
        );
    }

    page
}

/// Build one roster book from a blob entry; `page_series` gates the position
/// rule (decoration must name this page's series).
fn series_book_from_blob(
    book: SeriesPageBook,
    page_series: Option<&str>,
) -> Option<GoodreadsSeriesBook> {
    let gr_key = book
        .book_id
        .map(StringOrInt::into_string)
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_default();

    let decorated = book.title.unwrap_or_default();
    let position = RE_TITLE_SERIES.captures(&decorated).and_then(|caps| {
        let decoration_series = normalize_series_name(caps[1].trim());
        match page_series {
            Some(p) if p == decoration_series => caps[2].parse::<f64>().ok(),
            _ => None,
        }
    });

    let title = match book.book_title_bare.filter(|t| !t.trim().is_empty()) {
        Some(bare) => bare.trim().to_string(),
        None => RE_TITLE_SERIES.replace(&decorated, "").trim().to_string(),
    };
    if title.is_empty() {
        tracing::warn!(gr_key = %gr_key, "GR series page: entry has no usable title — dropping");
        return None;
    }

    let year = book
        .publication_date
        .map(StringOrInt::into_string)
        .as_deref()
        .and_then(year_from_date_str);

    let cover_url = book
        .image_url
        .filter(|u| !u.contains("nophoto") && !u.contains("loading-trans"))
        .and_then(|u| crate::provider_util::validate_cover_url(&u, GOODREADS_BASE_URL))
        .map(|u| crate::provider_util::upscale_cover_url(&u));

    Some(GoodreadsSeriesBook {
        title,
        gr_key,
        position,
        year,
        cover_url,
    })
}

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
    fn autocomplete_checked_parser_rejects_bad_top_levels_but_isolates_entries() {
        for body in ["<html>blocked</html>", r#"{"hits":[]}"#, "null"] {
            assert!(
                parse_autocomplete_json_checked(body).is_err(),
                "{body:?} is not a valid autocomplete array"
            );
        }

        let empty = parse_autocomplete_json_checked("[]")
            .expect("an explicit empty array is a healthy miss");
        assert!(empty.is_empty());

        let mixed = r#"[
            {"bookUrl":{"nested":"garbage"},"title":"Broken Entry"},
            {"bookUrl":"/book/show/5907","title":"Survivor","author":{"name":"J.R.R. Tolkien"}}
        ]"#;
        let results = parse_autocomplete_json_checked(mixed)
            .expect("bad entries must not poison a valid top-level array");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Survivor");
    }

    #[test]
    fn autocomplete_decoration_populates_series_fields() {
        // The search-card decoration is provider data — the volume evidence
        // must ride the result fields (the HTML road already extracts it).
        let body = r#"[
            {"bookId":"47212","bookUrl":"/book/show/47212","title":"Storm Front (The Dresden Files, #1)","bookTitleBare":"Storm Front","author":{"name":"Jim Butcher"}},
            {"bookId":"11084145","bookUrl":"/book/show/11084145","title":"Steve Jobs","bookTitleBare":"Steve Jobs","author":{"name":"Walter Isaacson"}}
        ]"#;
        let results = parse_autocomplete_json(body);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].series_name.as_deref(), Some("The Dresden Files"));
        assert_eq!(results[0].series_position, Some(1.0));
        assert_eq!(results[1].series_name, None);
        assert_eq!(results[1].series_position, None);
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
    // Detail page parsing — Next.js / Apollo-state primary path (2026-07-16)
    // =========================================================================

    /// Real page captured 2026-07-16 from production: Sapiens: A Brief
    /// History of Humankind by Yuval Noah Harari, GR book id 23692271. GR's
    /// React/Next redesign moved book data into a `__NEXT_DATA__` Apollo-state
    /// blob; this is the ground truth for that new shape. All assertions below
    /// were read directly out of the fixture's embedded JSON.
    const BOOK_PAGE_SAPIENS: &str = include_str!("../../fixtures/gr-book-23692271.html");

    #[test]
    fn detail_next_data_sapiens_fixture_extracts_full_metadata() {
        // This is a regression pin for the live bug: `parse_detail_html`
        // undercounted genres on this exact fixture before the Apollo-state
        // path existed (legacy HTML-anchor scraping found only 7 of the 10
        // genres the page's own data carries — GR renders the rest without a
        // clickable link). That undercount made this assertion FAIL against
        // the pre-fix code; it now passes because the blob is read directly.
        let result = parse_detail_html(BOOK_PAGE_SAPIENS).expect("must parse");

        assert_eq!(
            result.title.as_deref(),
            Some("Sapiens: A Brief History of Humankind")
        );
        assert_eq!(result.author.as_deref(), Some("Yuval Noah Harari"));
        // This edition's Apollo `BookDetails` carries `isbn: null` AND
        // `isbn13: null` — genuinely absent from the blob (verified in the
        // fixture JSON), not a parse failure. Legacy JSON-LD agreed (also
        // None) — no regression here, just nothing to report.
        assert_eq!(result.isbn, None, "this edition's blob has no ISBN at all");
        assert!(
            (result.rating.expect("rating") - 4.33).abs() < 0.001,
            "rating from Work.stats.averageRating"
        );
        assert_eq!(
            result.rating_count,
            Some(1_310_797),
            "rating count from Work.stats.ratingsCount"
        );
        assert_eq!(
            result.page_count,
            Some(512),
            "page count from Book.details.numPages"
        );
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(
            result.cover_url.as_deref(),
            Some("https://m.media-amazon.com/images/S/compressed.photo.goodreads.com/books/1703329310i/23692271.jpg")
        );
        assert_eq!(result.book_format.as_deref(), Some("Paperback"));
        let desc = result.description.expect("description");
        assert!(desc.starts_with("From a renowned historian comes a groundbreaking narrative"));
        assert!(desc.ends_with("Robert Wright, and Sharon Moalem."));
        assert_eq!(
            desc.chars().count(),
            1565,
            "Book.description length, unaltered — no tags/entities to strip in this fixture"
        );
        // All 10 genres from `bookGenres` — the legacy path only ever found 7
        // (only entries GR renders as clickable anchor tags; sociology,
        // historical, and evolution are listed in the data but never link out).
        assert_eq!(
            result.genres,
            vec![
                "non-fiction",
                "history",
                "science",
                "audiobook",
                "philosophy",
                "anthropology",
                "psychology",
                "sociology",
                "historical",
                "evolution",
            ]
        );
        assert_eq!(result.series_name.as_deref(), Some("Homo"));
        assert_eq!(result.series_position, Some(1.0));
        // Work.details.publicationTime = 1293868800000ms = 2011-01-01 UTC —
        // the WORK's original/first-published date (matches legacy's "First
        // published" semantics), not this specific paperback printing's own
        // 2015 publicationTime.
        assert_eq!(result.publish_date.as_deref(), Some("2011-01-01"));
    }

    #[test]
    fn detail_next_data_neither_shape_present_is_none() {
        // A page with NEITHER the new __NEXT_DATA__ blob NOR any legacy
        // JSON-LD/Formatted/genre-link markers. Layout drift, not truth —
        // must come back empty, never a fabricated partial result. (This
        // also exercises the final "both paths failed" warn in
        // `parse_detail_html`.)
        let html = "<html><body><p>Some unrelated page content with no recognizable markers.</p></body></html>";
        assert!(parse_detail_html(html).is_none());
    }

    #[test]
    fn detail_next_data_malformed_json_falls_through_to_none() {
        // The __NEXT_DATA__ script exists (so the primary path is attempted)
        // but its body isn't valid JSON, and there are no legacy markers
        // either. Exercises `parse_detail_next_data`'s "not valid JSON" warn
        // branch; must degrade to None, not panic.
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{this is not valid json}</script></body></html>"#;
        assert!(parse_detail_html(html).is_none());
    }

    #[test]
    fn detail_next_data_no_book_entity_falls_through_to_none() {
        // Valid __NEXT_DATA__ JSON with a real apolloState shape, but zero
        // `Book:`-prefixed entries and no `getBookByLegacyId` pointer —
        // simulates GR shipping a page (or a non-book page) where the Apollo
        // cache never got a Book entity. Exercises `find_apollo_book`'s "no
        // resolvable Book entity" warn branch; must degrade to None (no
        // legacy markers present to fall back on).
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"apolloState":{"ROOT_QUERY":{"__typename":"Query"}}}}}</script></body></html>"#;
        assert!(parse_detail_html(html).is_none());
    }

    #[test]
    fn detail_next_data_multiple_legacyid_pointers_abstains() {
        // Two getBookByLegacyId pointers = two preloaded books; iteration
        // order must never pick one. Abstain (and here, with no legacy
        // markers, the whole parse comes back None) — a wrong book is worse
        // than none.
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"apolloState":{"ROOT_QUERY":{"getBookByLegacyId({\"legacyId\":\"1\"})":{"__ref":"Book:kca://book/1"},"getBookByLegacyId({\"legacyId\":\"2\"})":{"__ref":"Book:kca://book/2"}},"Book:kca://book/1":{"title":"Book One"},"Book:kca://book/2":{"title":"Book Two"}}}}}</script></body></html>"#;
        assert!(parse_detail_html(html).is_none());
    }

    #[test]
    fn detail_next_data_malformed_present_field_drops_alone() {
        // `details` is PRESENT but the wrong shape (an array). The field must
        // drop alone — title and the rest of the Book still parse. Pins the
        // per-field leniency of `lenient_apollo_book` (serde defaults only
        // cover ABSENT fields).
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"apolloState":{"ROOT_QUERY":{"getBookByLegacyId({\"legacyId\":\"9\"})":{"__ref":"Book:kca://book/9"}},"Book:kca://book/9":{"title":"Resilient Title","details":[1,2,3]}}}}}</script></body></html>"#;
        let result = parse_detail_html(html).expect("title must survive a malformed details field");
        assert_eq!(result.title.as_deref(), Some("Resilient Title"));
        assert!(result.isbn.is_none());
    }

    #[test]
    fn detail_next_data_extracts_book_to_work_legacy_id_without_promoting_book_id() {
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"apolloState":{"ROOT_QUERY":{"getBookByLegacyId({\"legacyId\":\"77\"})":{"__ref":"Book:kca://book/77"}},"Book:kca://book/77":{"title":"Bridge Book","work":{"__ref":"Work:kca://work/900"}},"Work:kca://work/900":{"legacyId":"900"}}}}}</script></body></html>"#;
        let result = parse_detail_html(html).expect("Book page with a resolved Work entity");
        assert_eq!(result.work_id.as_deref(), Some("900"));
        assert_ne!(result.work_id.as_deref(), Some("77"));
    }

    // =========================================================================
    // Series detail page parsing (2026-07 React layout)
    // =========================================================================

    /// Real page captured 2026-07-03: umbrella series, 5 primary == 5 total,
    /// decorations name the SUB-series (Night's Dawn).
    const SERIES_PAGE_CONFEDERATION: &str = include_str!("../../fixtures/gr-series-108562.html");

    /// Real page captured 2026-07-03: 3 primary works listed FIRST, followed
    /// by 24 omnibus/split-edition/translation entries (27 total).
    const SERIES_PAGE_NIGHTS_DAWN: &str = include_str!("../../fixtures/gr-series-43318.html");

    fn attr_encode(json: &str) -> String {
        json.replace('&', "&amp;").replace('"', "&quot;")
    }

    /// Build a minimal new-layout series page from raw JSON pieces.
    fn series_page(
        header_title: &str,
        subtitle: &str,
        series_json: &[&str],
        pagination_json: Option<&str>,
    ) -> String {
        let mut html = String::from("<html><body>");
        let header = format!(
            r#"{{"title":"{header_title}","subtitle":"{subtitle}","description":{{"html":""}}}}"#
        );
        html.push_str(&format!(
            r#"<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{}"></div>"#,
            attr_encode(&header)
        ));
        for s in series_json {
            html.push_str(&format!(
                r#"<div data-react-class="ReactComponents.SeriesList" data-react-props="{}"></div>"#,
                attr_encode(s)
            ));
        }
        if let Some(p) = pagination_json {
            html.push_str(&format!(
                r#"<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{}"></div>"#,
                attr_encode(p)
            ));
        }
        html.push_str("</body></html>");
        html
    }

    fn series_entry(book_json: &str) -> String {
        format!(r#"{{"isLibrarianView":false,"readOnlyStars":false,"book":{book_json}}}"#)
    }

    #[test]
    fn series_detail_cu_fixture_parses_all_five_books_unnumbered() {
        let page = parse_series_detail_html(SERIES_PAGE_CONFEDERATION);
        assert_eq!(page.primary_count, Some(5));
        assert!(!page.has_next);
        let keys: Vec<&str> = page.books.iter().map(|b| b.gr_key.as_str()).collect();
        assert_eq!(keys, ["45245", "479561", "45260", "45257", "126413"]);
        let titles: Vec<&str> = page.books.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "The Reality Dysfunction",
                "The Neutronium Alchemist",
                "The Naked God",
                "A Second Chance at Eden",
                "The Confederation Handbook"
            ]
        );
        assert_eq!(
            page.books.iter().map(|b| b.year).collect::<Vec<_>>(),
            [Some(1996), Some(1997), Some(1999), Some(1998), Some(2000)]
        );
        // Umbrella page: every decoration names Night's Dawn, not this series —
        // positions are never borrowed from a different series.
        assert!(
            page.books.iter().all(|b| b.position.is_none()),
            "umbrella page must not borrow sub-series numbers"
        );
        let cover = page.books[0].cover_url.as_deref().expect("cover url");
        assert!(cover.starts_with("https://"), "validated host: {cover}");
        assert!(!cover.contains("_SY180_"), "size token upscaled: {cover}");
    }

    #[test]
    fn series_detail_nd_fixture_lists_primaries_first_with_matching_positions() {
        let page = parse_series_detail_html(SERIES_PAGE_NIGHTS_DAWN);
        assert_eq!(page.primary_count, Some(3));
        assert!(!page.has_next, "27 total works fit one 100-per-page page");
        assert_eq!(
            page.books.len(),
            27,
            "the parser reports the page faithfully — the primary cutoff is roster policy"
        );
        let first3: Vec<(&str, Option<f64>)> = page.books[..3]
            .iter()
            .map(|b| (b.gr_key.as_str(), b.position))
            .collect();
        assert_eq!(
            first3,
            [
                ("45245", Some(1.0)),
                ("479561", Some(2.0)),
                ("45260", Some(3.0))
            ]
        );
        // A translation decorates a DIFFERENT series name ("Zorii Nopții") —
        // its number never becomes a position.
        let ro = page
            .books
            .iter()
            .find(|b| b.gr_key == "16111029")
            .expect("Romanian split edition is on the page");
        assert_eq!(ro.position, None);
        // The omnibus decorates a RANGE (#1-3) — not a position.
        let omnibus = page
            .books
            .iter()
            .find(|b| b.gr_key == "5198367")
            .expect("trilogy omnibus is on the page");
        assert_eq!(omnibus.position, None);
    }

    #[test]
    fn series_detail_poison_entry_drops_alone() {
        let good = series_entry(
            r#"{"bookId":"42","title":"Survivor (Saga, #1)","bookTitleBare":"Survivor","imageUrl":"https://i.gr-assets.com/images/S/x/42._SY180_.jpg","publicationDate":"2001"}"#,
        );
        let list = format!(
            r#"{{"series":[{{"isLibrarianView":false,"readOnlyStars":false,"book":"not an object"}},{good}]}}"#
        );
        let html = series_page(
            "Saga Series",
            "2 primary works • 2 total works",
            &[&list],
            None,
        );
        let page = parse_series_detail_html(&html);
        assert_eq!(page.books.len(), 1, "the rogue entry drops alone");
        assert_eq!(page.books[0].gr_key, "42");
        assert_eq!(
            page.books[0].position,
            Some(1.0),
            "decoration names this series — position taken"
        );
        assert_eq!(page.books[0].year, Some(2001));
    }

    #[test]
    fn series_detail_pagination_from_counter_blob() {
        let list = format!(
            r#"{{"series":[{}]}}"#,
            series_entry(r#"{"bookId":"1","title":"B1","bookTitleBare":"B1"}"#)
        );
        let p1 = series_page(
            "Big Series",
            "250 primary works • 250 total works",
            &[&list],
            Some(r#"{"numWorks":250,"currentPageNumber":1,"perPage":100}"#),
        );
        assert!(parse_series_detail_html(&p1).has_next);
        let p3 = series_page(
            "Big Series",
            "250 primary works • 250 total works",
            &[&list],
            Some(r#"{"numWorks":250,"currentPageNumber":3,"perPage":100}"#),
        );
        assert!(!parse_series_detail_html(&p3).has_next);
    }

    #[test]
    fn series_detail_old_layout_is_unreadable_and_empty() {
        // The pre-2026-07 layout (<h3>Book N</h3> headings) no longer exists on
        // GR; a page in that shape is DRIFT and must parse as unreadable —
        // loudly empty — never as data.
        let html = r#"<h3>Book 1</h3><a href="/book/show/47212">Storm Front</a>
                      <h3>Book 2</h3><a href="/book/show/47213">Fool Moon</a>"#;
        let page = parse_series_detail_html(html);
        assert!(page.books.is_empty());
        assert_eq!(page.primary_count, None);
        assert!(!page.has_next);
    }

    #[test]
    fn series_detail_missing_primary_count_reports_none() {
        let list = format!(
            r#"{{"series":[{}]}}"#,
            series_entry(r#"{"bookId":"7","title":"X","bookTitleBare":"X"}"#)
        );
        let html = series_page("Odd Series", "some redesigned subtitle", &[&list], None);
        let page = parse_series_detail_html(&html);
        assert_eq!(
            page.primary_count, None,
            "an unparseable header is signalled, never guessed"
        );
        assert_eq!(page.books.len(), 1);
    }

    #[test]
    fn series_detail_nophoto_cover_dropped_book_kept() {
        let list = format!(
            r#"{{"series":[{}]}}"#,
            series_entry(
                r#"{"bookId":"9","title":"Y","bookTitleBare":"Y","imageUrl":"https://s.gr-assets.com/assets/nophoto/book/111x148.png"}"#
            )
        );
        let html = series_page("Y Series", "1 primary work • 1 total work", &[&list], None);
        let page = parse_series_detail_html(&html);
        assert_eq!(page.books.len(), 1);
        assert!(page.books[0].cover_url.is_none());
    }

    #[test]
    fn series_detail_title_prefers_bare_form_and_strips_decoration_fallback() {
        let list = format!(
            r#"{{"series":[{},{}]}}"#,
            series_entry(r#"{"bookId":"11","title":"Alpha (Greek, #1)","bookTitleBare":"Alpha"}"#),
            series_entry(r#"{"bookId":"12","title":"Beta (Greek, #2)"}"#)
        );
        let html = series_page(
            "Greek Series",
            "2 primary works • 2 total works",
            &[&list],
            None,
        );
        let page = parse_series_detail_html(&html);
        assert_eq!(page.books[0].title, "Alpha");
        assert_eq!(
            page.books[1].title, "Beta",
            "decoration stripped when GR omits the bare form"
        );
        assert_eq!(page.books[1].position, Some(2.0));
    }

    #[test]
    fn series_detail_list_before_header_keeps_books_without_positions() {
        // GR has reordered page components before; the degrade contract is
        // books retained, positions never guessed from a header not yet seen.
        let list = format!(
            r#"{{"series":[{}]}}"#,
            series_entry(r#"{"bookId":"21","title":"Gamma (Greek, #1)","bookTitleBare":"Gamma"}"#)
        );
        let header = r#"{"title":"Greek Series","subtitle":"1 primary work • 1 total work","description":{"html":""}}"#;
        let html = format!(
            r#"<html><body><div data-react-class="ReactComponents.SeriesList" data-react-props="{}"></div><div data-react-class="ReactComponents.SeriesHeader" data-react-props="{}"></div></body></html>"#,
            attr_encode(&list),
            attr_encode(header)
        );
        let page = parse_series_detail_html(&html);
        assert_eq!(page.books.len(), 1, "books survive mount reordering");
        assert_eq!(
            page.books[0].position, None,
            "no position is assigned from a header that had not been seen yet"
        );
        assert_eq!(
            page.primary_count,
            Some(1),
            "the header is still read for the roster cutoff"
        );
    }
}
