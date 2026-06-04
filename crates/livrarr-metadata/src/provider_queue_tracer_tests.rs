//! Tracer integration tests for `DefaultProviderQueue`: real provider clients
//! (from `livrarr-external-data`) driven end-to-end through the queue against
//! hand-rolled local HTTP servers. They exercise the queue's scatter-gather and
//! outcome classification, so they live with the queue rather than with the
//! provider clients.

mod audnexus_tracer_tests {
    //! End-to-end smoke test of `ProviderClient::Audnexus` through
    //! `DefaultProviderQueue` against a hand-rolled local HTTP server.
    //!
    //! Purpose: validate that the trait shape (`ProviderClient` enum +
    //! `ProviderQueue::dispatch_enrichment`) actually holds against real
    //! `reqwest`/`HttpClient`/`tokio` plumbing — not just stub clients.
    //! If this compiles and passes, the trait is sound for the rest of the
    //! cutover (Hardcover, OpenLibrary, Goodreads, LLM).

    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::EnrichmentContext;
    use crate::{CircuitBreakerConfig, EnrichmentMode, ProviderQueue, ProviderQueueConfig};
    use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDbCreate};
    use livrarr_domain::{MetadataProvider, RequestPriority, UserRole};
    use livrarr_external_data::{AudnexusClient, ProviderClient, ProviderOutcome};
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
        let (work, _) = db
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

mod goodreads_tracer_tests {
    //! End-to-end smoke test of `ProviderClient::Goodreads` through
    //! `DefaultProviderQueue` against a hand-rolled local HTTP server.
    //!
    //! Mirrors the Audnexus tracer pattern. Two scenarios:
    //!   - direct gr_key lookup against a canned JSON-LD detail page → Success
    //!   - anti-bot challenge body → WillRetry { AntiBotBlock } per IR

    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::EnrichmentContext;
    use crate::{CircuitBreakerConfig, EnrichmentMode, ProviderQueue, ProviderQueueConfig};
    use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDbCreate};
    use livrarr_domain::{MetadataProvider, RequestPriority, UserRole, WillRetryReason};
    use livrarr_external_data::{GoodreadsClient, ProviderClient, ProviderOutcome};
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
        let (work, _) = db
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
