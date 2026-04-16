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
use livrarr_db::MetadataConfig;
use livrarr_domain::{MetadataProvider, Work};
use livrarr_http::HttpClient;

use crate::audnexus::query_audnexus;
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
                    narration_type: None,
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
    token: String,
    metadata_cfg: MetadataConfig,
    retry_backoff_secs: i64,
}

impl HardcoverClient {
    pub fn new(http: HttpClient, token: impl Into<String>, metadata_cfg: MetadataConfig) -> Self {
        Self {
            http,
            token: token.into(),
            metadata_cfg,
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
        let result = query_hardcover(
            &self.http,
            &work.title,
            &work.author_name,
            &self.token,
            &self.metadata_cfg,
        )
        .await;

        match result {
            Ok(hc) => {
                let payload = NormalizedWorkDetail {
                    title: hc.title,
                    subtitle: hc.subtitle,
                    original_title: hc.original_title,
                    author_name: None,
                    description: hc.description,
                    year: None,
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
                    isbn_13: hc.isbn_13,
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
            // The legacy String error doesn't discriminate "no results" from
            // network failure. The orchestration cutover will tighten this
            // (typed errors per provider). For now: treat any error as
            // WillRetry — background dispatch defers, manual coerces to merge.
            Err(_) => ProviderOutcome::WillRetry {
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

/// Goodreads adapter — placeholder.
///
/// `livrarr_metadata::goodreads` exposes parsers (`parse_search_html`,
/// `parse_detail_html`, ...) but not an HTTP fetcher; the existing fetcher
/// lives inside `livrarr-server`'s `enrich_foreign_work` orchestration. Pulling
/// it out cleanly requires deciding the GR fetch interface (search-by-title
/// vs detail-by-`detail_url` vs cover-only), which is a design decision that
/// should land alongside the orchestration cutover, not in isolation.
///
/// This variant returns `NotFound` so it's safe to register but never
/// dispatched-against in a production path until properly implemented.
#[derive(Clone, Default)]
pub struct GoodreadsClient;

impl GoodreadsClient {
    pub fn new() -> Self {
        Self
    }

    async fn fetch(
        &self,
        _work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        ProviderOutcome::NotFound
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
