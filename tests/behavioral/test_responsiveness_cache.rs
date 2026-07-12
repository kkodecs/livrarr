//! Responsiveness U-B1 RED behavioral suite.
//!
//! Coverage map:
//! - `prefer_cache_second_pass_uses_cached_success_without_fetching`: REQ-009 / AC-014 zero-second-fetch.
//! - `error_and_not_found_outcomes_are_not_cached`: REQ-009 / AC-014 success-only caching.
//! - `bypass_fetches_and_rewrites_fresh_cache_entry`: REQ-009 / AC-014 user-refresh bypass + rewrite.
//! - `prefer_cache_refetches_stale_entry_and_refreshes_row`: REQ-009 / AC-014 TTL honored.
//! - `eviction_removes_oldest_rows_to_cap`: REQ-009 / AC-015 eviction.
//! - `metadata_cache_toml_parses_section_and_defaults`: REQ-009 / AC-015 TOML config.
//! - `work_service_doors_thread_expected_freshness`: REQ-009 / D-004 freshness door mapping.
//! - `cache_hit_does_not_emit_provider_call_record`: REQ-009 / AC-014 call records instrument real HTTP only.

use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use livrarr_behavioral::stubs::{StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    CreateUserDbRequest, CreateWorkDbRequest, EnrichmentRetryDb, ProviderCacheEntry,
    ProviderResponseCacheDb, ProviderRetryStateDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{CandidateId, CapturedIdentity, IdentityMethod, IdentityState};
use livrarr_domain::identity_matching::identity_key;
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{
    CallOperation, CallOutcomeClass, ProviderCallRecord, ProviderCallSink, RefreshSurface,
    WorkService,
};
use livrarr_domain::{
    Freshness, IdentityStatus, MetadataProvider, RequestPriority, UserId, UserRole, Work,
};
use livrarr_enrichment::{
    DefaultMergeEngine, DefaultProviderQueueBuilder, EnrichmentMode, EnrichmentService,
    EnrichmentServiceImpl, PriorityModel, ProviderQueueConfig,
};
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::work_service::WorkServiceImpl;

#[derive(Default)]
struct CollectingCallSink {
    records: Mutex<Vec<ProviderCallRecord>>,
}

impl CollectingCallSink {
    fn records(&self) -> Vec<ProviderCallRecord> {
        self.records.lock().unwrap().clone()
    }
}

impl ProviderCallSink for CollectingCallSink {
    fn record(&self, rec: ProviderCallRecord) {
        self.records.lock().unwrap().push(rec);
    }
}

fn queue_config(provider: MetadataProvider) -> ProviderQueueConfig {
    ProviderQueueConfig {
        provider,
        max_attempts: 10,
        max_suppressed_passes: 3,
        max_suppression_window_secs: 3600,
    }
}

fn payload(label: &str) -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
        title: Some(format!("{label} title")),
        author_name: Some("Cache Author".to_string()),
        description: Some(format!("{label} description")),
        language: Some("en".to_string()),
        ..NormalizedWorkDetail::default()
    }))
}

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("responsiveness_cache_{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::Admin,
        api_key_hash: format!("apikey_{suffix}"),
    })
    .await
    .expect("seed user")
    .id
}

#[allow(clippy::too_many_arguments)]
async fn seed_work(
    db: &SqliteDb,
    user_id: UserId,
    title: &str,
    provider_anchor: &str,
    provider: MetadataProvider,
) -> Work {
    let (normalized_title, normalized_author) = identity_key(title, "Cache Author");
    let mut req = CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Cache Author".to_string(),
        normalized_title,
        normalized_author,
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    };

    match provider {
        MetadataProvider::GoogleBooks => req.isbn_13 = Some(provider_anchor.to_string()),
        MetadataProvider::Goodreads => req.gr_key = Some(provider_anchor.to_string()),
        MetadataProvider::Hardcover => req.isbn_13 = Some(provider_anchor.to_string()),
        MetadataProvider::OpenLibrary => req.ol_key = Some(provider_anchor.to_string()),
        MetadataProvider::Audnexus | MetadataProvider::Audible => {
            req.asin = Some(provider_anchor.to_string())
        }
        MetadataProvider::Llm | MetadataProvider::Readarr => {}
    }

    let (work, created) = db.create_work(req).await.expect("seed work");
    assert!(created, "fixture should create a fresh work row");
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("seed confirmed identity");
    db.get_work(user_id, work.id)
        .await
        .expect("reload seeded work")
}

fn cache_entry(
    provider: MetadataProvider,
    anchor_type: &str,
    anchor: &str,
    payload_json: &str,
    fetched_at: chrono::DateTime<Utc>,
) -> ProviderCacheEntry {
    ProviderCacheEntry {
        provider,
        anchor_type: anchor_type.to_string(),
        anchor: anchor.to_string(),
        payload_json: payload_json.to_string(),
        fetched_at,
    }
}

fn enrichment_service(
    db: Arc<SqliteDb>,
    provider: MetadataProvider,
    stub: StubProviderClient,
    ttl: Duration,
    max_rows: i64,
    sink: Option<Arc<dyn ProviderCallSink>>,
) -> impl EnrichmentService {
    let mut builder = DefaultProviderQueueBuilder::new().with_provider_cache(ttl, max_rows);
    if let Some(sink) = sink {
        builder = builder.with_call_sink(sink);
    }
    let queue = builder
        .add_provider(provider, ProviderClient::Stub(stub), queue_config(provider))
        .build(db.clone());

    EnrichmentServiceImpl::new(
        db,
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    )
}

async fn run_enrichment<S: EnrichmentService>(
    service: &S,
    user_id: UserId,
    work_id: i64,
    freshness: Freshness,
) {
    service
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Background,
            None,
            RequestPriority::Low,
            freshness,
        )
        .await
        .expect("enrichment pass");
}

fn seed_input(title: &str) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: "Cache Author".to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: None,
        year: Some(2026),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn confirmed_candidate(title: &str, ol_key: &str) -> livrarr_domain::identity::WorkCandidate {
    seed_add_box(
        seed_input(title),
        IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: Some(ol_key.to_string()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: title.to_string(),
                author_name: "Cache Author".to_string(),
                language: Some("en".to_string()),
            },
            method: IdentityMethod::UserSelected,
            score: None,
        },
        Some(CandidateId(format!("candidate-{ol_key}"))),
        false,
    )
}

fn work_service(
    db: SqliteDb,
    workflow: StubEnrichmentWorkflow,
) -> WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher> {
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

#[tokio::test]
async fn prefer_cache_second_pass_uses_cached_success_without_fetching() {
    let db = Arc::new(livrarr_db::test_helpers::create_test_db().await);
    let user_id = create_user(&db, "zero_second_fetch").await;
    let work = seed_work(
        &db,
        user_id,
        "Zero Second Cache",
        "9780000000001",
        MetadataProvider::GoogleBooks,
    )
    .await;
    let google_books = StubProviderClient::new(MetadataProvider::GoogleBooks, payload("gb"));
    let google_books_spy = google_books.clone();
    let service = enrichment_service(
        db.clone(),
        MetadataProvider::GoogleBooks,
        google_books,
        Duration::days(7),
        100,
        None,
    );

    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;
    db.reset_enrichment_for_refresh(user_id, work.id)
        .await
        .expect("clear retry-state so cache, not terminal skip, is tested");
    db.reset_all_retry_states(user_id, work.id)
        .await
        .expect("clear provider terminal row so cache, not terminal skip, is tested");
    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;

    assert_eq!(
        google_books_spy.call_count(),
        1,
        "AC-014: the second PreferCache pass within TTL must be served from \
         the provider-response cache and issue zero GoogleBooks client fetches"
    );
    assert!(
        db.get_provider_cache_entry(MetadataProvider::GoogleBooks, "isbn13", "9780000000001")
            .await
            .expect("read cached GoogleBooks payload")
            .is_some(),
        "AC-014: the zero-fetch second pass must be backed by a real cached \
         success payload, not by provider_retry_state terminal skipping"
    );
}

#[tokio::test]
async fn error_and_not_found_outcomes_are_not_cached() {
    let db = Arc::new(livrarr_db::test_helpers::create_test_db().await);
    let user_id = create_user(&db, "success_only").await;
    let work = seed_work(
        &db,
        user_id,
        "Missing Provider Result",
        "gr_missing",
        MetadataProvider::Goodreads,
    )
    .await;
    let goodreads = StubProviderClient::new(MetadataProvider::Goodreads, ProviderOutcome::NotFound);
    let goodreads_spy = goodreads.clone();
    let service = enrichment_service(
        db.clone(),
        MetadataProvider::Goodreads,
        goodreads,
        Duration::days(7),
        100,
        None,
    );

    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;
    db.reset_enrichment_for_refresh(user_id, work.id)
        .await
        .expect("clear retry-state so cache, not terminal skip, is tested");
    db.reset_all_retry_states(user_id, work.id)
        .await
        .expect("clear provider terminal row so cache, not terminal skip, is tested");
    let cached = db
        .get_provider_cache_entry(MetadataProvider::Goodreads, "gr_key", "gr_missing")
        .await
        .expect("read provider cache");
    assert!(
        cached.is_none(),
        "AC-014: a NotFound provider outcome must not be cached"
    );

    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;
    assert_eq!(
        goodreads_spy.call_count(),
        2,
        "AC-014: not-found/transient outcomes are never pinned for a TTL; \
         the second PreferCache pass must fetch Goodreads again"
    );
}

#[tokio::test]
async fn bypass_fetches_and_rewrites_fresh_cache_entry() {
    let db = Arc::new(livrarr_db::test_helpers::create_test_db().await);
    let user_id = create_user(&db, "bypass_rewrite").await;
    let work = seed_work(
        &db,
        user_id,
        "Bypass Cache",
        "OLBYPASSW",
        MetadataProvider::OpenLibrary,
    )
    .await;
    let old_fetched_at = Utc::now() - Duration::hours(1);
    db.upsert_provider_cache_entry(cache_entry(
        MetadataProvider::OpenLibrary,
        "ol_key",
        "OLBYPASSW",
        r#"{"title":"old cached payload"}"#,
        old_fetched_at,
    ))
    .await
    .expect("seed fresh cache row");
    let open_library = StubProviderClient::new(MetadataProvider::OpenLibrary, payload("fresh ol"));
    let open_library_spy = open_library.clone();
    let service = enrichment_service(
        db.clone(),
        MetadataProvider::OpenLibrary,
        open_library,
        Duration::days(7),
        100,
        None,
    );

    run_enrichment(&service, user_id, work.id, Freshness::Bypass).await;

    assert_eq!(
        open_library_spy.call_count(),
        1,
        "AC-014: Freshness::Bypass must ignore a fresh cache row and fetch OpenLibrary"
    );
    let rewritten = db
        .get_provider_cache_entry(MetadataProvider::OpenLibrary, "ol_key", "OLBYPASSW")
        .await
        .expect("read rewritten cache row")
        .expect("cache row should remain present after bypass rewrite");
    assert_ne!(
        rewritten.payload_json, r#"{"title":"old cached payload"}"#,
        "AC-014: a bypass fetch must overwrite the cached payload"
    );
    assert!(
        rewritten.fetched_at > old_fetched_at,
        "AC-014: a bypass fetch must refresh fetched_at"
    );
}

#[tokio::test]
async fn prefer_cache_refetches_stale_entry_and_refreshes_row() {
    let db = Arc::new(livrarr_db::test_helpers::create_test_db().await);
    let user_id = create_user(&db, "ttl_honored").await;
    let work = seed_work(
        &db,
        user_id,
        "TTL Cache",
        "9780000000002",
        MetadataProvider::Hardcover,
    )
    .await;
    let stale_fetched_at = Utc::now() - Duration::hours(2);
    db.upsert_provider_cache_entry(cache_entry(
        MetadataProvider::Hardcover,
        "isbn13",
        "9780000000002",
        r#"{"title":"stale payload"}"#,
        stale_fetched_at,
    ))
    .await
    .expect("seed stale cache row");
    let hardcover = StubProviderClient::new(MetadataProvider::Hardcover, payload("fresh hc"));
    let hardcover_spy = hardcover.clone();
    let service = enrichment_service(
        db.clone(),
        MetadataProvider::Hardcover,
        hardcover,
        Duration::minutes(5),
        100,
        None,
    );

    let refetch_started_at = Utc::now();
    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;

    assert_eq!(
        hardcover_spy.call_count(),
        1,
        "AC-014: a stale cache row must not satisfy PreferCache"
    );
    let refreshed = db
        .get_provider_cache_entry(MetadataProvider::Hardcover, "isbn13", "9780000000002")
        .await
        .expect("read refreshed cache row")
        .expect("cache row should remain present after stale refresh");
    assert!(
        refreshed.fetched_at >= refetch_started_at,
        "AC-014 (review R-6): fetched_at must be stamped at the actual \
         re-fetch time, not merely newer than the stale seed"
    );
}

#[tokio::test]
async fn eviction_removes_oldest_rows_to_cap() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let base = Utc::now() - Duration::days(1);
    for idx in 0..5 {
        db.upsert_provider_cache_entry(cache_entry(
            MetadataProvider::Audible,
            "asin",
            &format!("ASIN{idx}"),
            &format!(r#"{{"idx":{idx}}}"#),
            base + Duration::minutes(idx),
        ))
        .await
        .expect("seed cache entry");
    }

    let evicted = db
        .evict_provider_cache_to_cap(3)
        .await
        .expect("evict provider cache to cap");
    assert_eq!(evicted, 2, "AC-015: evicted-count must be pinned");

    for idx in 0..2 {
        assert!(
            db.get_provider_cache_entry(MetadataProvider::Audible, "asin", &format!("ASIN{idx}"))
                .await
                .expect("read evicted cache key")
                .is_none(),
            "AC-015: oldest fetched_at row ASIN{idx} should be evicted"
        );
    }
    for idx in 2..5 {
        assert!(
            db.get_provider_cache_entry(MetadataProvider::Audible, "asin", &format!("ASIN{idx}"))
                .await
                .expect("read retained cache key")
                .is_some(),
            "AC-015: newer fetched_at row ASIN{idx} should remain"
        );
    }
    assert_eq!(
        db.count_provider_cache_entries()
            .await
            .expect("count provider cache entries"),
        3,
        "AC-015: cache count should equal the configured cap after eviction"
    );

    // Review R-7: identical fetched_at timestamps must not break the cap.
    // Which of the tied rows survives is unspecified; the cap is not.
    let tied_at = Utc::now() - Duration::hours(3);
    for idx in 0..3 {
        db.upsert_provider_cache_entry(cache_entry(
            MetadataProvider::Audible,
            "asin",
            &format!("TIE{idx}"),
            r#"{"tied":true}"#,
            tied_at,
        ))
        .await
        .expect("seed tied cache entry");
    }
    let evicted_tied = db
        .evict_provider_cache_to_cap(1)
        .await
        .expect("evict tied provider cache to cap");
    assert_eq!(
        evicted_tied, 5,
        "AC-015 (review R-7): eviction over tied timestamps still evicts \
         exactly down to the cap"
    );
    assert_eq!(
        db.count_provider_cache_entries()
            .await
            .expect("count provider cache entries after tied eviction"),
        1,
        "AC-015 (review R-7): the cap holds even when fetched_at ties"
    );
}

#[tokio::test]
async fn metadata_cache_toml_parses_section_and_defaults() {
    let parsed: livrarr_server::config::AppConfig = toml::from_str(
        r#"
        [metadata_cache]
        ttl_days = 3
        max_rows = 42
        "#,
    )
    .expect("parse metadata_cache section");
    assert_eq!(parsed.metadata_cache.ttl_days, 3);
    assert_eq!(parsed.metadata_cache.max_rows, 42);

    let defaults: livrarr_server::config::AppConfig =
        toml::from_str("").expect("parse empty config");
    assert_eq!(
        defaults.metadata_cache.ttl_days, 7,
        "AC-015: absent [metadata_cache] should default ttl_days to 7"
    );
    assert_eq!(
        defaults.metadata_cache.max_rows, 100_000,
        "AC-015: absent [metadata_cache] should default max_rows to 100000"
    );

    let db = livrarr_db::test_helpers::create_test_db().await;
    assert_eq!(
        db.count_provider_cache_entries()
            .await
            .expect("count provider cache entries"),
        0,
        "RED gate: the TOML defaults are paired with the still-red cache storage surface"
    );
}

#[tokio::test]
async fn work_service_doors_thread_expected_freshness() {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_user(&db, "door_mapping").await;
    let refresh_work = seed_work(
        &db,
        user_id,
        "Refresh Door Cache",
        "OLDOORW",
        MetadataProvider::OpenLibrary,
    )
    .await;
    let workflow = StubEnrichmentWorkflow::succeeding();
    let workflow_spy = workflow.clone();
    let service = work_service(db.clone(), workflow);

    service
        .refresh(user_id, refresh_work.id, RefreshSurface::Interactive)
        .await
        .expect("interactive refresh");
    service
        .refresh(user_id, refresh_work.id, RefreshSurface::Bulk)
        .await
        .expect("bulk refresh");
    service
        .add(user_id, confirmed_candidate("Add Door Cache", "OLADDDOORW"))
        .await
        .expect("synchronous add");

    assert_eq!(
        workflow_spy.freshness_calls(),
        vec![Freshness::Bypass, Freshness::Bypass, Freshness::PreferCache],
        "D-004: interactive refresh and bulk refresh must bypass the cache, \
         while the add/ensure background path must prefer cache"
    );
    assert_eq!(
        db.count_provider_cache_entries()
            .await
            .expect("count provider cache entries"),
        0,
        "RED gate: the door-mapping pin runs in the provider-cache suite and \
         should turn green only when the cache DB surface exists"
    );
}

#[tokio::test]
async fn cache_hit_does_not_emit_provider_call_record() {
    let db = Arc::new(livrarr_db::test_helpers::create_test_db().await);
    let user_id = create_user(&db, "call_sink").await;
    let work = seed_work(
        &db,
        user_id,
        "Call Sink Cache",
        "ASIN-CALL-SINK",
        MetadataProvider::Audible,
    )
    .await;
    let sink = Arc::new(CollectingCallSink::default());
    let audible = StubProviderClient::new(MetadataProvider::Audible, payload("audible"));
    let audible_spy = audible.clone();
    let audible_client = ProviderClient::Stub(audible).with_call_sink(sink.clone());
    let queue = DefaultProviderQueueBuilder::new()
        .with_provider_cache(Duration::days(7), 100)
        .add_provider(
            MetadataProvider::Audible,
            audible_client,
            queue_config(MetadataProvider::Audible),
        )
        .build(db.clone());
    let service = EnrichmentServiceImpl::new(
        db.clone(),
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    );

    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;
    let fetch_records: Vec<_> = sink
        .records()
        .into_iter()
        .filter(|record| {
            record.provider == MetadataProvider::Audible.record_key()
                && record.operation == CallOperation::Enrich
                && record.outcome == CallOutcomeClass::Success
        })
        .collect();
    assert_eq!(
        fetch_records.len(),
        1,
        "AC-014: the real fetch pass should produce one provider call record"
    );

    db.reset_enrichment_for_refresh(user_id, work.id)
        .await
        .expect("clear retry-state so cache, not terminal skip, is tested");
    db.reset_all_retry_states(user_id, work.id)
        .await
        .expect("clear provider terminal row so cache, not terminal skip, is tested");
    run_enrichment(&service, user_id, work.id, Freshness::PreferCache).await;
    let all_records = sink.records();
    let audible_records: Vec<_> = all_records
        .iter()
        .filter(|record| record.provider == MetadataProvider::Audible.record_key())
        .collect();
    assert_eq!(
        audible_spy.call_count(),
        1,
        "AC-014: a cache hit should not call the Audible client"
    );
    assert_eq!(
        audible_records.len(),
        1,
        "AC-014: a served-from-cache pass must write no additional provider call record"
    );
}
