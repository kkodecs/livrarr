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
use livrarr_domain::identity_layer::{
    title_parts_from_provider, IdentityProvider, RouteKind, RouteOwner, RouteProvenance,
    SettlementCommit, WorkContributor, WorkIdentityRepository, WorkRoute, WorkRouteState,
};
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
    ProviderAuthorRef, ProviderCredit, RequestPriority,
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
    let expected_generation: i64 =
        sqlx::query_scalar("SELECT identity_generation FROM works WHERE id=?1 AND user_id=?2")
            .bind(result.work.id)
            .bind(user_id)
            .fetch_one(db.pool())
            .await
            .expect("read coherent pre-settlement generation");
    WorkIdentityRepository::commit_settlement(
        db,
        SettlementCommit {
            user_id,
            existing_work_id: Some(result.work.id),
            add_source: None,
            identity_title: title_parts_from_provider(title.to_string(), None)
                .expect("fixture title"),
            text_distinction: None,
            contributors: vec![WorkContributor {
                user_id,
                work_id: result.work.id,
                author_id,
                ordinal: 0,
                roles: vec![],
            }],
            routes: vec![WorkRoute {
                id: 0,
                user_id,
                owner: RouteOwner::Work(result.work.id),
                resolved_work_id: result.work.id,
                provider: IdentityProvider::OpenLibrary,
                kind: RouteKind::OpenLibraryWork,
                provider_scoped_id: "OL9001W".to_string(),
                state: WorkRouteState::Active,
                provenance: RouteProvenance::UserChoice,
                user_confirmed: true,
                observed_at: Utc::now(),
            }],
            absorbed_work_ids: vec![],
            expected_generation,
            review_cards: vec![],
        },
    )
    .await
    .expect("production F2 settled-work writer");
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
                    credit: ProviderCredit::AssertedAuthor,
                },
                ProviderAuthorRef {
                    key: AuthorRouteKey::OpenLibrary(ol_key("OL9502A")),
                    name: "Unrelated Contributor".to_string(),
                    credit: ProviderCredit::AssertedAuthor,
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
                    credit: ProviderCredit::AssertedAuthor,
                },
                ProviderAuthorRef {
                    key: AuthorRouteKey::OpenLibrary(ol_key("OL9512A")),
                    name: "Both Agree Author".to_string(),
                    credit: ProviderCredit::AssertedAuthor,
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
                credit: ProviderCredit::AssertedAuthor,
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
      {"contribution":null,"author":{"id":41,"name":"Author One"}},
      {"contribution":null,"author":{"id":42,"name":"Author Two"}}
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
        credit: ProviderCredit::AssertedAuthor,
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

// ===========================================================================
// U9R-F02 / F03 / F04 — durable state across passes
//
// Every pin below runs the same author through more than one claimed pass,
// which is the seam the single-pass suite never crossed: what an earlier pass
// wrote is durable, and `prepare_key_attempts` deliberately never hands a
// terminal or not-yet-due attempt back, so a later pass's in-memory tally is
// empty by design and cannot be the whole truth about the author.
// ===========================================================================

use livrarr_domain::AuthorProviderError;

/// A gateway whose keyed answer may be a provider failure, so a pass can leave
/// the durable retry a real provider outage leaves. Identical to
/// `StubAuthorProviderGateway` in every other respect.
struct ScriptedAuthorProviderGateway {
    keyed_results:
        HashMap<(AuthorProvider, String), Result<Vec<ProviderAuthorRef>, AuthorProviderError>>,
    ol_search_results: Vec<OpenLibraryAuthorCandidate>,
    ol_catalog_pages: Vec<OpenLibraryCatalogPage>,
    calls: Mutex<Vec<livrarr_db::AuthorProviderCall>>,
}

impl ScriptedAuthorProviderGateway {
    fn new() -> Self {
        Self {
            keyed_results: HashMap::new(),
            ol_search_results: vec![],
            ol_catalog_pages: vec![],
            calls: Mutex::new(vec![]),
        }
    }

    fn with_keyed(
        mut self,
        provider: AuthorProvider,
        work_route: &str,
        result: Result<Vec<ProviderAuthorRef>, AuthorProviderError>,
    ) -> Self {
        self.keyed_results
            .insert((provider, work_route.to_string()), result);
        self
    }

    fn with_name_search(
        mut self,
        candidates: Vec<OpenLibraryAuthorCandidate>,
        pages: Vec<OpenLibraryCatalogPage>,
    ) -> Self {
        self.ol_search_results = candidates;
        self.ol_catalog_pages = pages;
        self
    }

    fn calls(&self) -> Vec<livrarr_db::AuthorProviderCall> {
        self.calls
            .lock()
            .expect("scripted call log mutex poisoned")
            .clone()
    }

    fn record_call(&self, provider: AuthorProvider, work_route: String, priority: RequestPriority) {
        self.calls
            .lock()
            .expect("scripted call log mutex poisoned")
            .push(livrarr_db::AuthorProviderCall {
                provider,
                work_route,
                priority,
            });
    }
}

impl AuthorProviderGateway for ScriptedAuthorProviderGateway {
    async fn fetch_work_authors(
        &self,
        provider: AuthorProvider,
        work_route: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, AuthorProviderError> {
        self.record_call(provider, work_route.clone(), priority);
        self.keyed_results
            .get(&(provider, work_route))
            .cloned()
            .unwrap_or_else(|| Ok(vec![]))
    }

    async fn search_open_library_authors(
        &self,
        query: String,
        limit: u32,
        priority: RequestPriority,
    ) -> Result<Vec<OpenLibraryAuthorCandidate>, AuthorProviderError> {
        self.record_call(
            AuthorProvider::OpenLibrary,
            format!("ol_search:{query}:limit={limit}"),
            priority,
        );
        Ok(self.ol_search_results.clone())
    }

    async fn fetch_open_library_catalog_page(
        &self,
        author_route: OpenLibraryAuthorKey,
        cursor: Option<String>,
        priority: RequestPriority,
    ) -> Result<OpenLibraryCatalogPage, AuthorProviderError> {
        self.record_call(
            AuthorProvider::OpenLibrary,
            format!("ol_catalog:{author_route:?}:cursor={cursor:?}"),
            priority,
        );
        let page = match cursor.as_deref() {
            None => self.ol_catalog_pages.first(),
            Some(requested) => self
                .ol_catalog_pages
                .windows(2)
                .find(|pages| pages[0].next_cursor.as_deref() == Some(requested))
                .map(|pages| &pages[1]),
        };
        page.cloned().ok_or(AuthorProviderError::NotConfigured)
    }
}

fn scripted_service(
    db: &SqliteDb,
    gateway: ScriptedAuthorProviderGateway,
) -> AuthorLinkingServiceImpl<SqliteDb, ScriptedAuthorProviderGateway> {
    AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway,
    }
}

/// One contributor a provider credits as this book's author.
fn asserted_author(route: &str, name: &str) -> ProviderAuthorRef {
    ProviderAuthorRef {
        key: AuthorRouteKey::OpenLibrary(ol_key(route)),
        name: name.to_string(),
        credit: ProviderCredit::AssertedAuthor,
    }
}

/// Record one observed spelling of the author's name through the production
/// writer. This is what makes an author immediately due without touching the
/// settled evidence its fingerprint is taken over.
async fn observe_author_name(db: &SqliteDb, user_id: i64, author_id: i64, name: &str) {
    use livrarr_db::AuthorNameVariantDb;
    db.record_author_observed_names(
        user_id,
        author_id,
        &[livrarr_domain::ProviderAuthorNameObservation {
            source: livrarr_domain::AuthorNameSource::OpenLibrary,
            name: name.to_string(),
        }],
    )
    .await
    .expect("production author-name observation writer");
}

async fn progress_row(db: &SqliteDb, author_id: i64) -> (String, Option<i64>, String, i64) {
    sqlx::query_as(
        "SELECT state, tier, next_attempt_at, evidence_generation \
           FROM author_link_progress WHERE author_id = ?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("durable progress row")
}

async fn key_attempt_row(
    db: &SqliteDb,
    author_id: i64,
) -> (i64, i64, String, String, Option<String>) {
    sqlx::query_as(
        "SELECT id, work_id, work_route, state, next_attempt_at \
           FROM author_link_key_attempts WHERE author_id = ?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("durable key attempt row")
}

async fn candidate_census(db: &SqliteDb, author_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM author_link_candidates WHERE author_id = ?")
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("candidate census")
}

/// Door: Recurring author-link sweep -> a linked author's scheduled recheck.
/// U9R-F02: every key attempt of a linked author is terminal, so its scheduled
/// recheck runs no key and its tally is empty by design. Deriving the final
/// state from that tally alone demotes `Linked` to `ParkedNoEvidence` and pulls
/// the next look from the linked week to the parked day — which `sweep_progress`
/// then reports as a parked author, every linked author in the library, forever.
#[tokio::test]
async fn a_linked_authors_unchanged_recheck_stays_linked_and_calls_no_provider() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, claim) = settled_author_claim(
        &db,
        user_id,
        "linked-recheck",
        "Linked Recheck Work",
        "Linked Recheck Author",
    )
    .await;

    let linking = scripted_service(
        &db,
        ScriptedAuthorProviderGateway::new().with_keyed(
            AuthorProvider::OpenLibrary,
            "OL9001W",
            Ok(vec![asserted_author("OL9601A", "Linked Recheck Author")]),
        ),
    );
    let linked = linking.run_author(claim).await.expect("linking pass");
    assert!(
        matches!(linked.state, AuthorLinkProgressState::Linked),
        "the first pass must link the author: {:?}",
        linked.state
    );

    // The author comes back at its own linked recheck, with nothing about the
    // evidence changed.
    let recheck_claim =
        claim_author_at(&db, author_id, Utc::now() + chrono::Duration::hours(169)).await;
    let horizon = Utc::now();
    let quiet = scripted_service(&db, ScriptedAuthorProviderGateway::new());
    let update = quiet.run_author(recheck_claim).await.expect("recheck pass");

    assert!(
        quiet.gateway.calls().is_empty(),
        "an unchanged recheck of a linked author must ask no provider anything: {:?}",
        quiet.gateway.calls()
    );
    assert!(
        matches!(update.state, AuthorLinkProgressState::Linked),
        "the durable route still says linked: {:?}",
        update.state
    );
    assert!(
        update.next_attempt_at >= horizon + chrono::Duration::hours(167),
        "a linked author is looked at again on the linked interval, not the parked one"
    );
    assert_eq!(
        progress_row(&db, author_id).await.0,
        "linked",
        "the visible sweep state must still be linked"
    );
}

/// The author's next claim taken at a stated instant, so a test can step past a
/// lease or a recheck window it cannot wait for.
async fn claim_author_at(
    db: &SqliteDb,
    author_id: i64,
    at: chrono::DateTime<Utc>,
) -> livrarr_db::AuthorLinkClaim {
    db.claim_due(at, at + chrono::Duration::minutes(5), 10)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("the author must be claimable")
}

/// Door: Recurring author-link sweep -> display-name convergence while a key
/// retry is pending.
/// U9R-F02: a name observation makes the author due immediately, well before a
/// scheduled key retry. That pass runs no key — the retry is not due — so a
/// tally-only derivation overwrites the retry's state and its deadline with a
/// parked state a day out, delaying the provider's recovery by almost 24 hours.
#[tokio::test]
async fn a_scheduled_key_retry_survives_a_dirty_name_only_pass() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, claim) = settled_author_claim(
        &db,
        user_id,
        "retry-survives",
        "Retry Survives Work",
        "Retry Survives Author",
    )
    .await;

    let failing = scripted_service(
        &db,
        ScriptedAuthorProviderGateway::new().with_keyed(
            AuthorProvider::OpenLibrary,
            "OL9001W",
            Err(AuthorProviderError::Retryable {
                error: "OpenLibrary HTTP 503".to_string(),
                retry_not_before: None,
            }),
        ),
    );
    let failed = failing.run_author(claim).await.expect("failing pass");
    assert!(
        matches!(failed.state, AuthorLinkProgressState::RetryableFailure),
        "the first pass must schedule a key retry: {:?}",
        failed.state
    );
    let (_, _, _, attempt_state, retry_deadline) = key_attempt_row(&db, author_id).await;
    assert_eq!(attempt_state, "retryable");
    let retry_deadline = retry_deadline.expect("a retryable attempt carries its deadline");

    // A name observation makes the author due now — long before the retry is.
    observe_author_name(&db, user_id, author_id, "R. S. Author").await;
    let dirty_claim = claim_author_at(&db, author_id, Utc::now()).await;
    let quiet = scripted_service(&db, ScriptedAuthorProviderGateway::new());
    let update = quiet
        .run_author(dirty_claim)
        .await
        .expect("dirty-name-only pass");

    assert!(
        quiet.gateway.calls().is_empty(),
        "the retry is not due, so this pass calls no provider"
    );
    assert!(
        matches!(update.state, AuthorLinkProgressState::RetryableFailure),
        "a local name pass must not retire a scheduled provider retry: {:?}",
        update.state
    );
    let (state, _, next_attempt_at, _) = progress_row(&db, author_id).await;
    assert_eq!(state, "retryable_failure");
    assert_eq!(
        next_attempt_at, retry_deadline,
        "the author stays due at the retry's own deadline, not a fresh parked day"
    );
}

/// Door: Recurring author-link sweep -> a review-holding author's recheck.
/// U9R-F02: a pending question is durable, so an unchanged recheck that writes
/// no new candidate must not report the author as holding no evidence — the
/// review page still has a card the user has not answered.
#[tokio::test]
async fn a_pending_review_card_survives_an_unchanged_recheck() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, claim) = settled_author_claim(
        &db,
        user_id,
        "review-survives",
        "Review Survives Work",
        "Review Survives Author",
    )
    .await;

    let parking = scripted_service(
        &db,
        ScriptedAuthorProviderGateway::new().with_keyed(
            AuthorProvider::OpenLibrary,
            "OL9001W",
            Ok(vec![asserted_author("OL9602A", "Someone Else Entirely")]),
        ),
    );
    let parked = parking.run_author(claim).await.expect("parking pass");
    assert!(
        matches!(parked.state, AuthorLinkProgressState::NeedsReview),
        "the first pass must park a question: {:?}",
        parked.state
    );

    let recheck_claim =
        claim_author_at(&db, author_id, Utc::now() + chrono::Duration::hours(25)).await;
    let quiet = scripted_service(&db, ScriptedAuthorProviderGateway::new());
    let update = quiet.run_author(recheck_claim).await.expect("recheck pass");

    assert!(
        matches!(update.state, AuthorLinkProgressState::NeedsReview),
        "an unanswered question still needs review: {:?}",
        update.state
    );
    assert_eq!(progress_row(&db, author_id).await.0, "needs_review");
    assert_eq!(
        candidate_census(&db, author_id).await,
        1,
        "the recheck must not duplicate the card either"
    );
}

/// Door: Recurring author-link sweep -> Tier-2 entry after an interrupted pass.
/// U9R-F03: each key completion commits on its own, so a process that stops
/// between the last completion and the Tier-2 gate leaves a generation whose
/// keys are all terminal and whose durable authorial count is zero. Gating
/// Tier 2 on this pass's in-memory attempts means that owed name search is never
/// run again — the fingerprint is unchanged, so no later pass ever has an
/// attempt to show. The durable state, not the pass, has to answer.
#[tokio::test]
async fn tier_two_still_runs_after_a_pass_stops_between_the_last_key_and_the_gate() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, claim) = settled_author_claim(
        &db,
        user_id,
        "tier-two-resume",
        "Tier Two Resume Work",
        "Tier Two Resume Author",
    )
    .await;

    // Pass 1: the OpenLibrary key is retryable, which correctly defers Tier 2 —
    // a key that is about to answer properly beats a name search.
    let failing = scripted_service(
        &db,
        ScriptedAuthorProviderGateway::new().with_keyed(
            AuthorProvider::OpenLibrary,
            "OL9001W",
            Err(AuthorProviderError::Retryable {
                error: "OpenLibrary HTTP 503".to_string(),
                retry_not_before: None,
            }),
        ),
    );
    let deferred = failing.run_author(claim).await.expect("deferred pass");
    assert_eq!(deferred.tier, Some(1), "Tier 2 must not have run yet");

    // The retry then completes with no authorial credit and the process stops
    // before the Tier-2 gate. Both writes below are the production ones the road
    // itself calls, in the order it calls them; only the gate never runs.
    let generation = progress_row(&db, author_id).await.3;
    let (attempt_id, work_id, work_route, _, _) = key_attempt_row(&db, author_id).await;

    // The retry is five minutes out and `prepare_key_attempts` reads the wall
    // clock itself, so a test cannot hand it a later instant the way it hands
    // one to `claim_due`. Moving the stored deadline back is this test stepping
    // time forward; every write that follows is a production one.
    sqlx::query(
        "UPDATE author_link_key_attempts \
            SET next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 minute') \
          WHERE id = ?",
    )
    .bind(attempt_id)
    .execute(db.pool())
    .await
    .expect("step the retry window forward");

    let resume_claim =
        claim_author_at(&db, author_id, Utc::now() + chrono::Duration::hours(1)).await;
    let reclaimed = db
        .prepare_key_attempts(
            resume_claim.clone(),
            generation,
            vec![livrarr_domain::SettledWorkProviderKey {
                work_id,
                provider: AuthorProvider::OpenLibrary,
                work_route: work_route.clone(),
            }],
        )
        .await
        .expect("production key-attempt writer");
    assert_eq!(reclaimed.len(), 1, "the due retry must be reclaimable");
    db.complete_key_attempt(
        resume_claim,
        attempt_id,
        livrarr_domain::AuthorKeyAttemptOutcome::Succeeded,
        0,
    )
    .await
    .expect("production key-completion writer");

    // A later sweep picks the author up with the same evidence. Tier 2 is still
    // owed and nothing in this pass can prove it.
    let resumed_claim =
        claim_author_at(&db, author_id, Utc::now() + chrono::Duration::hours(2)).await;
    let searching = scripted_service(
        &db,
        ScriptedAuthorProviderGateway::new().with_name_search(
            vec![OpenLibraryAuthorCandidate {
                route_key: ol_key("OL9701A"),
                name: "Tier Two Resume Author".to_string(),
                alternate_names: vec![],
                top_work: Some("Tier Two Resume Work".to_string()),
                work_count: Some(4),
            }],
            vec![OpenLibraryCatalogPage {
                titles: vec!["Tier Two Resume Work".to_string()],
                next_cursor: None,
            }],
        ),
    );
    let update = searching
        .run_author(resumed_claim)
        .await
        .expect("resumed pass");

    assert_eq!(
        update.tier,
        Some(2),
        "the owed name search must run on resume"
    );
    let searches = searching
        .gateway
        .calls()
        .into_iter()
        .filter(|call| call.work_route.starts_with("ol_search:"))
        .count();
    assert_eq!(
        searches, 1,
        "Tier 2 runs once, not zero times and not twice"
    );
    assert_eq!(
        candidate_census(&db, author_id).await,
        1,
        "its outcome is persisted as a review card"
    );

    // And it is not owed twice: the next unchanged recheck asks nothing.
    let quiet_claim =
        claim_author_at(&db, author_id, Utc::now() + chrono::Duration::hours(25)).await;
    let quiet = scripted_service(&db, ScriptedAuthorProviderGateway::new());
    quiet.run_author(quiet_claim).await.expect("quiet recheck");
    assert!(
        quiet.gateway.calls().is_empty(),
        "a completed Tier 2 must not replay on an unchanged recheck: {:?}",
        quiet.gateway.calls()
    );
    assert_eq!(candidate_census(&db, author_id).await, 1);
}

/// Door: Recurring author-link sweep -> `run_due`'s operator metric.
/// U9R-F04: `unchanged_fingerprint` is meant to say how many claimed authors
/// re-read evidence they had already evaluated. Counted off a boolean that means
/// two different things on two branches, it reports the exact opposite for a
/// Tier-3 author: the first pass — which evaluated brand-new evidence — is
/// counted, and the genuinely unchanged second pass is not.
#[tokio::test]
async fn unchanged_fingerprint_counts_the_re_read_pass_and_not_the_first_one() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let author = author_service(db.clone())
        .add(
            user_id,
            AddAuthorRequest {
                name: "Metric Author".to_string(),
                sort_name: None,
                ol_key: None,
                monitored: false,
            },
        )
        .await
        .expect("real standalone author door");
    let author_id = author.author().id;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: empty_gateway(),
    };
    let first = service
        .run_due(10, CancellationToken::new())
        .await
        .expect("first tick");
    assert_eq!(first.evaluated, 1);
    assert_eq!(
        first.unchanged_fingerprint, 0,
        "the first evaluation of an author's evidence is not a re-read"
    );

    // The author comes back with its evidence untouched.
    observe_author_name(&db, user_id, author_id, "M. Author").await;
    let second = service
        .run_due(10, CancellationToken::new())
        .await
        .expect("second tick");
    assert_eq!(second.evaluated, 1);
    assert_eq!(
        second.unchanged_fingerprint, 1,
        "the pass that re-read the same evidence is the one the metric counts"
    );
}

/// Door: Recurring author-link sweep -> `run_due`'s operator metric.
/// U9R-F04: on the settled branch the same boolean tracked whether any key ran,
/// so an author whose evidence genuinely changed was still reported as unchanged
/// whenever its works carried no provider key to walk.
#[tokio::test]
async fn changed_evidence_without_a_usable_provider_key_is_not_counted_as_unchanged() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    work_service(db.clone(), "keyless")
        .add(
            user_id,
            seed_add_box(
                seed_input("Keyless Work", "Keyless Author", None),
                IdentityState::Confirmed {
                    anchors: CapturedIdentity {
                        ol_key: None,
                        gr_key: None,
                        hc_key: None,
                        isbn_13: None,
                        asin: None,
                        title: "Keyless Work".to_string(),
                        author_name: "Keyless Author".to_string(),
                        language: Some("en".to_string()),
                    },
                    method: IdentityMethod::UserSelected,
                    score: None,
                },
                None,
                false,
            ),
        )
        .await
        .expect("production settled-work writer");

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: empty_gateway(),
    };
    let tick = service
        .run_due(10, CancellationToken::new())
        .await
        .expect("first tick");
    assert_eq!(tick.evaluated, 1);
    assert_eq!(
        tick.unchanged_fingerprint, 0,
        "evidence this sweep had never evaluated is not a re-read, whether or not \
         it carried a key worth walking"
    );
}
