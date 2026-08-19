//! Behavioral pins for the identity/cover bug fix.
//!
//! Contract: `docs/design-subtitle-matching.md` (r3). Two deletions and their
//! consequences:
//!
//! * **C1** — a one-sided subtitle no longer needs a hard identifier to agree.
//!   A subtitle is edition-level; the work record carries the bare title.
//! * **C2** — the old Goodreads title-similarity cover gate remains gone. Round
//!   15 now excludes Goodreads at cover-candidate assembly instead: payload
//!   parsing and identity trust survive, but Goodreads art cannot be selected.
//!
//! Every test here drives a real production entry point: the payload-trust
//! function the resolver calls, and the real `MergeEngine::merge` chokepoint.

use std::collections::HashMap;

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType, CapturedIdentity};
use livrarr_domain::services::{
    EnrichmentWorkflow, EnrichmentWorkflowError, IdentityPreviewOutcome, IdentityPreviewRecord,
    SiblingAction, WorkIdentityRepository, WorkService,
};
use livrarr_domain::{
    normalize_for_matching, AnchorQuery, MetadataProvider, OutcomeClass, UserId, Work, WorkId,
};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::english_identity_resolver::verify_gr_payload;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, MergeOutput, PriorityModel,
    ReconstructedOutcome,
};

const USER_ID: UserId = 7;
const WORK_ID: WorkId = 41;

const GR_COVER: &str = "https://images.gr-assets.com/books/einstein.jpg";
const OL_COVER: &str = "https://covers.openlibrary.org/b/id/12345-L.jpg";

fn empty_detail() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: None,
        subtitle: None,
        original_title: None,
        author_name: None,
        description: None,
        year: None,
        series_name: None,
        series_position: None,
        genres: None,
        language: None,
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        hc_key: None,
        gr_key: None,
        gr_work_key: None,
        ol_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        cover_url: None,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// C1 — the Goodreads payload the PO certified by hand
// ---------------------------------------------------------------------------

/// The Einstein case verbatim from the 2026-07-25 log. The library holds the
/// work-level title; Goodreads returns the edition-level title with its
/// subtitle, and lists a different printing's ISBN. Under the old rule the
/// payload was declined because no hard identifier agreed — and none ever could,
/// since the bridge requires ISBN or ASIN *equality* between two printings that
/// differ by construction.
///
/// RED before C1, green after.
#[test]
fn verify_gr_payload_trusts_the_edition_subtitle_when_the_author_agrees() {
    let captured = CapturedIdentity {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: Some("9780743264747".to_string()),
        asin: None,
        title: "Einstein".to_string(),
        author_name: "Walter Isaacson".to_string(),
        language: Some("en".to_string()),
    };
    let payload = NormalizedWorkDetail {
        title: Some("Einstein: His Life and Universe".to_string()),
        author_name: Some("Walter Isaacson".to_string()),
        isbn_13: Some("9780743264730".to_string()),
        gr_key: Some("10884".to_string()),
        ..empty_detail()
    };

    assert!(
        verify_gr_payload(&payload, &captured),
        "an edition subtitle with an agreeing author must be trusted"
    );
}

/// The author bar is what C1 leans on, so an authorless payload must still be
/// declined — the grey arm requires agreement, and an absent author abstains.
#[test]
fn verify_gr_payload_still_declines_an_authorless_payload() {
    let captured = CapturedIdentity {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: Some("9780743264747".to_string()),
        asin: None,
        title: "Einstein".to_string(),
        author_name: "Walter Isaacson".to_string(),
        language: Some("en".to_string()),
    };
    let payload = NormalizedWorkDetail {
        title: Some("Einstein: His Life and Universe".to_string()),
        author_name: None,
        gr_key: Some("10884".to_string()),
        ..empty_detail()
    };

    assert!(!verify_gr_payload(&payload, &captured));
}

/// A payload whose author is a different person is declined regardless of the
/// subtitle relaxation.
#[test]
fn verify_gr_payload_still_declines_a_disagreeing_author() {
    let captured = CapturedIdentity {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: "Einstein".to_string(),
        author_name: "Walter Isaacson".to_string(),
        language: Some("en".to_string()),
    };
    let payload = NormalizedWorkDetail {
        title: Some("Einstein: His Life and Universe".to_string()),
        author_name: Some("Jürgen Neffe".to_string()),
        gr_key: Some("10884".to_string()),
        ..empty_detail()
    };

    assert!(!verify_gr_payload(&payload, &captured));
}

// ---------------------------------------------------------------------------
// C2 — the Goodreads cover gate, through the real merge chokepoint
// ---------------------------------------------------------------------------

fn cover_first_priority(cover: Vec<MetadataProvider>) -> PriorityModel {
    PriorityModel {
        content: vec![MetadataProvider::OpenLibrary],
        description: vec![MetadataProvider::OpenLibrary],
        cover,
        audio: vec![MetadataProvider::Audnexus],
    }
}

/// An English work carrying both an OpenLibrary key and a Goodreads key — the
/// exact shape the deleted gate keyed on.
fn einstein_work(cover_manual: bool) -> Work {
    Work {
        id: WORK_ID,
        user_id: USER_ID,
        title: "Einstein".to_string(),
        author_name: "Walter Isaacson".to_string(),
        language: Some("en".to_string()),
        ol_key: Some("OL4288870W".to_string()),
        gr_key: Some("10884".to_string()),
        cover_url: Some(OL_COVER.to_string()),
        cover_manual,
        ..Default::default()
    }
}

fn gr_payload_with_subtitle() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some("Einstein: His Life and Universe".to_string()),
        author_name: Some("Walter Isaacson".to_string()),
        gr_key: Some("10884".to_string()),
        cover_url: Some(GR_COVER.to_string()),
        ..empty_detail()
    }
}

fn merge_input(work: Work, results: Vec<(MetadataProvider, NormalizedWorkDetail)>) -> MergeInput {
    let mut provider_results = HashMap::new();
    for (provider, payload) in results {
        provider_results.insert(
            provider,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(payload),
            },
        );
    }
    MergeInput {
        current_work: work,
        current_provenance: Vec::new(),
        provider_results,
        mode: EnrichmentMode::Background,
        priority_model: cover_first_priority(vec![
            MetadataProvider::Goodreads,
            MetadataProvider::OpenLibrary,
        ]),
    }
}

async fn run_merge(input: MergeInput) -> MergeOutput {
    let engine = DefaultMergeEngine::new(input.priority_model.clone());
    engine.merge(input).await.expect("merge should succeed")
}

/// Round 15 contains Goodreads art at candidate assembly without restoring the
/// deleted raw-title gate. Even this trusted, subtitled payload is parsed but
/// cannot produce a cover selection.
#[tokio::test]
async fn merge_excludes_a_goodreads_cover_whose_title_carries_a_subtitle() {
    let output = run_merge(merge_input(
        einstein_work(false),
        vec![(MetadataProvider::Goodreads, gr_payload_with_subtitle())],
    ))
    .await;

    assert!(output.cover_resolution.is_none());
}

// The gate also stripped `payload.gr_key` alongside the cover. That half has no
// assertion here, deliberately: anchors do not move through the merge —
// `UpdateWorkEnrichmentDbRequest` carries no anchor fields, and `gr_key` is not a
// merged field, so `MergeOutput` cannot observe the difference. The strip was
// inert within the merge, which is exactly why the design called it incoherent
// on its own terms. Pinning it would require a door that does not exist.

/// Deleting the gate must not disturb the user's own choice. A work whose cover
/// the user set is left alone — that protection lives in `resolve_cover`, not in
/// the gate, and is unchanged by C2.
#[tokio::test]
async fn merge_leaves_a_user_chosen_cover_alone() {
    let output = run_merge(merge_input(
        einstein_work(true),
        vec![(MetadataProvider::Goodreads, gr_payload_with_subtitle())],
    ))
    .await;

    assert!(
        output.cover_resolution.is_none(),
        "a user-set cover must never be replaced by a provider cover"
    );
}

// ---------------------------------------------------------------------------
// C1 free rider — the identity-edit modal stops threatening the user's data
// ---------------------------------------------------------------------------

/// Resolves every anchor query, but answers each provider with the shape that
/// provider really returns: Goodreads hands back the edition-level title with
/// its subtitle, OpenLibrary the work-level title alone. That asymmetry is the
/// whole point — it is what put the sibling in the grey band.
#[derive(Clone)]
struct SubtitlePreviewEnrichment;

impl EnrichmentWorkflow for SubtitlePreviewEnrichment {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: livrarr_domain::services::EnrichmentMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _priority: livrarr_domain::RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<livrarr_domain::services::EnrichmentResult, EnrichmentWorkflowError> {
        Err(EnrichmentWorkflowError::Queue(
            "enrich_work is not part of the preview door under test".into(),
        ))
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
        _data: livrarr_domain::services::SourceProviderData,
    ) {
    }

    async fn fetch_anchor_preview(
        &self,
        _provider: MetadataProvider,
        query: AnchorQuery,
        _language: Option<String>,
        _priority: livrarr_domain::RequestPriority,
    ) -> Result<IdentityPreviewOutcome, EnrichmentWorkflowError> {
        let mut record = IdentityPreviewRecord {
            author: Some("Walter Isaacson".to_string()),
            language: Some("en".to_string()),
            ..IdentityPreviewRecord::default()
        };
        match query {
            AnchorQuery::GrKey(k) => {
                record.title = Some("Einstein: His Life and Universe".to_string());
                record.gr_key = Some(k);
            }
            AnchorQuery::OlKey(k) => {
                record.title = Some("Einstein".to_string());
                record.ol_key = Some(k);
            }
            AnchorQuery::HcKey(k) => {
                record.title = Some("Einstein".to_string());
                record.hc_key = Some(k);
            }
            AnchorQuery::Isbn13(v) => {
                record.title = Some("Einstein".to_string());
                record.isbn_13 = Some(v);
            }
            AnchorQuery::Asin(v) => {
                record.title = Some("Einstein".to_string());
                record.asin = Some(v);
            }
        }
        Ok(IdentityPreviewOutcome::Resolved(Box::new(record)))
    }
}

/// The free rider. `proven_agreement` is the modal's sibling keep/drop verdict
/// and routes through the same matching authority C1 changes. Before C1 the
/// certified Goodreads record and the OpenLibrary sibling landed in the grey
/// band with no hard identifier in common, so the modal offered to **clear the
/// user's OpenLibrary id** on confirm. After C1 the sibling proves agreement and
/// is kept.
///
/// (The PO's live case showed the drop cause `unproven`, which is the
/// no-usable-record arm; this pin exercises the `disagrees` arm, where both
/// records are present and the verdict genuinely turns on the title rule.)
///
/// RED before C1, green after.
#[tokio::test]
async fn preview_keeps_the_openlibrary_sibling_when_goodreads_adds_a_subtitle() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Einstein".to_string(),
            author_name: "Walter Isaacson".to_string(),
            normalized_title: normalize_for_matching("Einstein"),
            normalized_author: normalize_for_matching("Walter Isaacson"),
            language: Some("en".to_string()),
            ..Default::default()
        })
        .await
        .expect("create work");

    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL4288870W",
        AnchorSetter::User,
    )
    .await
    .expect("seed the OpenLibrary anchor");

    let service = WorkServiceImpl::new(
        db.clone(),
        SubtitlePreviewEnrichment,
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("data dir").path().to_path_buf(),
    );

    let preview = service
        .preview_identity_edit(
            user_id,
            work.id,
            "10884",
            Some(AnchorType::new(AnchorType::GR_WORK)),
        )
        .await
        .expect("preview the Goodreads key");

    let ol = preview
        .siblings
        .iter()
        .find(|s| s.slot.as_str() == AnchorType::OL_WORK)
        .expect("the OpenLibrary sibling must be assessed");

    assert_eq!(
        ol.action,
        SiblingAction::Keep,
        "the OpenLibrary id must be kept, not offered for deletion (cause: {:?})",
        ol.cause
    );
}

/// A foreign work is still routed away from the English-centric providers by the
/// language policy at the same chokepoint. C2 removes one policy from `merge`;
/// it must not remove the other.
#[tokio::test]
async fn merge_still_drops_language_incompatible_providers() {
    let mut work = einstein_work(false);
    work.language = Some("de".to_string());
    work.title = "Die Krone Der Sterne".to_string();

    let output = run_merge(merge_input(
        work,
        vec![(
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                title: Some("The Crown of Stars".to_string()),
                cover_url: Some(OL_COVER.to_string()),
                ..empty_detail()
            },
        )],
    ))
    .await;

    assert!(
        output
            .work_update
            .as_ref()
            .map(|u| u.as_inner().title.is_none())
            .unwrap_or(true),
        "an OpenLibrary payload must not retitle a German work"
    );
}
