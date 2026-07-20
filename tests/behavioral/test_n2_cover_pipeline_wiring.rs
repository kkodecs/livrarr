//! V1/AC-5: proves the door->road wiring end-to-end, not just by reading
//! code. `WorkServiceImpl::refresh()` (the same `run_unified_enrichment`
//! chokepoint add and background-retry also call — traced at
//! `crates/livrarr-metadata/src/work_service.rs`: `ensure_identity_and_enrichment`
//! line ~2802 and `convergence_service.rs` line ~138 both call it directly)
//! runs a real merge result through the cover-write gate and materialize,
//! landing a real file on disk and a real DB row — no `.candidate.*` files,
//! no literal `"add"` source, no 0x0 dims.

use std::collections::HashMap;

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate};
use livrarr_domain::identity::CandidateId;
use livrarr_domain::services::{
    EnrichmentMode, EnrichmentResult, EnrichmentWorkflow, EnrichmentWorkflowError, RefreshSurface,
    SourceProviderData, WorkService,
};
use livrarr_domain::{
    normalize_for_matching, CoverMediaType, CoverResolution, CoverTrust, EnrichmentStatus,
    RequestPriority, UserId, UserRole, Work, WorkId,
};
use livrarr_metadata::work_service::WorkServiceImpl;

/// Returns a canned merge result carrying a real cover resolution — stands
/// in for a real provider payload having won the merge, so the test proves
/// what happens to that resolution AFTER the merge, not the merge itself
/// (the merge engine's own priority logic is covered elsewhere).
#[derive(Clone)]
struct CoverResolvingWorkflow;

impl EnrichmentWorkflow for CoverResolvingWorkflow {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<CandidateId>,
        _priority: RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("goodreads".into()),
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: Some(CoverResolution {
                url: "https://i.gr-assets.com/books/won-the-merge.jpg".into(),
                source: "goodreads".into(),
                trust: CoverTrust::Validated,
                media_type: CoverMediaType::Ebook,
            }),
            audiobook_cover_resolution: None,
            identity_not_found: false,
            changed: true,
            attempted: true,
        })
    }

    async fn reset_for_manual_refresh(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<(), EnrichmentWorkflowError> {
        Ok(())
    }

    async fn inject_source_data(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _data: SourceProviderData,
    ) {
    }
}

fn fake_jpeg(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::new(width, height);
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Jpeg,
        )
        .expect("encode test jpeg");
    buf
}

#[tokio::test]
async fn v1_refresh_reaches_the_cover_write_gate_and_lands_a_provenanced_file() {
    let db = create_test_db().await;
    let user_id = db
        .create_user(livrarr_db::CreateUserDbRequest {
            username: "n2-wiring-test".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "key".into(),
        })
        .await
        .unwrap()
        .id;

    let (work, _created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Pipeline Wiring Test".into(),
            author_name: "Wiring Author".into(),
            normalized_title: normalize_for_matching("Pipeline Wiring Test"),
            normalized_author: normalize_for_matching("Wiring Author"),
            language: Some("en".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // An anchorless seed work defaults to IdentityStatus::Pending, which
    // refresh()'s identity gate would block before ever reaching
    // run_unified_enrichment — irrelevant to what this test proves, so settle
    // it the same way a resolved identity would.
    db.set_identity_status(user_id, work.id, livrarr_domain::IdentityStatus::Confirmed)
        .await
        .unwrap();

    // Seed an incumbent BELOW the floor (a real rescue scenario end to end),
    // stamped with the exact "add" placeholder S3 diagnoses.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/thin.jpg"),
        "add",
        CoverTrust::Unvalidated,
        200,
        300,
    )
    .await
    .unwrap();

    let data_dir = tempfile::tempdir().unwrap();
    let http = StubHttpFetcher::with_ok(200, fake_jpeg(600, 900));

    let svc = WorkServiceImpl::new(
        db.clone(),
        CoverResolvingWorkflow,
        http,
        data_dir.path().to_path_buf(),
    );

    // Note: this test's stub EnrichmentWorkflow is a thin fake — unlike the
    // real EnrichmentServiceImpl, it never calls apply_enrichment_merge, so
    // it does NOT persist enrichment_status itself (that's the real merge
    // engine's job, covered elsewhere). This test isolates and proves ONLY
    // what run_unified_enrichment does with the EnrichmentResult it gets
    // back — specifically, what happens to cover_resolution.
    let _result = svc
        .refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh should succeed");

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://i.gr-assets.com/books/won-the-merge.jpg"),
        "the merge-picked cover must reach the row"
    );
    assert_eq!(
        after.cover_source.as_deref(),
        Some("goodreads"),
        "must stamp the real provider, never the literal 'add' placeholder \
         the row started with (V3 root cause, fixed)"
    );
    assert_eq!(after.cover_trust, CoverTrust::Validated);
    assert_eq!(
        (after.cover_width, after.cover_height),
        (600, 900),
        "must stamp the MEASURED dims of the bytes actually downloaded"
    );

    let covers_dir = data_dir.path().join("covers").join(user_id.to_string());
    let final_path = covers_dir.join(format!("{}.jpg", work.id));
    assert!(final_path.exists());
    assert_eq!(
        tokio::fs::read(&final_path).await.unwrap(),
        fake_jpeg(600, 900)
    );
    assert!(!covers_dir
        .join(format!("{}.candidate.tmp", work.id))
        .exists());
    assert!(!covers_dir
        .join(format!("{}.candidate.meta.json", work.id))
        .exists());
}
