//! Review-round fixes for the cover pipeline consolidation:
//! - T-A: startup passes run strictly in order (layout migration ->
//!   gate recovery -> provenance backfill).
//! - T-B: a transient DB error during recovery must not discard a pending
//!   candidate (only a definitive work-not-found may).
//! - T-C: two gate runs for the same (user, work, slot) are serialized; the
//!   second sees the first's committed state instead of corrupting the
//!   shared candidate files.
//! - T-F: recovery invalidates stale thumbnails when it completes or heals
//!   a cover write.
//! - T-G: the provenance backfill maps amazon-family hosts per slot —
//!   goodreads for the ebook slot, audible for the audiobook slot.
//! - T-E: regression lock — a cover-only enrichment (no text-field change)
//!   still fires the tag write with the accepted cover bytes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    ApplyEnrichmentMergeRequest, CreateUserDbRequest, CreateWorkDbRequest,
    UpdateWorkEnrichmentDbRequest, UpdateWorkUserFieldsDbRequest, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    normalize_for_matching, ApplyMergeOutcome, CoverMediaType, CoverResolution, CoverTrust,
    DbError, MediaType, UserId, UserRole, Work, WorkId,
};
use tokio::sync::Notify;

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

async fn seed_user_and_work(db: &SqliteDb) -> (i64, Work) {
    let user_id = db
        .create_user(CreateUserDbRequest {
            username: "n2-review-fixes".into(),
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
            title: "Review Fix Work".into(),
            author_name: "Review Author".into(),
            normalized_title: normalize_for_matching("Review Fix Work"),
            normalized_author: normalize_for_matching("Review Author"),
            language: Some("en".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    (user_id, work)
}

async fn write_candidate_meta(
    path: &std::path::Path,
    url: &str,
    source: &str,
    trust: CoverTrust,
    w: i32,
    h: i32,
) {
    let json = serde_json::json!({
        "url": url,
        "source": source,
        "trust": match trust {
            CoverTrust::Unvalidated => "unvalidated",
            CoverTrust::Validated => "validated",
            CoverTrust::User => "user",
        },
        "width": w,
        "height": h,
    });
    tokio::fs::write(path, serde_json::to_vec(&json).unwrap())
        .await
        .unwrap();
}

/// WorkDb wrapper around a real SqliteDb with two failure/interleaving knobs:
/// - `fail_get_work`: every `get_work` returns a transient-class (non-NotFound)
///   error while set.
/// - `park_first_cover_update`: the FIRST `update_cover_metadata` call signals
///   `reached_commit` and then waits for `release` before delegating — lets a
///   test freeze one gate run at its commit step while another runs.
#[derive(Clone)]
struct HookedWorkDb {
    inner: SqliteDb,
    fail_get_work: Arc<AtomicBool>,
    park_armed: Arc<AtomicBool>,
    reached_commit: Arc<Notify>,
    release: Arc<Notify>,
}

impl HookedWorkDb {
    fn new(inner: SqliteDb) -> Self {
        Self {
            inner,
            fail_get_work: Arc::new(AtomicBool::new(false)),
            park_armed: Arc::new(AtomicBool::new(false)),
            reached_commit: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }

    fn transient_error() -> DbError {
        DbError::Io("simulated transient pool error".into())
    }
}

impl WorkDb for HookedWorkDb {
    async fn get_work(&self, user_id: UserId, work_id: WorkId) -> Result<Work, DbError> {
        if self.fail_get_work.load(Ordering::SeqCst) {
            return Err(Self::transient_error());
        }
        self.inner.get_work(user_id, work_id).await
    }

    async fn update_cover_metadata(
        &self,
        user_id: UserId,
        work_id: WorkId,
        cover_url: Option<&str>,
        cover_source: &str,
        cover_trust: CoverTrust,
        cover_width: i32,
        cover_height: i32,
    ) -> Result<(), DbError> {
        if self.park_armed.swap(false, Ordering::SeqCst) {
            self.reached_commit.notify_one();
            self.release.notified().await;
        }
        self.inner
            .update_cover_metadata(
                user_id,
                work_id,
                cover_url,
                cover_source,
                cover_trust,
                cover_width,
                cover_height,
            )
            .await
    }

    async fn update_audiobook_cover_metadata(
        &self,
        user_id: UserId,
        work_id: WorkId,
        audiobook_cover_url: Option<&str>,
        audiobook_cover_source: &str,
        audiobook_cover_trust: CoverTrust,
        audiobook_cover_width: i32,
        audiobook_cover_height: i32,
    ) -> Result<(), DbError> {
        self.inner
            .update_audiobook_cover_metadata(
                user_id,
                work_id,
                audiobook_cover_url,
                audiobook_cover_source,
                audiobook_cover_trust,
                audiobook_cover_width,
                audiobook_cover_height,
            )
            .await
    }

    async fn update_cover_dimensions(
        &self,
        user_id: UserId,
        work_id: WorkId,
        width: i32,
        height: i32,
    ) -> Result<(), DbError> {
        self.inner
            .update_cover_dimensions(user_id, work_id, width, height)
            .await
    }

    async fn update_audiobook_cover_dimensions(
        &self,
        user_id: UserId,
        work_id: WorkId,
        width: i32,
        height: i32,
    ) -> Result<(), DbError> {
        self.inner
            .update_audiobook_cover_dimensions(user_id, work_id, width, height)
            .await
    }

    async fn list_convergence_due(
        &self,
        user_id: UserId,
        now: chrono::DateTime<chrono::Utc>,
        threshold: u32,
        limit: i64,
    ) -> Result<Vec<WorkId>, DbError> {
        self.inner
            .list_convergence_due(user_id, now, threshold, limit)
            .await
    }

    async fn set_next_convergence_at(
        &self,
        user_id: UserId,
        work_id: WorkId,
        at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), DbError> {
        self.inner
            .set_next_convergence_at(user_id, work_id, at)
            .await
    }

    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        self.inner.list_works(user_id).await
    }

    async fn list_works_by_author(
        &self,
        user_id: UserId,
        author_id: livrarr_db::AuthorId,
    ) -> Result<Vec<Work>, DbError> {
        self.inner.list_works_by_author(user_id, author_id).await
    }

    async fn list_works_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
        sort_by: &str,
        sort_dir: &str,
        media_type: Option<MediaType>,
        language: Option<&str>,
    ) -> Result<(Vec<Work>, i64), DbError> {
        self.inner
            .list_works_paginated(
                user_id, page, per_page, sort_by, sort_dir, media_type, language,
            )
            .await
    }

    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError> {
        self.inner.update_work_enrichment(user_id, id, req).await
    }

    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError> {
        self.inner
            .update_work_user_fields(user_id, work_id, req)
            .await
    }

    async fn set_cover_manual(
        &self,
        user_id: UserId,
        id: WorkId,
        manual: bool,
    ) -> Result<(), DbError> {
        self.inner.set_cover_manual(user_id, id, manual).await
    }

    async fn set_identity_status(
        &self,
        user_id: UserId,
        id: WorkId,
        status: livrarr_domain::IdentityStatus,
    ) -> Result<(), DbError> {
        self.inner.set_identity_status(user_id, id, status).await
    }

    async fn delete_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError> {
        self.inner.delete_work(user_id, id).await
    }

    async fn merge_works(&self, req: livrarr_db::MergeWorksDbRequest) -> Result<Work, DbError> {
        self.inner.merge_works(req).await
    }

    async fn set_work_series_id(
        &self,
        user_id: UserId,
        work_id: WorkId,
        series_id: Option<i64>,
    ) -> Result<(), DbError> {
        self.inner
            .set_work_series_id(user_id, work_id, series_id)
            .await
    }

    async fn normalize_work_series_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        series_name: &str,
        series_position: Option<f64>,
    ) -> Result<(), DbError> {
        self.inner
            .normalize_work_series_fields(user_id, work_id, series_name, series_position)
            .await
    }

    async fn list_orphan_series_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        self.inner.list_orphan_series_works_all_users().await
    }

    async fn work_exists_by_ol_key(&self, user_id: UserId, ol_key: &str) -> Result<bool, DbError> {
        self.inner.work_exists_by_ol_key(user_id, ol_key).await
    }

    async fn list_works_for_enrichment(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        self.inner.list_works_for_enrichment(user_id).await
    }

    async fn list_works_by_author_ol_keys(
        &self,
        user_id: UserId,
        author_ol_key: &str,
    ) -> Result<Vec<String>, DbError> {
        self.inner
            .list_works_by_author_ol_keys(user_id, author_ol_key)
            .await
    }

    async fn find_by_normalized_match(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
    ) -> Result<Vec<Work>, DbError> {
        self.inner
            .find_by_normalized_match(user_id, title, author)
            .await
    }

    async fn list_monitored_works_all_users(&self) -> Result<Vec<Work>, DbError> {
        self.inner.list_monitored_works_all_users().await
    }

    async fn list_work_owners_all_users(&self) -> Result<Vec<(WorkId, UserId)>, DbError> {
        self.inner.list_work_owners_all_users().await
    }

    async fn list_stale_unenriched_works(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Work>, DbError> {
        self.inner.list_stale_unenriched_works(older_than).await
    }

    async fn list_failed_works_without_retry_state(&self) -> Result<Vec<Work>, DbError> {
        self.inner.list_failed_works_without_retry_state().await
    }

    async fn apply_enrichment_merge(
        &self,
        req: ApplyEnrichmentMergeRequest,
    ) -> Result<ApplyMergeOutcome, DbError> {
        self.inner.apply_enrichment_merge(req).await
    }

    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        self.inner.reset_for_manual_refresh(user_id, work_id).await
    }

    async fn get_merge_generation(&self, user_id: UserId, work_id: WorkId) -> Result<i64, DbError> {
        self.inner.get_merge_generation(user_id, work_id).await
    }

    async fn list_conflict_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        self.inner.list_conflict_works(user_id).await
    }

    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError> {
        self.inner
            .search_works(user_id, query, page, per_page)
            .await
    }

    async fn list_work_provider_keys_by_author(
        &self,
        user_id: UserId,
        author_id: i64,
    ) -> Result<Vec<(Option<String>, Option<String>)>, DbError> {
        self.inner
            .list_work_provider_keys_by_author(user_id, author_id)
            .await
    }

    async fn find_normalized_match_no_anchor_for_user(
        &self,
        user_id: UserId,
        raw_title: &str,
        raw_author: &str,
    ) -> Result<Option<Work>, DbError> {
        self.inner
            .find_normalized_match_no_anchor_for_user(user_id, raw_title, raw_author)
            .await
    }

    async fn find_works_by_bridge(
        &self,
        user_id: UserId,
        isbn_13: Option<&str>,
        asin: Option<&str>,
    ) -> Result<Vec<Work>, DbError> {
        self.inner
            .find_works_by_bridge(user_id, isbn_13, asin)
            .await
    }

    async fn list_identity_pending_works(&self) -> Result<Vec<Work>, DbError> {
        self.inner.list_identity_pending_works().await
    }
}

// =============================================================================
// T-A: startup passes run strictly in order
// =============================================================================

#[tokio::test]
async fn t_a_startup_sequence_runs_migration_then_recovery_then_backfill() {
    // Order-sensitive by construction, with real passes:
    // - A legacy root-level {W}.jpg AND a committed-but-unrenamed candidate
    //   (row == meta, tmp pending) coexist for the same work. Migration must
    //   run FIRST: it adopts the legacy file into the user directory while
    //   the final path is still free; recovery then completes the pending
    //   rename over it (the row says the candidate bytes are current). Run
    //   the other way around, recovery's rename claims the final path first
    //   and migration then refuses to overwrite it — the legacy root file
    //   would survive, which this test rejects.
    // - A second work with a placeholder source proves the backfill pass
    //   also ran within the same sequence.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    let candidate_bytes = fake_jpeg(500, 700);
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://new.example/committed.jpg"),
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await
    .unwrap();
    tokio::fs::write(
        user_dir.join(format!("{}.candidate.tmp", work.id)),
        &candidate_bytes,
    )
    .await
    .unwrap();
    write_candidate_meta(
        &user_dir.join(format!("{}.candidate.meta.json", work.id)),
        "https://new.example/committed.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;
    let legacy_root_file = covers_root.path().join(format!("{}.jpg", work.id));
    tokio::fs::write(&legacy_root_file, b"legacy-root-bytes")
        .await
        .unwrap();

    let (work2, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Backfill Pass Probe".into(),
            author_name: "Review Author".into(),
            normalized_title: normalize_for_matching("Backfill Pass Probe"),
            normalized_author: normalize_for_matching("Review Author"),
            ..Default::default()
        })
        .await
        .unwrap();
    db.update_cover_metadata(
        user_id,
        work2.id,
        Some("https://i.gr-assets.com/probe.jpg"),
        "add",
        CoverTrust::Validated,
        600,
        900,
    )
    .await
    .unwrap();

    livrarr_metadata::cover_startup::run_cover_startup_passes(&db, covers_root.path()).await;

    assert!(
        !legacy_root_file.exists(),
        "migration must have run BEFORE recovery — with the order flipped, \
         recovery claims the final path first and the legacy root file is \
         left behind"
    );
    let final_bytes = tokio::fs::read(user_dir.join(format!("{}.jpg", work.id)))
        .await
        .unwrap();
    assert_eq!(
        final_bytes, candidate_bytes,
        "the committed candidate must win the final path (the row describes \
         it); the adopted legacy bytes are superseded by the completed rename"
    );
    assert!(!user_dir.join(format!("{}.candidate.tmp", work.id)).exists());
    assert!(!user_dir
        .join(format!("{}.candidate.meta.json", work.id))
        .exists());

    let probe = db.get_work(user_id, work2.id).await.unwrap();
    assert_eq!(
        probe.cover_source.as_deref(),
        Some("goodreads"),
        "the provenance backfill pass must have run as part of the sequence"
    );
}

// =============================================================================
// T-B: transient DB error during recovery must not discard candidate files
// =============================================================================

#[tokio::test]
async fn t_b_transient_get_work_error_leaves_candidate_files_and_row_untouched() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    // A committed-but-unrenamed candidate: row == meta, tmp still pending.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://new.example/committed.jpg"),
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await
    .unwrap();
    let tmp_path = user_dir.join(format!("{}.candidate.tmp", work.id));
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    tokio::fs::write(&tmp_path, b"committed-candidate-bytes")
        .await
        .unwrap();
    write_candidate_meta(
        &meta_path,
        "https://new.example/committed.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;

    // Every get_work now fails with a TRANSIENT error (not NotFound) — e.g.
    // the pool is briefly saturated at startup.
    let hooked = HookedWorkDb::new(db.clone());
    hooked.fail_get_work.store(true, Ordering::SeqCst);

    let report = livrarr_metadata::cover_write_gate_recovery::recover_pending_cover_writes(
        &hooked,
        covers_root.path(),
    )
    .await;

    assert!(
        tmp_path.exists(),
        "a transient DB error must NOT discard the committed candidate tmp — \
         only a definitive work-not-found may"
    );
    assert!(
        meta_path.exists(),
        "a transient DB error must NOT discard the candidate meta sidecar"
    );
    assert_eq!(report.orphaned, 0, "a transient error is not an orphan");
    assert_eq!(
        report.skipped, 1,
        "the candidate is skipped for a later retry"
    );

    // The row was never touched (verified through the un-hooked handle).
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://new.example/committed.jpg")
    );

    // And once the DB recovers, the next pass completes the rename normally.
    hooked.fail_get_work.store(false, Ordering::SeqCst);
    let report2 = livrarr_metadata::cover_write_gate_recovery::recover_pending_cover_writes(
        &hooked,
        covers_root.path(),
    )
    .await;
    assert_eq!(report2.completed, 1);
    assert!(user_dir.join(format!("{}.jpg", work.id)).exists());
}

// =============================================================================
// T-C: same-(user, work, slot) gate runs are serialized
// =============================================================================

#[tokio::test]
async fn t_c_two_gate_runs_for_one_slot_are_serialized_not_interleaved() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    let bytes_a = fake_jpeg(500, 700);
    let bytes_b = fake_jpeg(800, 1200);

    // Fetch order is deterministic: run A starts first (and is then parked at
    // its DB commit), so it pops response 1; run B pops response 2.
    let http = StubHttpFetcher::with_ok(200, bytes_a.clone());
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: bytes_b.clone(),
    }));

    let hooked = HookedWorkDb::new(db.clone());
    hooked.park_armed.store(true, Ordering::SeqCst);

    let gate_a = {
        let hooked = hooked.clone();
        let http = http.clone();
        let covers_dir = covers_dir.path().to_path_buf();
        let work_id = work.id;
        tokio::spawn(async move {
            livrarr_metadata::cover_write_gate::run_cover_write_gate(
                &hooked,
                &http,
                user_id,
                livrarr_metadata::cover_write_gate::CoverWriteGateInput {
                    covers_dir,
                    work_id,
                    media_type: CoverMediaType::Ebook,
                    resolution: CoverResolution {
                        url: "https://i.gr-assets.com/candidate-a.jpg".into(),
                        source: "goodreads".into(),
                        trust: CoverTrust::Validated,
                        media_type: CoverMediaType::Ebook,
                    },
                },
            )
            .await
        })
    };

    // Run A is now frozen at its DB-commit step, holding the slot mid-protocol.
    hooked.reached_commit.notified().await;

    let gate_b = {
        let hooked = hooked.clone();
        let http = http.clone();
        let covers_dir = covers_dir.path().to_path_buf();
        let work_id = work.id;
        tokio::spawn(async move {
            livrarr_metadata::cover_write_gate::run_cover_write_gate(
                &hooked,
                &http,
                user_id,
                livrarr_metadata::cover_write_gate::CoverWriteGateInput {
                    covers_dir,
                    work_id,
                    media_type: CoverMediaType::Ebook,
                    resolution: CoverResolution {
                        url: "https://assets.hardcover.app/candidate-b.jpg".into(),
                        source: "hardcover".into(),
                        trust: CoverTrust::Validated,
                        media_type: CoverMediaType::Ebook,
                    },
                },
            )
            .await
        })
    };

    // Give run B time to make progress: without serialization it tramples
    // run A's shared {id}.candidate.* files and completes its own protocol
    // while A is still mid-flight.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Release A; both runs finish.
    hooked.release.notify_one();
    let outcome_a = gate_a.await.unwrap();
    let outcome_b = gate_b.await.unwrap();

    assert!(
        outcome_a.is_accepted(),
        "run A had no incumbent at its decision point — it must commit"
    );
    assert!(
        matches!(
            outcome_b,
            livrarr_metadata::cover_write_gate::GateOutcome::Rejected
        ),
        "run B must observe run A's committed state (goodreads outranks \
         hardcover at the same trust/quality tier) and be rejected — \
         got {outcome_b:?}"
    );

    // The binding invariant survived the race: the row describes the bytes
    // actually on disk.
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://i.gr-assets.com/candidate-a.jpg")
    );
    assert_eq!((after.cover_width, after.cover_height), (500, 700));
    let final_bytes = tokio::fs::read(covers_dir.path().join(format!("{}.jpg", work.id)))
        .await
        .unwrap();
    assert_eq!(
        final_bytes, bytes_a,
        "the file on disk must be the committed run's bytes — an interleaved \
         run must never leave the row describing one image and the disk \
         holding another"
    );
    assert!(!covers_dir
        .path()
        .join(format!("{}.candidate.tmp", work.id))
        .exists());
    assert!(!covers_dir
        .path()
        .join(format!("{}.candidate.meta.json", work.id))
        .exists());
}

// =============================================================================
// T-F: recovery invalidates stale thumbnails when it completes or heals
// =============================================================================

#[tokio::test]
async fn t_f_recovery_completing_a_rename_invalidates_the_stale_thumbnail() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    // Committed-but-unrenamed candidate; a stale thumbnail of the OLD art is
    // still on disk (the crashed gate never reached its invalidation step).
    // The work has no audiobook cover of its own, so the audiobook serving
    // route falls back to the ebook art — its fallback thumb is stale too.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://new.example/committed.jpg"),
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await
    .unwrap();
    let tmp_path = user_dir.join(format!("{}.candidate.tmp", work.id));
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    tokio::fs::write(&tmp_path, fake_jpeg(500, 700))
        .await
        .unwrap();
    write_candidate_meta(
        &meta_path,
        "https://new.example/committed.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;
    let thumb_path = user_dir.join(format!("{}_thumb.jpg", work.id));
    let audio_fallback_thumb = user_dir.join(format!("{}_audio_thumb.jpg", work.id));
    tokio::fs::write(&thumb_path, b"stale-old-art-thumb")
        .await
        .unwrap();
    tokio::fs::write(&audio_fallback_thumb, b"stale-old-art-audio-thumb")
        .await
        .unwrap();

    let report = livrarr_metadata::cover_write_gate_recovery::recover_pending_cover_writes(
        &db,
        covers_root.path(),
    )
    .await;
    assert_eq!(report.completed, 1);

    assert!(
        !thumb_path.exists(),
        "recovery replaced the cover file — the stale thumbnail must be \
         invalidated exactly as the gate's own accept path does"
    );
    assert!(
        !audio_fallback_thumb.exists(),
        "the work has no audiobook cover, so the audiobook fallback thumb \
         renders the ebook art — it must be invalidated too"
    );
}

#[tokio::test]
async fn t_f_recovery_healing_a_row_invalidates_the_stale_thumbnail() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    // Rename already ran (tmp gone, new art at the final path), the DB
    // commit was lost (row still describes the old cover), and the old
    // art's thumbnail is still on disk.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/previous.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    tokio::fs::write(
        user_dir.join(format!("{}.jpg", work.id)),
        fake_jpeg(500, 700),
    )
    .await
    .unwrap();
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    write_candidate_meta(
        &meta_path,
        "https://new.example/healed.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;
    let thumb_path = user_dir.join(format!("{}_thumb.jpg", work.id));
    tokio::fs::write(&thumb_path, b"stale-old-art-thumb")
        .await
        .unwrap();

    let report = livrarr_metadata::cover_write_gate_recovery::recover_pending_cover_writes(
        &db,
        covers_root.path(),
    )
    .await;
    assert_eq!(report.healed, 1);

    assert!(
        !thumb_path.exists(),
        "the healed row now describes the new art — the old art's thumbnail \
         must not keep serving"
    );
}

// =============================================================================
// T-E: cover-only enrichment still fires the tag write with the accepted bytes
// =============================================================================

const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const MINIMAL_OPF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Placeholder</dc:title>
  </metadata>
  <manifest>
  </manifest>
  <spine>
  </spine>
</package>"#;

fn write_minimal_epub(path: &std::path::Path) {
    use std::io::Write;
    let file = std::fs::File::create(path).expect("create epub");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("META-INF/container.xml", opts)
        .expect("start container");
    zip.write_all(CONTAINER_XML.as_bytes()).expect("container");
    zip.start_file("OEBPS/content.opf", opts)
        .expect("start opf");
    zip.write_all(MINIMAL_OPF.as_bytes()).expect("opf");
    zip.finish().expect("finish epub");
}

fn read_epub_cover(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open epub");
    let mut archive = zip::ZipArchive::new(file).expect("read epub zip");
    let mut entry = archive.by_name("OEBPS/images/cover.jpg").ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).expect("read cover entry");
    Some(buf)
}

/// A provider queue whose single Goodreads result carries ONLY a cover URL —
/// no text field. The merge, the `changed` computation, the gate, and the
/// retag all run REAL.
struct CoverOnlyQueue {
    cover_url: String,
}

impl livrarr_metadata::ProviderQueue for CoverOnlyQueue {
    async fn dispatch_enrichment(
        &self,
        work: &Work,
        _context: livrarr_metadata::EnrichmentContext,
    ) -> Result<livrarr_metadata::ScatterGatherResult, livrarr_metadata::ProviderQueueError> {
        let mut outcomes = std::collections::HashMap::new();
        outcomes.insert(
            livrarr_domain::MetadataProvider::Goodreads,
            livrarr_external_data::ProviderOutcome::Success(Box::new(
                livrarr_external_data::NormalizedWorkDetail {
                    cover_url: Some(self.cover_url.clone()),
                    ..Default::default()
                },
            )),
        );
        Ok(livrarr_metadata::ScatterGatherResult {
            work_id: work.id,
            outcomes,
            merge_eligible: true,
            deferred: false,
        })
    }
}

#[tokio::test]
async fn t_e_cover_only_change_through_real_enrichment_retags_with_the_accepted_bytes() {
    // An enrichment pass whose ONLY yield is a cover resolution must still
    // fire the tag write, and the bytes embedded into the book file must be
    // the gate-accepted cover bytes — materialize's prefetched-bytes
    // fallback applies even when no download URL is passed.
    use livrarr_db::{CreateLibraryItemDbRequest, LibraryItemDb, RootFolderDb};
    use livrarr_domain::services::{RefreshSurface, WorkService};
    use livrarr_domain::TagStatus;

    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    db.set_identity_status(user_id, work.id, livrarr_domain::IdentityStatus::Confirmed)
        .await
        .unwrap();

    let data_dir = tempfile::tempdir().unwrap();
    let books_dir = data_dir.path().join("books");
    tokio::fs::create_dir_all(&books_dir).await.unwrap();
    let epub_path = books_dir.join("cover-only.epub");
    write_minimal_epub(&epub_path);
    assert_eq!(
        read_epub_cover(&epub_path),
        None,
        "the fixture epub must start with no embedded cover"
    );

    let root = db
        .create_root_folder(books_dir.to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id: work.id,
        root_folder_id: root.id,
        path: epub_path.to_string_lossy().into_owned(),
        media_type: MediaType::Ebook,
        file_size: 1024,
        import_id: None,
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();

    let cover_jpeg = fake_jpeg(640, 960);
    let http = StubHttpFetcher::with_ok(200, cover_jpeg.clone());

    let enrichment = livrarr_metadata::EnrichmentServiceImpl::new(
        Arc::new(db.clone()),
        Arc::new(CoverOnlyQueue {
            cover_url: "https://i.gr-assets.com/only-cover.jpg".into(),
        }),
        Arc::new(livrarr_metadata::DefaultMergeEngine::new(
            livrarr_metadata::PriorityModel::english(),
        )),
        false,
    );
    let workflow = livrarr_metadata::enrichment_workflow_service::EnrichmentWorkflowImpl::new(
        Arc::new(enrichment),
        db.clone(),
    );
    let svc = livrarr_metadata::work_service::WorkServiceImpl::new(
        db.clone(),
        workflow,
        http,
        data_dir.path().to_path_buf(),
    );

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh should succeed");

    // The REAL EnrichmentResult carried the cover resolution through the
    // gate: the row describes the accepted bytes...
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://i.gr-assets.com/only-cover.jpg")
    );
    assert_eq!(after.cover_source.as_deref(), Some("goodreads"));
    assert_eq!((after.cover_width, after.cover_height), (640, 960));

    // ...and the retag embedded those SAME bytes into the book file — the
    // write_tags_batch call received the accepted cover, not None.
    let embedded = read_epub_cover(&epub_path)
        .expect("the epub must carry an embedded cover after the cover-only refresh");
    assert_eq!(
        embedded, cover_jpeg,
        "the bytes embedded into the file must be the gate-accepted cover bytes"
    );
}

// =============================================================================
// T-G: provenance backfill maps amazon-family hosts per slot
// =============================================================================

#[tokio::test]
async fn t_g_amazon_host_backfills_goodreads_for_ebook_and_audible_for_audiobook() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;

    // The SAME amazon-family asset URL sits in both slots with the
    // create-time placeholder source.
    let amazon_url = "https://m.media-amazon.com/images/I/81Nzlrfud+L.jpg";
    db.update_cover_metadata(
        user_id,
        work.id,
        Some(amazon_url),
        "add",
        CoverTrust::Validated,
        600,
        900,
    )
    .await
    .unwrap();
    db.update_audiobook_cover_metadata(
        user_id,
        work.id,
        Some(amazon_url),
        "add",
        CoverTrust::Validated,
        1000,
        1000,
    )
    .await
    .unwrap();

    livrarr_metadata::cover_provenance_backfill::run_cover_provenance_backfill(&db).await;

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_source.as_deref(),
        Some("goodreads"),
        "ebook slot: amazon-family hosts carry Goodreads art in this system"
    );
    assert_eq!(
        after.audiobook_cover_source.as_deref(),
        Some("audible"),
        "audiobook slot: amazon-family hosts carry Audible/Audnexus art — \
         stamping goodreads here would mis-rank future comparator tiebreaks"
    );
}
