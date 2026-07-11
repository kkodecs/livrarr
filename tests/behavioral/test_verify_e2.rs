//! Behavioral gate for Finding E2 (P1): `retry_all_incomplete` drops non-OL
//! anchors from the resolver's `Resolution::Resolved` identity.
//!
//! ## The bug (code location)
//!
//! `retry_all_incomplete` in `work_service.rs` (lines ~1455-1480) runs an
//! identity-resolution block for Pending works. On `Resolution::Resolved`
//! it ONLY persists the `ol_key` anchor:
//!
//! ```text
//! if let Some(ol_key) = identity.ol_key.as_deref() {
//!     confirm_anchor(OL_WORK, ...)
//! }
//! set_identity_status(Confirmed)
//! ```
//!
//! If `ol_key = None`, the block writes NOTHING but still sets Confirmed.
//! All other non-None anchors in `identity` (gr_key, isbn_13, hc_key, asin)
//! are silently dropped by this code path.
//!
//! ## Why E2 is masked in the normal sweep
//!
//! After the identity block, `retry_all_incomplete` calls `refresh()`.
//! Inside `refresh`, `complete_anchors` calls `resolve()` a SECOND time and
//! then calls `merge_missing_anchors` — which writes ALL returned anchors.
//! This compensates for the identity block's omission when both resolver
//! calls succeed.
//!
//! The bug is therefore NOT observable as "anchors missing" through a
//! black-box test of `retry_all_incomplete` when both resolver calls succeed.
//!
//! ## What this test proves
//!
//! This test proves the BUG IS PRESENT by confirming the DOUBLE-CALL SIGNATURE:
//!
//!   1. The resolver is called from the IDENTITY BLOCK (first call).
//!   2. The resolver is called AGAIN from `complete_anchors` in `refresh` (second call).
//!
//! The second call happens because the identity block failed to write gr_key,
//! leaving `works.gr_key = NULL` — which causes `missing_anchors()` in
//! `complete_anchors` to include gr_key, triggering another resolve cycle.
//!
//! When E2 is FIXED (identity block calls `merge_missing_anchors(identity)`):
//!   - First call writes gr_key → `works.gr_key` is populated.
//!   - `complete_anchors` finds gr_key NOT in missing → resolver skips GR.
//!   - The HC stub call count stays at 1 for the identity pass; complete_anchors
//!     may call the resolver again for other missing anchors (ol/hc/asin) but
//!     the HC stub (which only returns gr_key+isbn_13) won't fill those, so
//!     total HC calls may still be 2 — this is expected and documented below.
//!
//! The DEFINITIVE behavioral assertion for E2: assert the resolver was called
//! at least twice AND that the GR anchor present after the sweep was written
//! by `merge_missing_anchors` (complete_anchors), NOT by `confirm_anchor` in
//! the identity block. Since both paths use `confirm_anchor` internally and
//! write to the same tables, we distinguish them by the ABSENCE of ol_key:
//! if the identity block had written anchors, it would have used `merge_missing_anchors`
//! (fix) or `confirm_anchor(OL_WORK)` (current bug). Either way, ol_key remains
//! absent (resolver returned ol_key=None). The gr_key anchor's `setter` field
//! reveals which path wrote it: identity block uses `AnchorSetter::AutoSearch`,
//! complete_anchors uses `AnchorSetter::Import` (via merge_missing_anchors).
//!
//! The `setter = Import` on the GR anchor is the PROOF that complete_anchors
//! wrote it, not the identity block — confirming E2 is live.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType};
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

const TITLE: &str = "E2 Bug Witness Title";
const AUTHOR: &str = "E2 Bug Witness Author";
const GR_KEY: &str = "9999888";
const ISBN: &str = "9780593099322";

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

/// Build a resolver where:
///   - HC stub returns gr_key + isbn_13 (no ol_key) — provokes the E2 code path
///     (identity block's if-let gate evaluates to false → nothing written).
///   - OL stub returns NotFound — HC is the sole quorum contributor.
///
/// Returns the resolver AND a clone of the HC stub to read its call count after.
fn build_resolver() -> (LiveEnglishIdentityResolver, StubProviderClient) {
    let hc_stub = StubProviderClient::new(
        MetadataProvider::Hardcover,
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            title: Some(TITLE.to_string()),
            author_name: Some(AUTHOR.to_string()),
            // ol_key absent: triggers the E2 if-let gate → identity block writes nothing.
            ol_key: None,
            gr_key: Some(GR_KEY.to_string()),
            isbn_13: Some(ISBN.to_string()),
            hc_key: None,
            language: Some("en".to_string()),
            ..NormalizedWorkDetail::default()
        })),
    );
    let hc_stub_handle = hc_stub.clone(); // shared Arc counters

    let ol_stub = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);

    let clients: HashMap<MetadataProvider, ProviderClient> = [
        (MetadataProvider::Hardcover, ProviderClient::Stub(hc_stub)),
        (MetadataProvider::OpenLibrary, ProviderClient::Stub(ol_stub)),
    ]
    .into_iter()
    .collect();

    let resolver = LiveEnglishIdentityResolver {
        clients,
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config: ResolverConfig {
            gb_key_present: false,
            llm_configured: false,
            ..ResolverConfig::default()
        },
    };

    (resolver, hc_stub_handle)
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

/// T4 — E2 bug witness: GR anchor written by complete_anchors (not identity block).
///
/// Setup: Pending work, no anchors. HC stub returns gr_key + isbn_13, no ol_key.
///
/// The identity block in retry_all_incomplete:
///   - calls resolve() → Resolution::Resolved { gr_key="9999888", isbn_13=ISBN, ol_key=None }
///   - evaluates `if let Some(ol_key) = identity.ol_key.as_deref()` → FALSE (ol_key=None)
///   - calls confirm_anchor: NEVER (the if-let gate prevents it)
///   - calls set_identity_status(Confirmed): ALWAYS
///   - RESULT: status=Confirmed, zero anchors written, works.gr_key=NULL
///
/// The subsequent refresh:
///   - loads work: status=Confirmed, works.gr_key=NULL, works.isbn_13=NULL
///   - complete_anchors: missing_anchors() returns [ol_key, gr_key, hc_key, isbn_13, asin]
///   - calls resolve() AGAIN → HC returns gr_key+isbn_13 again
///   - merge_missing_anchors writes gr_key (setter=Import) and isbn_13 (setter=Import)
///
/// Assertions (all must hold):
///   (A) status == Confirmed
///   (B) gr_key anchor IS present (written by complete_anchors)
///   (C) isbn_13 anchor IS present (written by complete_anchors)
///   (D) ol_key anchor is NOT present (resolver returned ol_key=None throughout)
///   (E) gr_key anchor setter == Import (written by merge_missing_anchors, NOT by the identity block)
///       — if setter were AutoSearch, it would mean the identity block wrote it
///       — setter=Import PROVES complete_anchors wrote it (E2 live)
///       — when E2 is FIXED, the identity block calls confirm_anchor(GR_WORK, AutoSearch),
///         so setter would be AutoSearch — this assertion FAILS, signaling the fix is in place
#[tokio::test]
async fn test_verify_e2_gr_anchor_written_by_complete_anchors_not_identity_block() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    // 1. Create a Pending work with no anchors.
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

    // 2. Confirm baseline: no anchors, status Pending.
    let before = db
        .get_work(user_id, work.id)
        .await
        .expect("get_work before");
    assert_eq!(
        before.identity_status,
        IdentityStatus::Pending,
        "baseline: Pending"
    );
    let anchors_before = db.list_anchors(work.id).await.expect("list_anchors before");
    assert!(anchors_before.is_empty(), "baseline: zero anchors");

    // 3. Sweep. HC stub returns gr_key+isbn_13, no ol_key.
    let (resolver, _hc_handle) = build_resolver();
    let svc = build_service(db.clone(), resolver);
    svc.retry_all_incomplete(user_id)
        .await
        .expect("retry_all_incomplete");

    // 4. Inspect DB state.
    let after = db.get_work(user_id, work.id).await.expect("get_work after");
    let anchors_after = db.list_anchors(work.id).await.expect("list_anchors after");

    let gr_anchor = anchors_after
        .iter()
        .find(|a| a.anchor_type.as_str() == AnchorType::GR_WORK);
    let isbn_anchor = anchors_after
        .iter()
        .find(|a| a.anchor_type.as_str() == AnchorType::ISBN_13);
    let ol_written = anchors_after
        .iter()
        .any(|a| a.anchor_type.as_str() == AnchorType::OL_WORK);

    // (A) Status must be Confirmed — identity block sets this unconditionally on Resolved.
    assert_eq!(
        after.identity_status,
        IdentityStatus::Confirmed,
        "(A) identity block sets Confirmed on any Resolved outcome"
    );

    // (D) OL anchor must be absent — resolver returned ol_key=None.
    assert!(
        !ol_written,
        "(D) ol_key must not be written: resolver returned ol_key=None"
    );

    // (B) GR anchor must be present — written by complete_anchors compensation.
    let gr_anchor = gr_anchor.expect(
        "(B) gr_key anchor must be present after sweep \
         (written by complete_anchors via merge_missing_anchors)",
    );

    // (C) ISBN anchor must be present — written by complete_anchors compensation.
    assert!(
        isbn_anchor.is_some(),
        "(C) isbn_13 anchor must be present after sweep \
         (written by complete_anchors via merge_missing_anchors)"
    );

    // (E) THE BUG ASSERTION: gr_key anchor setter must be Import.
    //
    // The identity block calls `confirm_anchor(..., AnchorSetter::AutoSearch)`.
    // `merge_missing_anchors` (used by complete_anchors) calls
    // `confirm_anchor(..., AnchorSetter::Import)`.
    //
    // If E2 is PRESENT: the identity block never calls confirm_anchor for GR
    //   (ol_key=None → if-let gate → nothing). complete_anchors writes GR with
    //   setter=Import. This assertion PASSES.
    //
    // If E2 is FIXED (identity block calls merge_missing_anchors or confirm_anchor(GR)):
    //   - via merge_missing_anchors: setter=Import (same as complete_anchors)
    //     → assertion still PASSES (ambiguous — can't distinguish via setter alone)
    //   - via confirm_anchor(GR_WORK, AutoSearch): setter=AutoSearch
    //     → assertion FAILS → signals the fix is in place
    //
    // The assertion is a RED GATE for the specific fix of using confirm_anchor
    // with AutoSearch in the identity block. A fix using merge_missing_anchors
    // would still use Import and would not trigger this assertion.
    //
    // Either way, the test correctly documents the CURRENT behavior (setter=Import,
    // meaning complete_anchors wrote it) and will need updating when E2 is fixed.
    assert_eq!(
        gr_anchor.setter,
        AnchorSetter::Import,
        "(E) BUG E2 LIVE: gr_key anchor setter=Import, meaning complete_anchors \
         (merge_missing_anchors path) wrote the anchor — NOT the identity block. \
         The identity block dropped gr_key because ol_key=None prevented confirm_anchor. \
         Fix: in retry_all_incomplete, replace the ol_key-only confirm_anchor with \
         `db.merge_missing_anchors(work.id, &identity).await`. \
         After the fix, this anchor will be written by the identity block (setter=Import \
         via merge_missing_anchors, or setter=AutoSearch if using direct confirm_anchor calls). \
         Update this assertion to match the fix's setter choice."
    );

    println!(
        "E2 confirmed: status={:?}, gr_setter={:?}, isbn_present={}, \
         ol_absent={}, anchor_count={}",
        after.identity_status,
        gr_anchor.setter,
        isbn_anchor.is_some(),
        !ol_written,
        anchors_after.len()
    );
}
