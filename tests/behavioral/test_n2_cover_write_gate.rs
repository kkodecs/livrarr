//! S2 cover write gate: AC-2 (rescue), AC-3 (desync guard), AC-4
//! (sovereignty), AC-5 (provenance), AC-8 (media-slot integrity), AC-9
//! (suffix unification). Exercises `run_cover_write_gate` directly against a
//! real SQLite `:memory:` DB, a real tempdir, and a stub HTTP fetcher — the
//! gate's own boundary, independent of the full `WorkServiceImpl` stack.

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateUserDbRequest, CreateWorkDbRequest, UserDb, WorkDb, WorkDbCreate};
use livrarr_domain::{
    normalize_for_matching, CoverMediaType, CoverResolution, CoverTrust, UserRole, Work,
};
use livrarr_metadata::cover_write_gate::{
    run_cover_write_gate, run_user_cover_write, CandidateMeta, CoverWriteGateInput, GateOutcome,
    UserCoverError, UserCoverInput, UserCoverPayload,
};

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
    // Each test opens its own fresh :memory: DB via create_test_db(), so a
    // fixed username never collides across tests.
    let user_id = db
        .create_user(CreateUserDbRequest {
            username: "n2-gate-test".into(),
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

fn ebook_resolution(url: &str, source: &str, trust: CoverTrust) -> CoverResolution {
    CoverResolution {
        url: url.to_string(),
        source: source.to_string(),
        trust,
        media_type: CoverMediaType::Ebook,
    }
}

#[tokio::test]
async fn ac2_rescue_incumbent_below_floor_is_replaced_by_bigger_same_trust_candidate() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://i.gr-assets.com/old.jpg"),
        "goodreads",
        CoverTrust::Validated,
        300,
        400,
    )
    .await
    .unwrap();
    let final_path = covers_dir.path().join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, fake_jpeg(300, 400))
        .await
        .unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(500, 700));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://i.gr-assets.com/new.jpg",
                "goodreads",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(
        matches!(
            outcome,
            GateOutcome::Accepted {
                width: 500,
                height: 700,
                ..
            }
        ),
        "below-floor incumbent with an above-floor same-trust candidate must be rescued"
    );
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://i.gr-assets.com/new.jpg")
    );
    assert_eq!(after.cover_source.as_deref(), Some("goodreads"));
    assert_eq!(after.cover_trust, CoverTrust::Validated);
    assert_eq!((after.cover_width, after.cover_height), (500, 700));
    let bytes_on_disk = tokio::fs::read(&final_path).await.unwrap();
    assert_eq!(bytes_on_disk, fake_jpeg(500, 700));
    assert!(!covers_dir
        .path()
        .join(format!("{}.candidate.tmp", work.id))
        .exists());
    assert!(!covers_dir
        .path()
        .join(format!("{}.candidate.meta.json", work.id))
        .exists());
}

#[tokio::test]
async fn ac3_desync_guard_rejected_candidate_leaves_file_and_row_untouched() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    let incumbent_bytes = fake_jpeg(800, 1200);
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://assets.hardcover.app/old.jpg"),
        "hardcover",
        CoverTrust::Validated,
        800,
        1200,
    )
    .await
    .unwrap();
    let final_path = covers_dir.path().join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, &incumbent_bytes)
        .await
        .unwrap();

    // Same trust, but smaller (below floor) — must lose regardless of rank.
    let http = StubHttpFetcher::with_ok(200, fake_jpeg(200, 300));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://covers.openlibrary.org/new.jpg",
                "openlibrary",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(matches!(outcome, GateOutcome::Rejected));
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://assets.hardcover.app/old.jpg"),
        "a rejected candidate's URL must never persist on the row"
    );
    assert_eq!(after.cover_source.as_deref(), Some("hardcover"));
    assert_eq!((after.cover_width, after.cover_height), (800, 1200));
    let bytes_after = tokio::fs::read(&final_path).await.unwrap();
    assert_eq!(
        bytes_after, incumbent_bytes,
        "the file on disk must be untouched"
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

#[tokio::test]
async fn ac4_sovereignty_user_trust_incumbent_never_downloads_or_writes() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://example.test/user-pick.jpg"),
        "user_upload",
        CoverTrust::User,
        900,
        1300,
    )
    .await
    .unwrap();

    // A User lock is honored only while its file exists on disk — materialize
    // the user's cover so this test pins the protected case.
    tokio::fs::write(
        covers_dir.path().join(format!("{}.jpg", work.id)),
        fake_jpeg(900, 1300),
    )
    .await
    .unwrap();

    // A huge candidate that must never actually be fetched.
    let http = StubHttpFetcher::with_ok(200, fake_jpeg(2000, 3000));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://i.gr-assets.com/never.jpg",
                "goodreads",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(matches!(outcome, GateOutcome::NoOp));
    assert_eq!(
        http.call_count(),
        0,
        "a User-trust incumbent must never trigger a download — the comparator is never invoked"
    );
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://example.test/user-pick.jpg")
    );
    assert_eq!(after.cover_trust, CoverTrust::User);
}

#[tokio::test]
async fn ac4_sovereignty_applies_to_the_audiobook_slot_too() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    db.update_audiobook_cover_metadata(
        user_id,
        work.id,
        Some("https://example.test/user-audio.jpg"),
        "user_upload",
        CoverTrust::User,
        900,
        1300,
    )
    .await
    .unwrap();

    // A User lock is honored only while its file exists on disk — materialize
    // the user's audiobook cover so this test pins the protected case.
    tokio::fs::write(
        covers_dir.path().join(format!("{}_audio.jpg", work.id)),
        fake_jpeg(900, 1300),
    )
    .await
    .unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(2000, 3000));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Audiobook,
            resolution: CoverResolution {
                url: "https://m.media-amazon.com/never.jpg".into(),
                source: "audible".into(),
                trust: CoverTrust::Validated,
                media_type: CoverMediaType::Audiobook,
            },
        },
    )
    .await;

    assert!(matches!(outcome, GateOutcome::NoOp));
    assert_eq!(http.call_count(), 0);
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.audiobook_cover_trust, CoverTrust::User);
    assert_eq!(
        after.audiobook_cover_url.as_deref(),
        Some("https://example.test/user-audio.jpg")
    );
}

#[tokio::test]
async fn user_lock_in_the_committed_unrenamed_crash_window_still_blocks() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    // The crash-safe protocol's committed-but-unrenamed state: the DB row is
    // already User, the final .jpg is missing, and the candidate tmp + meta
    // sidecar await startup recovery's rename. A provider candidate must not
    // bulldoze the pending user cover.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://example.test/user-pick.jpg"),
        "user_upload",
        CoverTrust::User,
        900,
        1300,
    )
    .await
    .unwrap();
    let tmp = covers_dir.path().join(format!("{}.candidate.tmp", work.id));
    let meta = covers_dir
        .path()
        .join(format!("{}.candidate.meta.json", work.id));
    tokio::fs::write(&tmp, fake_jpeg(900, 1300)).await.unwrap();
    tokio::fs::write(&meta, b"{}").await.unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(2000, 3000));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://i.gr-assets.com/never.jpg",
                "goodreads",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(matches!(outcome, GateOutcome::NoOp));
    assert_eq!(http.call_count(), 0);
    assert!(
        tokio::fs::try_exists(&tmp).await.unwrap() && tokio::fs::try_exists(&meta).await.unwrap(),
        "the pending user candidate files must be left for recovery"
    );
}

#[tokio::test]
async fn user_trust_row_with_no_file_on_disk_is_replaceable() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    // A damaged slot: User trust stamped by a failed add-time download —
    // cover_url set, 0x0 dims, no file on disk.
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://covers.openlibrary.org/failed.jpg"),
        "add",
        CoverTrust::User,
        0,
        0,
    )
    .await
    .unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(640, 960));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://assets.hardcover.app/real.jpg",
                "hardcover",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(
        matches!(outcome, GateOutcome::Accepted { .. }),
        "a fileless User lock must not refuse a real candidate"
    );
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.cover_trust, CoverTrust::Validated);
    assert_eq!(after.cover_source.as_deref(), Some("hardcover"));
    assert_ne!((after.cover_width, after.cover_height), (0, 0));
}

#[tokio::test]
async fn ac5_no_incumbent_initial_save_stamps_real_source_trust_and_measured_dims() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(640, 960));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://assets.hardcover.app/first.jpg",
                "hardcover",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(matches!(
        outcome,
        GateOutcome::Accepted {
            width: 640,
            height: 960,
            ..
        }
    ));
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_source.as_deref(),
        Some("hardcover"),
        "must stamp the real provider — never the literal 'add' placeholder"
    );
    assert_eq!(after.cover_trust, CoverTrust::Validated);
    assert_eq!((after.cover_width, after.cover_height), (640, 960));
    assert_ne!((after.cover_width, after.cover_height), (0, 0));
}

#[tokio::test]
async fn ac5_audiobook_slot_is_symmetric_with_its_own_dims_writer() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(720, 1080));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Audiobook,
            resolution: CoverResolution {
                url: "https://m.media-amazon.com/audio-first.jpg".into(),
                source: "audible".into(),
                trust: CoverTrust::Validated,
                media_type: CoverMediaType::Audiobook,
            },
        },
    )
    .await;

    assert!(outcome.is_accepted());
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.audiobook_cover_source.as_deref(), Some("audible"));
    assert_eq!(after.audiobook_cover_trust, CoverTrust::Validated);
    assert_eq!(
        (after.audiobook_cover_width, after.audiobook_cover_height),
        (720, 1080)
    );
}

#[tokio::test]
async fn ac8_ebook_gate_call_never_writes_audiobook_columns() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    db.update_audiobook_cover_metadata(
        user_id,
        work.id,
        Some("https://m.media-amazon.com/audio.jpg"),
        "audible",
        CoverTrust::Validated,
        1000,
        1000,
    )
    .await
    .unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(640, 960));
    let _ = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://assets.hardcover.app/ebook.jpg",
                "hardcover",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.audiobook_cover_url.as_deref(),
        Some("https://m.media-amazon.com/audio.jpg"),
        "an ebook-slot save must never write the audiobook slot"
    );
    assert_eq!(after.audiobook_cover_source.as_deref(), Some("audible"));
    assert_eq!(
        (after.audiobook_cover_width, after.audiobook_cover_height),
        (1000, 1000)
    );
    // And the ebook slot it WAS asked to save did land.
    assert_eq!(after.cover_source.as_deref(), Some("hardcover"));
}

#[tokio::test]
async fn ac9_audiobook_save_lands_at_the_audio_suffix_path_never_the_legacy_one() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(500, 700));
    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Audiobook,
            resolution: CoverResolution {
                url: "https://m.media-amazon.com/audio.jpg".into(),
                source: "audible".into(),
                trust: CoverTrust::Validated,
                media_type: CoverMediaType::Audiobook,
            },
        },
    )
    .await;

    assert!(outcome.is_accepted());
    let expected_path = covers_dir.path().join(format!("{}_audio.jpg", work.id));
    assert!(
        expected_path.exists(),
        "must save at the _audio suffix — save suffix == serve suffix"
    );
    assert!(!covers_dir
        .path()
        .join(format!("{}_audiobook.jpg", work.id))
        .exists());
}

#[tokio::test]
async fn refresh_with_unchanged_pick_and_file_present_is_idempotent_no_redownload() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://i.gr-assets.com/same.jpg"),
        "goodreads",
        CoverTrust::Validated,
        500,
        700,
    )
    .await
    .unwrap();
    let final_path = covers_dir.path().join(format!("{}.jpg", work.id));
    tokio::fs::write(&final_path, fake_jpeg(500, 700))
        .await
        .unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(9999, 9999));

    let outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://i.gr-assets.com/same.jpg",
                "goodreads",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(matches!(outcome, GateOutcome::AlreadyCurrent));
    assert_eq!(
        http.call_count(),
        0,
        "an unchanged pick with the file already present must not re-download every refresh"
    );
}

// =============================================================================
// R3 — user covers through the write gate (`run_user_cover_write`)
// =============================================================================

/// Pre-refactor sidecars always had `url` as a required plain string.
/// `#[serde(default)]` on the new `Option<String>` field must not break
/// parsing a v1 file left on disk across an upgrade — this is a pure serde
/// round-trip check, independent of the DB/filesystem harness above.
#[test]
fn v1_candidate_meta_with_string_url_still_deserializes() {
    let json = r#"{"url":"https://i.gr-assets.com/old.jpg","source":"goodreads","trust":"validated","width":500,"height":700}"#;
    let meta: CandidateMeta = serde_json::from_str(json).unwrap();
    assert_eq!(meta.url.as_deref(), Some("https://i.gr-assets.com/old.jpg"));
    assert_eq!(meta.source, "goodreads");
    assert_eq!(meta.trust, CoverTrust::Validated);
    assert_eq!((meta.width, meta.height), (500, 700));
}

#[tokio::test]
async fn user_select_replaces_an_existing_user_trust_cover() {
    // The enrichment gate's User-NoOp guard must NOT apply to a user's own
    // pick — replacing an earlier User-trust cover with a new one is exactly
    // the case `run_user_cover_write` exists to serve.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://example.test/old-user-pick.jpg"),
        "user_upload",
        CoverTrust::User,
        900,
        1300,
    )
    .await
    .unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(640, 960));

    let outcome = run_user_cover_write(
        &db,
        &http,
        user_id,
        UserCoverInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            payload: UserCoverPayload::Url {
                url: "https://covers.openlibrary.org/new-user-pick.jpg".into(),
                source: "isbn_ol".into(),
            },
        },
    )
    .await
    .expect("a user write must not error here");

    assert!(
        matches!(
            outcome,
            GateOutcome::Accepted {
                width: 640,
                height: 960,
                ..
            }
        ),
        "a user's own pick must replace their own earlier User-trust cover, not NoOp: {outcome:?}"
    );
    assert_eq!(http.call_count(), 1);

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://covers.openlibrary.org/new-user-pick.jpg")
    );
    assert_eq!(after.cover_source.as_deref(), Some("isbn_ol"));
    assert_eq!(after.cover_trust, CoverTrust::User);
    assert!(
        after.cover_manual,
        "the ebook slot derives cover_manual from trust=User via update_cover_metadata"
    );
}

#[tokio::test]
async fn upload_rejects_bad_magic_bytes() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();
    let http = StubHttpFetcher::new();

    let result = run_user_cover_write(
        &db,
        &http,
        user_id,
        UserCoverInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            payload: UserCoverPayload::Bytes {
                data: b"this is plainly not an image".to_vec(),
            },
        },
    )
    .await;

    match result {
        Err(UserCoverError::Validation(msg)) => {
            assert!(msg.contains("unsupported format"), "message was: {msg}");
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
    assert_eq!(http.call_count(), 0, "an upload must never hit the network");
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert!(
        after.cover_url.is_none(),
        "a rejected upload must not touch the row"
    );
}

#[tokio::test]
async fn upload_rejects_oversized_file() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();
    let http = StubHttpFetcher::new();

    let oversized = vec![0u8; 5 * 1024 * 1024 + 1];
    let result = run_user_cover_write(
        &db,
        &http,
        user_id,
        UserCoverInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            payload: UserCoverPayload::Bytes { data: oversized },
        },
    )
    .await;

    match result {
        Err(UserCoverError::Validation(msg)) => {
            assert!(msg.contains("too large"), "message was: {msg}");
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert!(after.cover_url.is_none());
}

#[tokio::test]
async fn upload_rejects_oversized_dimensions() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();
    let http = StubHttpFetcher::new();

    // A thin 8001x1 JPEG trips the >8000-per-side cap without allocating a
    // huge buffer.
    let huge = fake_jpeg(8001, 1);
    let result = run_user_cover_write(
        &db,
        &http,
        user_id,
        UserCoverInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            payload: UserCoverPayload::Bytes { data: huge },
        },
    )
    .await;

    match result {
        Err(UserCoverError::Validation(msg)) => {
            assert!(msg.contains("too large"), "message was: {msg}");
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn upload_valid_png_is_reencoded_to_jpeg_and_accepted() {
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();
    let http = StubHttpFetcher::new();

    let png_bytes = include_bytes!("fixtures/test_cover_100x150.png").to_vec();
    assert!(
        png_bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
        "fixture must actually be a PNG"
    );

    let outcome = run_user_cover_write(
        &db,
        &http,
        user_id,
        UserCoverInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            payload: UserCoverPayload::Bytes { data: png_bytes },
        },
    )
    .await
    .expect("a valid PNG must be accepted");

    match outcome {
        GateOutcome::Accepted {
            bytes,
            width,
            height,
        } => {
            assert_eq!((width, height), (100, 150));
            assert!(
                bytes.starts_with(&[0xFF, 0xD8]),
                "an uploaded PNG must be re-encoded to JPEG before being written to disk"
            );
        }
        other => panic!("expected Accepted, got {other:?}"),
    }
    assert_eq!(http.call_count(), 0, "an upload must never hit the network");

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.cover_source.as_deref(), Some("user_upload"));
    assert_eq!(after.cover_trust, CoverTrust::User);
    assert!(after.cover_url.is_none(), "an upload has no source URL");
    assert!(after.cover_manual);

    let final_path = covers_dir.path().join(format!("{}.jpg", work.id));
    let on_disk = tokio::fs::read(&final_path).await.unwrap();
    assert!(on_disk.starts_with(&[0xFF, 0xD8]));
}

#[tokio::test]
async fn enrichment_still_noops_after_a_real_user_cover_write() {
    // Regression pin for the refactor: after a user's OWN write lands (via
    // `run_user_cover_write`, not a synthetic DB-only seed), the enrichment
    // entry point must still respect it — never re-download, never touch
    // the row.
    let db = create_test_db().await;
    let (user_id, work) = seed_user_and_work(&db).await;
    let covers_dir = tempfile::tempdir().unwrap();

    let http = StubHttpFetcher::with_ok(200, fake_jpeg(900, 1300));
    let user_outcome = run_user_cover_write(
        &db,
        &http,
        user_id,
        UserCoverInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            payload: UserCoverPayload::Url {
                url: "https://example.test/user-pick.jpg".into(),
                source: "isbn_ol".into(),
            },
        },
    )
    .await
    .unwrap();
    assert!(user_outcome.is_accepted());
    assert_eq!(http.call_count(), 1);

    let enrich_outcome = run_cover_write_gate(
        &db,
        &http,
        user_id,
        CoverWriteGateInput {
            covers_dir: covers_dir.path().to_path_buf(),
            work_id: work.id,
            media_type: CoverMediaType::Ebook,
            resolution: ebook_resolution(
                "https://i.gr-assets.com/never.jpg",
                "goodreads",
                CoverTrust::Validated,
            ),
        },
    )
    .await;

    assert!(matches!(enrich_outcome, GateOutcome::NoOp));
    assert_eq!(
        http.call_count(),
        1,
        "the enrichment attempt must never download — the User-trust guard blocks before fetch"
    );
    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(
        after.cover_url.as_deref(),
        Some("https://example.test/user-pick.jpg"),
        "the user's own pick must survive an enrichment attempt"
    );
    assert_eq!(after.cover_trust, CoverTrust::User);
}
