use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use livrarr_db::{
    create_test_db, CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    identity::CandidateId, normalize_for_matching, EnrichmentStatus, MetadataProvider,
    OutcomeClass, RequestPriority, UserId, UserRole, Work, WorkId,
};
use livrarr_external_data::{
    transport_cache::TransportCache, NormalizedWorkDetail, ProviderOutcome,
};
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentContext, EnrichmentMode, EnrichmentService,
    EnrichmentServiceImpl, PriorityModel, ProviderQueue, ProviderQueueError, ScatterGatherResult,
};
use tokio::sync::Mutex;

#[path = "add_metadata_preservation/incumbent_rank.rs"]
mod add_metadata_preservation_rank;

#[path = "add_metadata_preservation/dates.rs"]
mod add_metadata_preservation_dates;

// Regression tests use the real SQLite, queue, merge and enrichment service.
// Only the external provider clients are fixtures. Startup wiring is checked
// separately through the server's actual composition function.
mod priority_database_regression {
    use super::*;
    use livrarr_db::{ProvenanceDb, ProviderPolicyDb};
    use livrarr_domain::{
        identity_layer::{IdentityProvider, RouteKind},
        Freshness, WorkField,
    };
    use livrarr_external_data::{ProviderClient, StubProviderClient};
    use livrarr_metadata::{DefaultProviderQueueBuilder, ProviderQueueConfig};

    async fn configured_model(db: &livrarr_db::sqlite::SqliteDb) -> PriorityModel {
        let snapshot = db.load_provider_policy_snapshot().await.unwrap();
        let policy = snapshot.for_language("en");
        let mut model = PriorityModel::english();
        model.content = policy.ebook.entries.iter().map(|p| p.provider).collect();
        model.description = model.content.clone();
        model.audio = policy
            .audiobook
            .entries
            .iter()
            .map(|p| p.provider)
            .collect();
        model
    }

    async fn custom_order(db: &livrarr_db::sqlite::SqliteDb) {
        sqlx::query("DELETE FROM provider_policy WHERE language = 'en'")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO provider_policy (language, kind, provider, rank) VALUES
            ('en','ebook','google_books',0), ('en','ebook','hardcover',1),
            ('en','ebook','goodreads',2), ('en','audiobook','audible',0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
    }

    fn detail(provider: MetadataProvider) -> NormalizedWorkDetail {
        NormalizedWorkDetail {
            title: Some("Priority Fixture".into()),
            author_name: Some("Priority Author".into()),
            language: Some("en".into()),
            isbn_13: Some("9780140447934".into()),
            description: Some(format!("{} description", provider.record_key())),
            ..Default::default()
        }
    }

    async fn selected_work(db: &livrarr_db::sqlite::SqliteDb) -> Work {
        let user = livrarr_behavioral::stubs::create_test_user(db).await;
        livrarr_db::test_helpers::settle_work_fixture(
            db,
            user,
            "Priority Fixture",
            "Priority Author",
            Some("en"),
            &[
                (
                    IdentityProvider::IsbnRegistry,
                    RouteKind::Isbn13Edition,
                    "9780140447934",
                ),
                (
                    IdentityProvider::Goodreads,
                    RouteKind::GoodreadsBookEdition,
                    "12345",
                ),
            ],
        )
        .await
    }

    async fn run_selected_order(cached: bool) {
        let db = Arc::new(create_test_db().await);
        custom_order(&db).await;
        let work = selected_work(&db).await;
        if cached {
            // Existing library records may retain their legacy ISBN. The cache
            // admission gate still reads it; seeding it through the actual DB
            // writer tests that supported starting state without changing the
            // route-based identity or weakening cache admission.
            livrarr_domain::services::WorkIdentityRepository::confirm_anchor(
                db.as_ref(),
                work.id,
                livrarr_domain::identity::AnchorType::new("isbn_13"),
                "9780140447934",
                livrarr_domain::identity::AnchorSetter::Import,
            )
            .await
            .unwrap();
        }
        let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
        let mut builder = DefaultProviderQueueBuilder::new();
        let mut probes = Vec::new();
        let mut payloads = HashMap::new();
        for provider in [
            MetadataProvider::Hardcover,
            MetadataProvider::GoogleBooks,
            MetadataProvider::Goodreads,
        ] {
            let payload = detail(provider);
            payloads.insert(provider, payload.clone());
            let client =
                StubProviderClient::new(provider, ProviderOutcome::Success(Box::new(payload)));
            probes.push(client.clone());
            builder = builder.add_provider(
                provider,
                ProviderClient::Stub(client),
                ProviderQueueConfig {
                    provider,
                    max_attempts: 3,
                },
            );
        }
        let queue = Arc::new(builder.build(db.clone()));
        let service = EnrichmentServiceImpl::new(
            db.clone(),
            queue,
            Arc::new(DefaultMergeEngine::new(configured_model(&db).await)),
            false,
        )
        .with_transport_cache(cache.clone());
        let candidate = cached.then(|| CandidateId("priority-cache-fixture".into()));
        if let Some(id) = &candidate {
            cache.cache_put(work.user_id, id.clone(), payloads);
        }
        service
            .enrich_work(
                work.user_id,
                work.id,
                EnrichmentMode::Manual,
                candidate,
                RequestPriority::Normal,
                Freshness::Bypass,
            )
            .await
            .unwrap();
        for client in &probes {
            assert_eq!(
                client.call_count(),
                usize::from(!cached),
                "verify cache/network path before checking priority"
            );
        }
        let saved = db.get_work(work.user_id, work.id).await.unwrap();
        assert_eq!(
            saved.description.as_deref(),
            Some("google_books description"),
            "the saved database order must select the description, including cached reuse"
        );
        let provenance = db
            .list_work_provenance(work.user_id, work.id)
            .await
            .unwrap();
        assert!(provenance.iter().any(|p| p.field == WorkField::Description
            && p.source == Some(MetadataProvider::GoogleBooks)));
    }

    #[tokio::test]
    async fn database_order_controls_live_persisted_description() {
        run_selected_order(false).await;
    }

    #[tokio::test]
    async fn database_order_controls_cached_persisted_description_without_fetch() {
        run_selected_order(true).await;
    }

    #[tokio::test]
    async fn database_seeds_the_requested_english_and_foreign_orders() {
        use MetadataProvider::*;
        let db = create_test_db().await;
        let snapshot = db.load_provider_policy_snapshot().await.unwrap();
        let providers = |list: &livrarr_domain::services::ProviderList| {
            list.entries.iter().map(|p| p.provider).collect::<Vec<_>>()
        };
        assert_eq!(
            providers(&snapshot.for_language("en").ebook),
            vec![
                Hardcover,
                GoogleBooks,
                Goodreads,
                Readarr,
                OpenLibrary,
                Audible
            ]
        );
        assert_eq!(
            providers(&snapshot.for_language("fr").ebook),
            vec![GoogleBooks, Goodreads, Readarr, Audible]
        );
        assert_eq!(
            providers(&snapshot.for_language("en").audiobook),
            vec![
                Audible,
                Audnexus,
                Hardcover,
                Goodreads,
                OpenLibrary,
                GoogleBooks
            ]
        );
    }

    #[tokio::test]
    async fn malformed_database_rank_is_rejected_instead_of_clamped() {
        let db = create_test_db().await;
        sqlx::query("UPDATE provider_policy SET rank = -1 WHERE language = '*' AND provider = 'google_books'")
            .execute(db.pool()).await.unwrap();
        assert!(
            db.load_provider_policy_snapshot().await.is_err(),
            "invalid stored policy must not silently become a different priority"
        );
    }
}

#[derive(Clone, Default)]
struct StubProviderQueue {
    plans: Arc<Mutex<VecDeque<ScatterGatherResult>>>,
    dispatch_count: Arc<Mutex<usize>>,
    persist: Option<(livrarr_db::sqlite::SqliteDb, UserId)>,
}

impl StubProviderQueue {
    fn with_persisted_plans(
        db: livrarr_db::sqlite::SqliteDb,
        user_id: UserId,
        plans: Vec<ScatterGatherResult>,
    ) -> Self {
        Self {
            plans: Arc::new(Mutex::new(plans.into())),
            dispatch_count: Arc::new(Mutex::new(0)),
            persist: Some((db, user_id)),
        }
    }

    async fn dispatch_count(&self) -> usize {
        *self.dispatch_count.lock().await
    }
}

impl ProviderQueue for StubProviderQueue {
    async fn dispatch_enrichment(
        &self,
        _work: &Work,
        _context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        *self.dispatch_count.lock().await += 1;
        let result = self.plans.lock().await.pop_front().ok_or_else(|| {
            ProviderQueueError::Db(livrarr_domain::DbError::Conflict {
                message: "unexpected provider dispatch".to_string(),
            })
        })?;
        if let Some((db, user_id)) = &self.persist {
            persist_scatter_result(db, *user_id, &result).await?;
        }
        Ok(result)
    }
}

async fn persist_scatter_result(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    result: &ScatterGatherResult,
) -> Result<(), ProviderQueueError> {
    use livrarr_db::ProviderRetryStateDb;

    for (provider, outcome) in &result.outcomes {
        match outcome {
            ProviderOutcome::Success(payload) => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::Success,
                    Some(serde_json::to_string(&**payload).map_err(|err| {
                        ProviderQueueError::Db(livrarr_domain::DbError::Io(Box::new(err)))
                    })?),
                )
                .await?;
            }
            ProviderOutcome::NotFound => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::NotFound,
                    None,
                )
                .await?;
            }
            ProviderOutcome::NotConfigured => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::NotConfigured,
                    None,
                )
                .await?;
            }
            ProviderOutcome::PermanentFailure { .. } => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::PermanentFailure,
                    None,
                )
                .await?;
            }
            ProviderOutcome::Conflict { .. } => {
                db.record_terminal_outcome(
                    user_id,
                    result.work_id,
                    *provider,
                    OutcomeClass::Conflict,
                    None,
                )
                .await?;
            }
            ProviderOutcome::WillRetry {
                next_attempt_at, ..
            } => {
                db.record_will_retry(user_id, result.work_id, *provider, *next_attempt_at)
                    .await?;
            }
        }
    }
    Ok(())
}

fn work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    }
}

async fn seed_user_and_work(
    db: &livrarr_db::sqlite::SqliteDb,
    username: &str,
    title: &str,
) -> (UserId, Work) {
    let user = db
        .create_user(CreateUserDbRequest {
            username: username.to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: format!("api-{username}"),
        })
        .await
        .expect("test user should be created");
    let (work, _) = db
        .create_work(work_req(user.id, title, "Contract Author"))
        .await
        .expect("test work should be created");
    (user.id, work)
}

async fn seed_work_for_user(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    title: &str,
    gr_key: Option<&str>,
) -> Work {
    db.create_work(CreateWorkDbRequest {
        gr_key: gr_key.map(str::to_string),
        ..work_req(user_id, title, "Contract Author")
    })
    .await
    .expect("test work should be created")
    .0
}

fn payload_with_cover() -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
        title: Some("Contract Title".to_string()),
        author_name: Some("Contract Author".to_string()),
        description: Some("A provider description".to_string()),
        gr_key: Some("gr-refactor".to_string()),
        isbn_13: Some("9780000000003".to_string()),
        cover_url: Some("https://covers.example.test/ebook.jpg".to_string()),
        ..Default::default()
    }))
}

fn scatter(
    work_id: WorkId,
    outcomes: HashMap<MetadataProvider, ProviderOutcome<NormalizedWorkDetail>>,
) -> ScatterGatherResult {
    ScatterGatherResult {
        work_id,
        outcomes,
        merge_eligible: true,
        deferred: false,
        provider_chase_attempted: true,
        search_leg_fired: false,
        ledger_accounting: livrarr_domain::services::LedgerPassAccounting::Idle,
        search_provider_identity: Vec::new(),
        search_route_proposals: Vec::new(),
    }
}

fn service(
    db: livrarr_db::sqlite::SqliteDb,
    queue: StubProviderQueue,
    cache: TransportCache,
) -> impl EnrichmentService {
    EnrichmentServiceImpl::new(
        Arc::new(db),
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    )
    .with_transport_cache(Arc::new(cache.clone()))
}

#[tokio::test]
async fn add_box_and_author_page_paths_converge_on_same_metadata_and_covers() {
    // AC-001
    let db = create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "add-box".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api-add-box".to_string(),
        })
        .await
        .expect("test user should be created");
    let user_id = user.id;
    // REQ-007: anchors arrive at creation (identity capture), never from the
    // merge — the first-created row carries the gr anchor; the add-box door's
    // create dedups onto the same row.
    let author_page_work =
        seed_work_for_user(&db, user_id, "Add Box Title", Some("gr-refactor")).await;
    let (add_box_work, _) = db
        .create_work(work_req(user_id, "Add Box Title", "Contract Author"))
        .await
        .expect("test work should be created");

    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            add_box_work.id,
            HashMap::from([(MetadataProvider::Hardcover, payload_with_cover())]),
        )],
    );
    let queue_probe = queue.clone();
    let cache = TransportCache::new(Duration::from_secs(60));
    let service = service(db.clone(), queue, cache.clone());

    service
        .enrich_work(
            user_id,
            add_box_work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("network path should enrich");

    let cached_payload = match payload_with_cover() {
        ProviderOutcome::Success(payload) => *payload,
        _ => unreachable!("payload_with_cover always returns a successful payload"),
    };
    cache.cache_put(
        user_id,
        CandidateId("cached-candidate-with-cover".to_string()),
        HashMap::from([(MetadataProvider::Hardcover, cached_payload)]),
    );

    let reuse_result = service
        .enrich_work(
            user_id,
            author_page_work.id,
            EnrichmentMode::Manual,
            Some(CandidateId("cached-candidate-with-cover".to_string())),
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("candidate reuse path should enrich without a second network dispatch");

    assert_eq!(
        queue_probe.dispatch_count().await,
        1,
        "the candidate-reuse door should use cached payloads instead of re-dispatching providers"
    );
    // EXP-SEM-R1-03: candidate reuse is zero-network — cache-served payloads
    // never count as a provider fetch, so the cached continuation reports
    // zero fetch attempts and can never charge the REQ-027 attempt ledger.
    assert!(
        !reuse_result.provider_chase_attempted,
        "a cache-served continuation must not report an attempted provider chase"
    );
    assert_eq!(
        reuse_result.ledger_accounting,
        livrarr_domain::services::LedgerPassAccounting::Idle,
        "a cache-served continuation contributes nothing to the ledger fold"
    );

    let add_box = db.get_work(user_id, add_box_work.id).await.unwrap();
    let author_page = db.get_work(user_id, author_page_work.id).await.unwrap();
    assert_eq!(add_box.title, author_page.title);
    assert_eq!(add_box.description, author_page.description);
    assert_eq!(add_box.gr_key, author_page.gr_key);
    assert_eq!(add_box.isbn_13, author_page.isbn_13);
    assert_eq!(add_box.cover_url, author_page.cover_url);
    assert_eq!(add_box.audiobook_cover_url, author_page.audiobook_cover_url);
}

#[tokio::test]
async fn failed_enrichment_sets_failed() {
    // AC-009
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "failed", "Failed Title").await;
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([(MetadataProvider::Hardcover, ProviderOutcome::NotConfigured)]),
        )],
    );
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("failed provider run should not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Failed);
    assert_eq!(
        db.get_work(user_id, work.id)
            .await
            .unwrap()
            .enrichment_status,
        EnrichmentStatus::Failed
    );
}

#[tokio::test]
async fn unconfigured_provider_is_skipped_remaining_providers_save_the_work() {
    // AC-017
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "skip", "Skip Title").await;
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([
                (MetadataProvider::Hardcover, ProviderOutcome::NotConfigured),
                (MetadataProvider::Goodreads, payload_with_cover()),
            ]),
        )],
    );
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("unconfigured provider must not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
    let saved = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        saved.description,
        Some("A provider description".to_string())
    );
    // The remaining provider still saves its non-cover data. Round 15 keeps
    // Goodreads payload parsing intact but excludes its cover at candidate
    // assembly, so a GR-only scatter has no in-memory cover resolution.
    assert!(result.cover_resolution.is_none());
}

#[tokio::test]
async fn all_providers_no_usable_data_saves_seed_and_lands_thin_or_failed() {
    // AC-018
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "thin", "Thin Title").await;
    let empty_success = ProviderOutcome::Success(Box::new(NormalizedWorkDetail::default()));
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([(MetadataProvider::Hardcover, empty_success)]),
        )],
    );
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("empty provider results must not block the add");

    assert_eq!(result.enrichment_status, EnrichmentStatus::Thin);
    let saved = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(saved.title, "Thin Title");
    assert_eq!(saved.author_name, "Contract Author");
}

#[tokio::test]
async fn one_empty_success_and_rest_errors_lands_thin_not_failed() {
    // AC-020
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db, "mixed", "Mixed Title").await;
    let empty_success = ProviderOutcome::Success(Box::new(NormalizedWorkDetail::default()));
    let queue = StubProviderQueue::with_persisted_plans(
        db.clone(),
        user_id,
        vec![scatter(
            work.id,
            HashMap::from([
                (MetadataProvider::OpenLibrary, empty_success),
                (MetadataProvider::Hardcover, ProviderOutcome::NotConfigured),
            ]),
        )],
    );
    let service = service(
        db.clone(),
        queue,
        TransportCache::new(Duration::from_secs(60)),
    );

    let result = service
        .enrich_work(
            user_id,
            work.id,
            EnrichmentMode::Manual,
            None,
            RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .expect("mixed empty-success plus errors should complete");

    assert_eq!(
        result.enrichment_status,
        EnrichmentStatus::Thin,
        "a successful empty provider response makes the work Thin, not Failed"
    );
}

// OpenAI-authored tests for the actual production startup constructor.
// Append to the existing behavioral pipeline target after product handback.
#[cfg(test)]
mod provider_priority_startup_tests {
    use super::*;
    use livrarr_db::{ConfigDb, UserDb, WorkDb};
    use livrarr_domain::{
        identity::CandidateId,
        identity_layer::{IdentityProvider, RouteKind},
        services::{FetchResponse, ProviderCallRecord, ProviderCallSink},
        Freshness, MetadataProvider as P, RequestPriority,
    };
    use livrarr_external_data::{transport_cache::TransportCache, NormalizedWorkDetail};
    use livrarr_metadata::{EnrichmentMode, EnrichmentService};
    use std::collections::HashMap;
    use std::time::Duration;
    struct Sink;
    impl ProviderCallSink for Sink {
        fn record(&self, _: ProviderCallRecord) {}
    }

    async fn work(
        db: &livrarr_db::sqlite::SqliteDb,
        language: Option<&str>,
    ) -> livrarr_domain::Work {
        let user = db
            .create_user(livrarr_db::CreateUserDbRequest {
                username: "priority-startup".into(),
                password_hash: "fixture".into(),
                role: livrarr_domain::UserRole::Admin,
                api_key_hash: "fixture".into(),
            })
            .await
            .unwrap();
        let settled = livrarr_db::test_helpers::settle_work_fixture(
            db,
            user.id,
            "Priority Fixture",
            "Priority Author",
            language,
            &[(
                IdentityProvider::IsbnRegistry,
                RouteKind::Isbn13Edition,
                "9780140447934",
            )],
        )
        .await;
        // Represent an existing library record with a retained legacy ISBN.
        // Current cache admission still reads that field; route-only newly
        // created Works fall back to fetching, a separate pre-existing issue.
        livrarr_domain::services::WorkIdentityRepository::confirm_anchor(
            db,
            settled.id,
            livrarr_domain::identity::AnchorType::new("isbn_13"),
            "9780140447934",
            livrarr_domain::identity::AnchorSetter::Import,
        )
        .await
        .unwrap();
        db.get_work(user.id, settled.id).await.unwrap()
    }

    async fn startup(
        db: &livrarr_db::sqlite::SqliteDb,
        cache: &Arc<TransportCache>,
        fetcher: &livrarr_http::fetcher::HttpFetcherImpl,
    ) -> Result<Arc<livrarr_server::state::LiveEnrichmentService>, String> {
        let live = livrarr_external_data::live_config::LiveMetadataConfig::new(
            db.get_metadata_config().await.unwrap(),
        );
        let mut config = (*live.snapshot()).clone();
        config.google_books_api_key = Some("fixture-key".into());
        live.replace(config);
        let gr = livrarr_external_data::GoodreadsClient::new(
            fetcher.clone(),
            livrarr_http::HttpClient::builder().build().unwrap(),
            "https://www.goodreads.com",
        );
        let sink: Arc<dyn ProviderCallSink> = Arc::new(Sink);
        livrarr_server::enrichment_composition::build_enrichment_pipeline(
            db,
            fetcher,
            gr,
            &live,
            &sink,
            cache,
            &livrarr_server::config::MetadataCacheConfig {
                ttl_days: 30,
                max_rows: 100,
            },
        )
        .await
        .map(|(_, service)| service)
        .map_err(|e| e.to_string())
    }

    fn no_network() -> livrarr_http::fetcher::HttpFetcherImpl {
        livrarr_http::fetcher::HttpFetcherImpl::new()
            .unwrap()
            .with_scripted_transport(|_| panic!("cached enrichment must not send HTTP"))
    }
    fn payloads(language: Option<&str>, providers: &[P]) -> HashMap<P, NormalizedWorkDetail> {
        providers
            .iter()
            .map(|p| {
                (
                    *p,
                    NormalizedWorkDetail {
                        title: Some("Priority Fixture".into()),
                        author_name: Some("Priority Author".into()),
                        isbn_13: Some("9780140447934".into()),
                        language: language.map(str::to_string),
                        description: Some(format!("{} description", p.record_key())),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }
    async fn cached(
        service: &impl EnrichmentService,
        cache: &TransportCache,
        db: &livrarr_db::sqlite::SqliteDb,
        work: &livrarr_domain::Work,
        providers: &[P],
    ) -> String {
        let id = CandidateId("startup-priority".into());
        cache.cache_put(
            work.user_id,
            id.clone(),
            payloads(work.language.as_deref(), providers),
        );
        service
            .enrich_work(
                work.user_id,
                work.id,
                EnrichmentMode::Manual,
                Some(id),
                RequestPriority::Normal,
                Freshness::Bypass,
            )
            .await
            .unwrap();
        db.get_work(work.user_id, work.id)
            .await
            .unwrap()
            .description
            .unwrap()
    }

    #[tokio::test]
    async fn actual_startup_applies_defaults_and_language_routing() {
        for language in [
            None,
            Some(""),
            Some("en"),
            Some("en-US"),
            Some("eng"),
            Some("fr"),
        ] {
            let db = livrarr_db::create_test_db().await;
            let work = work(&db, language).await;
            let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
            let service = startup(&db, &cache, &no_network()).await.unwrap();
            let expected = if language == Some("fr") {
                "google_books description"
            } else {
                "hardcover description"
            };
            assert_eq!(
                cached(
                    service.as_ref(),
                    &cache,
                    &db,
                    &work,
                    &[P::Hardcover, P::GoogleBooks, P::Goodreads]
                )
                .await,
                expected
            );
            // Compare provider subsets on fresh Works: a saved Hardcover
            // description intentionally outranks later Google-only offers.
            let db = livrarr_db::create_test_db().await;
            let work = self::work(&db, language).await;
            let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
            let service = startup(&db, &cache, &no_network()).await.unwrap();
            assert_eq!(
                cached(
                    service.as_ref(),
                    &cache,
                    &db,
                    &work,
                    &[P::GoogleBooks, P::Goodreads]
                )
                .await,
                "google_books description"
            );
        }
    }
    #[tokio::test]
    async fn actual_startup_snapshot_is_unchanged_until_reconstructed() {
        let db = livrarr_db::create_test_db().await;
        let work = work(&db, Some("en")).await;
        let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
        let fetcher = no_network();
        let before = startup(&db, &cache, &fetcher).await.unwrap();
        sqlx::query("UPDATE provider_policy SET rank = CASE provider WHEN 'google_books' THEN 0 WHEN 'hardcover' THEN 1 ELSE rank END WHERE language='en' AND kind='ebook'")
            .execute(db.pool()).await.unwrap();
        assert_eq!(
            cached(
                before.as_ref(),
                &cache,
                &db,
                &work,
                &[P::Hardcover, P::GoogleBooks]
            )
            .await,
            "hardcover description"
        );
        let after = startup(&db, &cache, &fetcher).await.unwrap();
        assert_eq!(
            cached(
                after.as_ref(),
                &cache,
                &db,
                &work,
                &[P::Hardcover, P::GoogleBooks]
            )
            .await,
            "google_books description"
        );
    }
    #[tokio::test]
    async fn actual_startup_rejects_bad_policy() {
        let db = livrarr_db::create_test_db().await;
        sqlx::query(
            "UPDATE provider_policy SET rank = 999 WHERE language='en' AND provider='hardcover'",
        )
        .execute(db.pool())
        .await
        .unwrap();
        let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
        assert!(startup(&db, &cache, &no_network()).await.is_err());
    }
    #[tokio::test]
    async fn actual_startup_dispatches_google_for_english_and_saves_its_response() {
        use livrarr_http::fetcher::ScriptedTransportOutcome;
        let db = livrarr_db::create_test_db().await;
        let work = work(&db, Some("en")).await;
        let google_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let probe = google_calls.clone();
        let fetcher=livrarr_http::fetcher::HttpFetcherImpl::new().unwrap().with_scripted_transport(move |request| {
            let google=request.url.contains("googleapis.com/books");
            let body=if google {
                probe.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                br#"{"totalItems":1,"items":[{"id":"fixture-volume","volumeInfo":{"title":"Priority Fixture","authors":["Priority Author"],"description":"Google startup description","language":"en","industryIdentifiers":[{"type":"ISBN_13","identifier":"9780140447934"}]}}]}"#.to_vec()
            } else { b"{}".to_vec() };
            ScriptedTransportOutcome::Response {delay:Duration::ZERO, response:FetchResponse {status:if google {200} else {404},headers:vec![("content-type".into(),"application/json".into())],body}}
        });
        let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
        let service = startup(&db, &cache, &fetcher).await.unwrap();
        service
            .enrich_work(
                work.user_id,
                work.id,
                EnrichmentMode::Manual,
                None,
                RequestPriority::Normal,
                Freshness::Bypass,
            )
            .await
            .unwrap();
        assert!(
            google_calls.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "actual startup applicability must admit Google for English"
        );
        assert_eq!(
            db.get_work(work.user_id, work.id)
                .await
                .unwrap()
                .description
                .as_deref(),
            Some("Google startup description")
        );
    }
    #[tokio::test]
    async fn audio_detail_database_order_does_not_change_cover_order() {
        let db = livrarr_db::create_test_db().await;
        let work = work(&db, Some("en")).await;
        sqlx::query("UPDATE provider_policy SET rank=CASE provider WHEN 'hardcover' THEN 0 WHEN 'audible' THEN 2 ELSE rank END WHERE language='en' AND kind='audiobook'").execute(db.pool()).await.unwrap();
        let cache = Arc::new(TransportCache::new(Duration::from_secs(60)));
        let service = startup(&db, &cache, &no_network()).await.unwrap();
        let mut offers = payloads(Some("en"), &[P::Hardcover, P::Audible]);
        for (provider, detail) in &mut offers {
            detail.duration_seconds = Some(if *provider == P::Hardcover {
                1234
            } else {
                5678
            });
            detail.cover_url = Some(format!(
                "https://covers.example.test/{}.jpg",
                provider.record_key()
            ));
        }
        let id = CandidateId("audio-order".into());
        cache.cache_put(work.user_id, id.clone(), offers);
        let result = service
            .enrich_work(
                work.user_id,
                work.id,
                EnrichmentMode::Manual,
                Some(id),
                RequestPriority::Normal,
                Freshness::Bypass,
            )
            .await
            .unwrap();
        assert_eq!(
            db.get_work(work.user_id, work.id)
                .await
                .unwrap()
                .duration_seconds,
            Some(1234)
        );
        assert_eq!(result.audiobook_cover_resolution.unwrap().source, "audible");
        assert_eq!(result.cover_resolution.unwrap().source, "hardcover");
    }
}
