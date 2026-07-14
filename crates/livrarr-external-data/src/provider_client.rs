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
use livrarr_domain::services::{
    CallOperation, CallOutcomeClass, FetchRequest, HttpFetcher, HttpMethod, ProviderCallRecord,
    ProviderCallSink, RateBucket, UserAgentProfile,
};
use livrarr_domain::{AnchorQuery, MetadataProvider, RequestPriority, WillRetryReason, Work};
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use livrarr_http::HttpClient;
use std::time::Duration;

use crate::audnexus::{query_audnexus, query_audnexus_by_asin, AudnexusCache, AudnexusResult};
use crate::goodreads::{self, GoodreadsDetailResult, GoodreadsFetchError, GOODREADS_BASE_URL};
use crate::hardcover::query_hardcover;
use crate::openlibrary::query_ol_detail;
use crate::{NormalizedWorkDetail, ProviderOutcome};

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
    GoogleBooks(crate::google_books::GoogleBooksClient),
    Audible(crate::audible::AudibleCatalogClient),
}

impl ProviderClient {
    /// `priority` is the caller's queue-ordering hint (B4): the identity
    /// fan-out and other lookup/discovery callers pass their own
    /// `RequestPriority` through to whatever transport request this ends up
    /// making. Providers with no live request on this surface (e.g. a
    /// key-direct path) simply ignore it.
    pub async fn fetch(
        &self,
        work: &Work,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match self {
            Self::Stub(s) => {
                *s.last_priority.lock().unwrap() = Some(priority);
                s.fetch(work).await
            }
            Self::Audnexus(a) => a.fetch(work, priority).await,
            Self::Hardcover(h) => h.fetch(work, priority).await,
            Self::OpenLibrary(o) => o.fetch(work, priority).await,
            Self::Goodreads(g) => g.fetch(work, priority).await,
            Self::GoogleBooks(g) => g.fetch(work, priority).await,
            Self::Audible(a) => a.fetch(work, priority).await,
        }
    }

    /// Anchor-only enrichment fetch (REQ-006). Each provider accepts exactly
    /// the spec's anchor mapping — GoogleBooks: Isbn13; Goodreads: GrKey ONLY
    /// (the GR ISBN/title-search tier is an identity-surface capability, never an
    /// enrichment fetch); Hardcover: HcKey or Isbn13; OpenLibrary: OlKey or
    /// Isbn13; Audnexus/Audible: Asin. No branch falls back to text search —
    /// those paths live only behind the lookup/identity entry points. A
    /// (provider, query-kind) mismatch is NotFound + warn. Emits one
    /// Enrich-operation ProviderCallRecord per call through the injected sink.
    pub async fn fetch_by_anchor(
        &self,
        query: AnchorQuery,
        language: Option<&str>,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let provider = self.provider();
        if !anchor_kind_accepted(provider, &query) {
            tracing::warn!(
                provider = provider.record_key(),
                "fetch_by_anchor: anchor kind not accepted by this provider — \
                 queue derivation bug; returning NotFound"
            );
            return ProviderOutcome::NotFound;
        }
        let started_at = Utc::now();
        let t0 = std::time::Instant::now();
        let outcome = match (self, &query) {
            (Self::Stub(s), _) => {
                *s.last_priority.lock().unwrap() = Some(priority);
                s.fire().await
            }
            (Self::Audnexus(a), AnchorQuery::Asin(asin)) => a.fetch_by_asin(asin, priority).await,
            (Self::Hardcover(h), q) => h.fetch_by_anchor_query(q, priority).await,
            (Self::OpenLibrary(o), q) => o.fetch_by_anchor_query(q, priority).await,
            (Self::Goodreads(g), AnchorQuery::GrKey(key)) => {
                g.fetch_detail_by_key(key, language, priority).await
            }
            (Self::GoogleBooks(g), AnchorQuery::Isbn13(isbn)) => {
                g.fetch_by_isbn(isbn, priority).await
            }
            (Self::Audible(a), AnchorQuery::Asin(asin)) => a.fetch_by_asin(asin, priority).await,
            // Unreachable: the acceptance gate above filtered every other pairing.
            _ => return ProviderOutcome::NotFound,
        };
        if let Some(sink) = self.sink_ref() {
            let (class, detail) = outcome_record_class(&outcome);
            sink.record(ProviderCallRecord {
                provider: provider.record_key().to_string(),
                operation: CallOperation::Enrich,
                work_id: None,
                started_at,
                duration_ms: t0.elapsed().as_millis() as i64,
                outcome: class,
                detail,
            });
        }
        outcome
    }

    fn sink_ref(&self) -> Option<&Arc<dyn ProviderCallSink>> {
        match self {
            Self::Stub(s) => s.call_sink.as_ref(),
            Self::Audnexus(a) => a.call_sink.as_ref(),
            Self::Hardcover(h) => h.call_sink.as_ref(),
            Self::OpenLibrary(o) => o.call_sink.as_ref(),
            Self::Goodreads(g) => g.call_sink.as_ref(),
            Self::GoogleBooks(g) => g.sink_ref(),
            Self::Audible(a) => a.sink_ref(),
        }
    }

    /// Inject the call-record sink (REQ-001) into the wrapped client.
    /// Stub clients record through the same central wrapper as real clients when a sink is injected.
    pub fn with_call_sink(self, sink: Arc<dyn ProviderCallSink>) -> Self {
        match self {
            Self::Stub(c) => Self::Stub(c.with_call_sink(sink)),
            Self::Audnexus(c) => Self::Audnexus(c.with_call_sink(sink)),
            Self::Hardcover(c) => Self::Hardcover(c.with_call_sink(sink)),
            Self::OpenLibrary(c) => Self::OpenLibrary(c.with_call_sink(sink)),
            Self::Goodreads(c) => Self::Goodreads(c.with_call_sink(sink)),
            Self::GoogleBooks(c) => Self::GoogleBooks(c.with_call_sink(sink)),
            Self::Audible(c) => Self::Audible(c.with_call_sink(sink)),
        }
    }

    pub fn provider(&self) -> MetadataProvider {
        match self {
            Self::Stub(s) => s.provider,
            Self::Audnexus(_) => MetadataProvider::Audnexus,
            Self::Hardcover(_) => MetadataProvider::Hardcover,
            Self::OpenLibrary(_) => MetadataProvider::OpenLibrary,
            Self::Goodreads(_) => MetadataProvider::Goodreads,
            Self::GoogleBooks(_) => MetadataProvider::GoogleBooks,
            Self::Audible(_) => MetadataProvider::Audible,
        }
    }

    pub fn call_count(&self) -> usize {
        match self {
            Self::Stub(s) => s.call_count(),
            Self::Audnexus(_)
            | Self::Hardcover(_)
            | Self::OpenLibrary(_)
            | Self::Goodreads(_)
            | Self::GoogleBooks(_)
            | Self::Audible(_) => 0,
        }
    }
}

/// REQ-006 anchor mapping: which anchor kinds each provider's enrichment
/// surface accepts. GR is GrKey ONLY (its ISBN/title-search tier is an identity-surface
/// capability); HC/OL accept their own key or ISBN; Audnexus/Audible are
/// ASIN-keyed; GB is ISBN-keyed. Llm/Readarr are never scatter providers.
fn anchor_kind_accepted(provider: MetadataProvider, query: &AnchorQuery) -> bool {
    matches!(
        (provider, query),
        (MetadataProvider::GoogleBooks, AnchorQuery::Isbn13(_))
            | (MetadataProvider::Goodreads, AnchorQuery::GrKey(_))
            | (
                MetadataProvider::Hardcover,
                AnchorQuery::HcKey(_) | AnchorQuery::Isbn13(_)
            )
            | (
                MetadataProvider::OpenLibrary,
                AnchorQuery::OlKey(_) | AnchorQuery::Isbn13(_)
            )
            | (
                MetadataProvider::Audnexus | MetadataProvider::Audible,
                AnchorQuery::Asin(_)
            )
    )
}

/// Common `WillRetry { CircuitOpen }` mapping (R-11 / Step 4): every provider
/// client that detects `ProviderFetchError::CircuitOpen` /
/// `FetchError::CircuitOpen` maps it through this one helper, never to
/// `RateLimit` (corrupts retry accounting).
fn circuit_open_outcome(retry_after: Duration) -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::WillRetry {
        reason: WillRetryReason::CircuitOpen,
        next_attempt_at: Utc::now()
            + chrono::Duration::from_std(retry_after)
                .unwrap_or_else(|_| chrono::Duration::seconds(60)),
    }
}

/// REQ-001 outcome mapping: ProviderOutcome (the control-flow vocabulary) →
/// CallOutcomeClass (the reporting vocabulary), explicit per variant.
/// Conflict never originates from a client fetch; it maps to Error
/// with a detail tag rather than a catch-all.
fn outcome_record_class(
    outcome: &ProviderOutcome<NormalizedWorkDetail>,
) -> (CallOutcomeClass, Option<String>) {
    match outcome {
        ProviderOutcome::Success(_) => (CallOutcomeClass::Success, None),
        ProviderOutcome::NotFound => (CallOutcomeClass::NotFound, None),
        ProviderOutcome::NotConfigured => (
            CallOutcomeClass::SkippedPolicy,
            Some("not_configured".to_string()),
        ),
        ProviderOutcome::WillRetry { reason, .. } => match reason {
            WillRetryReason::Timeout => (CallOutcomeClass::Timeout, None),
            WillRetryReason::RateLimit => (CallOutcomeClass::RateLimited, None),
            WillRetryReason::AntiBotBlock => (
                CallOutcomeClass::RateLimited,
                Some("anti_bot_block".to_string()),
            ),
            WillRetryReason::ServerError => {
                (CallOutcomeClass::Error, Some("server_error".to_string()))
            }
            // Observability only — mirrors the queue's existing
            // `record_queue_skip` precedent of tagging a pacing/breaker skip
            // as RateLimited with a `detail` string.
            WillRetryReason::CircuitOpen => (
                CallOutcomeClass::RateLimited,
                Some("circuit_open".to_string()),
            ),
        },
        ProviderOutcome::PermanentFailure { reason } => {
            (CallOutcomeClass::Error, Some(format!("{reason:?}")))
        }
        ProviderOutcome::Conflict { .. } => (CallOutcomeClass::Error, Some("conflict".to_string())),
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
    /// Optional fetch delay so tests can drive the resolver's per-call timeout.
    delay: Option<std::time::Duration>,
    /// Last `RequestPriority` this stub received via either `fetch` (the
    /// identity fan-out) or `fetch_by_anchor` (the enrichment queue) — B4.
    /// `None` until a call happens.
    last_priority: Arc<Mutex<Option<RequestPriority>>>,
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl StubProviderClient {
    pub fn new(provider: MetadataProvider, outcome: ProviderOutcome<NormalizedWorkDetail>) -> Self {
        Self {
            provider,
            outcome: Arc::new(Mutex::new(outcome)),
            panic_on_call: false,
            call_count: Arc::new(AtomicUsize::new(0)),
            delay: None,
            last_priority: Arc::new(Mutex::new(None)),
            call_sink: None,
        }
    }

    pub fn with_panic(provider: MetadataProvider) -> Self {
        Self {
            provider,
            // Panic before the lock is touched, so the outcome value is irrelevant.
            outcome: Arc::new(Mutex::new(ProviderOutcome::NotFound)),
            panic_on_call: true,
            call_count: Arc::new(AtomicUsize::new(0)),
            delay: None,
            last_priority: Arc::new(Mutex::new(None)),
            call_sink: None,
        }
    }

    /// Make `fetch` sleep before returning, so a test can exceed the resolver's
    /// `call_timeout` and exercise the abstention path (REQ-025).
    pub fn with_delay(mut self, delay: std::time::Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn with_call_sink(mut self, sink: Arc<dyn ProviderCallSink>) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    /// The `RequestPriority` the most recent `fetch`/`fetch_by_anchor` call
    /// carried (B4) — lets a test assert a caller's priority actually
    /// reached the provider client, not just that a call happened.
    pub fn last_priority(&self) -> Option<RequestPriority> {
        *self.last_priority.lock().unwrap()
    }

    /// Shared scripted-call body for both fetch surfaces: counts the call,
    /// honors the configured delay/panic, returns the scripted outcome. The
    /// enum-level anchor-kind gate runs BEFORE this, so a mismatched
    /// fetch_by_anchor never increments the count (AC-007).
    async fn fire(&self) -> ProviderOutcome<NormalizedWorkDetail> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        if self.panic_on_call {
            panic!(
                "StubProviderClient panic-on-call: provider={:?}",
                self.provider
            );
        }
        self.outcome.lock().unwrap().clone()
    }

    async fn fetch(&self, _work: &Work) -> ProviderOutcome<NormalizedWorkDetail> {
        self.fire().await
    }
}

/// Real-network Audnexus adapter — the Phase 1.5 tracer. Wraps the lifted
/// `crate::audnexus::query_audnexus` and maps its return value
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
    fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    base_url: String,
    retry_backoff_secs: i64,
    cache: AudnexusCache,
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl AudnexusClient {
    pub fn new(
        fetcher: livrarr_http::fetcher::HttpFetcherImpl,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            fetcher,
            base_url: base_url.into(),
            retry_backoff_secs: 5 * 60,
            cache: crate::audnexus::AudnexusCache::new(),
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001).
    pub fn with_call_sink(mut self, sink: Arc<dyn ProviderCallSink>) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    async fn fetch(
        &self,
        work: &Work,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let result = query_audnexus(
            &self.fetcher,
            &self.base_url,
            work.asin.as_deref(),
            &work.title,
            &work.author_name,
            &self.cache,
            priority,
        )
        .await;

        match result {
            Ok(Some(audnexus)) => ProviderOutcome::Success(Box::new(audnexus_payload(audnexus))),
            Ok(None) => ProviderOutcome::NotFound,
            Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                circuit_open_outcome(retry_after)
            }
            Err(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }

    /// Anchor-only fetch (REQ-006): ASIN lookup with no title/author fallback.
    async fn fetch_by_asin(
        &self,
        asin: &str,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match query_audnexus_by_asin(&self.fetcher, &self.base_url, asin, &self.cache, priority)
            .await
        {
            Ok(Some(audnexus)) => ProviderOutcome::Success(Box::new(audnexus_payload(audnexus))),
            Ok(None) => ProviderOutcome::NotFound,
            Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                circuit_open_outcome(retry_after)
            }
            Err(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }
}

/// Map a parsed Audnexus hit onto the normalized payload shape (shared by the
/// work-seeded and anchor-only fetch surfaces).
fn audnexus_payload(audnexus: AudnexusResult) -> NormalizedWorkDetail {
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
        cover_url: audnexus.cover_url.clone(),
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    };
    if let Some(asin) = audnexus.asin {
        payload.additional_asins.push(asin);
    }
    payload
}

/// Real-network Hardcover adapter. Wraps `crate::hardcover::query_hardcover`
/// and maps its return value onto `ProviderOutcome<NormalizedWorkDetail>`.
///
/// Holds the shared `HttpFetcherImpl` (all HC HTTP rides the outbound queue)
/// and a `LiveMetadataConfig` handle read per fetch for `hardcover_enabled` +
/// `hardcover_api_token`, so config changes take effect without restart. The
/// title+author query is deterministic and two-tier (REQ-016/D10): exact
/// title + author-in-list match first, then the shared 0.75 title+author
/// picker; nothing clearing the bar means HC abstains (`NoMatch`) rather
/// than adopting a fuzzy hit.
#[derive(Clone)]
pub struct HardcoverClient {
    fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    /// Reads `hardcover_enabled` + `hardcover_api_token` per fetch — config
    /// changes via UI take effect on the next enrichment without restart.
    live_config: crate::live_config::LiveMetadataConfig,
    retry_backoff_secs: i64,
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl HardcoverClient {
    pub fn new(
        fetcher: livrarr_http::fetcher::HttpFetcherImpl,
        live_config: crate::live_config::LiveMetadataConfig,
    ) -> Self {
        Self {
            fetcher,
            live_config,
            retry_backoff_secs: 5 * 60,
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001).
    pub fn with_call_sink(mut self, sink: Arc<dyn ProviderCallSink>) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    /// Anchor-only fetch (REQ-006). ISBN is the working anchor tier; no
    /// by-hc_key detail query exists in the current Hardcover integration, so
    /// the HcKey arm is a recorded gap that reports NotFound rather than
    /// falling back to text search.
    async fn fetch_by_anchor_query(
        &self,
        query: &AnchorQuery,
        priority: RequestPriority,
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
        match query {
            AnchorQuery::Isbn13(isbn) => {
                let normalized = livrarr_domain::strip_isbn_punctuation(isbn);
                match crate::hardcover::query_hardcover_by_isbn(
                    &self.fetcher,
                    &normalized,
                    &token,
                    cfg.as_ref(),
                    priority,
                )
                .await
                {
                    Ok(Some(hc)) => self.build_success(hc, &token, priority).await,
                    Ok(None) => ProviderOutcome::NotFound,
                    Err(crate::hardcover::HardcoverError::CircuitOpen(retry_after)) => {
                        circuit_open_outcome(retry_after)
                    }
                    Err(crate::hardcover::HardcoverError::Http(_)) => ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    },
                    Err(_) => ProviderOutcome::NotFound,
                }
            }
            AnchorQuery::HcKey(key) => {
                let book_id = match key.trim().parse::<i64>() {
                    Ok(id) => id,
                    Err(_) => return ProviderOutcome::NotFound,
                };
                match crate::hardcover::query_hardcover_by_key(
                    &self.fetcher,
                    book_id,
                    &token,
                    priority,
                )
                .await
                {
                    Ok(Some(hc)) => self.build_success(hc, &token, priority).await,
                    Ok(None) => ProviderOutcome::NotFound,
                    Err(crate::hardcover::HardcoverError::CircuitOpen(retry_after)) => {
                        circuit_open_outcome(retry_after)
                    }
                    Err(crate::hardcover::HardcoverError::Http(_)) => ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    },
                    Err(_) => ProviderOutcome::NotFound,
                }
            }
            _ => ProviderOutcome::NotFound,
        }
    }

    async fn fetch(
        &self,
        work: &Work,
        priority: RequestPriority,
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

        // ISBN-first path: query by ISBN, verify ISBN in hit's isbns array
        if let Some(isbn) = work.isbn_13.as_deref().filter(|s| !s.is_empty()) {
            let normalized = livrarr_domain::strip_isbn_punctuation(isbn);
            match crate::hardcover::query_hardcover_by_isbn(
                &self.fetcher,
                &normalized,
                &token,
                cfg.as_ref(),
                priority,
            )
            .await
            {
                Ok(Some(hc)) => {
                    if let ProviderOutcome::Success(mut p) =
                        self.build_success(hc, &token, priority).await
                    {
                        p.isbn_13 = Some(normalized.clone());
                        return ProviderOutcome::Success(p);
                    }
                    return ProviderOutcome::NotFound;
                }
                Ok(None) => {
                    tracing::debug!(isbn = %normalized, "HC ISBN search: no verified match");
                }
                Err(crate::hardcover::HardcoverError::CircuitOpen(retry_after)) => {
                    return circuit_open_outcome(retry_after);
                }
                Err(crate::hardcover::HardcoverError::Http(e)) => {
                    tracing::debug!(isbn = %normalized, error = %e, "HC ISBN search failed");
                    return ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    };
                }
                Err(_) => {
                    tracing::debug!(isbn = %normalized, "HC ISBN search: no results");
                }
            }
        }

        // Title+author search (existing behavior)
        let result = query_hardcover(
            &self.fetcher,
            &work.title,
            &work.author_name,
            &token,
            priority,
        )
        .await;

        match result {
            Ok(hc) => self.build_success(hc, &token, priority).await,
            Err(
                crate::hardcover::HardcoverError::NoResults
                | crate::hardcover::HardcoverError::NoMatch(_),
            ) => ProviderOutcome::NotFound,
            Err(crate::hardcover::HardcoverError::CircuitOpen(retry_after)) => {
                circuit_open_outcome(retry_after)
            }
            Err(crate::hardcover::HardcoverError::Http(_)) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }

    async fn build_success(
        &self,
        hc: crate::hardcover::HardcoverResult,
        token: &str,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let year = hc
            .publish_date
            .as_deref()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse::<i32>().ok());

        let mut isbn_13 = hc.isbn_13.clone();
        if let Some(ref hc_id) = hc.hc_key {
            if let Ok(Some(better_isbn)) = crate::hardcover::fetch_hardcover_editions(
                &self.fetcher,
                hc_id,
                token,
                "en",
                priority,
            )
            .await
            {
                isbn_13 = Some(better_isbn);
            }
        }

        let payload = NormalizedWorkDetail {
            title: hc.title,
            subtitle: hc.subtitle,
            original_title: hc.original_title,
            author_name: hc.author_name,
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
}

/// Real-network OpenLibrary adapter. Wraps
/// `crate::openlibrary::query_ol_detail`. OL detail fetch is keyed on
/// `work.ol_key`; works without an `ol_key` are reported as `NotFound` without
/// hitting the network.
#[derive(Clone)]
pub struct OpenLibraryClient<F: HttpFetcher = livrarr_http::fetcher::HttpFetcherImpl> {
    fetcher: F,
    retry_backoff_secs: i64,
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl<F: HttpFetcher> OpenLibraryClient<F> {
    pub fn new(fetcher: F) -> Self {
        Self {
            fetcher,
            retry_backoff_secs: 5 * 60,
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001).
    pub fn with_call_sink(mut self, sink: Arc<dyn ProviderCallSink>) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub fn with_retry_backoff(mut self, secs: i64) -> Self {
        self.retry_backoff_secs = secs;
        self
    }

    /// Anchor-only fetch (REQ-006): ol_key direct detail, or ISBN resolved to
    /// an OL work key first; no title/author fallback on this surface.
    async fn fetch_by_anchor_query(
        &self,
        query: &AnchorQuery,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match query {
            AnchorQuery::OlKey(key) => self.detail_by_key(key, priority).await,
            AnchorQuery::Isbn13(isbn) => {
                let normalized = livrarr_domain::strip_isbn_punctuation(isbn);
                match self.isbn_lookup(&normalized, priority).await {
                    Ok(Some(ol_work_key)) => self.detail_by_key(&ol_work_key, priority).await,
                    Ok(None) => ProviderOutcome::NotFound,
                    Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                        circuit_open_outcome(retry_after)
                    }
                    Err(_) => ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    },
                }
            }
            _ => ProviderOutcome::NotFound,
        }
    }

    /// OL detail fetch by work key. `query_ol_detail`'s error is mostly opaque
    /// (parse and 4xx/5xx are indistinguishable), so a miss maps to NotFound —
    /// mirroring the seeded fetch's tier behavior, never a text search. A
    /// breaker-open pause is the one error kind that must NOT collapse into
    /// NotFound (R-11: NotFound is phase-2 terminal — persisting it would turn
    /// a temporary pause into a permanent miss).
    async fn detail_by_key(
        &self,
        ol_key: &str,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match query_ol_detail(&self.fetcher, ol_key, priority).await {
            Ok(detail) => ProviderOutcome::Success(Box::new(self.build_payload(ol_key, detail))),
            Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                circuit_open_outcome(retry_after)
            }
            Err(e) => {
                tracing::debug!(ol_key = %ol_key, error = %e, "OL key detail miss");
                ProviderOutcome::NotFound
            }
        }
    }

    async fn fetch(
        &self,
        work: &Work,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        // Tier 1: ISBN lookup. Transient failures on this strong signal return
        // immediately (circuit pause or retry-later) — mirroring the Hardcover
        // tiers — so a blip never degrades the fetch to the weaker fuzzy tier.
        // Only a genuine no-match (Ok(None)) falls through.
        if let Some(isbn) = work.isbn_13.as_deref().filter(|s| !s.is_empty()) {
            let normalized = livrarr_domain::strip_isbn_punctuation(isbn);
            match self.isbn_lookup(&normalized, priority).await {
                Ok(Some(ol_work_key)) => {
                    match query_ol_detail(&self.fetcher, &ol_work_key, priority).await {
                        Ok(detail) => {
                            let mut payload = self.build_payload(&ol_work_key, detail);
                            payload.isbn_13 = Some(normalized.clone());
                            return ProviderOutcome::Success(Box::new(payload));
                        }
                        Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                            return circuit_open_outcome(retry_after);
                        }
                        Err(crate::types::ProviderFetchError::NotFound) => {
                            tracing::debug!(isbn = %normalized, "OL ISBN detail: work absent");
                        }
                        Err(e) => {
                            tracing::debug!(isbn = %normalized, error = %e, "OL ISBN detail failed");
                            return ProviderOutcome::WillRetry {
                                reason: livrarr_domain::WillRetryReason::ServerError,
                                next_attempt_at: Utc::now()
                                    + chrono::Duration::seconds(self.retry_backoff_secs),
                            };
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!(isbn = %normalized, "OL ISBN lookup: no work found");
                }
                Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                    return circuit_open_outcome(retry_after);
                }
                Err(e) => {
                    tracing::debug!(isbn = %normalized, error = %e, "OL ISBN lookup failed");
                    return ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    };
                }
            }
        }

        // Tier 2: ol_key direct lookup. Same strong-signal rule as tier 1:
        // transient failures return; the fuzzy tier is never their fallback.
        if let Some(ol_key) = work.ol_key.as_deref().filter(|s| !s.is_empty()) {
            match query_ol_detail(&self.fetcher, ol_key, priority).await {
                Ok(detail) => {
                    return ProviderOutcome::Success(Box::new(self.build_payload(ol_key, detail)));
                }
                Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                    return circuit_open_outcome(retry_after);
                }
                Err(crate::types::ProviderFetchError::NotFound) => {
                    tracing::debug!(ol_key = %ol_key, "OL key detail: work absent");
                }
                Err(e) => {
                    tracing::debug!(ol_key = %ol_key, error = %e, "OL key detail failed");
                    return ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    };
                }
            }
        }

        // Tier 3: title+author search fallback
        match self.title_author_search(work, priority).await {
            Ok(Some(payload)) => ProviderOutcome::Success(Box::new(payload)),
            Ok(None) => ProviderOutcome::NotFound,
            Err(crate::types::ProviderFetchError::CircuitOpen(retry_after)) => {
                circuit_open_outcome(retry_after)
            }
            Err(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }

    fn build_payload(
        &self,
        ol_key: &str,
        detail: crate::openlibrary::OlDetailResult,
    ) -> NormalizedWorkDetail {
        let cover_url = detail
            .cover_id
            .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg?default=false"));
        NormalizedWorkDetail {
            title: detail.title,
            description: detail.description,
            ol_key: Some(ol_key.to_string()),
            isbn_13: detail.isbn_13,
            cover_url,
            ..Default::default()
        }
    }

    async fn isbn_lookup(
        &self,
        isbn: &str,
        priority: RequestPriority,
    ) -> Result<Option<String>, crate::types::ProviderFetchError> {
        crate::openlibrary::isbn_lookup(&self.fetcher, isbn, priority).await
    }

    async fn title_author_search(
        &self,
        work: &Work,
        priority: RequestPriority,
    ) -> Result<Option<NormalizedWorkDetail>, crate::types::ProviderFetchError> {
        let query = format!("{} {}", work.title, work.author_name);
        let url = format!(
            "https://openlibrary.org/search.json?q={}&fields=key,title,author_name&limit=10",
            urlencoding::encode(&query)
        );

        let req = FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(30),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority,
        };
        let resp = match self.fetcher.fetch(req).await {
            Ok(r) => r,
            Err(livrarr_domain::services::FetchError::CircuitOpen { retry_after }) => {
                return Err(crate::types::ProviderFetchError::CircuitOpen(retry_after));
            }
            Err(e) => {
                return Err(crate::types::ProviderFetchError::Other(format!(
                    "OL search failed: {e}"
                )))
            }
        };

        if !(200..300).contains(&resp.status) {
            if (500..600).contains(&resp.status) {
                outbound_queue::shared()
                    .report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
            }
            return Err(crate::types::ProviderFetchError::Other(format!(
                "OL search returned {}",
                resp.status
            )));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| {
            crate::types::ProviderFetchError::Other(format!("OL search parse error: {e}"))
        })?;
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Success);

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let candidates: Vec<(String, String)> = docs
            .iter()
            .filter_map(|doc| {
                let title = doc.get("title")?.as_str()?.to_string();
                let author = doc
                    .get("author_name")
                    .and_then(|a| a.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((title, author))
            })
            .collect();

        let best_idx = crate::audible::score_provider_candidates(
            &work.title,
            &work.author_name,
            &candidates,
            0.75,
            1,
        );

        let idx = match best_idx {
            Some(i) => i,
            None => return Ok(None),
        };

        let ol_key = docs[idx]
            .get("key")
            .and_then(|k| k.as_str())
            .map(|k| k.strip_prefix("/works/").unwrap_or(k).to_string());

        let ol_key = match ol_key {
            Some(k) => k,
            None => return Ok(None),
        };

        match query_ol_detail(&self.fetcher, &ol_key, priority).await {
            Ok(detail) => Ok(Some(self.build_payload(&ol_key, detail))),
            Err(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod openlibrary_qw2_pins {
    use super::*;
    use livrarr_domain::services::{FetchError, FetchResponse};

    fn ok_json(value: serde_json::Value) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: value.to_string().into_bytes(),
        })
    }

    fn server_error(status: u16) -> Result<FetchResponse, FetchError> {
        Err(FetchError::HttpError {
            status,
            classification: "server_error".to_string(),
        })
    }

    fn work(isbn_13: Option<&str>, ol_key: Option<&str>) -> Work {
        Work {
            id: 1,
            user_id: 1,
            title: "Fuzzy Success".to_string(),
            author_name: "Fuzzy Author".to_string(),
            isbn_13: isbn_13.map(str::to_string),
            ol_key: ol_key.map(str::to_string),
            ..Work::default()
        }
    }

    fn fuzzy_success_search() -> Result<FetchResponse, FetchError> {
        ok_json(serde_json::json!({
            "docs": [{
                "key": "/works/OLFUZZY1W",
                "title": "Fuzzy Success",
                "author_name": ["Fuzzy Author"]
            }]
        }))
    }

    fn fuzzy_success_detail() -> Result<FetchResponse, FetchError> {
        ok_json(serde_json::json!({
            "title": "Fuzzy Success",
            "description": "detail"
        }))
    }

    fn empty_editions() -> Result<FetchResponse, FetchError> {
        ok_json(serde_json::json!({ "entries": [] }))
    }

    fn has_fuzzy_search(fetcher: &crate::test_support::RecordingHttpFetcher) -> bool {
        fetcher
            .requests()
            .iter()
            .any(|req| req.url.contains("/search.json?q="))
    }

    #[tokio::test]
    async fn qw2_openlibrary_isbn_circuit_open_never_falls_to_fuzzy_search() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_response(Err(
            FetchError::CircuitOpen {
                retry_after: Duration::from_secs(17),
            },
        ));
        fetcher.push_response(fuzzy_success_search());
        fetcher.push_response(fuzzy_success_detail());
        fetcher.push_response(empty_editions());
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(
                &work(Some("978-1-234-56789-7"), None),
                RequestPriority::Normal,
            )
            .await;

        assert!(matches!(
            outcome,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                ..
            }
        ));
        assert!(!has_fuzzy_search(&client.fetcher));
    }

    #[tokio::test]
    async fn qw2_openlibrary_isbn_server_error_never_falls_to_fuzzy_search() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_response(server_error(503));
        fetcher.push_response(fuzzy_success_search());
        fetcher.push_response(fuzzy_success_detail());
        fetcher.push_response(empty_editions());
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(
                &work(Some("978-1-234-56789-7"), None),
                RequestPriority::Normal,
            )
            .await;

        assert!(matches!(
            outcome,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::ServerError,
                ..
            }
        ));
        assert!(!has_fuzzy_search(&client.fetcher));
    }

    #[tokio::test]
    async fn qw2_openlibrary_ol_key_server_error_never_falls_to_fuzzy_search() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_response(server_error(502));
        fetcher.push_response(fuzzy_success_search());
        fetcher.push_response(fuzzy_success_detail());
        fetcher.push_response(empty_editions());
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(&work(None, Some("OL999W")), RequestPriority::Normal)
            .await;

        assert!(matches!(
            outcome,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::ServerError,
                ..
            }
        ));
        assert!(!has_fuzzy_search(&client.fetcher));
    }

    #[tokio::test]
    async fn qw2_openlibrary_genuine_isbn_no_match_can_fall_through_to_fuzzy_success() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_response(ok_json(
            serde_json::json!({ "works": [] }),
        ));
        fetcher.push_response(fuzzy_success_search());
        fetcher.push_response(fuzzy_success_detail());
        fetcher.push_response(empty_editions());
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(
                &work(Some("978-1-234-56789-7"), None),
                RequestPriority::Normal,
            )
            .await;

        assert!(matches!(outcome, ProviderOutcome::Success(_)));
        assert!(has_fuzzy_search(&client.fetcher));
    }

    #[tokio::test]
    async fn qw2_openlibrary_dead_ol_key_404_can_fall_through_to_fuzzy_success() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);
        fetcher.push_response(fuzzy_success_search());
        fetcher.push_response(fuzzy_success_detail());
        fetcher.push_response(empty_editions());
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(&work(None, Some("OL404GONE")), RequestPriority::Normal)
            .await;

        assert!(matches!(outcome, ProviderOutcome::Success(_)));
        assert!(has_fuzzy_search(&client.fetcher));
    }
}

/// GR-sourced candidate text (a search/autocomplete hit's title + author) —
/// the only text allowed to vouch for a freshly resolved gr_key when the
/// detail page won't parse (#148). Never the work's own title: a
/// hallucinated key must not self-verify.
#[derive(Debug, Clone)]
struct GrCandidateText {
    title: String,
    /// GR's own undecorated form of `title` (`bookTitleBare`), when the
    /// autocomplete hit carried one. Preferred as the payload title: the
    /// decorated form ("Pandora's Star (Commonwealth Saga, #1)") reads as a
    /// one-sided series marker to the identity authority and keeps GR's
    /// answer out of the quorum cluster it belongs to.
    title_bare: Option<String>,
    author: Option<String>,
    /// Cover from the autocomplete/search hit. Kept so a GR payload still
    /// carries a cover when the detail page won't fetch (anti-bot): GR outranks
    /// OpenLibrary in the cover priority, so this wins over the OL/ISBN fallback
    /// that otherwise produces the weaker import covers.
    cover_url: Option<String>,
    /// Series evidence from the hit's search-card decoration. Travels into
    /// the payload so the identity quorum's volume veto can see the volume
    /// the picker saw — a picked later-volume record must never read as a
    /// bare-title twin downstream.
    series_name: Option<String>,
    series_position: Option<f64>,
}

impl GrCandidateText {
    /// The hit's cover, validated + upscaled, ready to use as a payload cover
    /// (same treatment `normalize` gives the detail-page cover).
    fn cover(&self) -> Option<String> {
        self.cover_url
            .clone()
            .filter(|u| goodreads::validate_cover_url(u))
            .map(|u| crate::provider_util::upscale_cover_url(&u))
    }
}

/// A resolved GR detail target plus the candidate text that chose it
/// (absent on the established-key and LLM-direct tiers).
struct ResolvedGrDetail {
    url: String,
    candidate: Option<GrCandidateText>,
}

/// Real-network Goodreads adapter. Wraps the lifted
/// `crate::goodreads::{search_goodreads, fetch_goodreads_detail}`
/// helpers and maps their errors onto `ProviderOutcome<NormalizedWorkDetail>`.
///
/// Resolution order:
///   1. If `work.gr_key` is populated, fetch the detail page directly
///      (skips a search round-trip — see R-21 canonical-identity policy).
///   2. Otherwise, search by `title author` and pick deterministically among
///      hits (`gr_best_match`: junk-edition filter + the shared title+author
///      picker, REQ-012/D6/ST-07) — no LLM is involved in the pick. GR is a
///      hostile scraping target (anti-bot, HTML drift, noisy results full of
///      study guides and alternate editions), so a wrong pick is worse than
///      none: nothing clearing the bar means GR abstains rather than
///      adopting a fuzzy match.
///   3. Resolve the chosen hit's (often relative) `detail_url` against
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
    fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    /// Kept solely for the `llm_extract_payload` pass-through — the LLM
    /// caller is out of scope for the outbound queue.
    http: HttpClient,
    base_url: String,
    retry_backoff_secs: i64,
    /// Reads `llm_*` per fetch — the LLM extraction fallback for
    /// foreign-language pages activates whenever live config has LLM
    /// configured. None means the client wasn't given a live-config handle
    /// (test / smoke-test path); LLM fallback disabled.
    live_config: Option<crate::live_config::LiveMetadataConfig>,
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl GoodreadsClient {
    pub fn new(
        fetcher: livrarr_http::fetcher::HttpFetcherImpl,
        http: HttpClient,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            fetcher,
            http,
            base_url: base_url.into(),
            retry_backoff_secs: 5 * 60,
            live_config: None,
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001).
    pub fn with_call_sink(mut self, sink: Arc<dyn ProviderCallSink>) -> Self {
        self.call_sink = Some(sink);
        self
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

    /// Anchor-only fetch (REQ-006): detail page by gr_key — the only GR
    /// enrichment surface (autocomplete/detail-by-key endpoints, ST-012). The
    /// search/disambiguation tiers live behind the identity entry points,
    /// never here.
    async fn fetch_detail_by_key(
        &self,
        gr_key: &str,
        language: Option<&str>,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let detail_url = goodreads::detail_url_for_gr_key(&self.base_url, gr_key);
        let html = match goodreads::fetch_goodreads_html(&self.fetcher, &detail_url, priority).await
        {
            Ok(h) => h,
            Err(err) => return self.map_fetch_err(err),
        };
        if let Some(detail) = goodreads::parse_detail_html(&html) {
            return ProviderOutcome::Success(Box::new(self.normalize(&detail_url, detail)));
        }
        if let Some(res) = self.llm_extract_payload(&html, language, &detail_url).await {
            return match res {
                Ok(mut payload) => {
                    if payload.gr_key.is_none() {
                        payload.gr_key = Some(gr_key.to_string());
                    }
                    ProviderOutcome::Success(Box::new(payload))
                }
                Err(err) => self.map_fetch_err(err),
            };
        }
        ProviderOutcome::NotFound
    }

    /// LLM extraction fallback for a fetched detail page (foreign-language
    /// HTML where JSON-LD/regex parsing misses). Returns None when the LLM is
    /// not configured or its extraction also parses nothing — callers fall
    /// through; Some(Err) carries fetch-class errors.
    ///
    /// Contract (REQ-012/confer b): this is repair, not selection — a payload
    /// built here carries zero extra trust. It returns the same
    /// `NormalizedWorkDetail` shape as a direct parse, with no provenance or
    /// confidence marker distinguishing it, so it rides through the same
    /// deterministic matching, vetoes, and bars as any other payload; repair
    /// can never raise confidence.
    async fn llm_extract_payload(
        &self,
        html: &str,
        language: Option<&str>,
        detail_url: &str,
    ) -> Option<Result<NormalizedWorkDetail, GoodreadsFetchError>> {
        let live = self.live_config.as_ref()?;
        let cfg = live.snapshot();
        if !cfg.llm_enabled {
            return None;
        }
        let key = cfg
            .llm_api_key
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())?;
        let endpoint = cfg
            .llm_endpoint
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())?;
        let model = cfg
            .llm_model
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("gemini-3.1-flash-lite-preview");
        let language_hint = language
            .and_then(crate::language::get_language_info)
            .map(|info| info.english_name)
            .unwrap_or("the original");
        match goodreads::extract_with_llm(
            &self.http,
            endpoint,
            key,
            model,
            html,
            language_hint,
            detail_url,
        )
        .await
        {
            Ok(payload) => Some(Ok(payload)),
            Err(GoodreadsFetchError::Parse) => None,
            Err(err) => Some(Err(err)),
        }
    }

    async fn fetch(
        &self,
        work: &Work,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let had_gr_key = work.gr_key.as_deref().is_some_and(|k| !k.is_empty());
        let resolved = match self.resolve_detail_url(work, priority).await {
            Ok(Some(resolved)) => resolved,
            Ok(None) => return ProviderOutcome::NotFound,
            Err(err) => return self.map_fetch_err(err),
        };
        let detail_url = resolved.url;

        // Extract gr_key from the resolved URL so the key survives a page
        // fetch/parse failure.
        let resolved_gr_key = goodreads::extract_gr_key(&detail_url);

        // Direct parse path. On Parse failure, optionally fall through to
        // LLM extraction if configured (typical for foreign-language pages
        // where JSON-LD / regex don't match the locale-specific HTML).
        let html = match goodreads::fetch_goodreads_html(&self.fetcher, &detail_url, priority).await
        {
            Ok(h) => h,
            Err(err) => {
                if !had_gr_key {
                    if let Some(payload) = self
                        .fallback_key_payload(work, &resolved_gr_key, &resolved.candidate, priority)
                        .await
                    {
                        tracing::info!(
                            gr_key = payload.gr_key.as_deref().unwrap_or(""),
                            verified = payload.title.is_some(),
                            "GR page fetch failed; returning key payload"
                        );
                        return ProviderOutcome::Success(Box::new(payload));
                    }
                }
                return self.map_fetch_err(err);
            }
        };

        if let Some(detail) = goodreads::parse_detail_html(&html) {
            let mut payload = self.normalize(&detail_url, detail);
            // Detail page parsed but carried no cover — fall back to the cover
            // that came with the search hit (GR outranks OL, so this beats the
            // ISBN-resolved OL cover the merge would otherwise use).
            if payload.cover_url.is_none() {
                payload.cover_url = resolved.candidate.as_ref().and_then(|c| c.cover());
            }
            apply_candidate_text(&mut payload, &resolved.candidate);
            return ProviderOutcome::Success(Box::new(payload));
        }

        // Direct parse yielded nothing. Try LLM extraction when live config
        // has LLM enabled + key + endpoint set.
        if let Some(res) = self
            .llm_extract_payload(&html, work.language.as_deref(), &detail_url)
            .await
        {
            match res {
                Ok(mut payload) => {
                    if payload.gr_key.is_none() {
                        payload.gr_key = resolved_gr_key;
                    }
                    apply_candidate_text(&mut payload, &resolved.candidate);
                    return ProviderOutcome::Success(Box::new(payload));
                }
                Err(err) => return self.map_fetch_err(err),
            }
        }

        // All parse paths failed; fall back to a key payload carrying
        // whatever GR-sourced candidate text can vouch for the key.
        if !had_gr_key {
            if let Some(payload) = self
                .fallback_key_payload(work, &resolved_gr_key, &resolved.candidate, priority)
                .await
            {
                tracing::info!(
                    gr_key = payload.gr_key.as_deref().unwrap_or(""),
                    verified = payload.title.is_some(),
                    "GR parse failed; returning key payload"
                );
                return ProviderOutcome::Success(Box::new(payload));
            }
        }

        ProviderOutcome::NotFound
    }

    /// Build the parse-failure fallback payload for a freshly resolved key.
    ///
    /// A bare key is unverifiable (REQ-024 strips it on the identity surface
    /// and the quorum can't cluster a text-less payload — #148), so the
    /// payload carries GR-SOURCED candidate text: the search hit that chose
    /// the key when one exists, else one autocomplete lookup confirming the
    /// key (LLM-direct keys arrive with no GR data at all). The work's own
    /// title is never stamped — a hallucinated key must not self-verify.
    async fn fallback_key_payload(
        &self,
        work: &Work,
        resolved_gr_key: &Option<String>,
        candidate: &Option<GrCandidateText>,
        priority: RequestPriority,
    ) -> Option<NormalizedWorkDetail> {
        let key = resolved_gr_key.as_ref()?;

        let confirmed = match candidate {
            Some(c) => Some(c.clone()),
            None => self.confirm_key_via_search(work, key, priority).await,
        };

        let cover_url = confirmed.as_ref().and_then(|c| c.cover());
        let (title, author_name, series_name, series_position) = match confirmed {
            // Same bare-over-decorated preference as the detail path. The
            // decoration's series evidence rides along: the volume the picker
            // saw must stay visible to the identity quorum's veto.
            Some(c) => (
                Some(c.title_bare.unwrap_or(c.title)),
                c.author,
                c.series_name,
                c.series_position,
            ),
            None => (None, None, None, None),
        };
        Some(NormalizedWorkDetail {
            title,
            author_name,
            series_name,
            series_position,
            gr_key: Some(key.clone()),
            cover_url,
            ..Default::default()
        })
    }

    /// One search round-trip to vouch for an LLM-direct key: returns the
    /// matching hit's text only when the hit's own gr_key equals the key.
    async fn confirm_key_via_search(
        &self,
        work: &Work,
        key: &str,
        priority: RequestPriority,
    ) -> Option<GrCandidateText> {
        let hits =
            goodreads::search_goodreads(&self.fetcher, &self.base_url, &work.title, priority)
                .await
                .ok()?;
        hits.iter()
            .find(|h| goodreads::extract_gr_key(&h.detail_url).as_deref() == Some(key))
            .map(|h| GrCandidateText {
                title: h.title.clone(),
                title_bare: h.title_bare.clone(),
                author: h.author.clone(),
                cover_url: h.cover_url.clone(),
                series_name: h.series_name.clone(),
                series_position: h.series_position,
            })
    }

    async fn resolve_detail_url(
        &self,
        work: &Work,
        priority: RequestPriority,
    ) -> Result<Option<ResolvedGrDetail>, GoodreadsFetchError> {
        // 1. work.gr_key — canonical GR identity. No candidate text: the key
        // is already established, nothing needs vouching.
        if let Some(key) = work.gr_key.as_deref().filter(|k| !k.is_empty()) {
            return Ok(Some(ResolvedGrDetail {
                url: goodreads::detail_url_for_gr_key(&self.base_url, key),
                candidate: None,
            }));
        }

        let title = &work.title;
        let author = &work.author_name;

        // 2. Search GR by title+author via the WAF-free autocomplete endpoint,
        // then a deterministic best-match pick (no LLM). A fetch error here is
        // most often GR rate-limiting / anti-bot during a bulk burst — log it
        // (previously a silent `?`, which hid these failures) and still
        // propagate so map_fetch_err can schedule a retry.
        let mut hits = match goodreads::search_goodreads(
            &self.fetcher,
            &self.base_url,
            title,
            priority,
        )
        .await
        {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(title = %title, error = ?e, "GR autocomplete failed (likely rate-limit/anti-bot)");
                return Err(e);
            }
        };

        if hits.is_empty() && !title.is_ascii() {
            let ascii_title: String = title.chars().filter(|c| c.is_ascii()).collect();
            if !ascii_title.trim().is_empty() {
                hits = match goodreads::search_goodreads(
                    &self.fetcher,
                    &self.base_url,
                    &ascii_title,
                    priority,
                )
                .await
                {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!(title = %title, error = ?e, "GR autocomplete (ascii retry) failed (likely rate-limit/anti-bot)");
                        return Err(e);
                    }
                };
            }
        }

        if let Some(idx) = gr_best_match(title, author, &hits) {
            tracing::debug!(title = %title, chosen_idx = idx, "GR search result selected (deterministic)");
            return Ok(Some(ResolvedGrDetail {
                url: goodreads::resolve_detail_url(&self.base_url, &hits[idx].detail_url),
                candidate: Some(GrCandidateText {
                    title: hits[idx].title.clone(),
                    title_bare: hits[idx].title_bare.clone(),
                    author: hits[idx].author.clone(),
                    cover_url: hits[idx].cover_url.clone(),
                    series_name: hits[idx].series_name.clone(),
                    series_position: hits[idx].series_position,
                }),
            }));
        }

        // No confident GR match. Identity must degrade without an LLM (spec:
        // work-creation-consistency) — we do NOT ask an LLM to recall a key
        // from memory: a fabricated key is worse than no key. The other
        // providers carry identity; GR simply abstains. Log so abstains are
        // visible (empty results vs hits-present-but-no-confident-match).
        if hits.is_empty() {
            tracing::debug!(title = %title, "GR autocomplete: no results");
        } else {
            tracing::debug!(
                title = %title,
                hit_count = hits.len(),
                "GR abstained: no confident title/author match"
            );
        }
        Ok(None)
    }

    fn map_fetch_err(&self, err: GoodreadsFetchError) -> ProviderOutcome<NormalizedWorkDetail> {
        let backoff = chrono::Duration::seconds(self.retry_backoff_secs);
        match err {
            GoodreadsFetchError::CircuitOpen(retry_after) => circuit_open_outcome(retry_after),
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
            .map(|u| crate::provider_util::upscale_cover_url(&u));
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

/// Prefer GR's own undecorated title (`bookTitleBare`) over whatever the
/// detail page or LLM extraction produced. GR's display titles carry
/// "(Series, #N)" search-card decoration that reads as a one-sided series
/// marker to the identity authority — the decorated form keeps an otherwise
/// correct GR answer out of the quorum cluster. Series name/position ride
/// their own payload fields, so nothing is lost: when the detail parse left
/// them empty, the candidate's decoration evidence fills them, keeping the
/// volume the picker saw visible to the quorum's veto. Provider data
/// selection, not a cleaning step: every value comes from GR itself.
fn apply_candidate_text(payload: &mut NormalizedWorkDetail, candidate: &Option<GrCandidateText>) {
    let Some(c) = candidate.as_ref() else { return };
    if let Some(bare) = c.title_bare.as_deref().filter(|t| !t.trim().is_empty()) {
        payload.title = Some(bare.to_string());
    }
    if payload.series_name.is_none() {
        payload.series_name = c.series_name.clone();
    }
    if payload.series_position.is_none() {
        payload.series_position = c.series_position;
    }
}

/// Junk Goodreads editions that share a title with the real book (study
/// guides, summaries). Filtered before the deterministic match — the job the
/// removed LLM disambiguation prompt did ("reject SparkNotes/CliffsNotes...").
fn is_gr_junk_edition(title: &str) -> bool {
    const JUNK: [&str; 6] = [
        "sparknotes",
        "cliffsnotes",
        "bookrags",
        "study guide",
        "summary of",
        "summary and analysis",
    ];
    let lower = title.to_lowercase();
    JUNK.iter().any(|needle| lower.contains(needle))
}

/// Deterministic Goodreads search-hit selection — no LLM. Drop junk
/// editions, require author-token overlap (unchanged guard), then judge the
/// title through the one matching authority: `Same` or `Grey` (the authority
/// floors grey at 0.75 of the MAIN title) picks; `Different` and
/// `VetoVolume` never do. The hit's DECORATED title is parsed — decoration
/// "(Series, #N)" reads structurally as a series marker and its volume
/// evidence participates in the veto, while a subtitled record still matches
/// a bare seed on main-title equality (the 2026-07-03 refresh-residue
/// shape). Ranking: Same beats Grey, higher grey score beats lower, then
/// author overlap; earliest hit wins ties (GR relevance order). Returns
/// None when nothing qualifies — a wrong GR key is worse than none, so GR
/// abstains.
fn gr_best_match(
    title: &str,
    author: &str,
    hits: &[goodreads::GoodreadsSearchResult],
) -> Option<usize> {
    use livrarr_domain::identity_matching::{parse_title, title_verdict, TitleVerdict};
    use livrarr_domain::text_norm;

    let seed = parse_title(title);
    let seed_author_tokens = text_norm::author_tokens(author);

    // (index, tier: Same=2 / Grey=1, grey score, author overlap)
    let mut best: Option<(usize, u8, f64, u32)> = None;
    for (idx, h) in hits.iter().enumerate() {
        if is_gr_junk_edition(&h.title) {
            continue;
        }
        let author_overlap = seed_author_tokens
            .intersection(&text_norm::author_tokens(
                h.author.as_deref().unwrap_or_default(),
            ))
            .count() as u32;
        if author_overlap < 1 {
            continue;
        }
        let (tier, score) = match title_verdict(&seed, &parse_title(&h.title)) {
            TitleVerdict::Same => (2u8, 1.0),
            TitleVerdict::Grey { score, .. } => (1u8, score),
            TitleVerdict::Different | TitleVerdict::VetoVolume => continue,
        };
        let beats_best = match best {
            None => true,
            Some((_, best_tier, best_score, best_overlap)) => {
                tier > best_tier
                    || (tier == best_tier
                        && (score > best_score
                            || (score == best_score && author_overlap > best_overlap)))
            }
        };
        if beats_best {
            best = Some((idx, tier, score, author_overlap));
        }
    }
    best.map(|(idx, ..)| idx)
}

/// Construct a `GoodreadsClient` against the production Goodreads URL.
impl GoodreadsClient {
    pub fn production(fetcher: livrarr_http::fetcher::HttpFetcherImpl, http: HttpClient) -> Self {
        Self::new(fetcher, http, GOODREADS_BASE_URL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goodreads::GoodreadsSearchResult;

    fn hit(title: &str, title_bare: Option<&str>, author: &str) -> GoodreadsSearchResult {
        GoodreadsSearchResult {
            title: title.to_string(),
            title_bare: title_bare.map(str::to_string),
            author: Some(author.to_string()),
            detail_url: "/book/show/1".to_string(),
            cover_url: None,
            year: None,
            rating: None,
            series_name: None,
            series_position: None,
        }
    }

    #[test]
    fn picker_matches_subtitled_record_from_bare_seed() {
        // The 2026-07-03 bulk-refresh residue shape: the seed title is bare
        // while GR's record carries the full subtitle — a whole-title token
        // comparison dilutes below any flat bar, but the authority sees equal
        // main titles with a one-sided subtitle (grey, floored).
        let hits = vec![hit(
            "The Power Broker: Robert Moses and the Fall of New York",
            Some("The Power Broker: Robert Moses and the Fall of New York"),
            "Robert A. Caro",
        )];
        assert_eq!(
            gr_best_match("The Power Broker", "Robert A. Caro", &hits),
            Some(0)
        );
    }

    #[test]
    fn picker_takes_decorated_series_hit_for_bare_seed() {
        // Search-card decoration "(Series, #N)" is structural to the
        // authority (a series marker), never a similarity penalty.
        let hits = vec![hit(
            "Storm Front (The Dresden Files, #1)",
            Some("Storm Front"),
            "Jim Butcher",
        )];
        assert_eq!(gr_best_match("Storm Front", "Jim Butcher", &hits), Some(0));
    }

    #[test]
    fn picker_still_rejects_sequels() {
        let hits = vec![hit("Dune Messiah", Some("Dune Messiah"), "Frank Herbert")];
        assert_eq!(gr_best_match("Dune", "Frank Herbert", &hits), None);
    }

    #[test]
    fn picker_requires_author_overlap() {
        let hits = vec![hit(
            "The Power Broker: Robert Moses and the Fall of New York",
            None,
            "Somebody Else",
        )];
        assert_eq!(
            gr_best_match("The Power Broker", "Robert A. Caro", &hits),
            None
        );
    }

    #[test]
    fn picker_vetoes_conflicting_volumes() {
        // Equal mains but contradicting volume evidence must never auto-pick —
        // the decoration carries the hit's volume, the seed carries its own.
        let hits = vec![hit("Alpha (Saga, #3)", Some("Alpha"), "Ann Author")];
        assert_eq!(gr_best_match("Alpha, Vol. 2", "Ann Author", &hits), None);
    }

    #[test]
    fn picker_prefers_exact_match_over_subtitled_grey() {
        let hits = vec![
            hit(
                "The Power Broker: Robert Moses and the Fall of New York",
                None,
                "Robert A. Caro",
            ),
            hit("The Power Broker", None, "Robert A. Caro"),
        ];
        assert_eq!(
            gr_best_match("The Power Broker", "Robert A. Caro", &hits),
            Some(1)
        );
    }

    #[test]
    fn picker_junk_filter_still_applies() {
        let hits = vec![hit(
            "Summary of The Power Broker by Robert A. Caro",
            None,
            "Robert A. Caro",
        )];
        assert_eq!(
            gr_best_match("The Power Broker", "Robert A. Caro", &hits),
            None
        );
    }
}
