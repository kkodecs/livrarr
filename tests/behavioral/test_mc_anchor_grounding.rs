#![allow(dead_code)]

use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDbCreate};
use livrarr_domain::services::{
    CallOperation, CallOutcomeClass, ProviderCallRecord, ProviderCallSink,
};
use livrarr_domain::{AnchorQuery, MetadataProvider, RequestPriority, Work};
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::{
    CircuitBreakerConfig, DefaultProviderQueueBuilder, EnrichmentContext, EnrichmentMode,
    ProviderQueue, ProviderQueueConfig,
};

#[derive(Default)]
struct RecordingSink {
    records: std::sync::Mutex<Vec<ProviderCallRecord>>,
}

impl RecordingSink {
    fn records(&self) -> Vec<ProviderCallRecord> {
        self.records.lock().expect("recording sink lock").clone()
    }
}

impl ProviderCallSink for RecordingSink {
    fn record(&self, rec: ProviderCallRecord) {
        self.records.lock().expect("recording sink lock").push(rec);
    }
}

fn default_config(provider: MetadataProvider) -> ProviderQueueConfig {
    ProviderQueueConfig {
        provider,
        concurrency: 2,
        requests_per_second: 1000.0,
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

fn context() -> EnrichmentContext {
    EnrichmentContext {
        priority: RequestPriority::High,
        mode: EnrichmentMode::Manual,
    }
}

fn payload(title: &str) -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
        title: Some(title.to_string()),
        description: Some(format!("{title} description")),
        cover_url: Some(format!("https://example.test/{title}.jpg")),
        ..NormalizedWorkDetail::default()
    }))
}

async fn create_work(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: i64,
    isbn_13: Option<&str>,
    gr_key: Option<&str>,
) -> Work {
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Anchor Grounding Work".to_string(),
            author_name: "Grounded Author".to_string(),
            normalized_title: "anchor grounding work".to_string(),
            normalized_author: "grounded author".to_string(),
            isbn_13: isbn_13.map(str::to_string),
            gr_key: gr_key.map(str::to_string),
            language: Some("en".to_string()),
            monitor_ebook: true,
            ..Default::default()
        })
        .await
        .unwrap();
    work
}

#[tokio::test]
async fn test_mc_goodreads_requires_gr_key_and_does_not_use_isbn_anchor() {
    // REQ-006 / AC-007
    let sink = std::sync::Arc::new(RecordingSink::default());
    let goodreads = StubProviderClient::new(MetadataProvider::Goodreads, payload("goodreads"));
    let db = std::sync::Arc::new(create_test_db().await);
    let user_id = create_test_user(db.as_ref()).await;
    let work = create_work(db.as_ref(), user_id, Some("9780140447934"), None).await;
    let queue = DefaultProviderQueueBuilder::new()
        .with_call_sink(sink.clone())
        .add_provider(
            MetadataProvider::Goodreads,
            ProviderClient::Stub(goodreads.clone()),
            default_config(MetadataProvider::Goodreads),
        )
        .build(db);

    let result = queue.dispatch_enrichment(&work, context()).await.unwrap();

    assert_eq!(goodreads.call_count(), 0);
    assert!(matches!(
        result.outcomes.get(&MetadataProvider::Goodreads),
        Some(ProviderOutcome::NotFound)
    ));
    let records = sink.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider, "goodreads");
    assert_eq!(records[0].operation, CallOperation::Enrich);
    assert_eq!(records[0].outcome, CallOutcomeClass::SkippedNoAnchor);
}

#[tokio::test]
async fn test_mc_anchorless_work_skips_all_anchorless_providers_without_fetches() {
    // REQ-006 / AC-007
    let sink = std::sync::Arc::new(RecordingSink::default());
    let db = std::sync::Arc::new(create_test_db().await);
    let user_id = create_test_user(db.as_ref()).await;
    let work = create_work(db.as_ref(), user_id, None, None).await;
    let google_books = StubProviderClient::new(MetadataProvider::GoogleBooks, payload("gb"));
    let goodreads = StubProviderClient::new(MetadataProvider::Goodreads, payload("gr"));
    let hardcover = StubProviderClient::new(MetadataProvider::Hardcover, payload("hc"));

    let queue = DefaultProviderQueueBuilder::new()
        .with_call_sink(sink.clone())
        .add_provider(
            MetadataProvider::GoogleBooks,
            ProviderClient::Stub(google_books.clone()),
            default_config(MetadataProvider::GoogleBooks),
        )
        .add_provider(
            MetadataProvider::Goodreads,
            ProviderClient::Stub(goodreads.clone()),
            default_config(MetadataProvider::Goodreads),
        )
        .add_provider(
            MetadataProvider::Hardcover,
            ProviderClient::Stub(hardcover.clone()),
            default_config(MetadataProvider::Hardcover),
        )
        .build(db);

    let _ = queue.dispatch_enrichment(&work, context()).await.unwrap();

    assert_eq!(google_books.call_count(), 0);
    assert_eq!(goodreads.call_count(), 0);
    assert_eq!(hardcover.call_count(), 0);
    assert_eq!(
        sink.records()
            .iter()
            .filter(|rec| rec.outcome == CallOutcomeClass::SkippedNoAnchor)
            .count(),
        3
    );
}

#[tokio::test]
async fn test_mc_anchor_query_variant_set_has_no_title_author_and_fetch_by_anchor_is_real_surface()
{
    // REQ-006 / AC-007
    let queries = [
        AnchorQuery::Isbn13("9780140447934".to_string()),
        AnchorQuery::GrKey("12345".to_string()),
        AnchorQuery::HcKey("hc-work".to_string()),
        AnchorQuery::OlKey("OL123W".to_string()),
        AnchorQuery::Asin("B000TEST".to_string()),
    ];
    let mut seen = Vec::new();
    for query in queries {
        let name = match query {
            AnchorQuery::Isbn13(_) => "isbn_13",
            AnchorQuery::GrKey(_) => "gr_key",
            AnchorQuery::HcKey(_) => "hc_key",
            AnchorQuery::OlKey(_) => "ol_key",
            AnchorQuery::Asin(_) => "asin",
        };
        seen.push(name);
    }
    assert_eq!(seen, vec!["isbn_13", "gr_key", "hc_key", "ol_key", "asin"]);

    let client = ProviderClient::Stub(StubProviderClient::new(
        MetadataProvider::Goodreads,
        ProviderOutcome::NotFound,
    ));
    let _ = client
        .fetch_by_anchor(
            AnchorQuery::GrKey("12345".to_string()),
            Some("en"),
            RequestPriority::Normal,
        )
        .await;
}

#[tokio::test]
async fn test_mc_provider_client_fetch_by_anchor_mapping_rejects_wrong_anchor_kind() {
    // REQ-006 / AC-007
    let client = ProviderClient::Stub(StubProviderClient::new(
        MetadataProvider::Goodreads,
        ProviderOutcome::NotFound,
    ));

    let outcome = client
        .fetch_by_anchor(
            AnchorQuery::Isbn13("9780140447934".to_string()),
            Some("en"),
            RequestPriority::Normal,
        )
        .await;

    assert!(matches!(outcome, ProviderOutcome::NotFound));
    assert_eq!(client.call_count(), 0);
}
