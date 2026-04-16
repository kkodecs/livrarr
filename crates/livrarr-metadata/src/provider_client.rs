//! Per-provider client seam used by `DefaultProviderQueue`.
//!
//! Real network adapters are added as variants here as the cutover progresses.
//! Phase 1.5 introduces `Audnexus` as the tracer — proves the trait shape holds
//! against real `reqwest`/`tokio`/`HttpClient` plumbing before the rest of the
//! providers (Hardcover, OpenLibrary, Goodreads, LLM) are wired in.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use livrarr_domain::{MetadataProvider, Work};
use livrarr_http::HttpClient;

use crate::audnexus::query_audnexus;
use crate::{EnrichmentContext, NormalizedWorkDetail, ProviderOutcome};

/// Heterogeneous provider client. Enum dispatch instead of `Box<dyn>` because
/// `trait_variant::make(Send)` traits are not dyn-compatible. New real-provider
/// adapters are added as new variants here.
#[derive(Clone)]
pub enum ProviderClient {
    Stub(StubProviderClient),
    Audnexus(AudnexusClient),
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
        }
    }

    pub fn provider(&self) -> MetadataProvider {
        match self {
            Self::Stub(s) => s.provider,
            Self::Audnexus(_) => MetadataProvider::Audnexus,
        }
    }

    pub fn call_count(&self) -> usize {
        match self {
            Self::Stub(s) => s.call_count(),
            // Real network adapters don't track call counts — the queue tracks
            // dispatch counts elsewhere; this accessor exists for stub-driven tests.
            Self::Audnexus(_) => 0,
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
