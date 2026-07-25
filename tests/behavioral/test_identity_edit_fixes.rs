//! identity-edit fix unit — red-first pins.
//!
//! Contract: `docs/design-identity-edit-fixes.md` (r1), which repairs defects the
//! `a7f03540` merge knowingly carried. Every test here fails on that commit and passes
//! once its fix lands. Numbering follows the design's F-items.
//!
//! All surfaces referenced already exist, so this file compiles ungated — its RED points
//! at behavior, never at a missing type.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use livrarr_behavioral::stubs::{create_test_user, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::WorkDb;
use livrarr_db::{backfill_work_identity_ledger, CreateWorkDbRequest, WorkDbCreate};
use livrarr_domain::identity::{
    AnchorSetter, AnchorType, ConflictSource, IdentityMode, PendingReason,
};
use livrarr_domain::identity_edit::classify_identifier_input;
use livrarr_domain::services::{
    EnrichmentMode, EnrichmentResult, EnrichmentWorkflow, EnrichmentWorkflowError,
    IdentityPreviewOutcome, IdentityPreviewRecord, SourceProviderData, WorkIdentityRepository,
    WorkService,
};
use livrarr_domain::AnchorQuery;
use livrarr_domain::{
    normalize_for_matching, EnrichmentStatus, IdentityStatus, UserId, Work, WorkId,
};
use livrarr_metadata::work_service::WorkServiceImpl;

async fn create_work(db: &SqliteDb, user_id: UserId, title: &str) -> i64 {
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: title.to_string(),
            author_name: "Test Author".to_string(),
            normalized_title: normalize_for_matching(title),
            normalized_author: normalize_for_matching("Test Author"),
            ..Default::default()
        })
        .await
        .expect("create test work");
    work.id
}

async fn create_user_n(db: &SqliteDb, n: usize) -> UserId {
    use livrarr_db::UserDb;
    db.create_user(livrarr_db::CreateUserDbRequest {
        username: format!("user{n}"),
        password_hash: "hash".into(),
        role: livrarr_domain::UserRole::User,
        api_key_hash: format!("apikey{n}"),
    })
    .await
    .expect("create user")
    .id
}

async fn confirmed_owner_count(db: &SqliteDb, work_id: i64, anchor_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed'",
    )
    .bind(work_id)
    .bind(anchor_type)
    .fetch_one(db.pool())
    .await
    .expect("count confirmed anchors")
}

// ---------------------------------------------------------------------------
// F3 — startup backfill must see an owner that exists only in the ledger
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F3.
///
/// Constructed-state justification (CLAUDE.md "Tests drive the real door"): the door
/// under test IS `backfill_work_identity_ledger` running against arbitrary pre-existing
/// data — that is its entire contract. The legacy shape it must survive (a confirmed
/// ledger row whose denormalized column is NULL) is by definition one today's writers no
/// longer produce, so it cannot be reached forward through them. The ledger row itself is
/// still written by the real production writer (`confirm_anchor`); only the legacy column
/// state is set directly.
///
/// RED on a7f03540: the pass builds its owner map from works that have a non-NULL legacy
/// column, so the true owner is invisible, the column-only work is elected, and the
/// second confirmed insert violates `uniq_user_confirmed_work_anchor` — backfill returns
/// Err and **startup fails**.
#[tokio::test]
async fn f3_backfill_preserves_a_confirmed_owner_that_exists_only_in_the_ledger() {
    let db = create_test_db().await;
    let user = create_test_user(&db).await;

    // Lower id, legacy column only, no ledger row — the work "lowest id wins" would elect.
    let column_only = create_work(&db, user, "Column Only").await;
    sqlx::query("UPDATE works SET gr_key = '777123' WHERE id = ?")
        .bind(column_only)
        .execute(db.pool())
        .await
        .expect("seed legacy column");

    // Higher id, real confirmed ledger row, every legacy column NULL.
    let ledger_owner = create_work(&db, user, "Ledger Owner").await;
    db.confirm_anchor(
        ledger_owner,
        AnchorType::new(AnchorType::GR_WORK),
        "777123",
        AnchorSetter::User,
    )
    .await
    .expect("real writer confirms the ledger row");
    sqlx::query("UPDATE works SET gr_key = NULL WHERE id = ?")
        .bind(ledger_owner)
        .execute(db.pool())
        .await
        .expect("reproduce the legacy ledger-only shape");

    backfill_work_identity_ledger(db.pool())
        .await
        .expect("backfill must not fail startup when the owner is ledger-only");

    assert_eq!(
        confirmed_owner_count(&db, ledger_owner, AnchorType::GR_WORK).await,
        1,
        "the existing ledger-only owner must be preserved"
    );
    assert_eq!(
        confirmed_owner_count(&db, column_only, AnchorType::GR_WORK).await,
        0,
        "a column-only work must not be elected owner over an existing ledger owner"
    );

    let kept: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(column_only)
        .fetch_one(db.pool())
        .await
        .expect("loser column");
    assert_eq!(
        kept.as_deref(),
        Some("777123"),
        "the loser keeps its column — visible and clearable, never silently dropped"
    );
}

// ---------------------------------------------------------------------------
// F1b — the delayed NotFound conclusion must claim a pre-wait generation
// ---------------------------------------------------------------------------

/// Enrichment that parks inside `enrich_work` until released, then reports that it
/// could not verify the work's identity.
///
/// This is the barrier for F1b, and it is the honest one: `identity_not_found` is
/// decided *by this call*, so holding the call open is exactly the window in which a
/// user edit can land before the conclusion is written. Nothing proceeds on a timer —
/// the test waits on `entered()` for proof the road is genuinely in flight.
#[derive(Clone)]
struct ParkingNotFoundEnrichment {
    entered: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
}

impl ParkingNotFoundEnrichment {
    fn new() -> Self {
        Self {
            entered: Arc::new(AtomicUsize::new(0)),
            release: Arc::new(tokio::sync::Semaphore::new(0)),
        }
    }

    fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

impl EnrichmentWorkflow for ParkingNotFoundEnrichment {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _priority: livrarr_domain::RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let _permit = self
            .release
            .acquire()
            .await
            .expect("release semaphore stays open");
        Ok(EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("parked-stub".into()),
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            identity_not_found: true,
            changed: false,
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

/// Design: `docs/design-identity-edit-fixes.md` F1b.
///
/// RED on a7f03540: `complete_add` DOES claim a generation — but it reads it at
/// `crates/livrarr-metadata/src/work_service.rs:1192`, *after* enrichment returned. The
/// CAS therefore claims a generation the NotFound decision never observed, and succeeds
/// in stamping a stale conclusion over an edit the user made mid-flight. The write is
/// guarded in form and unguarded in fact — which is why the AC-10 race repro, and any
/// "is there a CAS?" review, both miss it.
#[tokio::test]
async fn f1b_a_delayed_not_found_must_not_overwrite_an_edit_made_during_the_wait() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Bridge-Anchored Work").await;

    // The enrichment gate refuses to run for a held identity (Pending/Conflict/
    // NeedsReview), so a work that never leaves Pending never reaches the wait this
    // test is about. Seed a real bridge anchor through the production writer that also
    // recomputes the badge: the work becomes non-anchorless and unheld, exactly the
    // state `complete_add` enriches.
    db.confirm_anchor_and_recompute_badge(
        work_id,
        AnchorType::new(AnchorType::ISBN_13),
        "9780306406157",
        AnchorSetter::User,
    )
    .await
    .expect("seed a bridge anchor and raise the badge");
    let seeded = db.get_work(user_id, work_id).await.expect("seeded work");
    assert!(
        !matches!(
            seeded.identity_status,
            IdentityStatus::Pending | IdentityStatus::Conflict | IdentityStatus::NeedsReview
        ),
        "the fixture must leave the work unheld or enrichment never runs: {:?}",
        seeded.identity_status
    );

    let enrichment = ParkingNotFoundEnrichment::new();
    let service = Arc::new(WorkServiceImpl::new(
        db.clone(),
        enrichment.clone(),
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("data dir").path().to_path_buf(),
    ));

    // 1. Start the add-completion road. It parks inside enrichment.
    let bg = service.clone();
    let add = tokio::spawn(async move {
        bg.complete_add(
            user_id,
            work_id,
            None,
            None,
            IdentityMode::Background,
            ConflictSource::ManualAdd,
        )
        .await;
    });

    // 2. Proceed on proof the road is in flight, never on a timer.
    let dispatched = tokio::time::timeout(Duration::from_secs(20), async {
        while enrichment.entered() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        dispatched.is_ok(),
        "enrichment never ran, so this test observed nothing — fix the harness before \
         trusting any verdict from it."
    );

    // 3. Mid-wait, the user certifies a real identity through the production write
    //    chokepoint (confirm_anchor_in_tx — the single point that enforces the identity
    //    write contract and advances identity_generation).
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "424242",
        AnchorSetter::User,
    )
    .await
    .expect("user certifies a GR identity mid-flight");
    let after_edit = db
        .get_work_with_identity_generation(user_id, work_id)
        .await
        .expect("read generation after the edit")
        .1;

    // 4. Release the stale conclusion.
    enrichment.release();
    add.await.expect("complete_add task");

    let work = db.get_work(user_id, work_id).await.expect("final work");
    assert_ne!(
        work.identity_status,
        IdentityStatus::NotFound,
        "a NotFound decided before the user's edit must not park the corrected work"
    );
    assert_eq!(
        work.gr_key.as_deref(),
        Some("424242"),
        "the user's certified identity must survive the delayed conclusion"
    );
    let final_generation = db
        .get_work_with_identity_generation(user_id, work_id)
        .await
        .expect("read final generation")
        .1;
    assert_eq!(
        final_generation, after_edit,
        "a superseded conclusion must write nothing at all"
    );
}

// ---------------------------------------------------------------------------
// F4b — a same-value commit must still clean up the slot's pending rows
// ---------------------------------------------------------------------------

/// Enrichment that certifies whatever GR key it is asked about. Only the preview seam
/// (`fetch_anchor_preview`) is exercised; `enrich_work` is never reached by the
/// preview/commit doors under test.
#[derive(Clone)]
struct CertifyingPreviewEnrichment {
    title: String,
    author: String,
}

impl EnrichmentWorkflow for CertifyingPreviewEnrichment {
    async fn enrich_work(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        _mode: EnrichmentMode,
        _candidate_id: Option<livrarr_domain::identity::CandidateId>,
        _priority: livrarr_domain::RequestPriority,
        _freshness: livrarr_domain::Freshness,
    ) -> Result<EnrichmentResult, EnrichmentWorkflowError> {
        Err(EnrichmentWorkflowError::Queue(
            "enrich_work is not part of the preview/commit doors under test".into(),
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
        _data: SourceProviderData,
    ) {
    }

    async fn fetch_anchor_preview(
        &self,
        _provider: livrarr_domain::MetadataProvider,
        query: AnchorQuery,
        _language: Option<String>,
        _priority: livrarr_domain::RequestPriority,
    ) -> Result<IdentityPreviewOutcome, EnrichmentWorkflowError> {
        let mut record = IdentityPreviewRecord {
            title: Some(self.title.clone()),
            author: Some(self.author.clone()),
            language: Some("en".to_string()),
            ..IdentityPreviewRecord::default()
        };
        match query {
            AnchorQuery::GrKey(k) => record.gr_key = Some(k),
            AnchorQuery::OlKey(k) => record.ol_key = Some(k),
            AnchorQuery::HcKey(k) => record.hc_key = Some(k),
            AnchorQuery::Isbn13(v) => record.isbn_13 = Some(v),
            AnchorQuery::Asin(v) => record.asin = Some(v),
        }
        Ok(IdentityPreviewOutcome::Resolved(Box::new(record)))
    }
}

/// Design: `docs/design-identity-edit-fixes.md` F4b.
///
/// RED on a7f03540: `is_true_no_op`
/// (`crates/livrarr-metadata/src/work_service.rs:2253-2258`) weighs the confirmed value,
/// the column, the drop set, implicated conflicts, dead-ends and badge coherence — and
/// never looks at the slot's pending rows. Re-certifying the value you already have
/// therefore returns `changed:false` without entering the transaction, and the stale
/// pending guess AC-20 requires deleting survives, still affirmable.
#[tokio::test]
async fn f4b_a_same_value_commit_still_deletes_the_slots_pending_rows() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work_id = create_work(&db, user_id, "Test Author Book").await;

    // Already user-confirmed and mirrored into the column, via the real writer.
    db.confirm_anchor_and_recompute_badge(
        work_id,
        AnchorType::new(AnchorType::GR_WORK),
        "123",
        AnchorSetter::User,
    )
    .await
    .expect("seed the confirmed GR anchor");

    // A stale fuzzy guess in the same slot, via the real pending writer.
    db.record_pending_anchor(work_id, AnchorType::new(AnchorType::GR_WORK), "999")
        .await
        .expect("seed the stale pending guess");

    let service = WorkServiceImpl::new(
        db.clone(),
        CertifyingPreviewEnrichment {
            title: "Test Author Book".to_string(),
            author: "Test Author".to_string(),
        },
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("data dir").path().to_path_buf(),
    );

    // The user re-certifies the value the work already carries.
    let preview = service
        .preview_identity_edit(
            user_id,
            work_id,
            "123",
            Some(AnchorType::new(AnchorType::GR_WORK)),
        )
        .await
        .expect("preview the same value");
    let token = preview.preview_id.clone().expect("a certifiable preview");

    service
        .commit_identity_edit(
            user_id,
            work_id,
            AnchorType::new(AnchorType::GR_WORK),
            &token,
        )
        .await
        .expect("commit the same value");

    let pending_left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = 'gr_work' AND confidence = 'pending'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("count pending rows");
    assert_eq!(
        pending_left, 0,
        "committing a slot must clear its stale pending guesses even when the certified \
         value is unchanged (AC-20)"
    );
}

// ---------------------------------------------------------------------------
// F5a — a globally full preview store must replace the caller's own oldest token
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F5a.
///
/// RED on a7f03540: per-user eviction fires only once the caller already holds
/// `PREVIEW_PER_USER_CAP` tokens (`crates/livrarr-metadata/src/work_service.rs:2526`),
/// but the global-capacity rejection at `:2531` runs regardless. A caller holding an
/// evictable token but sitting under the per-user cap is refused instead of having their
/// oldest replaced.
///
/// No constructed state: every token in the store is created by the real
/// `preview_identity_edit` door, which is the only thing that mints one.
#[tokio::test]
async fn f5a_a_full_preview_store_replaces_the_callers_oldest_token() {
    const GLOBAL_CAP: usize = 64;
    const PER_USER_CAP: usize = 4;

    let db = create_test_db().await;
    let service = WorkServiceImpl::new(
        db.clone(),
        CertifyingPreviewEnrichment {
            title: "Test Author Book".to_string(),
            author: "Test Author".to_string(),
        },
        StubHttpFetcher::new(),
        tempfile::tempdir().expect("data dir").path().to_path_buf(),
    );

    let mut next_key = 1_000_000;

    // The caller takes two tokens first, while there is still room — so that when the
    // store fills they hold an evictable token but remain under the per-user cap.
    let caller = create_user_n(&db, 0).await;
    let caller_work = create_work(&db, caller, "Test Author Book").await;
    for _ in 0..2 {
        next_key += 1;
        service
            .preview_identity_edit(
                caller,
                caller_work,
                &next_key.to_string(),
                Some(AnchorType::new(AnchorType::GR_WORK)),
            )
            .await
            .expect("caller's early previews fit");
    }

    // Fill the rest of the store with other users, PER_USER_CAP each.
    let mut placed = 2;
    let mut other = 0;
    while placed < GLOBAL_CAP {
        other += 1;
        let user = create_user_n(&db, other).await;
        let work = create_work(&db, user, "Test Author Book").await;
        for _ in 0..PER_USER_CAP.min(GLOBAL_CAP - placed) {
            next_key += 1;
            service
                .preview_identity_edit(
                    user,
                    work,
                    &next_key.to_string(),
                    Some(AnchorType::new(AnchorType::GR_WORK)),
                )
                .await
                .expect("other users fill the store");
            placed += 1;
        }
    }

    // The store is now globally full and the caller holds 2 of 4 permitted tokens.
    next_key += 1;
    service
        .preview_identity_edit(
            caller,
            caller_work,
            &next_key.to_string(),
            Some(AnchorType::new(AnchorType::GR_WORK)),
        )
        .await
        .expect("a full store must replace the caller's oldest token, not refuse them");
}

// ---------------------------------------------------------------------------
// F4a — a pending row is state whatever its value
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F4a.
///
/// The `clear_identity_slot` trait contract
/// (`crates/livrarr-domain/src/services/work.rs:529-530`) defines an empty slot as "no
/// confirmed row, no nonempty column, and **no pending row**" — presence, not value.
///
/// **This is user-reachable, and no fixture is constructed.** `set_identity_pending`
/// writes a pending `ol_work` row whose `anchor_value` is the empty-string sentinel
/// ("Empty string sentinel for pending anchor_value per IR v2 decision",
/// `crates/livrarr-db/src/sqlite_work_identity.rs:414-418`), and the ordinary add path
/// calls it for any candidate that arrives Pending
/// (`crates/livrarr-metadata/src/work_service.rs:1083`). So adding a book livrarr cannot
/// identify produces exactly this row, through production code, today.
///
/// (A correction worth keeping: `record_pending_anchor` DOES reject empty values, which
/// is why this looked legacy-only at first glance. It is a different writer that creates
/// the sentinel. One writer's validation says nothing about another's.)
///
/// RED on a7f03540: the user clicks clear on that slot, gets `EmptySlot`/404, and the row
/// stays — the slot is permanently unclearable.
#[tokio::test]
async fn f4a_clear_removes_a_pending_row_whose_value_is_empty() {
    let db = create_test_db().await;
    let user = create_test_user(&db).await;
    let work = create_work(&db, user, "Unidentifiable Book").await;

    // The real writer the add path uses when a candidate cannot be identified.
    db.set_identity_pending(work, PendingReason::NoCandidates, AnchorSetter::User)
        .await
        .expect("add path parks the work as Pending");

    let seeded: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = 'ol_work' AND confidence = 'pending' \
           AND anchor_value = ''",
    )
    .bind(work)
    .fetch_one(db.pool())
    .await
    .expect("count sentinel rows");
    assert_eq!(
        seeded, 1,
        "fixture guard: the production writer must have produced the empty sentinel row"
    );

    db.apply_identity_clear(work, user, AnchorType::new(AnchorType::OL_WORK))
        .await
        .expect("a slot holding a pending row is not empty and must clear");

    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = 'ol_work'",
    )
    .bind(work)
    .fetch_one(db.pool())
    .await
    .expect("count remaining rows");
    assert_eq!(left, 0, "clear must delete the pending row it reported on");
}

// ---------------------------------------------------------------------------
// F5b — URL classification must match the real host, not any substring
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F5b.
///
/// RED on a7f03540: `url_segment` lowercases the whole input and tests
/// `contains("goodreads.com")`, so a provider domain appearing anywhere — a query
/// parameter, a path, a fragment — is accepted as if it were the URL's host.
#[test]
fn f5b_a_provider_domain_outside_the_host_does_not_classify() {
    // The domain appears only in a query parameter of an unrelated host.
    let err = classify_identifier_input(
        "https://evil.example/?next=goodreads.com/book/show/12345",
        None,
    );
    assert!(
        err.is_err(),
        "a non-Goodreads host must not classify as Goodreads: {err:?}"
    );

    // Same shape for the other two providers.
    assert!(classify_identifier_input(
        "https://evil.example/?u=openlibrary.org/works/OL123W",
        None
    )
    .is_err());
    assert!(
        classify_identifier_input("https://evil.example/?u=amazon.com/dp/B00TEST123", None)
            .is_err()
    );
}

/// Real provider URLs must keep classifying — the F5b fix must not narrow them.
#[test]
fn f5b_real_provider_urls_still_classify() {
    let (slot, value) =
        classify_identifier_input("https://www.goodreads.com/book/show/12345", None)
            .expect("canonical Goodreads URL");
    assert_eq!(slot.as_str(), AnchorType::GR_WORK);
    assert_eq!(value, "12345");

    let (slot, value) =
        classify_identifier_input("https://openlibrary.org/works/OL123W/Some-Title", None)
            .expect("canonical OpenLibrary URL");
    assert_eq!(slot.as_str(), AnchorType::OL_WORK);
    assert_eq!(value, "OL123W");

    // Regional Amazon domains are in scope; the trailing-dot pattern must survive.
    let (slot, _) = classify_identifier_input("https://www.amazon.co.uk/dp/B00TEST123", None)
        .expect("regional Amazon URL");
    assert_eq!(slot.as_str(), AnchorType::ASIN);

    // A subdomain of the provider is still the provider.
    let (slot, value) = classify_identifier_input("https://m.goodreads.com/book/show/999", None)
        .expect("provider subdomain");
    assert_eq!(slot.as_str(), AnchorType::GR_WORK);
    assert_eq!(value, "999");
}

/// A lookalike domain must not pass as the provider.
#[test]
fn f5b_lookalike_domains_do_not_classify() {
    assert!(
        classify_identifier_input("https://notgoodreads.com/book/show/12345", None).is_err(),
        "a suffix-only lookalike must not classify"
    );
    assert!(
        classify_identifier_input("https://goodreads.com.evil.example/book/show/12345", None)
            .is_err(),
        "the provider domain as a left label of another host must not classify"
    );
}
