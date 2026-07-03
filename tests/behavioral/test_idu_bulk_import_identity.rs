#![allow(dead_code, unused_imports)]

//! Behavioral red gate for bulk list import identity resolution.
//!
//! The production path under test is `ListService::preview` -> `ListService::confirm`.
//! These tests wire a real `WorkServiceImpl` with a stubbed
//! `LiveEnglishIdentityResolver`, so a resolvable Goodreads row can only become
//! `IdentityStatus::Confirmed` if confirm routes through `WorkService::resolve_identity`
//! synchronously before calling `WorkService::add`.

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::WorkDb;
use livrarr_domain::services::ListService;
use livrarr_domain::{IdentityStatus, MetadataProvider, UserId, Work};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::list_service::{ListServiceImpl, NoOpBibliographyTrigger};
use livrarr_metadata::work_service::{
    StubNoEnrichment, StubNoLlm, StubTagService, WorkServiceImpl,
};
use livrarr_metadata::{DefaultMergeEngine, PriorityModel};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const DUNE_TITLE: &str = "Dune";
const DUNE_AUTHOR: &str = "Frank Herbert";
const DUNE_GR_KEY: &str = "234225";
const DUNE_ROW_ISBN: &str = "9780441013593";
const RESOLVED_OL_KEY: &str = "OL27448W";
const RESOLVED_ISBN: &str = "9780441172719";

type TestWorkService = WorkServiceImpl<
    SqliteDb,
    StubNoEnrichment,
    StubHttpFetcher,
    StubNoLlm,
    DefaultMergeEngine,
    StubTagService,
>;

type TestListService =
    ListServiceImpl<SqliteDb, TestWorkService, StubHttpFetcher, NoOpBibliographyTrigger>;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-idu-test-{}", std::process::id()))
}

fn success(detail: NormalizedWorkDetail) -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::Success(Box::new(detail))
}

fn resolver_with_stubs(stubs: Vec<StubProviderClient>) -> LiveEnglishIdentityResolver {
    let clients = stubs
        .into_iter()
        .map(|s| (s.provider, ProviderClient::Stub(s)))
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

async fn make_service_with_resolver(
    resolver: LiveEnglishIdentityResolver,
) -> (TestListService, UserId, SqliteDb) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let http = StubHttpFetcher::new();
    let work_service = WorkServiceImpl::new_with_all(
        db.clone(),
        StubNoEnrichment,
        http.clone(),
        livrarr_http::HttpClient::builder()
            .build()
            .expect("test HttpClient"),
        StubNoLlm,
        test_data_dir(),
        DefaultMergeEngine::new(PriorityModel::english()),
        Arc::new(StubTagService),
    )
    .with_resolver(Arc::new(resolver));

    (
        ListServiceImpl::new(db.clone(), work_service, http, NoOpBibliographyTrigger),
        user_id,
        db,
    )
}

fn goodreads_csv_with_resolvable_row() -> Vec<u8> {
    format!(
        "Book Id,Title,Author,ISBN13,Original Publication Year,Exclusive Shelf\n\
         {DUNE_GR_KEY},{DUNE_TITLE},{DUNE_AUTHOR},=\"{DUNE_ROW_ISBN}\",1965,to-read\n"
    )
    .into_bytes()
}

fn goodreads_csv_title_author_only() -> Vec<u8> {
    b"Book Id,Title,Author,ISBN13,Original Publication Year,Exclusive Shelf\n\
      ,The Left Hand of Darkness,Ursula K. Le Guin,,1969,to-read\n"
        .to_vec()
}

async fn confirm_first_row(svc: &TestListService, user_id: UserId, csv: Vec<u8>) {
    let preview = svc
        .preview(user_id, csv)
        .await
        .expect("preview Goodreads CSV");
    let result = svc
        .confirm(user_id, &preview.preview_id, None, &[0], None)
        .await
        .expect("confirm selected row");

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].row_index, 0);
    assert_eq!(result.results[0].status, "added");
}

async fn persisted_work_by_title(db: &SqliteDb, user_id: UserId, title: &str) -> Work {
    let listed = db.list_works(user_id).await.expect("list works");
    let work_id = listed
        .iter()
        .find(|w| w.title == title)
        .map(|w| w.id)
        .expect("confirmed row should create work");

    db.get_work(user_id, work_id)
        .await
        .expect("fetch persisted work by id")
}

fn resolving_openlibrary_stub() -> StubProviderClient {
    StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        success(NormalizedWorkDetail {
            ol_key: Some(RESOLVED_OL_KEY.to_string()),
            isbn_13: Some(RESOLVED_ISBN.to_string()),
            title: Some(DUNE_TITLE.to_string()),
            author_name: Some(DUNE_AUTHOR.to_string()),
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        }),
    )
}

/// AC-IDU-1: A bulk Goodreads row carrying resolvable anchors is confirmed
/// immediately by the same shared identity path as interactive Add Work.
#[tokio::test]
async fn test_idu_bulk_import_identity_ac_1_goodreads_confirm_persists_confirmed_immediately() {
    let ol = resolving_openlibrary_stub();
    let hc = StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound);
    let resolver = resolver_with_stubs(vec![ol.clone(), hc]);
    let (svc, user_id, db) = make_service_with_resolver(resolver).await;

    confirm_first_row(&svc, user_id, goodreads_csv_with_resolvable_row()).await;
    let work = persisted_work_by_title(&db, user_id, DUNE_TITLE).await;

    assert_eq!(
        ol.call_count(),
        1,
        "bulk confirm should synchronously call the wired identity resolver's provider fan-out"
    );
    assert_eq!(
        work.identity_status,
        IdentityStatus::Confirmed,
        "AC-IDU-1: resolvable bulk import rows must be Confirmed immediately after confirm() returns"
    );
}

/// AC-IDU-2: A title/author-only miss never blocks import. It still creates the
/// work and leaves identity pending for later convergence.
#[tokio::test]
async fn test_idu_bulk_import_identity_ac_2_title_author_only_miss_adds_pending_work() {
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let hc = StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound);
    let resolver = resolver_with_stubs(vec![ol, hc]);
    let (svc, user_id, db) = make_service_with_resolver(resolver).await;

    confirm_first_row(&svc, user_id, goodreads_csv_title_author_only()).await;
    let work = persisted_work_by_title(&db, user_id, "The Left Hand of Darkness").await;

    assert_eq!(
        work.identity_status,
        IdentityStatus::Pending,
        "AC-IDU-2: unresolved title/author-only imports remain Pending, not failed"
    );
}

/// AC-IDU-3: The confirmed status is observable as soon as `confirm()` returns;
/// this test intentionally does not invoke any async resolver/enrichment job tick.
#[tokio::test]
async fn test_idu_bulk_import_identity_ac_3_confirmed_status_requires_no_background_job() {
    let ol = resolving_openlibrary_stub();
    let hc = StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound);
    let resolver = resolver_with_stubs(vec![ol, hc]);
    let (svc, user_id, db) = make_service_with_resolver(resolver).await;

    confirm_first_row(&svc, user_id, goodreads_csv_with_resolvable_row()).await;
    let work = persisted_work_by_title(&db, user_id, DUNE_TITLE).await;

    assert_eq!(
        work.identity_status,
        IdentityStatus::Confirmed,
        "AC-IDU-3: confirm() itself must persist Confirmed; no background tick is part of this test"
    );
}

/// AC-IDU-4: The persisted anchors come from the resolver's canonical identity,
/// not a verbatim copy of the raw Goodreads row.
#[tokio::test]
async fn test_idu_bulk_import_identity_ac_4_persists_resolver_canonical_anchors() {
    let ol = resolving_openlibrary_stub();
    let hc = StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound);
    let resolver = resolver_with_stubs(vec![ol, hc]);
    let (svc, user_id, db) = make_service_with_resolver(resolver).await;

    confirm_first_row(&svc, user_id, goodreads_csv_with_resolvable_row()).await;
    let work = persisted_work_by_title(&db, user_id, DUNE_TITLE).await;

    assert_eq!(
        work.ol_key.as_deref(),
        Some(RESOLVED_OL_KEY),
        "AC-IDU-4: canonical work anchor should be the resolver-returned OL work key"
    );
    assert_eq!(
        work.isbn_13.as_deref(),
        Some(RESOLVED_ISBN),
        "AC-IDU-4: bridge identifiers should converge to the resolver-returned canonical payload"
    );
    assert_ne!(
        work.isbn_13.as_deref(),
        Some(DUNE_ROW_ISBN),
        "AC-IDU-4: this must not be a verbatim copy of the raw row ISBN"
    );
}
