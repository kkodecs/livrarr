//! Behavioral pins for the unlabelled-credit gate and its script carve-out (U9).
//!
//! U8 stopped credits carrying a *named* non-author label. It did not stop
//! credits carrying no label at all — Hardcover's `contribution: null` and
//! OpenLibrary's `authors[]` membership — because both certified as the same
//! author claim a Goodreads "Author" edge did. These pins hold the distinction
//! the fix draws: an asserted credit is trusted and cards on any disagreement;
//! an unlabelled credit is a placement, so it auto-links only on name agreement
//! and survives a mismatch only when the offered name is written in a non-Latin
//! script.
//!
//! Creation cases use the real service doors over an in-memory `SqliteDb`.
//! Adapter cases use the concrete OL/GR/HC clients behind the repository's stub
//! HTTP seam, and the migration case runs the real SQL onto a real
//! partially-migrated database.
//!
//! This is a NEW binary rather than an extension of `test_author_link_role_gate`
//! because the `ProviderAuthorRef.role` → `credit` change breaks that file's
//! shipped fixtures, and editing a shipped test is not this seat's to do. See
//! `build/design-176/BLOCKED-U9-TESTS.md` §1.

use std::borrow::Cow;
use std::str::FromStr;

use chrono::{Duration, Utc};
use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::pool::{backfill_author_identity, backfill_normalized_identity};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{AuthorLinkClaim, AuthorLinkDb, AuthorNameVariantDb, GuardedRouteWrite};
use livrarr_domain::identity::{CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate};
use livrarr_domain::identity_matching::AuthorVerdict;
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{AuthorLinkService, FetchError, FetchResponse, WorkService};
use livrarr_domain::{
    guard_author_route, AuthorEvidenceFingerprint, AuthorLinkCandidateReason,
    AuthorLinkCandidateStatus, AuthorLinkProgressState, AuthorLinkProgressUpdate,
    AuthorLinkTrigger, AuthorNameSource, AuthorProvider, AuthorRouteEvidenceSource,
    AuthorRouteGuardResult, AuthorRouteKey, OpenLibraryAuthorKey, ProviderAuthorNameObservation,
    ProviderAuthorRef, ProviderCredit, RequestPriority, RouteWriteOutcome,
};
use livrarr_external_data::live_config::LiveMetadataConfig;
use livrarr_external_data::types::ProviderFetchError;
use livrarr_external_data::{
    AuthorProviderGatewayImpl, GoodreadsClient, HardcoverClient, OpenLibraryClient,
};
use livrarr_http::HttpClient;
use livrarr_metadata::author_linking::AuthorLinkingServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::author_link::readarr_author_route_evidence;
use livrarr_server::readarr_client::RdAuthor;
use livrarr_server::readarr_import_service::{LiveReadarrImportService, ReadarrImportService};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Column, Row};
use tracing_test::traced_test;

static ALL_MIGRATIONS: Migrator = sqlx::migrate!("../livrarr-db/migrations");

type RealWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

// ---------------------------------------------------------------------------
// Fixtures — every author, work, route and name variant is built by a
// production writer; only the pre-081 candidate rows are fixture SQL, and each
// says why.
// ---------------------------------------------------------------------------

fn data_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "livrarr-author-link-credit-gate-{label}-{}",
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

/// The provider keys one settled work carries into the Tier-1 walk.
#[derive(Default, Clone)]
struct WorkKeys {
    ol: Option<&'static str>,
    gr: Option<&'static str>,
    hc: Option<&'static str>,
}

impl WorkKeys {
    fn ol(key: &'static str) -> Self {
        Self {
            ol: Some(key),
            ..Self::default()
        }
    }

    fn hc(key: &'static str) -> Self {
        Self {
            hc: Some(key),
            ..Self::default()
        }
    }

    fn gr(key: &'static str) -> Self {
        Self {
            gr: Some(key),
            ..Self::default()
        }
    }

    fn gr_and_hc(gr: &'static str, hc: &'static str) -> Self {
        Self {
            ol: None,
            gr: Some(gr),
            hc: Some(hc),
        }
    }
}

fn add_box_candidate(title: &str, author: &str, keys: WorkKeys) -> WorkCandidate {
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
                ol_key: keys.ol.map(str::to_string),
                gr_key: keys.gr.map(str::to_string),
                hc_key: keys.hc.map(str::to_string),
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

fn gr_route(raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::parse(AuthorProvider::Goodreads, raw).expect("Goodreads author id")
}

fn hc_route(raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::parse(AuthorProvider::Hardcover, raw).expect("Hardcover author id")
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
        .user_agent("livrarr-author-link-credit-gate-test")
        .build()
        .expect("test HTTP client");
    AuthorProviderGatewayImpl::new(
        OpenLibraryClient::new(fetcher.clone()),
        GoodreadsClient::new(fetcher.clone(), http, "https://www.goodreads.com"),
        HardcoverClient::new(
            fetcher,
            LiveMetadataConfig::new(metadata_config(true, Some("test-token"))),
        ),
    )
}

/// One canned answer, for a fetcher that serves several reads in order.
fn response(status: u16, body: Vec<u8>) -> Result<FetchResponse, FetchError> {
    Ok(FetchResponse {
        status,
        headers: vec![],
        body,
    })
}

/// One Hardcover book's contributions, in the shape the live GraphQL answer
/// carries them.
fn hardcover_body(contributions: &str) -> Vec<u8> {
    format!(r#"{{"data":{{"editions":[{{"contributions":[{contributions}]}}]}}}}"#).into_bytes()
}

/// One OpenLibrary author-search answer, in the `docs[]` shape the live endpoint
/// returns.
///
/// Every candidate carries the same headline work, so `top_work_matches` — the
/// first key the road orders candidates by — cannot decide the order, and what
/// the ordering pin observes is the verdict.
fn open_library_author_search(candidates: &[(&str, &str)]) -> Vec<u8> {
    let docs = candidates
        .iter()
        .map(|(key, name)| {
            format!(
                r#"{{"key":"{key}","name":"{name}","top_work":"Tier Two Work","work_count":3}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"docs":[{docs}]}}"#).into_bytes()
}

/// One page of an OpenLibrary author's catalog. The `links` object carries no
/// `next`, which is how the live endpoint says this page is the last one.
fn open_library_catalog(titles: &[&str]) -> Vec<u8> {
    let entries = titles
        .iter()
        .map(|title| format!(r#"{{"title":"{title}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"entries":[{entries}],"links":{{}}}}"#).into_bytes()
}

/// One Goodreads book page in the current layout, with the contributor edges
/// spelled the way the live Apollo cache spells them.
fn goodreads_apollo_page(edges: &str, name: &str) -> Vec<u8> {
    format!(
        r#"<html><script id="__NEXT_DATA__" type="application/json">
        {{"props":{{"pageProps":{{"apolloState":{{
            "Book:kca://book/1":{{{edges}}},
            "Contributor:kca://author/1":{{"name":"{name}","legacyId":31}}
        }}}}}}}}</script></html>"#
    )
    .into_bytes()
}

/// The Goodreads JSON-LD fallback: a plain `author[]` container, no role field
/// anywhere in it.
fn goodreads_json_ld_page(name: &str) -> Vec<u8> {
    format!(
        r#"<html><script type="application/ld+json">
           {{"author":[{{"@type":"Person","name":"{name}","url":"/author/show/31"}}]}}
           </script></html>"#
    )
    .into_bytes()
}

/// One settled work and its converged author, through the production add door
/// and the production enqueue writer. No claim is taken, so a caller may record
/// further name variants before the road first sees the author.
async fn settled_author(
    db: &SqliteDb,
    user_id: i64,
    label: &str,
    title: &str,
    author_name: &str,
    keys: WorkKeys,
) -> (i64, i64) {
    let result = work_service(db.clone(), label)
        .add(user_id, add_box_candidate(title, author_name, keys))
        .await
        .expect("production settled-work writer");
    let author_id = result.author_id.expect("converged author id");
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("production enqueue writer");
    (author_id, result.work.id)
}

/// The same, plus a live claim on the author's linking task from the production
/// claim writer.
async fn settled_author_claim(
    db: &SqliteDb,
    user_id: i64,
    label: &str,
    title: &str,
    author_name: &str,
    keys: WorkKeys,
) -> (i64, i64, AuthorLinkClaim) {
    let (author_id, work_id) = settled_author(db, user_id, label, title, author_name, keys).await;
    let claim = claim_author(db, author_id, Utc::now()).await;
    (author_id, work_id, claim)
}

/// The author's next claim, taken at a stated wall-clock instant so a test can
/// step past a lease or a retry window it cannot wait for.
async fn claim_author(db: &SqliteDb, author_id: i64, at: chrono::DateTime<Utc>) -> AuthorLinkClaim {
    db.claim_due(at, at + Duration::minutes(5), 10)
        .await
        .expect("production claim writer")
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("the author must be claimable")
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

async fn variant_names(db: &SqliteDb, author_id: i64) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM author_name_variants WHERE author_id = ?")
        .bind(author_id)
        .fetch_all(db.pool())
        .await
        .expect("variant census")
}

/// Record one more spelling of the author's name through the production
/// observation writer, so the guard's snapshot is one the road really builds.
async fn observe_name(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    source: AuthorNameSource,
    name: &str,
) {
    db.record_author_observed_names(
        user_id,
        author_id,
        &[ProviderAuthorNameObservation {
            source,
            name: name.to_string(),
        }],
    )
    .await
    .expect("production name-variant writer");
}

// ===========================================================================
// 1 — an unlabelled credit that agrees still links (D9-1 guard table, row 4)
// ===========================================================================

/// Door: Recurring author-link sweep -> the concrete Hardcover adapter.
/// D9-1: Hardcover marks authorship by an empty field, so `contribution: null`
/// is an unlabelled placement. A placement whose name agrees is still the
/// author, and the route it earns under U8 must survive this unit unchanged.
#[tokio::test]
async fn an_unlabeled_credit_whose_name_agrees_still_links() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "unlabeled-agree",
        "Agreeing Work",
        "Agreeing Author",
        WorkKeys::hc("4200"),
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(r#"{"contribution":null,"author":{"id":77,"name":"Agreeing Author"}}"#),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("real Hardcover adapter road");

    let routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes");
    assert_eq!(routes.len(), 1, "an agreeing placement is still the author");
    assert_eq!(routes[0].key, hc_route("77"));
    assert_eq!(
        candidate_count(&db, author_id).await,
        0,
        "an agreeing credit asks nothing"
    );
}

// ===========================================================================
// 2 — an unlabelled Latin mismatch is dropped, and Tier 2 stays shut
// ===========================================================================

/// Door: Recurring author-link sweep -> the concrete Hardcover adapter.
/// D9-1 guard table row 5 + INV-U9-4 + INV-U9-3: a Hardcover translator entered
/// with a blank `contribution` is a placement, not a claim, so a name
/// disagreement in Latin script is not a question worth asking — nothing is
/// written anywhere. It is nevertheless an authorial observation, so Tier 2 must
/// not open and flood the review page with name-search candidates instead.
#[tokio::test]
async fn an_unlabeled_latin_mismatch_writes_nothing_and_keeps_tier_two_shut() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "unlabeled-latin",
        "Latin Drop Work",
        "Latin Drop Author",
        WorkKeys::hc("4201"),
    )
    .await;

    let fetcher = StubHttpFetcher::with_ok(
        200,
        hardcover_body(r#"{"contribution":null,"author":{"id":78,"name":"Jean-François Ménard"}}"#),
    );
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(fetcher.clone()),
    };
    let update = service
        .run_author(claim)
        .await
        .expect("real Hardcover adapter road");

    assert_eq!(
        route_count(&db, author_id).await,
        0,
        "a dropped credit writes no route in any state"
    );
    assert_eq!(
        candidate_count(&db, author_id).await,
        0,
        "a dropped credit is not a question, in any status"
    );
    assert!(
        !variant_names(&db, author_id)
            .await
            .iter()
            .any(|name| name == "Jean-François Ménard"),
        "a dropped credit is not a name this author is known by"
    );
    assert!(
        db.list_review(user_id).await.expect("review").is_empty(),
        "the user is asked nothing"
    );

    assert_ne!(
        update.tier,
        Some(2),
        "a dropped credit is still an authorial observation, so Tier 2 stays shut"
    );
    assert_eq!(
        fetcher.call_count(),
        1,
        "exactly the one keyed Hardcover read — no OpenLibrary name search"
    );
}

// ===========================================================================
// 3 — an unlabelled non-Latin mismatch is kept as one card
// ===========================================================================

/// Door: Recurring author-link sweep -> the concrete Hardcover adapter.
/// D9-1 guard table row 6 + D9-2: the carve-out. A name written in another
/// writing system may be this same author transliterated, and nothing in this
/// unit can tell that from a foreign-language translator — so the question
/// survives, carrying the book it was raised on.
#[tokio::test]
async fn an_unlabeled_non_latin_mismatch_keeps_exactly_one_card() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "unlabeled-non-latin",
        "Transliteration Work",
        "Walter Isaacson",
        WorkKeys::hc("4202"),
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(r#"{"contribution":null,"author":{"id":79,"name":"Уолтер Айзексон"}}"#),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("real Hardcover adapter road");

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1, "the transliteration is one question");
    let candidates = &review[0].candidates;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].key, hc_route("79"));
    assert_eq!(candidates[0].candidate_name, "Уолтер Айзексон");
    assert!(matches!(
        candidates[0].reason,
        AuthorLinkCandidateReason::NameGuardFailed
    ));
    assert_eq!(
        candidates[0].evidence_work_id,
        Some(work_id),
        "a kept card must say which book raised it"
    );
    assert_eq!(
        route_count(&db, author_id).await,
        0,
        "a kept card is a question, never a route"
    );
}

// ===========================================================================
// 4 — an asserted credit cards on any disagreement, script-blind
// ===========================================================================

/// Door: Recurring author-link sweep -> the concrete Goodreads adapter.
/// D9-1 guard table row 3 + INV-U9-5: Goodreads is the only provider that
/// asserts a role. An assertion is trusted, so a Latin-script name mismatch on
/// one is still a real question and is never dropped. No live row can reach this
/// branch (ST-U9-005), so this test is the only proof it works.
#[tokio::test]
async fn an_asserted_credit_with_a_latin_mismatch_still_cards() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "asserted-mismatch",
        "Asserted Work",
        "Asserted Author",
        WorkKeys::gr("9300"),
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            goodreads_apollo_page(
                r#""primaryContributorEdge":{"node":{"__ref":"Contributor:kca://author/1"},"role":"Author"}"#,
                "Someone Else Entirely",
            ),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("real Goodreads adapter road");

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1, "an asserted mismatch is a real question");
    let candidates = &review[0].candidates;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].key, gr_route("31"));
    assert_eq!(candidates[0].candidate_name, "Someone Else Entirely");
    assert_eq!(candidates[0].evidence_work_id, Some(work_id));
    assert_eq!(route_count(&db, author_id).await, 0);
}

// ===========================================================================
// 5 — the authorial observation survives the pass that made it
// ===========================================================================

/// Door: Recurring author-link sweep, twice, across a key retry boundary.
/// D9-3b / INV-U9-3 [U9-F03]: the Tier-2 gate cannot read a per-pass in-memory
/// tally. A terminal key attempt is never returned by `prepare_key_attempts`
/// again, so pass 2 rebuilds its tally from scratch and — with the surviving
/// sibling reporting only labelled credits — would open Tier 2 on a question
/// pass 1 already answered. The gate must read the generation's persisted
/// per-attempt counts instead.
#[tokio::test]
async fn a_dropped_credit_keeps_tier_two_shut_on_a_later_pass() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    // One work, two keys, so both attempts live in one evidence generation.
    // Neither is OpenLibrary: an outstanding OpenLibrary retry defers Tier 2 on
    // its own and would mask what this test is about.
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "durable-observation",
        "Two Key Work",
        "Two Key Author",
        WorkKeys::gr_and_hc("9301", "4203"),
    )
    .await;

    // Attempts run ordered by work, then provider name: goodreads, then
    // hardcover. Pass 1 answers Goodreads with a refusal it may retry, and
    // Hardcover with an unlabelled Latin mismatch that drops.
    //
    // All three answers are queued here, pass 2's included, because the stub
    // pops the head only while more than one response remains and then serves
    // that last one to every later caller. A response pushed between the two
    // passes would queue *behind* pass 1's sticky tail and pass 2 would be
    // handed the leftover Hardcover body instead.
    let fetcher = StubHttpFetcher::new();
    fetcher.push_response(response(503, b"upstream busy".to_vec()));
    fetcher.push_response(response(
        200,
        hardcover_body(r#"{"contribution":null,"author":{"id":80,"name":"Jean-François Ménard"}}"#),
    ));
    fetcher.push_response(response(
        200,
        goodreads_apollo_page(
            r#""primaryContributorEdge":{"node":{"__ref":"Contributor:kca://author/1"},"role":"Translator"}"#,
            "Translating Person",
        ),
    ));

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(fetcher.clone()),
    };
    let first = service
        .run_author(claim)
        .await
        .expect("pass 1 over two keys");
    assert_ne!(first.tier, Some(2), "pass 1 observed an authorial slot");
    assert_eq!(candidate_count(&db, author_id).await, 0);

    // What pass 1 durably recorded, per key. Only the Hardcover key observed an
    // authorial slot, and it is the key that will not run again — which is the
    // whole reason the count has to outlive the pass that made it.
    let generation = current_generation(&db, author_id).await;
    assert_eq!(
        key_attempt(&db, author_id, "goodreads", generation).await,
        ("retryable".to_string(), 1, 0),
        "a refused key is retryable and observed nothing"
    );
    assert_eq!(
        key_attempt(&db, author_id, "hardcover", generation).await,
        ("succeeded".to_string(), 1, 1),
        "the dropped credit is a durable authorial observation on the key that saw it"
    );

    // Constructed-state justification: the production path produces exactly this
    // row — a retryable Goodreads attempt whose next try is a backoff away. Only
    // the wall clock separates the two passes, and a test cannot wait it out.
    sqlx::query(
        "UPDATE author_link_key_attempts \
            SET next_attempt_at = '2000-01-01T00:00:00.000Z' \
          WHERE author_id = ? AND state = 'retryable'",
    )
    .bind(author_id)
    .execute(db.pool())
    .await
    .expect("bring the retryable attempt forward");

    // Pass 2: the Hardcover attempt is terminal and is not handed back, so only
    // Goodreads runs — and it credits nobody as an author. Its body is the one
    // still sitting at the head of the queue.
    let calls_before = fetcher.call_count();
    let hardcover_before = attempt_snapshot(&db, author_id, "hardcover").await;
    let later = claim_author(&db, author_id, Utc::now() + Duration::minutes(30)).await;
    let second = service
        .run_author(later)
        .await
        .expect("pass 2 over one key");

    assert_ne!(
        second.tier,
        Some(2),
        "what pass 1 observed must still close Tier 2 on pass 2"
    );
    assert_eq!(
        fetcher.call_count() - calls_before,
        1,
        "exactly the one reclaimed Goodreads read — no OpenLibrary name search"
    );
    assert_eq!(
        candidate_count(&db, author_id).await,
        0,
        "no Tier-2 candidate may be written"
    );

    // It was the reclaimed Goodreads key that ran, and only it: a pass that
    // replayed Hardcover instead would also have made exactly one call and
    // written no candidate, so the identity of the key is the assertion.
    assert_eq!(
        key_attempt(&db, author_id, "goodreads", generation).await,
        ("succeeded".to_string(), 2, 0),
        "the reclaimed key ran a second time and credited nobody as an author"
    );
    assert_eq!(
        attempt_snapshot(&db, author_id, "hardcover").await,
        hardcover_before,
        "a terminal key is never replayed inside its generation"
    );
    assert_eq!(
        generation_authorial_credits(&db, author_id, generation).await,
        1,
        "the generation still holds the one observation pass 1 made"
    );
}

// ===========================================================================
// 6 — the Goodreads JSON-LD fallback is a placement, not an assertion
// ===========================================================================

/// Door: Recurring author-link sweep -> the concrete Goodreads adapter, JSON-LD
/// fallback path.
/// D9-1 [U9-F02], PO-confirmed 2026-07-31: the JSON-LD fallback reads a plain
/// `author[]` container with no role field anywhere in it — structurally the
/// same shape as OpenLibrary's `authors[]`, which the same rule calls
/// unlabelled. Classifying it as asserted would make one Goodreads book mean two
/// different things depending on which parser answered.
#[tokio::test]
async fn goodreads_json_ld_is_an_unlabeled_placement() {
    // A Latin mismatch through the fallback is dropped, not carded. This is the
    // shipped-behaviour change the ruling costs, made executable.
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "json-ld-drop",
        "Fallback Work",
        "Fallback Author",
        WorkKeys::gr("9302"),
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            goodreads_json_ld_page("Cristina Macía Orio"),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("real Goodreads JSON-LD road");

    assert_eq!(
        candidate_count(&db, author_id).await,
        0,
        "an unlabelled Latin mismatch drops, whichever Goodreads parser answered"
    );
    assert_eq!(route_count(&db, author_id).await, 0);

    // The same container, agreeing: a placement that agrees is still the author.
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "json-ld-link",
        "Fallback Agree Work",
        "Fallback Agree Author",
        WorkKeys::gr("9303"),
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            goodreads_json_ld_page("Fallback Agree Author"),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("real Goodreads JSON-LD road");

    let routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes");
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].key, gr_route("31"));
    assert_eq!(candidate_count(&db, author_id).await, 0);
}

// ===========================================================================
// 7 — one author's own spellings stop manufacturing ambiguity
// ===========================================================================

/// Door: Recurring author-link sweep -> the road's name snapshot.
/// D9-4 / INV-U9-6: `author_verdict`'s both-sides-unambiguous rule is right for
/// lists of different people and wrong when one side is one person's own alias
/// list — two spellings that canonicalize identically make an exact match read
/// as ambiguous. Collapsing them at the snapshot builder fixes the comparison
/// without touching the verdict authority.
///
/// Without this the carve-out makes a *worse* bug than the one it fixes: a
/// byte-identical name is an absurd but visible Grey card today, and would
/// become a silent Latin drop.
#[tokio::test]
async fn canonically_identical_spellings_stop_making_an_exact_match_grey() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id) = settled_author(
        &db,
        user_id,
        "dedupe-agree",
        "Rowling Work",
        "J. K. Rowling",
        WorkKeys::hc("4204"),
    )
    .await;
    // A second spelling of the same person, from a different provider, through
    // the production observation writer. Both canonicalize to `j k rowling`.
    observe_name(
        &db,
        user_id,
        author_id,
        AuthorNameSource::Goodreads,
        "J.K. Rowling",
    )
    .await;

    let claim = claim_author(&db, author_id, Utc::now()).await;
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(r#"{"contribution":null,"author":{"id":81,"name":"J.K. Rowling"}}"#),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("deduped-snapshot road");

    let routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes");
    assert_eq!(
        routes.len(),
        1,
        "an author's own two spellings are one name, not ambiguous evidence"
    );
    assert_eq!(routes[0].key, hc_route("81"));
    assert_eq!(candidate_count(&db, author_id).await, 0);
}

/// Door: Recurring author-link sweep -> the road's name snapshot.
/// D9-4, negative control: only byte-equal canonical forms collapse, so an
/// author who really is known by two *distinct* names keeps two columns and an
/// offer compatible with both is still ambiguous. That is the protection the
/// dedup preserves — real multi-column ambiguity, not the duplicate-spelling
/// artifact the unit exists to remove. Carried on an *asserted* credit so the
/// verdict is readable on the card the disagreement parks.
#[tokio::test]
async fn a_compatible_but_distinct_name_still_reads_as_grey() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id) = settled_author(
        &db,
        user_id,
        "dedupe-control",
        "Rowling Control Work",
        "J. K. Rowling",
        WorkKeys::gr("9304"),
    )
    .await;
    observe_name(
        &db,
        user_id,
        author_id,
        AuthorNameSource::Goodreads,
        "J.K. Rowling",
    )
    .await;
    // A third spelling that is genuinely a different name: `j a rowling` is its
    // own canonical key, so the dedup leaves two distinct columns standing.
    observe_name(
        &db,
        user_id,
        author_id,
        AuthorNameSource::Hardcover,
        "J. A. Rowling",
    )
    .await;

    let claim = claim_author(&db, author_id, Utc::now()).await;
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            goodreads_apollo_page(
                r#""primaryContributorEdge":{"node":{"__ref":"Contributor:kca://author/1"},"role":"Author"}"#,
                "J. Rowling",
            ),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("deduped-snapshot road");

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(
        review.len(),
        1,
        "a name compatible with two distinct columns is still a question"
    );
    let parked = &review[0].candidates[0];
    assert_eq!(parked.candidate_name, "J. Rowling");
    assert_eq!(
        parked.name_verdict,
        AuthorVerdict::Grey,
        "multi-distinct-column ambiguity survives the canonical-form dedup"
    );
    assert_eq!(route_count(&db, author_id).await, 0);
}

// ===========================================================================
// 8a — the Readarr door builds its own snapshot, and it is deduped too
// ===========================================================================

/// Door: Readarr import -> the production import service's own snapshot builder,
/// guard call and route writer.
/// ST-U9-007 / INV-U9-6: two independent builders feed the one guard, and a fix
/// applied to only one of them is half a fix. The Readarr door dedups by exact
/// string equality, so an author holding two canonically identical spellings
/// rejects its own Readarr record.
///
/// The snapshot is read from `LiveReadarrImportService::author_associated_names`
/// — the builder D9-4 names — and the verdict is dispatched to the same two
/// write doors `resolve_readarr_author_route` dispatches to, so the outcome
/// asserted here is a route row the import really wrote, not a value a helper
/// returned.
#[tokio::test]
async fn the_readarr_door_agrees_over_canonically_identical_spellings() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id) = settled_author(
        &db,
        user_id,
        "readarr-dedupe",
        "Readarr Work",
        "J. K. Rowling",
        WorkKeys::ol("OL9001W"),
    )
    .await;
    observe_name(
        &db,
        user_id,
        author_id,
        AuthorNameSource::Goodreads,
        "J.K. Rowling",
    )
    .await;

    let rd_author: RdAuthor = serde_json::from_value(serde_json::json!({
        "id": 1,
        "authorName": "J.K. Rowling",
        "foreignAuthorId": "1077326",
    }))
    .expect("Readarr author record");

    let import_service = LiveReadarrImportService::new(db.clone());
    let names = import_service
        .author_associated_names(user_id, author_id)
        .await
        .expect("the production Readarr name snapshot");
    match readarr_author_route_evidence(&rd_author, &names)
        .expect("a named author with a canonical Goodreads id is judgeable")
    {
        AuthorRouteGuardResult::Agreed(evidence) => {
            import_service
                .submit_author_route_evidence(user_id, author_id, evidence)
                .await
                .expect("the production Readarr route writer");
        }
        AuthorRouteGuardResult::Rejected(rejected) => {
            import_service
                .record_author_route_rejection(user_id, author_id, rejected)
                .await
                .expect("the production Readarr rejection writer");
        }
        other => panic!(
            "this door certifies its own record as an assertion, so it can only \
             agree or reject, got {other:?}"
        ),
    }

    let routes = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes");
    assert_eq!(
        routes.len(),
        1,
        "the author's own two spellings must not make its own record ambiguous"
    );
    assert_eq!(routes[0].key, gr_route("1077326"));
    assert_eq!(
        candidate_count(&db, author_id).await,
        0,
        "an agreeing Readarr record raises no question"
    );
}

// ===========================================================================
// 8b — Tier-2 verdicts read the same deduped snapshot
// ===========================================================================

/// Door: Recurring author-link sweep -> Tier-2 OpenLibrary name search, over the
/// concrete Hardcover and OpenLibrary adapters.
/// D9-4b [U9-F06]: the same snapshot is passed to `run_name_search`, where a
/// candidate's primary verdict is computed against it and verdict strength
/// participates in review ordering. The dedup therefore changes Tier-2 verdicts
/// too — a second behaviour change, and it gets its own pin.
///
/// Two candidates, so the ordering claim is provable. Both corroborate the same
/// one settled work, so review order falls to verdict strength and, only on a
/// tie, the route key: the duplicate-alias candidate holds the *higher* key, so
/// a false Grey sorts it second and the Agree the dedup makes true sorts it
/// first. One candidate could not tell those apart.
///
/// The author holds the collapsing pair plus one genuinely distinct name, so the
/// two verdicts under test are different for different reasons: the candidate
/// offering the duplicate spelling matches exactly one surviving column (Agree,
/// and a false Grey before the dedup), while the candidate offering the shorter
/// name matches both surviving columns and is ambiguous on the merits.
#[tokio::test]
async fn a_tier_two_candidate_is_not_a_false_grey_over_duplicate_aliases() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id) = settled_author(
        &db,
        user_id,
        "tier2-dedupe",
        "Tier Two Work",
        "J. K. Rowling",
        WorkKeys::hc("4205"),
    )
    .await;
    observe_name(
        &db,
        user_id,
        author_id,
        AuthorNameSource::Goodreads,
        "J.K. Rowling",
    )
    .await;
    observe_name(
        &db,
        user_id,
        author_id,
        AuthorNameSource::Hardcover,
        "J. A. Rowling",
    )
    .await;

    // The three real reads this road makes, in the order it makes them: the
    // keyed Hardcover credit, the OpenLibrary name search, then one catalog page
    // per candidate. The keyed read credits only a translator, so nothing it
    // returns is an authorial observation and the author stays eligible for
    // Tier 2.
    let fetcher = StubHttpFetcher::new();
    fetcher.push_response(response(
        200,
        hardcover_body(
            r#"{"contribution":"Translated by","author":{"id":82,"name":"Translating Person"}}"#,
        ),
    ));
    fetcher.push_response(response(
        200,
        open_library_author_search(&[("OL9805A", "J.K. Rowling"), ("OL1234A", "J. Rowling")]),
    ));
    // Byte-identical pages: whichever order the road reads the two catalogs in,
    // both candidates corroborate the same single settled work, so the review
    // order under test is decided by the verdicts and nothing else.
    fetcher.push_response(response(200, open_library_catalog(&["Tier Two Work"])));
    fetcher.push_response(response(200, open_library_catalog(&["Tier Two Work"])));

    let claim = claim_author(&db, author_id, Utc::now()).await;
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(fetcher),
    };
    let update = service
        .run_author(claim)
        .await
        .expect("Tier-2 road over the real adapters");
    assert_eq!(
        update.tier,
        Some(2),
        "a labelled-only read leaves Tier 2 open"
    );

    let review = db.list_review(user_id).await.expect("review");
    assert_eq!(review.len(), 1);
    let candidates = &review[0].candidates;
    assert_eq!(candidates.len(), 2, "both search results are parked");

    assert_eq!(
        candidates[0].key,
        ol_route("OL9805A"),
        "the corrected verdict, not the route key, must decide which question is read first"
    );
    assert_eq!(
        candidates[0].primary_name_verdict,
        AuthorVerdict::Agree,
        "a duplicate-canonical alias list must not manufacture a false Grey on Tier 2"
    );
    assert_eq!(candidates[0].name_verdict, AuthorVerdict::Agree);

    assert_eq!(candidates[1].key, ol_route("OL1234A"));
    assert_eq!(
        candidates[1].primary_name_verdict,
        AuthorVerdict::Grey,
        "a compatible but distinct name stays genuinely ambiguous"
    );
    assert_eq!(candidates[1].name_verdict, AuthorVerdict::Grey);
}

// ===========================================================================
// 9 — OpenLibrary tells absence apart from unreadability
// ===========================================================================

/// Door: the concrete OpenLibrary keyed adapter.
/// D9-1a [U9-F01]: OpenLibrary is the one gateway with no role-drop branch, so a
/// mechanical port of its helper turns `{"key": null}`, `{"key": 42}`,
/// `{"key": ""}` and `{"key": "/type/"}` into author placements. Under U9 a
/// matching name on one of those would mint an automatic route and a Latin
/// mismatch would vanish silently. Every entry unreadable is the shape having
/// moved (INV-U9-2), never a book nobody wrote.
#[tokio::test]
async fn open_library_reports_drift_when_every_type_is_present_but_unreadable() {
    let record = br#"{"authors":[
        {"type":{"key":null},"author":{"key":"/authors/OL7101A"},"name":"Null Type"},
        {"type":{"key":42},"author":{"key":"/authors/OL7102A"},"name":"Numeric Type"},
        {"type":{"key":""},"author":{"key":"/authors/OL7103A"},"name":"Blank Type"},
        {"type":{"key":"/type/"},"author":{"key":"/authors/OL7104A"},"name":"Bare Prefix"}
    ]}"#;

    let outcome = OpenLibraryClient::new(StubHttpFetcher::with_ok(200, record.to_vec()))
        .fetch_work_authors("OL7100W".to_string(), RequestPriority::Low)
        .await;

    assert!(
        matches!(outcome, Err(ProviderFetchError::LayoutDrift(_))),
        "an all-unreadable role shape is drift, not an uncredited book, got {outcome:?}"
    );
}

/// Door: the concrete OpenLibrary keyed adapter.
/// D9-1a [U9-F01]: one unreadable entry beside one readable one is parse-drift
/// discipline — warn, drop the malformed entry, and answer with the valid ref
/// only. The warning is the load-bearing half: it is the signal that tells
/// partial layout damage apart from clean data, and a silent drop returns the
/// same refs while telling the operator nothing.
#[tokio::test]
#[traced_test]
async fn open_library_drops_one_unreadable_entry_and_keeps_the_valid_one() {
    let record = br#"{"authors":[
        {"type":{"key":null},"author":{"key":"/authors/OL7105A"},"name":"Unreadable Type"},
        {"type":{"key":"/type/author_role"},"author":{"key":"/authors/OL7106A"},"name":"Real Author"}
    ]}"#;

    let refs = OpenLibraryClient::new(StubHttpFetcher::with_ok(200, record.to_vec()))
        .fetch_work_authors("OL7100W".to_string(), RequestPriority::Low)
        .await
        .expect("a mixed response still answers");

    assert_eq!(refs.len(), 1, "only the readable credit survives");
    assert_eq!(refs[0].name, "Real Author");
    assert_eq!(refs[0].key, ol_route("OL7106A"));
    assert_eq!(refs[0].credit, ProviderCredit::UnlabeledAuthorSlot);

    // `logs_assert` is handed only the lines this test's own span carries, so a
    // sibling test reaching the same warn callsite in parallel cannot be counted
    // here — which is exactly what a thread-scoped capture could not guarantee.
    logs_assert(|lines: &[&str]| {
        let warnings_naming = |needle: &str| {
            lines
                .iter()
                .filter(|line| line.contains("WARN") && line.contains(needle))
                .count()
        };
        match (warnings_naming("OL7105A"), warnings_naming("OL7106A")) {
            (1, 0) => Ok(()),
            (dropped, kept) => Err(format!(
                "the dropped entry is warned about exactly once — the parser-drift \
                 signal — and a readable credit is not drift and must raise no \
                 warning; got {dropped} naming OL7105A and {kept} naming OL7106A"
            )),
        }
    });
}

/// Door: the concrete OpenLibrary keyed adapter.
/// D9-1a: the two shapes OpenLibrary really sends still reach the unlabelled
/// path. `author_role` is boilerplate structure and a missing type is container
/// membership; neither is an editorial claim about the person, and a different
/// role type is a labelled credit.
#[tokio::test]
async fn open_library_absent_and_author_role_types_are_both_placements() {
    let record = br#"{"authors":[
        {"type":{"key":"/type/author_role"},"author":{"key":"/authors/OL7107A"},"name":"Typed Author"},
        {"author":{"key":"/authors/OL7108A"},"name":"Untyped Author"},
        {"type":{"key":"/type/translator_role"},"author":{"key":"/authors/OL7109A"},"name":"Translator Person"}
    ]}"#;

    let refs = OpenLibraryClient::new(StubHttpFetcher::with_ok(200, record.to_vec()))
        .fetch_work_authors("OL7100W".to_string(), RequestPriority::Low)
        .await
        .expect("OpenLibrary contributor read");

    assert_eq!(refs.len(), 3);
    assert_eq!(refs[0].credit, ProviderCredit::UnlabeledAuthorSlot);
    assert_eq!(
        refs[1].credit,
        ProviderCredit::UnlabeledAuthorSlot,
        "the authors[] container is a placement, not a claim"
    );
    assert_eq!(
        refs[2].credit,
        ProviderCredit::Labeled("translator_role".to_string())
    );
}

// ===========================================================================
// 12 + 13 — migration 081 on a real migrated SQLite, with four controls
// ===========================================================================

async fn migration_080_db() -> SqliteDb {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("SQLite options")
        .pragma("foreign_keys", "ON")
        .pragma("busy_timeout", "5000");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("migration fixture pool");
    let through_080 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 80)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    through_080
        .run(&pool)
        .await
        .expect("apply real migrations through 080");
    backfill_normalized_identity(&pool)
        .await
        .expect("run production work-identity startup repair");
    backfill_author_identity(&pool)
        .await
        .expect("run production author-identity startup repair");
    SqliteDb::new(pool)
}

async fn apply_migration_081(db: &SqliteDb) {
    let only_081 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version == 81)
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    };
    assert_eq!(
        only_081.iter().count(),
        1,
        "migration 081 must exist and be the only one applied here"
    );
    only_081
        .run(db.pool())
        .await
        .expect("upgrade real migration-080 fixture through 081");
}

/// Every column of one progress row, as text, for a byte-identical comparison.
async fn progress_snapshot(db: &SqliteDb, author_id: i64) -> Vec<(String, Option<String>)> {
    row_snapshot(
        db,
        "SELECT state, tier, cursor, evaluated_fingerprint, evidence_generation, \
                display_name_generation, display_name_dirty, attempt_count, next_attempt_at, \
                claim_token, lease_until, last_error, would_have_linked_at_090, trigger, \
                updated_at \
           FROM author_link_progress WHERE author_id = ?",
        author_id,
    )
    .await
}

/// Every column of one route row, as text.
async fn route_snapshot(db: &SqliteDb, route_id: i64) -> Vec<(String, Option<String>)> {
    row_snapshot(
        db,
        "SELECT user_id, author_id, provider, route_value, state, provenance, evidence_work_id, \
                created_at, verified_at, removed_at, removed_by_user_id \
           FROM author_provider_routes WHERE id = ?",
        route_id,
    )
    .await
}

async fn row_snapshot(db: &SqliteDb, sql: &str, id: i64) -> Vec<(String, Option<String>)> {
    let row = sqlx::query(sql)
        .bind(id)
        .fetch_one(db.pool())
        .await
        .expect("row snapshot");
    columns_as_text(&row)
}

fn columns_as_text(row: &sqlx::sqlite::SqliteRow) -> Vec<(String, Option<String>)> {
    (0..row.len())
        .map(|index| {
            (
                row.column(index).name().to_string(),
                row.try_get::<Option<String>, _>(index).unwrap_or_else(|_| {
                    row.try_get::<Option<i64>, _>(index)
                        .expect("column is text or integer")
                        .map(|value| value.to_string())
                }),
            )
        })
        .collect()
}

/// What one key attempt durably recorded: the state it completed in, how many
/// times it has run, and how many authorial slots it observed.
async fn key_attempt(
    db: &SqliteDb,
    author_id: i64,
    provider: &str,
    generation: i64,
) -> (String, i64, i64) {
    sqlx::query_as(
        "SELECT state, attempt_count, authorial_credits_seen FROM author_link_key_attempts \
          WHERE author_id = ? AND provider = ? AND evidence_generation = ?",
    )
    .bind(author_id)
    .bind(provider)
    .bind(generation)
    .fetch_one(db.pool())
    .await
    .expect("key attempt observation")
}

/// Every mutable column of one provider's current key attempt, as text, so a
/// pass that touched it at all is visible.
async fn attempt_snapshot(
    db: &SqliteDb,
    author_id: i64,
    provider: &str,
) -> Vec<(String, Option<String>)> {
    let row = sqlx::query(
        "SELECT evidence_generation, state, claim_token, attempt_count, next_attempt_at, \
                last_error, diagnostic_code, authorial_credits_seen, updated_at \
           FROM author_link_key_attempts WHERE author_id = ? AND provider = ?",
    )
    .bind(author_id)
    .bind(provider)
    .fetch_one(db.pool())
    .await
    .expect("key attempt snapshot");
    columns_as_text(&row)
}

/// The whole generation's durable authorial-slot count — what the Tier-2 gate
/// must read instead of a per-pass tally.
async fn generation_authorial_credits(db: &SqliteDb, author_id: i64, generation: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(authorial_credits_seen), 0) FROM author_link_key_attempts \
          WHERE author_id = ? AND evidence_generation = ?",
    )
    .bind(author_id)
    .bind(generation)
    .fetch_one(db.pool())
    .await
    .expect("generation observation sum")
}

/// Seed one pre-081 candidate row.
///
/// Constructed-state justification: this row is by definition a *pre-081*
/// candidate, and after 081 no production writer can produce one — the insert
/// path is the current schema's. For the migration to have anything to select,
/// the shipped shape has to be fixture SQL.
#[allow(clippy::too_many_arguments)]
async fn seed_pre_081_candidate(
    db: &SqliteDb,
    user_id: i64,
    author_id: i64,
    route_value: &str,
    reason: &str,
    status: &str,
    generation: i64,
) {
    sqlx::query(
        "INSERT INTO author_link_candidates \
             (user_id, author_id, provider, route_value, candidate_name, reason, name_verdict, \
              primary_name_verdict, top_work_preview, catalog_evidence_state, \
              corroborated_title_count, settled_work_count, previously_removed, status, \
              evidence_generation, observed_at) \
         VALUES (?, ?, 'hardcover', ?, 'Jean-François Ménard', ?, 'disagree', 'disagree', NULL, \
                 'pending', 0, 1, 0, ?, ?, '2026-07-29T10:00:00.000Z')",
    )
    .bind(user_id)
    .bind(author_id)
    .bind(route_value)
    .bind(reason)
    .bind(status)
    .bind(generation)
    .execute(db.pool())
    .await
    .expect("seed the pre-081 candidate");
}

/// Door: migration 081 -> the recurring sweep.
/// D9-5 + INV-U9-8 [U9-F08]: the shipped unlabelled-credit questions are retired
/// by the migration's targeted requeue plus the road's own generation semantics.
/// No candidate is deleted and no route is touched — the byte-identical route
/// assertion is valid on this path precisely because nothing reaches
/// `apply_guarded_route`.
///
/// The four control authors are the U9-F08 correction: r1's single control could
/// catch only a full-library update. Superseded, dismissed and
/// legacy-contradiction rows must each fail the predicate on their own.
#[tokio::test]
async fn migration_081_requeues_only_affected_authors_and_the_rewalk_drops_the_junk() {
    let db = migration_080_db().await;
    let user_id = create_test_user(&db).await;

    // --- affected author: a pending name-guard question and an inherited route
    //
    // The pre-081 state is seeded, never walked. `complete_key_attempt` writes
    // the new observation column in the same statement that records an attempt's
    // transition (D9-3b), so the current road cannot run at a schema below 081 —
    // a state the composition root forecloses, since every migration completes
    // before the server serves. Each row below therefore comes from the
    // production writer that can still produce it at 080, and only from fixture
    // SQL where no current writer can.
    let (harmed_author, harmed_work) = settled_author(
        &db,
        user_id,
        "harmed",
        "Harmed Work",
        "Harmed Author",
        WorkKeys::hc("4206"),
    )
    .await;

    // The route the migration must not touch, written by the production guarded
    // writer over evidence the production guard minted — so its provenance
    // really is inherited and the Agree capability was really earned.
    let AuthorRouteGuardResult::Agreed(agreed) = guard_author_route(
        &["Harmed Author".to_string()],
        ProviderAuthorRef {
            key: hc_route("83"),
            name: "Harmed Author".to_string(),
            credit: ProviderCredit::UnlabeledAuthorSlot,
        },
        Some(harmed_work),
        AuthorRouteEvidenceSource::Tier1SettledWork,
    ) else {
        panic!("an unlabelled placement whose name agrees must mint the write capability");
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

    // Constructed-state justification: a *completed* 080-era key attempt. The
    // shipped U8-era writer (1f76ccfb) produced exactly this row shape — no
    // observation column, because 081 is what adds it — and the current writer
    // refuses to produce it by design. It is the row 081's ALTER has to
    // backfill, and the `attempt_counts` assertion below reads only it.
    sqlx::query(
        "INSERT INTO author_link_key_attempts \
             (user_id, author_id, evidence_generation, work_id, provider, work_route, \
              state, attempt_count, updated_at) \
         VALUES (?, ?, 1, ?, 'hardcover', '4206', 'succeeded', 1, '2026-07-29T10:00:00.000Z')",
    )
    .bind(user_id)
    .bind(harmed_author)
    .bind(harmed_work)
    .execute(db.pool())
    .await
    .expect("seed the completed 080-era key attempt");

    // A settled pass, through the production progress writer: generation 1
    // recorded, its evaluated fingerprint stored and the next recheck a week
    // out. This is the state 081 has to clear for the re-walk to happen.
    let harmed_claim = claim_author(&db, harmed_author, Utc::now()).await;
    db.advance_progress(
        harmed_claim,
        AuthorLinkProgressUpdate {
            state: AuthorLinkProgressState::Linked,
            tier: Some(1),
            cursor: None,
            evaluated_fingerprint: AuthorEvidenceFingerprint {
                settled_work_count: 1,
                settled_provider_key_count: 1,
                content_hash: "pre-081-evaluated-fingerprint".to_string(),
            },
            evidence_generation: 1,
            next_attempt_at: Utc::now() + Duration::hours(168),
            last_error: None,
            display_name_generation: 0,
            display_name_dirty: false,
            would_have_linked_at_090: false,
        },
    )
    .await
    .expect("production progress writer");

    seed_pre_081_candidate(
        &db,
        user_id,
        harmed_author,
        "9802",
        "name_guard_failed",
        "pending",
        current_generation(&db, harmed_author).await,
    )
    .await;

    // A worker holding the row when the operator upgrades: the requeue has to
    // void that claim, or the stale worker would still own the author. The
    // author linked, so its own recheck is a week out — this is the first
    // instant it can be claimed again.
    let stale_claim = claim_author(&db, harmed_author, Utc::now() + Duration::hours(169)).await;

    // --- four controls, every one of which must be byte-identical after 081 ---
    let mut controls = Vec::new();

    // (a) no pending candidate at all, and not yet due.
    let (quiet, _, _) = settled_author_claim(
        &db,
        user_id,
        "quiet",
        "Quiet Work",
        "Quiet Author",
        WorkKeys::ol("OL9002W"),
    )
    .await;
    controls.push(("no candidate at all", quiet));

    // (b) only a superseded name-guard row from a stale generation.
    let (superseded, _, _) = settled_author_claim(
        &db,
        user_id,
        "superseded",
        "Superseded Work",
        "Superseded Author",
        WorkKeys::ol("OL9003W"),
    )
    .await;
    seed_pre_081_candidate(
        &db,
        user_id,
        superseded,
        "9803",
        "name_guard_failed",
        "superseded",
        current_generation(&db, superseded).await - 1,
    )
    .await;
    controls.push(("only a superseded row", superseded));

    // (c) only a dismissed name-guard row.
    let (dismissed, _, _) = settled_author_claim(
        &db,
        user_id,
        "dismissed",
        "Dismissed Work",
        "Dismissed Author",
        WorkKeys::ol("OL9004W"),
    )
    .await;
    seed_pre_081_candidate(
        &db,
        user_id,
        dismissed,
        "9804",
        "name_guard_failed",
        "dismissed",
        current_generation(&db, dismissed).await,
    )
    .await;
    controls.push(("only a dismissed row", dismissed));

    // (d) only a current legacy-contradiction row — a question U9 cannot change.
    let (contradiction, _, _) = settled_author_claim(
        &db,
        user_id,
        "contradiction",
        "Contradiction Work",
        "Contradiction Author",
        WorkKeys::ol("OL9005W"),
    )
    .await;
    seed_pre_081_candidate(
        &db,
        user_id,
        contradiction,
        "9805",
        "legacy_contradiction",
        "pending",
        current_generation(&db, contradiction).await,
    )
    .await;
    controls.push(("only a legacy-contradiction row", contradiction));

    let mut before = Vec::new();
    for (label, author_id) in &controls {
        before.push((*label, *author_id, progress_snapshot(&db, *author_id).await));
    }
    let route_before = route_snapshot(&db, inherited_route).await;

    apply_migration_081(&db).await;

    // The new durable observation column exists and defaults truthfully: a
    // pre-081 attempt recorded no observation, which is the bounded backfill
    // residual D9-3b names.
    let attempt_counts: Vec<i64> = sqlx::query_scalar(
        "SELECT authorial_credits_seen FROM author_link_key_attempts WHERE author_id = ?",
    )
    .bind(harmed_author)
    .fetch_all(db.pool())
    .await
    .expect("the new attempt column must exist after 081");
    assert_eq!(
        attempt_counts,
        vec![0],
        "the harmed author's one pre-081 attempt reads as having observed nothing"
    );

    for (label, author_id, snapshot) in &before {
        assert_eq!(
            &progress_snapshot(&db, *author_id).await,
            snapshot,
            "control author with {label} must be byte-identical after 081"
        );
    }

    // The harmed author is due again, claim voided and fingerprint cleared, so
    // the next pass is a full re-walk.
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
    assert!(
        db.begin_evidence_generation(stale_claim, 9).await.is_err(),
        "the requeue must void a live claim"
    );

    // --- the re-walk: an all-Latin unlabelled mismatch, nothing new parked ---
    let generation_before = current_generation(&db, harmed_author).await;
    let claim = claim_author(&db, harmed_author, Utc::now() + Duration::hours(31)).await;
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(
                r#"{"contribution":null,"author":{"id":84,"name":"Jean-François Ménard"}}"#,
            ),
        )),
    };
    service.run_author(claim).await.expect("post-081 re-walk");

    let generation_after = current_generation(&db, harmed_author).await;
    assert!(
        generation_after > generation_before,
        "a cleared fingerprint opens a new generation"
    );

    // The re-walk's own attempt carries the observation the dropped credit made,
    // written by the same statement that recorded the attempt's transition. An
    // implementation that ignores the new parameter satisfies column presence
    // and fails here.
    assert_eq!(
        key_attempt(&db, harmed_author, "hardcover", generation_after).await,
        ("succeeded".to_string(), 1, 1),
        "the re-walk's dropped credit is durably recorded on its own attempt"
    );

    let status: String = sqlx::query_scalar(
        "SELECT status FROM author_link_candidates WHERE author_id = ? AND route_value = '9802'",
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
        db.list_review(user_id)
            .await
            .expect("review")
            .iter()
            .all(|review| review.author.id != harmed_author),
        "the harmed author is left asking the user nothing"
    );

    // The route the harmed author already held is untouched: this unit adds no
    // retirement, removal or reactivation behaviour.
    assert_eq!(
        route_snapshot(&db, inherited_route).await,
        route_before,
        "migration 081 writes no route row"
    );
    assert!(
        service
            .run_author(claim_author(&db, harmed_author, Utc::now() + Duration::hours(60)).await)
            .await
            .is_ok(),
        "the author stays runnable after the re-walk"
    );
}

async fn current_generation(db: &SqliteDb, author_id: i64) -> i64 {
    sqlx::query_scalar("SELECT evidence_generation FROM author_link_progress WHERE author_id = ?")
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("generation observation")
}

// ===========================================================================
// 14 — a dismissal is durable, and only the re-resolve door revokes it
// ===========================================================================

/// Door: the real dismiss, re-resolve and remove-route service doors over a real
/// `SqliteDb`.
/// D9-6 [U9-F05 + U9-R2-F01]: without durable dismissals the surviving
/// transliteration cards return every time the evidence fingerprint changes —
/// adding a book to the author is enough — so dismissing them is work the user
/// redoes indefinitely. The escape hatch has to be one transaction: a revocation
/// without the replay leaves a question that can never come back (the new
/// generation's attempts are terminal), and a replay without the revocation
/// re-suppresses immediately.
#[tokio::test]
async fn a_dismissal_survives_new_evidence_and_only_re_resolve_revokes_it() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "durable-dismissal",
        "Dismissal Work",
        "Walter Isaacson",
        WorkKeys::hc("4207"),
    )
    .await;

    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(r#"{"contribution":null,"author":{"id":85,"name":"Уолтер Айзексон"}}"#),
        )),
    };
    service.run_author(claim).await.expect("first link pass");

    let review = db.list_review(user_id).await.expect("review");
    let candidate_id = review[0].candidates[0].id;
    service
        .dismiss_candidate(user_id, candidate_id)
        .await
        .expect("real dismiss door");

    // New evidence, through the real work writer: this is the exact trigger that
    // makes a dismissed question come back today.
    work_service(db.clone(), "durable-dismissal-2")
        .add(
            user_id,
            add_box_candidate(
                "Second Dismissal Work",
                "Walter Isaacson",
                WorkKeys::hc("4208"),
            ),
        )
        .await
        .expect("production settled-work writer");

    let claim = claim_author(&db, author_id, Utc::now() + Duration::minutes(10)).await;
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(r#"{"contribution":null,"author":{"id":85,"name":"Уолтер Айзексон"}}"#),
        )),
    };
    service
        .run_author(claim)
        .await
        .expect("re-walk on new evidence");

    assert!(
        db.list_review(user_id)
            .await
            .expect("review")
            .iter()
            .all(|review| review.author.id != author_id),
        "an answered question must never be asked again on the automatic path"
    );

    // A worker takes the author and is still holding it when the user acts. A
    // lease the completed re-walk already released proves nothing about
    // invalidation, so this claim is taken and deliberately never run.
    let generation_before = current_generation(&db, author_id).await;
    let stale_claim = claim_author(&db, author_id, Utc::now() + Duration::hours(25)).await;

    // The user asks for the author to be looked at again, with the evidence
    // unchanged. A test that changes the evidence here proves nothing.
    let progress = service
        .re_resolve(user_id, author_id)
        .await
        .expect("real re-resolve door");

    // (a) revoked, never deleted.
    let dismissal: (String, Option<String>) =
        sqlx::query_as("SELECT status, revoked_at FROM author_link_candidates WHERE id = ?")
            .bind(candidate_id)
            .fetch_one(db.pool())
            .await
            .expect("the dismissed row must still exist");
    assert_eq!(
        dismissal.0, "dismissed",
        "the user's decision and its resolution time stay on the record"
    );
    assert!(
        dismissal.1.is_some(),
        "only the revocation stamp says the answer no longer binds"
    );

    // (b) immediately due, with the live lease invalidated on the row itself and
    // the worker still holding its token refused.
    assert!(progress.next_attempt_at <= Utc::now());
    assert!(progress.lease_token.is_none());
    assert!(progress.lease_expires_at.is_none());
    let claim_fields: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT claim_token, lease_until FROM author_link_progress WHERE author_id = ?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("progress claim fields");
    assert_eq!(
        claim_fields,
        (None, None),
        "the user just changed the state the worker is deciding from"
    );
    assert!(
        db.begin_evidence_generation(stale_claim, generation_before + 1)
            .await
            .is_err(),
        "the stale worker must not be able to write under its voided claim"
    );

    // (c) + (d) a new generation opens even though nothing about the evidence
    // changed, and the question comes back.
    let claim = claim_author(&db, author_id, Utc::now() + Duration::minutes(20)).await;
    service
        .run_author(claim)
        .await
        .expect("post-revocation re-walk");
    assert!(
        current_generation(&db, author_id).await > generation_before,
        "the replay must open a new generation on unchanged evidence"
    );
    let review = db.list_review(user_id).await.expect("review");
    let returned = review
        .iter()
        .find(|review| review.author.id == author_id)
        .expect("the revoked question must return");
    assert!(
        returned
            .candidates
            .iter()
            .any(|candidate| candidate.key == hc_route("85")
                && matches!(candidate.status, AuthorLinkCandidateStatus::Pending)),
        "the question the user un-answered is pending again"
    );
}

/// Door: the real remove-route service door.
/// D9-6, negative control: `remove_route` re-arms the author through the *same*
/// `UserReResolve` trigger the re-resolve door uses, and the shared value is why
/// this is easy to get wrong. Removing one route is not a statement that every
/// question the user already answered should be re-asked.
#[tokio::test]
async fn removing_a_route_re_arms_the_author_without_revoking_a_dismissal() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author_id, _work_id, claim) = settled_author_claim(
        &db,
        user_id,
        "remove-route-control",
        "Control Work",
        "Walter Isaacson",
        WorkKeys::hc("4209"),
    )
    .await;

    // One edition crediting two people with a blank `contribution`: the agreeing
    // one attaches the removable route, the transliteration parks the question
    // this test dismisses. Both come through the real Hardcover adapter, so both
    // credits are certified by the gateway that reads the wire shape.
    let service = AuthorLinkingServiceImpl {
        db: db.clone(),
        gateway: real_gateway(StubHttpFetcher::with_ok(
            200,
            hardcover_body(
                r#"{"contribution":null,"author":{"id":86,"name":"Walter Isaacson"}},
                   {"contribution":null,"author":{"id":87,"name":"Уолтер Айзексон"}}"#,
            ),
        )),
    };
    service.run_author(claim).await.expect("link pass");

    let review = db.list_review(user_id).await.expect("review");
    let candidate_id = review[0]
        .candidates
        .iter()
        .find(|candidate| candidate.key == hc_route("87"))
        .expect("the transliteration question")
        .id;
    service
        .dismiss_candidate(user_id, candidate_id)
        .await
        .expect("real dismiss door");

    let route_id = db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes")
        .first()
        .expect("the agreeing credit attached")
        .id;
    service
        .remove_route(user_id, author_id, route_id)
        .await
        .expect("real remove-route door");

    let revoked_at: Option<String> =
        sqlx::query_scalar("SELECT revoked_at FROM author_link_candidates WHERE id = ?")
            .bind(candidate_id)
            .fetch_one(db.pool())
            .await
            .expect("the dismissed row must still exist");
    assert!(
        revoked_at.is_none(),
        "removing one route must not re-ask every question the user answered"
    );

    // And the dismissal still suppresses: the re-armed author re-walks without
    // putting the question back.
    let claim = claim_author(&db, author_id, Utc::now() + Duration::minutes(10)).await;
    service.run_author(claim).await.expect("re-armed re-walk");
    assert!(
        !db.list_review(user_id)
            .await
            .expect("review")
            .iter()
            .any(|review| {
                if review.author.id == author_id {
                    review.candidates.iter().any(|c| c.key == hc_route("87"))
                } else {
                    false
                }
            }),
        "the dismissed question stays suppressed after a route removal"
    );
}
