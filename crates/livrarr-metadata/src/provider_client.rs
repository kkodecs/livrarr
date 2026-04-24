//! Per-provider client seam used by `DefaultProviderQueue`.
//!
//! Real network adapters are added as variants here as the cutover progresses.
//! Phase 1.5 (Sessions 2+3):
//!   - Tracer: `Audnexus` (proves the trait shape against real reqwest plumbing).
//!   - Lift complete: `Hardcover`, `OpenLibrary` (full real wrappers, smoke tested).
//!   - Placeholder: `Goodreads` (variant exists; the existing `goodreads` module
//!     has parsers but no fetch function — that lives in `handlers/enrichment.rs`
//!     and gets pulled in during the orchestration cutover).
//!   - Deferred: `Llm` — `MetadataProvider::Llm` is a dependent-step (post-HC
//!     disambiguation, R-17), not a parallel scatter-gather provider. A
//!     `Llm(_)` variant requires the queue to grow dependent-step orchestration
//!     first. Lands during the orchestration cutover.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use livrarr_domain::{MetadataProvider, Work};
use livrarr_http::HttpClient;

use crate::audnexus::query_audnexus;
use crate::goodreads::{self, GoodreadsDetailResult, GoodreadsFetchError, GOODREADS_BASE_URL};
use crate::hardcover::query_hardcover;
use crate::openlibrary::query_ol_detail;
use crate::{EnrichmentContext, NormalizedWorkDetail, ProviderOutcome};

/// Heterogeneous provider client. Enum dispatch instead of `Box<dyn>` because
/// `trait_variant::make(Send)` traits are not dyn-compatible. New real-provider
/// adapters are added as new variants here.
#[derive(Clone)]
pub enum ProviderClient {
    Stub(StubProviderClient),
    Audnexus(AudnexusClient),
    Hardcover(HardcoverClient),
    OpenLibrary(OpenLibraryClient),
    Goodreads(GoodreadsClient),
}

impl ProviderClient {
    pub async fn fetch(
        &self,
        work: &Work,
        ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match self {
            Self::Stub(s) => s.fetch(work, ctx).await,
            Self::Audnexus(a) => a.fetch(work, ctx).await,
            Self::Hardcover(h) => h.fetch(work, ctx).await,
            Self::OpenLibrary(o) => o.fetch(work, ctx).await,
            Self::Goodreads(g) => g.fetch(work, ctx).await,
        }
    }

    pub fn provider(&self) -> MetadataProvider {
        match self {
            Self::Stub(s) => s.provider,
            Self::Audnexus(_) => MetadataProvider::Audnexus,
            Self::Hardcover(_) => MetadataProvider::Hardcover,
            Self::OpenLibrary(_) => MetadataProvider::OpenLibrary,
            Self::Goodreads(_) => MetadataProvider::Goodreads,
        }
    }

    pub fn call_count(&self) -> usize {
        match self {
            Self::Stub(s) => s.call_count(),
            // Real network adapters don't track call counts — the queue tracks
            // dispatch counts elsewhere; this accessor exists for stub-driven tests.
            Self::Audnexus(_) | Self::Hardcover(_) | Self::OpenLibrary(_) | Self::Goodreads(_) => 0,
        }
    }
}

/// Scriptable provider client for behavioral tests. The harness builds one of
/// these per scenario, configures the outcome it should return, and reads
/// `call_count` to assert dispatch behavior.
#[derive(Clone)]
pub struct StubProviderClient {
    pub provider: MetadataProvider,
    outcome: Arc<Mutex<ProviderOutcome<NormalizedWorkDetail>>>,
    panic_on_call: bool,
    call_count: Arc<AtomicUsize>,
}

impl StubProviderClient {
    pub fn new(provider: MetadataProvider, outcome: ProviderOutcome<NormalizedWorkDetail>) -> Self {
        Self {
            provider,
            outcome: Arc::new(Mutex::new(outcome)),
            panic_on_call: false,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_panic(provider: MetadataProvider) -> Self {
        Self {
            provider,
            // Panic before the lock is touched, so the outcome value is irrelevant.
            outcome: Arc::new(Mutex::new(ProviderOutcome::NotFound)),
            panic_on_call: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    async fn fetch(
        &self,
        _work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if self.panic_on_call {
            panic!(
                "StubProviderClient panic-on-call: provider={:?}",
                self.provider
            );
        }
        self.outcome.lock().unwrap().clone()
    }
}

/// Real-network Audnexus adapter — the Phase 1.5 tracer. Wraps the lifted
/// `livrarr_metadata::audnexus::query_audnexus` and maps its return value
/// onto `ProviderOutcome<NormalizedWorkDetail>`.
///
/// Outcome mapping:
///   - `Ok(Some(_))` → `Success(payload)` populated with narrators / runtime / asin.
///   - `Ok(None)` → `NotFound`.
///   - `Err(_)` → `WillRetry { reason: ServerError, next_attempt_at: now + 5min }`.
///     Audnexus's stringified errors don't discriminate timeout / 5xx / DNS;
///     a coarser classification can land alongside the rest of the cutover when
///     each provider's failure taxonomy gets pulled into typed errors.
#[derive(Clone)]
pub struct AudnexusClient {
    http: HttpClient,
    base_url: String,
    retry_backoff_secs: i64,
}

impl AudnexusClient {
    pub fn new(http: HttpClient, base_url: impl Into<String>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            retry_backoff_secs: 5 * 60,
        }
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    async fn fetch(
        &self,
        work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let result = query_audnexus(
            &self.http,
            &self.base_url,
            work.asin.as_deref(),
            &work.title,
            &work.author_name,
        )
        .await;

        match result {
            Ok(Some(audnexus)) => {
                let narrator = if audnexus.narrators_empty {
                    None
                } else {
                    Some(audnexus.narrators)
                };
                let mut payload = NormalizedWorkDetail {
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
                    duration_seconds: audnexus.duration_seconds,
                    publisher: None,
                    publish_date: None,
                    hc_key: None,
                    gr_key: None,
                    ol_key: None,
                    isbn_13: None,
                    asin: audnexus.asin.clone(),
                    narrator,
                    // Legacy parity: a non-empty narrators list implies human
                    // narration (Audnexus doesn't expose narration_type explicitly).
                    narration_type: if audnexus.narrators_empty {
                        None
                    } else {
                        Some(livrarr_domain::NarrationType::Human)
                    },
                    abridged: None,
                    rating: None,
                    rating_count: None,
                    cover_url: None,
                    additional_isbns: Vec::new(),
                    additional_asins: Vec::new(),
                };
                if let Some(asin) = audnexus.asin {
                    payload.additional_asins.push(asin);
                }
                ProviderOutcome::Success(Box::new(payload))
            }
            Ok(None) => ProviderOutcome::NotFound,
            Err(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }
}

/// Real-network Hardcover adapter. Wraps `livrarr_metadata::hardcover::query_hardcover`
/// and maps its return value onto `ProviderOutcome<NormalizedWorkDetail>`.
///
/// Holds a clone of `MetadataConfig` because the inner query consults
/// `llm_enabled` / `llm_endpoint` / `llm_api_key` / `llm_model` for the Tier 2
/// disambiguation fallback. The orchestration cutover may rework that path so
/// the LLM fan-out happens through `MetadataProvider::Llm` instead — until then,
/// HC owns its own LLM call.
#[derive(Clone)]
pub struct HardcoverClient {
    http: HttpClient,
    /// Reads `hardcover_enabled` + `hardcover_api_token` per fetch — config
    /// changes via UI take effect on the next enrichment without restart.
    /// Also exposes `llm_*` fields for the inner llm_disambiguate fallback.
    live_config: crate::live_config::LiveMetadataConfig,
    retry_backoff_secs: i64,
}

impl HardcoverClient {
    pub fn new(http: HttpClient, live_config: crate::live_config::LiveMetadataConfig) -> Self {
        Self {
            http,
            live_config,
            retry_backoff_secs: 5 * 60,
        }
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    async fn fetch(
        &self,
        work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let cfg = self.live_config.snapshot();
        if !cfg.hardcover_enabled {
            return ProviderOutcome::NotConfigured;
        }
        let token = match cfg
            .hardcover_api_token
            .as_deref()
            .map(|t| {
                t.trim()
                    .trim_start_matches("Bearer ")
                    .trim_start_matches("bearer ")
            })
            .filter(|t| !t.is_empty())
        {
            Some(t) => t.to_string(),
            None => return ProviderOutcome::NotConfigured,
        };

        let result = query_hardcover(
            &self.http,
            &work.title,
            &work.author_name,
            &token,
            cfg.as_ref(),
        )
        .await;

        match result {
            Ok(hc) => {
                // Legacy parity: derive year from publish_date (YYYY prefix).
                let year = hc
                    .publish_date
                    .as_deref()
                    .and_then(|d| d.get(..4))
                    .and_then(|y| y.parse::<i32>().ok());

                // Legacy parity: when the search hit yielded an hc_key, fetch the
                // editions list and prefer an English-language edition's ISBN-13
                // over whatever the search result returned. The search result's
                // ISBN often points at a non-English or sub-optimal edition.
                let mut isbn_13 = hc.isbn_13.clone();
                if let Some(ref hc_id) = hc.hc_key {
                    if let Ok(Some(better_isbn)) =
                        crate::hardcover::fetch_hardcover_editions(&self.http, hc_id, &token, "en")
                            .await
                    {
                        isbn_13 = Some(better_isbn);
                    }
                }

                let payload = NormalizedWorkDetail {
                    title: hc.title,
                    subtitle: hc.subtitle,
                    original_title: hc.original_title,
                    author_name: None,
                    description: hc.description,
                    year,
                    series_name: hc.series_name,
                    series_position: hc.series_position,
                    genres: hc.genres,
                    language: None,
                    page_count: hc.page_count,
                    duration_seconds: None,
                    publisher: hc.publisher,
                    publish_date: hc.publish_date,
                    hc_key: hc.hc_key,
                    gr_key: None,
                    ol_key: None,
                    isbn_13,
                    asin: None,
                    narrator: None,
                    narration_type: None,
                    abridged: None,
                    rating: hc.rating,
                    rating_count: hc.rating_count,
                    cover_url: hc.cover_url,
                    additional_isbns: Vec::new(),
                    additional_asins: Vec::new(),
                };
                ProviderOutcome::Success(Box::new(payload))
            }
            // Discriminate HC's stringified errors. "No results" / "no exact
            // match" mean HC genuinely doesn't have this book — those are
            // NotFound, NOT a provider failure. Treating them as WillRetry
            // counts them toward the breaker and trips it after 5 consecutive
            // misses, suppressing HC for the rest of the bulk run. Real HTTP /
            // network / parse errors stay WillRetry.
            //
            // Proper fix is typed errors out of query_hardcover; until then,
            // string matching keeps the breaker honest.
            Err(
                crate::hardcover::HardcoverError::NoResults
                | crate::hardcover::HardcoverError::NoMatch(_),
            ) => ProviderOutcome::NotFound,
            Err(crate::hardcover::HardcoverError::Http(_)) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }
}

/// Real-network OpenLibrary adapter. Wraps
/// `livrarr_metadata::openlibrary::query_ol_detail`. OL detail fetch is keyed on
/// `work.ol_key`; works without an `ol_key` are reported as `NotFound` without
/// hitting the network.
#[derive(Clone)]
pub struct OpenLibraryClient {
    http: HttpClient,
    retry_backoff_secs: i64,
}

impl OpenLibraryClient {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
            retry_backoff_secs: 5 * 60,
        }
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    async fn fetch(
        &self,
        work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let ol_key = match work.ol_key.as_deref().filter(|s| !s.is_empty()) {
            Some(k) => k,
            None => return ProviderOutcome::NotFound,
        };

        match query_ol_detail(&self.http, ol_key).await {
            Ok(detail) => {
                let payload = NormalizedWorkDetail {
                    title: None,
                    subtitle: None,
                    original_title: None,
                    author_name: None,
                    description: detail.description,
                    year: None,
                    series_name: None,
                    series_position: None,
                    genres: None,
                    language: None,
                    page_count: None,
                    duration_seconds: None,
                    publisher: None,
                    publish_date: None,
                    hc_key: None,
                    gr_key: None,
                    ol_key: Some(ol_key.to_string()),
                    isbn_13: detail.isbn_13,
                    asin: None,
                    narrator: None,
                    narration_type: None,
                    abridged: None,
                    rating: None,
                    rating_count: None,
                    cover_url: None,
                    additional_isbns: Vec::new(),
                    additional_asins: Vec::new(),
                };
                ProviderOutcome::Success(Box::new(payload))
            }
            Err(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }
}

/// Real-network Goodreads adapter. Wraps the lifted
/// `livrarr_metadata::goodreads::{search_goodreads, fetch_goodreads_detail}`
/// helpers and maps their errors onto `ProviderOutcome<NormalizedWorkDetail>`.
///
/// Resolution order:
///   1. If `work.gr_key` is populated, fetch the detail page directly
///      (skips a search round-trip — see R-21 canonical-identity policy).
///   2. Otherwise, search by `title author`; on empty results, retry once
///      with non-ASCII characters stripped from the title (legacy parity for
///      titles with diacritics). Take the first hit.
///   3. Resolve the search hit's (often relative) `detail_url` against
///      `base_url` and fetch the detail page.
///
/// Outcome mapping:
///   - Detail page parsed → `Success(payload)` with cover_url, description,
///     series, genres, year (derived from publish_date), rating, etc.
///   - Empty search results / no `parse_detail_html` output → `NotFound`.
///   - `GoodreadsFetchError::AntiBot` → `WillRetry { AntiBotBlock }` per IR
///     (anti-bot challenges are typically transient/IP-based).
///   - HTTP 429 → `WillRetry { RateLimit }`.
///   - HTTP 5xx / network / body-read failures → `WillRetry { ServerError }`.
///   - HTTP 4xx (other than 429) → `NotFound` (typically 404 on a stale URL).
///   - Detail page returned 200 OK but unparseable → `NotFound`.
#[derive(Clone)]
pub struct GoodreadsClient {
    http: HttpClient,
    base_url: String,
    retry_backoff_secs: i64,
    /// Reads `llm_*` per fetch — the LLM extraction fallback for
    /// foreign-language pages activates whenever live config has LLM
    /// configured. None means the client wasn't given a live-config handle
    /// (test / smoke-test path); LLM fallback disabled.
    live_config: Option<crate::live_config::LiveMetadataConfig>,
}

impl GoodreadsClient {
    pub fn new(http: HttpClient, base_url: impl Into<String>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            retry_backoff_secs: 5 * 60,
            live_config: None,
        }
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    /// Enable the LLM extraction fallback by giving the client a handle to
    /// the shared live config. The client reads `llm_*` per fetch, so config
    /// changes (enable/disable, key/model swap) take effect on the next
    /// enrichment call without restart.
    pub fn with_live_config(mut self, live_config: crate::live_config::LiveMetadataConfig) -> Self {
        self.live_config = Some(live_config);
        self
    }

    async fn fetch(
        &self,
        work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let detail_url = match self.resolve_detail_url(work).await {
            Ok(Some(url)) => url,
            Ok(None) => return ProviderOutcome::NotFound,
            Err(err) => return self.map_fetch_err(err),
        };

        // Direct parse path. On Parse failure, optionally fall through to
        // LLM extraction if configured (typical for foreign-language pages
        // where JSON-LD / regex don't match the locale-specific HTML).
        let html = match goodreads::fetch_goodreads_html(&self.http, &detail_url).await {
            Ok(h) => h,
            Err(err) => return self.map_fetch_err(err),
        };

        if let Some(detail) = goodreads::parse_detail_html(&html) {
            return ProviderOutcome::Success(Box::new(self.normalize(&detail_url, detail)));
        }

        // Direct parse yielded nothing. Try LLM extraction when live config
        // has LLM enabled + key + endpoint set.
        if let Some(live) = &self.live_config {
            let cfg = live.snapshot();
            let key = cfg
                .llm_api_key
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let endpoint = cfg
                .llm_endpoint
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            if let (true, Some(endpoint), Some(key)) = (cfg.llm_enabled, endpoint, key) {
                let model = cfg
                    .llm_model
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("gemini-3.1-flash-lite-preview");
                let language_hint = work
                    .language
                    .as_deref()
                    .and_then(crate::language::get_language_info)
                    .map(|info| info.english_name)
                    .unwrap_or("the original");
                match goodreads::extract_with_llm(
                    &self.http,
                    endpoint,
                    key,
                    model,
                    &html,
                    language_hint,
                )
                .await
                {
                    Ok(mut payload) => {
                        // Normalize gr_key from the resolved URL — the LLM
                        // doesn't know our identifier scheme.
                        if payload.gr_key.is_none() {
                            payload.gr_key = goodreads::extract_gr_key(&detail_url);
                        }
                        return ProviderOutcome::Success(Box::new(payload));
                    }
                    Err(GoodreadsFetchError::Parse) => return ProviderOutcome::NotFound,
                    Err(err) => return self.map_fetch_err(err),
                }
            }
        }

        // No LLM configured + direct parse failed → NotFound.
        ProviderOutcome::NotFound
    }

    async fn resolve_detail_url(&self, work: &Work) -> Result<Option<String>, GoodreadsFetchError> {
        // Priority order:
        //   1. work.gr_key — canonical GR identity (R-21).
        //   2. work.detail_url — typically set on foreign-work add (the GR
        //      URL the user picked from the foreign search results).
        //   3. Search by title+author with ASCII-strip diacritics fallback.
        if let Some(key) = work.gr_key.as_deref().filter(|k| !k.is_empty()) {
            return Ok(Some(goodreads::detail_url_for_gr_key(&self.base_url, key)));
        }

        if let Some(url) = work.detail_url.as_deref().filter(|u| !u.is_empty()) {
            // Validate it's a Goodreads URL (SSRF guard against stale data
            // pointing somewhere unexpected).
            if goodreads::validate_detail_url(url) {
                let resolved = goodreads::resolve_detail_url(&self.base_url, url);
                return Ok(Some(resolved));
            }
        }

        let title = &work.title;
        let author = &work.author_name;
        let mut hits =
            goodreads::search_goodreads(&self.http, &self.base_url, title, author).await?;

        if hits.is_empty() && !title.is_ascii() {
            // Legacy parity: titles with diacritics often miss in GR's search;
            // retry once with a stripped-ASCII title.
            let ascii_title: String = title.chars().filter(|c| c.is_ascii()).collect();
            if !ascii_title.trim().is_empty() {
                hits =
                    goodreads::search_goodreads(&self.http, &self.base_url, &ascii_title, author)
                        .await?;
            }
        }

        Ok(hits
            .into_iter()
            .next()
            .map(|top| goodreads::resolve_detail_url(&self.base_url, &top.detail_url)))
    }

    fn map_fetch_err(&self, err: GoodreadsFetchError) -> ProviderOutcome<NormalizedWorkDetail> {
        let backoff = chrono::Duration::seconds(self.retry_backoff_secs);
        match err {
            GoodreadsFetchError::AntiBot => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::AntiBotBlock,
                next_attempt_at: Utc::now() + backoff,
            },
            GoodreadsFetchError::HttpStatus(429) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::RateLimit,
                next_attempt_at: Utc::now() + backoff,
            },
            GoodreadsFetchError::HttpStatus(code) if (500..600).contains(&code) => {
                ProviderOutcome::WillRetry {
                    reason: livrarr_domain::WillRetryReason::ServerError,
                    next_attempt_at: Utc::now() + backoff,
                }
            }
            // 4xx other than 429: stale URL, deleted page, etc. — treat as
            // NotFound rather than burning retries against a permanent miss.
            GoodreadsFetchError::HttpStatus(_) => ProviderOutcome::NotFound,
            GoodreadsFetchError::Network(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + backoff,
            },
            GoodreadsFetchError::Parse => ProviderOutcome::NotFound,
        }
    }

    fn normalize(&self, detail_url: &str, detail: GoodreadsDetailResult) -> NormalizedWorkDetail {
        let year = detail
            .publish_date
            .as_deref()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse::<i32>().ok());
        let gr_key = goodreads::extract_gr_key(detail_url);
        let isbn_13 = detail.isbn.filter(|s| s.len() >= 10);
        let cover_url = detail
            .cover_url
            .filter(|u| goodreads::validate_cover_url(u))
            .map(|u| crate::cover::upscale_cover_url(&u));
        let genres = if detail.genres.is_empty() {
            None
        } else {
            Some(detail.genres)
        };

        NormalizedWorkDetail {
            title: detail.title,
            subtitle: None,
            original_title: None,
            author_name: detail.author,
            description: detail.description,
            year,
            series_name: detail.series_name,
            series_position: detail.series_position,
            genres,
            language: detail.language,
            page_count: detail.page_count.filter(|&p| p > 0),
            duration_seconds: None,
            publisher: None,
            publish_date: detail.publish_date,
            hc_key: None,
            gr_key,
            ol_key: None,
            isbn_13,
            asin: None,
            narrator: None,
            narration_type: None,
            abridged: None,
            rating: detail.rating,
            rating_count: detail.rating_count,
            cover_url,
            additional_isbns: Vec::new(),
            additional_asins: Vec::new(),
        }
    }
}

/// Construct a `GoodreadsClient` against the production Goodreads URL.
impl GoodreadsClient {
    pub fn production(http: HttpClient) -> Self {
        Self::new(http, GOODREADS_BASE_URL)
    }
}

#[cfg(test)]
mod audnexus_tracer_tests {
    //! End-to-end smoke test of `ProviderClient::Audnexus` through
    //! `DefaultProviderQueue` against a hand-rolled local HTTP server.
    //!
    //! Purpose: validate that the trait shape (`ProviderClient` enum +
    //! `ProviderQueue::dispatch_enrichment`) actually holds against real
    //! `reqwest`/`HttpClient`/`tokio` plumbing — not just stub clients.
    //! If this compiles and passes, the trait is sound for the rest of the
    //! cutover (Hardcover, OpenLibrary, Goodreads, LLM).

    use super::*;
    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::{CircuitBreakerConfig, EnrichmentMode, ProviderQueue, ProviderQueueConfig};
    use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb};
    use livrarr_domain::{MetadataProvider, RequestPriority, UserRole};
    use livrarr_http::HttpClient;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_canned_audnexus_server(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            // Read until end of headers — single read is enough for these tiny GETs.
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        });
        url
    }

    fn audnexus_config() -> ProviderQueueConfig {
        ProviderQueueConfig {
            provider: MetadataProvider::Audnexus,
            concurrency: 1,
            requests_per_second: 1.0,
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 3,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
            max_attempts: 3,
            max_suppressed_passes: 3,
            max_suppression_window_secs: 3600,
        }
    }

    async fn seed_db_and_work() -> (livrarr_db::sqlite::SqliteDb, livrarr_domain::Work) {
        let db = livrarr_db::create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "tracer_user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey".to_string(),
            })
            .await
            .unwrap()
            .id;
        let work = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Tracer Audiobook".to_string(),
                author_name: "Tracer Author".to_string(),
                author_id: None,
                ol_key: None,
                year: Some(2024),
                cover_url: None,
                ..Default::default()
            })
            .await
            .unwrap();
        (db, work)
    }

    #[tokio::test]
    async fn audnexus_through_queue_returns_success_for_canned_response() {
        let body = serde_json::json!({
            "asin": "B07TRACER01",
            "narrators": [{"name": "Sample Narrator"}],
            "runtimeLengthSec": 12345
        })
        .to_string();
        let url = spawn_canned_audnexus_server(body).await;

        let (db, work) = seed_db_and_work().await;
        let http = HttpClient::builder().build().unwrap();
        let client = AudnexusClient::new(http, url);

        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(
                MetadataProvider::Audnexus,
                ProviderClient::Audnexus(client),
                audnexus_config(),
            )
            .build(Arc::new(db));

        let ctx = EnrichmentContext {
            priority: RequestPriority::Low,
            mode: EnrichmentMode::Background,
        };

        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();

        let outcome = result
            .outcomes
            .get(&MetadataProvider::Audnexus)
            .expect("Audnexus must appear in scatter-gather outcomes");
        match outcome {
            ProviderOutcome::Success(payload) => {
                assert_eq!(payload.asin.as_deref(), Some("B07TRACER01"));
                assert_eq!(payload.duration_seconds, Some(12345));
                let narrators = payload
                    .narrator
                    .as_ref()
                    .expect("narrators must be populated for a successful Audnexus hit");
                assert_eq!(narrators, &vec!["Sample Narrator".to_string()]);
            }
            other => panic!("expected Success outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn audnexus_through_queue_returns_will_retry_when_endpoint_unreachable() {
        // Bind, immediately drop — port is observed-then-closed; reqwest will fail
        // to connect. Forces the WillRetry error path through the trait.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        drop(listener);

        let (db, work) = seed_db_and_work().await;
        let http = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let client = AudnexusClient::new(http, url);

        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(
                MetadataProvider::Audnexus,
                ProviderClient::Audnexus(client),
                audnexus_config(),
            )
            .build(Arc::new(db));

        let ctx = EnrichmentContext {
            priority: RequestPriority::Low,
            mode: EnrichmentMode::Background,
        };

        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();
        let outcome = result.outcomes.get(&MetadataProvider::Audnexus).unwrap();
        assert!(
            matches!(outcome, ProviderOutcome::WillRetry { .. }),
            "expected WillRetry on unreachable endpoint, got {outcome:?}"
        );
    }
}

#[cfg(test)]
mod goodreads_tracer_tests {
    //! End-to-end smoke test of `ProviderClient::Goodreads` through
    //! `DefaultProviderQueue` against a hand-rolled local HTTP server.
    //!
    //! Mirrors the Audnexus tracer pattern. Two scenarios:
    //!   - direct gr_key lookup against a canned JSON-LD detail page → Success
    //!   - anti-bot challenge body → WillRetry { AntiBotBlock } per IR

    use super::*;
    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::{CircuitBreakerConfig, EnrichmentMode, ProviderQueue, ProviderQueueConfig};
    use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb};
    use livrarr_domain::{MetadataProvider, RequestPriority, UserRole, WillRetryReason};
    use livrarr_http::HttpClient;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_canned_html_server(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            // Single read is enough for these tiny GETs (request line + headers fit easily).
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        });
        url
    }

    fn goodreads_config() -> ProviderQueueConfig {
        ProviderQueueConfig {
            provider: MetadataProvider::Goodreads,
            concurrency: 1,
            requests_per_second: 1.0,
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 3,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
            max_attempts: 3,
            max_suppressed_passes: 3,
            max_suppression_window_secs: 3600,
        }
    }

    async fn seed_db_and_work_with_gr_key(
        gr_key: Option<&str>,
    ) -> (livrarr_db::sqlite::SqliteDb, livrarr_domain::Work) {
        let db = livrarr_db::create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "gr_tracer_user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey".to_string(),
            })
            .await
            .unwrap()
            .id;
        let work = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Tracer Book".to_string(),
                author_name: "Tracer Author".to_string(),
                author_id: None,
                ol_key: None,
                year: Some(2024),
                cover_url: None,
                gr_key: gr_key.map(|s| s.to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        (db, work)
    }

    /// Minimal GR detail HTML — JSON-LD Book block with the fields
    /// `parse_detail_html` extracts.
    fn canned_detail_html() -> String {
        r#"<html><head>
<script type="application/ld+json">{
  "@context": "https://schema.org",
  "@type": "Book",
  "name": "Tracer Book",
  "author": [{"@type":"Person","name":"Tracer Author"}],
  "isbn": "9781234567890",
  "numberOfPages": 321,
  "inLanguage": "en",
  "image": "https://i.gr-assets.com/images/S/compressed.photo.goodreads.com/books/1700000000l/12345.jpg",
  "aggregateRating": {"@type":"AggregateRating","ratingValue":4.25,"ratingCount":9876}
}</script>
</head><body>Anything goes here.</body></html>"#
            .to_string()
    }

    #[tokio::test]
    async fn goodreads_through_queue_returns_success_for_direct_gr_key_lookup() {
        let url = spawn_canned_html_server(canned_detail_html()).await;

        let (db, work) = seed_db_and_work_with_gr_key(Some("12345.Tracer_Book")).await;
        let http = HttpClient::builder().build().unwrap();
        let client = GoodreadsClient::new(http, url);

        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(
                MetadataProvider::Goodreads,
                ProviderClient::Goodreads(client),
                goodreads_config(),
            )
            .build(Arc::new(db));

        let ctx = EnrichmentContext {
            priority: RequestPriority::Low,
            mode: EnrichmentMode::Background,
        };

        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();
        let outcome = result
            .outcomes
            .get(&MetadataProvider::Goodreads)
            .expect("Goodreads must appear in scatter-gather outcomes");
        match outcome {
            ProviderOutcome::Success(payload) => {
                assert_eq!(payload.title.as_deref(), Some("Tracer Book"));
                assert_eq!(payload.author_name.as_deref(), Some("Tracer Author"));
                assert_eq!(payload.isbn_13.as_deref(), Some("9781234567890"));
                assert_eq!(payload.page_count, Some(321));
                assert_eq!(payload.language.as_deref(), Some("en"));
                assert_eq!(payload.gr_key.as_deref(), Some("12345.Tracer_Book"));
                assert!(
                    payload
                        .cover_url
                        .as_deref()
                        .is_some_and(|u| u.contains("gr-assets.com")),
                    "cover_url should pass validate_cover_url, got {:?}",
                    payload.cover_url
                );
                assert!(
                    (payload.rating.unwrap_or(0.0) - 4.25).abs() < 0.001,
                    "rating mismatch: {:?}",
                    payload.rating
                );
                assert_eq!(payload.rating_count, Some(9876));
            }
            other => panic!("expected Success outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn goodreads_through_queue_returns_will_retry_anti_bot_on_challenge_page() {
        // Small (< 10KB) body containing an anti-bot indicator — triggers
        // `is_anti_bot_page`. The lifted `fetch_goodreads_html` maps that to
        // `GoodreadsFetchError::AntiBot`, which `GoodreadsClient::fetch` maps
        // to WillRetry { AntiBotBlock }.
        let body = r#"<html><head><title>Just a moment</title></head>
<body><div class="cf-browser-verification">Checking your browser...</div></body></html>"#
            .to_string();
        let url = spawn_canned_html_server(body).await;

        let (db, work) = seed_db_and_work_with_gr_key(Some("99999.Blocked")).await;
        let http = HttpClient::builder().build().unwrap();
        let client = GoodreadsClient::new(http, url);

        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(
                MetadataProvider::Goodreads,
                ProviderClient::Goodreads(client),
                goodreads_config(),
            )
            .build(Arc::new(db));

        let ctx = EnrichmentContext {
            priority: RequestPriority::Low,
            mode: EnrichmentMode::Background,
        };

        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();
        let outcome = result.outcomes.get(&MetadataProvider::Goodreads).unwrap();
        match outcome {
            ProviderOutcome::WillRetry { reason, .. } => {
                assert_eq!(*reason, WillRetryReason::AntiBotBlock);
            }
            other => panic!("expected WillRetry {{ AntiBotBlock }}, got {other:?}"),
        }
    }
}
