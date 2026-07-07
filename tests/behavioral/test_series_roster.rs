//! Behavioral tests for the persisted series roster (REQ-010, sprint-c-series;
//! amended by N1 gr-series-parser 2026-07-03):
//! AC-022 (amended) — first expansion fetches + persists exactly once; later
//! expansions serve from the store with zero GR requests. An UNREADABLE/empty
//! parse is NEVER persisted: the view degrades to linked works and the next
//! expansion refetches, so the store heals when GR yields books again;
//! AC-023 — a monitor-worker run write-throughs the roster it fetched, but an
//! empty fetch never erases stored data (provider-drift guard);
//! N1 — the roster keeps exactly the header's primary count: GR lists
//! omnibuses/split editions/translations after the primaries.

use std::sync::Arc;

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateSeriesDbRequest, SeriesCacheDb, SeriesCacheEntry,
    SeriesDb, SeriesRosterDb,
};
use livrarr_domain::services::{
    FetchResponse, MonitorSeriesServiceRequest, SeriesMonitorWorkerParams, SeriesQueryService,
};
use livrarr_domain::UserId;
use livrarr_metadata::series_query_service::SeriesQueryServiceImpl;
use livrarr_metadata::work_service::{StubNoLlm, WorkServiceImpl};

type TestWorkService = WorkServiceImpl<
    SqliteDb,
    StubEnrichmentWorkflow,
    StubHttpFetcher,
    livrarr_metadata::work_service::StubNoLlm,
    livrarr_metadata::DefaultMergeEngine,
    livrarr_metadata::work_service::StubTagService,
>;

type TestSeriesService =
    SeriesQueryServiceImpl<SqliteDb, StubHttpFetcher, TestWorkService, StubNoLlm>;

fn series_service(db: SqliteDb, fetcher: StubHttpFetcher) -> TestSeriesService {
    let work_service: Arc<TestWorkService> = Arc::new(WorkServiceImpl::new(
        db.clone(),
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    ));
    SeriesQueryServiceImpl::new(db, fetcher, work_service, StubNoLlm)
}

fn ok_page(body: &str) -> Result<FetchResponse, livrarr_domain::services::FetchError> {
    Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: body.as_bytes().to_vec(),
    })
}

/// Minimal NEW-layout (2026-07 React) GR series page: header carrying the
/// primary count, one SeriesList blob with two decorated entries, single page.
const ROSTER_HTML: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;The Dresden Files Series&quot;,&quot;subtitle&quot;:&quot;2 primary works • 2 total works&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47212&quot;,&quot;title&quot;:&quot;Storm Front (The Dresden Files, #1)&quot;,&quot;bookTitleBare&quot;:&quot;Storm Front&quot;,&quot;publicationDate&quot;:&quot;2000&quot;}},{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47213&quot;,&quot;title&quot;:&quot;Fool Moon (The Dresden Files, #2)&quot;,&quot;bookTitleBare&quot;:&quot;Fool Moon&quot;,&quot;publicationDate&quot;:&quot;2001&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:2,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

/// Same page but GR lists a third, non-primary entry (an omnibus) after the
/// two primaries — the header still says "2 primary works".
const ROSTER_HTML_WITH_TRAILING_OMNIBUS: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;The Dresden Files Series&quot;,&quot;subtitle&quot;:&quot;2 primary works • 3 total works&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47212&quot;,&quot;title&quot;:&quot;Storm Front (The Dresden Files, #1)&quot;,&quot;bookTitleBare&quot;:&quot;Storm Front&quot;,&quot;publicationDate&quot;:&quot;2000&quot;}},{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47213&quot;,&quot;title&quot;:&quot;Fool Moon (The Dresden Files, #2)&quot;,&quot;bookTitleBare&quot;:&quot;Fool Moon&quot;,&quot;publicationDate&quot;:&quot;2001&quot;}},{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;90001&quot;,&quot;title&quot;:&quot;Dresden Files Boxed Set (The Dresden Files, #1-2)&quot;,&quot;bookTitleBare&quot;:&quot;Dresden Files Boxed Set&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:3,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}"></div>
</body></html>"#;

/// A 200 body in no known series-page shape (drift / interstitial).
const UNREADABLE_HTML: &str = "<html>redesigned beyond recognition</html>";

/// Page 1 of a two-page series: the header declares THREE primaries but this
/// page lists only two, and the pagination blob says another page exists
/// (numWorks 3, perPage 2).
const ROSTER_HTML_PAGE1_OF_2: &str = r#"<html><body>
<div data-react-class="ReactComponents.SeriesHeader" data-react-props="{&quot;title&quot;:&quot;The Dresden Files Series&quot;,&quot;subtitle&quot;:&quot;3 primary works • 3 total works&quot;,&quot;description&quot;:{&quot;html&quot;:&quot;&quot;}}"></div>
<div data-react-class="ReactComponents.SeriesList" data-react-props="{&quot;series&quot;:[{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47212&quot;,&quot;title&quot;:&quot;Storm Front (The Dresden Files, #1)&quot;,&quot;bookTitleBare&quot;:&quot;Storm Front&quot;,&quot;publicationDate&quot;:&quot;2000&quot;}},{&quot;isLibrarianView&quot;:false,&quot;readOnlyStars&quot;:false,&quot;book&quot;:{&quot;bookId&quot;:&quot;47213&quot;,&quot;title&quot;:&quot;Fool Moon (The Dresden Files, #2)&quot;,&quot;bookTitleBare&quot;:&quot;Fool Moon&quot;,&quot;publicationDate&quot;:&quot;2001&quot;}}]}"></div>
<div data-react-class="ReactComponents.FullPagePaginationControls" data-react-props="{&quot;numWorks&quot;:3,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:2}"></div>
</body></html>"#;

async fn seed_series(db: &SqliteDb, gr_key: &str) -> (UserId, i64, i64) {
    let user_id = create_test_user(db).await;
    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Jim Butcher".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: Some("12345".to_string()),
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("author");
    let series = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: author.id,
            name: "The Dresden Files".to_string(),
            gr_key: gr_key.to_string(),
            monitor_ebook: false,
            monitor_audiobook: false,
            monitor_language: None,
            work_count: 2,
        })
        .await
        .expect("series");
    (user_id, author.id, series.id)
}

/// AC-022: first expansion fetches the GR series page once and persists the
/// roster; the second expansion serves from the store with ZERO new fetches.
#[tokio::test]
async fn first_expand_fetches_once_then_store_only() {
    let db = create_test_db().await;
    let fetcher = StubHttpFetcher::with_ok(200, ROSTER_HTML.as_bytes().to_vec());
    let (user_id, _author_id, series_id) = seed_series(&db, "99001").await;
    let svc = series_service(db.clone(), fetcher.clone());

    let first = svc.series_books(user_id, series_id).await.expect("books");
    assert!(first.roster_available);
    assert_eq!(first.rows.len(), 2);
    let fetches_after_first = fetcher.call_count();
    assert!(fetches_after_first >= 1, "first expand must fetch GR");

    let roster = db
        .get_series_roster(series_id)
        .await
        .expect("roster query")
        .expect("roster persisted");
    assert_eq!(roster.entries.len(), 2);
    assert_eq!(roster.entries[0].position, Some(1.0));
    assert_eq!(roster.entries[1].position, Some(2.0));

    let second = svc.series_books(user_id, series_id).await.expect("books");
    assert_eq!(second.rows.len(), 2);
    assert_eq!(
        fetcher.call_count(),
        fetches_after_first,
        "second expand must serve from the store with zero GR requests"
    );
}

/// AC-022 (amended, N1): an unreadable/empty parse is never persisted — the
/// view degrades to linked works, the next expansion refetches, and the store
/// heals as soon as GR yields books.
#[tokio::test]
async fn unreadable_parse_is_not_persisted_and_heals_on_refetch() {
    let db = create_test_db().await;
    let fetcher = StubHttpFetcher::with_ok(200, UNREADABLE_HTML.as_bytes().to_vec());
    fetcher.push_response(ok_page(ROSTER_HTML));
    let (user_id, _author_id, series_id) = seed_series(&db, "99002").await;
    let svc = series_service(db.clone(), fetcher.clone());

    let first = svc.series_books(user_id, series_id).await.expect("books");
    assert!(
        !first.roster_available,
        "an unreadable roster degrades — it is not presented as empty truth"
    );
    assert!(first.rows.is_empty());
    assert_eq!(fetcher.call_count(), 1);
    assert!(
        db.get_series_roster(series_id)
            .await
            .expect("roster query")
            .is_none(),
        "emptiness must never be persisted"
    );

    let second = svc.series_books(user_id, series_id).await.expect("books");
    assert!(second.roster_available);
    assert_eq!(second.rows.len(), 2);
    assert_eq!(
        fetcher.call_count(),
        2,
        "an absent roster refetches instead of serving cached nothing"
    );
    assert_eq!(
        db.get_series_roster(series_id)
            .await
            .expect("roster query")
            .expect("healed roster")
            .entries
            .len(),
        2
    );

    svc.series_books(user_id, series_id).await.expect("books");
    assert_eq!(
        fetcher.call_count(),
        2,
        "a healed roster serves from the store"
    );
}

/// N1: the roster keeps exactly the header's primary count — trailing
/// omnibus/split/translation entries never enter the roster.
#[tokio::test]
async fn roster_respects_primary_count_cutoff() {
    let db = create_test_db().await;
    let fetcher =
        StubHttpFetcher::with_ok(200, ROSTER_HTML_WITH_TRAILING_OMNIBUS.as_bytes().to_vec());
    let (user_id, _author_id, series_id) = seed_series(&db, "99006").await;
    let svc = series_service(db.clone(), fetcher.clone());

    let books = svc.series_books(user_id, series_id).await.expect("books");
    assert!(books.roster_available);
    assert_eq!(
        books.rows.len(),
        2,
        "only the primary works enter the roster"
    );
    assert_eq!(
        db.get_series_roster(series_id)
            .await
            .expect("roster query")
            .expect("persisted")
            .entries
            .len(),
        2
    );
}

/// AC-023: the monitor worker write-throughs the roster it fetched; a
/// subsequent expansion triggers no GR fetch.
#[tokio::test]
async fn worker_run_persists_roster_and_expansion_uses_store() {
    let db = create_test_db().await;
    let fetcher = StubHttpFetcher::with_ok(200, ROSTER_HTML.as_bytes().to_vec());
    let (user_id, author_id, series_id) = seed_series(&db, "99003").await;
    // Worker only creates works for monitored series.
    db.update_series_flags(user_id, series_id, true, false, None)
        .await
        .expect("monitor");
    let svc = series_service(db.clone(), fetcher.clone());

    svc.run_series_monitor_worker(SeriesMonitorWorkerParams {
        user_id,
        author_id,
        series_id,
        series_name: "The Dresden Files".to_string(),
        series_gr_key: "99003".to_string(),
        monitor_ebook: true,
        monitor_audiobook: false,
    })
    .await
    .expect("worker run");

    let roster = db
        .get_series_roster(series_id)
        .await
        .expect("roster query")
        .expect("worker write-through persisted the roster");
    assert_eq!(roster.entries.len(), 2);

    let fetches_after_worker = fetcher.call_count();
    let books = svc.series_books(user_id, series_id).await.expect("books");
    assert!(books.roster_available);
    assert_eq!(
        fetcher.call_count(),
        fetches_after_worker,
        "expansion after a worker run must not refetch"
    );
}

/// N1 drift guard: a worker run whose fetch parses EMPTY leaves the stored
/// roster and work_count untouched — provider drift must never erase data.
#[tokio::test]
async fn worker_empty_fetch_does_not_erase_stored_roster() {
    let db = create_test_db().await;
    let fetcher = StubHttpFetcher::with_ok(200, ROSTER_HTML.as_bytes().to_vec());
    fetcher.push_response(ok_page(UNREADABLE_HTML));
    let (user_id, author_id, series_id) = seed_series(&db, "99005").await;
    db.update_series_flags(user_id, series_id, true, false, None)
        .await
        .expect("monitor");
    let svc = series_service(db.clone(), fetcher.clone());

    // First expand consumes the good response and persists 2 entries.
    let first = svc.series_books(user_id, series_id).await.expect("books");
    assert_eq!(first.rows.len(), 2);

    // The worker run consumes the drifted response.
    svc.run_series_monitor_worker(SeriesMonitorWorkerParams {
        user_id,
        author_id,
        series_id,
        series_name: "The Dresden Files".to_string(),
        series_gr_key: "99005".to_string(),
        monitor_ebook: true,
        monitor_audiobook: false,
    })
    .await
    .expect("worker run");

    let roster = db
        .get_series_roster(series_id)
        .await
        .expect("roster query")
        .expect("roster still present");
    assert_eq!(
        roster.entries.len(),
        2,
        "an empty fetch must not overwrite a good roster"
    );
    let series = db
        .get_series(user_id, series_id)
        .await
        .expect("series query")
        .expect("series");
    assert_eq!(
        series.work_count, 2,
        "work_count untouched on an empty fetch"
    );
}

/// N1 (review R-3): an unreadable LATER page must not yield a partial
/// roster — page 1 collects two of three declared primaries, page 2 is
/// drifted; the run must leave the stored roster and work_count untouched.
#[tokio::test]
async fn worker_partial_pagination_does_not_persist_partial_roster() {
    let db = create_test_db().await;
    let fetcher = StubHttpFetcher::with_ok(200, ROSTER_HTML.as_bytes().to_vec());
    fetcher.push_response(ok_page(ROSTER_HTML_PAGE1_OF_2));
    fetcher.push_response(ok_page(UNREADABLE_HTML));
    let (user_id, author_id, series_id) = seed_series(&db, "99007").await;
    db.update_series_flags(user_id, series_id, true, false, None)
        .await
        .expect("monitor");
    let svc = series_service(db.clone(), fetcher.clone());

    // Good single-page expand persists the full 2-entry roster.
    let first = svc.series_books(user_id, series_id).await.expect("books");
    assert_eq!(first.rows.len(), 2);

    // Worker run walks page 1 (2 of 3 declared primaries) then an unreadable
    // page 2 — the partial collection must be discarded, not persisted.
    svc.run_series_monitor_worker(SeriesMonitorWorkerParams {
        user_id,
        author_id,
        series_id,
        series_name: "The Dresden Files".to_string(),
        series_gr_key: "99007".to_string(),
        monitor_ebook: true,
        monitor_audiobook: false,
    })
    .await
    .expect("worker run");

    assert_eq!(
        fetcher.call_count(),
        3,
        "expand + both pagination pages were fetched"
    );
    let roster = db
        .get_series_roster(series_id)
        .await
        .expect("roster query")
        .expect("roster still present");
    assert_eq!(
        roster.entries.len(),
        2,
        "a partial pagination walk must not overwrite the stored roster"
    );
    let series = db
        .get_series(user_id, series_id)
        .await
        .expect("series query")
        .expect("series");
    assert_eq!(
        series.work_count, 2,
        "work_count untouched on a partial walk"
    );
}

/// N1 (review R-1): the stub-collision road applies the same
/// emptiness-is-never-truth rule — a collided row's stored-EMPTY roster
/// (pre-N1 break window) reads as absent, gets refetched, and heals.
#[tokio::test]
async fn stub_collision_with_stored_empty_roster_refetches_and_heals() {
    let db = create_test_db().await;
    // Only fetch expected: the series ROSTER page (the author's series list
    // is served from the pre-seeded cache).
    let fetcher = StubHttpFetcher::with_ok(200, ROSTER_HTML.as_bytes().to_vec());
    let user_id = create_test_user(&db).await;
    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Jim Butcher".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: Some("12345".to_string()),
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("author");
    // Existing GR-backed row that already owns the key the stub resolves to,
    // holding a stored-EMPTY roster AND the stale work_count 0 the
    // broken-parser window left behind.
    let gr_backed = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: author.id,
            name: "The Dresden Files".to_string(),
            gr_key: "99010".to_string(),
            monitor_ebook: false,
            monitor_audiobook: false,
            monitor_language: None,
            work_count: 0,
        })
        .await
        .expect("gr-backed series");
    db.save_series_roster(gr_backed.id, &[])
        .await
        .expect("seed empty roster");
    // The stub the user expands.
    let stub = db
        .upsert_series(CreateSeriesDbRequest {
            user_id,
            author_id: author.id,
            name: "The Dresden Files".to_string(),
            gr_key: "stub:the dresden files".to_string(),
            monitor_ebook: false,
            monitor_audiobook: false,
            monitor_language: None,
            work_count: 0,
        })
        .await
        .expect("stub series");
    // Author's GR series list served from cache — resolution needs no fetch.
    db.save_series_cache(
        author.id,
        &[SeriesCacheEntry {
            name: "The Dresden Files".to_string(),
            gr_key: "99010".to_string(),
            book_count: 2,
            language: None,
        }],
        None,
    )
    .await
    .expect("seed series cache");
    let svc = series_service(db.clone(), fetcher.clone());

    let books = svc.series_books(user_id, stub.id).await.expect("books");
    assert!(
        books.roster_available,
        "collision road must refetch through the stored-empty roster, not present it"
    );
    assert_eq!(books.rows.len(), 2);
    assert_eq!(
        fetcher.call_count(),
        1,
        "exactly the roster page fetch — series list came from the cache"
    );
    assert_eq!(
        db.get_series_roster(gr_backed.id)
            .await
            .expect("roster query")
            .expect("healed roster on the collided row")
            .entries
            .len(),
        2,
        "the collided row's empty roster heals in place"
    );
    let healed = db
        .get_series(user_id, gr_backed.id)
        .await
        .expect("series query")
        .expect("series");
    assert_eq!(
        healed.work_count, 2,
        "the stale work_count heals with the roster (count IS the roster size)"
    );
}

// =============================================================================
// #112: series-list language classification (author-level series tab, not
// the per-series roster above). Confidence gate reuses the project's one
// matching authority (identity_matching::title_verdict/author_verdict).
// =============================================================================

/// A GR author series-list page: one series link + a nearby book count,
/// matching what `parse_series_list_html` actually expects (older-style
/// `<a href="/series/{gr_key}...">`, not the React roster-detail layout).
fn series_list_html(gr_key: &str, name: &str, book_count: i32) -> String {
    format!(
        r#"<html><body><a href="/series/{gr_key}-x">{name}</a> ({book_count} books)</body></html>"#
    )
}

fn gb_search_response(items: Vec<serde_json::Value>) -> String {
    serde_json::json!({"items": items}).to_string()
}

async fn configure_gb_key(db: &SqliteDb) {
    use livrarr_db::{ConfigDb, UpdateMetadataConfigRequest};
    db.update_metadata_config(UpdateMetadataConfigRequest {
        hardcover_enabled: None,
        hardcover_api_token: None,
        llm_enabled: None,
        llm_provider: None,
        llm_endpoint: None,
        llm_api_key: None,
        llm_model: None,
        audnexus_url: None,
        languages: None,
        google_books_api_key: Some(Some("test-gb-key".to_string())),
    })
    .await
    .expect("configure GB key");
}

#[tokio::test]
async fn author_series_list_classifies_language_via_confident_gb_match() {
    let db = create_test_db().await;
    let (user_id, author_id, _series_id) = seed_series(&db, "88001").await;
    configure_gb_key(&db).await;

    let fetcher = StubHttpFetcher::new();
    fetcher.push_response(ok_page(&series_list_html("88001", "The Dresden Files", 17)));
    fetcher.push_response(ok_page(&gb_search_response(vec![serde_json::json!({
        "volumeInfo": {
            "title": "The Dresden Files",
            "authors": ["Jim Butcher"],
            "language": "en",
        }
    })])));
    let svc = series_service(db.clone(), fetcher.clone());

    let view = svc
        .refresh_author_series(user_id, author_id)
        .await
        .expect("refresh_author_series");

    assert_eq!(view.series.len(), 1);
    assert_eq!(
        view.series[0].language.as_deref(),
        Some("en"),
        "a title+author-matching GB volume must classify the series"
    );
}

#[tokio::test]
async fn author_series_list_defaults_to_target_language_on_author_mismatch() {
    // The confidence gate must not accept a title-only match: an unrelated
    // author's book with the same series title must not tag the language —
    // reviewed risk (R-2): a loose match could wrongly hide a real English
    // series. #112 follow-up: "no confident match" now defaults to the
    // author's own target language (absence of evidence isn't evidence of a
    // foreign series) rather than a literal Unknown — a real "language
    // unknown" tag showing on an author's own well-known series read as
    // broken, not cautious.
    let db = create_test_db().await;
    let (user_id, author_id, _series_id) = seed_series(&db, "88002").await;
    configure_gb_key(&db).await;

    let fetcher = StubHttpFetcher::new();
    fetcher.push_response(ok_page(&series_list_html("88002", "The Dresden Files", 17)));
    fetcher.push_response(ok_page(&gb_search_response(vec![serde_json::json!({
        "volumeInfo": {
            "title": "The Dresden Files",
            "authors": ["Some Unrelated Author"],
            "language": "es",
        }
    })])));
    let svc = series_service(db.clone(), fetcher.clone());

    let view = svc
        .refresh_author_series(user_id, author_id)
        .await
        .expect("refresh_author_series");

    assert_eq!(view.series.len(), 1);
    assert_eq!(
        view.series[0].language.as_deref(),
        Some("en"),
        "an author mismatch must never produce a false language verdict — \
         defaults to the author's own target language instead"
    );
}

#[tokio::test]
async fn monitor_series_uses_detected_language_over_the_requested_default() {
    // #112: the series-tab "Monitor" dropdown is a single section-wide
    // default (AuthorDetailPage.tsx) — it can't know a specific series is
    // foreign. The backend must override it with the cache's own detected
    // language when known, so a Spanish-only series never gets stamped "en"
    // just because that's what the dropdown happened to say.
    let db = create_test_db().await;
    let (user_id, author_id, _series_id) = seed_series(&db, "88003").await;

    db.save_series_cache(
        author_id,
        &[SeriesCacheEntry {
            name: "Criptonomicón".to_string(),
            gr_key: "88003".to_string(),
            book_count: 3,
            language: Some("es".to_string()),
        }],
        None,
    )
    .await
    .expect("seed series cache");

    let svc = series_service(db.clone(), StubHttpFetcher::new());

    let view = svc
        .monitor_series(
            user_id,
            author_id,
            MonitorSeriesServiceRequest {
                gr_key: "88003".to_string(),
                monitor_ebook: true,
                monitor_audiobook: false,
                // The frontend's section-wide dropdown still says English —
                // the detected "es" must win over this.
                language: Some("en".to_string()),
            },
        )
        .await
        .expect("monitor_series");

    assert_eq!(
        view.language.as_deref(),
        Some("es"),
        "the response must surface the real detected language"
    );

    let series = db
        .get_series(user_id, view.id)
        .await
        .expect("get_series")
        .expect("series row");
    assert_eq!(
        series.monitor_language.as_deref(),
        Some("es"),
        "the persisted monitor_language must be the detected language, not the requested default"
    );
}
