//! AC-11: crash recovery at each of the cover-write gate's protocol step
//! boundaries (S2) — after the candidate sidecar write, after a reject
//! decision, after the DB commit, and after the rename — followed by
//! recovery, restores the full invariant: the row describes an existing
//! on-disk file with correct url/source/trust/dims, and no `.candidate.*`
//! file survives.

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate};
use livrarr_domain::{normalize_for_matching, CoverTrust, UserRole, Work};
use livrarr_metadata::cover_write_gate_recovery::recover_pending_cover_writes;

async fn seed_user_and_work(db: &SqliteDb) -> (i64, Work) {
    let user_id = db
        .create_user(CreateUserDbRequest {
            username: "n2-recovery-test".into(),
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
            title: "Test Work".into(),
            author_name: "Test Author".into(),
            normalized_title: normalize_for_matching("Test Work"),
            normalized_author: normalize_for_matching("Test Author"),
            ..Default::default()
        })
        .await
        .unwrap();
    (user_id, work)
}

async fn write_meta(
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

/// Like `write_meta`, but for a user-upload-shaped sidecar, whose `url` is
/// `None` (no source URL to record — this is unrepresentable in the pre-R3
/// sidecar schema, which is exactly the gap the `Option<String>` migration
/// closes).
async fn write_meta_opt_url(
    path: &std::path::Path,
    url: Option<&str>,
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

fn no_candidate_files_survive(user_dir: &std::path::Path, work_id: i64) -> bool {
    let names = [
        format!("{work_id}.candidate.tmp"),
        format!("{work_id}.candidate.meta.json"),
    ];
    names.iter().all(|n| !user_dir.join(n).exists())
}

#[tokio::test]
async fn ac11_boundary1_crash_after_sidecar_write_before_decision_discards_both() {
    // Undecided: meta+tmp exist, row is still the OLD incumbent (never
    // matched the candidate) — nothing was ever accepted.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/incumbent.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    let final_path = user_dir.join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, b"incumbent-bytes")
        .await
        .unwrap();

    let tmp_path = user_dir.join(format!("{}.candidate.tmp", work.id));
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    tokio::fs::write(&tmp_path, b"candidate-bytes")
        .await
        .unwrap();
    write_meta(
        &meta_path,
        "https://new.example/candidate.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.discarded, 1);

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://old.example/incumbent.jpg")
    );
    assert_eq!(after.cover_source.as_deref(), Some("hardcover"));
    assert_eq!((after.cover_width, after.cover_height), (800, 1200));
    assert_eq!(
        tokio::fs::read(&final_path).await.unwrap(),
        b"incumbent-bytes"
    );
    assert!(no_candidate_files_survive(&user_dir, work.id));
}

#[tokio::test]
async fn ac11_boundary2_crash_mid_reject_cleanup_leaves_only_a_harmless_orphan_tmp() {
    // The FIXED cleanup order (meta deleted before tmp) means the only
    // observable interleaving of a crash mid-reject-cleanup is "meta gone,
    // tmp still present" — invisible to recovery (which scans for meta
    // files), and therefore incapable of ever mis-healing the row. This
    // guards the exact bug the ordering fix closes: deleting tmp first
    // would leave "meta present, tmp gone", indistinguishable from a
    // completed ACCEPT's lost rename, and recovery would wrongly overwrite
    // the row with the REJECTED candidate's values.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/incumbent.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    let final_path = user_dir.join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, b"incumbent-bytes")
        .await
        .unwrap();

    // Simulate: reject decided, meta already deleted, crash before tmp's
    // own deletion ran.
    let tmp_path = user_dir.join(format!("{}.candidate.tmp", work.id));
    tokio::fs::write(&tmp_path, b"rejected-candidate-bytes")
        .await
        .unwrap();

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.completed, 0);
    assert_eq!(report.healed, 0);
    assert_eq!(
        report.discarded, 0,
        "no meta file exists for recovery to act on"
    );

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://old.example/incumbent.jpg"),
        "the row must never be healed toward a rejected candidate"
    );
    assert_eq!(
        tokio::fs::read(&final_path).await.unwrap(),
        b"incumbent-bytes"
    );
}

#[tokio::test]
async fn ac11_boundary3_crash_after_db_commit_before_rename_completes_the_rename() {
    // Committed: the DB row already matches the meta sidecar, but the
    // rename never ran — tmp is still the candidate tmp, final path is
    // whatever it was before (possibly absent on a first save).
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://new.example/candidate.jpg"),
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await
    .unwrap();

    let tmp_path = user_dir.join(format!("{}.candidate.tmp", work.id));
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    tokio::fs::write(&tmp_path, b"new-candidate-bytes")
        .await
        .unwrap();
    write_meta(
        &meta_path,
        "https://new.example/candidate.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.completed, 1);

    let final_path = user_dir.join(format!("{}.jpg", work.id));
    assert_eq!(
        tokio::fs::read(&final_path).await.unwrap(),
        b"new-candidate-bytes",
        "recovery must complete the lost rename"
    );
    assert!(no_candidate_files_survive(&user_dir, work.id));
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://new.example/candidate.jpg")
    );
}

#[tokio::test]
async fn ac11_boundary4_crash_after_rename_before_meta_delete_just_deletes_stale_meta() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://new.example/candidate.jpg"),
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await
    .unwrap();
    let final_path = user_dir.join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, b"new-candidate-bytes")
        .await
        .unwrap();

    // tmp is already gone (rename succeeded); only the stale meta remains.
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    write_meta(
        &meta_path,
        "https://new.example/candidate.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.healed, 0, "row already matches meta — a no-op heal");
    assert_eq!(report.completed, 0);
    assert!(no_candidate_files_survive(&user_dir, work.id));
    assert_eq!(
        tokio::fs::read(&final_path).await.unwrap(),
        b"new-candidate-bytes",
        "the already-correct final file must be untouched"
    );
}

#[tokio::test]
async fn ac11_heals_row_when_tmp_is_gone_but_row_disagrees_with_meta() {
    // tmp gone (rename ran) but the row doesn't match meta — provenance
    // must converge to the meta sidecar's values (source/trust/dims, not
    // dims alone), and the stale meta is removed.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    // Row still shows the OLD pre-commit values (simulating the DB commit's
    // own write having been lost/reverted independently of the file rename).
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/incumbent.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    let final_path = user_dir.join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, b"new-candidate-bytes")
        .await
        .unwrap();

    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    write_meta(
        &meta_path,
        "https://new.example/candidate.jpg",
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.healed, 1);

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://new.example/candidate.jpg")
    );
    assert_eq!(after.cover_source.as_deref(), Some("goodreads"));
    assert_eq!(after.cover_trust, CoverTrust::Validated);
    assert_eq!((after.cover_width, after.cover_height), (500, 700));
    assert!(no_candidate_files_survive(&user_dir, work.id));
}

#[tokio::test]
async fn ac11_orphan_work_candidate_is_discarded() {
    let db = create_test_db().await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join("1");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    let tmp_path = user_dir.join("999999.candidate.tmp");
    let meta_path = user_dir.join("999999.candidate.meta.json");
    tokio::fs::write(&tmp_path, b"orphan-bytes").await.unwrap();
    write_meta(
        &meta_path,
        "https://x/y.jpg",
        "goodreads",
        CoverTrust::Validated,
        1,
        1,
    )
    .await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.orphaned, 1);
    assert!(!tmp_path.exists());
    assert!(!meta_path.exists());
}

// =============================================================================
// R3 — recovery for user-initiated writes (url: None sidecars)
// =============================================================================

#[tokio::test]
async fn user_upload_heals_row_to_none_url_and_user_trust_when_tmp_is_gone() {
    // tmp gone (rename already ran) but the row disagrees with the meta —
    // healing a user-upload-shaped candidate (url: None) must land `None`
    // on the row, not a fabricated URL, and must stamp User trust.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    // Row still shows the OLD pre-commit incumbent (simulating the DB
    // commit's own write having been lost independently of the rename).
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/incumbent.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    let final_path = user_dir.join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, b"uploaded-jpeg-bytes")
        .await
        .unwrap();

    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    write_meta_opt_url(&meta_path, None, "user_upload", CoverTrust::User, 400, 600).await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.healed, 1);

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url, None,
        "healing a url-less upload candidate must not fabricate a URL"
    );
    assert_eq!(after.cover_source.as_deref(), Some("user_upload"));
    assert_eq!(after.cover_trust, CoverTrust::User);
    assert_eq!((after.cover_width, after.cover_height), (400, 600));
    assert!(
        after.cover_manual,
        "the heal goes through the same update_cover_metadata that derives cover_manual from trust"
    );
    assert!(no_candidate_files_survive(&user_dir, work.id));
}

#[tokio::test]
async fn user_upload_candidate_is_discarded_when_crash_landed_before_db_commit() {
    // meta+tmp exist (url: None, a user-upload-shaped candidate), but the
    // row is still the OLD incumbent — never matched the candidate, so
    // nothing was ever accepted. Recovery must discard, not fabricate.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    let user_dir = covers_root.path().join(user_id.to_string());
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://old.example/incumbent.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    let final_path = user_dir.join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, b"incumbent-bytes")
        .await
        .unwrap();

    let tmp_path = user_dir.join(format!("{}.candidate.tmp", work.id));
    let meta_path = user_dir.join(format!("{}.candidate.meta.json", work.id));
    tokio::fs::write(&tmp_path, b"pending-upload-bytes")
        .await
        .unwrap();
    write_meta_opt_url(&meta_path, None, "user_upload", CoverTrust::User, 400, 600).await;

    let report = recover_pending_cover_writes(&db, covers_root.path()).await;
    assert_eq!(report.discarded, 1);

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://old.example/incumbent.jpg"),
        "an undecided user-upload candidate must never be healed onto the row"
    );
    assert_eq!(after.cover_source.as_deref(), Some("hardcover"));
    assert_eq!(
        tokio::fs::read(&final_path).await.unwrap(),
        b"incumbent-bytes"
    );
    assert!(no_candidate_files_survive(&user_dir, work.id));
}
