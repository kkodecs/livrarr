#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{
    AnchorConfidence, AnchorProvenance, AnchorSetter, AnchorType, Candidate, CandidateId,
    CapturedIdentity, ConflictSource, IdentityConflictKind, IdentityMethod, IdentityMode,
    IncomingConflictPayload, LatencyTier, MatchBasis, NewIdentityConflict, PendingReason,
    Resolution, ResolutionScore, ResolverVerdictKind, WorkSeed,
};
use livrarr_domain::services::{WorkIdentityError, WorkIdentityRepository};
use livrarr_domain::{IdentityStatus, MetadataProvider, UserId, Work, WorkId};
use livrarr_metadata::async_resolver::settle_identity;
use livrarr_metadata::english_identity_resolver::EnglishIdentityResolver;

struct ScriptedResolver {
    calls: AtomicUsize,
    result: Mutex<Resolution>,
    expected_tier: Option<LatencyTier>,
}

impl ScriptedResolver {
    fn new(result: Resolution) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            result: Mutex::new(result),
            expected_tier: None,
        }
    }

    fn expecting_tier(result: Resolution, expected_tier: LatencyTier) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            result: Mutex::new(result),
            expected_tier: Some(expected_tier),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl EnglishIdentityResolver for ScriptedResolver {
    async fn resolve(
        &self,
        _user_id: UserId,
        _seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(expected_tier) = self.expected_tier {
            assert_eq!(tier, expected_tier);
        }
        Ok(self.result.lock().expect("resolver result").clone())
    }
}

struct SeededWork {
    db: SqliteDb,
    user_id: UserId,
    work: Work,
}

#[derive(Clone, Copy)]
struct SeedAnchors {
    ol_key: Option<&'static str>,
    gr_key: Option<&'static str>,
    hc_key: Option<&'static str>,
    isbn_13: Option<&'static str>,
    asin: Option<&'static str>,
}

impl SeedAnchors {
    const NONE: Self = Self {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
    };
}

async fn seed_work(
    title: &str,
    identity_status: IdentityStatus,
    anchors: SeedAnchors,
) -> SeededWork {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: title.to_string(),
            author_name: "Frank Herbert".to_string(),
            normalized_title: title.to_ascii_lowercase(),
            normalized_author: "frank herbert".to_string(),
            year: Some(1965),
            cover_url: Some("https://covers.example/dune.jpg".to_string()),
            language: Some("en".to_string()),
            description: Some("Desert planet politics.".to_string()),
            series_name: Some("Dune".to_string()),
            series_position: Some(1.0),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..Default::default()
        })
        .await
        .expect("seed work");
    assert!(created, "test work fixture should be unique");

    seed_anchor_set(&db, work.id, anchors).await;
    db.set_identity_status(user_id, work.id, identity_status)
        .await
        .expect("seed identity status");

    let work = db
        .get_work(user_id, work.id)
        .await
        .expect("read seeded work");

    SeededWork { db, user_id, work }
}

async fn seed_anchor_set(db: &SqliteDb, work_id: WorkId, anchors: SeedAnchors) {
    if let Some(value) = anchors.ol_key {
        confirm_anchor(db, work_id, AnchorType::OL_WORK, value).await;
    }
    if let Some(value) = anchors.gr_key {
        confirm_anchor(db, work_id, AnchorType::GR_WORK, value).await;
    }
    if let Some(value) = anchors.hc_key {
        confirm_anchor(db, work_id, AnchorType::HC_WORK, value).await;
    }
    if let Some(value) = anchors.isbn_13 {
        confirm_anchor(db, work_id, AnchorType::ISBN_13, value).await;
    }
    if let Some(value) = anchors.asin {
        confirm_anchor(db, work_id, AnchorType::ASIN, value).await;
    }
}

async fn confirm_anchor(db: &SqliteDb, work_id: WorkId, anchor_type: &str, value: &str) {
    db.confirm_anchor(
        work_id,
        AnchorType::new(anchor_type),
        value,
        AnchorSetter::Import,
    )
    .await
    .expect("confirm starting anchor");
}

fn captured(
    ol_key: Option<&str>,
    gr_key: Option<&str>,
    hc_key: Option<&str>,
    isbn_13: Option<&str>,
    asin: Option<&str>,
) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: gr_key.map(str::to_string),
        hc_key: hc_key.map(str::to_string),
        isbn_13: isbn_13.map(str::to_string),
        asin: asin.map(str::to_string),
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    }
}

fn anchorless_captured(title: &str) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: title.to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    }
}

fn hard_provenance(identity: &CapturedIdentity) -> AnchorProvenance {
    AnchorProvenance {
        ol_key: identity.ol_key.as_ref().map(|_| MatchBasis::Hard),
        gr_key: identity.gr_key.as_ref().map(|_| MatchBasis::Hard),
        hc_key: identity.hc_key.as_ref().map(|_| MatchBasis::Hard),
        isbn_13: identity.isbn_13.as_ref().map(|_| MatchBasis::Hard),
        asin: identity.asin.as_ref().map(|_| MatchBasis::Hard),
    }
}

fn resolved(identity: CapturedIdentity) -> Resolution {
    let provenance = hard_provenance(&identity);
    Resolution::Resolved {
        identity,
        method: IdentityMethod::IsbnDirect,
        candidate_id: CandidateId("candidate-resolved".to_string()),
        provenance,
    }
}

fn unresolved(reason: PendingReason, captured: CapturedIdentity) -> Resolution {
    let provenance = hard_provenance(&captured);
    Resolution::Unresolved {
        captured,
        reason,
        candidate_id: None,
        provenance,
    }
}

fn needs_confirmation() -> Resolution {
    Resolution::NeedsConfirmation {
        candidates: vec![Candidate {
            candidate_id: CandidateId("candidate-ambiguous".to_string()),
            anchors: captured(None, None, None, Some("9780441013593"), None),
            cover_url: None,
            sources: vec![MetadataProvider::OpenLibrary],
            score: ResolutionScore {
                title_jaccard: 0.72,
                author_overlap: 1,
                runner_up_delta: 0.01,
            },
            existing_work_id: None,
        }],
    }
}

fn conflict_resolution(
    user_id: UserId,
    existing_work_id: WorkId,
    captured: CapturedIdentity,
    tied: Vec<CapturedIdentity>,
) -> Resolution {
    Resolution::Conflict {
        conflict: NewIdentityConflict {
            user_id,
            existing_work_id,
            kind: IdentityConflictKind::QuorumTie,
            incoming: incoming_from_captured(&captured),
            raised_by: ConflictSource::Refresh,
            raised_source_path: None,
        },
        captured,
        tied,
    }
}

fn incoming_from_captured(captured: &CapturedIdentity) -> IncomingConflictPayload {
    IncomingConflictPayload {
        ol_key: captured.ol_key.clone(),
        gr_key: captured.gr_key.clone(),
        hc_key: captured.hc_key.clone(),
        isbn_13: captured.isbn_13.clone(),
        asin: captured.asin.clone(),
        title: captured.title.clone(),
        author_name: captured.author_name.clone(),
        year: None,
        cover_url: None,
        top_candidates: Vec::new(),
    }
}

async fn read_work(case: &SeededWork) -> Work {
    case.db
        .get_work(case.user_id, case.work.id)
        .await
        .expect("read work after settle")
}

async fn confirmed_anchors(case: &SeededWork) -> Vec<(String, String)> {
    case.db
        .list_anchors(case.work.id)
        .await
        .expect("list anchors")
        .into_iter()
        .filter(|anchor| anchor.confidence == AnchorConfidence::Confirmed)
        .map(|anchor| (anchor.anchor_type.as_str().to_string(), anchor.anchor_value))
        .collect()
}

fn assert_anchor(anchors: &[(String, String)], anchor_type: &str, value: &str) {
    assert!(
        anchors
            .iter()
            .any(|(kind, stored)| kind == anchor_type && stored == value),
        "missing confirmed anchor {anchor_type}={value}; actual anchors: {anchors:?}"
    );
}

fn assert_report(
    report: &livrarr_domain::identity::IdentityReport,
    prior_status: IdentityStatus,
    final_status: IdentityStatus,
    verdict: Option<ResolverVerdictKind>,
    merged: &[&str],
) {
    assert_eq!(report.prior_status, prior_status);
    assert_eq!(report.final_status, final_status);
    assert_eq!(report.verdict, verdict);
    for expected in merged {
        assert!(
            report
                .anchors_merged
                .iter()
                .any(|actual| actual == expected),
            "expected report to include merged anchor {expected}, got {:?}",
            report.anchors_merged
        );
    }
}

#[tokio::test]
async fn ac_001_pending_work_anchor_ends_confirmed_with_anchor_persisted() {
    // AC-001 / REQ-001, REQ-003
    // The resolved identity's title must match the work's (the old containment
    // gate tolerated the "AC001" prefix; the matching authority does not).
    let case = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver =
        ScriptedResolver::new(resolved(captured(Some("OL45883W"), None, None, None, None)));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Confirmed);
    assert_anchor(&anchors, AnchorType::OL_WORK, "OL45883W");
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Resolved),
        &["ol_work"],
    );
}

#[tokio::test]
async fn ac_002_pending_bridge_only_ends_provisional() {
    // AC-002 / REQ-003
    let case = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::new(resolved(captured(
        None,
        None,
        None,
        Some("9780441013593"),
        Some("B000TEST12"),
    )));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Provisional);
    assert_anchor(&anchors, AnchorType::ISBN_13, "9780441013593");
    assert_anchor(&anchors, AnchorType::ASIN, "B000TEST12");
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Provisional,
        Some(ResolverVerdictKind::Resolved),
        &["isbn_13", "asin"],
    );
}

#[tokio::test]
async fn ac_003_background_no_candidates_stays_pending() {
    // AC-003 / REQ-003, REQ-005
    let case = seed_work("AC003 Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::expecting_tier(
        unresolved(
            PendingReason::NoCandidates,
            anchorless_captured("AC003 Dune"),
        ),
        LatencyTier::Background,
    );

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Pending);
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Pending,
        Some(ResolverVerdictKind::Unresolved),
        &[],
    );
}

#[tokio::test]
async fn ac_004_interactive_no_candidates_stays_pending() {
    // AC-004 / REQ-005
    let case = seed_work("AC004 Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::expecting_tier(
        unresolved(
            PendingReason::NoCandidates,
            anchorless_captured("AC004 Dune"),
        ),
        LatencyTier::Interactive,
    );

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Interactive,
        ConflictSource::ManualAdd,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Pending);
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Pending,
        Some(ResolverVerdictKind::Unresolved),
        &[],
    );
}

#[tokio::test]
async fn ac_005_resolvable_work_reaches_same_identity_in_both_modes() {
    // AC-005 / REQ-005
    // Separate DBs per seed_work; identical titles are fine and must match
    // the scripted identity's title for the authority-grade gate.
    let interactive = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let background = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let identity = captured(
        Some("OL45883W"),
        Some("234225"),
        None,
        Some("9780441013593"),
        None,
    );
    let interactive_resolver =
        ScriptedResolver::expecting_tier(resolved(identity.clone()), LatencyTier::Interactive);
    let background_resolver =
        ScriptedResolver::expecting_tier(resolved(identity), LatencyTier::Background);

    let interactive_report = settle_identity(
        &interactive_resolver,
        &interactive.db,
        interactive.user_id,
        &interactive.work,
        IdentityMode::Interactive,
        ConflictSource::ManualAdd,
    )
    .await
    .expect("settle interactive identity");
    let background_report = settle_identity(
        &background_resolver,
        &background.db,
        background.user_id,
        &background.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle background identity");

    let after_interactive = read_work(&interactive).await;
    let after_background = read_work(&background).await;
    assert_eq!(after_interactive.identity_status, IdentityStatus::Confirmed);
    assert_eq!(after_background.identity_status, IdentityStatus::Confirmed);
    assert_eq!(after_interactive.ol_key, after_background.ol_key);
    assert_eq!(after_interactive.gr_key, after_background.gr_key);
    assert_eq!(after_interactive.isbn_13, after_background.isbn_13);
    assert_eq!(
        interactive_report.final_status,
        background_report.final_status
    );
}

#[tokio::test]
async fn ac_006_transient_unresolved_merges_captured_anchors_and_stays_pending() {
    // AC-006 / REQ-003
    let case = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::new(unresolved(
        PendingReason::OlUnavailable,
        captured(None, None, None, Some("9780441013593"), None),
    ));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Pending);
    assert_anchor(&anchors, AnchorType::ISBN_13, "9780441013593");
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Pending,
        Some(ResolverVerdictKind::Unresolved),
        &["isbn_13"],
    );
}

#[tokio::test]
async fn ac_007_pending_conflict_ends_terminal_conflict() {
    // AC-007 / REQ-003
    let case = seed_work("AC007 Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let conflict = conflict_resolution(
        case.user_id,
        case.work.id,
        captured(Some("OL111W"), None, None, None, None),
        vec![captured(Some("OL222W"), None, None, None, None)],
    );
    let resolver = ScriptedResolver::new(conflict);

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Conflict);
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Conflict,
        Some(ResolverVerdictKind::Conflict),
        &[],
    );
}

#[tokio::test]
async fn ac_008_confirmed_weaker_verdict_does_not_overwrite_or_downgrade() {
    // AC-008 / REQ-004
    let case = seed_work(
        "Dune",
        IdentityStatus::Confirmed,
        SeedAnchors {
            ol_key: Some("OL111W"),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
        },
    )
    .await;
    let resolver = ScriptedResolver::new(unresolved(
        PendingReason::MalformedResponse,
        captured(Some("OL222W"), None, None, Some("9780441013593"), None),
    ));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Confirmed);
    assert_eq!(after.ol_key.as_deref(), Some("OL111W"));
    assert_anchor(&anchors, AnchorType::OL_WORK, "OL111W");
    // REQ-006 contradiction veto (settle-road matching): an identity whose
    // work key contradicts the established anchor merges NOTHING — its
    // non-contradicting ids are held as pending anchors, never confirmed.
    assert!(!anchors
        .iter()
        .any(|(kind, value)| kind == AnchorType::ISBN_13 && value == "9780441013593"));
    assert!(!anchors
        .iter()
        .any(|(kind, value)| kind == AnchorType::OL_WORK && value == "OL222W"));
    assert_report(
        &report,
        IdentityStatus::Confirmed,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Unresolved),
        &[],
    );
}

#[tokio::test]
async fn ac_009_identity_settle_does_not_change_metadata_fields() {
    // AC-009 / REQ-004
    let case = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let before = case.work.clone();
    let resolver = ScriptedResolver::new(resolved(captured(
        Some("OL45883W"),
        None,
        None,
        Some("9780441013593"),
        None,
    )));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;

    assert_eq!(after.cover_url, before.cover_url);
    assert_eq!(after.description, before.description);
    assert_eq!(after.series_name, before.series_name);
    assert_eq!(after.series_position, before.series_position);
    assert_eq!(after.year, before.year);
    assert_eq!(after.language, before.language);
    assert_eq!(after.enrichment_status, before.enrichment_status);
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Resolved),
        &["ol_work", "isbn_13"],
    );
}

#[tokio::test]
async fn ac_010_terminal_conflict_not_found_needs_review_are_untouched() {
    // AC-010 / REQ-006
    for (title, status) in [
        ("AC010 Conflict", IdentityStatus::Conflict),
        ("AC010 NotFound", IdentityStatus::NotFound),
        ("AC010 NeedsReview", IdentityStatus::NeedsReview),
    ] {
        let case = seed_work(title, status, SeedAnchors::NONE).await;
        let resolver =
            ScriptedResolver::new(resolved(captured(Some("OL45883W"), None, None, None, None)));

        let report = settle_identity(
            &resolver,
            &case.db,
            case.user_id,
            &case.work,
            IdentityMode::Background,
            ConflictSource::Refresh,
        )
        .await
        .expect("settle identity");
        let after = read_work(&case).await;

        assert_eq!(resolver.call_count(), 0);
        assert_eq!(after.identity_status, status);
        assert_report(&report, status, status, None, &[]);
    }
}

#[tokio::test]
async fn ac_011_running_twice_on_confirmed_is_noop_without_duplicate_anchors() {
    // AC-011 / REQ-007
    let case = seed_work(
        "AC011 Dune",
        IdentityStatus::Confirmed,
        SeedAnchors {
            ol_key: Some("OL45883W"),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
        },
    )
    .await;
    let resolver =
        ScriptedResolver::new(resolved(captured(Some("OL45883W"), None, None, None, None)));

    let first = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("first settle");
    let reread = read_work(&case).await;
    let second = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &reread,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("second settle");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Confirmed);
    assert_eq!(
        anchors
            .iter()
            .filter(|(kind, value)| kind == AnchorType::OL_WORK && value == "OL45883W")
            .count(),
        1
    );
    assert_report(
        &first,
        IdentityStatus::Confirmed,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Resolved),
        &[],
    );
    assert_report(
        &second,
        IdentityStatus::Confirmed,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Resolved),
        &[],
    );
}

#[tokio::test]
async fn ac_012_never_writes_not_found_and_existing_not_found_is_preserved() {
    // AC-012 / REQ-002, REQ-006
    let pending = seed_work("AC012 Pending", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let pending_resolver = ScriptedResolver::new(unresolved(
        PendingReason::NoCandidates,
        anchorless_captured("AC012 Pending"),
    ));
    let pending_report = settle_identity(
        &pending_resolver,
        &pending.db,
        pending.user_id,
        &pending.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle pending identity");
    let pending_after = read_work(&pending).await;

    let not_found = seed_work(
        "AC012 NotFound",
        IdentityStatus::NotFound,
        SeedAnchors::NONE,
    )
    .await;
    let not_found_resolver =
        ScriptedResolver::new(resolved(captured(Some("OL45883W"), None, None, None, None)));
    let not_found_report = settle_identity(
        &not_found_resolver,
        &not_found.db,
        not_found.user_id,
        &not_found.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle not-found identity");
    let not_found_after = read_work(&not_found).await;

    assert_ne!(pending_after.identity_status, IdentityStatus::NotFound);
    assert_eq!(pending_after.identity_status, IdentityStatus::Pending);
    assert_eq!(not_found_after.identity_status, IdentityStatus::NotFound);
    assert_report(
        &pending_report,
        IdentityStatus::Pending,
        IdentityStatus::Pending,
        Some(ResolverVerdictKind::Unresolved),
        &[],
    );
    assert_report(
        &not_found_report,
        IdentityStatus::NotFound,
        IdentityStatus::NotFound,
        None,
        &[],
    );
}

#[tokio::test]
async fn ac_013_engine_performs_badge_and_anchor_writes_report_is_audit_only() {
    // AC-013 / REQ-008
    let case = seed_work("Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::new(resolved(captured(
        Some("OL45883W"),
        None,
        None,
        Some("9780441013593"),
        None,
    )));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Confirmed);
    assert_anchor(&anchors, AnchorType::OL_WORK, "OL45883W");
    assert_anchor(&anchors, AnchorType::ISBN_13, "9780441013593");
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Resolved),
        &["ol_work", "isbn_13"],
    );
}

#[tokio::test]
async fn ac_014_provisional_reresolves_to_work_anchor_upgrades_to_confirmed() {
    // AC-014 / REQ-003, REQ-007
    let case = seed_work(
        "Dune",
        IdentityStatus::Provisional,
        SeedAnchors {
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: Some("9780441013593"),
            asin: None,
        },
    )
    .await;
    let resolver = ScriptedResolver::new(resolved(captured(
        Some("OL45883W"),
        None,
        None,
        Some("9780441013593"),
        None,
    )));

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Confirmed);
    assert_anchor(&anchors, AnchorType::ISBN_13, "9780441013593");
    assert_anchor(&anchors, AnchorType::OL_WORK, "OL45883W");
    assert_report(
        &report,
        IdentityStatus::Provisional,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Resolved),
        &["ol_work"],
    );
}

#[tokio::test]
async fn ac_015_provisional_non_contradicting_verdict_is_preserved() {
    // AC-015 / REQ-003, REQ-004
    let case = seed_work(
        "AC015 Dune",
        IdentityStatus::Provisional,
        SeedAnchors {
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: Some("9780441013593"),
            asin: None,
        },
    )
    .await;
    let resolver = ScriptedResolver::new(needs_confirmation());

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;
    let anchors = confirmed_anchors(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Provisional);
    assert_anchor(&anchors, AnchorType::ISBN_13, "9780441013593");
    assert_report(
        &report,
        IdentityStatus::Provisional,
        IdentityStatus::Provisional,
        Some(ResolverVerdictKind::NeedsConfirmation),
        &[],
    );
}

#[tokio::test]
async fn ac_016_background_needs_confirmation_pending_becomes_needs_review() {
    // AC-016 / REQ-003, REQ-005
    let case = seed_work("AC016 Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::expecting_tier(needs_confirmation(), LatencyTier::Background);

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::NeedsReview);
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::NeedsReview,
        Some(ResolverVerdictKind::NeedsConfirmation),
        &[],
    );
}

#[tokio::test]
async fn ac_017_interactive_needs_confirmation_pending_stays_pending() {
    // AC-017 / REQ-005
    let case = seed_work("AC017 Dune", IdentityStatus::Pending, SeedAnchors::NONE).await;
    let resolver = ScriptedResolver::expecting_tier(needs_confirmation(), LatencyTier::Interactive);

    let report = settle_identity(
        &resolver,
        &case.db,
        case.user_id,
        &case.work,
        IdentityMode::Interactive,
        ConflictSource::ManualAdd,
    )
    .await
    .expect("settle identity");
    let after = read_work(&case).await;

    assert_eq!(after.identity_status, IdentityStatus::Pending);
    assert_report(
        &report,
        IdentityStatus::Pending,
        IdentityStatus::Pending,
        Some(ResolverVerdictKind::NeedsConfirmation),
        &[],
    );
}

#[tokio::test]
async fn ac_018_conflict_preservation_uses_tied_anchor_contradiction_not_kind() {
    // AC-018 / REQ-003, REQ-004
    let contradicting = seed_work(
        "AC018 Contradicting Dune",
        IdentityStatus::Confirmed,
        SeedAnchors {
            ol_key: Some("OL111W"),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
        },
    )
    .await;
    let contradicting_resolver = ScriptedResolver::new(conflict_resolution(
        contradicting.user_id,
        contradicting.work.id,
        captured(Some("OL111W"), None, None, None, None),
        vec![captured(Some("OL222W"), None, None, None, None)],
    ));

    let contradicting_report = settle_identity(
        &contradicting_resolver,
        &contradicting.db,
        contradicting.user_id,
        &contradicting.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle contradicting conflict");
    let contradicting_after = read_work(&contradicting).await;

    assert_eq!(
        contradicting_after.identity_status,
        IdentityStatus::Conflict,
        "a different same-kind work anchor on a non-representative tied cluster must raise Conflict"
    );
    assert_report(
        &contradicting_report,
        IdentityStatus::Confirmed,
        IdentityStatus::Conflict,
        Some(ResolverVerdictKind::Conflict),
        &[],
    );

    let anchorless = seed_work(
        "AC018 Anchorless Dune",
        IdentityStatus::Confirmed,
        SeedAnchors {
            ol_key: Some("OL333W"),
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
        },
    )
    .await;
    let anchorless_resolver = ScriptedResolver::new(conflict_resolution(
        anchorless.user_id,
        anchorless.work.id,
        anchorless_captured("Anchorless representative"),
        vec![anchorless_captured("Anchorless tied cluster")],
    ));

    let anchorless_report = settle_identity(
        &anchorless_resolver,
        &anchorless.db,
        anchorless.user_id,
        &anchorless.work,
        IdentityMode::Background,
        ConflictSource::Refresh,
    )
    .await
    .expect("settle anchorless conflict");
    let anchorless_after = read_work(&anchorless).await;
    let anchorless_anchors = confirmed_anchors(&anchorless).await;

    assert_eq!(anchorless_after.identity_status, IdentityStatus::Confirmed);
    assert_anchor(&anchorless_anchors, AnchorType::OL_WORK, "OL333W");
    assert_report(
        &anchorless_report,
        IdentityStatus::Confirmed,
        IdentityStatus::Confirmed,
        Some(ResolverVerdictKind::Conflict),
        &[],
    );
}
