//! Readarr's actual add/enrichment producer, followed by persisted source ranking.
//! Only the external HTTP exchange is scripted; no settled result, normalized
//! Readarr outcome, provenance row, queue, or merge result is supplied by the test.

use super::{Entry, Fixture, OFFERED, SAVED};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use livrarr_db::{CreateImportDbRequest, ImportDb, ProviderPolicyDb};
use livrarr_domain::{
    identity::{IdentityState, PendingReason},
    seed::{seed_readarr_import, SeedInput, SeedLanguage},
    services::{FetchResponse, SourceProviderData, WorkService},
    MetadataProvider as P,
};
use livrarr_enrichment::{
    DefaultMergeEngine, DefaultProviderQueueBuilder, EnrichmentServiceImpl, ProviderQueueConfig,
};
use livrarr_external_data::{provider_client::OpenLibraryClient, ProviderClient};
use livrarr_http::fetcher::{HttpFetcherImpl, ScriptedTransportOutcome};
use livrarr_metadata::{
    enrichment_workflow_service::EnrichmentWorkflowImpl, work_service::WorkServiceImpl,
};

// REQ-005 / AC-005
#[tokio::test]
async fn setter_006_readarr_add_produces_provider_description_and_receives_source_rank() {
    let db = livrarr_db::test_helpers::create_activated_test_db().await;
    let user_id = livrarr_behavioral::stubs::create_test_user(&db).await;
    let import_id = "readarr-source-rank".to_string();
    db.create_import(CreateImportDbRequest {
        id: import_id.clone(),
        user_id,
        source: "readarr".into(),
        source_url: None,
        target_root_folder_id: None,
    })
    .await
    .expect("create the real import FK target");

    let search_calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = Arc::clone(&search_calls);
    let http = HttpFetcherImpl::new()
        .expect("production HTTP fetcher")
        .with_scripted_transport(move |request| {
            assert!(
                request
                    .url
                    .starts_with("https://openlibrary.org/search.json?"),
                "unexpected external request: {}",
                request.url
            );
            observed_calls.fetch_add(1, Ordering::SeqCst);
            // A deliberate, valid empty search response: no remote provider
            // supplies the description whose Readarr origin is under test.
            ScriptedTransportOutcome::Response {
                delay: Duration::ZERO,
                response: FetchResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: br#"{"numFound":0,"docs":[]}"#.to_vec(),
                },
            }
        });
    let queue = DefaultProviderQueueBuilder::new()
        .add_provider(
            P::OpenLibrary,
            ProviderClient::OpenLibrary(OpenLibraryClient::new(http.clone())),
            ProviderQueueConfig {
                provider: P::OpenLibrary,
                max_attempts: 3,
            },
        )
        .build(Arc::new(db.clone()));
    let engine = DefaultMergeEngine::from_policy(Arc::new(
        db.load_provider_policy_snapshot()
            .await
            .expect("the normal persisted provider policy"),
    ));
    let enrichment = Arc::new(EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(queue),
        Arc::new(engine),
        false,
    ));
    let data_dir = tempfile::tempdir().expect("isolated service data directory");
    let service = WorkServiceImpl::new(
        db.clone(),
        EnrichmentWorkflowImpl::new(enrichment),
        http,
        data_dir.path().to_path_buf(),
    );
    let candidate = seed_readarr_import(
        SeedInput {
            title: "The Winter Expedition".into(),
            author_name: "Ada Rivers".into(),
            language: SeedLanguage::resolve(Some("en"), "en"),
            author_ol_key: None,
            year: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        IdentityState::Pending {
            reason: PendingReason::NoCandidates,
            seed_anchors: None,
            top_candidates: Vec::new(),
        },
        SourceProviderData {
            description: Some(SAVED.into()),
            ..Default::default()
        },
        true,
        true,
        import_id,
    );
    assert!(
        candidate.fields.description.is_none(),
        "the ordinary create seed must not supply the tested description"
    );
    let added = service
        .add(user_id, candidate)
        .await
        .expect("Readarr seed through the actual WorkService add and common enrichment");
    assert!(added.created);
    assert!(
        search_calls.load(Ordering::SeqCst) > 0,
        "the real queue/client must execute the scripted external search"
    );
    assert_eq!(
        added.work.description.as_deref(),
        Some(SAVED),
        "synchronous add returns the retained Readarr description"
    );

    // Attach the small merge fixture to the real service-created Work.
    // No Readarr value or provenance is written by fixture setup here.
    let fixture = Fixture::for_persisted_work(db, user_id, added.work.id).await;
    fixture.assert_provider_description(SAVED, P::Readarr).await;
    let before = fixture.provenance().await;

    // Later offers are controlled merge-boundary inputs. The
    // incumbent they challenge came exclusively from the real producer.
    fixture
        .offer(Entry::Live, P::OpenLibrary, Some(OFFERED))
        .await;
    fixture.assert_retained(&before).await;
    fixture
        .offer(Entry::Live, P::GoogleBooks, Some(OFFERED))
        .await;
    fixture
        .assert_provider_description(OFFERED, P::GoogleBooks)
        .await;
}
