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
    CallOperation, CallOutcomeClass, ProviderCallRecord, ProviderCallSink,
};
use livrarr_domain::{AnchorQuery, MetadataProvider, WillRetryReason, Work};
use livrarr_http::HttpClient;

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
    pub async fn fetch(&self, work: &Work) -> ProviderOutcome<NormalizedWorkDetail> {
        match self {
            Self::Stub(s) => s.fetch(work).await,
            Self::Audnexus(a) => a.fetch(work).await,
            Self::Hardcover(h) => h.fetch(work).await,
            Self::OpenLibrary(o) => o.fetch(work).await,
            Self::Goodreads(g) => g.fetch(work).await,
            Self::GoogleBooks(g) => g.fetch(work).await,
            Self::Audible(a) => a.fetch(work).await,
        }
    }

    /// Anchor-only enrichment fetch (REQ-006). Each provider accepts exactly
    /// the spec's anchor mapping — GoogleBooks: Isbn13; Goodreads: GrKey ONLY
    /// (the GR ISBN→LLM tier is an identity-surface capability, never an
    /// enrichment fetch); Hardcover: HcKey or Isbn13; OpenLibrary: OlKey or
    /// Isbn13; Audnexus/Audible: Asin. No branch falls back to text search —
    /// those paths live only behind the lookup/identity entry points. A
    /// (provider, query-kind) mismatch is NotFound + warn. Emits one
    /// Enrich-operation ProviderCallRecord per call through the injected sink.
    pub async fn fetch_by_anchor(
        &self,
        query: AnchorQuery,
        language: Option<&str>,
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
            (Self::Stub(s), _) => s.fire().await,
            (Self::Audnexus(a), AnchorQuery::Asin(asin)) => a.fetch_by_asin(asin).await,
            (Self::Hardcover(h), q) => h.fetch_by_anchor_query(q).await,
            (Self::OpenLibrary(o), q) => o.fetch_by_anchor_query(q).await,
            (Self::Goodreads(g), AnchorQuery::GrKey(key)) => {
                g.fetch_detail_by_key(key, language).await
            }
            (Self::GoogleBooks(g), AnchorQuery::Isbn13(isbn)) => g.fetch_by_isbn(isbn).await,
            (Self::Audible(a), AnchorQuery::Asin(asin)) => a.fetch_by_asin(asin).await,
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
            Self::Stub(_) => None,
            Self::Audnexus(a) => a.call_sink.as_ref(),
            Self::Hardcover(h) => h.call_sink.as_ref(),
            Self::OpenLibrary(o) => o.call_sink.as_ref(),
            Self::Goodreads(g) => g.call_sink.as_ref(),
            Self::GoogleBooks(g) => g.sink_ref(),
            Self::Audible(a) => a.sink_ref(),
        }
    }

    /// Inject the call-record sink (REQ-001) into the wrapped client.
    /// Stub clients are test doubles and don't record.
    pub fn with_call_sink(self, sink: Arc<dyn ProviderCallSink>) -> Self {
        match self {
            Self::Stub(c) => Self::Stub(c),
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
/// surface accepts. GR is GrKey ONLY (its ISBN→LLM tier is an identity-surface
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

/// REQ-001 outcome mapping: ProviderOutcome (the control-flow vocabulary) →
/// CallOutcomeClass (the reporting vocabulary), explicit per variant.
/// Conflict/Suppressed never originate from a client fetch; they map to Error
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
        },
        ProviderOutcome::PermanentFailure { reason } => {
            (CallOutcomeClass::Error, Some(format!("{reason:?}")))
        }
        ProviderOutcome::Conflict { .. } => (CallOutcomeClass::Error, Some("conflict".to_string())),
        ProviderOutcome::Suppressed { .. } => {
            (CallOutcomeClass::Error, Some("suppressed".to_string()))
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
    /// Optional fetch delay so tests can drive the resolver's per-call timeout.
    delay: Option<std::time::Duration>,
}

impl StubProviderClient {
    pub fn new(provider: MetadataProvider, outcome: ProviderOutcome<NormalizedWorkDetail>) -> Self {
        Self {
            provider,
            outcome: Arc::new(Mutex::new(outcome)),
            panic_on_call: false,
            call_count: Arc::new(AtomicUsize::new(0)),
            delay: None,
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
        }
    }

    /// Make `fetch` sleep before returning, so a test can exceed the resolver's
    /// `call_timeout` and exercise the abstention path (REQ-025).
    pub fn with_delay(mut self, delay: std::time::Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
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
    http: HttpClient,
    base_url: String,
    retry_backoff_secs: i64,
    cache: AudnexusCache,
    #[allow(dead_code)] // read at green: REQ-001 record emission
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl AudnexusClient {
    pub fn new(http: HttpClient, base_url: impl Into<String>) -> Self {
        Self {
            http,
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

    async fn fetch(&self, work: &Work) -> ProviderOutcome<NormalizedWorkDetail> {
        let result = query_audnexus(
            &self.http,
            &self.base_url,
            work.asin.as_deref(),
            &work.title,
            &work.author_name,
            &self.cache,
        )
        .await;

        match result {
            Ok(Some(audnexus)) => ProviderOutcome::Success(Box::new(audnexus_payload(audnexus))),
            Ok(None) => ProviderOutcome::NotFound,
            Err(_) => ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: Utc::now() + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }

    /// Anchor-only fetch (REQ-006): ASIN lookup with no title/author fallback.
    async fn fetch_by_asin(&self, asin: &str) -> ProviderOutcome<NormalizedWorkDetail> {
        match query_audnexus_by_asin(&self.http, &self.base_url, asin, &self.cache).await {
            Ok(Some(audnexus)) => ProviderOutcome::Success(Box::new(audnexus_payload(audnexus))),
            Ok(None) => ProviderOutcome::NotFound,
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
    #[allow(dead_code)] // read at green: REQ-001 record emission
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl HardcoverClient {
    pub fn new(http: HttpClient, live_config: crate::live_config::LiveMetadataConfig) -> Self {
        Self {
            http,
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
                let normalized = livrarr_domain::normalize_isbn(isbn);
                match crate::hardcover::query_hardcover_by_isbn(
                    &self.http,
                    &normalized,
                    &token,
                    cfg.as_ref(),
                )
                .await
                {
                    Ok(Some(hc)) => self.build_success(hc, &token).await,
                    Ok(None) => ProviderOutcome::NotFound,
                    Err(crate::hardcover::HardcoverError::Http(_)) => ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    },
                    Err(_) => ProviderOutcome::NotFound,
                }
            }
            AnchorQuery::HcKey(key) => {
                tracing::warn!(
                    hc_key = %key,
                    "Hardcover by-key enrichment fetch has no existing query path — NotFound"
                );
                ProviderOutcome::NotFound
            }
            _ => ProviderOutcome::NotFound,
        }
    }

    async fn fetch(&self, work: &Work) -> ProviderOutcome<NormalizedWorkDetail> {
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
            let normalized = livrarr_domain::normalize_isbn(isbn);
            match crate::hardcover::query_hardcover_by_isbn(
                &self.http,
                &normalized,
                &token,
                cfg.as_ref(),
            )
            .await
            {
                Ok(Some(hc)) => {
                    return self.build_success(hc, &token).await;
                }
                Ok(None) => {
                    tracing::debug!(isbn = %normalized, "HC ISBN search: no verified match");
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
            &self.http,
            &work.title,
            &work.author_name,
            &token,
            cfg.as_ref(),
        )
        .await;

        match result {
            Ok(hc) => self.build_success(hc, &token).await,
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

    async fn build_success(
        &self,
        hc: crate::hardcover::HardcoverResult,
        token: &str,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let year = hc
            .publish_date
            .as_deref()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse::<i32>().ok());

        let mut isbn_13 = hc.isbn_13.clone();
        if let Some(ref hc_id) = hc.hc_key {
            if let Ok(Some(better_isbn)) =
                crate::hardcover::fetch_hardcover_editions(&self.http, hc_id, token, "en").await
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
pub struct OpenLibraryClient {
    http: HttpClient,
    retry_backoff_secs: i64,
    #[allow(dead_code)] // read at green: REQ-001 record emission
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl OpenLibraryClient {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
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
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match query {
            AnchorQuery::OlKey(key) => self.detail_by_key(key).await,
            AnchorQuery::Isbn13(isbn) => {
                let normalized = livrarr_domain::normalize_isbn(isbn);
                match self.isbn_lookup(&normalized).await {
                    Ok(Some(ol_work_key)) => self.detail_by_key(&ol_work_key).await,
                    Ok(None) => ProviderOutcome::NotFound,
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

    /// OL detail fetch by work key. `query_ol_detail`'s error is an opaque
    /// string (parse and transport are indistinguishable), so a miss maps to
    /// NotFound — mirroring the seeded fetch's tier behavior, never a text
    /// search.
    async fn detail_by_key(&self, ol_key: &str) -> ProviderOutcome<NormalizedWorkDetail> {
        match query_ol_detail(&self.http, ol_key).await {
            Ok(detail) => ProviderOutcome::Success(Box::new(self.build_payload(ol_key, detail))),
            Err(e) => {
                tracing::debug!(ol_key = %ol_key, error = %e, "OL key detail miss");
                ProviderOutcome::NotFound
            }
        }
    }

    async fn fetch(&self, work: &Work) -> ProviderOutcome<NormalizedWorkDetail> {
        // Tier 1: ISBN lookup
        if let Some(isbn) = work.isbn_13.as_deref().filter(|s| !s.is_empty()) {
            let normalized = livrarr_domain::normalize_isbn(isbn);
            match self.isbn_lookup(&normalized).await {
                Ok(Some(ol_work_key)) => match query_ol_detail(&self.http, &ol_work_key).await {
                    Ok(detail) => {
                        return ProviderOutcome::Success(Box::new(
                            self.build_payload(&ol_work_key, detail),
                        ));
                    }
                    Err(e) => {
                        tracing::debug!(isbn = %normalized, error = %e, "OL ISBN detail miss");
                    }
                },
                Ok(None) => {
                    tracing::debug!(isbn = %normalized, "OL ISBN lookup: no work found");
                }
                Err(e) => {
                    tracing::debug!(isbn = %normalized, error = %e, "OL ISBN lookup failed");
                }
            }
        }

        // Tier 2: ol_key direct lookup (existing behavior)
        if let Some(ol_key) = work.ol_key.as_deref().filter(|s| !s.is_empty()) {
            match query_ol_detail(&self.http, ol_key).await {
                Ok(detail) => {
                    return ProviderOutcome::Success(Box::new(self.build_payload(ol_key, detail)));
                }
                Err(e) => {
                    tracing::debug!(ol_key = %ol_key, error = %e, "OL key detail miss");
                }
            }
        }

        // Tier 3: title+author search fallback
        match self.title_author_search(work).await {
            Ok(Some(payload)) => ProviderOutcome::Success(Box::new(payload)),
            Ok(None) => ProviderOutcome::NotFound,
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
            .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg"));
        NormalizedWorkDetail {
            title: detail.title,
            description: detail.description,
            ol_key: Some(ol_key.to_string()),
            isbn_13: detail.isbn_13,
            cover_url,
            ..Default::default()
        }
    }

    async fn isbn_lookup(&self, isbn: &str) -> Result<Option<String>, String> {
        let url = format!("https://openlibrary.org/isbn/{isbn}.json");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("OL ISBN fetch failed: {e}"))?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("OL ISBN parse error: {e}"))?;

        let ol_work_key = data
            .get("works")
            .and_then(|w| w.as_array())
            .and_then(|arr| arr.first())
            .and_then(|w| w.get("key"))
            .and_then(|k| k.as_str())
            .map(|k| k.strip_prefix("/works/").unwrap_or(k).to_string());

        Ok(ol_work_key)
    }

    async fn title_author_search(
        &self,
        work: &Work,
    ) -> Result<Option<NormalizedWorkDetail>, String> {
        let query = format!("{} {}", work.title, work.author_name);
        let url = format!(
            "https://openlibrary.org/search.json?q={}&fields=key,title,author_name&limit=10",
            urlencoding::encode(&query)
        );

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("OL search failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("OL search returned {}", resp.status()));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("OL search parse error: {e}"))?;

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

        match query_ol_detail(&self.http, &ol_key).await {
            Ok(detail) => Ok(Some(self.build_payload(&ol_key, detail))),
            Err(_) => Ok(None),
        }
    }
}

/// GR-sourced candidate text (a search/autocomplete hit's title + author) —
/// the only text allowed to vouch for a freshly resolved gr_key when the
/// detail page won't parse (#148). Never the work's own title: a
/// hallucinated key must not self-verify.
#[derive(Debug, Clone)]
struct GrCandidateText {
    title: String,
    author: Option<String>,
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
///   2. Otherwise, search by `title author` and use the LLM to disambiguate
///      among hits. GR is a hostile scraping target (anti-bot, HTML drift,
///      noisy results full of study guides and alternate editions) — naive
///      first-hit matching is unreliable, and LLM judgment is required.
///      Without an LLM configured, this path returns `NotFound`. Use
///      Hardcover + OpenLibrary for LLM-free English enrichment; GR
///      contributes cover quality + supplemental fields when LLM is
///      available.
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
    http: HttpClient,
    base_url: String,
    retry_backoff_secs: i64,
    /// Reads `llm_*` per fetch — the LLM extraction fallback for
    /// foreign-language pages activates whenever live config has LLM
    /// configured. None means the client wasn't given a live-config handle
    /// (test / smoke-test path); LLM fallback disabled.
    live_config: Option<crate::live_config::LiveMetadataConfig>,
    #[allow(dead_code)] // read at green: REQ-001 record emission
    call_sink: Option<Arc<dyn ProviderCallSink>>,
}

impl GoodreadsClient {
    pub fn new(http: HttpClient, base_url: impl Into<String>) -> Self {
        Self {
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
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        let detail_url = goodreads::detail_url_for_gr_key(&self.base_url, gr_key);
        let html = match goodreads::fetch_goodreads_html(&self.http, &detail_url).await {
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

    async fn fetch(&self, work: &Work) -> ProviderOutcome<NormalizedWorkDetail> {
        let had_gr_key = work.gr_key.as_deref().is_some_and(|k| !k.is_empty());
        let resolved = match self.resolve_detail_url(work).await {
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
        let html = match goodreads::fetch_goodreads_html(&self.http, &detail_url).await {
            Ok(h) => h,
            Err(err) => {
                if !had_gr_key {
                    if let Some(payload) = self
                        .fallback_key_payload(work, &resolved_gr_key, &resolved.candidate)
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
            return ProviderOutcome::Success(Box::new(self.normalize(&detail_url, detail)));
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
                    return ProviderOutcome::Success(Box::new(payload));
                }
                Err(err) => return self.map_fetch_err(err),
            }
        }

        // All parse paths failed; fall back to a key payload carrying
        // whatever GR-sourced candidate text can vouch for the key.
        if !had_gr_key {
            if let Some(payload) = self
                .fallback_key_payload(work, &resolved_gr_key, &resolved.candidate)
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
    ) -> Option<NormalizedWorkDetail> {
        let key = resolved_gr_key.as_ref()?;

        let confirmed = match candidate {
            Some(c) => Some(c.clone()),
            None => self.confirm_key_via_search(work, key).await,
        };

        let (title, author_name) = match confirmed {
            Some(c) => (Some(c.title), c.author),
            None => (None, None),
        };
        Some(NormalizedWorkDetail {
            title,
            author_name,
            gr_key: Some(key.clone()),
            ..Default::default()
        })
    }

    /// One search round-trip to vouch for an LLM-direct key: returns the
    /// matching hit's text only when the hit's own gr_key equals the key.
    async fn confirm_key_via_search(&self, work: &Work, key: &str) -> Option<GrCandidateText> {
        let hits =
            goodreads::search_goodreads(&self.http, &self.base_url, &work.title, &work.author_name)
                .await
                .ok()?;
        hits.iter()
            .find(|h| goodreads::extract_gr_key(&h.detail_url).as_deref() == Some(key))
            .map(|h| GrCandidateText {
                title: h.title.clone(),
                author: h.author.clone(),
            })
    }

    async fn resolve_detail_url(
        &self,
        work: &Work,
    ) -> Result<Option<ResolvedGrDetail>, GoodreadsFetchError> {
        // 1. work.gr_key — canonical GR identity. No candidate text: the key
        // is already established, nothing needs vouching.
        if let Some(key) = work.gr_key.as_deref().filter(|k| !k.is_empty()) {
            return Ok(Some(ResolvedGrDetail {
                url: goodreads::detail_url_for_gr_key(&self.base_url, key),
                candidate: None,
            }));
        }

        // 2. ISBN search — exact match via isbn: prefix query.
        if let Some(isbn) = work.isbn_13.as_deref().filter(|s| !s.is_empty()) {
            let normalized = livrarr_domain::normalize_isbn(isbn);
            let query = format!("isbn:{normalized}");
            match goodreads::search_goodreads_by_query(&self.http, &self.base_url, &query).await {
                Ok(hits) if !hits.is_empty() => {
                    if let Some(live) = &self.live_config {
                        let cfg = live.snapshot();
                        match gr_llm_disambiguate(
                            &self.http,
                            cfg.as_ref(),
                            &work.title,
                            &work.author_name,
                            &hits,
                        )
                        .await
                        {
                            Ok(Some(idx)) => {
                                tracing::info!(isbn = %normalized, chosen_idx = idx, "LLM selected GR ISBN result");
                                return Ok(Some(ResolvedGrDetail {
                                    url: goodreads::resolve_detail_url(
                                        &self.base_url,
                                        &hits[idx].detail_url,
                                    ),
                                    candidate: Some(GrCandidateText {
                                        title: hits[idx].title.clone(),
                                        author: hits[idx].author.clone(),
                                    }),
                                }));
                            }
                            Ok(None) => {
                                tracing::debug!(isbn = %normalized, "LLM declined all GR ISBN candidates");
                            }
                            Err(e) => {
                                tracing::debug!(isbn = %normalized, error = %e, "GR ISBN LLM disambiguation failed");
                            }
                        }
                    }
                }
                Ok(_) => {
                    tracing::debug!(isbn = %isbn, "GR ISBN search: no results");
                }
                Err(e) => {
                    tracing::debug!(isbn = %isbn, error = ?e, "GR ISBN search failed");
                }
            }
        }

        let title = &work.title;
        let author = &work.author_name;

        // 3. Search GR by title+author (autocomplete — GR's own data) + LLM
        // disambiguation among the REAL hits. Runs before any LLM-recalled
        // key: a key chosen from live hits cannot be a hallucination, and it
        // arrives with candidate text that can vouch for it (#148; PO
        // decision 2026-06-11 after a live LLM-direct key proved wrong).
        let mut hits =
            goodreads::search_goodreads(&self.http, &self.base_url, title, author).await?;

        if hits.is_empty() && !title.is_ascii() {
            let ascii_title: String = title.chars().filter(|c| c.is_ascii()).collect();
            if !ascii_title.trim().is_empty() {
                hits =
                    goodreads::search_goodreads(&self.http, &self.base_url, &ascii_title, author)
                        .await?;
            }
        }

        if !hits.is_empty() {
            if let Some(live) = &self.live_config {
                let cfg = live.snapshot();
                match gr_llm_disambiguate(&self.http, cfg.as_ref(), title, author, &hits).await {
                    Ok(Some(idx)) => {
                        tracing::info!(title = %title, chosen_idx = idx, "LLM selected GR search result");
                        return Ok(Some(ResolvedGrDetail {
                            url: goodreads::resolve_detail_url(
                                &self.base_url,
                                &hits[idx].detail_url,
                            ),
                            candidate: Some(GrCandidateText {
                                title: hits[idx].title.clone(),
                                author: hits[idx].author.clone(),
                            }),
                        }));
                    }
                    Ok(None) => {
                        tracing::debug!(title = %title, "LLM declined all GR candidates");
                    }
                    Err(e) => {
                        tracing::debug!(title = %title, error = %e, "GR LLM disambiguation unavailable");
                    }
                }
            }
        }

        // 4. Last resort: ask the LLM to recall the GR key from its own
        // knowledge — hallucination-prone, so the downstream quarantine
        // (confirm-or-drop, REQ-024) is what makes this safe to keep.
        if let Some(live) = &self.live_config {
            let cfg = live.snapshot();
            match gr_llm_key_lookup(&self.http, cfg.as_ref(), title, author).await {
                Ok(Some(key)) => {
                    tracing::info!(title = %title, gr_key = %key, "LLM resolved GR key directly (unconfirmed)");
                    return Ok(Some(ResolvedGrDetail {
                        url: goodreads::detail_url_for_gr_key(&self.base_url, &key),
                        candidate: None,
                    }));
                }
                Ok(None) => {
                    tracing::debug!(title = %title, "LLM returned no GR key");
                }
                Err(e) => {
                    tracing::debug!(title = %title, error = %e, "LLM GR key lookup unavailable");
                }
            }
        }

        Ok(None)
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

/// Construct a `GoodreadsClient` against the production Goodreads URL.
impl GoodreadsClient {
    pub fn production(http: HttpClient) -> Self {
        Self::new(http, GOODREADS_BASE_URL)
    }
}

/// LLM disambiguation for GR search results — same pattern as HC's llm_disambiguate.
async fn gr_llm_disambiguate(
    http: &HttpClient,
    cfg: &livrarr_domain::settings::MetadataConfig,
    title: &str,
    author: &str,
    hits: &[goodreads::GoodreadsSearchResult],
) -> Result<Option<usize>, String> {
    if !cfg.llm_enabled {
        return Err("LLM disabled".into());
    }
    let endpoint = cfg
        .llm_endpoint
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM not configured")?;
    let api_key = cfg
        .llm_api_key
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM API key not configured")?;
    let model = cfg
        .llm_model
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM model not configured")?;

    let mut candidates = String::new();
    for (i, hit) in hits.iter().enumerate() {
        let a = hit.author.as_deref().unwrap_or("?");
        let y = hit
            .year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "?".into());
        candidates.push_str(&format!("{i}: \"{}\" by {a} ({y})\n", hit.title));
    }

    let prompt = format!(
        "I'm looking for the book \"{title}\" by {author}.\n\n\
         These are search results from Goodreads:\n{candidates}\n\
         Which result (by number) is the correct match? \
         Reject study guides, summaries, SparkNotes, BookRags, and CliffsNotes — those are NOT the real book.\n\
         Reply with ONLY the number. If none match, reply \"none\"."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 10,
        "temperature": 0.0,
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM HTTP {status}: {text}"));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("LLM parse error: {e}"))?;

    let answer = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();

    tracing::debug!(
        candidates_count = hits.len(),
        raw_answer = %answer,
        "GR LLM disambiguation"
    );

    if answer == "none" || answer.is_empty() {
        return Ok(None);
    }

    match answer.parse::<usize>() {
        Ok(idx) if idx < hits.len() => Ok(Some(idx)),
        _ => {
            tracing::warn!(answer = %answer, "GR LLM returned unparseable disambiguation result");
            Ok(None)
        }
    }
}

async fn gr_llm_key_lookup(
    http: &HttpClient,
    cfg: &livrarr_domain::settings::MetadataConfig,
    title: &str,
    author: &str,
) -> Result<Option<String>, String> {
    if !cfg.llm_enabled {
        return Err("LLM disabled".into());
    }
    let endpoint = cfg
        .llm_endpoint
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM not configured")?;
    let api_key = cfg
        .llm_api_key
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM API key not configured")?;
    let model = cfg
        .llm_model
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("LLM model not configured")?;

    let prompt = format!(
        "What is the Goodreads numeric book ID for \"{title}\" by {author}? \
         Return ONLY a JSON object: {{\"gr_id\": \"<numeric_id>\"}}. \
         IMPORTANT: If you are not confident you have the correct ID, \
         return {{\"gr_id\": null}}. Do NOT guess or fabricate an ID. \
         A wrong ID is worse than no ID. No explanation."
    );

    let url = format!(
        "{}chat/completions",
        endpoint.trim_end_matches('/').to_owned() + "/"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 30,
        "temperature": 0.0,
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM HTTP {status}: {text}"));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("LLM parse error: {e}"))?;

    let answer = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let parsed: Result<serde_json::Value, _> = serde_json::from_str(answer);
    let gr_id = parsed
        .ok()
        .and_then(|v| v.get("gr_id")?.as_str().map(String::from));

    match gr_id {
        Some(id) if !id.is_empty() && id != "null" => {
            tracing::debug!(title = %title, gr_key = %id, "LLM provided GR key");
            Ok(Some(id))
        }
        _ => Ok(None),
    }
}
