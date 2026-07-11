//! Anchor persistence on the identity road: a verified resolve must persist
//! EVERY non-null anchor the resolver returns — including when `ol_key` is
//! absent from the result.
//!
//! `retry_all_incomplete` routes Pending works through `settle_identity`,
//! whose verified `Resolved` branch calls `merge_missing_anchors`, which
//! writes each non-null anchor type (OL/GR/HC/ISBN/ASIN) independently.
//! Nothing in that chain may key the writes on `ol_key`'s presence.
//!
//! The rest of the registered suite (test_s6_retry_all_incomplete) exercises
//! this road only with resolver results that DO carry `ol_key`; this test pins
//! the ol_key-absent case. (Origin: Finding E2 — a pre-settle_identity code
//! path persisted only `ol_key` and silently dropped the rest.)

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::AnchorType;
use livrarr_domain::services::{WorkIdentityRepository, WorkService};
use livrarr_domain::{normalize_for_matching, IdentityStatus, MetadataProvider};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::{LiveEnglishIdentityResolver, ResolverConfig};
use livrarr_metadata::work_service::WorkServiceImpl;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const TITLE: &str = "E2 Anchor Persistence Title";
const AUTHOR: &str = "E2 Anchor Persistence Author";
const GR_KEY: &str = "9999888";
const ISBN: &str = "9780593099322";

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

/// Resolver where Hardcover is the sole contributor and its payload carries
/// gr_key + isbn_13 but NO ol_key; OpenLibrary returns NotFound.
fn build_resolver() -> LiveEnglishIdentityResolver {
    let hc_stub = StubProviderClient::new(
        MetadataProvider::Hardcover,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some(TITLE.to_string()),
            author_name: Some(AUTHOR.to_string()),
            ol_key: None,
            gr_key: Some(GR_KEY.to_string()),
            isbn_13: Some(ISBN.to_string()),
            hc_key: None,
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let ol_stub = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);

    let clients: HashMap<MetadataProvider, ProviderClient> = [
        (MetadataProvider::Hardcover, ProviderClient::Stub(hc_stub)),
        (MetadataProvider::OpenLibrary, ProviderClient::Stub(ol_stub)),
    ]
    .into_iter()
    .collect();

    LiveEnglishIdentityResolver {
        clients,
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            ..ResolverConfig::default()
        },
    }
}

fn build_service(db: SqliteDb, resolver: LiveEnglishIdentityResolver) -> TestWorkService {
    let data_dir = std::env::temp_dir().join(format!("livrarr-verify-e2-{}", std::process::id()));

    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        data_dir,
    )
    .with_resolver(Arc::new(resolver))
}

/// A Pending work swept by `retry_all_incomplete` whose resolver result has
/// gr_key + isbn_13 but no ol_key must end Confirmed with BOTH anchors
/// persisted (ledger and works.* sync) and no OL anchor invented.
#[tokio::test]
async fn test_verify_e2_settle_identity_persists_all_anchors_without_ol_key() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // Pending work, zero anchors.
    let req = CreateWorkDbRequest {
        user_id,
        title: TITLE.to_string(),
        author_name: AUTHOR.to_string(),
        normalized_title: normalize_for_matching(TITLE),
        normalized_author: normalize_for_matching(AUTHOR),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: false,
        ..Default::default()
    };
    let (work, created) = db.create_work(req).await.expect("create_work");
    assert!(created, "work fixture should be created");
    db.set_identity_status(user_id, work.id, IdentityStatus::Pending)
        .await
        .expect("seed Pending status");

    let anchors_before = db.list_anchors(work.id).await.expect("list_anchors before");
    assert!(anchors_before.is_empty(), "baseline: zero anchors");

    let svc = build_service(db.clone(), build_resolver());
    svc.retry_all_incomplete(user_id)
        .await
        .expect("retry_all_incomplete");

    let after = db.get_work(user_id, work.id).await.expect("get_work after");
    let anchors_after = db.list_anchors(work.id).await.expect("list_anchors after");

    assert_eq!(
        after.identity_status,
        IdentityStatus::Confirmed,
        "gr_key is a work anchor, so a verified resolve must confirm identity"
    );

    assert!(
        anchors_after
            .iter()
            .any(|a| a.anchor_type.as_str() == AnchorType::GR_WORK),
        "gr_key anchor must persist even though the resolver returned no ol_key"
    );
    assert!(
        anchors_after
            .iter()
            .any(|a| a.anchor_type.as_str() == AnchorType::ISBN_13),
        "isbn_13 anchor must persist even though the resolver returned no ol_key"
    );
    assert!(
        !anchors_after
            .iter()
            .any(|a| a.anchor_type.as_str() == AnchorType::OL_WORK),
        "no ol_key anchor may be invented: the resolver returned ol_key=None"
    );

    assert_eq!(
        after.gr_key.as_deref(),
        Some(GR_KEY),
        "confirmed gr_key anchor must sync to works.gr_key"
    );
}
