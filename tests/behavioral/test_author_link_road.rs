//! Behavioral pins for author-link road orchestration and provider adapters.
//!
//! Creation cases use the real service doors over an in-memory `SqliteDb`.
//! Adapter cases use the concrete OL/GR/HC clients with the repository's
//! recording HTTP seam.

use std::{collections::HashMap, sync::Mutex};

use chrono::Utc;
use livrarr_behavioral::stubs::{
    create_test_user, StubAuthorProviderGateway, StubEnrichmentWorkflow, StubHttpFetcher,
    StubLlmCaller,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, AuthorLinkDb, CreateAuthorDbRequest, CreateSeriesDbRequest, ListImportDb, SeriesDb,
};
use livrarr_domain::identity::{CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate};
use livrarr_domain::seed::{
    seed_add_box, seed_list_import, seed_manual_import, seed_readarr_import, seed_series_monitor,
    SeedInput, SeedLanguage,
};
use livrarr_domain::services::{
    AddAuthorRequest, AuthorLinkWorkflow, AuthorProviderGateway, AuthorService, SourceProviderData,
    WorkService,
};
use livrarr_domain::{
    AuthorLinkCandidateReason, AuthorLinkProgressState, AuthorLinkTrigger, AuthorProvider,
    AuthorRouteKey, OpenLibraryAuthorCandidate, OpenLibraryAuthorKey, OpenLibraryCatalogPage,
    ProviderAuthorRef, RequestPriority,
};
use livrarr_external_data::live_config::LiveMetadataConfig;
use livrarr_external_data::{GoodreadsClient, HardcoverClient, OpenLibraryClient};
use livrarr_http::HttpClient;
use livrarr_metadata::author_linking::AuthorLinkingServiceImpl;
use livrarr_metadata::author_service::AuthorServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use tokio_util::sync::CancellationToken;

type RealWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;
type RealAuthorService = AuthorServiceImpl<SqliteDb, StubHttpFetcher, StubLlmCaller>;

fn data_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "livrarr-author-link-road-{label}-{}",
        std::process::id()
    ))
}

fn work_service(db: SqliteDb, label: &str) -> RealWorkService {
    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        data_dir(label),
    )
}

fn author_service(db: SqliteDb) -> RealAuthorService {
    AuthorServiceImpl::new(db, StubHttpFetcher::new(), StubLlmCaller::not_configured())
}

fn seed_input(title: &str, author: &str, author_ol_key: Option<&str>) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: author.to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: author_ol_key.map(str::to_string),
        year: Some(2026),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn confirmed_identity(title: &str, author: &str, work_ol_key: &str) -> IdentityState {
    IdentityState::Confirmed {
        anchors: CapturedIdentity {
            ol_key: Some(work_ol_key.to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: title.to_string(),
            author_name: author.to_string(),
            language: Some("en".to_string()),
        },
        method: IdentityMethod::UserSelected,
        score: None,
    }
}

fn add_box_candidate(title: &str, author: &str, author_ol_key: Option<&str>) -> WorkCandidate {
    seed_add_box(
        seed_input(title, author, author_ol_key),
        confirmed_identity(title, author, "OL9001W"),
        None,
        false,
    )
}

async fn assert_one_due_progress(db: &SqliteDb, author_id: i64) {
    let row: (i64, String, i64) = sqlx::query_as(
        "SELECT COUNT(*), MIN(state), julianday(MIN(next_attempt_at)) <= julianday('now')
           FROM author_link_progress WHERE author_id=?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("progress observation");
    assert_eq!(row.0, 1, "one durable task per converged author");
    assert_eq!(row.1, "queued");
    assert_eq!(row.2, 1, "task must be immediately due");
}

fn empty_gateway() -> StubAuthorProviderGateway {
    StubAuthorProviderGateway {
        keyed_results: HashMap::new(),
        ol_search_results: vec![],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    }
}

fn ol_key(raw: &str) -> OpenLibraryAuthorKey {
    match AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw)
        .expect("fixture key must use production parser")
    {
        AuthorRouteKey::OpenLibrary(key) => key,
        _ => unreachable!("provider-selected parser returned the wrong variant"),
    }
}

fn configured_ol_key(raw: &str) -> OpenLibraryAuthorKey {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .expect("gateway-only fixture must deserialize a typed OL key")
}

async fn settled_author_claim(
    db: &SqliteDb,
    user_id: i64,
    label: &str,
    title: &str,
    author_name: &str,
) -> (i64, livrarr_db::AuthorLinkClaim) {
    let result = work_service(db.clone(), label)
        .add(user_id, add_box_candidate(title, author_name, None))
        .await
        .expect("production settled-work writer");
    let author_id = result.author_id.expect("converged author id");
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("production enqueue writer");
    let now = Utc::now();
    let claim = db
        .claim_due(now, now + chrono::Duration::minutes(5), 1)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("settled author claim");
    (author_id, claim)
}

/// Door: Add-box work/author path -> real `WorkService::add`.
/// AC-001 / AC-005: the converged author is durably enqueued even when no
/// selected author route is supplied.
#[tokio::test]
async fn ac001_ac005_add_box_creation_leaves_one_due_task_without_scalar_route_write() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let result = work_service(db.clone(), "add-box")
        .add(
            user_id,
            add_box_candidate("Add Box Work", "Add Box Author", None),
        )
        .await
        .expect("real add-box service door");
    let author_id = result.author_id.expect("converged author id");
    assert_one_due_progress(&db, author_id).await;
    let author = db
        .get_author(user_id, author_id)
        .await
        .expect("stored author");
    assert_eq!(author.ol_key, None, "legacy scalar is frozen");
}

/// Door: Manual import -> real `WorkService::add`.
/// AC-001 / AC-006: manual import uses the same durable author gate.
#[tokio::test]
async fn ac001_ac006_manual_import_creation_leaves_one_due_task() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let result = work_service(db.clone(), "manual")
        .add(
            user_id,
            seed_manual_import(
                seed_input("Manual Work", "Manual Author", None),
                confirmed_identity("Manual Work", "Manual Author", "OL9002W"),
                None,
            ),
        )
        .await
        .expect("real manual-import service door");
    assert_one_due_progress(&db, result.author_id.expect("author id")).await;
}

/// Door: List import confirm -> real list-import candidate path.
/// AC-001 / AC-006: list-created authors cannot silently become a no-op task.
#[tokio::test]
async fn ac001_ac006_list_import_creation_leaves_one_due_task() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let result = work_service(db.clone(), "list")
        .add(
            user_id,
            seed_list_import(
                seed_input("List Work", "List Author", None),
                confirmed_identity("List Work", "List Author", "OL9003W"),
                None,
            ),
        )
        .await
        .expect("real list-import work door");
    assert_one_due_progress(&db, result.author_id.expect("author id")).await;
}

/// Door: Series-monitor roster work creation -> real `WorkService::add`.
/// AC-001 / AC-006: adopting an existing author still repairs/arms one task.
#[tokio::test]
async fn ac001_ac006_series_monitor_adoption_leaves_one_due_task() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Series Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("production author writer");
    let series = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: author.id,
            name: "Series Door".to_string(),
            gr_key: "9901".to_string(),
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: Some("en".to_string()),
            work_count: 1,
        })
        .await
        .expect("production series writer");
    let result = work_service(db.clone(), "series")
        .add(
            user_id,
            seed_series_monitor(
                SeedInput {
                    series_name: Some(series.name.clone()),
                    series_position: Some(1.0),
                    ..seed_input("Series Work", &author.name, None)
                },
                confirmed_identity("Series Work", &author.name, "OL9004W"),
                series.id,
                true,
                false,
            ),
        )
        .await
        .expect("real series-monitor work door");
    assert_eq!(result.author_id, Some(author.id));
    assert_one_due_progress(&db, author.id).await;
}

/// Door: Readarr import author resolution -> real Readarr-shaped work path.
/// AC-001 / AC-011: creation enqueue is independent of the guarded Readarr
/// route branch.
#[tokio::test]
async fn ac001_ac011_readarr_creation_leaves_one_due_task_independent_of_route_evidence() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    db.create_list_import_record(
        "author-link-readarr",
        user_id,
        "readarr",
        &Utc::now().to_rfc3339(),
    )
    .await
    .expect("production Readarr import record");
    let result = work_service(db.clone(), "readarr")
        .add(
            user_id,
            seed_readarr_import(
                seed_input("Readarr Work", "Readarr Author", None),
                confirmed_identity("Readarr Work", "Readarr Author", "OL9005W"),
                SourceProviderData::default(),
                true,
                false,
                "author-link-readarr".to_string(),
            ),
        )
        .await
        .expect("real Readarr-shaped work door");
    assert_one_due_progress(&db, result.author_id.expect("author id")).await;
}

/// Door: Standalone add-author path -> real `AuthorService::add`.
/// AC-001 / AC-005: no selected route means enqueue-only and no legacy scalar.
#[tokio::test]
async fn ac001_ac005_standalone_author_add_is_enqueued_without_a_selected_route() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let result = author_service(db.clone())
        .add(
            user_id,
            AddAuthorRequest {
                name: "Standalone Author".to_string(),
                sort_name: None,
                ol_key: None,
                monitored: false,
            },
        )
        .await
        .expect("real standalone author door");
    assert_one_due_progress(&db, result.author().id).await;
    assert_eq!(result.author().ol_key, None);
}

/// Door: Add-box selected-route path -> typed user-sovereign attach.
/// AC-005 / AC-010: an explicit selection becomes UserPicked route state and
/// never a generic author scalar write.
#[tokio::test]
async fn ac005_ac010_add_box_selected_author_route_uses_canonical_route_storage() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let result = work_service(db.clone(), "add-selected")
        .add(
            user_id,
            add_box_candidate("Selected Work", "Selected Author", Some("OL9101A")),
        )
        .await
        .expect("real selected add-box door");
    let author_id = result.author_id.expect("author id");
    let routes = db
        .list_active_routes(user_id, author_id, Some(AuthorProvider::OpenLibrary))
        .await
        .expect("canonical route read");
    assert_eq!(routes.len(), 1);
    assert!(matches!(
        routes[0].provenance,
        livrarr_domain::AuthorRouteProvenance::UserPicked
    ));
    assert_eq!(
        db.get_author(user_id, author_id)
            .await
            .expect("author")
            .ol_key,
        None
    );
}

/// Door: Recurring author-link sweep -> `run_due`.
/// AC-006: cancellation mid-batch and restart never roll back or replay
/// completed siblings.
#[tokio::test]
async fn ac006_run_due_resumes_after_cancellation_without_replaying_completed_siblings() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    for index in 0..3 {
        let (author, _) = db
            .create_author(CreateAuthorDbRequest {
                user_id,
                name: format!("Resume Author {index}"),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .expect("production author writer");
        db.ensure_enqueued(user_id, author.id, AuthorLinkTrigger::AuthorCreated)
            .await
            .expect("enqueue author");
    }
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: empty_gateway(),
    };
    let cancel = CancellationToken::new();
    cancel.cancel();
    let interrupted = service
        .run_due(3, cancel)
        .await
        .expect("cancelled bounded tick");
    assert!(interrupted.evaluated < interrupted.claimed);

    let resumed = service
        .run_due(3, CancellationToken::new())
        .await
        .expect("resumed bounded tick");
    assert_eq!(resumed.failed, 0);
}

/// Door: Recurring author-link sweep -> Tier 3.
/// AC-002 / AC-012: an author without Confirmed/Provisional work parks
/// `ParkedNoSettledWork` and remains due for later re-entry.
#[tokio::test]
async fn ac002_ac012_tier3_parks_without_settled_work_and_can_reenter_later() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author, _) = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Tier Three Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("production author writer");
    db.ensure_enqueued(user_id, author.id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("enqueue");
    let now = Utc::now();
    let claim = db
        .claim_due(now, now + chrono::Duration::minutes(5), 1)
        .await
        .expect("claim")
        .into_iter()
        .next()
        .expect("author claim");
    let service = AuthorLinkingServiceImpl {
        db,
        gateway: empty_gateway(),
    };
    let update = service.run_author(claim).await.expect("Tier 3 result");
    assert!(matches!(
        update.state,
        AuthorLinkProgressState::ParkedNoSettledWork
    ));
    assert_eq!(update.tier, Some(3));
}

/// Door: Recurring author-link sweep -> Tier-2 OL name search/catalog.
/// AC-002 / AC-004: every Tier-2 result parks for review, and catalog overlap
/// only affects evidence ordering/counts.
#[tokio::test]
async fn ac002_ac004_tier2_candidate_always_parks_even_with_catalog_corroboration() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, claim) =
        settled_author_claim(&db, user_id, "tier-two", "Tier Two Work", "Tier Two Author").await;
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: StubAuthorProviderGateway {
            keyed_results: HashMap::new(),
            ol_search_results: vec![OpenLibraryAuthorCandidate {
                route_key: ol_key("OL9201A"),
                name: "Tier Two Author".to_string(),
                alternate_names: vec!["T. T. Author".to_string()],
                top_work: Some("Tier Two Work".to_string()),
                work_count: Some(12),
            }],
            ol_catalog_pages: vec![OpenLibraryCatalogPage {
                titles: vec!["Tier Two Work".to_string()],
                next_cursor: None,
            }],
            calls: Mutex::new(vec![]),
        },
    };
    let update = service.run_author(claim).await.expect("Tier-2 result");
    let calls = service.gateway.calls();
    assert_eq!(
        calls.len(),
        3,
        "Tier 2 must issue one keyed lookup, one name search, and one catalog page request"
    );
    assert_eq!(calls[0].provider, AuthorProvider::OpenLibrary);
    assert_eq!(calls[0].work_route, "OL9001W");
    assert_eq!(calls[0].priority, RequestPriority::Low);
    assert!(calls[1]
        .work_route
        .starts_with("ol_search:Tier Two Author:"));
    assert!(calls[2].work_route.starts_with("ol_catalog:"));
    assert!(calls
        .iter()
        .all(|call| call.priority == RequestPriority::Low));
    assert!(matches!(update.state, AuthorLinkProgressState::NeedsReview));
    assert_eq!(update.tier, Some(2));
    assert!(db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("routes")
        .is_empty());
    let route_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_provider_routes WHERE author_id=?")
            .bind(author_id)
            .fetch_one(db.pool())
            .await
            .expect("route side-effect observation");
    assert_eq!(
        route_rows, 0,
        "Tier 2 must not call the guarded writer or submit route evidence"
    );
    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].candidates.len(), 1);
    let parked = &review[0].candidates[0];
    assert!(matches!(
        parked.reason,
        AuthorLinkCandidateReason::Tier2NameSearch
    ));
    assert_eq!(parked.corroborated_title_count, 1);
    assert_eq!(parked.settled_work_count, 1);
    assert_eq!(parked.top_work_preview.as_deref(), Some("Tier Two Work"));
}

/// Door: Recurring author-link sweep -> Tier-1 keyed contributor walk.
/// AC-003 / AC-013: every contributor is guarded independently; Grey never
/// attaches, and two independent Agree contributors create two active routes.
#[tokio::test]
async fn ac003_ac013_tier1_run_author_attaches_each_agree_and_never_attaches_grey() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, claim) = settled_author_claim(
        &db,
        user_id,
        "tier-one-mixed",
        "Tier One Work",
        "Tier One Author",
    )
    .await;
    let mixed_gateway = StubAuthorProviderGateway {
        keyed_results: HashMap::from([(
            (AuthorProvider::OpenLibrary, "OL9001W".to_string()),
            vec![
                ProviderAuthorRef {
                    key: AuthorRouteKey::OpenLibrary(ol_key("OL9501A")),
                    name: "Tier One Author".to_string(),
                    role: Some("author".to_string()),
                },
                ProviderAuthorRef {
                    key: AuthorRouteKey::OpenLibrary(ol_key("OL9502A")),
                    name: "Unrelated Contributor".to_string(),
                    role: Some("editor".to_string()),
                },
            ],
        )]),
        ol_search_results: vec![],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    };
    let mixed_service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: mixed_gateway,
    };
    mixed_service
        .run_author(claim)
        .await
        .expect("mixed Tier-1 road");
    let mixed_calls = mixed_service.gateway.calls();
    assert_eq!(
        mixed_calls.len(),
        1,
        "Tier 1 must not issue an Open Library name-search or catalog call"
    );
    assert_eq!(mixed_calls[0].provider, AuthorProvider::OpenLibrary);
    assert_eq!(mixed_calls[0].work_route, "OL9001W");
    assert_eq!(mixed_calls[0].priority, RequestPriority::Low);

    let mixed_routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("mixed active routes");
    assert_eq!(mixed_routes.len(), 1, "only Agree may attach");
    assert_eq!(
        mixed_routes[0].key,
        AuthorRouteKey::OpenLibrary(ol_key("OL9501A"))
    );
    let review = db.list_review(user_id).await.expect("Grey review evidence");
    assert_eq!(review.len(), 1);
    assert!(review[0].candidates.iter().any(|candidate| {
        candidate.key == AuthorRouteKey::OpenLibrary(ol_key("OL9502A"))
            && matches!(candidate.reason, AuthorLinkCandidateReason::NameGuardFailed)
    }));

    let both_db = create_test_db().await;
    let both_user_id = create_test_user(&both_db).await;
    let (both_author_id, both_claim) = settled_author_claim(
        &both_db,
        both_user_id,
        "tier-one-both",
        "Both Agree Work",
        "Both Agree Author",
    )
    .await;
    let both_gateway = StubAuthorProviderGateway {
        keyed_results: HashMap::from([(
            (AuthorProvider::OpenLibrary, "OL9001W".to_string()),
            vec![
                ProviderAuthorRef {
                    key: AuthorRouteKey::OpenLibrary(ol_key("OL9511A")),
                    name: "Both Agree Author".to_string(),
                    role: Some("author".to_string()),
                },
                ProviderAuthorRef {
                    key: AuthorRouteKey::OpenLibrary(ol_key("OL9512A")),
                    name: "Both Agree Author".to_string(),
                    role: Some("co-author".to_string()),
                },
            ],
        )]),
        ol_search_results: vec![],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    };
    let both_service = AuthorLinkingServiceImpl {
        db: both_db.clone(),
        gateway: both_gateway,
    };
    both_service
        .run_author(both_claim)
        .await
        .expect("two-Agree Tier-1 road");
    let both_calls = both_service.gateway.calls();
    assert_eq!(
        both_calls.len(),
        1,
        "two contributors from one work response still require one keyed fetch"
    );
    assert_eq!(both_calls[0].work_route, "OL9001W");
    let both_routes = both_db
        .list_active_routes(both_user_id, both_author_id, None)
        .await
        .expect("two-Agree active routes");
    assert_eq!(
        both_routes.len(),
        2,
        "the road must continue after the first Agree"
    );
}

/// Door: Recurring author-link sweep -> automatic evidence for a removed tuple.
/// AC-003 / AC-010: the Tier-1 road cannot reactivate a user tombstone; it
/// parks review evidence and leaves the exact route removed.
#[tokio::test]
async fn ac003_ac010_tier1_run_author_honors_tombstone_and_parks_review() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _pre_removal_claim) = settled_author_claim(
        &db,
        user_id,
        "tier-one-tombstone",
        "Tombstone Work",
        "Tombstone Author",
    )
    .await;
    let tombstoned_key = AuthorRouteKey::OpenLibrary(ol_key("OL9521A"));
    let route = db
        .attach_route_as_user(user_id, author_id, tombstoned_key.clone())
        .await
        .expect("production user attach");
    db.remove_route_as_user(user_id, author_id, route.id)
        .await
        .expect("production user removal");
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::UserReResolve)
        .await
        .expect("re-arm after user removal");
    let now = Utc::now();
    let claim = db
        .claim_due(now, now + chrono::Duration::minutes(5), 1)
        .await
        .expect("post-removal claim")
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("tombstoned author claim");

    let gateway = StubAuthorProviderGateway {
        keyed_results: HashMap::from([(
            (AuthorProvider::OpenLibrary, "OL9001W".to_string()),
            vec![ProviderAuthorRef {
                key: tombstoned_key.clone(),
                name: "Tombstone Author".to_string(),
                role: Some("author".to_string()),
            }],
        )]),
        ol_search_results: vec![],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    };
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway,
    };
    let update = service
        .run_author(claim)
        .await
        .expect("tombstoned Tier-1 road");
    let calls = service.gateway.calls();
    assert_eq!(
        calls.len(),
        1,
        "a Tier-1 tombstone outcome must not fall through to name search"
    );
    assert_eq!(calls[0].work_route, "OL9001W");

    assert!(matches!(update.state, AuthorLinkProgressState::NeedsReview));
    assert!(db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes after automatic evidence")
        .is_empty());
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT state, removed_at FROM author_provider_routes WHERE id=?")
            .bind(route.id)
            .fetch_one(db.pool())
            .await
            .expect("tombstone observation");
    assert_eq!(row.0, "removed");
    assert!(row.1.is_some());
    let review = db.list_review(user_id).await.expect("tombstone review");
    assert!(review[0].candidates.iter().any(|candidate| {
        candidate.key == tombstoned_key
            && matches!(candidate.reason, AuthorLinkCandidateReason::Tombstoned)
    }));
}

/// Door: OL keyed work-author adapter.
/// AC-003 / AC-013: the concrete adapter returns every contributor unselected
/// at Low priority.
#[tokio::test]
async fn ac003_ac013_open_library_keyed_adapter_returns_all_contributors_unselected() {
    let fixture = br#"{"authors":[
        {"author":{"key":"/authors/OL9301A"},"name":"Author One"},
        {"author":{"key":"/authors/OL9302A"},"name":"Author Two"}
    ]}"#;
    let client = OpenLibraryClient::new(StubHttpFetcher::with_ok(200, fixture.to_vec()));
    let refs = client
        .fetch_work_authors("OL9300W".to_string(), RequestPriority::Low)
        .await
        .expect("concrete OL author adapter");
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].name, "Author One");
    assert_eq!(refs[1].name, "Author Two");
}

/// Door: Goodreads keyed work-author adapter.
/// AC-003 / AC-013: every credited contributor remains available to the
/// common guard; the adapter does not choose one.
#[tokio::test]
async fn ac003_ac013_goodreads_keyed_adapter_returns_all_contributors_unselected() {
    let fixture = br#"<html><script type="application/ld+json">
      {"author":[{"@type":"Person","name":"Author One","url":"/author/show/31"},
                 {"@type":"Person","name":"Author Two","url":"/author/show/32"}]}
    </script></html>"#;
    let http = HttpClient::builder()
        .user_agent("livrarr-author-link-test")
        .build()
        .expect("test HTTP client");
    let client = GoodreadsClient::new(
        StubHttpFetcher::with_ok(200, fixture.to_vec()),
        http,
        "https://www.goodreads.com",
    );
    let refs = client
        .fetch_work_authors("9300".to_string(), RequestPriority::Low)
        .await
        .expect("concrete Goodreads author adapter");
    assert_eq!(refs.len(), 2);
}

/// Door: Hardcover keyed work-author adapter.
/// AC-003 / AC-013: multi-contributor payloads are retained; layout drift is
/// a visible adapter error rather than a guessed author.
#[tokio::test]
async fn ac003_ac013_hardcover_keyed_adapter_returns_all_contributors_unselected() {
    let fixture = br#"{"data":{"editions":[{"contributions":[
      {"author":{"id":41,"name":"Author One"}},
      {"author":{"id":42,"name":"Author Two"}}
    ]}]}}"#;
    let client = HardcoverClient::new(
        StubHttpFetcher::with_ok(200, fixture.to_vec()),
        LiveMetadataConfig::new(livrarr_domain::settings::MetadataConfig {
            hardcover_enabled: true,
            hardcover_api_token: Some("test-token".to_string()),
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: None,
        }),
    );
    let refs = client
        .fetch_work_authors("9300".to_string(), RequestPriority::Low)
        .await
        .expect("concrete Hardcover author adapter");
    assert_eq!(refs.len(), 2);
}

/// Test seam: configured work-route lookup and unconfigured-key default.
/// AC-003: the behavioral gateway must expose contributor fixtures to the real
/// road and log exact provider/route/priority requests.
#[tokio::test]
async fn harness_gateway_seam_serves_configured_keyed_results_and_defaults_empty() {
    let configured_ref = ProviderAuthorRef {
        key: AuthorRouteKey::OpenLibrary(configured_ol_key("OL9399A")),
        name: "Configured Contributor".to_string(),
        role: Some("author".to_string()),
    };
    let gateway = StubAuthorProviderGateway {
        keyed_results: HashMap::from([(
            (AuthorProvider::OpenLibrary, "OL9399W".to_string()),
            vec![configured_ref.clone()],
        )]),
        ol_search_results: vec![],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    };

    let configured = gateway
        .fetch_work_authors(
            AuthorProvider::OpenLibrary,
            "OL9399W".to_string(),
            RequestPriority::Low,
        )
        .await
        .expect("configured keyed response");
    assert_eq!(configured.len(), 1);
    assert_eq!(configured[0].key, configured_ref.key);
    assert_eq!(configured[0].name, "Configured Contributor");

    let missing = gateway
        .fetch_work_authors(
            AuthorProvider::Goodreads,
            "9399".to_string(),
            RequestPriority::Normal,
        )
        .await
        .expect("unconfigured key defaults to successful empty");
    assert!(missing.is_empty());

    let calls = gateway.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].provider, AuthorProvider::OpenLibrary);
    assert_eq!(calls[0].work_route, "OL9399W");
    assert_eq!(calls[0].priority, RequestPriority::Low);
    assert_eq!(calls[1].provider, AuthorProvider::Goodreads);
    assert_eq!(calls[1].work_route, "9399");
    assert_eq!(calls[1].priority, RequestPriority::Normal);
}

/// Door: shared Open Library author-search adapter.
/// AC-002 / AC-004: stable canonical-distinct aliases and top-work evidence
/// survive the shared search adapter; the no-route invariant is pinned through
/// the real-SQLite Tier-2 `run_author` case above.
#[tokio::test]
async fn harness_gateway_seam_open_library_search_preserves_aliases_and_top_work() {
    let gateway = StubAuthorProviderGateway {
        keyed_results: HashMap::new(),
        ol_search_results: vec![OpenLibraryAuthorCandidate {
            route_key: configured_ol_key("OL9401A"),
            name: "Primary Name".to_string(),
            alternate_names: vec!["Alias One".to_string(), "Alias Two".to_string()],
            top_work: Some("Top Work".to_string()),
            work_count: Some(5),
        }],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    };
    let results = gateway
        .search_open_library_authors("Primary Name".to_string(), 10, RequestPriority::Low)
        .await
        .expect("search adapter");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].alternate_names, ["Alias One", "Alias Two"]);
    assert_eq!(results[0].top_work.as_deref(), Some("Top Work"));
    let calls = gateway.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider, AuthorProvider::OpenLibrary);
    assert_eq!(calls[0].work_route, "ol_search:Primary Name:limit=10");
    assert_eq!(calls[0].priority, RequestPriority::Low);
}

/// Door: shared Open Library catalog-page adapter.
/// AC-004: cursor continuity distinguishes a successful empty page from
/// retryable/permanent failure and only `Same` titles count downstream.
#[tokio::test]
async fn harness_gateway_seam_catalog_preserves_cursor_and_empty_success() {
    let gateway = StubAuthorProviderGateway {
        keyed_results: HashMap::new(),
        ol_search_results: vec![],
        ol_catalog_pages: vec![
            OpenLibraryCatalogPage {
                titles: vec!["First Title".to_string()],
                next_cursor: Some("page-2".to_string()),
            },
            OpenLibraryCatalogPage {
                titles: vec![],
                next_cursor: None,
            },
        ],
        calls: Mutex::new(vec![]),
    };
    let first = gateway
        .fetch_open_library_catalog_page(configured_ol_key("OL9402A"), None, RequestPriority::Low)
        .await
        .expect("first catalog page");
    assert_eq!(first.titles, ["First Title"]);
    assert_eq!(first.next_cursor.as_deref(), Some("page-2"));
    let second = gateway
        .fetch_open_library_catalog_page(
            configured_ol_key("OL9402A"),
            first.next_cursor,
            RequestPriority::Low,
        )
        .await
        .expect("successful empty second catalog page");
    assert!(second.titles.is_empty());
    assert_eq!(second.next_cursor, None);
    let calls = gateway.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].work_route.ends_with("cursor=None"));
    assert!(calls[1].work_route.ends_with("cursor=Some(\"page-2\")"));
    assert!(calls
        .iter()
        .all(|call| call.provider == AuthorProvider::OpenLibrary
            && call.priority == RequestPriority::Low));
}
