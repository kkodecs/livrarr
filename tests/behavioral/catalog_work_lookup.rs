//! Catalog Work lookup REQ-001..003. Run with `--test test_ilr_contracts catalog_work_lookup::`.
//! HTTP fixtures supply bytes only: matching, queue decisions, SQLite settlement,
//! Add and detail presentation all execute production code. Add's resolver and
//! enrichment queue both use concrete adapters. No captured routes or successful
//! provider outcomes are injected into Add.

use super::*;
use livrarr_domain::identity_matching::{
    classify_search_fallback, identity_key, parse_title, pick_best_candidate, title_verdict,
    SearchFallbackCandidate, SearchFallbackDecision, TitleVerdict,
};
use livrarr_domain::services::LedgerPassAccounting;
use livrarr_external_data::ProviderOutcome;

const AMP: &str = "The Strange Case of Dr. Jekyll & Mr. Hyde";
const AND: &str = "The Strange Case of Dr. Jekyll and Mr. Hyde";
const AUTHOR: &str = "Robert Louis Stevenson";
const ISBN: &str = "9789176393871";
const OL: &str = "OL123456W";
const HC: &str = "543210";

#[derive(Clone, Copy)]
enum IsbnReply {
    Missing,
    ServerError,
    RateLimited,
}

#[derive(Clone, Debug)]
struct ObservedRequest {
    url: String,
    body: Value,
    at: std::time::Instant,
}

impl ObservedRequest {
    fn hc_term(&self) -> Option<&str> {
        self.url
            .contains("api.hardcover.app")
            .then(|| {
                self.body
                    .pointer("/variables/query")
                    .and_then(Value::as_str)
            })
            .flatten()
    }

    fn isbn_lookup(&self, provider: MetadataProvider) -> bool {
        match provider {
            MetadataProvider::OpenLibrary => self.url.contains(&format!("/isbn/{ISBN}.json")),
            MetadataProvider::Hardcover => self.hc_term() == Some(ISBN),
            _ => false,
        }
    }

    fn title_search(&self, provider: MetadataProvider) -> bool {
        match provider {
            MetadataProvider::OpenLibrary => self.url.contains("/search.json"),
            MetadataProvider::Hardcover => self.hc_term().is_some_and(|term| term != ISBN),
            _ => false,
        }
    }
}

type Requests = Arc<Mutex<Vec<ObservedRequest>>>;

// Route by request contents, never a sticky response queue. Every request goes
// through HttpFetcherImpl and its real outbound queue; no external socket is used.
fn transport(reply: IsbnReply, search_hit: bool) -> (DiscoveryTransportFixture, Requests) {
    let requests: Requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let scripted = Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
        let entry = ObservedRequest {
            url: request.url.clone(),
            at: std::time::Instant::now(),
            body: request
                .body
                .as_deref()
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
                .unwrap_or(Value::Null),
        };
        observed.lock().unwrap().push(entry.clone());
        let hc = request.url.contains("api.hardcover.app");
        if entry.isbn_lookup(MetadataProvider::OpenLibrary)
            || entry.isbn_lookup(MetadataProvider::Hardcover)
        {
            return match reply {
                IsbnReply::Missing if hc => round13_response(round21_hardcover_search_response()),
                IsbnReply::Missing => round17_status_response(404, b"{}".to_vec()),
                IsbnReply::ServerError => round17_status_response(503, b"unavailable".to_vec()),
                IsbnReply::RateLimited => round17_status_response(429, b"slow down".to_vec()),
            };
        }
        if entry.title_search(MetadataProvider::OpenLibrary) {
            // The initial resolver's search is honestly empty. A later catalog
            // search returns a candidate after the edition miss, exercising the
            // queue fallback rather than satisfying identity before enrichment.
            let after_isbn = observed
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.isbn_lookup(MetadataProvider::OpenLibrary));
            let docs = if search_hit && after_isbn {
                // Different edition evidence cannot veto this Work match.
                json!([{"key": format!("/works/{OL}"), "title": AND,
                    "author_name": [AUTHOR], "isbn": ["9780306406157"]}])
            } else {
                json!([])
            };
            return round13_response(json!({"docs": docs}).to_string().into_bytes());
        }
        if entry.title_search(MetadataProvider::Hardcover) {
            let after_isbn = observed
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.isbn_lookup(MetadataProvider::Hardcover));
            let hits = if search_hit && after_isbn {
                json!([{"document": {"id": HC, "title": AND,
                    "author_names": [AUTHOR], "isbns": ["9780306406157"]}}])
            } else {
                json!([])
            };
            return round13_response(
                json!({"data": {"search": {"results": {"hits": hits}}}})
                    .to_string()
                    .into_bytes(),
            );
        }
        if hc && entry.body.pointer("/variables/id") == Some(&json!(543210)) {
            return round13_response(
                json!({"data": {"books_by_pk": {
                    "id": 543210, "title": AND, "description": "Catalog Work detail"
                }}})
                .to_string()
                .into_bytes(),
            );
        }
        if hc && entry.body.pointer("/variables/bookId") == Some(&json!(543210)) {
            return round13_response(
                json!({"data": {"editions": [{
                    "isbn_13": "9780306406157", "language": {"language": "English"}
                }]}})
                .to_string()
                .into_bytes(),
            );
        }
        if request.url.contains(&format!("/works/{OL}/editions.json")) {
            return round13_response(b"{\"entries\":[]}".to_vec());
        }
        if request.url.contains(&format!("/works/{OL}.json")) {
            return round13_response(
                json!({"title": AND,
                "description": "Catalog Work detail"})
                .to_string()
                .into_bytes(),
            );
        }
        if request.url.contains("auto_complete") {
            return round13_response(b"[]".to_vec());
        }
        panic!("unhandled catalog fixture request: {entry:?}");
    }) as Arc<_>;
    let mut fixture = round13_search_transport(scripted);
    fixture.hardcover_search = true;
    (fixture, requests)
}

async fn author(harness: &RouteHarness) -> i64 {
    // Pre-existing Author avoids unrelated delayed bibliography jobs in Add.
    harness
        .db
        .create_author(CreateAuthorDbRequest {
            user_id: harness.user_id,
            name: AUTHOR.to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("create existing Author")
        .0
        .id
}

fn add_request(title: &str) -> Value {
    json!({"title": title, "authorName": AUTHOR, "language": "en",
        "isbn13": ISBN, "olKey": null, "hcKey": null, "grKey": null,
        "asin": null, "candidateId": null, "coverManual": false})
}

async fn dispatch(harness: &RouteHarness, work_id: i64) -> livrarr_enrichment::ScatterGatherResult {
    let work = harness.db.get_work(harness.user_id, work_id).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(8),
        harness.state.provider_queue.dispatch_enrichment(
            &work,
            livrarr_enrichment::EnrichmentContext {
                priority: livrarr_domain::RequestPriority::Interactive,
                mode: livrarr_enrichment::EnrichmentMode::Manual,
                freshness: livrarr_domain::Freshness::Bypass,
                search_only: false,
            },
        ),
    )
    .await
    .expect("one dispatch must terminate")
    .expect("dispatch through the production provider queue")
}

async fn isbn_work(harness: &RouteHarness) -> i64 {
    // Existing-Work tests use the real transactional settlement writer and
    // Edition route ownership; they do not claim to cover the Add door.
    let work_id = seed_round13_search_work(
        harness,
        AND,
        AUTHOR,
        "en",
        Some((
            ilr::IdentityProvider::IsbnRegistry,
            ilr::RouteKind::Isbn13Edition,
            ISBN,
        )),
    )
    .await
    .0;
    // A real Goodreads Work route suppresses its unrelated title-search leg.
    // Otherwise that leg's honest miss could hide missing OL/HC accounting.
    // It is intentionally not a BookEdition fetch key.
    round22_add_work_route(
        harness,
        work_id,
        ilr::IdentityProvider::Goodreads,
        ilr::RouteKind::GoodreadsWork,
        "1357911",
    )
    .await;
    work_id
}

#[tokio::test]
async fn add_isbn_miss_saves_catalog_routes_and_selected_isbn_before_delayed_refresh() {
    let _breaker = lock_breaker().await;
    let (fixture, requests) = transport(IsbnReply::Missing, true);
    let harness =
        build_route_harness_with_identity_http(None, Vec::new(), Some(fixture), None, true).await;
    author(&harness).await;
    // Keep the title identical here: this isolates same-attempt fallback from
    // the separately pinned ampersand matcher regression.
    let started = std::time::Instant::now();
    let added = call_router_json(
        &harness,
        Method::POST,
        "/api/v1/work".into(),
        Some(add_request(AND)),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.json);
    assert_eq!(added.json["created"], true);
    assert_eq!(added.json["work"]["isbn13"], ISBN);
    assert!(added.json["work"]["olKey"].is_null());
    assert!(added.json["work"]["hcKey"].is_null());
    let work_id = added.json["work"]["id"].as_i64().unwrap();

    // The Add handler's next refresh sleeps five seconds AFTER complete_add.
    // Observe for at most four seconds after the last provider's FIRST ISBN
    // request. The delayed refresh cannot start before five seconds after
    // complete_add finishes. Allow initial resolver queue admission separately.
    // Leave Tokio time real for SQLite and queue workers.
    let deadline = started + Duration::from_secs(15);
    let mut captured = None;
    while std::time::Instant::now() < deadline {
        let identity = harness
            .db
            .read_captured_identity(harness.user_id, work_id)
            .await
            .unwrap();
        let ol = identity
            .active_routes
            .iter()
            .any(|r| r.kind == ilr::RouteKind::OpenLibraryWork && r.provider_scoped_id == OL);
        let hc = identity
            .active_routes
            .iter()
            .any(|r| r.kind == ilr::RouteKind::HardcoverWork && r.provider_scoped_id == HC);
        if ol && hc {
            captured = Some(identity);
            break;
        }
        let last_anchor = {
            let seen = requests.lock().unwrap();
            [MetadataProvider::OpenLibrary, MetadataProvider::Hardcover]
                .iter()
                .map(|p| seen.iter().find(|r| r.isbn_lookup(*p)).map(|r| r.at))
                .collect::<Option<Vec<_>>>()
                .and_then(|times| times.into_iter().max())
        };
        if last_anchor.is_some_and(|at| at.elapsed() >= Duration::from_secs(4)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let seen = requests.lock().unwrap().clone();
    for provider in [MetadataProvider::OpenLibrary, MetadataProvider::Hardcover] {
        let anchor = seen.iter().position(|r| r.isbn_lookup(provider));
        assert!(
            anchor.is_some(),
            "{provider:?} must really miss the selected edition: {seen:?}"
        );
        let search = seen
            .iter()
            .enumerate()
            .find(|(index, r)| *index > anchor.unwrap() && r.title_search(provider))
            .map(|(index, _)| index);
        assert!(search.is_some(),
            "REQ-002: {provider:?} must search the Work in the same Add attempt after ISBN NotFound; requests={seen:?}");
        assert!(
            anchor.unwrap() < search.unwrap(),
            "ISBN miss must precede Work search"
        );
        assert_eq!(seen.iter().filter(|r| r.isbn_lookup(provider)).count(), 1);
    }
    let captured =
        captured.expect("both real adapter search results must settle before delayed Refresh");
    assert!(captured
        .active_routes
        .iter()
        .any(|r| r.kind == ilr::RouteKind::Isbn13Edition
            && r.provider_scoped_id == ISBN
            && matches!(r.owner, RouteOwner::Edition(_))));
    for kind in [
        ilr::RouteKind::OpenLibraryWork,
        ilr::RouteKind::HardcoverWork,
    ] {
        assert!(
            captured.active_routes.iter().any(|r| r.kind == kind
                && matches!(
                    r.provenance,
                    ilr::RouteProvenance::TextDecisiveSearchFallback { .. }
                )),
            "Work match must travel through normal search settlement"
        );
    }
    let detail = call_router_json(
        &harness,
        Method::GET,
        format!("/api/v1/work/{work_id}"),
        None,
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.json["olKey"], OL, "{}", detail.json);
    assert_eq!(detail.json["hcKey"], HC, "{}", detail.json);
    assert_eq!(
        detail.json["isbn13"], ISBN,
        "chosen edition survives unrelated catalog ISBN"
    );
}

async fn known_work_id_precedes_isbn(provider: MetadataProvider) {
    let _breaker = lock_breaker().await;
    let (fixture, requests) = transport(IsbnReply::Missing, false);
    let harness = build_route_harness_with_provider_details(None, Vec::new(), Some(fixture)).await;
    let work_id = isbn_work(&harness).await;
    let (identity_provider, kind, key) = match provider {
        MetadataProvider::Hardcover => (
            ilr::IdentityProvider::Hardcover,
            ilr::RouteKind::HardcoverWork,
            HC,
        ),
        MetadataProvider::OpenLibrary => (
            ilr::IdentityProvider::OpenLibrary,
            ilr::RouteKind::OpenLibraryWork,
            OL,
        ),
        _ => unreachable!(),
    };
    round22_add_work_route(&harness, work_id, identity_provider, kind, key).await;
    let result = dispatch(&harness, work_id).await;
    let seen = requests.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|r| r.isbn_lookup(provider)),
        "REQ-001: known {provider:?} Work ID must outrank the obscure edition ISBN: {seen:?}"
    );
    assert!(
        !seen.iter().any(|r| r.title_search(provider)),
        "a known Work ID needs no search"
    );
    assert!(
        seen.iter().any(|r| match provider {
            MetadataProvider::Hardcover =>
                r.url.contains("api.hardcover.app")
                    && r.body.pointer("/variables/id") == Some(&json!(543210))
                    && r.body["query"]
                        .as_str()
                        .is_some_and(|q| q.contains("books_by_pk")),
            MetadataProvider::OpenLibrary => r.url.ends_with(&format!("/works/{OL}.json")),
            _ => false,
        }),
        "actual catalog detail request must address the Work key: {seen:?}"
    );
    assert!(
        matches!(
            result.outcomes.get(&provider),
            Some(ProviderOutcome::Success(_))
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn hardcover_work_id_precedes_unrecognized_isbn_on_actual_http_request() {
    known_work_id_precedes_isbn(MetadataProvider::Hardcover).await;
}

#[tokio::test]
async fn openlibrary_work_id_keeps_precedence_on_actual_http_request() {
    known_work_id_precedes_isbn(MetadataProvider::OpenLibrary).await;
}

#[tokio::test]
async fn healthy_isbn_misses_search_once_and_no_match_terminates() {
    let _breaker = lock_breaker().await;
    let (fixture, requests) = transport(IsbnReply::Missing, false);
    let harness = build_route_harness_with_provider_details(None, Vec::new(), Some(fixture)).await;
    let work_id = isbn_work(&harness).await;
    let result = dispatch(&harness, work_id).await;
    let seen = requests.lock().unwrap().clone();
    for provider in [MetadataProvider::OpenLibrary, MetadataProvider::Hardcover] {
        assert_eq!(seen.iter().filter(|r| r.isbn_lookup(provider)).count(), 1);
        assert_eq!(
            seen.iter().filter(|r| r.title_search(provider)).count(),
            1,
            "REQ-002/AC-004: one healthy miss permits one bounded same-attempt search: {seen:?}"
        );
        assert!(matches!(
            result.outcomes.get(&provider),
            Some(ProviderOutcome::NotFound)
        ));
        assert_eq!(
            harness
                .db
                .get_retry_state(harness.user_id, work_id, provider)
                .await
                .unwrap()
                .and_then(|s| s.last_outcome),
            Some(OutcomeClass::NotFound)
        );
    }
    assert!(result.search_leg_fired);
    assert_eq!(result.ledger_accounting, LedgerPassAccounting::CardOrMiss);
    assert!(result.search_provider_identity.is_empty());
    assert!(result.search_route_proposals.is_empty());
    // Generation ledger persistence/threshold is already covered by the real
    // convergence tests ac026c and round21_failed_search_leg in the parent.
}

async fn failed_isbn_does_not_search(reply: IsbnReply) {
    let _breaker = lock_breaker().await;
    let (fixture, requests) = transport(reply, true);
    let harness = build_route_harness_with_provider_details(None, Vec::new(), Some(fixture)).await;
    let work_id = isbn_work(&harness).await;
    let result = dispatch(&harness, work_id).await;
    let seen = requests.lock().unwrap().clone();
    for provider in [MetadataProvider::OpenLibrary, MetadataProvider::Hardcover] {
        assert_eq!(seen.iter().filter(|r| r.isbn_lookup(provider)).count(), 1);
        assert!(!seen.iter().any(|r| r.title_search(provider)),
            "failed {provider:?} HTTP request must never authorize a missing-edition fallback: {seen:?}");
        let outcome = result.outcomes.get(&provider).unwrap();
        // This existing harness has max_attempts=1. Both rate-limit and
        // server-error retries hit its existing budget; neither is NotFound.
        assert!(
            matches!(
                outcome,
                ProviderOutcome::PermanentFailure {
                    reason: livrarr_domain::PermanentFailureReason::RetryBudgetExhausted,
                }
            ),
            "{outcome:?}"
        );
        let state = harness
            .db
            .get_retry_state(harness.user_id, work_id, provider)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.last_outcome, Some(OutcomeClass::PermanentFailure));
    }
    assert_eq!(result.ledger_accounting, LedgerPassAccounting::LegFailed);
    assert!(!result.search_leg_fired);
    assert!(result.search_provider_identity.is_empty());
    assert!(result.search_route_proposals.is_empty());
}

#[tokio::test]
async fn isbn_server_errors_do_not_become_misses_or_burn_search_budget() {
    failed_isbn_does_not_search(IsbnReply::ServerError).await;
}

#[tokio::test]
async fn isbn_rate_limits_do_not_become_misses_or_burn_search_budget() {
    failed_isbn_does_not_search(IsbnReply::RateLimited).await;
}

#[test]
fn jekyll_ampersand_and_same_author_is_a_decisive_work_match() {
    for (seed, candidate) in [(AMP, AND), (AND, AMP)] {
        assert_eq!(
            title_verdict(&parse_title(seed), &parse_title(candidate)),
            TitleVerdict::Same,
            "REQ-003: ordinary English &/and title variants"
        );
        assert_eq!(
            pick_best_candidate(seed, AUTHOR, &[(candidate.into(), AUTHOR.into())], false),
            Some(0)
        );
        assert_eq!(
            classify_search_fallback(
                seed,
                AUTHOR,
                &[SearchFallbackCandidate {
                    title: candidate,
                    author: AUTHOR,
                    provider_work_id: OL,
                }],
                false
            ),
            SearchFallbackDecision::AutoLink { candidate_index: 0 }
        );
        assert_eq!(
            identity_key(seed, AUTHOR),
            identity_key(candidate, AUTHOR),
            "legacy exact-key storage must agree with the same title comparison"
        );
    }
}

#[test]
fn title_variation_does_not_erase_author_collection_subtitle_or_volume_evidence() {
    for (candidate, candidate_author) in [
        (AND, "Joanne Mattern"),
        (
            "The Strange Case of Dr. Jekyll and Mr. Hyde and Other Stories",
            AUTHOR,
        ),
        (
            "The Strange Case of Dr. Jekyll and Mr. Hyde: A Study Guide",
            AUTHOR,
        ),
        (
            "The Strange Case of Dr. Jekyll and Mr. Hyde: Book 2",
            AUTHOR,
        ),
    ] {
        assert_eq!(
            pick_best_candidate(
                AMP,
                AUTHOR,
                &[(candidate.into(), candidate_author.into())],
                false
            ),
            None,
            "must not auto-pick {candidate:?} by {candidate_author:?}"
        );
        assert!(!matches!(
            classify_search_fallback(
                AMP,
                AUTHOR,
                &[SearchFallbackCandidate {
                    title: candidate,
                    author: candidate_author,
                    provider_work_id: OL,
                }],
                false
            ),
            SearchFallbackDecision::AutoLink { .. }
        ));
    }
    assert_ne!(
        title_verdict(
            &parse_title("Jekyll & Hyde: Book 1"),
            &parse_title("Jekyll and Hyde: Book 2")
        ),
        TitleVerdict::Same
    );
    // An omitted conjunction is not an &/and spelling. Do not solve the bug by
    // deleting the word `and` or declaring arbitrary punctuation equivalent.
    assert_ne!(
        title_verdict(
            &parse_title(AND),
            &parse_title("The Strange Case of Dr. Jekyll Mr. Hyde")
        ),
        TitleVerdict::Same
    );
}

#[tokio::test]
async fn stored_ampersand_work_is_found_by_and_group_key_without_changing_display() {
    let harness = build_route_harness().await;
    let author_id = author(&harness).await;
    let mut command = settlement_commit(harness.user_id, author_id, None);
    command.identity_title = ilr::title_parts_from_provider(AMP.into(), None).unwrap();
    let saved = harness
        .db
        .commit_settlement(command)
        .await
        .unwrap()
        .identity;
    let query = ilr::title_parts_from_provider(AND.into(), None).unwrap();
    let found = harness
        .db
        .list_captured_identities_in_group(harness.user_id, query.normalized_main, author_id)
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|i| i.own_work_id).collect::<Vec<_>>(),
        vec![saved.own_work_id],
        "REQ-003: actual SQLite group lookup must share &/and normalization"
    );
    assert_eq!(
        found[0].identity_title.main, AMP,
        "display title must be preserved"
    );
    assert_eq!(
        harness
            .db
            .read_captured_identity(harness.user_id, saved.own_work_id)
            .await
            .unwrap()
            .identity_generation,
        saved.identity_generation,
        "lookup is read-only"
    );
}

#[tokio::test]
async fn equivalent_spelling_with_conflicting_work_id_enters_review_without_a_duplicate() {
    let _breaker = lock_breaker().await;
    let harness = build_route_harness().await;
    let author_id = author(&harness).await;
    let mut command = settlement_commit(harness.user_id, author_id, None);
    command.identity_title = ilr::title_parts_from_provider(AMP.into(), None).unwrap();
    let saved = harness
        .db
        .commit_settlement(command)
        .await
        .unwrap()
        .identity;
    round22_add_work_route(
        &harness,
        saved.own_work_id,
        ilr::IdentityProvider::Hardcover,
        ilr::RouteKind::HardcoverWork,
        HC,
    )
    .await;
    let mut request = add_request(AND);
    request["isbn13"] = Value::Null;
    request["hcKey"] = json!("543211");
    let response =
        call_router_json(&harness, Method::POST, "/api/v1/work".into(), Some(request)).await;
    assert_eq!(
        response.status,
        StatusCode::CONFLICT,
        "REQ-003: existing group with a conflicting catalog ID needs review: {}",
        response.json
    );
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id")
        .bind(harness.user_id)
        .fetch_all(harness.db.pool())
        .await
        .unwrap();
    assert_eq!(
        ids,
        vec![saved.own_work_id],
        "no hidden duplicate or automatic merge"
    );
    let after = harness
        .db
        .read_captured_identity(harness.user_id, saved.own_work_id)
        .await
        .unwrap();
    assert_eq!(after.identity_title.main, AMP);
    assert!(after
        .active_routes
        .iter()
        .any(|r| r.kind == ilr::RouteKind::HardcoverWork && r.provider_scoped_id == HC));
    let cards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 AND kind='GroupIdentity' AND status='pending'")
        .bind(harness.user_id).bind(saved.own_work_id).fetch_one(harness.db.pool()).await.unwrap();
    assert_eq!(cards, 1);
}

// Frozen outputs of HEAD 9f366b77, not the current normalization functions:
// title_parts_from_provider preserves punctuation in normalized_main, while
// identity_key/canonical_phrase removes punctuation (including the ampersand).
const HISTORICAL_AMP_MAIN: &str = "strange case of dr. jekyll & mr. hyde";
const HISTORICAL_AND_MAIN: &str = "strange case of dr. jekyll and mr. hyde";
const HISTORICAL_LEGACY_AMP_KEY: &str = "strange case of dr jekyll mr hyde";
const HISTORICAL_AUTHOR_KEY: &str = "robert louis stevenson";
const OMITTED_CONJUNCTION: &str = "The Strange Case of Dr. Jekyll Mr. Hyde";

async fn historical_routed_work(
    harness: &RouteHarness,
    author_id: i64,
    display: &str,
    stored_main: &str,
    hardcover_id: &str,
    isbn: &str,
) -> CapturedIdentity {
    let mut command = settlement_commit(harness.user_id, author_id, None);
    // This tuple is exactly what the old title_parts_from_provider produced
    // for these plain main titles. The real transaction writes both old keys;
    // no SQL rekey or injected resolution outcome is needed.
    command.identity_title = IdentityTitleTuple {
        main: display.into(),
        subtitle: None,
        volume: None,
        normalized_main: stored_main.into(),
        normalized_subtitle: String::new(),
        normalized_volume: String::new(),
        provenance: ilr::EvidenceProvenance::Provider(ilr::IdentityProvider::Other(
            "provider".into(),
        )),
    };
    command.routes = [
        (
            ilr::IdentityProvider::Hardcover,
            ilr::RouteKind::HardcoverWork,
            hardcover_id,
        ),
        (
            ilr::IdentityProvider::IsbnRegistry,
            ilr::RouteKind::Isbn13Edition,
            isbn,
        ),
    ]
    .into_iter()
    .map(|(provider, kind, value)| ilr::WorkRoute {
        id: 0,
        user_id: harness.user_id,
        owner: RouteOwner::Work(0),
        resolved_work_id: 0,
        provider,
        kind,
        provider_scoped_id: value.into(),
        state: ilr::WorkRouteState::Active,
        provenance: ilr::RouteProvenance::UserChoice,
        user_confirmed: true,
        observed_at: Utc::now(),
    })
    .collect();
    let captured = harness
        .db
        .commit_settlement(command)
        .await
        .unwrap()
        .identity;
    assert!(captured
        .active_routes
        .iter()
        .any(|r| r.kind == ilr::RouteKind::HardcoverWork
            && r.provider_scoped_id == hardcover_id
            && r.owner == RouteOwner::Work(captured.own_work_id)));
    let edition_id = captured
        .active_routes
        .iter()
        .find_map(|route| {
            if route.kind == ilr::RouteKind::Isbn13Edition && route.provider_scoped_id == isbn {
                if let RouteOwner::Edition(id) = route.owner {
                    return Some(id);
                }
            }
            None
        })
        .expect("production settlement homes the ISBN on an Edition");
    let edition_work: i64 =
        sqlx::query_scalar("SELECT work_id FROM editions WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(edition_id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(edition_work, captured.own_work_id);
    captured
}

// Snapshot only the packet's named read-only observables, not the entire Work
// DTO or unrelated enrichment state. Every route column is included so neither
// reownership nor replacement of an existing route can go unnoticed.
async fn historical_lookup_state(harness: &RouteHarness) -> Value {
    let works: Vec<String> = sqlx::query_scalar(
        "SELECT json_object('id',id,'title',title,'normalized_title',normalized_title, \
         'normalized_author',normalized_author,'main',normalized_identity_main, \
         'subtitle',normalized_identity_subtitle,'volume',normalized_identity_volume, \
         'generation',identity_generation) FROM works WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    let routes: Vec<String> = sqlx::query_scalar(
        "SELECT json_object('id',id,'user_id',user_id,'owner_type',owner_type, \
         'work_id',work_id,'edition_id',edition_id,'resolved_work_id',resolved_work_id, \
         'provider',provider,'kind',kind,'value',provider_scoped_id,'state',state, \
         'provenance',provenance,'user_confirmed',user_confirmed,'observed_at',observed_at) \
         FROM identity_routes WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    let edition_owners: Vec<(i64, i64)> =
        sqlx::query_as("SELECT id,work_id FROM editions WHERE user_id=?1 ORDER BY id")
            .bind(harness.user_id)
            .fetch_all(harness.db.pool())
            .await
            .unwrap();
    json!({"works": works, "routes": routes, "edition_owners": edition_owners})
}

#[tokio::test]
async fn historical_compatibility_group_reads_keep_both_works_and_edition_routes_unchanged() {
    let harness = build_route_harness().await;
    let author_id = author(&harness).await;
    let amp = historical_routed_work(&harness, author_id, AMP, HISTORICAL_AMP_MAIN, HC, ISBN).await;
    let and = historical_routed_work(
        &harness,
        author_id,
        AND,
        HISTORICAL_AND_MAIN,
        "543211",
        "9780306406157",
    )
    .await;
    assert_ne!(
        amp.own_work_id, and.own_work_id,
        "historical Works really coexist"
    );
    let stored_keys: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT title,normalized_title,normalized_identity_main FROM works \
         WHERE user_id=?1 ORDER BY id",
    )
    .bind(harness.user_id)
    .fetch_all(harness.db.pool())
    .await
    .unwrap();
    assert_eq!(
        stored_keys,
        vec![
            (
                AMP.into(),
                HISTORICAL_AMP_MAIN.into(),
                HISTORICAL_AMP_MAIN.into()
            ),
            (
                AND.into(),
                HISTORICAL_AND_MAIN.into(),
                HISTORICAL_AND_MAIN.into()
            ),
        ],
        "fixture must retain the old writer's actual stored recipes"
    );
    let before = historical_lookup_state(&harness).await;
    let mut expected = vec![amp.own_work_id, and.own_work_id];
    expected.sort_unstable();
    for spelling in [AMP, AND] {
        let query = ilr::title_parts_from_provider(spelling.into(), None).unwrap();
        let found = harness
            .db
            .list_captured_identities_in_group(harness.user_id, query.normalized_main, author_id)
            .await
            .unwrap();
        let mut ids: Vec<_> = found.iter().map(|work| work.own_work_id).collect();
        ids.sort_unstable();
        // Check immutability even when membership itself is wrong.
        assert_eq!(historical_lookup_state(&harness).await, before,
            "group read for {spelling:?} must preserve both Works, old keys, titles, generations and owners");
        assert_eq!(
            ids, expected,
            "both historical spellings belong in the group read for {spelling:?}"
        );
    }
}

async fn historical_legacy_amp_work(harness: &RouteHarness) -> Work {
    let author_id = author(harness).await;
    // HEAD add_fast computes identity_key and passes it to create_work. Freeze
    // that exact output; create_work/insert_work_row supplies the historical
    // row shape. In particular this old key omits '&', unlike the display.
    let (work, created) = harness
        .db
        .create_work(CreateWorkDbRequest {
            user_id: harness.user_id,
            title: AMP.into(),
            author_name: AUTHOR.into(),
            normalized_title: HISTORICAL_LEGACY_AMP_KEY.into(),
            normalized_author: HISTORICAL_AUTHOR_KEY.into(),
            author_id: Some(author_id),
            language: Some("en".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(created);
    work
}

#[tokio::test]
async fn historical_compatibility_batch_add_finds_ampersand_under_its_old_legacy_key() {
    let harness = build_route_harness().await;
    let existing = historical_legacy_amp_work(&harness).await;
    let candidate = livrarr_domain::seed::seed_list_import(
        livrarr_domain::seed::SeedInput {
            title: AND.into(),
            author_name: AUTHOR.into(),
            language: livrarr_domain::seed::SeedLanguage::resolve(Some("en"), "en"),
            author_ol_key: None,
            year: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        livrarr_domain::identity::IdentityState::Pending {
            reason: livrarr_domain::identity::PendingReason::NoCandidates,
            seed_anchors: None,
            top_candidates: vec![],
        },
        None,
    );
    // No identifier/bridge can short-circuit this: it enters the actual batch
    // service's normalized-title dedup through a production list-import seed.
    let added = harness
        .state
        .work_service
        .add_fast(harness.user_id, candidate)
        .await
        .unwrap();
    assert!(
        !added.created,
        "and-spelling batch Add must find the old ampersand Work"
    );
    assert_eq!(added.work.id, existing.id);
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id,title,normalized_title FROM works WHERE user_id=?1 ORDER BY id")
            .bind(harness.user_id)
            .fetch_all(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![(existing.id, AMP.into(), HISTORICAL_LEGACY_AMP_KEY.into())],
        "dedup must not create a duplicate or rewrite the existing display/key"
    );
}

#[tokio::test]
async fn historical_compatibility_legacy_key_collision_does_not_match_omitted_conjunction() {
    let harness = build_route_harness().await;
    let existing = historical_legacy_amp_work(&harness).await;
    let (query_title, query_author) = identity_key(OMITTED_CONJUNCTION, AUTHOR);
    assert_eq!(
        query_title, HISTORICAL_LEGACY_AMP_KEY,
        "this is the actual old-key collision, not a different lookup key"
    );
    // Both readers are real add_fast dedup entrances (Pending and Confirmed).
    // Returning no match is the named requirement; creation/merge policy for a
    // new omitted-conjunction title is deliberately not asserted here.
    let matches = harness
        .db
        .find_by_normalized_match(harness.user_id, &query_title, &query_author)
        .await
        .unwrap();
    let adopt = harness
        .db
        .find_normalized_match_no_anchor_for_user(harness.user_id, &query_title, &query_author)
        .await
        .unwrap();
    let matched_ids: Vec<_> = matches.iter().map(|work| work.id).collect();
    let adopt_id = adopt.as_ref().map(|work| work.id);
    assert!(matched_ids.is_empty() && adopt_id.is_none(),
        "stored title still contains '&'; a lossy old key must not make the omitted conjunction equivalent: matches={matched_ids:?}, adopt={adopt_id:?}");
    let stored: (String, String) =
        sqlx::query_as("SELECT title,normalized_title FROM works WHERE user_id=?1 AND id=?2")
            .bind(harness.user_id)
            .bind(existing.id)
            .fetch_one(harness.db.pool())
            .await
            .unwrap();
    assert_eq!(stored, (AMP.into(), HISTORICAL_LEGACY_AMP_KEY.into()));
}

// Production-review regressions. These use the same real router/SQLite/road
// harness, with no provider successes configured or identifier shortcuts in
// the batch candidates. The original thirteen tests above remain unchanged.
mod review_fixes {
    use super::*;

    fn direct_request(title: &str, ol: Option<&str>, hc: Option<&str>) -> Value {
        let mut request = add_request(title);
        request["isbn13"] = Value::Null;
        request["olKey"] = json!(ol);
        request["hcKey"] = json!(hc);
        request
    }

    fn batch_candidate(title: &str) -> livrarr_domain::identity::WorkCandidate {
        livrarr_domain::seed::seed_list_import(
            livrarr_domain::seed::SeedInput {
                title: title.into(),
                author_name: AUTHOR.into(),
                language: livrarr_domain::seed::SeedLanguage::resolve(Some("en"), "en"),
                author_ol_key: None,
                year: None,
                cover_url: None,
                detail_url: None,
                description: None,
                series_name: None,
                series_position: None,
            },
            livrarr_domain::identity::IdentityState::Pending {
                reason: livrarr_domain::identity::PendingReason::NoCandidates,
                seed_anchors: None,
                top_candidates: vec![],
            },
            None,
        )
    }

    async fn direct_create(harness: &RouteHarness, request: Value) -> CapturedIdentity {
        let response =
            call_router_json(harness, Method::POST, "/api/v1/work".into(), Some(request)).await;
        assert_eq!(response.status, StatusCode::OK, "setup: {}", response.json);
        assert_eq!(response.json["created"], true, "setup must create a Work");
        let id = response.json["work"]["id"].as_i64().unwrap();
        harness
            .db
            .read_captured_identity(harness.user_id, id)
            .await
            .unwrap()
    }

    async fn work_ids(harness: &RouteHarness) -> Vec<i64> {
        sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id")
            .bind(harness.user_id)
            .fetch_all(harness.db.pool())
            .await
            .unwrap()
    }

    async fn pending_cards(harness: &RouteHarness, work_id: i64) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_review_cards WHERE user_id=?1 AND work_id=?2 AND kind='GroupIdentity' AND status='pending'")
            .bind(harness.user_id).bind(work_id).fetch_one(harness.db.pool()).await.unwrap()
    }

    fn has_work_route(identity: &CapturedIdentity, kind: ilr::RouteKind, value: &str) -> bool {
        identity.active_routes.iter().any(|route| {
            route.kind == kind
                && route.provider_scoped_id == value
                && route.owner == RouteOwner::Work(identity.own_work_id)
        })
    }

    #[tokio::test]
    async fn direct_add_equivalent_mains_with_conflicting_subtitles_requires_review() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        author(&harness).await;
        let original = direct_create(
            &harness,
            direct_request("Jekyll & Hyde: Critical Essays", Some(OL), None),
        )
        .await;
        assert_eq!(original.identity_title.main, "Jekyll & Hyde");
        assert_eq!(
            original.identity_title.subtitle.as_deref(),
            Some("Critical Essays")
        );
        assert!(has_work_route(
            &original,
            ilr::RouteKind::OpenLibraryWork,
            OL
        ));

        // Different providers supply noncontradictory routes: subtitle evidence
        // alone must prevent the second Direct Add from silently attaching.
        let response = call_router_json(
            &harness,
            Method::POST,
            "/api/v1/work".into(),
            Some(direct_request(
                "Jekyll and Hyde: A Study Guide",
                None,
                Some(HC),
            )),
        )
        .await;
        let after = harness
            .db
            .read_captured_identity(harness.user_id, original.own_work_id)
            .await
            .unwrap();
        let cards = pending_cards(&harness, original.own_work_id).await;
        let incoming_active = has_work_route(&after, ilr::RouteKind::HardcoverWork, HC);
        eprintln!("subtitle conflict: status={}, response={}, pending_cards={cards}, incoming_hc_active={incoming_active}", response.status, response.json);
        assert_eq!(
            response.status,
            StatusCode::CONFLICT,
            "conflicting real subtitles must require review"
        );
        assert_eq!(work_ids(&harness).await, vec![original.own_work_id]);
        assert_eq!(
            after.identity_title, original.identity_title,
            "preserve the original identity tuple"
        );
        assert!(has_work_route(&after, ilr::RouteKind::OpenLibraryWork, OL));
        assert!(
            !incoming_active,
            "the incoming Hardcover route must remain inactive"
        );
        assert_eq!(cards, 1);
    }

    #[tokio::test]
    async fn direct_add_shared_openlibrary_does_not_hide_conflicting_hardcover() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        author(&harness).await;
        let original = direct_create(&harness, direct_request(AMP, Some(OL), Some(HC))).await;
        assert!(has_work_route(
            &original,
            ilr::RouteKind::OpenLibraryWork,
            OL
        ));
        assert!(has_work_route(&original, ilr::RouteKind::HardcoverWork, HC));

        let response = call_router_json(
            &harness,
            Method::POST,
            "/api/v1/work".into(),
            Some(direct_request(AND, Some(OL), Some("543211"))),
        )
        .await;
        let after = harness
            .db
            .read_captured_identity(harness.user_id, original.own_work_id)
            .await
            .unwrap();
        let cards = pending_cards(&harness, original.own_work_id).await;
        let incoming_active = has_work_route(&after, ilr::RouteKind::HardcoverWork, "543211");
        eprintln!("catalog conflict: status={}, response={}, pending_cards={cards}, incoming_hc_active={incoming_active}", response.status, response.json);
        assert_eq!(
            response.status,
            StatusCode::CONFLICT,
            "shared OL must not override contradictory HC Work IDs"
        );
        assert_eq!(work_ids(&harness).await, vec![original.own_work_id]);
        assert_eq!(after.identity_title, original.identity_title);
        assert!(has_work_route(&after, ilr::RouteKind::OpenLibraryWork, OL));
        assert!(has_work_route(&after, ilr::RouteKind::HardcoverWork, HC));
        assert!(
            !incoming_active,
            "conflicting Hardcover route must remain inactive"
        );
        assert_eq!(cards, 1);
    }

    #[tokio::test]
    async fn batch_add_omitted_conjunction_does_not_return_historical_ampersand_identity() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        let original = historical_legacy_amp_work(&harness).await;
        // This is the baseline legacy anchor writer, not a v2 settlement that
        // would replace the old normalized-title recipe and erase the collision.
        livrarr_domain::services::WorkIdentityRepository::confirm_anchor(
            &harness.db,
            original.id,
            livrarr_domain::identity::AnchorType::new(
                livrarr_domain::identity::AnchorType::OL_WORK,
            ),
            OL,
            livrarr_domain::identity::AnchorSetter::User,
        )
        .await
        .unwrap();
        let (query_key, _) = identity_key(OMITTED_CONJUNCTION, AUTHOR);
        assert_eq!(
            query_key, HISTORICAL_LEGACY_AMP_KEY,
            "fixture must reach the old-key collision"
        );
        let stored_before: (String, String) =
            sqlx::query_as("SELECT title,normalized_title FROM works WHERE user_id=?1 AND id=?2")
                .bind(harness.user_id)
                .bind(original.id)
                .fetch_one(harness.db.pool())
                .await
                .unwrap();
        assert_eq!(
            stored_before,
            (AMP.into(), HISTORICAL_LEGACY_AMP_KEY.into())
        );

        let result = harness
            .state
            .work_service
            .add_fast(harness.user_id, batch_candidate(OMITTED_CONJUNCTION))
            .await;
        eprintln!(
            "omitted-conjunction batch Add: original={}, outcome={:?}",
            original.id,
            result.as_ref().map(|added| (added.work.id, added.created))
        );
        let stored_title: String =
            sqlx::query_scalar("SELECT title FROM works WHERE user_id=?1 AND id=?2")
                .bind(harness.user_id)
                .bind(original.id)
                .fetch_one(harness.db.pool())
                .await
                .unwrap();
        assert_eq!(stored_title, AMP, "preserve the historical Work's display");
        let routes: Vec<(i64, String)> = sqlx::query_as("SELECT work_id,anchor_value FROM work_identity_anchors WHERE user_id=?1 AND work_id=?2 AND anchor_type='ol_work' AND confidence='confirmed' AND superseded_by IS NULL")
            .bind(harness.user_id).bind(original.id).fetch_all(harness.db.pool()).await.unwrap();
        assert_eq!(
            routes,
            vec![(original.id, OL.into())],
            "preserve historical route ownership"
        );
        match result {
            Ok(added) => assert_ne!(added.work.id, original.id, "omitted conjunction must not successfully return the ampersand Work as the requested identity"),
            // Distinct creation and explicit identity refusal/review are both
            // allowed. Infrastructure/setup errors are not a passing outcome.
            Err(livrarr_domain::services::WorkServiceError::AlreadyExists
                | livrarr_domain::services::WorkServiceError::EnrichmentConflict
                | livrarr_domain::services::WorkServiceError::Validation(_)) => {},
            Err(error) => panic!("unexpected failure instead of an identity outcome: {error:?}"),
        }
    }

    #[tokio::test]
    async fn batch_add_and_spelling_reuses_current_identity_road_ampersand_work() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        author(&harness).await;
        let original = direct_create(&harness, direct_request(AMP, None, None)).await;
        assert_eq!(original.identity_title.main, AMP);
        assert!(
            original.active_routes.is_empty(),
            "fixture has no identifier shortcut"
        );
        let added = harness
            .state
            .work_service
            .add_fast(harness.user_id, batch_candidate(AND))
            .await
            .unwrap();
        let ids = work_ids(&harness).await;
        eprintln!(
            "current-writer batch Add: original={}, returned={}, created={}, work_ids={ids:?}",
            original.own_work_id, added.work.id, added.created
        );
        assert_eq!(
            added.work.id, original.own_work_id,
            "and spelling must find the current identity-road ampersand Work"
        );
        assert!(!added.created);
        assert_eq!(ids, vec![original.own_work_id]);
        let after = harness
            .db
            .read_captured_identity(harness.user_id, original.own_work_id)
            .await
            .unwrap();
        assert_eq!(after.identity_title, original.identity_title);
    }

    #[tokio::test]
    async fn batch_add_unchanged_historical_title_with_five_conjunctions_reuses_work() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        let author_id = author(&harness).await;
        const TITLE: &str = "Alpha & Bravo and Charlie and Delta and Echo and Foxtrot";
        // Frozen identity_key output at 9f366b77: '&' was removed, while all
        // four literal 'and' words survived. Use the same real legacy writer
        // as the existing compatibility fixtures; never rewrite the stored key.
        const OLD_KEY: &str = "alpha bravo and charlie and delta and echo and foxtrot";
        let (original, created) = harness
            .db
            .create_work(CreateWorkDbRequest {
                user_id: harness.user_id,
                title: TITLE.into(),
                author_name: AUTHOR.into(),
                normalized_title: OLD_KEY.into(),
                normalized_author: HISTORICAL_AUTHOR_KEY.into(),
                author_id: Some(author_id),
                language: Some("en".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(created);

        let added = harness
            .state
            .work_service
            .add_fast(harness.user_id, batch_candidate(TITLE))
            .await
            .unwrap();
        let ids = work_ids(&harness).await;
        eprintln!(
            "five-conjunction batch Add: original={}, returned={}, created={}, work_ids={ids:?}",
            original.id, added.work.id, added.created
        );
        assert_eq!(
            added.work.id, original.id,
            "unchanged historical mixed title must deduplicate beyond four conjunctions"
        );
        assert!(!added.created);
        assert_eq!(ids, vec![original.id]);
        let stored: (String, String) =
            sqlx::query_as("SELECT title,normalized_title FROM works WHERE user_id=?1 AND id=?2")
                .bind(harness.user_id)
                .bind(original.id)
                .fetch_one(harness.db.pool())
                .await
                .unwrap();
        assert_eq!(stored, (TITLE.into(), OLD_KEY.into()));
    }
}

// R2 complete-tuple regressions: each case creates its own Work through the
// authenticated Direct Add route, then exercises the real batch Add consumer.
mod complete_tuple {
    use super::*;

    async fn direct_create(
        harness: &RouteHarness,
        title: &str,
        subtitle: Option<&str>,
        volume: Option<&str>,
    ) -> CapturedIdentity {
        author(harness).await;
        let mut request = add_request(title);
        request["isbn13"] = Value::Null;
        request["olKey"] = json!(OL);
        let response =
            call_router_json(harness, Method::POST, "/api/v1/work".into(), Some(request)).await;
        assert_eq!(response.status, StatusCode::OK, "setup: {}", response.json);
        assert_eq!(response.json["created"], true, "setup must create a Work");
        let work_id = response.json["work"]["id"].as_i64().unwrap();
        // Check actual writer columns: neither subtitle nor volume may remain
        // hidden inside the main string for this fixture to cover the defect.
        let stored: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT title,subtitle,identity_volume FROM works WHERE user_id=?1 AND id=?2",
        )
        .bind(harness.user_id)
        .bind(work_id)
        .fetch_one(harness.db.pool())
        .await
        .unwrap();
        assert_eq!(
            stored,
            (
                "Jekyll & Hyde".into(),
                subtitle.map(str::to_string),
                volume.map(str::to_string)
            ),
            "Direct Add must persist the intended separate tuple for {title:?}"
        );
        let identity = harness
            .db
            .read_captured_identity(harness.user_id, work_id)
            .await
            .unwrap();
        assert_eq!(identity.identity_title.main, stored.0);
        assert_eq!(identity.identity_title.subtitle, stored.1);
        assert_eq!(identity.identity_title.volume, stored.2);
        assert!(
            identity.active_routes.iter().any(|route| {
                route.kind == ilr::RouteKind::OpenLibraryWork
                    && route.provider_scoped_id == OL
                    && route.owner == RouteOwner::Work(work_id)
            }),
            "fixture must have a real route to preserve"
        );
        eprintln!("created separate tuple: work_id={work_id}, stored={stored:?}");
        identity
    }

    fn batch_candidate(title: &str) -> livrarr_domain::identity::WorkCandidate {
        // No provider id, edition bridge or source payload can short-circuit
        // complete-title validation in the batch service.
        livrarr_domain::seed::seed_list_import(
            livrarr_domain::seed::SeedInput {
                title: title.into(),
                author_name: AUTHOR.into(),
                language: livrarr_domain::seed::SeedLanguage::resolve(Some("en"), "en"),
                author_ol_key: None,
                year: None,
                cover_url: None,
                detail_url: None,
                description: None,
                series_name: None,
                series_position: None,
            },
            livrarr_domain::identity::IdentityState::Pending {
                reason: livrarr_domain::identity::PendingReason::NoCandidates,
                seed_anchors: None,
                top_candidates: vec![],
            },
            None,
        )
    }

    async fn assert_original_preserved(harness: &RouteHarness, original: &CapturedIdentity) {
        let after = harness
            .db
            .read_captured_identity(harness.user_id, original.own_work_id)
            .await
            .unwrap();
        assert_eq!(
            after.identity_title, original.identity_title,
            "preserve the complete original tuple"
        );
        for route in &original.active_routes {
            assert!(
                after.active_routes.iter().any(|current| {
                    current.id == route.id
                        && current.provider == route.provider
                        && current.kind == route.kind
                        && current.provider_scoped_id == route.provider_scoped_id
                        && current.owner == route.owner
                        && current.resolved_work_id == route.resolved_work_id
                }),
                "preserve active route {} and its ownership",
                route.id
            );
        }
        // Batch Add is a write: generation and unrelated metadata are not pinned.
    }

    async fn assert_batch_reuses(harness: &RouteHarness, original: &CapturedIdentity, title: &str) {
        let added = harness
            .state
            .work_service
            .add_fast(harness.user_id, batch_candidate(title))
            .await
            .unwrap();
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id")
            .bind(harness.user_id)
            .fetch_all(harness.db.pool())
            .await
            .unwrap();
        eprintln!("equivalent full tuple: requested={title:?}, original={}, returned={}, created={}, work_ids={ids:?}", original.own_work_id, added.work.id, added.created);
        assert_original_preserved(harness, original).await;
        assert_eq!(
            added.work.id, original.own_work_id,
            "full equivalent title {title:?} must reuse the original Work"
        );
        assert!(!added.created);
        assert_eq!(
            ids,
            vec![original.own_work_id],
            "full identity equivalence must not create a duplicate"
        );
    }

    async fn assert_batch_does_not_reuse(
        harness: &RouteHarness,
        original: &CapturedIdentity,
        title: &str,
    ) {
        let result = harness
            .state
            .work_service
            .add_fast(harness.user_id, batch_candidate(title))
            .await;
        eprintln!(
            "distinct tuple: requested={title:?}, original={}, outcome={:?}",
            original.own_work_id,
            result.as_ref().map(|added| (added.work.id, added.created))
        );
        assert_original_preserved(harness, original).await;
        match result {
            Ok(added) => assert_ne!(
                added.work.id, original.own_work_id,
                "{title:?} must not successfully return a Work with a different complete identity"
            ),
            Err(
                livrarr_domain::services::WorkServiceError::AlreadyExists
                | livrarr_domain::services::WorkServiceError::EnrichmentConflict
                | livrarr_domain::services::WorkServiceError::Validation(_),
            ) => {}
            Err(error) => {
                panic!("infrastructure failure is not a supported identity outcome: {error:?}")
            }
        }
    }

    #[tokio::test]
    async fn full_equivalent_subtitle_reuses_direct_add_work() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        let original = direct_create(
            &harness,
            "Jekyll & Hyde: Critical Essays",
            Some("Critical Essays"),
            None,
        )
        .await;
        assert_batch_reuses(&harness, &original, "Jekyll and Hyde: Critical Essays").await;
    }

    #[tokio::test]
    async fn bare_equivalent_main_does_not_return_subtitled_work() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        let original = direct_create(
            &harness,
            "Jekyll & Hyde: Critical Essays",
            Some("Critical Essays"),
            None,
        )
        .await;
        assert_batch_does_not_reuse(&harness, &original, "Jekyll and Hyde").await;
    }

    #[tokio::test]
    async fn full_equivalent_volume_reuses_direct_add_work() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        // RE_SERIES_VOLUME recognizes "Vol. 1"; title_parts_from_provider
        // removes its trailing parenthetical from main and stores volume=1.
        let original = direct_create(&harness, "Jekyll & Hyde (Vol. 1)", None, Some("1")).await;
        assert_batch_reuses(&harness, &original, "Jekyll and Hyde (Vol. 1)").await;
    }

    #[tokio::test]
    async fn different_volume_does_not_return_direct_add_work() {
        let _breaker = lock_breaker().await;
        let harness = build_route_harness().await;
        let original = direct_create(&harness, "Jekyll & Hyde (Vol. 1)", None, Some("1")).await;
        assert_batch_does_not_reuse(&harness, &original, "Jekyll and Hyde (Vol. 2)").await;
    }

    // Design-check supplements live here to reuse the complete-tuple fixtures
    // without changing any of the twenty-two existing tests or their helpers.
    mod tuple_supplement {
        use super::*;

        #[tokio::test]
        async fn bare_equivalent_main_does_not_return_volume_work() {
            let _breaker = lock_breaker().await;
            let harness = build_route_harness().await;
            let original = direct_create(&harness, "Jekyll & Hyde (Vol. 1)", None, Some("1")).await;
            assert_batch_does_not_reuse(&harness, &original, "Jekyll and Hyde").await;
        }

        #[tokio::test]
        async fn anchored_legacy_collision_cannot_succeed_without_requested_anchor() {
            let _breaker = lock_breaker().await;
            let harness = build_route_harness().await;
            let original = historical_legacy_amp_work(&harness).await;
            // Preserve a nonempty original route through the actual baseline
            // legacy writer, without rekeying the row through v2 settlement.
            livrarr_domain::services::WorkIdentityRepository::confirm_anchor(
                &harness.db,
                original.id,
                livrarr_domain::identity::AnchorType::new(
                    livrarr_domain::identity::AnchorType::OL_WORK,
                ),
                OL,
                livrarr_domain::identity::AnchorSetter::User,
            )
            .await
            .unwrap();
            let (request_key, request_author_key) = identity_key(OMITTED_CONJUNCTION, AUTHOR);
            assert_eq!(request_key, HISTORICAL_LEGACY_AMP_KEY);
            assert_eq!(request_author_key, HISTORICAL_AUTHOR_KEY);
            let before: (String, String, String) = sqlx::query_as(
                "SELECT title,normalized_title,normalized_author FROM works WHERE user_id=?1 AND id=?2",
            )
            .bind(harness.user_id).bind(original.id).fetch_one(harness.db.pool()).await.unwrap();
            assert_eq!(
                before,
                (AMP.into(), request_key.clone(), request_author_key.clone()),
                "fixture must reach the actual lossy-key guarded-insert collision"
            );

            const REQUESTED_OL: &str = "OL987654W";
            let result = harness
                .db
                .create_work_with_anchor(
                    CreateWorkDbRequest {
                        user_id: harness.user_id,
                        title: OMITTED_CONJUNCTION.into(),
                        author_name: AUTHOR.into(),
                        normalized_title: request_key,
                        normalized_author: request_author_key,
                        author_id: original.author_id,
                        language: Some("en".into()),
                        ..Default::default()
                    },
                    REQUESTED_OL,
                    livrarr_domain::identity::AnchorSetter::User,
                )
                .await;
            let requested_owners: Vec<i64> = sqlx::query_scalar(
                "SELECT work_id FROM work_identity_anchors WHERE user_id=?1 AND anchor_type='ol_work' \
                 AND anchor_value=?2 AND confidence='confirmed' AND superseded_by IS NULL ORDER BY work_id",
            )
            .bind(harness.user_id).bind(REQUESTED_OL).fetch_all(harness.db.pool()).await.unwrap();
            eprintln!("anchored legacy collision: original={}, result={:?}, requested_anchor={REQUESTED_OL}, confirmed_owners={requested_owners:?}",
                original.id, result.as_ref().map(|(work, created)| (work.id, *created, &work.title, &work.ol_key)));

            let after: (String, String, String) = sqlx::query_as(
                "SELECT title,normalized_title,normalized_author FROM works WHERE user_id=?1 AND id=?2",
            )
            .bind(harness.user_id).bind(original.id).fetch_one(harness.db.pool()).await.unwrap();
            assert_eq!(
                after, before,
                "preserve the historical Work's display and stored keys"
            );
            let original_anchors: Vec<String> = sqlx::query_scalar(
                "SELECT anchor_value FROM work_identity_anchors WHERE user_id=?1 AND work_id=?2 \
                 AND anchor_type='ol_work' AND confidence='confirmed' AND superseded_by IS NULL ORDER BY anchor_value",
            )
            .bind(harness.user_id).bind(original.id).fetch_all(harness.db.pool()).await.unwrap();
            assert_eq!(
                original_anchors,
                vec![OL.to_string()],
                "preserve the original confirmed route and its owner"
            );
            match result {
                Ok((work, _created)) => {
                    assert_ne!(
                        work.id, original.id,
                        "omitted conjunction is a distinct requested identity"
                    );
                    assert_eq!(work.title, OMITTED_CONJUNCTION);
                    assert_eq!(work.author_id, original.author_id);
                    assert_eq!(requested_owners, vec![work.id],
                        "successful anchored creation must persist the requested confirmed OL anchor on the returned Work");
                }
                Err(
                    livrarr_domain::DbError::Conflict { .. }
                    | livrarr_domain::DbError::IdentityCollision { .. },
                ) => {
                    // An explicit identity refusal is also supported, but it
                    // must not leave a partial new Work or requested anchor.
                    let ids: Vec<i64> =
                        sqlx::query_scalar("SELECT id FROM works WHERE user_id=?1 ORDER BY id")
                            .bind(harness.user_id)
                            .fetch_all(harness.db.pool())
                            .await
                            .unwrap();
                    assert_eq!(ids, vec![original.id]);
                    assert!(requested_owners.is_empty());
                }
                Err(error) => {
                    panic!("infrastructure failure is not an explicit identity refusal: {error:?}")
                }
            }
        }
    }
}
