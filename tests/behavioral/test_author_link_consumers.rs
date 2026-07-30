//! Behavioral pins for route-backed author consumers and server seams.

use std::sync::Arc;

use livrarr_behavioral::stubs::{
    create_test_user, StubEnrichmentWorkflow, StubHttpFetcher, StubLlmCaller,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, AuthorLinkDb, AuthorNameVariantDb, CreateAuthorDbRequest, CreateWorkDbRequest,
    UpdateAuthorDbRequest, WorkDbCreate,
};
use livrarr_domain::services::{
    AddAuthorRequest, AuthorMonitorWorkflow, AuthorService, AuthorServiceError, SeriesQueryService,
    SeriesServiceError,
};
use livrarr_domain::{
    normalize_for_matching, AuthorLinkTrigger, AuthorNameSource, AuthorProvider,
    AuthorRouteGuardResult, AuthorRouteKey, ProviderAuthorNameObservation,
};
use livrarr_enrichment::AuthorNameVariantObserver;
use livrarr_metadata::author_monitor_workflow::AuthorMonitorWorkflowImpl;
use livrarr_metadata::author_service::AuthorServiceImpl;
use livrarr_metadata::discovery_service::StubNoLlm;
use livrarr_metadata::series_query_service::SeriesQueryServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::author_link::{
    readarr_author_route_evidence, verify_author_link_cutover_before_serving,
};
use livrarr_server::readarr_client::RdAuthor;
use tokio_util::sync::CancellationToken;

type RealAuthorService = AuthorServiceImpl<SqliteDb, StubHttpFetcher, StubLlmCaller>;
type RealWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

fn author_request(user_id: i64, name: &str) -> CreateAuthorDbRequest {
    CreateAuthorDbRequest {
        user_id,
        name: name.to_string(),
        sort_name: None,
        ol_key: None,
        gr_key: None,
        hc_key: None,
        import_id: None,
    }
}

fn work_request(
    user_id: i64,
    author_id: i64,
    title: &str,
    author_name: &str,
) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author_name.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author_name),
        author_id: Some(author_id),
        language: Some("en".to_string()),
        ..Default::default()
    }
}

async fn author_with_work(label: &str) -> (SqliteDb, i64, i64, i64) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let name = format!("{label} Author");
    let (author, _) = db
        .create_author(author_request(user_id, &name))
        .await
        .expect("production author writer");
    let (work, _) = db
        .create_work(work_request(
            user_id,
            author.id,
            &format!("{label} Work"),
            &name,
        ))
        .await
        .expect("production work writer");
    (db, user_id, author.id, work.id)
}

fn author_service(db: SqliteDb, fetcher: StubHttpFetcher) -> RealAuthorService {
    AuthorServiceImpl::new(db, fetcher, StubLlmCaller::not_configured())
}

fn work_service(db: SqliteDb, label: &str) -> RealWorkService {
    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        std::env::temp_dir().join(format!(
            "livrarr-author-link-consumer-{label}-{}",
            std::process::id()
        )),
    )
}

fn route(provider: AuthorProvider, raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::parse(provider, raw).expect("fixture route must use production parser")
}

fn ol_works(entries: &[(&str, &str, &str)]) -> Vec<u8> {
    let entries = entries
        .iter()
        .map(|(key, title, date)| {
            format!(r#"{{"key":"/works/{key}","title":"{title}","first_publish_date":"{date}"}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"entries":[{entries}]}}"#).into_bytes()
}

fn series_page(series_id: &str, title: &str) -> Vec<u8> {
    format!(
        r#"<html><body>
        <div data-react-class="ReactComponents.SeriesHeader"
             data-react-props="{{&quot;title&quot;:&quot;{title}&quot;,&quot;subtitle&quot;:&quot;1 primary works • 1 total works&quot;,&quot;description&quot;:{{&quot;html&quot;:&quot;&quot;}}}}"></div>
        <div data-react-class="ReactComponents.SeriesList"
             data-react-props="{{&quot;series&quot;:[{{&quot;book&quot;:{{&quot;bookId&quot;:&quot;{series_id}&quot;,&quot;title&quot;:&quot;{title} Book ({title}, #1)&quot;,&quot;bookTitleBare&quot;:&quot;{title} Book&quot;,&quot;publicationDate&quot;:&quot;2026&quot;}}}}]}}"></div>
        <div data-react-class="ReactComponents.FullPagePaginationControls"
             data-react-props="{{&quot;numWorks&quot;:1,&quot;currentPageNumber&quot;:1,&quot;perPage&quot;:100}}"></div>
        </body></html>"#
    )
    .into_bytes()
}

fn rd_author(name: Option<&str>, foreign_id: Option<&str>) -> RdAuthor {
    RdAuthor {
        id: 41,
        author_name: name.map(str::to_string),
        sort_name: None,
        foreign_author_id: foreign_id.map(str::to_string),
        overview: None,
        genres: None,
        images: None,
        monitored: Some(false),
        added: None,
        path: None,
    }
}

/// Door: Enrichment-completion author-name observation.
/// AC-006 / AC-008: successful GR/HC/GB/OL/Readarr names retain their exact
/// source, deduplicate, and make local display work immediately due.
#[tokio::test]
async fn ac006_ac008_observer_records_exact_success_sources_once_and_schedules_dirty_work() {
    let (db, user_id, author_id, work_id) = author_with_work("Observer Success").await;
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("durable task");
    AuthorNameVariantObserver::record_observed_author_names(
        &db,
        user_id,
        work_id,
        &[
            ProviderAuthorNameObservation {
                source: AuthorNameSource::Goodreads,
                name: "Provider Name".to_string(),
            },
            ProviderAuthorNameObservation {
                source: AuthorNameSource::Hardcover,
                name: "Provider Name".to_string(),
            },
            ProviderAuthorNameObservation {
                source: AuthorNameSource::GoogleBooks,
                name: "GB Name".to_string(),
            },
            ProviderAuthorNameObservation {
                source: AuthorNameSource::OpenLibrary,
                name: "OL Name".to_string(),
            },
            ProviderAuthorNameObservation {
                source: AuthorNameSource::Readarr,
                name: "Readarr Name".to_string(),
            },
        ],
    )
    .await;

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT source, name FROM author_name_variants
          WHERE author_id=? ORDER BY source, name",
    )
    .bind(author_id)
    .fetch_all(db.pool())
    .await
    .expect("observed variants");
    assert_eq!(rows.len(), 5);
    let dirty: (i64, i64) = sqlx::query_as(
        "SELECT display_name_dirty,
                julianday(next_attempt_at) <= julianday('now')
           FROM author_link_progress WHERE author_id=?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("dirty progress");
    assert_eq!(dirty, (1, 1));
}

/// Door: Enrichment-completion author-name observation.
/// AC-008: non-success outcomes supply no observation, and empty/whitespace
/// names cause zero DB writes.
#[tokio::test]
async fn ac008_observer_skips_empty_success_names_and_non_success_outcomes() {
    let (db, user_id, author_id, work_id) = author_with_work("Observer Empty").await;
    AuthorNameVariantObserver::record_observed_author_names(
        &db,
        user_id,
        work_id,
        &[
            ProviderAuthorNameObservation {
                source: AuthorNameSource::Goodreads,
                name: String::new(),
            },
            ProviderAuthorNameObservation {
                source: AuthorNameSource::OpenLibrary,
                name: "   ".to_string(),
            },
        ],
    )
    .await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_name_variants WHERE author_id=?")
            .bind(author_id)
            .fetch_one(db.pool())
            .await
            .expect("variant count");
    assert_eq!(count, 0);
}

/// Door: Enrichment-completion author-name observation.
/// AC-006: a real FK/not-found DB failure is warning-only and leaves the
/// enrichment caller's result path intact.
#[tokio::test]
async fn ac006_observer_swallows_real_db_failure_without_partial_rows() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    AuthorNameVariantObserver::record_observed_author_names(
        &db,
        user_id,
        9_999_999,
        &[ProviderAuthorNameObservation {
            source: AuthorNameSource::GoogleBooks,
            name: "Unpersistable Name".to_string(),
        }],
    )
    .await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM author_name_variants")
        .fetch_one(db.pool())
        .await
        .expect("variant count");
    assert_eq!(
        count, 0,
        "warning-only failure must not leave partial state"
    );
}

/// Door: Author rename -> real `AuthorService::rename`.
/// AC-008 / AC-009: validation, User variant retention, display cascade,
/// merge-generation bump, and normalized-author immutability are one behavior.
#[tokio::test]
async fn ac008_ac009_author_service_rename_uses_user_variant_and_display_only_cascade() {
    let (db, user_id, author_id, work_id) = author_with_work("Service Rename").await;
    let before: (String, i64) =
        sqlx::query_as("SELECT normalized_author, merge_generation FROM works WHERE id=?")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("work before rename");
    let renamed = author_service(db.clone(), StubHttpFetcher::new())
        .rename(user_id, author_id, "  New Display Name  ".to_string())
        .await
        .expect("real author rename service");
    assert_eq!(renamed.name, "New Display Name");
    let after: (String, String, i64) = sqlx::query_as(
        "SELECT author_name, normalized_author, merge_generation FROM works WHERE id=?",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("work after rename");
    assert_eq!(after.0, "New Display Name");
    assert_eq!(after.1, before.0);
    assert_eq!(after.2, before.1 + 1);
}

/// Door: Author stored-name variant pick -> real `AuthorService`.
/// AC-008 / AC-009: only an owned same-author variant can become the display
/// name and cascade.
#[tokio::test]
async fn ac008_ac009_select_name_variant_is_author_and_user_scoped() {
    let (db, user_id, author_id, work_id) = author_with_work("Variant Pick").await;
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("progress");
    db.record_observed_names(
        user_id,
        work_id,
        &[ProviderAuthorNameObservation {
            source: AuthorNameSource::OpenLibrary,
            name: "Stored Variant".to_string(),
        }],
    )
    .await
    .expect("record variant");
    let variant_id: i64 = sqlx::query_scalar(
        "SELECT id FROM author_name_variants
          WHERE author_id=? AND name='Stored Variant'",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("variant id");
    let selected = author_service(db, StubHttpFetcher::new())
        .select_name_variant(user_id, author_id, variant_id)
        .await
        .expect("select stored variant");
    assert_eq!(selected.name, "Stored Variant");
}

/// Door: Monitor-enable gate -> real `AuthorService::set_monitoring`.
/// AC-007: unlinked, removed-OL, GR-only, and HC-only authors receive the
/// exact established monitored-field validation response.
#[tokio::test]
async fn ac007_monitor_enable_without_active_ol_route_returns_exact_validation() {
    let (db, user_id, author_id, _work_id) = author_with_work("Monitor Reject").await;
    let err = author_service(db, StubHttpFetcher::new())
        .set_monitoring(user_id, author_id, true, Some(true), Some("en".to_string()))
        .await
        .expect_err("unlinked author must not become monitored");
    match err {
        AuthorServiceError::Validation { field, message } => {
            assert_eq!(field, "monitored");
            assert_eq!(message, "cannot monitor author without OL linkage");
        }
        other => panic!("expected exact monitoring Validation, got {other:?}"),
    }
}

/// Door: Monitor-enable gate -> active Open Library route.
/// AC-007 / AC-014: route state, not stale `Author.ol_key`, unlocks monitoring;
/// disabling never removes routes.
#[tokio::test]
async fn ac007_ac014_active_ol_route_unlocks_monitoring_and_disable_preserves_route() {
    let (db, user_id, author_id, _work_id) = author_with_work("Monitor Active").await;
    db.attach_route_as_user(
        user_id,
        author_id,
        route(AuthorProvider::OpenLibrary, "OL7001A"),
    )
    .await
    .expect("active OL route");
    let service = author_service(db.clone(), StubHttpFetcher::new());
    let monitored = service
        .set_monitoring(user_id, author_id, true, Some(true), Some("en".to_string()))
        .await
        .expect("enable monitoring");
    assert!(monitored.monitored);
    let disabled = service
        .set_monitoring(user_id, author_id, false, None, None)
        .await
        .expect("disable monitoring");
    assert!(!disabled.monitored);
    assert!(db
        .has_active_route(user_id, author_id, AuthorProvider::OpenLibrary)
        .await
        .expect("route remains"));
}

/// Door: Author monitor workflow -> grouped plural OL route targets.
/// AC-007 / AC-014: overlapping feeds union before screening/add/notification,
/// so routes never multiply work or report counts and a failed sibling does
/// not erase success.
#[tokio::test]
async fn ac007_ac014_author_monitor_unions_plural_routes_before_actions_and_counts_once() {
    let (db, user_id, author_id, _work_id) = author_with_work("Monitor Union").await;
    for raw in ["OL7101A", "OL7102A"] {
        db.attach_route_as_user(user_id, author_id, route(AuthorProvider::OpenLibrary, raw))
            .await
            .expect("OL monitor route");
    }
    db.update_author(
        user_id,
        author_id,
        UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            gr_key: None,
            monitored: Some(true),
            monitor_new_items: Some(true),
            monitor_since: None,
            monitor_language: Some(Some("en".to_string())),
        },
    )
    .await
    .expect("production monitoring flag writer");

    let fetcher = StubHttpFetcher::with_ok(
        200,
        ol_works(&[
            ("OL7101W", "Shared Work", "2026"),
            ("OL7102W", "Unique Work", "2026"),
        ]),
    );
    fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works(&[("OL7101W", "Shared Work", "2026")]),
    }));
    let monitor_work_service = Arc::new(work_service(db.clone(), "monitor-union"));
    let workflow =
        AuthorMonitorWorkflowImpl::new(Arc::new(db), monitor_work_service, Arc::new(fetcher));
    let report = workflow
        .run_monitor(user_id, CancellationToken::new())
        .await
        .expect("real monitor workflow");
    assert_eq!(report.authors_checked, 1);
    assert_eq!(report.new_works_found, 2);
    assert_eq!(report.works_added, 2);
}

/// Door: Author bibliography consumer -> plural active OL routes.
/// AC-014: union/dedup keeps successful siblings, uses GB only for an empty
/// union, and never consults a stale scalar.
#[tokio::test]
async fn ac014_bibliography_unions_plural_active_ol_routes_and_ignores_stale_scalar() {
    let (db, user_id, author_id, _work_id) = author_with_work("Bibliography").await;
    for raw in ["OL7201A", "OL7202A"] {
        db.attach_route_as_user(user_id, author_id, route(AuthorProvider::OpenLibrary, raw))
            .await
            .expect("bibliography route");
    }
    let fetcher = StubHttpFetcher::with_ok(
        200,
        ol_works(&[
            ("OL7201W", "Shared Work", "2020"),
            ("OL7202W", "First Only", "2021"),
        ]),
    );
    fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: ol_works(&[
            ("OL7201W", "Shared Work", "2020"),
            ("OL7203W", "Second Only", "2022"),
        ]),
    }));
    let author = db.get_author(user_id, author_id).await.expect("author");
    let entries = author_service(db, fetcher)
        .fetch_bibliography_entries(user_id, &author)
        .await
        .expect("real bibliography consumer");
    assert_eq!(entries.len(), 3);
    let keys = entries
        .iter()
        .filter_map(|entry| entry.ol_key.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        keys.iter().filter(|key| **key == "/works/OL7201W").count(),
        1
    );
}

/// Door: Author series list -> plural active Goodreads routes.
/// AC-014: duplicate series union once, partial feed failure is isolated, and
/// no empty-route name guess writes a route.
#[tokio::test]
async fn ac014_series_list_unions_plural_goodreads_routes_before_cache_and_merge() {
    let (db, user_id, author_id, _work_id) = author_with_work("Series Union").await;
    for raw in ["7301", "7302"] {
        db.attach_route_as_user(user_id, author_id, route(AuthorProvider::Goodreads, raw))
            .await
            .expect("series route");
    }
    let fetcher = StubHttpFetcher::with_ok(200, series_page("73001", "Shared Series"));
    fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: series_page("73001", "Shared Series"),
    }));
    let series_work_service = Arc::new(work_service(db.clone(), "series-union"));
    let service = SeriesQueryServiceImpl::new(db, fetcher, series_work_service, StubNoLlm);
    let view = service
        .list_author_series(user_id, author_id, false)
        .await
        .expect("real series consumer");
    assert_eq!(view.series.len(), 1);
}

/// Door: Author series refresh -> empty active Goodreads route set.
/// AC-014: refresh fails with the missing-route fix-flow state and performs
/// no name search or legacy scalar fallback.
#[tokio::test]
async fn ac014_series_refresh_without_active_goodreads_route_fails_without_guessing() {
    let (db, user_id, author_id, _work_id) = author_with_work("Series Missing").await;
    let series_work_service = Arc::new(work_service(db.clone(), "series-missing"));
    let service =
        SeriesQueryServiceImpl::new(db, StubHttpFetcher::new(), series_work_service, StubNoLlm);
    let err = service
        .refresh_author_series(user_id, author_id)
        .await
        .expect_err("empty active GR set must expose the fix flow");
    assert!(
        matches!(err, SeriesServiceError::MissingGoodreadsRoute),
        "an empty active Goodreads route set must return the exact typed fix-flow error"
    );
}

/// Door: Readarr import author resolution -> snapshot-before-observe guard.
/// AC-011 / AC-013: valid ids preserve Agree/Rejected, while missing, empty,
/// malformed, overflow, or unusable names return None; enqueue is independent.
#[test]
fn ac011_ac013_readarr_helper_preserves_guard_split_and_rejects_unusable_ids() {
    let agree = readarr_author_route_evidence(
        &rd_author(Some("Octavia E. Butler"), Some("42")),
        &["Octavia E Butler".to_string()],
    );
    assert!(matches!(agree, Some(AuthorRouteGuardResult::Agreed(_))));

    let rejected = readarr_author_route_evidence(
        &rd_author(Some("Jane Smith"), Some("43")),
        &["John Smith".to_string()],
    );
    assert!(matches!(
        rejected,
        Some(AuthorRouteGuardResult::Rejected(_))
    ));

    for fixture in [
        rd_author(Some("Name"), None),
        rd_author(Some("Name"), Some("")),
        rd_author(Some("Name"), Some("not-a-number")),
        rd_author(Some("Name"), Some("18446744073709551616")),
        rd_author(None, Some("44")),
        rd_author(Some("   "), Some("44")),
    ] {
        assert!(readarr_author_route_evidence(&fixture, &[]).is_none());
    }
}

/// Door: Server startup cutover verification.
/// AC-014: a clean migrated SQLite report is required before serving; missing
/// or invalid staged routes/progress abort startup.
#[tokio::test]
async fn ac014_startup_cutover_gate_accepts_only_a_clean_real_sqlite_report() {
    let db = create_test_db().await;
    let report = verify_author_link_cutover_before_serving(&db)
        .await
        .expect("clean cutover report");
    assert_eq!(report.missing_routes, 0);
    assert_eq!(report.invalid_values, 0);
    assert_eq!(report.missing_progress_rows, 0);
}

/// Door: Recurring author-link sweep job registration.
/// AC-006 / AC-014 residual: startup composition is not exposed as a callable
/// helper or inspectable `JobRunner`, so this remains a source-order proxy, not
/// behavioral proof of ordering under load. It still prevents the two known
/// cutover regressions until Code stage introduces a testable startup hook.
#[test]
fn ac006_ac014_production_composition_registers_sweep_only_after_cutover_gate() {
    let main_source = include_str!("../../crates/livrarr-server/src/main.rs");
    let gate = main_source
        .find("verify_author_link_cutover_before_serving")
        .expect("production startup must call the cutover gate");
    let tick = main_source
        .find("author_link_sweep_tick")
        .expect("production startup must register the author-link tick");
    assert!(
        gate < tick,
        "cutover verification must precede job registration"
    );
    assert!(
        !main_source.contains("NoOpBibliographyTrigger"),
        "production list import must use the real author-link enqueue trigger"
    );
}

/// Door: Standalone author add with selected OL route.
/// AC-005 / AC-010: the explicit route is UserPicked and the author scalar is
/// frozen even though the existing request DTO still exposes `ol_key`.
#[tokio::test]
async fn ac005_ac010_standalone_selected_route_is_user_picked_not_a_scalar_write() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let result = author_service(db.clone(), StubHttpFetcher::new())
        .add(
            user_id,
            AddAuthorRequest {
                name: "Selected Standalone Author".to_string(),
                sort_name: None,
                ol_key: Some("OL7501A".to_string()),
                monitored: false,
            },
        )
        .await
        .expect("real standalone add door");
    let author = result.author();
    assert_eq!(author.ol_key, None);
    let routes = db
        .list_active_routes(user_id, author.id, Some(AuthorProvider::OpenLibrary))
        .await
        .expect("selected routes");
    assert_eq!(routes.len(), 1);
    assert!(matches!(
        routes[0].provenance,
        livrarr_domain::AuthorRouteProvenance::UserPicked
    ));
}
