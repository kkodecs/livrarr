//! Behavioral pins for role-aware author-route gating (U8).
//!
//! A provider crediting a translator or a narrator on a book is not crediting
//! them as its author. These pins hold the whole path that decision travels:
//! the gateways certify what each provider actually said, the guard refuses to
//! judge a name that was never offered as an author, and the review surface
//! shows which book the question came from.
//!
//! Creation cases use the real service doors over an in-memory `SqliteDb`.
//! Adapter cases use the concrete OL/GR/HC clients behind the repository's
//! stub HTTP seam, and the migration case runs the real SQL onto a real
//! partially-migrated database.

use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use chrono::{Duration, Utc};
use livrarr_behavioral::stubs::{
    create_test_user, StubAuthorProviderGateway, StubEnrichmentWorkflow, StubHttpFetcher,
};
use livrarr_db::pool::{backfill_author_identity, backfill_normalized_identity};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{AuthorLinkClaim, AuthorLinkDb, GuardedRouteWrite, WorkDb};
use livrarr_domain::identity::{CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate};
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::WorkService;
use livrarr_domain::{
    guard_author_route, AuthorEvidenceFingerprint, AuthorLinkCandidateReason, AuthorLinkCursor,
    AuthorLinkProgressState, AuthorLinkProgressUpdate, AuthorLinkTrigger, AuthorProvider,
    AuthorRouteEvidenceSource, AuthorRouteGuardResult, AuthorRouteKey, OpenLibraryAuthorCandidate,
    OpenLibraryAuthorKey, OpenLibraryCatalogPage, ProviderAuthorRef, RequestPriority,
    RouteWriteOutcome,
};
use livrarr_external_data::live_config::LiveMetadataConfig;
use livrarr_external_data::{
    AuthorProviderGatewayImpl, GoodreadsClient, HardcoverClient, OpenLibraryClient,
};
use livrarr_http::HttpClient;
use livrarr_metadata::author_linking::AuthorLinkingServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Column, Row};

static ALL_MIGRATIONS: Migrator = sqlx::migrate!("../livrarr-db/migrations");

type RealWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

// ---------------------------------------------------------------------------
// Fixtures — every author, work and route below is built by a production writer
// ---------------------------------------------------------------------------

fn data_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "livrarr-author-link-role-gate-{label}-{}",
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

fn add_box_candidate(title: &str, author: &str, work_ol_key: &str) -> WorkCandidate {
    seed_add_box(
        SeedInput {
            title: title.to_string(),
            author_name: author.to_string(),
            language: SeedLanguage::resolve(Some("en"), "en"),
            author_ol_key: None,
            year: Some(2026),
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
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
        },
        None,
        false,
    )
}

fn ol_key(raw: &str) -> OpenLibraryAuthorKey {
    match AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw)
        .expect("fixture key must use the production parser")
    {
        AuthorRouteKey::OpenLibrary(key) => key,
        other => panic!("provider-selected parser returned the wrong variant: {other:?}"),
    }
}

fn ol_route(raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::OpenLibrary(ol_key(raw))
}

fn provider_ref(raw_key: &str, name: &str, role: Option<&str>) -> ProviderAuthorRef {
    ProviderAuthorRef {
        key: ol_route(raw_key),
        name: name.to_string(),
        role: role.map(str::to_string),
    }
}

fn gateway_with(refs: Vec<ProviderAuthorRef>) -> StubAuthorProviderGateway {
    StubAuthorProviderGateway {
        keyed_results: HashMap::from([(
            (AuthorProvider::OpenLibrary, "OL9001W".to_string()),
            refs,
        )]),
        ol_search_results: vec![],
        ol_catalog_pages: vec![],
        calls: Mutex::new(vec![]),
    }
}

/// One settled work, its converged author, and a live claim on the author's
/// linking task — all through the production add door and the production claim
/// writer.
async fn settled_author_claim(
    db: &SqliteDb,
    user_id: i64,
    label: &str,
    title: &str,
    author_name: &str,
) -> (i64, i64, AuthorLinkClaim) {
    let result = work_service(db.clone(), label)
        .add(user_id, add_box_candidate(title, author_name, "OL9001W"))
        .await
        .expect("production settled-work writer");
    let author_id = result.author_id.expect("converged author id");
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("production enqueue writer");
    let now = Utc::now();
    let claim = db
        .claim_due(now, now + Duration::minutes(5), 10)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("settled author claim");
    (author_id, result.work.id, claim)
}

async fn candidate_count(db: &SqliteDb, author_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM author_link_candidates WHERE author_id = ?")
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("candidate census")
}

async fn route_count(db: &SqliteDb, author_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM author_provider_routes WHERE author_id = ?")
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("route census")
}

fn metadata_config(
    hardcover_enabled: bool,
    token: Option<&str>,
) -> livrarr_domain::settings::MetadataConfig {
    livrarr_domain::settings::MetadataConfig {
        hardcover_enabled,
        hardcover_api_token: token.map(str::to_string),
        llm_enabled: false,
        llm_provider: None,
        llm_endpoint: None,
        llm_api_key: None,
        llm_model: None,
        audnexus_url: "https://api.audnex.us".to_string(),
        languages: vec!["en".to_string()],
        google_books_api_key: None,
    }
}

/// The production gateway over a canned HTTP seam: the real OL/GR/HC adapters,
/// so a road test exercises the certification the adapters actually perform.
fn real_gateway(fetcher: StubHttpFetcher) -> AuthorProviderGatewayImpl<StubHttpFetcher> {
    let http = HttpClient::builder()
        .user_agent("livrarr-author-link-role-gate-test")
        .build()
        .expect("test HTTP client");
    AuthorProviderGatewayImpl::new(
        OpenLibraryClient::new(fetcher.clone()),
        GoodreadsClient::new(fetcher.clone(), http, "https://www.goodreads.com"),
        HardcoverClient::new(
            fetcher,
            LiveMetadataConfig::new(metadata_config(false, None)),
        ),
    )
}

fn hardcover_client(fetcher: StubHttpFetcher) -> HardcoverClient<StubHttpFetcher> {
    HardcoverClient::new(
        fetcher,
        LiveMetadataConfig::new(metadata_config(true, Some("test-token"))),
    )
}

// ---------------------------------------------------------------------------
// 1 — mixed credits
// ---------------------------------------------------------------------------

/// Door: Recurring author-link sweep -> Tier-1 keyed contributor walk.
/// D8-1 / D8-3: a translator and a narrator credited on the same book are not
/// author claims, so they produce no route, no candidate and no question. An
/// authorial credit that agrees attaches; an authorial credit that disagrees
/// parks exactly one candidate, carrying the book it was seen on.
#[tokio::test]
async fn mixed_credits_link_the_author_park_the_disagreeing_author_and_drop_the_rest() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "mixed",
        "Mixed Credit Work",
        "Mixed Credit Author",
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: gateway_with(vec![
            provider_ref("OL9601A", "Mixed Credit Author", Some("author")),
            provider_ref("OL9602A", "Translating Person", Some("Translated by")),
            provider_ref("OL9603A", "Narrating Person", Some("narrator")),
            provider_ref("OL9604A", "Someone Else Entirely", Some("author")),
        ]),
    };
    service.run_author(claim).await.expect("mixed-credit road");

    let routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes");
    assert_eq!(
        routes.len(),
        1,
        "only the agreeing author credit may attach"
    );
    assert_eq!(routes[0].key, ol_route("OL9601A"));
    assert_eq!(
        route_count(&db, author_id).await,
        1,
        "a non-authorial credit must not leave a route row in any state"
    );

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1, "the disagreeing author is one question");
    let candidates = &review[0].candidates;
    assert_eq!(
        candidates.len(),
        1,
        "translator and narrator credits are not questions, got {:?}",
        candidates
            .iter()
            .map(|c| (c.key.value(), c.candidate_name.clone()))
            .collect::<Vec<_>>()
    );
    let parked = &candidates[0];
    assert_eq!(parked.key, ol_route("OL9604A"));
    assert!(matches!(
        parked.reason,
        AuthorLinkCandidateReason::NameGuardFailed
    ));
    assert_eq!(
        parked.evidence_work_id,
        Some(work_id),
        "a parked Tier-1 question must say which book raised it"
    );
    assert_eq!(
        parked.evidence_work_title.as_deref(),
        Some("Mixed Credit Work")
    );
    assert_eq!(
        candidate_count(&db, author_id).await,
        1,
        "a filtered credit must not be persisted in any status"
    );

    // No name the author never claimed enters the variant ledger.
    let variant_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM author_name_variants WHERE author_id = ?")
            .bind(author_id)
            .fetch_all(db.pool())
            .await
            .expect("variant census");
    assert!(
        !variant_names
            .iter()
            .any(|name| name == "Translating Person" || name == "Narrating Person"),
        "non-authorial credits must not become author name variants, got {variant_names:?}"
    );
}

// ---------------------------------------------------------------------------
// 2 — an all-non-authorial answer is not an answer
// ---------------------------------------------------------------------------

/// Door: Recurring author-link sweep -> Tier-2 OL name search.
/// D8-3: Tier 2 exists for authors whose keys said nothing about their author.
/// A book that credited only a translator said nothing about its author, so the
/// author stays eligible for the name search instead of being parked as
/// answered.
#[tokio::test]
async fn an_all_non_authorial_answer_leaves_tier_two_eligible() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "all-filtered",
        "All Filtered Work",
        "All Filtered Author",
    )
    .await;

    let mut gateway = gateway_with(vec![
        provider_ref("OL9611A", "Translating Person", Some("Translated by")),
        provider_ref("OL9612A", "Narrating Person", Some("narrator")),
    ]);
    gateway.ol_search_results = vec![OpenLibraryAuthorCandidate {
        route_key: ol_key("OL9613A"),
        name: "All Filtered Author".to_string(),
        alternate_names: vec![],
        top_work: Some("All Filtered Work".to_string()),
        work_count: Some(3),
    }];
    gateway.ol_catalog_pages = vec![OpenLibraryCatalogPage {
        titles: vec!["All Filtered Work".to_string()],
        next_cursor: None,
    }];

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway,
    };
    let update = service
        .run_author(claim)
        .await
        .expect("all-non-authorial road");

    assert_eq!(
        update.tier,
        Some(2),
        "a keyed read that credited nobody as an author must not suppress Tier 2"
    );
    assert!(matches!(update.state, AuthorLinkProgressState::NeedsReview));
    let calls = service.gateway.calls();
    assert_eq!(
        calls.len(),
        3,
        "Tier 2 must issue its name search and catalog read, got {calls:?}"
    );
    assert!(calls[1]
        .work_route
        .starts_with("ol_search:All Filtered Author:"));

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].candidates.len(), 1, "one Tier-2 candidate");
    let parked = &review[0].candidates[0];
    assert!(matches!(
        parked.reason,
        AuthorLinkCandidateReason::Tier2NameSearch
    ));
    assert_eq!(
        parked.evidence_work_id, None,
        "a name-search candidate is not evidence from one book"
    );
    assert_eq!(parked.evidence_work_title, None);
    assert_eq!(route_count(&db, author_id).await, 0);
}

// ---------------------------------------------------------------------------
// 3 — the Open Library type vocabulary, end to end through the real adapter
// ---------------------------------------------------------------------------

/// Door: Recurring author-link sweep -> the concrete Open Library adapter.
/// D8-1: Open Library spells an ordinary author credit `/type/author_role`.
/// That entry must reach the authorial path exactly as before this change (the
/// regression U8-F07 names), while a different `/type/key` on the same list is
/// a non-authorial credit and must not link.
#[tokio::test]
async fn open_library_author_role_links_and_another_type_key_does_not() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "ol-type-key",
        "Type Key Work",
        "Type Key Author",
    )
    .await;

    // One Open Library work record: the author credit Open Library actually
    // sends, plus a same-named entry under a different role type.
    let record = br#"{"authors":[
        {"type":{"key":"/type/author_role"},
         "author":{"key":"/authors/OL9701A"},
         "name":"Type Key Author"},
        {"type":{"key":"/type/translator_role"},
         "author":{"key":"/authors/OL9702A"},
         "name":"Type Key Author"}
    ]}"#;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(200, record.to_vec())),
    };
    service
        .run_author(claim)
        .await
        .expect("real Open Library adapter road");

    let routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes");
    assert_eq!(
        routes.len(),
        1,
        "author_role links; another role type does not, got {:?}",
        routes
            .iter()
            .map(|route| route.key.value())
            .collect::<Vec<_>>()
    );
    assert_eq!(routes[0].key, ol_route("OL9701A"));
    assert_eq!(
        candidate_count(&db, author_id).await,
        0,
        "a non-authorial Open Library entry is filtered, never parked"
    );
}

// ---------------------------------------------------------------------------
// 4 — one person, two credits
// ---------------------------------------------------------------------------

/// Door: the concrete Hardcover keyed adapter.
/// D8-2: Hardcover credits contributors per edition, so one person can be the
/// narrator of one edition and the author of another. Aggregating by route
/// must keep the author credit whichever edition carried it — a role-blind
/// first-seen dedup would throw it away half the time.
#[tokio::test]
async fn a_person_credited_as_author_on_any_edition_aggregates_as_the_author() {
    let narrator_first = br#"{"data":{"editions":[
      {"contributions":[{"contribution":"Narrated by","author":{"id":77,"name":"Dual Credit Person"}}]},
      {"contributions":[{"contribution":null,"author":{"id":77,"name":"Dual Credit Person"}}]}
    ]}}"#;
    let refs = hardcover_client(StubHttpFetcher::with_ok(200, narrator_first.to_vec()))
        .fetch_work_authors("9400".to_string(), RequestPriority::Low)
        .await
        .expect("concrete Hardcover adapter, narrator credit first");
    assert_eq!(refs.len(), 1, "one person is one route");
    assert_eq!(refs[0].name, "Dual Credit Person");
    assert_eq!(
        refs[0].role.as_deref(),
        Some("author"),
        "any authorial credit makes the aggregated route authorial"
    );

    let author_first = br#"{"data":{"editions":[
      {"contributions":[{"contribution":null,"author":{"id":77,"name":"Dual Credit Person"}}]},
      {"contributions":[{"contribution":"Narrated by","author":{"id":77,"name":"Dual Credit Person"}}]}
    ]}}"#;
    let refs = hardcover_client(StubHttpFetcher::with_ok(200, author_first.to_vec()))
        .fetch_work_authors("9401".to_string(), RequestPriority::Low)
        .await
        .expect("concrete Hardcover adapter, author credit first");
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].role.as_deref(),
        Some("author"),
        "the order the editions came back in must not change the answer"
    );

    // Two different people keep their own credits and their own order.
    let two_people = br#"{"data":{"editions":[
      {"contributions":[
        {"contribution":"Narrated by","author":{"id":78,"name":"Only A Narrator"}},
        {"contribution":null,"author":{"id":79,"name":"The Author"}}
      ]}
    ]}}"#;
    let refs = hardcover_client(StubHttpFetcher::with_ok(200, two_people.to_vec()))
        .fetch_work_authors("9402".to_string(), RequestPriority::Low)
        .await
        .expect("concrete Hardcover adapter, two people");
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].name, "Only A Narrator");
    assert_eq!(refs[0].role.as_deref(), Some("Narrated by"));
    assert_eq!(refs[1].name, "The Author");
    assert_eq!(refs[1].role.as_deref(), Some("author"));
}

// ---------------------------------------------------------------------------
// 5 — migration 080 repairs the live harm, and touches nothing else
// ---------------------------------------------------------------------------

async fn migration_079_db() -> SqliteDb {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("SQLite options")
        .pragma("foreign_keys", "ON")
        .pragma("busy_timeout", "5000");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("migration fixture pool");
    let through_079 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 79)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    through_079
        .run(&pool)
        .await
        .expect("apply real migrations through 079");
    backfill_normalized_identity(&pool)
        .await
        .expect("run production work-identity startup repair");
    backfill_author_identity(&pool)
        .await
        .expect("run production author-identity startup repair");
    SqliteDb::new(pool)
}

async fn apply_migration_080(db: &SqliteDb) {
    let only_080 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version == 80)
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    };
    assert_eq!(
        only_080.iter().count(),
        1,
        "migration 080 must exist and be the only one applied here"
    );
    only_080
        .run(db.pool())
        .await
        .expect("upgrade real migration-079 fixture through 080");
}

/// Every column of one progress row, as text, for a byte-identical comparison.
async fn progress_snapshot(db: &SqliteDb, author_id: i64) -> Vec<(String, Option<String>)> {
    let row = sqlx::query(
        "SELECT state, tier, cursor, evaluated_fingerprint, evidence_generation, \
                display_name_generation, display_name_dirty, attempt_count, next_attempt_at, \
                claim_token, lease_until, last_error, would_have_linked_at_090, trigger, \
                updated_at \
           FROM author_link_progress WHERE author_id = ?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("progress snapshot");
    (0..row.len())
        .map(|index| {
            (
                row.column(index).name().to_string(),
                row.try_get::<Option<String>, _>(index).unwrap_or_else(|_| {
                    row.try_get::<Option<i64>, _>(index)
                        .expect("progress column is text or integer")
                        .map(|value| value.to_string())
                }),
            )
        })
        .collect()
}

/// Every column of one route row, as text.
async fn route_snapshot(db: &SqliteDb, route_id: i64) -> Vec<(String, Option<String>)> {
    let row = sqlx::query(
        "SELECT user_id, author_id, provider, route_value, state, provenance, evidence_work_id, \
                created_at, verified_at, removed_at, removed_by_user_id \
           FROM author_provider_routes WHERE id = ?",
    )
    .bind(route_id)
    .fetch_one(db.pool())
    .await
    .expect("route snapshot");
    (0..row.len())
        .map(|index| {
            (
                row.column(index).name().to_string(),
                row.try_get::<Option<String>, _>(index).unwrap_or_else(|_| {
                    row.try_get::<Option<i64>, _>(index)
                        .expect("route column is text or integer")
                        .map(|value| value.to_string())
                }),
            )
        })
        .collect()
}

fn fingerprint(hash: &str) -> AuthorEvidenceFingerprint {
    AuthorEvidenceFingerprint {
        settled_work_count: 1,
        settled_provider_key_count: 1,
        content_hash: hash.to_string(),
    }
}

fn parked_update(
    generation: i64,
    next_attempt_at: chrono::DateTime<Utc>,
) -> AuthorLinkProgressUpdate {
    AuthorLinkProgressUpdate {
        state: AuthorLinkProgressState::NeedsReview,
        tier: Some(1),
        cursor: None,
        evaluated_fingerprint: fingerprint("pre-080-evaluated-fingerprint"),
        evidence_generation: generation,
        next_attempt_at,
        last_error: None,
        display_name_generation: 0,
        display_name_dirty: false,
        would_have_linked_at_090: false,
    }
}

/// Door: migration 080 -> the recurring sweep.
/// D8-5: the shipped junk questions are retired by the migration's targeted
/// requeue plus the road's own generation semantics — no candidate is deleted
/// and no route is touched. The unaffected author is the control: a
/// full-library requeue would move it, and it must not.
#[tokio::test]
async fn migration_080_requeues_only_affected_authors_and_the_rewalk_retires_the_junk() {
    let db = migration_079_db().await;
    let user_id = create_test_user(&db).await;

    // --- affected author: a pending junk question and an inherited route ---
    let harmed = work_service(db.clone(), "harmed")
        .add(
            user_id,
            add_box_candidate("Harmed Work", "Harmed Author", "OL9001W"),
        )
        .await
        .expect("production settled-work writer");
    let harmed_author = harmed.author_id.expect("converged author id");
    let harmed_work = harmed.work.id;

    // The route the parked repair must not touch: written by the production
    // guarded writer, so its provenance really is `tier1_inherited`.
    let AuthorRouteGuardResult::Agreed(agreed) = guard_author_route(
        &["Harmed Author".to_string()],
        provider_ref("OL9801A", "Harmed Author", Some("author")),
        Some(harmed_work),
        AuthorRouteEvidenceSource::Tier1SettledWork,
    ) else {
        panic!("fixture names must agree");
    };
    let written = db
        .apply_guarded_route(GuardedRouteWrite {
            claim_token: None,
            author_id: harmed_author,
            evidence: agreed,
        })
        .await
        .expect("production guarded route writer");
    let RouteWriteOutcome::Attached(route) = written else {
        panic!("the fixture route must attach, got {written:?}");
    };
    let inherited_route = route.id;

    let now = Utc::now();
    let harmed_claim = db
        .claim_due(now, now + Duration::minutes(5), 10)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == harmed_author)
        .expect("harmed author claim");

    // Constructed-state justification: this row is by definition a *pre-080*
    // candidate. After 080 no production writer can produce one — the insert
    // path writes the new evidence column — so the shipped shape has to be
    // fixture SQL for the migration to have anything to repair.
    sqlx::query(
        "INSERT INTO author_link_candidates \
             (user_id, author_id, provider, route_value, candidate_name, reason, name_verdict, \
              primary_name_verdict, top_work_preview, catalog_evidence_state, \
              corroborated_title_count, settled_work_count, previously_removed, status, \
              evidence_generation, observed_at) \
         VALUES (?, ?, 'open_library', 'OL9802A', 'Narrating Person', 'name_guard_failed', \
                 'disagree', 'disagree', NULL, 'pending', 0, 1, 0, 'pending', 1, \
                 '2026-07-29T10:00:00.000Z')",
    )
    .bind(user_id)
    .bind(harmed_author)
    .execute(db.pool())
    .await
    .expect("seed the pre-080 junk candidate");

    // A settled pass: evaluated fingerprint stored, next attempt far away.
    db.advance_progress(
        harmed_claim.clone(),
        parked_update(1, now + Duration::hours(12)),
    )
    .await
    .expect("production progress writer");

    // A worker holding the row when the operator upgrades: the requeue has to
    // void that claim, or the stale worker would still own the author.
    let stale_claim = db
        .claim_due(
            now + Duration::hours(13),
            now + Duration::hours(13) + Duration::minutes(5),
            10,
        )
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == harmed_author)
        .expect("harmed author re-claim");

    // --- unaffected author: a settled, not-yet-due, question-free row ---
    let quiet = work_service(db.clone(), "quiet")
        .add(
            user_id,
            add_box_candidate("Quiet Work", "Quiet Author", "OL9002W"),
        )
        .await
        .expect("production settled-work writer");
    let quiet_author = quiet.author_id.expect("converged author id");
    // This author was enqueued after `now` was taken, so its own due time is
    // the one to claim against.
    let quiet_now = Utc::now();
    let quiet_claim = db
        .claim_due(quiet_now, quiet_now + Duration::minutes(5), 10)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == quiet_author)
        .expect("quiet author claim");
    db.advance_progress(
        quiet_claim,
        AuthorLinkProgressUpdate {
            state: AuthorLinkProgressState::Linked,
            tier: Some(1),
            cursor: Some(AuthorLinkCursor::Tier2Search),
            evaluated_fingerprint: fingerprint("quiet-evaluated-fingerprint"),
            evidence_generation: 1,
            next_attempt_at: now + Duration::hours(48),
            last_error: Some("a diagnostic the migration must not erase".to_string()),
            display_name_generation: 0,
            display_name_dirty: false,
            would_have_linked_at_090: false,
        },
    )
    .await
    .expect("production progress writer");

    let quiet_before = progress_snapshot(&db, quiet_author).await;
    let route_before = route_snapshot(&db, inherited_route).await;

    apply_migration_080(&db).await;

    // The control author is untouched in every column.
    assert_eq!(
        progress_snapshot(&db, quiet_author).await,
        quiet_before,
        "the requeue must be targeted, not full-library"
    );

    // The harmed author is due again, with the claim voided and the
    // fingerprint cleared so the next pass is a full re-walk.
    let harmed_after = progress_snapshot(&db, harmed_author).await;
    let field = |name: &str| -> Option<String> {
        harmed_after
            .iter()
            .find(|(column, _)| column == name)
            .and_then(|(_, value)| value.clone())
    };
    assert_eq!(field("state").as_deref(), Some("queued"));
    assert_eq!(field("evaluated_fingerprint"), None);
    assert_eq!(field("claim_token"), None);
    assert_eq!(field("lease_until"), None);

    // The stale worker's claim is gone: it cannot write the old generation.
    assert!(
        db.begin_evidence_generation(stale_claim, 9).await.is_err(),
        "the requeue must void a live claim"
    );

    // --- the re-walk: every credit non-authorial, nothing new parked ---
    let rewalk_now = Utc::now();
    let claim = db
        .claim_due(rewalk_now, rewalk_now + Duration::minutes(5), 10)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == harmed_author)
        .expect("the requeued author must be immediately due");

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: gateway_with(vec![
            provider_ref("OL9802A", "Narrating Person", Some("narrator")),
            provider_ref("OL9803A", "Translating Person", Some("Translated by")),
        ]),
    };
    service.run_author(claim).await.expect("post-080 re-walk");

    let generation: i64 = sqlx::query_scalar(
        "SELECT evidence_generation FROM author_link_progress WHERE author_id = ?",
    )
    .bind(harmed_author)
    .fetch_one(db.pool())
    .await
    .expect("generation observation");
    assert_eq!(
        generation, 2,
        "a cleared fingerprint opens a new generation"
    );

    let status: String = sqlx::query_scalar(
        "SELECT status FROM author_link_candidates WHERE author_id = ? AND route_value = 'OL9802A'",
    )
    .bind(harmed_author)
    .fetch_one(db.pool())
    .await
    .expect("junk candidate observation");
    assert_eq!(
        status, "superseded",
        "the junk question is retired, not deleted"
    );

    assert_eq!(
        candidate_count(&db, harmed_author).await,
        1,
        "the re-walk must add no replacement question"
    );
    assert!(
        db.list_review(user_id).await.expect("review").is_empty(),
        "no author is left asking the user anything"
    );

    // The parked route repair did not leak back in.
    assert_eq!(
        route_snapshot(&db, inherited_route).await,
        route_before,
        "U8 adds no retirement, removal or reactivation behavior"
    );
}

// ---------------------------------------------------------------------------
// 6 — the review card's evidence
// ---------------------------------------------------------------------------

/// Door: `list_review` -> the Review page's author card.
/// D8-4: a Tier-1 question came from one book, and the card says which. When
/// that book is gone the join has nothing to show and the field is absent —
/// never a stale title and never a broken read.
#[tokio::test]
async fn list_review_carries_the_evidence_book_title_and_survives_its_deletion() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "evidence-title",
        "The Evidence Book",
        "Evidence Author",
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: gateway_with(vec![provider_ref(
            "OL9901A",
            "Someone Else Entirely",
            Some("author"),
        )]),
    };
    service.run_author(claim).await.expect("evidence road");

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1);
    let parked = &review[0].candidates[0];
    assert_eq!(parked.evidence_work_id, Some(work_id));
    assert_eq!(
        parked.evidence_work_title.as_deref(),
        Some("The Evidence Book"),
        "the card names the book the credit was seen on"
    );

    db.delete_work(user_id, work_id)
        .await
        .expect("production work deletion");

    let review = db
        .list_review(user_id)
        .await
        .expect("review after the evidence book is gone");
    assert_eq!(review.len(), 1, "the question outlives its evidence book");
    let parked = &review[0].candidates[0];
    assert_eq!(
        parked.evidence_work_id, None,
        "the deleted book releases the reference"
    );
    assert_eq!(
        parked.evidence_work_title, None,
        "no title is better than a stale one"
    );
    assert_eq!(parked.author_id, author_id);
}
