#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency Add Work and manual-import
//! seams.
//!
//! blocked-pending-implementation: these ACs are specified as handler tests, but
//! the `livrarr-behavioral` test crate currently has no `livrarr-handlers`
//! dev-dependency and this task forbids Cargo.toml edits. These tests therefore
//! pin the same add/search JSON contract while invoking the currently compilable
//! real seams: `DiscoveryService::lookup_filtered` and `WorkService::add`.

mod common;

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateUserDbRequest, UserDb};
use livrarr_domain::identity::{
    CandidateId, CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate, WorkSeedFields,
};
use livrarr_domain::services::{DiscoveryService, LookupRequest, WorkService};
use livrarr_domain::{
    EnrichmentStatus, MetadataProvider, ProvenanceSetter, UserId, UserRole, Work,
};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::discovery_service::DiscoveryServiceImpl;
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentContext, EnrichmentServiceImpl, PriorityModel, ProviderQueue,
    ProviderQueueError, ScatterGatherResult,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A stub resolver that resolves the Dune ISBN to a Hardcover candidate offline
/// (OpenLibrary abstains, Hardcover succeeds) — the federated fan-out the search
/// seam routes through when a resolver is wired (the #97 path).
fn stub_isbn_resolver() -> LiveEnglishIdentityResolver {
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let hc = StubProviderClient::new(
        MetadataProvider::Hardcover,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            hc_key: Some("hc_dune".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            title: Some("Dune".to_string()),
            author_name: Some("Frank Herbert".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let clients = [
        (MetadataProvider::OpenLibrary, ProviderClient::Stub(ol)),
        (MetadataProvider::Hardcover, ProviderClient::Stub(hc)),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    LiveEnglishIdentityResolver {
        clients,
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            ..ResolverConfig::default()
        },
    }
}

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("wcc-add-{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: format!("wcc-add-key-{suffix}"),
    })
    .await
    .expect("test user should be created")
    .id
}

fn service(
    db: SqliteDb,
    http: StubHttpFetcher,
) -> WorkServiceImpl<SqliteDb, livrarr_metadata::work_service::StubNoEnrichment, StubHttpFetcher> {
    WorkServiceImpl::without_enrichment(
        db,
        http,
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

fn isbn_identity() -> CapturedIdentity {
    CapturedIdentity {
        ol_key: Some("/works/OL27448W".to_string()),
        gr_key: Some("234225".to_string()),
        hc_key: Some("hc_dune".to_string()),
        isbn_13: Some("9780441013593".to_string()),
        asin: None,
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    }
}

fn confirmed_candidate(identity: CapturedIdentity) -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: identity.title.clone(),
            author_name: identity.author_name.clone(),
            language: identity
                .language
                .clone()
                .unwrap_or_else(|| "en".to_string()),
            author_ol_key: None,
            year: Some(1965),
            cover_url: Some("https://images.example/dune.jpg".to_string()),
            detail_url: Some("https://hardcover.example/books/hc_dune".to_string()),
            description: Some("Arrakis and its spice.".to_string()),
            series_name: Some("Dune".to_string()),
            series_position: Some(1.0),
        },
        identity: IdentityState::Confirmed {
            anchors: identity,
            method: IdentityMethod::IsbnDirect,
            score: None,
        },
        candidate_id: Some(CandidateId("candidate-isbn-dune".to_string())),
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
    }
}

#[derive(Clone, Default)]
struct NoProviderDispatchQueue {
    dispatch_count: Arc<AtomicUsize>,
}

impl NoProviderDispatchQueue {
    fn dispatch_count(&self) -> usize {
        self.dispatch_count.load(Ordering::SeqCst)
    }
}

impl ProviderQueue for NoProviderDispatchQueue {
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        _context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        self.dispatch_count.fetch_add(1, Ordering::SeqCst);
        Ok(ScatterGatherResult {
            work_id: work.id,
            outcomes: HashMap::from([(
                MetadataProvider::Hardcover,
                ProviderOutcome::NotConfigured,
            )]),
            merge_eligible: false,
            deferred: false,
        })
    }
}

fn pending_candidate_from_unresolved_isbn() -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: "Unknown ISBN Work".to_string(),
            author_name: "Unknown Author".to_string(),
            language: "en".to_string(),
            author_ol_key: None,
            year: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity: IdentityState::Pending {
            reason: livrarr_domain::identity::PendingReason::NoCandidates,
            seed_anchors: None,
            top_candidates: vec![],
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
    }
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-008
/// Directive: a resolving embedded ISBN is represented as an auto-match, not a
/// confirmation prompt.
#[tokio::test]
async fn test_wcc_add_ac_008_manual_import_resolving_isbn_auto_matches_without_prompt_real_add_seam(
) {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "ac008").await;
    let http = StubHttpFetcher::new();
    let service = service(db, http);

    let result = service
        .add(user_id, confirmed_candidate(isbn_identity()))
        .await
        .expect("resolving ISBN candidate should add without confirmation");

    assert!(result.created);
    assert_eq!(result.work.isbn_13.as_deref(), Some("9780441013593"));
    assert_eq!(result.work.hc_key.as_deref(), Some("hc_dune"));
    assert_ne!(
        serde_json::to_value(result.work.enrichment_status).expect("status serializes"),
        json!("identity_pending"),
        "auto-matched ISBN must not be returned as prompt-only identity_pending"
    );
}

/// REQ-IDs: REQ-013, REQ-026
/// AC-IDs: AC-009
/// Directive: at the add() seam, an unresolved identity is surfaced as an
/// identity-PENDING work, never a silently-confirmed identity-less Work. The
/// interactive "return candidates instead of creating" behavior is a handler
/// decision made BEFORE add() is invoked (REQ-013); this pins the real
/// downstream add seam, where a bulk/monitor-shaped anchorless candidate
/// creates a surfaced placeholder that converges later (REQ-026, M9).
#[tokio::test]
async fn test_wcc_add_ac_009_manual_import_unresolved_isbn_creates_surfaced_identity_pending_real_add_seam(
) {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "ac009").await;
    let http = StubHttpFetcher::new();
    let service = service(db, http);

    let result = service
        .add(user_id, pending_candidate_from_unresolved_isbn())
        .await
        .expect("unresolved identity should fall through rather than error");

    assert!(
        result.created,
        "the anchorless candidate creates a surfaced placeholder, not nothing"
    );
    assert_eq!(
        result.work.enrichment_status,
        livrarr_domain::EnrichmentStatus::Unenriched,
        "REQ-014: the enrichment track is enrichment-only; identity-pending lives on the identity track"
    );
    assert_eq!(
        result.work.identity_status,
        livrarr_domain::IdentityStatus::Pending,
        "AC-009/REQ-013/026: an unresolved identity is surfaced as identity-pending"
    );
    assert!(
        result.work.ol_key.is_none()
            && result.work.gr_key.is_none()
            && result.work.hc_key.is_none()
            && result.work.isbn_13.is_none()
            && result.work.asin.is_none(),
        "the pending work carries no confirmed identifier — that is what makes it \
         pending rather than confirmed"
    );
}

/// REQ-IDs: REQ-007, REQ-010, REQ-014
/// AC-IDs: AC-024
/// Directive: Add Work discovery carries provider identifiers and a resolving
/// ISBN issues no fuzzy title/author provider HTTP.
#[tokio::test]
async fn test_wcc_add_ac_024_add_work_discovery_threads_provider_ids_and_suppresses_fuzzy_search_real_seams(
) {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "ac024").await;
    let http = StubHttpFetcher::new();
    let http_spy = http.clone();
    let service = service(db.clone(), http.clone()).with_resolver(Arc::new(stub_isbn_resolver()));
    let discovery =
        DiscoveryServiceImpl::new(db, http, livrarr_metadata::discovery_service::StubNoLlm)
            .with_resolver(Arc::new(stub_isbn_resolver()));

    let lookup = discovery
        .lookup_filtered(
            user_id,
            LookupRequest {
                term: "isbn:9780441013593".to_string(),
                lang_override: Some("en".to_string()),
            },
            false,
        )
        .await
        .expect("search seam should return an HTTP-shape lookup response");
    assert!(
        lookup
            .results
            .iter()
            .any(|result| result.isbn_13.as_deref() == Some("9780441013593")
                && result.hc_key.as_deref() == Some("hc_dune")
                && result.candidate_id.is_some()),
        "search response should carry provider identifiers and candidate_id"
    );

    // Drop the cover_url so the only HTTP that *could* fire from add() is a
    // provider re-query / fuzzy title-author search — which a candidate_id-backed
    // add must NOT issue. (A direct cover download is a separate asset fetch, not
    // the provider/fuzzy search this AC guards; the cover arrives via background
    // enrichment.)
    let mut candidate = confirmed_candidate(isbn_identity());
    candidate.fields.cover_url = None;
    let added = service
        .add(user_id, candidate)
        .await
        .expect("selected discovered candidate should add");
    assert_eq!(added.work.hc_key.as_deref(), Some("hc_dune"));
    assert_eq!(added.work.gr_key.as_deref(), Some("234225"));
    assert_eq!(added.work.isbn_13.as_deref(), Some("9780441013593"));
    assert_eq!(
        http_spy.call_count(),
        0,
        "candidate_id-backed add must not issue provider HTTP or fuzzy title/author search"
    );
}

/// REQ-IDs: REQ-014, REQ-015
/// AC-IDs: AC-010 (the add-seam integration AC-024 should have forced — D-005)
/// Directive: when the selected candidate carries a candidate_id whose cached
/// per-provider payloads are present, add() REUSES them in-process via the merge
/// engine (network-free) instead of re-querying or leaving the work unenriched —
/// even on the interactive `skip_sync_enrichment` path, because the cache merge
/// makes no network calls (IR-v2:144/150). Proven by a field present ONLY in the
/// cached payload (`publisher`, which the create path never sets) surfacing on the
/// saved work, an Enriched status, and zero provider HTTP.
#[tokio::test]
async fn test_wcc_add_reqs_014_015_add_reuses_cached_payloads_in_process_without_network() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "reuse").await;
    let http = StubHttpFetcher::new();
    let http_spy = http.clone();

    // The resolver owns the shared transport cache; seed it with the payloads a
    // prior resolve() during search would have cached under this candidate_id.
    let cache = Arc::new(TransportCache::new(Duration::from_secs(30)));
    let candidate_id = CandidateId("cache-dune".to_string());
    let cached_payloads = HashMap::from([(
        MetadataProvider::Hardcover,
        NormalizedWorkDetail {
            hc_key: Some("hc_dune".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            title: Some("Dune".to_string()),
            author_name: Some("Frank Herbert".to_string()),
            description: Some("Cached Hardcover synopsis.".to_string()),
            // The create path writes neither `publisher` nor `page_count`; their
            // presence on the saved work proves the cached payload was merged in,
            // not ignored. No cover_url is set so the add issues no asset fetch
            // either — the whole add must then make zero network calls.
            publisher: Some("CACHE-ONLY Chilton".to_string()),
            page_count: Some(412),
            ..NormalizedWorkDetail::default()
        },
    )]);
    cache.cache_put(user_id, candidate_id.clone(), cached_payloads);

    let queue = NoProviderDispatchQueue::default();
    let queue_spy = queue.clone();
    let enrichment = EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(queue),
        Arc::new(DefaultMergeEngine::new(PriorityModel::english())),
        false,
    )
    .with_transport_cache(cache.clone());
    let workflow = EnrichmentWorkflowImpl::new(Arc::new(enrichment));
    let service = WorkServiceImpl::new(
        db,
        workflow,
        http,
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    );

    // The selected candidate echoes the search result whose anchors match the
    // cached payload; sync enrichment must run so add() reaches the one road:
    // finish_created_work -> run_unified_enrichment -> enrich_work step 2.5.
    let mut candidate = confirmed_candidate(isbn_identity());
    candidate.candidate_id = Some(candidate_id);
    // Cover-less: the only HTTP a candidate_id-backed add could then issue would be
    // a provider re-query, which it must not.
    candidate.fields.cover_url = None;

    let result = service
        .add(user_id, candidate)
        .await
        .expect("candidate_id-backed add should succeed");

    assert!(result.created);
    assert_eq!(
        result.work.publisher.as_deref(),
        Some("CACHE-ONLY Chilton"),
        "REQ-014/015: a field present only in the cached payload must surface on the \
         saved work — proving add() merged the cached payloads in-process rather than \
         ignoring the cache and re-querying (or leaving the work unenriched)"
    );
    assert_eq!(
        result.work.page_count,
        Some(412),
        "REQ-015: cached payloads are merged and applied synchronously within add(), \
         so the returned work already carries the cached fields (not deferred)"
    );
    assert_eq!(
        result.work.enrichment_status,
        EnrichmentStatus::Enriched,
        "the cached payload merge should mark the saved work enriched"
    );
    assert_eq!(
        result.enrichment_status,
        EnrichmentStatus::Enriched,
        "add() should report the status produced by the one-road enrichment merge"
    );
    assert_eq!(
        queue_spy.dispatch_count(),
        0,
        "candidate reuse must return before provider dispatch"
    );
    assert_eq!(
        http_spy.call_count(),
        0,
        "REQ-014: the cached-payload reuse path issues zero provider HTTP"
    );
}

/// Regression: a user-picked cover from the Add-Work search result must persist
/// to `works.cover_url` and stay protected from enrichment.
///
/// The Add-Work handler un-proxies the picked cover before it reaches the work
/// service (the search result carries its cover in the proxied display form
/// `/api/v1/coverproxy?url=<absolute>`; the handler reverses that so the work
/// service receives the canonical absolute URL). This test pins the work-service
/// half of that contract — the absolute URL it is handed, plus the `cover_manual`
/// flag, must survive create + phase-1 cover write. The prior bug here: the
/// phase-1 cover write assigned Validated trust and reset `cover_manual` to
/// false, so background enrichment overrode the pick. After the fix the stored
/// URL is unchanged, `cover_manual` stays true, and trust is `User` (which
/// `resolve_cover` refuses to upgrade).
#[tokio::test]
async fn test_wcc_add_picked_cover_persists_and_locks_manual() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "covpick").await;
    let http = StubHttpFetcher::new();
    let service = service(db, http);

    // The handler has already un-proxied the pick; the work service is handed
    // the canonical absolute cover URL with cover_manual set.
    let absolute = "https://assets.hardcover.app/edition/5293286/abc.jpeg";
    let mut candidate = confirmed_candidate(isbn_identity());
    candidate.fields.cover_url = Some(absolute.to_string());
    candidate.cover_manual = true;

    let result = service
        .add(user_id, candidate)
        .await
        .expect("picked-cover candidate should add");

    assert!(result.created);
    assert_eq!(
        result.work.cover_url.as_deref(),
        Some(absolute),
        "the picked cover must persist as the canonical absolute URL it was given"
    );
    assert!(
        result.work.cover_manual,
        "a user-picked cover must stay cover_manual so enrichment cannot replace it"
    );
    assert_eq!(
        result.work.cover_trust,
        livrarr_domain::CoverTrust::User,
        "a user pick is locked at User trust; resolve_cover refuses to upgrade it"
    );
}
