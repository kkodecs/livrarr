use std::time::Duration;

use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use serde::Deserialize;

use crate::live_config::LiveMetadataConfig;
use crate::{NormalizedWorkDetail, ProviderOutcome};

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
    priority: RequestPriority,
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
        priority,
    };

    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| format!("GoogleBooks request failed: {e}"))?;

    if resp.status == 403 {
        tracing::warn!("GoogleBooks returned 403 (likely quota exhaustion or invalid API key)");
        // The empty-result return below is unchanged, but the breaker must still
        // learn: a quota-exhausted key answers 403 to everything, and silently
        // reporting "no results" forever taught it nothing.
        outbound_queue::shared().report_outcome(RateBucket::GoogleBooks, BreakerSignal::Failure);
        return Ok(vec![]);
    }
    if resp.status >= 400 {
        // No 404/410 exemption: `/books/v1/volumes` is a search endpoint with no
        // "this book is absent" status — an empty result set is a 200 with
        // `totalItems: 0` — so a 404 means the ROUTE moved or is blocked.
        outbound_queue::shared().report_outcome(RateBucket::GoogleBooks, BreakerSignal::Failure);
        return Err(format!("GoogleBooks returned {}", resp.status));
    }

    let search: GbSearchResponse =
        serde_json::from_slice(&resp.body).map_err(|e| format!("GoogleBooks parse error: {e}"))?;

    // A parsed response — including a legitimately empty one — is a healthy
    // answer. Without this the C4 Failure reports above could open the breaker
    // and nothing on this door could ever close it again: `record_success` is
    // the only transition out of HalfOpen.
    outbound_queue::shared().report_outcome(RateBucket::GoogleBooks, BreakerSignal::Success);

    Ok(search.items.unwrap_or_default())
}

/// Fetch + parse a Google Books search response for `GoogleBooksClient::fetch`
/// / `fetch_by_isbn`, applying `map_http_error`'s exact status classification.
///
/// The fetcher intercepts HTTP 429 at the transport level
/// (`FetchError::RateLimited`) rather than surfacing it as a normal response
/// status — translated back to `map_http_error(429)` here so the existing
/// 6-hour+jitter quota-exhaustion backoff (vs. the 300s generic backoff) is
/// preserved exactly. Any other transport failure (network, timeout, body)
/// maps to the same generic `WillRetry { ServerError }` the pre-fetcher code
/// used for both `.send()` failures and the manual `tokio::time::timeout`.
async fn fetch_gb_search<F: HttpFetcher>(
    fetcher: &F,
    api_key: &str,
    url: String,
    priority: RequestPriority,
) -> Result<GbSearchResponse, ProviderOutcome<NormalizedWorkDetail>> {
    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![("X-Goog-Api-Key".to_string(), api_key.to_string())],
        body: None,
        timeout: Duration::from_secs(10),
        rate_bucket: RateBucket::GoogleBooks,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };
    let resp = match fetcher.fetch(req).await {
        Ok(r) => r,
        Err(livrarr_domain::services::FetchError::RateLimited) => {
            tracing::warn!("GoogleBooks: request failed: rate limited");
            return Err(map_http_error(429));
        }
        Err(livrarr_domain::services::FetchError::CircuitOpen { retry_after }) => {
            return Err(ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::CircuitOpen,
                next_attempt_at: chrono::Utc::now()
                    + chrono::Duration::from_std(retry_after)
                        .unwrap_or_else(|_| chrono::Duration::seconds(60)),
            });
        }
        Err(livrarr_domain::services::FetchError::QueueFull { retry_after }) => {
            return Err(ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::QueueFull,
                next_attempt_at: chrono::Utc::now()
                    + chrono::Duration::from_std(retry_after)
                        .unwrap_or_else(|_| chrono::Duration::seconds(60)),
            });
        }
        Err(e) => {
            tracing::warn!("GoogleBooks: request failed: {e}");
            return Err(ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
            });
        }
    };

    // R-9: a 403 body's error reason discriminates quota exhaustion (breaker
    // TripImmediately, open until the next Pacific-midnight quota reset) from
    // a bad API key (breaker Failure, threshold-counted like any other
    // response-derived failure). The `ProviderOutcome` returned is unchanged
    // either way (`NotConfigured`, via `map_http_error(403)`) — only the
    // breaker signal differs.
    if resp.status == 403 {
        match gb_403_quota_reason(&resp.body) {
            Some(()) => {
                outbound_queue::shared().report_outcome(
                    RateBucket::GoogleBooks,
                    BreakerSignal::TripImmediately {
                        open_for: Some(duration_until_pacific_midnight()),
                    },
                );
            }
            None => {
                outbound_queue::shared()
                    .report_outcome(RateBucket::GoogleBooks, BreakerSignal::Failure);
            }
        }
        return Err(map_http_error(403));
    }

    if resp.status != 200 {
        tracing::warn!(status = resp.status, "GoogleBooks: HTTP error");
        // 403 is handled above with its own quota-vs-bad-key discrimination.
        // Everything else is a health signal — reporting only 5xx left a 401
        // storm invisible to the breaker. No 404/410 exemption: this is the
        // search endpoint, where an empty result set is a 200 with
        // `totalItems: 0`, so a 404 means the ROUTE moved or is blocked.
        outbound_queue::shared().report_outcome(RateBucket::GoogleBooks, BreakerSignal::Failure);
        return Err(map_http_error(resp.status));
    }

    let parsed: GbSearchResponse =
        serde_json::from_slice(&resp.body).map_err(|_| ProviderOutcome::WillRetry {
            reason: livrarr_domain::WillRetryReason::ServerError,
            next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        })?;
    outbound_queue::shared().report_outcome(RateBucket::GoogleBooks, BreakerSignal::Success);
    Ok(parsed)
}

/// R-9: does a GB 403 response body indicate daily-quota / rate-limit
/// exhaustion (`quotaExceeded` / `rateLimitExceeded`) rather than a bad API
/// key? `Some(())` = quota reason found; `None` = any other 403 (or an
/// unparseable body — treated as a bad key, the conservative default).
fn gb_403_quota_reason(body: &[u8]) -> Option<()> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let reason = json
        .pointer("/error/errors/0/reason")
        .and_then(|v| v.as_str())?;
    matches!(reason, "quotaExceeded" | "rateLimitExceeded").then_some(())
}

/// Duration until the next America/Los_Angeles midnight (Google Books' daily
/// quota reset). Uses a fixed UTC-8 (PST) offset — this workspace has no
/// chrono-tz dependency, so Pacific Daylight Time (UTC-7, roughly Mar-Nov)
/// reads up to one hour early. Acceptable for a quota-backoff heuristic
/// (the breaker still opens; a probe just becomes eligible up to an hour
/// sooner than the provider's actual reset), not a scheduling guarantee.
fn duration_until_pacific_midnight() -> Duration {
    let now = chrono::Utc::now();
    let pacific = chrono::FixedOffset::west_opt(8 * 3600).expect("valid fixed offset");
    let now_pacific = now.with_timezone(&pacific);
    let next_midnight_pacific = (now_pacific.date_naive() + chrono::Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always a valid time")
        .and_local_timezone(pacific)
        .single()
        .expect("a fixed offset has no DST ambiguity");
    let next_midnight_utc = next_midnight_pacific.with_timezone(&chrono::Utc);
    (next_midnight_utc - now).to_std().unwrap_or(Duration::ZERO)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GoogleBooksClient {
    fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    live_config: LiveMetadataConfig,
    base_url: String,
    call_sink: Option<std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>>,
}

impl GoogleBooksClient {
    pub fn new(
        fetcher: livrarr_http::fetcher::HttpFetcherImpl,
        live_config: LiveMetadataConfig,
    ) -> Self {
        Self {
            fetcher,
            live_config,
            base_url: DEFAULT_BASE_URL.to_string(),
            call_sink: None,
        }
    }

    pub fn with_base_url(
        fetcher: livrarr_http::fetcher::HttpFetcherImpl,
        live_config: LiveMetadataConfig,
        base_url: String,
    ) -> Self {
        Self {
            fetcher,
            live_config,
            base_url,
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001).
    pub fn with_call_sink(
        mut self,
        sink: std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>,
    ) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub(crate) fn sink_ref(
        &self,
    ) -> Option<&std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>> {
        self.call_sink.as_ref()
    }

    /// Anchor-only fetch (REQ-006): ISBN volumes query with the same key gate,
    /// transport handling, and ISBN verification as the seeded fetch — and no
    /// intitle/inauthor fallback.
    pub async fn fetch_by_isbn(
        &self,
        isbn: &str,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let cfg = self.live_config.snapshot();
        let api_key = match cfg
            .google_books_api_key
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            Some(k) => k.to_string(),
            None => {
                tracing::debug!(isbn = isbn, "GoogleBooks: no API key configured");
                return ProviderOutcome::NotConfigured;
            }
        };
        let url = format!(
            "{}/volumes?q=isbn:{}",
            self.base_url,
            urlencoding::encode(isbn),
        );

        let search = match fetch_gb_search(&self.fetcher, &api_key, url, priority).await {
            Ok(s) => s,
            Err(outcome) => return outcome,
        };

        let Some(items) = search.items.as_ref().filter(|v| !v.is_empty()) else {
            tracing::debug!(isbn = isbn, "GoogleBooks: no results");
            return ProviderOutcome::NotFound;
        };
        for vol in items.iter().filter_map(|v| v.volume_info.as_ref()) {
            if verify_isbn_match(isbn, &vol.industry_identifiers) {
                return ProviderOutcome::Success(Box::new(map_volume_to_detail(vol)));
            }
        }
        tracing::debug!(
            isbn = isbn,
            "GoogleBooks: ISBN not verified in returned volumes"
        );
        ProviderOutcome::NotFound
    }

    pub async fn fetch(
        &self,
        work: &livrarr_domain::Work,
        priority: RequestPriority,
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

        let search = match fetch_gb_search(&self.fetcher, &api_key, url, priority).await {
            Ok(s) => s,
            Err(outcome) => return outcome,
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
            let pairs: Vec<(String, String)> = items
                .iter()
                .map(|v| {
                    let vi = v.volume_info.as_ref();
                    let title = vi.and_then(|x| x.title.clone()).unwrap_or_default();
                    let author = vi
                        .and_then(|x| x.authors.as_ref())
                        .map(|a| a.join(" "))
                        .unwrap_or_default();
                    (title, author)
                })
                .collect();
            match livrarr_domain::identity_matching::pick_best_candidate(
                &work.title,
                &work.author_name,
                &pairs,
                false,
            ) {
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
// Field mapping
// ---------------------------------------------------------------------------

static RE_GB_SERIES: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\((.+?),\s*#(\d+(?:\.\d+)?)\)\s*$").unwrap());

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
    let (series_name, series_position) = vi
        .title
        .as_deref()
        .and_then(|t| {
            RE_GB_SERIES
                .captures(t)
                .map(|c| (Some(c[1].to_string()), c[2].parse::<f64>().ok()))
        })
        .unwrap_or((None, None));

    NormalizedWorkDetail {
        title: vi.title.clone(),
        subtitle: vi.subtitle.clone(),
        author_name: vi.authors.as_ref().and_then(|a| a.first().cloned()),
        description,
        year,
        series_name,
        series_position,
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
    crate::provider_util::validate_cover_url(&url, "")
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

/// Convert a checksum-valid ISBN-10 to its canonical ISBN-13.
///
/// Thin alias over the single normalization authority
/// [`livrarr_domain::normalization::normalize_isbn13`] (D-009): there is exactly
/// one implementation of the length+checksum+conversion rule in the workspace.
/// Note this now validates the ISBN-10 checksum (a malformed input yields `None`
/// rather than a fabricated ISBN-13).
pub fn isbn10_to_isbn13(isbn10: &str) -> Option<String> {
    livrarr_domain::normalization::normalize_isbn13(isbn10)
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
        429 => {
            // Rate-limited / daily quota exhausted (Google Books free tier is
            // ~1000 req/day; once spent, every call 429s until the daily reset).
            // Back off several hours so the background retry loop doesn't pound
            // an exhausted quota ~1 req/s; jitter spreads works so they don't
            // retry in lockstep. Foreground refresh (Manual/HardRefresh) bypasses
            // next_attempt_at, so a user can still force an immediate retry.
            let jitter_secs = (chrono::Utc::now().timestamp_subsec_nanos() % 10_800) as i64;
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::RateLimit,
                next_attempt_at: chrono::Utc::now()
                    + chrono::Duration::hours(6)
                    + chrono::Duration::seconds(jitter_secs),
            }
        }
        _ => ProviderOutcome::WillRetry {
            reason: livrarr_domain::WillRetryReason::ServerError,
            next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C4, test-plan item 6: a refusal must reach this client's own bucket.
    /// `/books/v1/volumes` is a SEARCH endpoint — an empty result set is a 200
    /// with `totalItems: 0` — so a 404 there means the route moved or is
    /// blocked, and gets no absence exemption.
    #[tokio::test]
    async fn a_search_refusal_and_a_dead_search_route_both_report_a_breaker_failure() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        let queue = outbound_queue::shared();

        for status in [403u16, 404u16, 401u16] {
            queue.set_breaker_config_for_tests(
                RateBucket::GoogleBooks,
                CircuitBreakerConfig {
                    failure_threshold: 1,
                    evaluation_window_secs: 60,
                    open_duration_secs: 60,
                    half_open_probe_count: 1,
                },
            );
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(status, vec![]);
            let _ = fetch_gb_volumes(
                &fetcher,
                "k",
                "https://www.googleapis.com/books/v1/volumes?q=x".to_string(),
                RequestPriority::Normal,
            )
            .await;
            let tripped = {
                let admission = queue
                    .acquire(RateBucket::GoogleBooks, RequestPriority::Normal)
                    .await;
                matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
            };
            assert!(
                tripped,
                "Google Books search returning {status} must report a breaker failure"
            );
        }
    }

    /// C4 added Failure reporting to this door but no Success, and
    /// `record_success` is the ONLY transition out of HalfOpen
    /// (`breaker.rs`). A door that can open a breaker but never close it
    /// leaves recovery to whichever unrelated code path happens to run next.
    /// A legitimate empty result set IS a healthy answer and must say so.
    #[tokio::test]
    async fn a_healthy_volumes_result_reports_operation_success() {
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        let queue = outbound_queue::shared();

        // One below the production threshold of 5.
        for _ in 0..4 {
            queue.report_outcome(RateBucket::GoogleBooks, BreakerSignal::Failure);
        }

        let canned = serde_json::json!({"totalItems": 0}).to_string();
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, canned.into_bytes());
        let volumes = fetch_gb_volumes(
            &fetcher,
            "test-key",
            "https://example.com/volumes".into(),
            RequestPriority::Normal,
        )
        .await
        .expect("a 200 with totalItems 0 is a healthy answer");
        assert!(volumes.is_empty());

        queue.report_outcome(RateBucket::GoogleBooks, BreakerSignal::Failure);

        let tripped = {
            let admission = queue
                .acquire(RateBucket::GoogleBooks, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            !tripped,
            "a healthy volumes fetch must report Success and clear the \
             accumulated failures — otherwise this door can open the breaker \
             but never close it"
        );
    }

    fn gb_identifier(identifier_type: Option<&str>, identifier: Option<&str>) -> GbIdentifier {
        GbIdentifier {
            identifier_type: identifier_type.map(str::to_string),
            identifier: identifier.map(str::to_string),
        }
    }

    fn test_fetcher() -> livrarr_http::fetcher::HttpFetcherImpl {
        livrarr_http::fetcher::HttpFetcherImpl::new().unwrap()
    }

    fn test_live_config() -> LiveMetadataConfig {
        LiveMetadataConfig::new(livrarr_domain::settings::MetadataConfig {
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
        let client = GoogleBooksClient::new(test_fetcher(), test_live_config());

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
            test_fetcher(),
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

    /// REQ-004: MetadataConfig Debug output redacts google_books_api_key.
    #[test]
    fn metadata_config_debug_redacts_google_books_key() {
        let cfg = livrarr_domain::settings::MetadataConfig {
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
        let no_key_config = LiveMetadataConfig::new(livrarr_domain::settings::MetadataConfig {
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
        let client = GoogleBooksClient::new(test_fetcher(), no_key_config);
        let work = livrarr_domain::Work::default();
        let result = client.fetch(&work, RequestPriority::Normal).await;
        assert!(
            matches!(result, ProviderOutcome::NotConfigured),
            "expected NotConfigured, got {result:?}"
        );
    }

    // -------------------------------------------------------------------
    // fetch_gb_search door-routing / error-mapping: the shared transport
    // helper behind GoogleBooksClient::fetch / fetch_by_isbn.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_gb_search_sends_googlebooks_bucket_get_and_api_key_header() {
        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        let canned = serde_json::json!({"totalItems": 0});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let search = fetch_gb_search(
            &fetcher,
            "test-key",
            "https://example.com/volumes".into(),
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert_eq!(search.total_items, Some(0));
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, "https://example.com/volumes");
        assert_eq!(req.rate_bucket, RateBucket::GoogleBooks);
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "X-Goog-Api-Key" && v == "test-key"));
        assert!(matches!(req.user_agent, UserAgentProfile::Server));
        assert!(!req.anti_bot_check);
        assert_eq!(req.timeout, Duration::from_secs(10));
        assert_eq!(req.max_body_bytes, 2 * 1024 * 1024);
    }

    /// REQ-016 preservation: the fetcher intercepts HTTP 429 as a transport
    /// error (`FetchError::RateLimited`) rather than a normal response — this
    /// must still resolve through `map_http_error(429)`'s 6-hour+jitter
    /// quota-exhaustion backoff, not the generic 300s `ServerError` backoff a
    /// plain transport failure gets.
    #[tokio::test]
    async fn fetch_gb_search_maps_fetcher_rate_limited_to_map_http_error_429() {
        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::RateLimited,
        );

        let outcome = fetch_gb_search(
            &fetcher,
            "test-key",
            "https://example.com/volumes".into(),
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            outcome,
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::RateLimit,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn fetch_gb_search_maps_fetcher_queue_full_to_queue_full_retry() {
        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        // D3/#6: the outbound queue's local admission cap is a transport-
        // level pause — must classify as WillRetry{QueueFull} (budget-
        // exempt), not the generic WillRetry{ServerError} (budget-consuming)
        // every other unmatched transport failure gets.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::QueueFull {
                retry_after: Duration::from_secs(1),
            },
        );

        let outcome = fetch_gb_search(
            &fetcher,
            "test-key",
            "https://example.com/volumes".into(),
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            outcome,
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::QueueFull,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn fetch_gb_search_maps_http_403_to_not_configured() {
        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(403, vec![]);

        let outcome = fetch_gb_search(
            &fetcher,
            "test-key",
            "https://example.com/volumes".into(),
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(outcome, ProviderOutcome::NotConfigured));
    }

    #[tokio::test]
    async fn fetch_gb_search_maps_network_error_to_server_error_retry() {
        let _guard = crate::test_support::lock_breaker(RateBucket::GoogleBooks).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::Timeout(Duration::from_secs(10)),
        );

        let outcome = fetch_gb_search(
            &fetcher,
            "test-key",
            "https://example.com/volumes".into(),
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            outcome,
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                ..
            }
        ));
    }
}
