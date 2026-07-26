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
use livrarr_domain::{
    AnchorQuery, MetadataProvider, PermanentFailureReason, RequestPriority, WillRetryReason, Work,
};
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

/// Common `WillRetry { QueueFull }` mapping (D3): the outbound queue's local
/// admission cap rejected the request — a transport-level pause exactly
/// like `circuit_open_outcome`'s `CircuitOpen`, never a provider verdict, so
/// it must never consume a retry-budget attempt either (`apply_budget_rules`
/// exempts both reasons identically).
fn queue_full_outcome(retry_after: Duration) -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::WillRetry {
        reason: WillRetryReason::QueueFull,
        next_attempt_at: Utc::now()
            + chrono::Duration::from_std(retry_after)
                .unwrap_or_else(|_| chrono::Duration::seconds(60)),
    }
}

/// Common `WillRetry { RateLimit }` mapping (Unit A): a live 429 is a real
/// provider verdict (unlike `CircuitOpen`), so it consumes one retry-budget
/// attempt. Backoff mirrors `google_books::map_http_error`'s quota-exhaustion
/// formula exactly — 6h + up to 3h jitter — so OL and Audnexus back off on
/// the same schedule as Google Books rather than pounding a rate-limited
/// provider every few minutes.
fn rate_limit_outcome() -> ProviderOutcome<NormalizedWorkDetail> {
    let jitter_secs = (Utc::now().timestamp_subsec_nanos() % 10_800) as i64;
    ProviderOutcome::WillRetry {
        reason: WillRetryReason::RateLimit,
        next_attempt_at: Utc::now()
            + chrono::Duration::hours(6)
            + chrono::Duration::seconds(jitter_secs),
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
            // D3: same observability treatment as CircuitOpen above — a
            // local admission-queue pause, not a provider-derived signal.
            WillRetryReason::QueueFull => (
                CallOutcomeClass::RateLimited,
                Some("queue_full".to_string()),
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
            Err(e) => audnexus_error_outcome(&e, self.retry_backoff_secs),
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
            Err(e) => audnexus_error_outcome(&e, self.retry_backoff_secs),
        }
    }
}

/// One classification of a `ProviderFetchError` into the outcome any
/// Audnexus caller — anchor (`fetch_by_asin`) or seeded (`fetch`) — must
/// report (Unit A). The single place this decision is made so the two entry
/// paths cannot drift. Audnexus is keyless: a 403 has no local credential to
/// fix, so — like any other unexpected 4xx — it is an explicit
/// `PermanentFailure`, never `NotConfigured` (which would be semantically
/// false for a provider with no key to check).
fn audnexus_error_outcome(
    err: &crate::types::ProviderFetchError,
    retry_backoff_secs: i64,
) -> ProviderOutcome<NormalizedWorkDetail> {
    match err {
        crate::types::ProviderFetchError::CircuitOpen(retry_after) => {
            circuit_open_outcome(*retry_after)
        }
        crate::types::ProviderFetchError::NotFound => ProviderOutcome::NotFound,
        crate::types::ProviderFetchError::RateLimited => rate_limit_outcome(),
        crate::types::ProviderFetchError::Transient => ProviderOutcome::WillRetry {
            reason: WillRetryReason::ServerError,
            next_attempt_at: Utc::now() + chrono::Duration::seconds(retry_backoff_secs),
        },
        crate::types::ProviderFetchError::QueueFull(retry_after) => {
            queue_full_outcome(*retry_after)
        }
        crate::types::ProviderFetchError::Other(_) => ProviderOutcome::PermanentFailure {
            reason: PermanentFailureReason::Unsupported,
        },
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
                    Ok(None) => {
                        // A parsed 200 carrying no matching book is a healthy
                        // answer, not a health failure. `hc_post` deliberately
                        // reports no Success (an operation may have a second
                        // leg), so the operation boundary must report it —
                        // otherwise a half-open Hardcover breaker can never be
                        // closed by a probe that legitimately misses.
                        crate::hardcover::report_hardcover_success();
                        ProviderOutcome::NotFound
                    }
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
                    Ok(None) => {
                        // A parsed 200 carrying no matching book is a healthy
                        // answer, not a health failure. `hc_post` deliberately
                        // reports no Success (an operation may have a second
                        // leg), so the operation boundary must report it —
                        // otherwise a half-open Hardcover breaker can never be
                        // closed by a probe that legitimately misses.
                        crate::hardcover::report_hardcover_success();
                        ProviderOutcome::NotFound
                    }
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
                    // A healthy ISBN miss is not the boundary on this seeded
                    // surface: title/author fallback still runs below.
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
            ) => {
                // The provider answered fine; it just has no match. One leg,
                // succeeded — report the operation healthy.
                crate::hardcover::report_hardcover_success();
                ProviderOutcome::NotFound
            }
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
        // The Hardcover operation reports ONE breaker outcome, here, and only
        // when every leg it ran succeeded. `hc_post` deliberately reports no
        // success of its own: a success clears every accumulated failure, so a
        // query-leg success landing before an editions-leg refusal meant a
        // permanently refused editions endpoint never reached the threshold.
        let mut editions_leg_ok = true;
        if let Some(ref hc_id) = hc.hc_key {
            match crate::hardcover::fetch_hardcover_editions(
                &self.fetcher,
                hc_id,
                token,
                "en",
                priority,
            )
            .await
            {
                Ok(Some(better_isbn)) => isbn_13 = Some(better_isbn),
                Ok(None) => {}
                // Still best-effort for the PAYLOAD — a missing ISBN never
                // fails the work fetch — but it is not a healthy operation.
                Err(_) => editions_leg_ok = false,
            }
        }
        if editions_leg_ok {
            crate::hardcover::report_hardcover_success();
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

/// One classification of a `ProviderFetchError` into the outcome any
/// OpenLibrary caller — anchor (`fetch_by_anchor_query`/`detail_by_key`) or
/// seeded (`fetch`) — must report (Unit A). The single place this decision
/// is made so the several entry paths built on `isbn_lookup`/`query_ol_detail`
/// cannot drift. OL is keyless: a 403 has no local credential to fix, so —
/// like any other unexpected 4xx — it is an explicit `PermanentFailure`,
/// never `NotConfigured` (which would be semantically false for a provider
/// with no key to check).
fn ol_error_outcome(
    err: &crate::types::ProviderFetchError,
    retry_backoff_secs: i64,
) -> ProviderOutcome<NormalizedWorkDetail> {
    match err {
        crate::types::ProviderFetchError::CircuitOpen(retry_after) => {
            circuit_open_outcome(*retry_after)
        }
        crate::types::ProviderFetchError::NotFound => ProviderOutcome::NotFound,
        crate::types::ProviderFetchError::RateLimited => rate_limit_outcome(),
        crate::types::ProviderFetchError::Transient => ProviderOutcome::WillRetry {
            reason: WillRetryReason::ServerError,
            next_attempt_at: Utc::now() + chrono::Duration::seconds(retry_backoff_secs),
        },
        crate::types::ProviderFetchError::QueueFull(retry_after) => {
            queue_full_outcome(*retry_after)
        }
        crate::types::ProviderFetchError::Other(_) => ProviderOutcome::PermanentFailure {
            reason: PermanentFailureReason::Unsupported,
        },
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
                    Ok(Some(ol_work_key)) => {
                        match query_ol_detail(&self.fetcher, &ol_work_key, priority, None, None)
                            .await
                        {
                            Ok(detail) => {
                                if detail.all_legs_succeeded {
                                    crate::openlibrary::report_openlibrary_success();
                                }
                                ProviderOutcome::Success(Box::new(
                                    self.build_payload(&ol_work_key, detail),
                                ))
                            }
                            Err(crate::types::ProviderFetchError::NotFound) => {
                                crate::openlibrary::report_openlibrary_success();
                                ProviderOutcome::NotFound
                            }
                            Err(e) => ol_error_outcome(&e, self.retry_backoff_secs),
                        }
                    }
                    Ok(None) => {
                        crate::openlibrary::report_openlibrary_success();
                        ProviderOutcome::NotFound
                    }
                    Err(e) => ol_error_outcome(&e, self.retry_backoff_secs),
                }
            }
            _ => ProviderOutcome::NotFound,
        }
    }

    /// OL detail fetch by work key. A genuine miss (HTTP 404/410) maps to
    /// NotFound — mirroring the seeded fetch's tier behavior, never a text
    /// search. A breaker-open pause, a live 429/5xx, or any other opaque
    /// failure are NOT collapsed into NotFound (R-11 / Unit A: NotFound is
    /// phase-2 terminal — persisting it would turn a temporary pause or a
    /// retryable rate-limit into a permanent miss).
    async fn detail_by_key(
        &self,
        ol_key: &str,
        priority: RequestPriority,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match query_ol_detail(&self.fetcher, ol_key, priority, None, None).await {
            Ok(detail) => {
                if detail.all_legs_succeeded {
                    crate::openlibrary::report_openlibrary_success();
                }
                ProviderOutcome::Success(Box::new(self.build_payload(ol_key, detail)))
            }
            Err(crate::types::ProviderFetchError::NotFound) => {
                crate::openlibrary::report_openlibrary_success();
                ProviderOutcome::NotFound
            }
            Err(e) => {
                tracing::debug!(ol_key = %ol_key, error = %e, "OL key detail miss");
                ol_error_outcome(&e, self.retry_backoff_secs)
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
                    match query_ol_detail(
                        &self.fetcher,
                        &ol_work_key,
                        priority,
                        work.language.as_deref(),
                        Some(work.title.as_str()),
                    )
                    .await
                    {
                        Ok(detail) => {
                            if detail.all_legs_succeeded {
                                crate::openlibrary::report_openlibrary_success();
                            }
                            let mut payload = self.build_payload(&ol_work_key, detail);
                            payload.isbn_13 = Some(normalized.clone());
                            return ProviderOutcome::Success(Box::new(payload));
                        }
                        Err(crate::types::ProviderFetchError::NotFound) => {
                            tracing::debug!(isbn = %normalized, "OL ISBN detail: work absent");
                        }
                        Err(e) => {
                            tracing::debug!(isbn = %normalized, error = %e, "OL ISBN detail failed");
                            return ol_error_outcome(&e, self.retry_backoff_secs);
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!(isbn = %normalized, "OL ISBN lookup: no work found");
                }
                Err(e) => {
                    tracing::debug!(isbn = %normalized, error = %e, "OL ISBN lookup failed");
                    return ol_error_outcome(&e, self.retry_backoff_secs);
                }
            }
        }

        // Tier 2: ol_key direct lookup. Same strong-signal rule as tier 1:
        // transient failures return; the fuzzy tier is never their fallback.
        if let Some(ol_key) = work.ol_key.as_deref().filter(|s| !s.is_empty()) {
            match query_ol_detail(
                &self.fetcher,
                ol_key,
                priority,
                work.language.as_deref(),
                Some(work.title.as_str()),
            )
            .await
            {
                Ok(detail) => {
                    if detail.all_legs_succeeded {
                        crate::openlibrary::report_openlibrary_success();
                    }
                    return ProviderOutcome::Success(Box::new(self.build_payload(ol_key, detail)));
                }
                Err(crate::types::ProviderFetchError::NotFound) => {
                    tracing::debug!(ol_key = %ol_key, "OL key detail: work absent");
                }
                Err(e) => {
                    tracing::debug!(ol_key = %ol_key, error = %e, "OL key detail failed");
                    return ol_error_outcome(&e, self.retry_backoff_secs);
                }
            }
        }

        // Tier 3: title+author search fallback. Errors route through the
        // same `ol_error_outcome` classification Tier 1/2 use, so a live
        // 429/5xx/QueueFull retries instead of terminalizing as NotFound.
        match self.title_author_search(work, priority).await {
            Ok(Some(payload)) => ProviderOutcome::Success(Box::new(payload)),
            Ok(None) => ProviderOutcome::NotFound,
            Err(e) => ol_error_outcome(&e, self.retry_backoff_secs),
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
            Err(livrarr_domain::services::FetchError::RateLimited) => {
                return Err(crate::types::ProviderFetchError::RateLimited);
            }
            Err(livrarr_domain::services::FetchError::CircuitOpen { retry_after }) => {
                return Err(crate::types::ProviderFetchError::CircuitOpen(retry_after));
            }
            // D3 residual: a local admission-cap rejection is budget-exempt
            // exactly like `CircuitOpen` — no HTTP was attempted, so it must
            // never be folded into the budget-consuming `Other`/ServerError
            // bucket below.
            Err(livrarr_domain::services::FetchError::QueueFull { retry_after }) => {
                return Err(crate::types::ProviderFetchError::QueueFull(retry_after));
            }
            Err(livrarr_domain::services::FetchError::HttpError { status, .. }) => {
                return Err(crate::openlibrary::classify_ol_error(status));
            }
            Err(e) => {
                tracing::debug!(error = %e, "OL search: transport failure");
                return Err(crate::types::ProviderFetchError::Transient);
            }
        };

        if !(200..300).contains(&resp.status) {
            // Search has no genuine-absence status — an empty result set is a
            // 200 with an empty array — so EVERY non-2xx here is a provider
            // health signal, 404 and 410 included. A 404 on a search route means
            // the route moved or is blocked, not that a book is missing;
            // exempting it would report every queried book as absent while the
            // provider status stayed green.
            outbound_queue::shared()
                .report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
            return Err(crate::openlibrary::classify_ol_error(resp.status));
        }

        let data: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| {
            crate::types::ProviderFetchError::Other(format!("OL search parse error: {e}"))
        })?;
        // Search is only the first leg when it selects a work. Report no
        // Success until this helper knows no detail/editions leg will follow.

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        // Keep each doc's ORIGINAL index: `filter_map` compacts the candidate
        // list (docs without a title are dropped), so the picker's index must
        // map back through `kept` before indexing `docs` — otherwise a dropped
        // earlier doc shifts the index and `docs[idx]` reads the wrong work's
        // key. Mirrors the Hardcover/Goodreads sites.
        let kept: Vec<(usize, (String, String))> = docs
            .iter()
            .enumerate()
            .filter_map(|(i, doc)| {
                let title = doc.get("title")?.as_str()?.to_string();
                let author = doc
                    .get("author_name")
                    .and_then(|a| a.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((i, (title, author)))
            })
            .collect();

        let candidates: Vec<(String, String)> = kept.iter().map(|(_, c)| c.clone()).collect();

        let best_idx = livrarr_domain::identity_matching::pick_best_candidate(
            &work.title,
            &work.author_name,
            &candidates,
            false,
        );

        let idx = match best_idx {
            Some(i) => kept[i].0,
            None => {
                tracing::debug!(
                    work_id = work.id,
                    title = %work.title,
                    author = %work.author_name,
                    candidates = candidates.len(),
                    top_candidate = candidates.first().map(|(t, _)| t.as_str()).unwrap_or(""),
                    "OpenLibrary title+author search: no candidate cleared the identity bar — OL result dropped from the fan-out"
                );
                crate::openlibrary::report_openlibrary_success();
                return Ok(None);
            }
        };

        let ol_key = docs[idx]
            .get("key")
            .and_then(|k| k.as_str())
            .map(|k| k.strip_prefix("/works/").unwrap_or(k).to_string());

        let ol_key = match ol_key {
            Some(k) => k,
            None => {
                crate::openlibrary::report_openlibrary_success();
                return Ok(None);
            }
        };

        match query_ol_detail(
            &self.fetcher,
            &ol_key,
            priority,
            work.language.as_deref(),
            Some(work.title.as_str()),
        )
        .await
        {
            Ok(detail) => {
                if detail.all_legs_succeeded {
                    crate::openlibrary::report_openlibrary_success();
                }
                Ok(Some(self.build_payload(&ol_key, detail)))
            }
            // A genuine miss stays NotFound; every other error (429/5xx/
            // QueueFull/CircuitOpen/Other) must propagate so the caller
            // routes it through `ol_error_outcome` — mirroring Tier 1/2,
            // which never collapse a live failure into a permanent miss.
            Err(crate::types::ProviderFetchError::NotFound) => {
                crate::openlibrary::report_openlibrary_success();
                Ok(None)
            }
            Err(e) => Err(e),
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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

    /// ISBN lookup is a child leg. Repeating `isbn=200, detail=200,
    /// editions=403` must accumulate the editions failures instead of clearing
    /// them at the start of every invocation.
    // Bug reproduction: subtitle-matching finding 1 — OpenLibrary ISBN
    // Success was reported before the composite operation finished.
    #[tokio::test]
    async fn openlibrary_isbn_composite_accumulates_later_editions_failures() {
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        let fetcher = crate::test_support::RecordingHttpFetcher::new();
        for _ in 0..12 {
            fetcher.push_response(ok_json(serde_json::json!({
                "works": [{"key": "/works/OL123W"}]
            })));
            fetcher.push_response(ok_json(serde_json::json!({"title": "Fuzzy Success"})));
            fetcher.push_response(Ok(FetchResponse {
                status: 403,
                headers: vec![],
                body: vec![],
            }));
        }
        let client = OpenLibraryClient::new(fetcher);

        for _ in 0..12 {
            let outcome = client
                .fetch(&work(Some("9781234567890"), None), RequestPriority::Normal)
                .await;
            assert!(matches!(outcome, ProviderOutcome::Success(_)));
        }

        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "the ISBN leg must not clear every later editions Failure"
        );
    }

    /// Title/author search is also a child leg when it selects a work and then
    /// fetches detail plus editions. Its parsed search response is not the
    /// operation boundary.
    #[tokio::test]
    async fn openlibrary_title_composite_accumulates_later_editions_failures() {
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        let fetcher = crate::test_support::RecordingHttpFetcher::new();
        for _ in 0..12 {
            fetcher.push_response(fuzzy_success_search());
            fetcher.push_response(fuzzy_success_detail());
            fetcher.push_response(Ok(FetchResponse {
                status: 403,
                headers: vec![],
                body: vec![],
            }));
        }
        let client = OpenLibraryClient::new(fetcher);

        for _ in 0..12 {
            let outcome = client
                .fetch(&work(None, None), RequestPriority::Normal)
                .await;
            assert!(matches!(outcome, ProviderOutcome::Success(_)));
        }

        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "the search leg must not clear every later editions Failure"
        );
    }

    /// A genuine item absence is a healthy final answer on the direct work-key
    /// surface and must clear a half-open/pending failure history at that outer
    /// boundary.
    #[tokio::test]
    async fn openlibrary_direct_key_absence_reports_outer_success() {
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        for _ in 0..4 {
            queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::OlKey("OLMISSINGW".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert!(matches!(outcome, ProviderOutcome::NotFound));

        queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            !tripped,
            "a healthy final work-key absence must report Success at the caller boundary"
        );
    }

    /// The ISBN anchor surface also ends at a genuine 404/410 miss, so that
    /// final absence must report Success even though the ISBN request helper
    /// itself is forbidden from doing so.
    #[tokio::test]
    async fn openlibrary_isbn_absence_reports_outer_success() {
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        for _ in 0..4 {
            queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::Isbn13("9781234567890".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert!(matches!(outcome, ProviderOutcome::NotFound));

        queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            !tripped,
            "a healthy final ISBN absence must report Success at the caller boundary"
        );
    }

    #[tokio::test]
    async fn qw2_openlibrary_dead_ol_key_404_can_fall_through_to_fuzzy_success() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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

    #[tokio::test]
    async fn provider_picker_conformance_openlibrary_abstains_on_grey_author_hit() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_response(ok_json(serde_json::json!({
                "docs": [{
                    "key": "/works/OLGREY1W",
                    "title": "Storm Front",
                    "author_name": ["Jane Smith"]
                }]
            })));
        let client = OpenLibraryClient::new(fetcher);
        let work = Work {
            id: 1,
            user_id: 1,
            title: "Storm Front".to_string(),
            author_name: "John Smith".to_string(),
            ..Work::default()
        };

        let outcome = client.fetch(&work, RequestPriority::Normal).await;

        assert!(
            matches!(outcome, ProviderOutcome::NotFound),
            "grey author hit must abstain, got {outcome:?}"
        );
    }

    // -----------------------------------------------------------------
    // #9: the Tier-3 candidate-detail step used to collapse EVERY
    // `query_ol_detail` error into `Ok(None)`, so a live 429/5xx on a
    // title+author search terminalized as `NotFound` instead of retrying —
    // unlike Tier 1 (ISBN) and Tier 2 (ol_key), which already route errors
    // through `ol_error_outcome`. Each test here drives a real search hit
    // (`fuzzy_success_search`) followed by a failing candidate-detail
    // fetch, proving the fuzzy path now mirrors Tier 1/2.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn tier3_candidate_detail_5xx_is_retryable_not_notfound() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_response(fuzzy_success_search());
        fetcher.push_response(server_error(503));
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(&work(None, None), RequestPriority::Normal)
            .await;

        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::ServerError,
                    ..
                }
            ),
            "Tier-3 candidate-detail 503 must retry, not terminalize as NotFound, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn tier3_candidate_detail_429_is_ratelimit_not_notfound() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_response(fuzzy_success_search());
        fetcher.push_response(server_error(429));
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(&work(None, None), RequestPriority::Normal)
            .await;

        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::RateLimit,
                    ..
                }
            ),
            "Tier-3 candidate-detail 429 must be a RateLimit WillRetry, not NotFound, got {outcome:?}"
        );
    }

    /// Tier-3 QueueFull residual: `title_author_search`'s OWN search-call
    /// transport-error match used to collapse `FetchError::QueueFull` into
    /// `ProviderFetchError::Other`, which burns retry budget like a genuine
    /// server error. A local admission-cap rejection never even attempted
    /// HTTP, so it must stay budget-exempt (mirrors `CircuitOpen`).
    #[tokio::test]
    async fn tier3_search_queue_full_is_budget_exempt_not_servererror() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::QueueFull {
                retry_after: Duration::from_secs(1),
            });
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(&work(None, None), RequestPriority::Normal)
            .await;

        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::QueueFull,
                    ..
                }
            ),
            "Tier-3 search QueueFull must be a budget-exempt WillRetry(QueueFull), got {outcome:?}"
        );
    }

    /// A genuine miss (404 on the candidate's detail page) must still
    /// terminalize as NotFound — proving the fix doesn't turn every error
    /// into a retry.
    #[tokio::test]
    async fn tier3_candidate_detail_404_stays_notfound() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_response(fuzzy_success_search());
        fetcher.push_response(Ok(FetchResponse {
            status: 404,
            headers: vec![],
            body: vec![],
        }));
        let client = OpenLibraryClient::new(fetcher);

        let outcome = client
            .fetch(&work(None, None), RequestPriority::Normal)
            .await;

        assert!(
            matches!(outcome, ProviderOutcome::NotFound),
            "a genuine miss (404) on the Tier-3 candidate-detail step must stay NotFound, got {outcome:?}"
        );
    }
}

/// Unit A: a live 429/5xx/403/other-4xx must be classified identically no
/// matter which entry path reached it. `ol_error_outcome` /
/// `audnexus_error_outcome` are the single shared helpers both the anchor
/// surface (`fetch_by_anchor_query`/`detail_by_key`/`fetch_by_asin`) and the
/// seeded surface (`fetch`) route through, so this module proves the
/// classification once per provider and — for OpenLibrary, where the
/// generic-fetcher client makes it possible — proves it again end-to-end
/// through real HTTP-status mocking on every tier.
#[cfg(test)]
mod unit_a_retry_classification {
    use super::*;
    use crate::test_support::RecordingHttpFetcher;
    use crate::types::ProviderFetchError;
    use livrarr_domain::services::FetchError;

    fn ol_work(isbn_13: Option<&str>, ol_key: Option<&str>) -> Work {
        Work {
            id: 1,
            user_id: 1,
            title: "Matrix Work".to_string(),
            author_name: "Matrix Author".to_string(),
            isbn_13: isbn_13.map(str::to_string),
            ol_key: ol_key.map(str::to_string),
            ..Work::default()
        }
    }

    fn assert_rate_limit(outcome: &ProviderOutcome<NormalizedWorkDetail>, ctx: &str) {
        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::RateLimit,
                    ..
                }
            ),
            "{ctx}: expected WillRetry(RateLimit), got {outcome:?}"
        );
    }

    fn assert_transient(outcome: &ProviderOutcome<NormalizedWorkDetail>, ctx: &str) {
        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::ServerError,
                    ..
                }
            ),
            "{ctx}: expected WillRetry(ServerError), got {outcome:?}"
        );
    }

    /// D3/#6: a local admission-cap rejection (no HTTP attempted) must
    /// classify as budget-exempt `WillRetry{QueueFull}` — never fall into
    /// `ServerError`'s budget-consuming bucket.
    fn assert_queue_full(outcome: &ProviderOutcome<NormalizedWorkDetail>, ctx: &str) {
        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::QueueFull,
                    ..
                }
            ),
            "{ctx}: expected WillRetry(QueueFull), got {outcome:?}"
        );
    }

    fn assert_permanent(outcome: &ProviderOutcome<NormalizedWorkDetail>, ctx: &str) {
        assert!(
            matches!(outcome, ProviderOutcome::PermanentFailure { .. }),
            "{ctx}: expected PermanentFailure, got {outcome:?}"
        );
        assert!(
            !matches!(outcome, ProviderOutcome::NotConfigured),
            "{ctx}: a keyless provider must never report NotConfigured, got {outcome:?}"
        );
    }

    // -----------------------------------------------------------------
    // OpenLibrary: end-to-end HTTP-status mocking across all four entry
    // points (OpenLibraryClient is generic over HttpFetcher, so it can be
    // driven directly with the crate's own test double).
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn openlibrary_429_matches_across_anchor_and_seeded() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = RecordingHttpFetcher::with_error(FetchError::RateLimited);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::OlKey("OL1W".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_rate_limit(&outcome, "OL anchor/ol_key");

        let fetcher = RecordingHttpFetcher::with_error(FetchError::RateLimited);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::Isbn13("9781234567890".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_rate_limit(&outcome, "OL anchor/isbn13");

        let fetcher = RecordingHttpFetcher::with_error(FetchError::RateLimited);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(&ol_work(None, Some("OL1W")), RequestPriority::Normal)
            .await;
        assert_rate_limit(&outcome, "OL seeded/ol_key");

        let fetcher = RecordingHttpFetcher::with_error(FetchError::RateLimited);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(
                &ol_work(Some("9781234567890"), None),
                RequestPriority::Normal,
            )
            .await;
        assert_rate_limit(&outcome, "OL seeded/isbn13");
    }

    #[tokio::test]
    async fn openlibrary_5xx_matches_across_anchor_and_seeded() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = RecordingHttpFetcher::with_ok(503, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::OlKey("OL1W".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_transient(&outcome, "OL anchor/ol_key");

        let fetcher = RecordingHttpFetcher::with_ok(503, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::Isbn13("9781234567890".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_transient(&outcome, "OL anchor/isbn13");

        let fetcher = RecordingHttpFetcher::with_ok(503, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(&ol_work(None, Some("OL1W")), RequestPriority::Normal)
            .await;
        assert_transient(&outcome, "OL seeded/ol_key");

        let fetcher = RecordingHttpFetcher::with_ok(503, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(
                &ol_work(Some("9781234567890"), None),
                RequestPriority::Normal,
            )
            .await;
        assert_transient(&outcome, "OL seeded/isbn13");
    }

    #[tokio::test]
    async fn openlibrary_403_matches_across_anchor_and_seeded() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = RecordingHttpFetcher::with_ok(403, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::OlKey("OL1W".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_permanent(&outcome, "OL anchor/ol_key");

        let fetcher = RecordingHttpFetcher::with_ok(403, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::Isbn13("9781234567890".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_permanent(&outcome, "OL anchor/isbn13");

        let fetcher = RecordingHttpFetcher::with_ok(403, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(&ol_work(None, Some("OL1W")), RequestPriority::Normal)
            .await;
        assert_permanent(&outcome, "OL seeded/ol_key");

        let fetcher = RecordingHttpFetcher::with_ok(403, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(
                &ol_work(Some("9781234567890"), None),
                RequestPriority::Normal,
            )
            .await;
        assert_permanent(&outcome, "OL seeded/isbn13");
    }

    #[tokio::test]
    async fn openlibrary_other_4xx_matches_across_anchor_and_seeded() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = RecordingHttpFetcher::with_ok(400, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::OlKey("OL1W".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_permanent(&outcome, "OL anchor/ol_key");

        let fetcher = RecordingHttpFetcher::with_ok(400, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::Isbn13("9781234567890".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_permanent(&outcome, "OL anchor/isbn13");

        let fetcher = RecordingHttpFetcher::with_ok(400, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(&ol_work(None, Some("OL1W")), RequestPriority::Normal)
            .await;
        assert_permanent(&outcome, "OL seeded/ol_key");

        let fetcher = RecordingHttpFetcher::with_ok(400, vec![]);
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(
                &ol_work(Some("9781234567890"), None),
                RequestPriority::Normal,
            )
            .await;
        assert_permanent(&outcome, "OL seeded/isbn13");
    }

    /// D3/#6: drives the REAL `OpenLibraryClient` adapter (its actual query
    /// construction and `ol_error_outcome` classification) against a stub
    /// TRANSPORT that returns `FetchError::QueueFull` — proving the
    /// provider→ProviderOutcome mapping this unit adds. Before this unit,
    /// `query_ol_detail`/`isbn_lookup`'s catch-all folded this into
    /// `Transient`, which `ol_error_outcome` then turned into a budget-
    /// CONSUMING `WillRetry{ServerError}` — eventually a terminal
    /// `PermanentFailure{RetryBudgetExhausted}` even though no HTTP was ever
    /// attempted. `apply_budget_rules`'s exemption for `WillRetry{QueueFull}`
    /// (livrarr-enrichment) is already covered by
    /// `will_retry_queue_full_survives_the_max_attempts_boundary`; this test
    /// closes the adjacent gap that test could not — that a real provider
    /// actually PRODUCES the exempt outcome in the first place.
    #[tokio::test]
    async fn openlibrary_queue_full_matches_across_anchor_and_seeded() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = RecordingHttpFetcher::with_error(FetchError::QueueFull {
            retry_after: Duration::from_secs(1),
        });
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::OlKey("OL1W".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_queue_full(&outcome, "OL anchor/ol_key");

        let fetcher = RecordingHttpFetcher::with_error(FetchError::QueueFull {
            retry_after: Duration::from_secs(1),
        });
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch_by_anchor_query(
                &AnchorQuery::Isbn13("9781234567890".to_string()),
                RequestPriority::Normal,
            )
            .await;
        assert_queue_full(&outcome, "OL anchor/isbn13");

        let fetcher = RecordingHttpFetcher::with_error(FetchError::QueueFull {
            retry_after: Duration::from_secs(1),
        });
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(&ol_work(None, Some("OL1W")), RequestPriority::Normal)
            .await;
        assert_queue_full(&outcome, "OL seeded/ol_key");

        let fetcher = RecordingHttpFetcher::with_error(FetchError::QueueFull {
            retry_after: Duration::from_secs(1),
        });
        let client = OpenLibraryClient::new(fetcher);
        let outcome = client
            .fetch(
                &ol_work(Some("9781234567890"), None),
                RequestPriority::Normal,
            )
            .await;
        assert_queue_full(&outcome, "OL seeded/isbn13");
    }

    // -----------------------------------------------------------------
    // Audnexus: `AudnexusClient` is hard-wired to the concrete
    // `HttpFetcherImpl` (not generic over `HttpFetcher`), so it cannot be
    // driven directly with a mock fetcher. Instead this proves the same
    // invariant in two parts: (1) `cached_fetch`'s HTTP-status classification
    // is already covered end-to-end in `audnexus.rs`'s own tests via the
    // generic `query_audnexus`/`query_audnexus_by_asin` free functions; (2)
    // `audnexus_error_outcome` — the ONE function both `AudnexusClient::fetch`
    // (seeded) and `fetch_by_asin` (anchor) delegate their `Err` arm to,
    // verbatim, with no other logic in between — is exercised here directly
    // against every `ProviderFetchError` variant. Together these prove the
    // anchor and seeded paths cannot classify the same failure differently,
    // without requiring `AudnexusClient` itself to become HTTP-mockable.
    // -----------------------------------------------------------------

    #[test]
    fn audnexus_error_outcome_classifies_every_variant() {
        assert_rate_limit(
            &audnexus_error_outcome(&ProviderFetchError::RateLimited, 300),
            "Audnexus RateLimited",
        );
        assert_transient(
            &audnexus_error_outcome(&ProviderFetchError::Transient, 300),
            "Audnexus Transient",
        );
        assert_permanent(
            &audnexus_error_outcome(&ProviderFetchError::Other("HTTP 403".to_string()), 300),
            "Audnexus Other(403)",
        );
        assert_permanent(
            &audnexus_error_outcome(&ProviderFetchError::Other("HTTP 400".to_string()), 300),
            "Audnexus Other(400)",
        );
        assert!(matches!(
            audnexus_error_outcome(&ProviderFetchError::NotFound, 300),
            ProviderOutcome::NotFound
        ));
        assert!(matches!(
            audnexus_error_outcome(
                &ProviderFetchError::CircuitOpen(Duration::from_secs(17)),
                300
            ),
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                ..
            }
        ));
        assert!(matches!(
            audnexus_error_outcome(&ProviderFetchError::QueueFull(Duration::from_secs(1)), 300),
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::QueueFull,
                ..
            }
        ));
    }

    #[test]
    fn ol_error_outcome_classifies_every_variant() {
        assert_rate_limit(
            &ol_error_outcome(&ProviderFetchError::RateLimited, 300),
            "OL RateLimited",
        );
        assert_transient(
            &ol_error_outcome(&ProviderFetchError::Transient, 300),
            "OL Transient",
        );
        assert_permanent(
            &ol_error_outcome(&ProviderFetchError::Other("HTTP 403".to_string()), 300),
            "OL Other(403)",
        );
        assert_permanent(
            &ol_error_outcome(&ProviderFetchError::Other("HTTP 400".to_string()), 300),
            "OL Other(400)",
        );
        assert!(matches!(
            ol_error_outcome(&ProviderFetchError::NotFound, 300),
            ProviderOutcome::NotFound
        ));
        assert!(matches!(
            ol_error_outcome(
                &ProviderFetchError::CircuitOpen(Duration::from_secs(17)),
                300
            ),
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                ..
            }
        ));
        assert!(matches!(
            ol_error_outcome(&ProviderFetchError::QueueFull(Duration::from_secs(1)), 300),
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::QueueFull,
                ..
            }
        ));
    }

    /// Consumes exactly one retry-budget attempt (Unit A / budget-exempt
    /// set): `apply_budget_rules` (livrarr-enrichment) exempts only
    /// `CircuitOpen` from attempt-counting — every other `WillRetry` reason,
    /// including the new `RateLimit`/`ServerError` paths this unit adds,
    /// falls through its generic arm and IS counted. This guardrails that
    /// invariant at the type level: neither helper ever emits `RateLimit` or
    /// `ServerError` bundled with `CircuitOpen`'s exemption semantics.
    #[test]
    fn rate_limit_and_transient_are_never_circuit_open() {
        let rl = rate_limit_outcome();
        assert!(!matches!(
            rl,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                ..
            }
        ));
        let transient = ol_error_outcome(&ProviderFetchError::Transient, 300);
        assert!(!matches!(
            transient,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                ..
            }
        ));
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
            self.report_gr_payload_usable();
            return ProviderOutcome::Success(Box::new(self.normalize(&detail_url, detail)));
        }
        if let Some(res) = self.llm_extract_payload(&html, language, &detail_url).await {
            return match res {
                Ok(mut payload) => {
                    if payload.gr_key.is_none() {
                        payload.gr_key = Some(gr_key.to_string());
                    }
                    self.report_gr_payload_usable();
                    ProviderOutcome::Success(Box::new(payload))
                }
                Err(err) => self.map_fetch_err(err),
            };
        }
        self.unreadable_page_outcome(&detail_url)
    }

    /// A readable book payload is the only real evidence Goodreads is working —
    /// a bare 200 is not (the page can be an unparseable shell). Reported here
    /// rather than in the fetch helper so an endpoint that answers but serves
    /// nothing usable cannot look healthy, and cannot clear accumulated
    /// failures on its way past.
    fn report_gr_payload_usable(&self) {
        outbound_queue::shared().report_outcome(RateBucket::Goodreads, BreakerSignal::Success);
    }

    /// The health half of an unreadable page, separated from the outcome half
    /// because the search tier keeps a degraded key-only `Success` (Unit B4)
    /// where the established-key tier returns `WillRetry`. Both fetched a page
    /// and both failed to read it, so both owe the breaker the same signal —
    /// reporting it only on the `WillRetry` path let a Goodreads layout break
    /// stay invisible for every work that reached GR by title search.
    fn report_gr_page_unreadable(&self, detail_url: &str) {
        tracing::warn!(
            detail_url = %detail_url,
            "Goodreads: 200 with no readable book payload"
        );
        outbound_queue::shared().report_outcome(RateBucket::Goodreads, BreakerSignal::Failure);
    }

    /// A 200 carrying no readable book is Goodreads' problem, not evidence the
    /// book is absent (PO ruling, 2026-07-26). Filing it as `NotFound` wrote the
    /// book off, left the provider status line dark, and taught the breaker
    /// nothing — so a layout change on their side would quietly empty a library
    /// one refresh at a time.
    fn unreadable_page_outcome(&self, detail_url: &str) -> ProviderOutcome<NormalizedWorkDetail> {
        self.report_gr_page_unreadable(detail_url);
        ProviderOutcome::WillRetry {
            reason: livrarr_domain::WillRetryReason::ServerError,
            next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
        }
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
        //
        // Unit B4: when `work.gr_key` is unset, `resolved.url` came from
        // `search_resolve_detail` — Goodreads' own scraped `bookUrl` field
        // (untrusted input). Fetch through the SSRF-safe method so a
        // valid-looking GR URL whose response redirects to a loopback/
        // private address is still rejected (the established-gr_key branch
        // above builds a trusted, self-constructed URL and stays on
        // `fetch_by_anchor`/`fetch_detail_by_key`'s unrestricted fetch).
        let html =
            match goodreads::fetch_goodreads_html_ssrf_safe(&self.fetcher, &detail_url, priority)
                .await
            {
                Ok(h) => h,
                Err(err) => {
                    if !had_gr_key {
                        if let Some(payload) = self
                            .fallback_key_payload(
                                work,
                                &resolved_gr_key,
                                &resolved.candidate,
                                priority,
                            )
                            .await
                        {
                            // Unit B4 #12 (PO-accepted degrade, made
                            // observable): an SSRF-rejected fetch must stay
                            // distinguishable in logs from an ordinary fetch
                            // failure, even though both degrade to the same
                            // key-only Success below.
                            if matches!(err, GoodreadsFetchError::SsrfRejected(_)) {
                                tracing::warn!(
                                    gr_key = payload.gr_key.as_deref().unwrap_or(""),
                                    "GR detail fetch blocked by SSRF guard (target or \
                                     redirect resolved to a private/reserved address); \
                                     degrading to key-only payload"
                                );
                            } else {
                                tracing::info!(
                                    gr_key = payload.gr_key.as_deref().unwrap_or(""),
                                    verified = payload.title.is_some(),
                                    "GR page fetch failed; returning key payload"
                                );
                            }
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
            self.report_gr_payload_usable();
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
                    self.report_gr_payload_usable();
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
                // The page WAS fetched and WAS unreadable. The key-only degrade
                // stays (Unit B4, PO-accepted), but the breaker must still hear
                // about it: returning a bare `Success` here meant a Goodreads
                // layout break was silent for every work resolved by title
                // search, which is exactly the class C3 exists to close.
                self.report_gr_page_unreadable(&detail_url);
                tracing::info!(
                    gr_key = payload.gr_key.as_deref().unwrap_or(""),
                    verified = payload.title.is_some(),
                    "GR parse failed; returning key payload"
                );
                return ProviderOutcome::Success(Box::new(payload));
            }
        }

        self.unreadable_page_outcome(&detail_url)
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
        search_resolve_detail(
            &self.fetcher,
            &self.base_url,
            &work.title,
            &work.author_name,
            work.isbn_13.as_deref().filter(|s| !s.is_empty()),
            priority,
        )
        .await
    }

    fn map_fetch_err(&self, err: GoodreadsFetchError) -> ProviderOutcome<NormalizedWorkDetail> {
        let backoff = chrono::Duration::seconds(self.retry_backoff_secs);
        match err {
            GoodreadsFetchError::CircuitOpen(retry_after) => circuit_open_outcome(retry_after),
            GoodreadsFetchError::QueueFull(retry_after) => queue_full_outcome(retry_after),
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
            // 404/410 is the only shape that means "this book is not here" —
            // the house rule every other provider already follows
            // (`ProviderFetchError::NotFound`, types.rs).
            GoodreadsFetchError::HttpStatus(404) | GoodreadsFetchError::HttpStatus(410) => {
                ProviderOutcome::NotFound
            }
            // Any other 4xx is Goodreads REFUSING us, not answering us. Filing
            // a refusal as NotFound told the user "no book was found for that
            // identifier — double-check the value", left the provider status
            // dot dark while every request was being turned away, and taught
            // the breaker nothing. It is a failure: it records as an error and
            // retries under the shared budget.
            GoodreadsFetchError::HttpStatus(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + backoff,
            },
            // The autocomplete route has no "absent" status, so NO status from
            // it may become `NotFound` — a moved or blocked route would
            // otherwise report every queried book as missing while the identity
            // path terminalized on it. 429 keeps its own classification so the
            // rate-limit backoff is unchanged.
            GoodreadsFetchError::SearchRouteFailure(429) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::RateLimit,
                next_attempt_at: Utc::now() + backoff,
            },
            GoodreadsFetchError::SearchRouteFailure(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + backoff,
            },
            GoodreadsFetchError::Network(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + backoff,
            },
            // Unit B4 #12 (PO-accepted): identical mapping to `Network`
            // above — this variant exists only to make an SSRF rejection
            // legible in logs at the call site, not to change its outcome.
            GoodreadsFetchError::SsrfRejected(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + backoff,
            },
            // A 200 whose body yielded no usable fields is a page we could not
            // read, not a book that does not exist — the same misfiling as the
            // refusal above, arrived at through a different door.
            GoodreadsFetchError::Parse => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + backoff,
            },
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

/// Search-tier resolution for a work with no established GR key: the seed's
/// own ISBN first (when it has one), then title. Free function over the
/// fetcher so both tiers are testable with the crate's recording double.
///
/// ISBN tier: autocomplete indexes ISBNs, and an ISBN query returns the
/// seed's OWN edition — whose detail page carries that same ISBN, so the
/// REQ-024 trust gate's edition bridge (`payload.isbn_13 == seed.isbn_13`)
/// corroborates a grey subtitled-from-bare title with no trust-rule change.
/// The hit must clear the same junk filter and deterministic picker as any
/// other candidate: an edition whose display title doesn't match the seed
/// (decorated printings) abstains here and falls through to the title tier.
/// A fetch error on this tier also falls through — the title tier keeps
/// today's error semantics, so a GR outage behaves exactly as before.
async fn search_resolve_detail<F: livrarr_domain::services::HttpFetcher>(
    fetcher: &F,
    base_url: &str,
    title: &str,
    author: &str,
    isbn_13: Option<&str>,
    priority: RequestPriority,
) -> Result<Option<ResolvedGrDetail>, GoodreadsFetchError> {
    if let Some(isbn) = isbn_13 {
        match goodreads::search_goodreads(fetcher, base_url, isbn, priority).await {
            Ok(isbn_hits) => {
                if let Some(idx) = gr_best_match(title, author, &isbn_hits) {
                    // Unit B4: `detail_url` is Goodreads' own scraped JSON
                    // field ("bookUrl") — untrusted input. Validate the RAW
                    // value before it can ever become a fetch target; an
                    // unsafe URL is rejected outright, never trusted, exactly
                    // like a title/author mismatch (falls through below).
                    if goodreads::validate_detail_url(&isbn_hits[idx].detail_url) {
                        tracing::debug!(
                            title = %title,
                            isbn,
                            chosen_idx = idx,
                            "GR ISBN-tier hit selected (deterministic)"
                        );
                        return Ok(Some(ResolvedGrDetail {
                            url: goodreads::resolve_detail_url(
                                base_url,
                                &isbn_hits[idx].detail_url,
                            ),
                            candidate: Some(GrCandidateText {
                                title: isbn_hits[idx].title.clone(),
                                title_bare: isbn_hits[idx].title_bare.clone(),
                                author: isbn_hits[idx].author.clone(),
                                cover_url: isbn_hits[idx].cover_url.clone(),
                                series_name: isbn_hits[idx].series_name.clone(),
                                series_position: isbn_hits[idx].series_position,
                            }),
                        }));
                    }
                    tracing::warn!(
                        title = %title,
                        isbn,
                        // Unit B4 #19: never log the untrusted candidate's
                        // full path/query (security-model-policy.md:91) —
                        // origin only.
                        url = %livrarr_http::normalized_origin(&isbn_hits[idx].detail_url)
                            .unwrap_or_else(|| "<unparseable>".to_string()),
                        "GR ISBN-tier hit rejected: detail_url failed SSRF validation"
                    );
                }
                tracing::debug!(
                    title = %title,
                    isbn,
                    hit_count = isbn_hits.len(),
                    "GR ISBN tier: no confident match — falling through to title search"
                );
            }
            Err(e) => {
                tracing::warn!(
                    title = %title,
                    isbn,
                    error = ?e,
                    "GR ISBN-tier autocomplete failed — falling through to title search"
                );
            }
        }
    }

    // Title tier: search GR by title via the WAF-free autocomplete endpoint,
    // then a deterministic best-match pick (no LLM). A fetch error here is
    // most often GR rate-limiting / anti-bot during a bulk burst — log it
    // (previously a silent `?`, which hid these failures) and still
    // propagate so map_fetch_err can schedule a retry.
    let mut hits = match goodreads::search_goodreads(fetcher, base_url, title, priority).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(title = %title, error = ?e, "GR autocomplete failed (likely rate-limit/anti-bot)");
            return Err(e);
        }
    };

    if hits.is_empty() && !title.is_ascii() {
        let ascii_title: String = title.chars().filter(|c| c.is_ascii()).collect();
        if !ascii_title.trim().is_empty() {
            hits = match goodreads::search_goodreads(fetcher, base_url, &ascii_title, priority)
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
        // Unit B4: same raw-bookUrl validation as the ISBN tier above.
        if goodreads::validate_detail_url(&hits[idx].detail_url) {
            tracing::debug!(title = %title, chosen_idx = idx, "GR search result selected (deterministic)");
            return Ok(Some(ResolvedGrDetail {
                url: goodreads::resolve_detail_url(base_url, &hits[idx].detail_url),
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
        tracing::warn!(
            title = %title,
            // Unit B4 #19: origin only — see the ISBN-tier site above.
            url = %livrarr_http::normalized_origin(&hits[idx].detail_url)
                .unwrap_or_else(|| "<unparseable>".to_string()),
            "GR title-tier hit rejected: detail_url failed SSRF validation"
        );
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

/// Deterministic Goodreads search-hit selection — no LLM. Drop junk editions
/// (R-7: pre-filtered before candidates are built, so a study-guide/summary
/// trap-corpus hit can never be picked), then delegate to the one shared
/// identity-grade picker (`identity_matching::pick_best_candidate`,
/// matching-conformance unit) with `accept_grey = true`: GR alone may return
/// a grey subtitled-from-bare pick, because that is the input the ratified
/// `verify_gr_payload` / AC-004 grey-corroboration hatch consumes downstream.
/// The hit's DECORATED title is parsed — decoration "(Series, #N)" reads
/// structurally as a series marker and its volume evidence participates in
/// the veto, while a subtitled record still matches a bare seed on
/// main-title equality (the 2026-07-03 refresh-residue shape). Ranking:
/// `Same` beats `Grey`, higher grey score beats lower, earliest hit wins
/// ties (GR relevance order). Returns None when nothing qualifies — a wrong
/// GR key is worse than none, so GR abstains.
fn gr_best_match(
    title: &str,
    author: &str,
    hits: &[goodreads::GoodreadsSearchResult],
) -> Option<usize> {
    let kept: Vec<(usize, (String, String))> = hits
        .iter()
        .enumerate()
        .filter(|(_, h)| !is_gr_junk_edition(&h.title))
        .map(|(i, h)| (i, (h.title.clone(), h.author.clone().unwrap_or_default())))
        .collect();
    let pairs: Vec<(String, String)> = kept.iter().map(|(_, p)| p.clone()).collect();
    livrarr_domain::identity_matching::pick_best_candidate(title, author, &pairs, true)
        .map(|pick| kept[pick].0)
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

    fn goodreads_test_client() -> GoodreadsClient {
        goodreads_client_at("https://www.goodreads.com")
    }

    fn goodreads_client_at(base_url: &str) -> GoodreadsClient {
        GoodreadsClient::new(
            livrarr_http::fetcher::HttpFetcherImpl::new().expect("fetcher"),
            HttpClient::builder().build().expect("http client"),
            base_url,
        )
    }

    /// The production door for an established Goodreads key, driven end to end
    /// against a real socket — `GoodreadsClient` owns a concrete fetcher, so no
    /// double can be injected and only a real server exercises this path.
    ///
    /// A 200 carrying a page we cannot read is Goodreads' problem, not evidence
    /// the book is absent. Before this, the door returned `NotFound`: the book
    /// was written off, the provider status line stayed dark, and the breaker
    /// was told the provider had SUCCEEDED — so a layout change on their side
    /// would empty a library one refresh at a time with nothing going red.
    ///
    /// This is the case the earlier unit test could not reach: it called the
    /// error mapper directly with an error value no production path constructs.
    #[tokio::test]
    async fn an_unreadable_goodreads_page_is_a_provider_failure_not_a_missing_book() {
        use livrarr_domain::services::RateBucket;
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_gr_breaker().await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Goodreads,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        // A 200 whose body carries no book fields at all.
        let base =
            crate::test_support::spawn_canned_http_server(200, "<html><body>nope</body></html>")
                .await;
        let client = goodreads_client_at(&base);

        let outcome = client
            .fetch_detail_by_key("10884", Some("en"), RequestPriority::Normal)
            .await;

        // Scoped so the permit this may hand back is released immediately —
        // holding one for the rest of the test occupies the bucket's in-flight
        // slot and stalls anything else reaching for the same bucket.
        let tripped = {
            let admission = queue
                .acquire(RateBucket::Goodreads, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        queue.reset_breaker_for_tests(RateBucket::Goodreads);

        assert!(
            !matches!(outcome, ProviderOutcome::NotFound),
            "an unreadable page must not be reported as a missing book, got {outcome:?}"
        );
        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: livrarr_domain::WillRetryReason::ServerError,
                    ..
                }
            ),
            "expected a retryable provider failure, got {outcome:?}"
        );
        assert!(
            tripped,
            "an unreadable page must report a breaker failure, not a success"
        );
    }

    /// The other way an unreadable page could look healthy: the legacy JSON-LD
    /// parser accepted ANY block declaring `"@type":"Book"`, even one carrying
    /// no other field, and built an all-`None` payload from it. That counted as
    /// a successful parse, so a stub shell reported `Success` to the breaker
    /// and cleared every accumulated failure on its way past — the same defect
    /// C3 removed from the bare-2xx report, one layer down.
    #[tokio::test]
    async fn a_jsonld_stub_carrying_no_book_fields_is_not_a_readable_page() {
        use livrarr_domain::services::RateBucket;
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_gr_breaker().await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Goodreads,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let base = crate::test_support::spawn_canned_http_server(
            200,
            r#"<html><head><script type="application/ld+json">{"@type":"Book"}</script></head><body></body></html>"#,
        )
        .await;
        let client = goodreads_client_at(&base);

        let outcome = client
            .fetch_detail_by_key("10884", Some("en"), RequestPriority::Normal)
            .await;

        let tripped = {
            let admission = queue
                .acquire(RateBucket::Goodreads, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        queue.reset_breaker_for_tests(RateBucket::Goodreads);

        assert!(
            !matches!(outcome, ProviderOutcome::Success(_)),
            "a JSON-LD stub with no book fields is not a readable payload, got {outcome:?}"
        );
        assert!(
            tripped,
            "an empty JSON-LD stub must report a breaker failure, not a success"
        );
    }

    /// The breaker clears every accumulated failure on a success, so any
    /// endpoint that reports success on a bare 2xx can mask a sibling endpoint
    /// refusing every request: success, failure, success, failure — the count
    /// never reaches the threshold and the breaker never opens.
    ///
    /// Runs at the PRODUCTION threshold deliberately. A test that forces the
    /// threshold to 1 cannot observe this shape at all, which is exactly why
    /// the earlier one missed it.
    #[tokio::test]
    async fn repeated_unreadable_pages_still_reach_the_production_breaker_threshold() {
        use livrarr_domain::services::RateBucket;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_gr_breaker().await;
        let queue = outbound_queue::shared();
        // Production config for this bucket — no threshold override.
        queue.reset_breaker_for_tests(RateBucket::Goodreads);

        let base =
            crate::test_support::spawn_canned_http_server(200, "<html><body>nope</body></html>")
                .await;
        let client = goodreads_client_at(&base);

        // Well past any sane threshold; if a stray success were clearing the
        // count, this would never open.
        for _ in 0..10 {
            let _ = client
                .fetch_detail_by_key("10884", Some("en"), RequestPriority::Normal)
                .await;
        }

        // Scoped so the permit this may hand back is released immediately —
        // holding one for the rest of the test occupies the bucket's in-flight
        // slot and stalls anything else reaching for the same bucket.
        let tripped = {
            let admission = queue
                .acquire(RateBucket::Goodreads, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        queue.reset_breaker_for_tests(RateBucket::Goodreads);

        assert!(
            tripped,
            "a provider serving nothing readable must eventually trip its breaker"
        );
    }

    /// A 404 from the AUTOCOMPLETE route is a dead route, not a missing book.
    /// Autocomplete has no "this book is absent" status — an empty result set is
    /// a 200 carrying `[]` — so if Goodreads moves or blocks that route, every
    /// work without a stored `gr_key` was being written off as absent, one
    /// terminal `NotFound` at a time, until the breaker happened to open.
    ///
    /// Drives the real door: `GoodreadsClient::fetch` on a work with no
    /// `gr_key`, against a real socket answering 404.
    #[tokio::test]
    async fn a_dead_autocomplete_route_is_not_a_missing_book() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        let base = crate::test_support::spawn_canned_http_server(404, "").await;
        let client = goodreads_client_at(&base);

        let work = Work {
            title: "Sapiens".to_string(),
            author_name: "Yuval Noah Harari".to_string(),
            ..Default::default()
        };

        let outcome = client.fetch(&work, RequestPriority::Normal).await;

        assert!(
            !matches!(outcome, ProviderOutcome::NotFound),
            "a dead autocomplete route must not report the book as absent, got {outcome:?}"
        );
    }

    /// The classification defect behind the identity modal telling the user
    /// "No book was found for that identifier — double-check the value" while
    /// Goodreads was in fact refusing every request. Only a genuine 404/410
    /// means the book is not there; every other refusal is a failure, and a
    /// page that loads but cannot be parsed is not an absent book either.
    #[test]
    fn a_goodreads_refusal_is_classified_as_failure_not_as_a_missing_book() {
        let client = goodreads_test_client();

        for status in [404u16, 410] {
            assert!(
                matches!(
                    client.map_fetch_err(GoodreadsFetchError::HttpStatus(status)),
                    ProviderOutcome::NotFound
                ),
                "HTTP {status} is a genuine absence"
            );
        }

        for status in [400u16, 401, 403, 451] {
            let outcome = client.map_fetch_err(GoodreadsFetchError::HttpStatus(status));
            assert!(
                matches!(
                    outcome,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        ..
                    }
                ),
                "HTTP {status} is a refusal, not a missing book: got {outcome:?}"
            );
        }

        // A 200 whose body yielded nothing is a page we could not read.
        assert!(matches!(
            client.map_fetch_err(GoodreadsFetchError::Parse),
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                ..
            }
        ));
    }

    /// Guards the two classifications the change deliberately left alone.
    #[test]
    fn goodreads_rate_limit_and_server_error_classifications_are_unchanged() {
        let client = goodreads_test_client();

        assert!(matches!(
            client.map_fetch_err(GoodreadsFetchError::HttpStatus(429)),
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::RateLimit,
                ..
            }
        ));
        assert!(matches!(
            client.map_fetch_err(GoodreadsFetchError::HttpStatus(503)),
            ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                ..
            }
        ));
    }

    /// The consumer that made the misfiling user-visible: a refusal must record
    /// as an error, which is what turns the provider dot in the left nav red.
    /// `not_found` is deliberately excluded from that count, so filing a
    /// refusal as NotFound left the dot dark while every request was refused.
    #[test]
    fn a_refusal_records_as_an_error_and_a_genuine_miss_does_not() {
        let client = goodreads_test_client();

        let (refusal_class, _) =
            outcome_record_class(&client.map_fetch_err(GoodreadsFetchError::HttpStatus(403)));
        assert_eq!(refusal_class, CallOutcomeClass::Error);

        let (miss_class, _) =
            outcome_record_class(&client.map_fetch_err(GoodreadsFetchError::HttpStatus(404)));
        assert_eq!(miss_class, CallOutcomeClass::NotFound);
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
    fn picker_rejects_shared_surname_author_grey() {
        let hits = vec![hit("Storm Front", Some("Storm Front"), "Jane Smith")];
        assert_eq!(gr_best_match("Storm Front", "John Smith", &hits), None);
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

    #[tokio::test]
    async fn isbn_tier_selects_the_seed_edition_before_title_search() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        let body = r#"[{"title":"Sapiens","bookTitleBare":"Sapiens","bookUrl":"/book/show/135802293","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            Some("9781529913934"),
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .expect("the ISBN-tier hit must resolve");

        assert!(resolved.url.ends_with("/book/show/135802293"));
        let reqs = fetcher.requests();
        assert_eq!(
            reqs.len(),
            1,
            "a confident ISBN-tier pick makes no title query"
        );
        assert!(reqs[0].url.ends_with("q=9781529913934"));
    }

    #[tokio::test]
    async fn isbn_tier_decorated_edition_title_falls_through_to_title_search() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        // The seed's edition exists on GR but its display title is decorated
        // beyond the picker's bar — the tier abstains and the title tier
        // still runs, preserving today's behavior for everything else.
        let isbn_body = r#"[{"title":"Sapiens (10 Year Anniversary Edition) /anglais","bookTitleBare":"Sapiens (10 Year Anniversary Edition) /anglais","bookUrl":"/book/show/135802293","author":{"name":"Yuval Noah Harari"}}]"#;
        let title_body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"/book/show/23692271","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, isbn_body.as_bytes().to_vec());
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 200,
            headers: vec![],
            body: title_body.as_bytes().to_vec(),
        }));

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            Some("9781529913934"),
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .expect("the title tier must still produce the grey subtitled pick");

        assert!(resolved.url.ends_with("/book/show/23692271"));
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 2, "ISBN query first, then the title query");
        assert!(reqs[0].url.ends_with("q=9781529913934"));
        assert!(reqs[1].url.ends_with("q=Sapiens"));
    }

    #[tokio::test]
    async fn isbn_tier_fetch_error_falls_through_to_title_search() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        let title_body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"/book/show/23692271","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::RateLimited,
        );
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 200,
            headers: vec![],
            body: title_body.as_bytes().to_vec(),
        }));

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            Some("9781529913934"),
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .expect("an ISBN-tier fetch error must not kill the title tier");

        assert!(resolved.url.ends_with("/book/show/23692271"));
        assert_eq!(fetcher.requests().len(), 2);
    }

    #[tokio::test]
    async fn no_isbn_seed_goes_straight_to_title_search() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        let body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"/book/show/23692271","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            None,
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .expect("the title tier is unchanged for a no-ISBN seed");

        assert!(resolved.url.ends_with("/book/show/23692271"));
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].url.ends_with("q=Sapiens"));
    }

    // =========================================================================
    // Unit B4: the raw scraped bookUrl must be SSRF-validated before it can
    // become a fetch target.
    // =========================================================================

    #[tokio::test]
    async fn title_tier_accepts_relative_and_absolute_https_book_urls() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        // Positive control alongside the rejection tests below — a
        // same-host relative or absolute bookUrl still resolves normally
        // through the new validation guard.
        for book_url in [
            "/book/show/23692271",
            "https://www.goodreads.com/book/show/23692271",
        ] {
            let body = format!(
                r#"[{{"title":"Sapiens: A Brief History of Humankind","bookUrl":"{book_url}","author":{{"name":"Yuval Noah Harari"}}}}]"#
            );
            let fetcher =
                crate::test_support::RecordingHttpFetcher::with_ok(200, body.into_bytes());

            let resolved = search_resolve_detail(
                &fetcher,
                "https://www.goodreads.com",
                "Sapiens",
                "Yuval Noah Harari",
                None,
                RequestPriority::Normal,
            )
            .await
            .unwrap()
            .expect("a same-host bookUrl (relative or absolute) must resolve");

            assert!(resolved.url.ends_with("/book/show/23692271"));
        }
    }

    #[tokio::test]
    async fn title_tier_rejects_external_host_book_url() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        // Goodreads' own scraped JSON is untrusted input — a bookUrl
        // pointing off Goodreads must never become a fetch target, even
        // when the title/author otherwise match. GR abstains exactly like a
        // title/author mismatch (no confident, SAFE match).
        let body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"https://evil.com/book/show/1","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            None,
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(resolved.is_none(), "an off-host bookUrl must never resolve");
    }

    #[tokio::test]
    async fn title_tier_rejects_http_scheme_book_url() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        // validate_detail_url requires https — a plain-http bookUrl (even on
        // the right host) must never resolve.
        let body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"http://www.goodreads.com/book/show/1","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            None,
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(
            resolved.is_none(),
            "a plain-http bookUrl must never resolve"
        );
    }

    #[tokio::test]
    async fn title_tier_rejects_private_ip_book_url() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        // A bookUrl pointing at a loopback/private address must never
        // resolve — the raw-value gate rejects it outright, before any
        // connection is ever attempted (fetch_ssrf_safe's redirect-hop check
        // is the second, independent layer for a URL that passes this gate
        // but whose *response* redirects somewhere unsafe).
        let body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"http://127.0.0.1/admin","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, body.as_bytes().to_vec());

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            None,
            RequestPriority::Normal,
        )
        .await
        .unwrap();

        assert!(
            resolved.is_none(),
            "a private-IP bookUrl must never resolve"
        );
    }

    #[tokio::test]
    async fn isbn_tier_rejects_invalid_book_url_and_still_falls_through_to_title_tier() {
        let _guard = crate::test_support::lock_gr_breaker().await;
        // The ISBN tier's hit clears the title/author bar but its bookUrl is
        // unsafe — it must be rejected WITHOUT ever being trusted, and the
        // title tier still gets its normal chance (mirrors the existing
        // decorated-title fallthrough behavior above).
        let isbn_body = r#"[{"title":"Sapiens","bookTitleBare":"Sapiens","bookUrl":"https://evil.com/x","author":{"name":"Yuval Noah Harari"}}]"#;
        let title_body = r#"[{"title":"Sapiens: A Brief History of Humankind","bookUrl":"/book/show/23692271","author":{"name":"Yuval Noah Harari"}}]"#;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, isbn_body.as_bytes().to_vec());
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 200,
            headers: vec![],
            body: title_body.as_bytes().to_vec(),
        }));

        let resolved = search_resolve_detail(
            &fetcher,
            "https://www.goodreads.com",
            "Sapiens",
            "Yuval Noah Harari",
            Some("9781529913934"),
            RequestPriority::Normal,
        )
        .await
        .unwrap()
        .expect("the title tier must still resolve after the ISBN tier's bookUrl is rejected");

        assert!(resolved.url.ends_with("/book/show/23692271"));
        assert_eq!(
            fetcher.requests().len(),
            2,
            "ISBN tier attempted (and rejected), then the title tier"
        );
    }

    #[test]
    fn rejected_detail_url_is_redacted_to_origin_only_before_logging() {
        // Unit B4 #19: the two SSRF-rejection `tracing::warn!` sites above
        // (ISBN-tier and title-tier) must never emit the untrusted, scraped
        // `bookUrl` candidate's full path/query — only its origin
        // (security-model-policy.md:91). There's no tracing-capture crate in
        // this crate's dependency graph (no tracing-subscriber/tracing-test)
        // and hand-rolling a capturing `Subscriber` for one LOW-severity
        // assertion would be disproportionate, so this pins the actual
        // helper both call sites now use (`livrarr_http::normalized_origin`,
        // reused rather than reinventing a redaction routine) against the
        // same off-host bookUrl shape `title_tier_rejects_external_host_book_url`
        // exercises above.
        let evil = "https://evil.com/book/show/1?ref=leak-me";
        let redacted = livrarr_http::normalized_origin(evil).expect("evil.com parses with a host");
        assert_eq!(redacted, "https://evil.com");
        assert!(!redacted.contains("book/show"));
        assert!(!redacted.contains("leak-me"));
    }

    #[tokio::test]
    async fn ssrf_rejected_detail_fetch_still_degrades_to_key_only_success() {
        // Unit B4 #12 — PINS the PO-accepted degrade: a search-tier hit
        // whose detail page sits on a private/reserved address must still
        // resolve to a key-only `ProviderOutcome::Success`, exactly like any
        // other fetch failure, never a hard failure that starves the
        // identity quorum of a GR key. Any FUTURE change to this must be
        // deliberate.
        //
        // `GoodreadsClient` is hardwired to the concrete `HttpFetcherImpl`
        // (not generic over `HttpFetcher`), so this drives the REAL fetcher
        // end-to-end rather than a test double. The autocomplete hit's
        // `bookUrl` is a relative path, which resolves against `base_url`
        // (this loopback test server) — so the "detail" fetch targets
        // 127.0.0.1, which the real SSRF preflight in `fetch_ssrf_safe_impl`
        // rejects exactly as it would reject a redirect to a private
        // address (same `FetchError::Ssrf` / `GoodreadsFetchError::
        // SsrfRejected` path either way — a loopback target is the simplest
        // way to trigger it without standing up a multi-hop redirect
        // chain). Confirmed by a temporary diagnostic print during
        // development that the error reaching `GoodreadsClient::fetch`
        // here really is `SsrfRejected(_)`, not some other transport
        // failure that happens to degrade the same way — the precise
        // variant-mapping guarantee itself lives in the narrower
        // `map_transport_err_distinguishes_ssrf_rejection_from_generic_network_failure`
        // test in `goodreads/client.rs`.
        //
        // Driving a REAL fetcher means this test needs the shared Goodreads
        // breaker CLOSED: an Open breaker refuses admission before a socket is
        // opened, so the autocomplete request is never made and the join below
        // waits forever on a connection that is not coming.
        let _guard = crate::test_support::lock_gr_breaker().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let body = br#"[{"title":"Sapiens","bookUrl":"/book/show/1","author":{"name":"Yuval Noah Harari"}}]"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.write_all(body).await;
            let _ = socket.shutdown().await;
        });

        let fetcher = livrarr_http::fetcher::HttpFetcherImpl::new().unwrap();
        let http = HttpClient::builder().build().unwrap();
        let client = GoodreadsClient::new(fetcher, http, base_url);

        let work = Work {
            title: "Sapiens".to_string(),
            author_name: "Yuval Noah Harari".to_string(),
            ..Default::default()
        };

        let outcome = client.fetch(&work, RequestPriority::Normal).await;
        // Bounded so a request that never arrives fails with this message
        // instead of wedging the whole test binary.
        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("the autocomplete request never reached the test server")
            .unwrap();

        match outcome {
            ProviderOutcome::Success(payload) => {
                assert_eq!(
                    payload.gr_key.as_deref(),
                    Some("1"),
                    "key-only degrade must still carry the resolved gr_key"
                );
            }
            other => panic!(
                "an SSRF-rejected detail fetch must still degrade to a key-only \
                 Success, got {other:?}"
            ),
        }
    }
}
