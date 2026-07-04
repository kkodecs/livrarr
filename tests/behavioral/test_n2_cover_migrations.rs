//! S3/S4 one-time startup migrations: AC-6 (provenance backfill), AC-7
//! (layout adoption), AC-10 (no fourth road — the migrations replacing the
//! old `run_cover_backfill` never download and never stamp unmeasured dims).

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate};
use livrarr_domain::{normalize_for_matching, CoverTrust, UserRole, Work};
use livrarr_metadata::cover_layout_migration::run_cover_layout_migration;
use livrarr_metadata::cover_provenance_backfill::run_cover_provenance_backfill;

async fn seed_user_and_work(db: &SqliteDb) -> (i64, Work) {
    let user_id = db
        .create_user(CreateUserDbRequest {
            username: "n2-migrations-test".into(),
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

#[tokio::test]
async fn ac6_backfill_derives_source_from_url_host_and_is_idempotent() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;

    // Simulate the diagnosed bug: enrichment won a Goodreads-hosted cover but
    // the row still carries the create-time "add" placeholder.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://i.gr-assets.com/books/x.jpg"),
        "add",
        CoverTrust::Validated,
        640,
        960,
    )
    .await
    .unwrap();

    let report1 = run_cover_provenance_backfill(&db).await;
    assert_eq!(report1.ebook_stamped, 1);

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.cover_source.as_deref(), Some("goodreads"));
    assert_eq!(
        after.cover_trust,
        CoverTrust::Validated,
        "trust is untouched"
    );
    assert_eq!(
        (after.cover_width, after.cover_height),
        (640, 960),
        "dims are untouched"
    );

    // Idempotent: a second run must be a no-op (source is no longer NULL/"add").
    let report2 = run_cover_provenance_backfill(&db).await;
    assert_eq!(report2.ebook_stamped, 0);
}

#[tokio::test]
async fn ac6_backfill_never_overwrites_a_real_provider_source() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;

    // A hardcover-hosted URL, but the source already says "hardcover" (real,
    // meaningful) — must be left exactly as-is even though the host matches.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://assets.hardcover.app/x.jpg"),
        "hardcover",
        CoverTrust::Validated,
        640,
        960,
    )
    .await
    .unwrap();

    let report = run_cover_provenance_backfill(&db).await;
    assert_eq!(report.ebook_stamped, 0);
}

#[tokio::test]
async fn ac6_backfill_skips_user_trust_rows() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://i.gr-assets.com/books/x.jpg"),
        "add",
        CoverTrust::User,
        640,
        960,
    )
    .await
    .unwrap();

    let report = run_cover_provenance_backfill(&db).await;
    assert_eq!(
        report.ebook_stamped, 0,
        "User-trust rows must never be touched"
    );
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.cover_source.as_deref(), Some("add"));
}

#[tokio::test]
async fn ac6_backfill_leaves_unknown_hosts_untouched() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://random-cdn.example.com/x.jpg"),
        "add",
        CoverTrust::Validated,
        640,
        960,
    )
    .await
    .unwrap();

    let report = run_cover_provenance_backfill(&db).await;
    assert_eq!(report.ebook_stamped, 0);
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_source.as_deref(),
        Some("add"),
        "left as-is per the literal rule"
    );
}

#[tokio::test]
async fn ac7_layout_migration_adopts_legacy_root_file_into_user_directory() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();

    let legacy_path = covers_root.path().join(format!("{}.jpg", work.id));
    tokio::fs::write(&legacy_path, b"legacy-ebook-bytes")
        .await
        .unwrap();

    let report = run_cover_layout_migration(&db, covers_root.path()).await;
    assert_eq!(report.adopted, 1);
    assert_eq!(report.orphaned, 0);

    let new_path = covers_root
        .path()
        .join(user_id.to_string())
        .join(format!("{}.jpg", work.id));
    assert!(
        new_path.exists(),
        "file must be adopted into the user directory"
    );
    assert!(
        !legacy_path.exists(),
        "the legacy root copy must be moved, not duplicated"
    );
    assert_eq!(
        tokio::fs::read(&new_path).await.unwrap(),
        b"legacy-ebook-bytes"
    );

    // Idempotent: a second run finds nothing left at the root.
    let report2 = run_cover_layout_migration(&db, covers_root.path()).await;
    assert_eq!(report2.adopted, 0);
}

#[tokio::test]
async fn ac7_layout_migration_renames_legacy_audiobook_suffix_to_audio() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();

    let legacy_path = covers_root
        .path()
        .join(format!("{}_audiobook.jpg", work.id));
    tokio::fs::write(&legacy_path, b"legacy-audiobook-bytes")
        .await
        .unwrap();

    let report = run_cover_layout_migration(&db, covers_root.path()).await;
    assert_eq!(report.adopted, 1);
    assert_eq!(report.legacy_audiobook_suffix_renamed, 1);

    let new_path = covers_root
        .path()
        .join(user_id.to_string())
        .join(format!("{}_audio.jpg", work.id));
    assert!(
        new_path.exists(),
        "must land at the canonical _audio suffix, not _audiobook"
    );
}

#[tokio::test]
async fn ac7_layout_migration_leaves_orphan_files_in_place_never_deletes() {
    let db = create_test_db().await;
    let covers_root = tempfile::tempdir().unwrap();

    // No matching work for id 999999.
    let orphan_path = covers_root.path().join("999999.jpg");
    tokio::fs::write(&orphan_path, b"orphan-bytes")
        .await
        .unwrap();

    let report = run_cover_layout_migration(&db, covers_root.path()).await;
    assert_eq!(report.orphaned, 1);
    assert_eq!(report.adopted, 0);
    assert!(
        orphan_path.exists(),
        "an orphan file must be left in place, never deleted"
    );
}

#[tokio::test]
async fn ac10_provenance_backfill_never_writes_a_cover_file_only_the_source_column() {
    // Structural proof: run_cover_provenance_backfill's signature carries no
    // HttpFetcher and no covers_dir — it is impossible for it to write a
    // cover file. It touches only the already-provenanced source column.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://i.gr-assets.com/x.jpg"),
        "add",
        CoverTrust::Validated,
        640,
        960,
    )
    .await
    .unwrap();

    let before_dims = {
        let w = db.get_work(user_id, work.id).await.unwrap();
        (w.cover_width, w.cover_height)
    };
    run_cover_provenance_backfill(&db).await;
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        (after.cover_width, after.cover_height),
        before_dims,
        "the backfill must never touch measured dims — only cover_source"
    );
}

#[tokio::test]
async fn ac10_layout_migration_never_downloads_only_reorganizes_existing_bytes() {
    // Structural proof: run_cover_layout_migration's signature carries no
    // HttpFetcher — it is impossible for it to download anything. A moved
    // file's bytes are unchanged (verified above); this test asserts the
    // migration makes no DB writes either (it only renames files).
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_root = tempfile::tempdir().unwrap();
    tokio::fs::write(
        covers_root.path().join(format!("{}.jpg", work.id)),
        b"bytes",
    )
    .await
    .unwrap();

    let before = db.get_work(user_id, work.id).await.unwrap();
    run_cover_layout_migration(&db, covers_root.path()).await;
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(before.cover_url, after.cover_url);
    assert_eq!(before.cover_source, after.cover_source);
    assert_eq!(before.cover_trust, after.cover_trust);
}
