//! Behavioral red gate for issue #175: concurrent CSV list-import rows sharing
//! one author must converge on a single author row (spec
//! `spec-bugfix-175-duplicate-authors.md` AC-001, REQ-001).
//!
//! The production path under test is `ListService::preview` ->
//! `ListService::confirm`, whose row pipeline runs up to 5 rows concurrently
//! (`buffer_unordered(5)`), each calling `WorkService::add` ->
//! `find_or_create_author`. All services are real; the DB is the real
//! in-memory `SqliteDb` writer. No injected state.

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::AuthorDb;
use livrarr_db::WorkDb;
use livrarr_domain::services::ListService;
use livrarr_domain::UserId;
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::ProviderClient;
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::list_service::{ListServiceImpl, NoOpBibliographyTrigger};
use livrarr_metadata::work_service::{StubNoEnrichment, WorkServiceImpl};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const SHARED_AUTHOR: &str = "Anne Rice";
const ROW_COUNT: usize = 12;

type TestWorkService = WorkServiceImpl<SqliteDb, StubNoEnrichment, StubHttpFetcher>;

type TestListService =
    ListServiceImpl<SqliteDb, TestWorkService, StubHttpFetcher, NoOpBibliographyTrigger>;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-dup-authors-test-{}", std::process::id()))
}

/// Resolver with no provider clients: every row misses identity resolution and
/// lands on the Pending create path, which is where `find_or_create_author`
/// races. Zero network either way.
fn empty_resolver() -> LiveEnglishIdentityResolver {
    LiveEnglishIdentityResolver {
        clients: HashMap::<_, ProviderClient>::new(),
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            ..ResolverConfig::default()
        },
    }
}

async fn make_service() -> (TestListService, UserId, SqliteDb) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let http = StubHttpFetcher::new();
    let work_service =
        WorkServiceImpl::new(db.clone(), StubNoEnrichment, http.clone(), test_data_dir())
            .with_resolver(Arc::new(empty_resolver()));

    (
        ListServiceImpl::new(db.clone(), work_service, http, NoOpBibliographyTrigger),
        user_id,
        db,
    )
}

/// Goodreads-format CSV: ROW_COUNT title-author-only rows, distinct titles,
/// one shared author. Distinct titles keep work-level dedup out of the way so
/// every row reaches author resolution.
fn csv_many_rows_one_author() -> Vec<u8> {
    let mut csv = String::from(
        "Book Id,Title,Author,ISBN13,Original Publication Year,Exclusive Shelf\n",
    );
    for i in 0..ROW_COUNT {
        csv.push_str(&format!(",Vampire Chronicle Vol {i},{SHARED_AUTHOR},,1990,to-read\n"));
    }
    csv.into_bytes()
}

/// AC-001: N concurrent same-author rows through the real confirm door leave
/// exactly ONE author row, with every created work attached to it.
#[tokio::test]
async fn test_dup_authors_ac_001_concurrent_import_rows_converge_on_one_author_row() {
    let (svc, user_id, db) = make_service().await;

    let preview = svc
        .preview(user_id, csv_many_rows_one_author())
        .await
        .expect("preview Goodreads CSV");
    let row_indices: Vec<usize> = (0..ROW_COUNT).collect();
    let result = svc
        .confirm(user_id, &preview.preview_id, None, &row_indices, None)
        .await
        .expect("confirm all rows");

    let added = result
        .results
        .iter()
        .filter(|r| r.status == "added")
        .count();
    assert_eq!(added, ROW_COUNT, "every distinct-title row should be added");

    let authors = db.list_authors(user_id).await.expect("list authors");
    let matching: Vec<_> = authors
        .iter()
        .filter(|a| a.name == SHARED_AUTHOR)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "AC-001: exactly one author row for {SHARED_AUTHOR}; got {} rows (ids: {:?})",
        matching.len(),
        matching.iter().map(|a| a.id).collect::<Vec<_>>()
    );

    let author_id = matching[0].id;
    let works = db.list_works(user_id).await.expect("list works");
    let imported: Vec<_> = works
        .iter()
        .filter(|w| w.title.starts_with("Vampire Chronicle Vol"))
        .collect();
    assert_eq!(
        imported.len(),
        ROW_COUNT,
        "AC-001: every imported work must persist; got {:?}",
        imported.iter().map(|w| &w.title).collect::<Vec<_>>()
    );
    for work in &imported {
        assert_eq!(
            work.author_name, SHARED_AUTHOR,
            "AC-001: work '{}' must keep the shared author's display name",
            work.title
        );
        assert_eq!(
            work.author_id,
            Some(author_id),
            "AC-001: work '{}' must attach to the single author row",
            work.title
        );
    }
}
