use std::time::Duration;

use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::text_norm;
use livrarr_http::HttpClient;
use serde::Deserialize;

use crate::live_config::LiveMetadataConfig;
use crate::{NormalizedWorkDetail, ProviderOutcome};

const MIN_TITLE_JACCARD: f64 = 0.75;
const MIN_AUTHOR_OVERLAP: u32 = 1;
const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/books/v1";

// ---------------------------------------------------------------------------
// Google Books API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GbSearchResponse {
    #[serde(default)]
    pub total_items: Option<i32>,
    #[serde(default)]
    pub items: Option<Vec<GbVolume>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GbVolume {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub volume_info: Option<GbVolumeInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GbVolumeInfo {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub authors: Option<Vec<String>>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub published_date: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub page_count: Option<i32>,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub image_links: Option<GbImageLinks>,
    #[serde(default)]
    pub industry_identifiers: Option<Vec<GbIdentifier>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GbImageLinks {
    #[serde(default)]
    pub small_thumbnail: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GbIdentifier {
    #[serde(default, rename = "type")]
    pub identifier_type: Option<String>,
    #[serde(default)]
    pub identifier: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared fetcher-based search (used by work_service + author_service)
// ---------------------------------------------------------------------------

/// Fetch Google Books volumes via the `HttpFetcher` abstraction.
/// Callers build their own URL (different query shapes for search vs bibliography)
/// and map the returned `GbVolume` vec to their own types.
/// Returns `Ok(vec![])` on 403 (quota/invalid key) — non-fatal.
pub async fn fetch_gb_volumes<F: HttpFetcher>(
    fetcher: &F,
    api_key: &str,
    url: String,
) -> Result<Vec<GbVolume>, String> {
    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![("X-Goog-Api-Key".into(), api_key.to_string())],
        body: None,
        timeout: Duration::from_secs(10),
        rate_bucket: RateBucket::GoogleBooks,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
    };

    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| format!("GoogleBooks request failed: {e}"))?;

    if resp.status == 403 {
        tracing::warn!("GoogleBooks returned 403 (likely quota exhaustion or invalid API key)");
        return Ok(vec![]);
    }
    if resp.status >= 400 {
        return Err(format!("GoogleBooks returned {}", resp.status));
    }

    let search: GbSearchResponse =
        serde_json::from_slice(&resp.body).map_err(|e| format!("GoogleBooks parse error: {e}"))?;

    Ok(search.items.unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GoogleBooksClient {
    http: HttpClient,
    live_config: LiveMetadataConfig,
    base_url: String,
}

impl GoogleBooksClient {
    pub fn new(http: HttpClient, live_config: LiveMetadataConfig) -> Self {
        Self {
            http,
            live_config,
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    pub fn with_base_url(
        http: HttpClient,
        live_config: LiveMetadataConfig,
        base_url: String,
    ) -> Self {
        Self {
            http,
            live_config,
            base_url,
        }
    }

    pub async fn fetch(
        &self,
        work: &livrarr_domain::Work,
        _ctx: &crate::EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let cfg = self.live_config.snapshot();
        let api_key = match cfg
            .google_books_api_key
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            Some(k) => k.to_string(),
            None => {
                tracing::debug!(work_id = work.id, "GoogleBooks: no API key configured");
                return ProviderOutcome::NotConfigured;
            }
        };

        let url = if let Some(isbn) = work.isbn_13.as_deref().filter(|s| !s.is_empty()) {
            tracing::debug!(work_id = work.id, isbn = isbn, "GoogleBooks: ISBN lookup");
            format!(
                "{}/volumes?q=isbn:{}",
                self.base_url,
                urlencoding::encode(isbn),
            )
        } else {
            if work.author_name.trim().is_empty() {
                tracing::debug!(
                    work_id = work.id,
                    "GoogleBooks: no author, skipping title+author query"
                );
                return ProviderOutcome::NotFound;
            }
            let lang = work.language.as_deref().unwrap_or("en");
            tracing::debug!(work_id = work.id, title = %work.title, author = %work.author_name, lang = lang, "GoogleBooks: title+author query");
            format!(
                "{}/volumes?q=intitle:{}+inauthor:{}&langRestrict={}&maxResults=5",
                self.base_url,
                urlencoding::encode(&work.title),
                urlencoding::encode(&work.author_name),
                urlencoding::encode(lang),
            )
        };

        let resp = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.http
                .get(&url)
                .header("X-Goog-Api-Key", &api_key)
                .send(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(work_id = work.id, "GoogleBooks: request failed: {e}");
                return ProviderOutcome::WillRetry {
                    reason: livrarr_domain::WillRetryReason::ServerError,
                    next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
                };
            }
            Err(_) => {
                tracing::warn!(work_id = work.id, "GoogleBooks: request timed out");
                return ProviderOutcome::WillRetry {
                    reason: livrarr_domain::WillRetryReason::ServerError,
                    next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
                };
            }
        };

        let status = resp.status().as_u16();
        if status != 200 {
            tracing::warn!(
                work_id = work.id,
                status = status,
                "GoogleBooks: HTTP error"
            );
            return map_http_error(status);
        }

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => {
                return ProviderOutcome::WillRetry {
                    reason: livrarr_domain::WillRetryReason::ServerError,
                    next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
                }
            }
        };

        let search: GbSearchResponse = match serde_json::from_str(&body) {
            Ok(s) => s,
            Err(_) => {
                return ProviderOutcome::WillRetry {
                    reason: livrarr_domain::WillRetryReason::ServerError,
                    next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
                }
            }
        };

        let total = search.total_items.unwrap_or(0);
        let items = match search.items.as_ref().filter(|v| !v.is_empty()) {
            Some(items) => items,
            None => {
                tracing::debug!(
                    work_id = work.id,
                    total_items = total,
                    "GoogleBooks: no results"
                );
                return ProviderOutcome::NotFound;
            }
        };

        if let Some(isbn) = work.isbn_13.as_deref().filter(|s| !s.is_empty()) {
            for vol in items.iter().filter_map(|v| v.volume_info.as_ref()) {
                if verify_isbn_match(isbn, &vol.industry_identifiers) {
                    let title = vol.title.as_deref().unwrap_or("?");
                    tracing::info!(
                        work_id = work.id,
                        gb_title = title,
                        "GoogleBooks: ISBN match found"
                    );
                    return ProviderOutcome::Success(Box::new(map_volume_to_detail(vol)));
                }
            }
            tracing::debug!(
                work_id = work.id,
                isbn = isbn,
                "GoogleBooks: ISBN not verified in returned volumes"
            );
            ProviderOutcome::NotFound
        } else {
            match score_candidates(&work.title, &work.author_name, items) {
                Some(idx) => {
                    let vi = items[idx].volume_info.as_ref().unwrap();
                    let gb_title = vi.title.as_deref().unwrap_or("?");
                    tracing::info!(
                        work_id = work.id,
                        gb_title = gb_title,
                        "GoogleBooks: candidate matched"
                    );
                    ProviderOutcome::Success(Box::new(map_volume_to_detail(vi)))
                }
                None => {
                    tracing::debug!(
                        work_id = work.id,
                        "GoogleBooks: no candidate above threshold"
                    );
                    ProviderOutcome::NotFound
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

pub fn score_candidates(work_title: &str, work_author: &str, items: &[GbVolume]) -> Option<usize> {
    let seed_title = text_norm::title_tokens(work_title);
    let seed_author = text_norm::author_tokens(work_author);

    let mut scored: Vec<(usize, f64, u32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, vol)| {
            let vi = vol.volume_info.as_ref()?;
            let title = vi.title.as_deref()?;
            let hit_title = text_norm::title_tokens(title);
            let title_jaccard = text_norm::jaccard(&seed_title, &hit_title);
            let hit_author_str = vi.authors.as_ref().map(|a| a.join(" ")).unwrap_or_default();
            let hit_author = text_norm::author_tokens(&hit_author_str);
            let author_overlap = seed_author.intersection(&hit_author).count() as u32;
            Some((i, title_jaccard, author_overlap))
        })
        .collect();

    scored.retain(|(_, j, o)| *j >= MIN_TITLE_JACCARD && *o >= MIN_AUTHOR_OVERLAP);

    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.2.cmp(&a.2))
    });

    scored.first().map(|(idx, _, _)| *idx)
}

// ---------------------------------------------------------------------------
// Field mapping
// ---------------------------------------------------------------------------

pub fn map_volume_to_detail(vi: &GbVolumeInfo) -> NormalizedWorkDetail {
    let description = vi.description.as_deref().map(strip_html_tags);
    let year = vi
        .published_date
        .as_deref()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse::<i32>().ok());
    let language = vi
        .language
        .as_deref()
        .map(livrarr_domain::normalize_language);
    let cover_url = vi.image_links.as_ref().and_then(normalize_cover_url);
    let isbn_13 = extract_isbn13(&vi.industry_identifiers);

    NormalizedWorkDetail {
        title: vi.title.clone(),
        subtitle: vi.subtitle.clone(),
        author_name: vi.authors.as_ref().and_then(|a| a.first().cloned()),
        description,
        year,
        series_name: None,
        series_position: None,
        genres: vi.categories.clone(),
        language,
        page_count: vi.page_count,
        duration_seconds: None,
        publisher: vi.publisher.clone(),
        publish_date: vi.published_date.clone(),
        hc_key: None,
        gr_key: None,
        ol_key: None,
        isbn_13,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        cover_url,
        original_title: None,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    }
}

pub fn normalize_cover_url(links: &GbImageLinks) -> Option<String> {
    let raw = links
        .thumbnail
        .as_deref()
        .or(links.small_thumbnail.as_deref())?;
    let mut url = raw.replace("zoom=1", "zoom=0");
    if url.starts_with("http://") {
        url = format!("https://{}", &url["http://".len()..]);
    }
    let parsed = url::Url::parse(&url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    crate::llm_scraper::validate_cover_url(&url, "")
}

pub fn strip_html_tags(html: &str) -> String {
    static RE_TAGS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]*>").unwrap());
    static RE_WS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\s+").unwrap());
    let stripped = RE_TAGS.replace_all(html, " ");
    let decoded = livrarr_domain::decode_xml_entities(&stripped);
    RE_WS.replace_all(decoded.trim(), " ").to_string()
}

pub fn isbn10_to_isbn13(isbn10: &str) -> Option<String> {
    let digits: String = isbn10.chars().filter(|c| *c != '-').collect();
    if digits.len() != 10 {
        return None;
    }
    let body = &digits[..9];
    if !body.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let last = digits.chars().nth(9)?;
    if !last.is_ascii_digit() && last != 'X' && last != 'x' {
        return None;
    }
    let prefix = format!("978{body}");
    let sum: u32 = prefix
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let d = c.to_digit(10).unwrap();
            if i % 2 == 0 {
                d
            } else {
                d * 3
            }
        })
        .sum();
    let check = (10 - (sum % 10)) % 10;
    Some(format!("{prefix}{check}"))
}

pub fn extract_isbn13(identifiers: &Option<Vec<GbIdentifier>>) -> Option<String> {
    let ids = identifiers.as_ref()?;
    for id in ids
        .iter()
        .filter_map(|i| match (&i.identifier_type, &i.identifier) {
            (Some(t), Some(v)) => Some((t.as_str(), v.as_str())),
            _ => None,
        })
    {
        if id.0 == "ISBN_13" {
            return Some(id.1.to_string());
        }
    }
    for id in ids
        .iter()
        .filter_map(|i| match (&i.identifier_type, &i.identifier) {
            (Some(t), Some(v)) => Some((t.as_str(), v.as_str())),
            _ => None,
        })
    {
        if id.0 == "ISBN_10" {
            return isbn10_to_isbn13(id.1);
        }
    }
    None
}

pub fn verify_isbn_match(requested_isbn: &str, identifiers: &Option<Vec<GbIdentifier>>) -> bool {
    let norm_requested: String = requested_isbn
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    let Some(ids) = identifiers.as_ref() else {
        return false;
    };
    for (t, v) in ids
        .iter()
        .filter_map(|i| match (&i.identifier_type, &i.identifier) {
            (Some(t), Some(v)) => Some((t.as_str(), v.as_str())),
            _ => None,
        })
    {
        let norm_v: String = v
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
            .collect();
        if t == "ISBN_13" && norm_v == norm_requested {
            return true;
        }
        if t == "ISBN_10" {
            if let Some(converted) = isbn10_to_isbn13(&norm_v) {
                if converted == norm_requested {
                    return true;
                }
            }
        }
    }
    false
}

fn map_http_error(status: u16) -> ProviderOutcome<NormalizedWorkDetail> {
    match status {
        403 => ProviderOutcome::NotConfigured,
        429 => ProviderOutcome::WillRetry {
            reason: livrarr_domain::WillRetryReason::RateLimit,
            next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(60),
        },
        _ => ProviderOutcome::WillRetry {
            reason: livrarr_domain::WillRetryReason::ServerError,
            next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gb_volume(title: Option<&str>, authors: &[&str]) -> GbVolume {
        GbVolume {
            id: Some("volume-id".to_string()),
            volume_info: Some(GbVolumeInfo {
                title: title.map(str::to_string),
                subtitle: None,
                authors: Some(authors.iter().map(|a| (*a).to_string()).collect()),
                description: None,
                published_date: None,
                publisher: None,
                page_count: None,
                categories: None,
                language: None,
                image_links: None,
                industry_identifiers: None,
            }),
        }
    }

    fn gb_identifier(identifier_type: Option<&str>, identifier: Option<&str>) -> GbIdentifier {
        GbIdentifier {
            identifier_type: identifier_type.map(str::to_string),
            identifier: identifier.map(str::to_string),
        }
    }

    fn test_http_client() -> HttpClient {
        HttpClient::builder().build().unwrap()
    }

    fn test_live_config() -> LiveMetadataConfig {
        LiveMetadataConfig::new(livrarr_db::MetadataConfig {
            hardcover_enabled: false,
            hardcover_api_token: None,
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: Some("gb-test-key".to_string()),
        })
    }

    /// REQ-001: GoogleBooksClient::new initializes the default Google Books API base URL.
    #[test]
    fn google_books_client_new_uses_default_base_url() {
        let client = GoogleBooksClient::new(test_http_client(), test_live_config());

        assert_eq!(client.base_url, DEFAULT_BASE_URL);
        assert_eq!(
            client
                .live_config
                .snapshot()
                .google_books_api_key
                .as_deref(),
            Some("gb-test-key")
        );
    }

    /// REQ-001: GoogleBooksClient::with_base_url stores the custom base URL used by tests.
    #[test]
    fn google_books_client_with_base_url_uses_custom_base_url() {
        let client = GoogleBooksClient::with_base_url(
            test_http_client(),
            test_live_config(),
            "http://127.0.0.1:9999/books/v1".to_string(),
        );

        assert_eq!(client.base_url, "http://127.0.0.1:9999/books/v1");
        assert_eq!(
            client
                .live_config
                .snapshot()
                .google_books_api_key
                .as_deref(),
            Some("gb-test-key")
        );
    }

    /// REQ-013: Google Books search JSON with totalItems and no items deserializes without panic.
    #[test]
    fn gb_search_response_deserializes_empty() {
        let json = r#"{"totalItems": 0}"#;
        let resp: GbSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total_items, Some(0));
        assert!(resp.items.is_none());
    }

    /// REQ-013: Google Books search JSON tolerates entirely missing optional fields.
    #[test]
    fn gb_search_response_tolerates_missing_fields() {
        let json = r#"{}"#;
        let resp: GbSearchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.total_items.is_none());
        assert!(resp.items.is_none());
    }

    /// REQ-013: Malformed identifier entries are deserialized as optional fields, not errors.
    #[test]
    fn gb_identifier_tolerates_missing_type() {
        let json = r#"{"identifier": "1234567890"}"#;
        let id: GbIdentifier = serde_json::from_str(json).unwrap();
        assert!(id.identifier_type.is_none());
        assert_eq!(id.identifier.as_deref(), Some("1234567890"));
    }

    /// REQ-003: score_candidates returns the exact-match index when title Jaccard and author overlap meet thresholds.
    #[test]
    fn score_candidates_perfect_match_returns_first_index() {
        let items = vec![gb_volume(Some("Kitchen"), &["Banana Yoshimoto"])];
        let seed_title = livrarr_domain::text_norm::title_tokens("Kitchen");
        let hit_title = livrarr_domain::text_norm::title_tokens("Kitchen");
        let seed_author = livrarr_domain::text_norm::author_tokens("Banana Yoshimoto");
        let hit_author = livrarr_domain::text_norm::author_tokens("Banana Yoshimoto");
        let author_overlap = seed_author.intersection(&hit_author).count() as u32;

        assert!(livrarr_domain::text_norm::jaccard(&seed_title, &hit_title) >= MIN_TITLE_JACCARD);
        assert!(author_overlap >= MIN_AUTHOR_OVERLAP);
        assert_eq!(
            score_candidates("Kitchen", "Banana Yoshimoto", &items),
            Some(0)
        );
    }

    /// REQ-003: score_candidates ranks candidates by title Jaccard, then author overlap, with no runner-up rejection.
    #[test]
    fn score_candidates_multiple_candidates_best_index_wins() {
        let items = vec![
            gb_volume(Some("Kitchen Confidential Revised"), &["Banana Yoshimoto"]),
            gb_volume(Some("Kitchen"), &["Banana Yoshimoto"]),
            gb_volume(Some("Kitchen"), &["Someone Else"]),
        ];
        let seed = livrarr_domain::text_norm::title_tokens("Kitchen");
        let runner_up = livrarr_domain::text_norm::title_tokens("Kitchen Confidential Revised");
        let best = livrarr_domain::text_norm::title_tokens("Kitchen");

        // Runner-up has lower Jaccard than best; best is exact match
        assert!(
            livrarr_domain::text_norm::jaccard(&seed, &runner_up)
                < livrarr_domain::text_norm::jaccard(&seed, &best)
        );
        // Best candidate (index 1) wins — exact title match + author overlap
        assert_eq!(
            score_candidates("Kitchen", "Banana Yoshimoto", &items),
            Some(1)
        );
    }

    /// REQ-003: score_candidates returns None when every candidate falls below the title threshold.
    #[test]
    fn score_candidates_low_title_jaccard_returns_none() {
        let items = vec![gb_volume(Some("Norwegian Wood"), &["Banana Yoshimoto"])];
        let seed = livrarr_domain::text_norm::title_tokens("Kitchen");
        let miss = livrarr_domain::text_norm::title_tokens("Norwegian Wood");

        assert!(livrarr_domain::text_norm::jaccard(&seed, &miss) < MIN_TITLE_JACCARD);
        assert_eq!(
            score_candidates("Kitchen", "Banana Yoshimoto", &items),
            None
        );
    }

    /// REQ-003: score_candidates returns None when title matches but author overlap is zero.
    #[test]
    fn score_candidates_zero_author_overlap_returns_none() {
        let items = vec![gb_volume(Some("Kitchen"), &["Haruki Murakami"])];
        let seed_author = livrarr_domain::text_norm::author_tokens("Banana Yoshimoto");
        let hit_author = livrarr_domain::text_norm::author_tokens("Haruki Murakami");

        assert_eq!(seed_author.intersection(&hit_author).count(), 0);
        assert_eq!(
            score_candidates("Kitchen", "Banana Yoshimoto", &items),
            None
        );
    }

    /// REQ-003: score_candidates returns None for empty input and skips candidates with missing volumeInfo/title.
    #[test]
    fn score_candidates_empty_or_malformed_items_return_none() {
        let malformed = vec![
            GbVolume {
                id: Some("missing-info".to_string()),
                volume_info: None,
            },
            gb_volume(None, &["Banana Yoshimoto"]),
        ];

        assert_eq!(score_candidates("Kitchen", "Banana Yoshimoto", &[]), None);
        assert_eq!(
            score_candidates("Kitchen", "Banana Yoshimoto", &malformed),
            None
        );
    }

    /// REQ-002 REQ-011 REQ-012 REQ-013 REQ-002: map_volume_to_detail maps all supported fields from canned Google Books JSON.
    #[test]
    fn map_volume_to_detail_maps_full_volume_info() {
        let json = r#"{
            "title": "Kitchen",
            "subtitle": "A Novel",
            "authors": ["Banana Yoshimoto", "Translator Name"],
            "description": "<p>A <b>quiet</b> novel &amp; story.</p>",
            "publishedDate": "1993-03-01",
            "publisher": "Grove Press",
            "pageCount": 160,
            "categories": ["Fiction", "Classics"],
            "language": "ja",
            "imageLinks": {
                "smallThumbnail": "http://books.google.com/books/content?id=small&zoom=1&source=gbs_api",
                "thumbnail": "http://books.google.com/books/content?id=large&zoom=1&source=gbs_api"
            },
            "industryIdentifiers": [
                {"type": "ISBN_10", "identifier": "0306406152"}
            ]
        }"#;
        let vi: GbVolumeInfo = serde_json::from_str(json).unwrap();
        let detail = map_volume_to_detail(&vi);

        assert_eq!(detail.title.as_deref(), Some("Kitchen"));
        assert_eq!(detail.subtitle.as_deref(), Some("A Novel"));
        assert_eq!(detail.author_name.as_deref(), Some("Banana Yoshimoto"));
        assert_eq!(
            detail.description.as_deref(),
            Some("A quiet novel & story.")
        );
        assert_eq!(detail.publish_date.as_deref(), Some("1993-03-01"));
        assert_eq!(detail.year, Some(1993));
        assert_eq!(detail.publisher.as_deref(), Some("Grove Press"));
        assert_eq!(detail.page_count, Some(160));
        assert_eq!(
            detail.genres,
            Some(vec!["Fiction".to_string(), "Classics".to_string()])
        );
        assert_eq!(detail.language.as_deref(), Some("ja"));
        assert_eq!(detail.isbn_13.as_deref(), Some("9780306406157"));
        assert_eq!(
            detail.cover_url.as_deref(),
            Some("https://books.google.com/books/content?id=large&zoom=0&source=gbs_api")
        );
    }

    /// REQ-013: map_volume_to_detail turns an empty volumeInfo object into a detail with absent optional fields.
    #[test]
    fn map_volume_to_detail_empty_volume_info_has_no_optional_fields() {
        let vi: GbVolumeInfo = serde_json::from_str("{}").unwrap();
        let detail = map_volume_to_detail(&vi);

        assert!(detail.title.is_none());
        assert!(detail.subtitle.is_none());
        assert!(detail.author_name.is_none());
        assert!(detail.description.is_none());
        assert!(detail.year.is_none());
        assert!(detail.publisher.is_none());
        assert!(detail.publish_date.is_none());
        assert!(detail.page_count.is_none());
        assert!(detail.genres.is_none());
        assert!(detail.language.is_none());
        assert!(detail.cover_url.is_none());
        assert!(detail.isbn_13.is_none());
    }

    /// REQ-013: map_volume_to_detail extracts the leading year from supported publishedDate formats.
    #[test]
    fn map_volume_to_detail_extracts_year_from_date_prefix() {
        for (published_date, expected_year) in [
            ("2024", Some(2024)),
            ("2024-01", Some(2024)),
            ("2024-01-15", Some(2024)),
            ("unknown", None),
        ] {
            let vi = GbVolumeInfo {
                title: None,
                subtitle: None,
                authors: None,
                description: None,
                published_date: Some(published_date.to_string()),
                publisher: None,
                page_count: None,
                categories: None,
                language: None,
                image_links: None,
                industry_identifiers: None,
            };

            assert_eq!(map_volume_to_detail(&vi).year, expected_year);
        }
    }

    /// REQ-011: normalize_cover_url prefers thumbnail, upgrades HTTP to HTTPS, and rewrites zoom=1 to zoom=0.
    #[test]
    fn normalize_cover_url_prefers_thumbnail_and_rewrites_google_url() {
        let links = GbImageLinks {
            small_thumbnail: Some(
                "http://books.google.com/books/content?id=small&zoom=1&source=gbs_api".to_string(),
            ),
            thumbnail: Some(
                "http://books.google.com/books/content?id=large&zoom=1&source=gbs_api".to_string(),
            ),
        };

        assert_eq!(
            normalize_cover_url(&links).as_deref(),
            Some("https://books.google.com/books/content?id=large&zoom=0&source=gbs_api")
        );
    }

    /// REQ-011: normalize_cover_url leaves URLs without a zoom query unchanged except for HTTPS normalization.
    #[test]
    fn normalize_cover_url_does_not_append_missing_zoom() {
        let links = GbImageLinks {
            small_thumbnail: None,
            thumbnail: Some(
                "http://books.google.com/books/content?id=large&source=gbs_api".to_string(),
            ),
        };

        assert_eq!(
            normalize_cover_url(&links).as_deref(),
            Some("https://books.google.com/books/content?id=large&source=gbs_api")
        );
    }

    /// REQ-011: normalize_cover_url rejects embedded credentials and SSRF-risk hosts.
    #[test]
    fn normalize_cover_url_rejects_credentials_and_private_hosts() {
        let credentials = GbImageLinks {
            small_thumbnail: None,
            thumbnail: Some(
                "https://user:pass@books.google.com/books/content?id=large&zoom=1".to_string(),
            ),
        };
        let localhost = GbImageLinks {
            small_thumbnail: None,
            thumbnail: Some("http://localhost/books/content?id=large&zoom=1".to_string()),
        };
        let private_ip = GbImageLinks {
            small_thumbnail: None,
            thumbnail: Some("http://127.0.0.1/books/content?id=large&zoom=1".to_string()),
        };

        assert!(normalize_cover_url(&credentials).is_none());
        assert!(normalize_cover_url(&localhost).is_none());
        assert!(normalize_cover_url(&private_ip).is_none());
    }

    /// REQ-011: normalize_cover_url returns None when Google Books provides no image links.
    #[test]
    fn normalize_cover_url_missing_links_returns_none() {
        let links = GbImageLinks {
            small_thumbnail: None,
            thumbnail: None,
        };

        assert!(normalize_cover_url(&links).is_none());
    }

    /// REQ-012: strip_html_tags removes tags, decodes basic entities, collapses whitespace, and preserves plain text.
    #[test]
    fn strip_html_tags_removes_tags_and_decodes_entities() {
        assert_eq!(strip_html_tags("<b>bold</b>"), "bold");
        assert_eq!(strip_html_tags("<p>para</p><p>next</p>"), "para next");
        assert_eq!(
            strip_html_tags("Tom &amp; Jerry &lt;3 &quot;Books&quot;"),
            "Tom & Jerry <3 \"Books\""
        );
        assert_eq!(strip_html_tags("plain text"), "plain text");
        assert_eq!(strip_html_tags(""), "");
    }

    /// REQ-002: isbn10_to_isbn13 converts valid ISBN-10 values, including hyphenated values.
    #[test]
    fn isbn10_to_isbn13_converts_valid_values() {
        assert_eq!(
            isbn10_to_isbn13("0306406152").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(
            isbn10_to_isbn13("0-306-40615-2").as_deref(),
            Some("9780306406157")
        );
    }

    /// REQ-002: isbn10_to_isbn13 rejects invalid length and non-numeric body characters.
    #[test]
    fn isbn10_to_isbn13_rejects_invalid_values() {
        assert!(isbn10_to_isbn13("030640615").is_none());
        assert!(isbn10_to_isbn13("03064061522").is_none());
        assert!(isbn10_to_isbn13("03064A6152").is_none());
    }

    /// REQ-002: extract_isbn13 returns ISBN-13 directly, preferring it over ISBN-10 conversion.
    #[test]
    fn extract_isbn13_prefers_direct_isbn13() {
        let identifiers = Some(vec![
            gb_identifier(Some("ISBN_10"), Some("0306406152")),
            gb_identifier(Some("ISBN_13"), Some("9784101010014")),
        ]);

        assert_eq!(
            extract_isbn13(&identifiers).as_deref(),
            Some("9784101010014")
        );
    }

    /// REQ-002: extract_isbn13 converts ISBN-10 when no ISBN-13 exists.
    #[test]
    fn extract_isbn13_converts_isbn10_only() {
        let identifiers = Some(vec![gb_identifier(Some("ISBN_10"), Some("0306406152"))]);

        assert_eq!(
            extract_isbn13(&identifiers).as_deref(),
            Some("9780306406157")
        );
    }

    /// REQ-002: extract_isbn13 returns None for absent identifiers or malformed entries.
    #[test]
    fn extract_isbn13_returns_none_for_absent_or_malformed_entries() {
        let malformed = Some(vec![
            gb_identifier(None, Some("9780306406157")),
            gb_identifier(Some("ISBN_13"), None),
            gb_identifier(Some("OTHER"), Some("not-isbn")),
        ]);

        assert!(extract_isbn13(&None).is_none());
        assert!(extract_isbn13(&malformed).is_none());
    }

    /// REQ-014: verify_isbn_match accepts exact ISBN-13 matches and ISBN-10 matches after conversion.
    #[test]
    fn verify_isbn_match_accepts_matching_isbn13_or_converted_isbn10() {
        let direct = Some(vec![gb_identifier(Some("ISBN_13"), Some("9780306406157"))]);
        let converted = Some(vec![gb_identifier(Some("ISBN_10"), Some("0306406152"))]);

        assert!(verify_isbn_match("9780306406157", &direct));
        assert!(verify_isbn_match("9780306406157", &converted));
    }

    /// REQ-014: verify_isbn_match rejects non-matches, missing identifiers, and malformed entries.
    #[test]
    fn verify_isbn_match_rejects_absent_or_different_isbn() {
        let different = Some(vec![gb_identifier(Some("ISBN_13"), Some("9784101010014"))]);
        let malformed = Some(vec![gb_identifier(None, Some("9780306406157"))]);

        assert!(!verify_isbn_match("9780306406157", &different));
        assert!(!verify_isbn_match("9780306406157", &malformed));
        assert!(!verify_isbn_match("9780306406157", &None));
    }

    /// REQ-016: map_http_error maps 403 to NotConfigured.
    #[test]
    fn map_http_error_403_returns_not_configured() {
        assert!(matches!(
            map_http_error(403),
            ProviderOutcome::NotConfigured
        ));
    }

    /// REQ-016: map_http_error maps 429 to a rate-limit retry outcome.
    #[test]
    fn map_http_error_429_returns_rate_limit_retry() {
        assert!(matches!(
            map_http_error(429),
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::RateLimit,
                ..
            }
        ));
    }

    /// REQ-016: map_http_error maps 5xx and unexpected statuses to server-error retry outcomes.
    #[test]
    fn map_http_error_server_and_unexpected_status_return_server_error_retry() {
        assert!(matches!(
            map_http_error(500),
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                ..
            }
        ));
        assert!(matches!(
            map_http_error(418),
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                ..
            }
        ));
    }

    /// REQ-002: isbn10_to_isbn13 handles ISBN-10 with 'X' check digit.
    #[test]
    fn isbn10_to_isbn13_handles_x_check_digit() {
        assert_eq!(
            isbn10_to_isbn13("155404295X").as_deref(),
            Some("9781554042951")
        );
        assert_eq!(
            isbn10_to_isbn13("1-55404-295-X").as_deref(),
            Some("9781554042951")
        );
    }

    /// REQ-003: score_candidates handles candidates with missing authors gracefully.
    #[test]
    fn score_candidates_candidate_with_no_authors_returns_none() {
        let items = vec![GbVolume {
            id: Some("no-author".to_string()),
            volume_info: Some(GbVolumeInfo {
                title: Some("Kitchen".to_string()),
                subtitle: None,
                authors: None,
                description: None,
                published_date: None,
                publisher: None,
                page_count: None,
                categories: None,
                language: None,
                image_links: None,
                industry_identifiers: None,
            }),
        }];
        assert_eq!(
            score_candidates("Kitchen", "Banana Yoshimoto", &items),
            None
        );
    }

    /// REQ-004: MetadataConfig Debug output redacts google_books_api_key.
    #[test]
    fn metadata_config_debug_redacts_google_books_key() {
        let cfg = livrarr_db::MetadataConfig {
            hardcover_enabled: false,
            hardcover_api_token: None,
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: String::new(),
            languages: vec![],
            google_books_api_key: Some("super-secret-key-12345".to_string()),
        };
        let debug_output = format!("{:?}", cfg);
        assert!(
            !debug_output.contains("super-secret-key-12345"),
            "raw key leaked in Debug output: {debug_output}"
        );
        assert!(debug_output.contains("[REDACTED]"));
    }

    /// REQ-004: fetch returns NotConfigured when API key is absent.
    #[tokio::test]
    async fn fetch_returns_not_configured_when_key_missing() {
        let no_key_config = LiveMetadataConfig::new(livrarr_db::MetadataConfig {
            hardcover_enabled: false,
            hardcover_api_token: None,
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: String::new(),
            languages: vec![],
            google_books_api_key: None,
        });
        let client = GoogleBooksClient::new(test_http_client(), no_key_config);
        let work = livrarr_domain::Work::default();
        let ctx = crate::EnrichmentContext {
            priority: crate::RequestPriority::Normal,
            mode: crate::EnrichmentMode::Background,
        };
        let result = client.fetch(&work, &ctx).await;
        assert!(
            matches!(result, ProviderOutcome::NotConfigured),
            "expected NotConfigured, got {result:?}"
        );
    }
}
