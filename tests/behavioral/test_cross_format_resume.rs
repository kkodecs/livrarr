//! Behavioral tests for cross-format resume.
//!
//! These tests are intentionally RED against the Stage-4 stubs. They compile
//! against the public contracts and encode the Stage-5 behavior.
//!
//! ir-v2 tdd_directives backend coverage map:
//! - domain-kash::parse_kash valid/rejects -> kash_parse_validates_minimal_sidecar_and_rejects_invalid_inputs
//! - domain-kash::anchor_at_or_before boundaries -> anchor_at_or_before_obeys_boundary_rules
//! - domain-kash::resolve_target edges/never-backward -> resolve_target_is_strictly_forward_with_edges
//! - domain-kash::chapter_label title/percent -> chapter_label_uses_chapter_title_when_available
//! - db-cross-format::KashLinkDb::upsert_link insert/same-identity/reset/constraint -> upsert_link_fresh_insert_returns_row, upsert_link_same_identity_preserves_state, upsert_link_identity_change_resets_only_that_link_state, upsert_link_new_audio_to_already_linked_ebook_returns_constraint
//! - db-cross-format::delete_link_for_audio cascade/idempotent -> delete_link_for_audio_removes_link_and_state_idempotently
//! - db-cross-format::link_for_item audio/ebook/none -> link_for_item_finds_audio_ebook_and_none
//! - db-cross-format::get_or_default missing/existing -> get_or_default_returns_zero_without_inserting, get_or_default_existing_row_round_trips_all_fields
//! - db-cross-format::set_decline format isolation/no prior row -> set_decline_writes_only_named_format_threshold, set_decline_inserts_default_state_when_missing
//! - db-cross-format::sync_to decrease/clear -> sync_to_may_decrease_and_clears_declines
//! - progress-extension::PlaybackProgressDb linked/seek/unlinked/two users -> progress_advances_furthest_monotonically_for_linked_item, seek_never_advances_furthest, progress_on_unlinked_item_creates_no_state_row, two_users_on_same_link_have_independent_furthest_rows
//! - progress-extension::finished_at lifecycle zones -> covered-by: playback_enhancement_tests after signature update
//! - progress-extension::FileService suppress_lifecycle -> file_service_update_progress_suppress_lifecycle_drops_cross_format_state
//! - progress-extension::handler legacy body -> update_progress_legacy_body_defaults_to_seek_and_succeeds
//! - server-cross-format-service::load_validated error classes/user scope -> resume_prompt_unlinked_returns_none, resume_prompt_duration_drift_returns_none, resume_prompt_epub_hash_mismatch_returns_none, anchors_for_item_returns_alignment_or_precise_errors, foreign_user_item_is_not_linked
//! - server-cross-format-service::resume_prompt directions/boundaries/decline/isolation -> resume_prompt_ebook_direction_returns_cfi_and_chapter_label, resume_prompt_audio_direction_returns_seconds_and_timestamp_label, resume_prompt_none_when_furthest_not_strictly_ahead, resume_prompt_fresh_link_without_progress_returns_none, decline_suppresses_until_furthest_advances, two_links_in_one_work_are_isolated
//! - server-cross-format-service::anchors_for_item order/errors -> anchors_for_item_returns_alignment_or_precise_errors
//! - server-cross-format-service::decline_resume format-only -> decline_resume_records_opened_format_only
//! - server-cross-format-service::sync_to_here mid-book/before-first -> sync_to_here_stores_preceding_anchor_even_when_decreasing, sync_to_here_before_first_anchor_stores_zero
//! - server-cross-format-service::AppState wiring -> S5/compile integration
//! - library-scan-hook::try_extract_chapters existing behavior/duration -> covered-by: test_consolidation_import_workflow
//! - library-scan-hook::establish_kash_link positive/negative/reconcile/poison/audio-not-opened -> establish_kash_link_matches_epub_without_opening_audio_file, establish_kash_link_no_sidecar_is_ok_and_creates_no_row, establish_kash_link_malformed_sidecar_leaves_existing_link_intact, establish_kash_link_no_matching_epub_errors_without_row, establish_kash_link_duration_drift_deletes_existing_link_and_state, establish_kash_link_removed_sidecar_deletes_existing_link_and_state, r003_poison_window_drift_listen_restore_yields_fresh_state, establish_kash_link_ebook_already_linked_keeps_original_link
//! - library-scan-hook::manual call-site wiring -> manual_extract_chapters_for_item_establishes_kash_link, manual_extract_chapters_for_item_kash_failure_does_not_panic_or_link
//! - handlers-cross-format status/validation -> cross_format_prompt_unlinked_returns_200_null, cross_format_unlinked_anchors_decline_and_sync_return_404, cross_format_prompt_invalid_current_ts_returns_400, cross_format_sync_invalid_current_ts_returns_400
//! - handlers-cross-format compile wall -> S5/CI cargo tree check
//! - frontend-resume/frontend-sleep-bookmark directives -> frontend/S5

use assert_matches::assert_matches;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use axum::routing::{get, post, put};
use axum::Router;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    create_test_db, AuthorDb, ChapterDb, CreateAuthorDbRequest, CreateLibraryItemDbRequest,
    CreateUserDbRequest, CreateWorkDbRequest, CrossFormatStateDb, DbError, KashLinkDb,
    LibraryItemDb, MediaType, NewKashLink, PlaybackProgressDb, ProgressKind, RootFolderDb,
    TagStatus, UserDb, UserRole, WorkDbCreate,
};
use livrarr_domain::kash::{
    anchor_at_or_before, chapter_label, parse_kash, resolve_target, AlignmentEntry, Kash,
    KashChapter, KashError, DURATION_TOLERANCE_SECS,
};
use livrarr_domain::services::{
    CrossFormatError, CrossFormatService, EmailPayload, FileService, FileServiceError,
    ItemProgress, ResumePrompt,
};
use livrarr_domain::{AuthType, LibraryItem, LibraryItemId, PlaybackProgress, User, UserId};
use livrarr_handlers::context::{HasCrossFormatService, HasFileService};
use livrarr_handlers::types::auth::AuthContext;
use livrarr_library::cross_format_service::CrossFormatServiceImpl;
use livrarr_library::file_service::FileServiceImpl;
use livrarr_library::import_workflow::{establish_kash_link, ImportWorkflowImpl, KashLinkError};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const AUDIO_REL: &str = "author/work/book.m4b";
const EBOOK_REL: &str = "author/work/book.epub";
const OTHER_AUDIO_REL: &str = "author/work/other.m4b";
const OTHER_EBOOK_REL: &str = "author/work/other.epub";
const EPUB_BYTES: &[u8] = b"fake epub bytes";
const OTHER_EPUB_BYTES: &[u8] = b"other fake epub bytes";

struct Seed {
    user_id: i64,
    work_id: i64,
    audio_item_id: i64,
    ebook_item_id: i64,
    audio_root: tempfile::TempDir,
    ebook_root: tempfile::TempDir,
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn kash_json(epub_hash: &str, duration: f64, alignment: serde_json::Value) -> Vec<u8> {
    json!({
        "version": 1,
        "epub_hash": epub_hash,
        "audio_hash": "irrelevant-provenance",
        "duration_seconds": duration,
        "chapters": [
            {"title": "Chapter 1", "start": 0.0, "end": 1800.0},
            {"title": "Chapter 2", "start": 1800.0, "end": 3600.0}
        ],
        "alignment": alignment
    })
    .to_string()
    .into_bytes()
}

fn valid_kash_bytes() -> Vec<u8> {
    kash_json(
        &sha256_hex(EPUB_BYTES),
        3600.0,
        json!([
            {"cfi": "epubcfi(/6/2!/4/2)", "ts": 10.0},
            {"cfi": "epubcfi(/6/4!/4/2)", "ts": 20.0}
        ]),
    )
}

fn sample_kash() -> Kash {
    Kash {
        version: 1,
        epub_hash: sha256_hex(EPUB_BYTES),
        audio_hash: "irrelevant-provenance".to_string(),
        duration_seconds: 3600.0,
        chapters: vec![
            KashChapter {
                title: "Chapter 1".to_string(),
                start: 0.0,
                end: 1800.0,
            },
            KashChapter {
                title: "Chapter 2".to_string(),
                start: 1800.0,
                end: 3600.0,
            },
        ],
        alignment: vec![
            AlignmentEntry {
                cfi: "epubcfi(/6/2!/4/2)".to_string(),
                ts: 10.0,
            },
            AlignmentEntry {
                cfi: "epubcfi(/6/4!/4/2)".to_string(),
                ts: 20.0,
            },
            AlignmentEntry {
                cfi: "epubcfi(/6/6!/4/2)".to_string(),
                ts: 40.0,
            },
        ],
    }
}

async fn seed_library(db: &SqliteDb) -> Seed {
    let audio_root = tempfile::tempdir().unwrap();
    let ebook_root = tempfile::tempdir().unwrap();

    let user = db
        .create_user(CreateUserDbRequest {
            username: format!("user-{}", uuidish()),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            api_key_hash: format!("api-{}", uuidish()),
        })
        .await
        .unwrap();

    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id: user.id,
            name: "Test Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .unwrap();

    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id: user.id,
            title: "Test Work".to_string(),
            author_name: "Test Author".to_string(),
            normalized_title: "test work".to_string(),
            normalized_author: "test author".to_string(),
            author_id: Some(author.id),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..Default::default()
        })
        .await
        .unwrap();

    let audio_root_row = db
        .create_root_folder(audio_root.path().to_str().unwrap(), MediaType::Audiobook)
        .await
        .unwrap();
    let ebook_root_row = db
        .create_root_folder(ebook_root.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();

    let audio_item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id: user.id,
            work_id: work.id,
            root_folder_id: audio_root_row.id,
            path: AUDIO_REL.to_string(),
            media_type: MediaType::Audiobook,
            file_size: 100_000,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();
    let ebook_item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id: user.id,
            work_id: work.id,
            root_folder_id: ebook_root_row.id,
            path: EBOOK_REL.to_string(),
            media_type: MediaType::Ebook,
            file_size: EPUB_BYTES.len() as i64,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();

    Seed {
        user_id: user.id,
        work_id: work.id,
        audio_item_id: audio_item.id,
        ebook_item_id: ebook_item.id,
        audio_root,
        ebook_root,
    }
}

fn uuidish() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed).to_string()
}

fn abs_path(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

fn write_file(root: &Path, rel: &str, bytes: &[u8]) {
    let path = abs_path(root, rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn write_kash_for_audio(root: &Path, audio_rel: &str, bytes: &[u8]) {
    let mut rel = PathBuf::from(audio_rel);
    rel.set_extension("kash");
    write_file(root, rel.to_str().unwrap(), bytes);
}

async fn raw_insert_link(
    db: &SqliteDb,
    audio_item_id: i64,
    ebook_item_id: i64,
    duration: f64,
    epub_hash: &str,
) -> i64 {
    let result = sqlx::query(
        "INSERT INTO kash_links (audio_item_id, ebook_item_id, container_duration_secs, epub_hash)
         VALUES (?, ?, ?, ?)",
    )
    .bind(audio_item_id)
    .bind(ebook_item_id)
    .bind(duration)
    .bind(epub_hash)
    .execute(db.pool())
    .await
    .unwrap();
    result.last_insert_rowid()
}

async fn raw_insert_state(
    db: &SqliteDb,
    user_id: i64,
    link_id: i64,
    furthest_ts: f64,
    ebook_decline: Option<f64>,
    audio_decline: Option<f64>,
) {
    sqlx::query(
        "INSERT INTO cross_format_state
         (user_id, kash_link_id, furthest_ts, ebook_declined_at_ts, audio_declined_at_ts)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(link_id)
    .bind(furthest_ts)
    .bind(ebook_decline)
    .bind(audio_decline)
    .execute(db.pool())
    .await
    .unwrap();
}

async fn state_count(db: &SqliteDb, link_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM cross_format_state WHERE kash_link_id = ?")
        .bind(link_id)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn link_count_for_audio(db: &SqliteDb, audio_item_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM kash_links WHERE audio_item_id = ?")
        .bind(audio_item_id)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn raw_furthest(db: &SqliteDb, user_id: i64, link_id: i64) -> Option<f64> {
    sqlx::query_scalar(
        "SELECT furthest_ts FROM cross_format_state WHERE user_id = ? AND kash_link_id = ?",
    )
    .bind(user_id)
    .bind(link_id)
    .fetch_optional(db.pool())
    .await
    .unwrap()
}

async fn add_library_item(
    db: &SqliteDb,
    seed: &Seed,
    root_folder_id: i64,
    rel: &str,
    media_type: MediaType,
    file_size: i64,
) -> i64 {
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id: seed.user_id,
        work_id: seed.work_id,
        root_folder_id,
        path: rel.to_string(),
        media_type,
        file_size,
        import_id: None,
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap()
    .id
}

async fn root_id_for(db: &SqliteDb, media_type: MediaType) -> i64 {
    db.get_root_folder_by_media_type(media_type)
        .await
        .unwrap()
        .unwrap()
        .id
}

fn new_link(seed: &Seed) -> NewKashLink {
    NewKashLink {
        audio_item_id: seed.audio_item_id,
        ebook_item_id: seed.ebook_item_id,
        container_duration_secs: 3600.0,
        epub_hash: sha256_hex(EPUB_BYTES),
    }
}

fn service(db: SqliteDb) -> CrossFormatServiceImpl<SqliteDb, FileServiceImpl<SqliteDb>> {
    CrossFormatServiceImpl::new(db.clone(), FileServiceImpl::new(db))
}

fn make_workflow(db: SqliteDb) -> ImportWorkflowImpl<SqliteDb> {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let data_dir = std::sync::Arc::new(std::path::PathBuf::from("/tmp/livrarr-test"));
    ImportWorkflowImpl::new(db, semaphore, data_dir)
}

// ============================================================================
// GROUP A: domain kash
// ============================================================================

/// AC-011/REQ-015: valid sidecars parse; malformed and invalid sidecars are rejected.
#[tokio::test]
async fn kash_parse_validates_minimal_sidecar_and_rejects_invalid_inputs() {
    // Expected red source today: livrarr_domain::kash::parse_kash todo!().
    let parsed = parse_kash(&valid_kash_bytes()).unwrap();
    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.alignment.len(), 2);

    let epub_hash = sha256_hex(EPUB_BYTES);
    let cases = [
        json!({"version":2,"epub_hash":epub_hash,"audio_hash":"x","duration_seconds":3600.0,"chapters":[],"alignment":[{"cfi":"a","ts":1.0}]}).to_string(),
        json!({"version":1,"epub_hash":epub_hash,"audio_hash":"x","duration_seconds":3600.0,"chapters":[],"alignment":[]}).to_string(),
        json!({"version":1,"epub_hash":epub_hash,"audio_hash":"x","duration_seconds":3600.0,"chapters":[],"alignment":[{"cfi":"a","ts":2.0},{"cfi":"b","ts":1.0}]}).to_string(),
        r#"{"version":1,"epub_hash":"abc","audio_hash":"x","duration_seconds":3600.0,"chapters":[],"alignment":[{"cfi":"a","ts":NaN}]}"#.to_string(),
        json!({"version":1,"epub_hash":epub_hash,"audio_hash":"x","duration_seconds":0.0,"chapters":[],"alignment":[{"cfi":"a","ts":1.0}]}).to_string(),
        json!({"version":1,"epub_hash":epub_hash,"audio_hash":"x","duration_seconds":-1.0,"chapters":[],"alignment":[{"cfi":"a","ts":1.0}]}).to_string(),
        json!({"version":1,"epub_hash":"","audio_hash":"x","duration_seconds":3600.0,"chapters":[],"alignment":[{"cfi":"a","ts":1.0}]}).to_string(),
        "{not json".to_string(),
    ];

    for case in cases {
        // Expected red source today: livrarr_domain::kash::parse_kash todo!().
        assert_matches!(parse_kash(case.as_bytes()), Err(KashError::Malformed(_)));
    }

    // Equal-ts ties are NOT malformed: real generators emit same-second ties.
    // Parse normalizes by keeping the final anchor of each tied run, so the
    // returned alignment is strictly increasing (only a DECREASE rejects).
    let tied = json!({"version":1,"epub_hash":epub_hash,"audio_hash":"x","duration_seconds":3600.0,"chapters":[],"alignment":[{"cfi":"a","ts":1.0},{"cfi":"b","ts":1.0},{"cfi":"c","ts":2.0}]}).to_string();
    let tied_parsed = parse_kash(tied.as_bytes()).unwrap();
    assert_eq!(tied_parsed.alignment.len(), 2);
    assert_eq!(tied_parsed.alignment[0].cfi, "b");
    assert_eq!(tied_parsed.alignment[0].ts, 1.0);
    assert_eq!(tied_parsed.alignment[1].cfi, "c");
}

/// AC-011/REQ-015: at-or-before lookup returns exact, preceding, last, or none at boundaries.
#[tokio::test]
async fn anchor_at_or_before_obeys_boundary_rules() {
    let kash = sample_kash();
    // Expected red source today: livrarr_domain::kash::anchor_at_or_before todo!().
    assert_eq!(
        anchor_at_or_before(&kash, 20.0).unwrap().cfi,
        kash.alignment[1].cfi
    );
    assert_eq!(
        anchor_at_or_before(&kash, 25.0).unwrap().cfi,
        kash.alignment[1].cfi
    );
    assert_eq!(
        anchor_at_or_before(&kash, 100.0).unwrap().cfi,
        kash.alignment[2].cfi
    );
    assert!(anchor_at_or_before(&kash, 1.0).is_none());
}

/// AC-007/AC-011/REQ-015: resolve_target never moves backward and handles gaps and edges.
#[tokio::test]
async fn resolve_target_is_strictly_forward_with_edges() {
    let kash = sample_kash();
    // Expected red source today: livrarr_domain::kash::resolve_target todo!().
    assert_eq!(resolve_target(&kash, 25.0, 9.0).unwrap().ts, 20.0);
    assert_eq!(resolve_target(&kash, 100.0, 25.0).unwrap().ts, 40.0);
    assert_eq!(resolve_target(&kash, 1.0, -1.0).unwrap().ts, 0.0);
    assert!(resolve_target(&kash, 25.0, 20.0).is_none());
    assert!(resolve_target(&kash, 25.0, 21.0).is_none());
    assert!(resolve_target(&kash, 1.0, 0.0).is_none());
}

/// AC-013/REQ-004: ebook labels are chapter-aware when possible and percentage-only otherwise.
#[tokio::test]
async fn chapter_label_uses_chapter_title_when_available() {
    let kash = sample_kash();
    // Expected red source today: livrarr_domain::kash::chapter_label todo!().
    let inside = chapter_label(&kash, 900.0);
    assert!(inside.contains("Chapter 1"));
    assert!(inside.contains("25%"));
    assert_eq!(chapter_label(&kash, 4000.0), "100%");
}

// ============================================================================
// GROUP B: db layer
// ============================================================================

/// REQ-002/REQ-003: fresh link insert returns the inserted identity.
#[tokio::test]
async fn upsert_link_fresh_insert_returns_row() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    // Expected red source today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();
    assert_eq!(link.audio_item_id, seed.audio_item_id);
    assert_eq!(link.ebook_item_id, seed.ebook_item_id);
}

/// REQ-016: same link identity with duration drift inside tolerance preserves state rows.
#[tokio::test]
async fn upsert_link_same_identity_preserves_state() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    // Expected red source today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();
    raw_insert_state(&db, seed.user_id, link.id, 120.0, Some(120.0), None).await;

    let mut refresh = new_link(&seed);
    refresh.container_duration_secs += DURATION_TOLERANCE_SECS;
    db.upsert_link(refresh).await.unwrap();

    assert_eq!(state_count(&db, link.id).await, 1);
}

/// REQ-008/REQ-016: identity-changing relinks clear only that link's cross-format state.
#[tokio::test]
async fn upsert_link_identity_change_resets_only_that_link_state() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let ebook_root_id = root_id_for(&db, MediaType::Ebook).await;
    let other_ebook_id = add_library_item(
        &db,
        &seed,
        ebook_root_id,
        OTHER_EBOOK_REL,
        MediaType::Ebook,
        OTHER_EPUB_BYTES.len() as i64,
    )
    .await;
    let audio_root_id = root_id_for(&db, MediaType::Audiobook).await;
    let other_audio_id = add_library_item(
        &db,
        &seed,
        audio_root_id,
        OTHER_AUDIO_REL,
        MediaType::Audiobook,
        100_000,
    )
    .await;
    let other_link_id = raw_insert_link(&db, other_audio_id, other_ebook_id, 3600.0, "other").await;
    raw_insert_state(&db, seed.user_id, other_link_id, 77.0, None, None).await;

    // Expected red source today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();
    raw_insert_state(&db, seed.user_id, link.id, 120.0, None, None).await;

    db.upsert_link(NewKashLink {
        audio_item_id: seed.audio_item_id,
        ebook_item_id: seed.ebook_item_id,
        container_duration_secs: 3600.0 + DURATION_TOLERANCE_SECS + 0.1,
        epub_hash: "changed".to_string(),
    })
    .await
    .unwrap();

    assert_eq!(state_count(&db, link.id).await, 0);
    assert_eq!(state_count(&db, other_link_id).await, 1);
}

/// REQ-002: one ebook can be linked to only one audio item; first link wins.
#[tokio::test]
async fn upsert_link_new_audio_to_already_linked_ebook_returns_constraint() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let audio_root_id = root_id_for(&db, MediaType::Audiobook).await;
    let other_audio_id = add_library_item(
        &db,
        &seed,
        audio_root_id,
        OTHER_AUDIO_REL,
        MediaType::Audiobook,
        100_000,
    )
    .await;
    // Expected red source today: KashLinkDb::upsert_link todo!().
    db.upsert_link(new_link(&seed)).await.unwrap();

    let err = db
        .upsert_link(NewKashLink {
            audio_item_id: other_audio_id,
            ebook_item_id: seed.ebook_item_id,
            container_duration_secs: 3600.0,
            epub_hash: sha256_hex(EPUB_BYTES),
        })
        .await
        .unwrap_err();
    assert_matches!(err, DbError::Constraint { .. });
}

/// REQ-002: links are discoverable from either side and absent items return None.
#[tokio::test]
async fn link_for_item_finds_audio_ebook_and_none() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    // Expected red source today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();

    assert_eq!(
        db.link_for_item(seed.audio_item_id).await.unwrap(),
        Some(link.clone())
    );
    assert_eq!(
        db.link_for_item(seed.ebook_item_id).await.unwrap(),
        Some(link)
    );
    assert_eq!(
        db.link_for_item(seed.ebook_item_id + 99_999).await.unwrap(),
        None
    );
}

/// REQ-008: deleting a link by audio cascades state and is idempotent.
#[tokio::test]
async fn delete_link_for_audio_removes_link_and_state_idempotently() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id =
        raw_insert_link(&db, seed.audio_item_id, seed.ebook_item_id, 3600.0, "hash").await;
    raw_insert_state(&db, seed.user_id, link_id, 44.0, None, None).await;

    // Expected red source today: KashLinkDb::delete_link_for_audio todo!().
    db.delete_link_for_audio(seed.audio_item_id).await.unwrap();
    db.delete_link_for_audio(seed.audio_item_id).await.unwrap();

    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
    assert_eq!(state_count(&db, link_id).await, 0);
}

/// REQ-017: missing state reads as zero-default without inserting a row.
#[tokio::test]
async fn get_or_default_returns_zero_without_inserting() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id =
        raw_insert_link(&db, seed.audio_item_id, seed.ebook_item_id, 3600.0, "hash").await;

    // Expected red source today: CrossFormatStateDb::get_or_default todo!().
    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();
    assert_eq!(state.furthest_ts, 0.0);
    assert_eq!(state.ebook_declined_at_ts, None);
    assert_eq!(state.audio_declined_at_ts, None);
    assert_eq!(state_count(&db, link_id).await, 0);
}

/// REQ-017: existing cross-format state round-trips every semantic field.
#[tokio::test]
async fn get_or_default_existing_row_round_trips_all_fields() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id =
        raw_insert_link(&db, seed.audio_item_id, seed.ebook_item_id, 3600.0, "hash").await;
    raw_insert_state(&db, seed.user_id, link_id, 123.0, Some(111.0), Some(99.0)).await;

    // Expected red source today: CrossFormatStateDb::get_or_default todo!().
    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();

    assert_eq!(state.user_id, seed.user_id);
    assert_eq!(state.kash_link_id, link_id);
    assert_eq!(state.furthest_ts, 123.0);
    assert_eq!(state.ebook_declined_at_ts, Some(111.0));
    assert_eq!(state.audio_declined_at_ts, Some(99.0));
}

/// AC-006/REQ-017: decline writes only the opened format threshold.
#[tokio::test]
async fn set_decline_writes_only_named_format_threshold() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id =
        raw_insert_link(&db, seed.audio_item_id, seed.ebook_item_id, 3600.0, "hash").await;
    raw_insert_state(&db, seed.user_id, link_id, 200.0, None, Some(90.0)).await;

    // Expected red source today: CrossFormatStateDb::set_decline todo!().
    db.set_decline(seed.user_id, link_id, MediaType::Ebook, 200.0)
        .await
        .unwrap();
    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();
    assert_eq!(state.furthest_ts, 200.0);
    assert_eq!(state.ebook_declined_at_ts, Some(200.0));
    assert_eq!(state.audio_declined_at_ts, Some(90.0));
}

/// AC-006/REQ-017: decline can create a state row without changing furthest.
#[tokio::test]
async fn set_decline_inserts_default_state_when_missing() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id =
        raw_insert_link(&db, seed.audio_item_id, seed.ebook_item_id, 3600.0, "hash").await;

    // Expected red source today: CrossFormatStateDb::set_decline todo!().
    db.set_decline(seed.user_id, link_id, MediaType::Audiobook, 0.0)
        .await
        .unwrap();
    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();

    assert_eq!(state.furthest_ts, 0.0);
    assert_eq!(state.ebook_declined_at_ts, None);
    assert_eq!(state.audio_declined_at_ts, Some(0.0));
    assert_eq!(state_count(&db, link_id).await, 1);
}

/// AC-016/REQ-018: sync_to may decrease furthest and clears both decline thresholds.
#[tokio::test]
async fn sync_to_may_decrease_and_clears_declines() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id =
        raw_insert_link(&db, seed.audio_item_id, seed.ebook_item_id, 3600.0, "hash").await;
    raw_insert_state(&db, seed.user_id, link_id, 300.0, Some(300.0), Some(300.0)).await;

    // Expected red source today: CrossFormatStateDb::sync_to todo!().
    db.sync_to(seed.user_id, link_id, 20.0).await.unwrap();
    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();
    assert_eq!(state.furthest_ts, 20.0);
    assert_eq!(state.ebook_declined_at_ts, None);
    assert_eq!(state.audio_declined_at_ts, None);
}

// ============================================================================
// GROUP C: progress extension
// ============================================================================

/// AC-004/REQ-003: genuine progress advances furthest monotonically.
#[tokio::test]
async fn progress_advances_furthest_monotonically_for_linked_item() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    // Seed step, expected red today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "120.0",
        0.25,
        ProgressKind::Progress,
        Some(120.0),
    )
    .await
    .unwrap();
    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "80.0",
        0.20,
        ProgressKind::Progress,
        Some(80.0),
    )
    .await
    .unwrap();

    assert_eq!(raw_furthest(&db, seed.user_id, link.id).await, Some(120.0));
}

/// AC-015/REQ-003: seeks never advance furthest, even with cross_format_ts.
#[tokio::test]
async fn seek_never_advances_furthest() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    // Seed step, expected red today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "3000.0",
        0.90,
        ProgressKind::Seek,
        Some(3000.0),
    )
    .await
    .unwrap();

    assert_eq!(raw_furthest(&db, seed.user_id, link.id).await, None);
}

/// REQ-003: progress on an unlinked item does not create cross-format state.
#[tokio::test]
async fn progress_on_unlinked_item_creates_no_state_row() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "120.0",
        0.25,
        ProgressKind::Progress,
        Some(120.0),
    )
    .await
    .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cross_format_state")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

/// AC-021/REQ-016: furthest rows are per user even on the same link.
#[tokio::test]
async fn two_users_on_same_link_have_independent_furthest_rows() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let user_b = db
        .create_user(CreateUserDbRequest {
            username: "other-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            api_key_hash: "other-api".to_string(),
        })
        .await
        .unwrap();
    // Seed step, expected red today: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "100.0",
        0.10,
        ProgressKind::Progress,
        Some(100.0),
    )
    .await
    .unwrap();
    db.upsert_progress(
        user_b.id,
        seed.audio_item_id,
        "200.0",
        0.20,
        ProgressKind::Progress,
        Some(200.0),
    )
    .await
    .unwrap();

    assert_eq!(raw_furthest(&db, seed.user_id, link.id).await, Some(100.0));
    assert_eq!(raw_furthest(&db, user_b.id, link.id).await, Some(200.0));
}

/// R-006/REQ-003: no-duration audiobook updates use no_lifecycle and drop cross-format args.
#[tokio::test]
async fn file_service_update_progress_suppress_lifecycle_drops_cross_format_state() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    // Seed step, expected red today until S5: KashLinkDb::upsert_link todo!().
    let link = db.upsert_link(new_link(&seed)).await.unwrap();
    let files = FileServiceImpl::new(db.clone());

    // The seed audiobook has NULL duration because update_chapter_scan_result
    // is intentionally never called in this test.
    files
        .update_progress(
            seed.user_id,
            seed.audio_item_id,
            "180.0",
            0.5,
            ProgressKind::Progress,
            Some(180.0),
        )
        .await
        .unwrap();

    let progress = db
        .get_progress(seed.user_id, seed.audio_item_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.position, "180.0");
    assert_eq!(progress.progress_pct, 0.5);
    assert_eq!(raw_furthest(&db, seed.user_id, link.id).await, None);
    assert_eq!(state_count(&db, link.id).await, 0);
}

// ============================================================================
// GROUP D: scan hook
// ============================================================================

/// AC-001/AC-002/REQ-009: an m4b sibling .kash binds to the matching on-disk epub.
#[tokio::test]
async fn establish_kash_link_matches_epub_without_opening_audio_file() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    // Expected red source today: establish_kash_link todo!() after sidecar detection.
    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap();

    let link = db.link_for_item(seed.audio_item_id).await.unwrap().unwrap();
    assert_eq!(link.ebook_item_id, seed.ebook_item_id);
}

/// AC-001/REQ-001: no sidecar is Ok and creates no row.
#[tokio::test]
async fn establish_kash_link_no_sidecar_is_ok_and_creates_no_row() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap();

    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
}

/// AC-002/REQ-002: sidecar with no matching ebook returns NoMatchingEbook and creates no row.
#[tokio::test]
async fn establish_kash_link_no_matching_epub_errors_without_row() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, b"wrong bytes");
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    // Expected red source today: establish_kash_link todo!() after sidecar detection.
    let err = establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap_err();
    assert_matches!(err, KashLinkError::NoMatchingEbook);
    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
}

/// AC-009/REQ-008: duration drift deletes any existing link and its state.
#[tokio::test]
async fn establish_kash_link_duration_drift_deletes_existing_link_and_state() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = raw_insert_link(
        &db,
        seed.audio_item_id,
        seed.ebook_item_id,
        3600.0,
        &sha256_hex(EPUB_BYTES),
    )
    .await;
    raw_insert_state(&db, seed.user_id, link_id, 100.0, None, None).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    // Expected red source today: establish_kash_link todo!() after sidecar detection.
    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0 + DURATION_TOLERANCE_SECS + 0.1,
    )
    .await
    .unwrap();

    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
    assert_eq!(state_count(&db, link_id).await, 0);
}

/// REQ-008/R-003: drifted listen windows cannot poison a later restored link.
#[tokio::test]
async fn r003_poison_window_drift_listen_restore_yields_fresh_state() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap();
    let original_link = db.link_for_item(seed.audio_item_id).await.unwrap().unwrap();

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "120.0",
        0.25,
        ProgressKind::Progress,
        Some(120.0),
    )
    .await
    .unwrap();
    assert_eq!(
        raw_furthest(&db, seed.user_id, original_link.id).await,
        Some(120.0)
    );

    let drifted_duration = 3600.0 + DURATION_TOLERANCE_SECS + 0.1;
    db.update_chapter_scan_result(seed.audio_item_id, "scanned", Some(drifted_duration))
        .await
        .unwrap();
    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        drifted_duration,
    )
    .await
    .unwrap();
    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
    assert_eq!(state_count(&db, original_link.id).await, 0);

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "240.0",
        0.40,
        ProgressKind::Progress,
        Some(240.0),
    )
    .await
    .unwrap();
    let state_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cross_format_state")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(state_rows, 0);

    db.update_chapter_scan_result(seed.audio_item_id, "scanned", Some(3600.0))
        .await
        .unwrap();
    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap();
    let restored_link = db.link_for_item(seed.audio_item_id).await.unwrap().unwrap();
    let restored_furthest = raw_furthest(&db, seed.user_id, restored_link.id).await;
    assert!(matches!(restored_furthest, None | Some(0.0)));
    assert_ne!(restored_furthest, Some(120.0));
    assert_ne!(restored_furthest, Some(240.0));

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "360.0",
        0.50,
        ProgressKind::Progress,
        Some(360.0),
    )
    .await
    .unwrap();
    assert_eq!(
        raw_furthest(&db, seed.user_id, restored_link.id).await,
        Some(360.0)
    );
}

/// REQ-008/R-003: removing the sidecar on rescan deletes an existing link and state.
#[tokio::test]
async fn establish_kash_link_removed_sidecar_deletes_existing_link_and_state() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = raw_insert_link(
        &db,
        seed.audio_item_id,
        seed.ebook_item_id,
        3600.0,
        &sha256_hex(EPUB_BYTES),
    )
    .await;
    raw_insert_state(&db, seed.user_id, link_id, 100.0, None, None).await;
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap();

    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
    assert_eq!(state_count(&db, link_id).await, 0);
}

/// REQ-008: malformed sidecars return KashUnreadable and leave existing links intact.
#[tokio::test]
async fn establish_kash_link_malformed_sidecar_leaves_existing_link_intact() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    raw_insert_link(
        &db,
        seed.audio_item_id,
        seed.ebook_item_id,
        3600.0,
        &sha256_hex(EPUB_BYTES),
    )
    .await;
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, b"{not-json");
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    // Expected red source today: establish_kash_link todo!() after sidecar detection.
    let err = establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap_err();

    assert_matches!(err, KashLinkError::KashUnreadable);
    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 1);
}

/// REQ-002: if the matching ebook is already linked to another audio, the original link wins.
#[tokio::test]
async fn establish_kash_link_ebook_already_linked_keeps_original_link() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let audio_root_id = root_id_for(&db, MediaType::Audiobook).await;
    let other_audio_id = add_library_item(
        &db,
        &seed,
        audio_root_id,
        OTHER_AUDIO_REL,
        MediaType::Audiobook,
        100_000,
    )
    .await;
    raw_insert_link(
        &db,
        other_audio_id,
        seed.ebook_item_id,
        3600.0,
        &sha256_hex(EPUB_BYTES),
    )
    .await;
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);

    // Expected red source today: establish_kash_link todo!() after sidecar detection.
    establish_kash_link(
        &db,
        seed.user_id,
        seed.audio_item_id,
        &audio_abs,
        seed.work_id,
        3600.0,
    )
    .await
    .unwrap();

    assert_eq!(link_count_for_audio(&db, other_audio_id).await, 1);
    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
}

/// R-007/REQ-001: manual import chapter hook establishes kash links without opening the m4b.
#[tokio::test]
async fn manual_extract_chapters_for_item_establishes_kash_link() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    db.update_chapter_scan_result(seed.audio_item_id, "scanned", Some(3600.0))
        .await
        .unwrap();
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);
    assert!(!audio_abs.exists(), "fixture must not create the m4b file");

    // Expected red source today: manual wiring still depends on parsing the m4b
    // target, so the absent audio file prevents link establishment.
    make_workflow(db.clone())
        .extract_chapters_for_item(
            seed.audio_item_id,
            &audio_abs,
            MediaType::Audiobook,
            seed.user_id,
            seed.work_id,
        )
        .await;

    let link = db.link_for_item(seed.audio_item_id).await.unwrap();
    assert_eq!(link.map(|l| l.ebook_item_id), Some(seed.ebook_item_id));
}

/// R-007/REQ-001: malformed kash sidecars are warning-only on the manual import path.
#[tokio::test]
async fn manual_extract_chapters_for_item_kash_failure_does_not_panic_or_link() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, b"{not-json");
    db.update_chapter_scan_result(seed.audio_item_id, "scanned", Some(3600.0))
        .await
        .unwrap();
    let audio_abs = abs_path(seed.audio_root.path(), AUDIO_REL);
    assert!(!audio_abs.exists(), "fixture must not create the m4b file");

    // The post-S5 contract is no panic and no link row; no #[should_panic].
    make_workflow(db.clone())
        .extract_chapters_for_item(
            seed.audio_item_id,
            &audio_abs,
            MediaType::Audiobook,
            seed.user_id,
            seed.work_id,
        )
        .await;

    assert_eq!(link_count_for_audio(&db, seed.audio_item_id).await, 0);
}

// ============================================================================
// GROUP E: cross-format service
// ============================================================================

async fn seed_valid_files_and_link(db: &SqliteDb, seed: &Seed) -> i64 {
    write_file(seed.ebook_root.path(), EBOOK_REL, EPUB_BYTES);
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, &valid_kash_bytes());
    db.update_chapter_scan_result(seed.audio_item_id, "scanned", Some(3600.0))
        .await
        .unwrap();
    raw_insert_link(
        db,
        seed.audio_item_id,
        seed.ebook_item_id,
        3600.0,
        &sha256_hex(EPUB_BYTES),
    )
    .await
}

/// AC-004/AC-013: audio progress ahead of ebook yields a CFI prompt with chapter label.
#[tokio::test]
async fn resume_prompt_ebook_direction_returns_cfi_and_chapter_label() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 20.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(prompt.format, MediaType::Ebook);
    assert_eq!(prompt.position, "epubcfi(/6/4!/4/2)");
    assert!(prompt.label.contains("Chapter 1"));
    assert!(prompt.label.contains('%'));
}

/// AC-014: ebook progress ahead of audio yields seconds position and H:MM:SS-style label.
#[tokio::test]
async fn resume_prompt_audio_direction_returns_seconds_and_timestamp_label() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 20.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.audio_item_id, 0.0)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(prompt.format, MediaType::Audiobook);
    assert_eq!(prompt.position, "20");
    assert!(prompt.label.contains("0:20"));
}

/// AC-008: unlinked items silently return no prompt.
#[tokio::test]
async fn resume_prompt_unlinked_returns_none() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap();

    assert!(prompt.is_none());
}

/// REQ-003: a valid fresh link with no cross-format progress does not prompt.
#[tokio::test]
async fn resume_prompt_fresh_link_without_progress_returns_none() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    seed_valid_files_and_link(&db, &seed).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap();

    assert!(prompt.is_none());
}

/// AC-009/REQ-008: duration drift makes prompts silently disappear.
#[tokio::test]
async fn resume_prompt_duration_drift_returns_none() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    db.update_chapter_scan_result(
        seed.audio_item_id,
        "scanned",
        Some(3600.0 + DURATION_TOLERANCE_SECS + 0.1),
    )
    .await
    .unwrap();
    raw_insert_state(&db, seed.user_id, link_id, 20.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap();

    assert!(prompt.is_none());
}

/// REQ-008: epub bytes whose hash no longer matches make prompts silently disappear.
#[tokio::test]
async fn resume_prompt_epub_hash_mismatch_returns_none() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    write_file(seed.ebook_root.path(), EBOOK_REL, b"changed epub bytes");
    raw_insert_state(&db, seed.user_id, link_id, 20.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap();

    assert!(prompt.is_none());
}

/// AC-007: no prompt when furthest resolves at or behind the opened position.
#[tokio::test]
async fn resume_prompt_none_when_furthest_not_strictly_ahead() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 20.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, seed.ebook_item_id, 20.0)
        .await
        .unwrap();

    assert!(prompt.is_none());
}

/// AC-006/REQ-017: decline suppresses until genuine progress advances beyond the threshold.
#[tokio::test]
async fn decline_suppresses_until_furthest_advances() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 20.0, None, None).await;
    let svc = service(db.clone());

    // Expected red source today: CrossFormatServiceImpl::decline_resume todo!().
    svc.decline_resume(seed.user_id, seed.ebook_item_id)
        .await
        .unwrap();
    assert!(svc
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap()
        .is_none());

    db.upsert_progress(
        seed.user_id,
        seed.audio_item_id,
        "40.0",
        0.50,
        ProgressKind::Progress,
        Some(40.0),
    )
    .await
    .unwrap();
    assert!(svc
        .resume_prompt(seed.user_id, seed.ebook_item_id, 0.0)
        .await
        .unwrap()
        .is_some());
}

/// AC-003: links in the same work are isolated.
#[tokio::test]
async fn two_links_in_one_work_are_isolated() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let audio_root_id = root_id_for(&db, MediaType::Audiobook).await;
    let ebook_root_id = root_id_for(&db, MediaType::Ebook).await;
    let other_audio_id = add_library_item(
        &db,
        &seed,
        audio_root_id,
        OTHER_AUDIO_REL,
        MediaType::Audiobook,
        100_000,
    )
    .await;
    let other_ebook_id = add_library_item(
        &db,
        &seed,
        ebook_root_id,
        OTHER_EBOOK_REL,
        MediaType::Ebook,
        OTHER_EPUB_BYTES.len() as i64,
    )
    .await;
    let link_a = seed_valid_files_and_link(&db, &seed).await;
    let link_b = raw_insert_link(
        &db,
        other_audio_id,
        other_ebook_id,
        3600.0,
        &sha256_hex(OTHER_EPUB_BYTES),
    )
    .await;
    raw_insert_state(&db, seed.user_id, link_a, 20.0, None, None).await;
    raw_insert_state(&db, seed.user_id, link_b, 0.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::resume_prompt todo!().
    let prompt = service(db)
        .resume_prompt(seed.user_id, other_ebook_id, 0.0)
        .await
        .unwrap();

    assert!(prompt.is_none());
}

/// REQ-015/REQ-008: anchors are returned in sidecar order for valid links and errors are precise.
#[tokio::test]
async fn anchors_for_item_returns_alignment_or_precise_errors() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    seed_valid_files_and_link(&db, &seed).await;
    let svc = service(db.clone());

    // Expected red source today: CrossFormatServiceImpl::anchors_for_item todo!().
    let anchors = svc
        .anchors_for_item(seed.user_id, seed.ebook_item_id)
        .await
        .unwrap();
    assert_eq!(
        anchors.iter().map(|a| a.ts).collect::<Vec<_>>(),
        vec![10.0, 20.0]
    );

    let unlinked = svc
        .anchors_for_item(seed.user_id, seed.ebook_item_id + 99_999)
        .await
        .unwrap_err();
    assert_matches!(unlinked, CrossFormatError::NotLinked);

    db.update_chapter_scan_result(
        seed.audio_item_id,
        "scanned",
        Some(3600.0 + DURATION_TOLERANCE_SECS + 0.1),
    )
    .await
    .unwrap();
    let drifted = svc
        .anchors_for_item(seed.user_id, seed.ebook_item_id)
        .await
        .unwrap_err();
    assert_matches!(drifted, CrossFormatError::LinkStale);

    db.update_chapter_scan_result(seed.audio_item_id, "scanned", Some(3600.0))
        .await
        .unwrap();
    write_kash_for_audio(seed.audio_root.path(), AUDIO_REL, b"{not-json");
    let malformed = svc
        .anchors_for_item(seed.user_id, seed.ebook_item_id)
        .await
        .unwrap_err();
    assert_matches!(malformed, CrossFormatError::KashUnreadable);
}

/// AC-021: a different user's probe is scoped as NotLinked.
#[tokio::test]
async fn foreign_user_item_is_not_linked() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    seed_valid_files_and_link(&db, &seed).await;
    let other = db
        .create_user(CreateUserDbRequest {
            username: "foreign-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            api_key_hash: "foreign-api".to_string(),
        })
        .await
        .unwrap();

    // Expected red source today: CrossFormatServiceImpl::anchors_for_item todo!().
    let err = service(db)
        .anchors_for_item(other.id, seed.ebook_item_id)
        .await
        .unwrap_err();

    assert_matches!(err, CrossFormatError::NotLinked);
}

/// AC-016/REQ-018: sync_to_here stores the nearest anchor at or before current_ts.
#[tokio::test]
async fn sync_to_here_stores_preceding_anchor_even_when_decreasing() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 300.0, Some(300.0), Some(300.0)).await;

    // Expected red source today: CrossFormatServiceImpl::sync_to_here todo!().
    service(db.clone())
        .sync_to_here(seed.user_id, seed.audio_item_id, 25.0)
        .await
        .unwrap();

    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();
    assert_eq!(state.furthest_ts, 20.0);
    assert_eq!(state.ebook_declined_at_ts, None);
    assert_eq!(state.audio_declined_at_ts, None);
}

/// REQ-018: sync before the first anchor stores the audio start (0 seconds).
#[tokio::test]
async fn sync_to_here_before_first_anchor_stores_zero() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 300.0, Some(300.0), Some(300.0)).await;

    // Expected red source today: CrossFormatServiceImpl::sync_to_here todo!().
    service(db.clone())
        .sync_to_here(seed.user_id, seed.audio_item_id, 1.0)
        .await
        .unwrap();

    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();
    assert_eq!(state.furthest_ts, 0.0);
    assert_eq!(state.ebook_declined_at_ts, None);
    assert_eq!(state.audio_declined_at_ts, None);
}

/// AC-006/REQ-017: decline_resume records the threshold for the opened item's format only.
#[tokio::test]
async fn decline_resume_records_opened_format_only() {
    let db = create_test_db().await;
    let seed = seed_library(&db).await;
    let link_id = seed_valid_files_and_link(&db, &seed).await;
    raw_insert_state(&db, seed.user_id, link_id, 40.0, None, None).await;

    // Expected red source today: CrossFormatServiceImpl::decline_resume todo!().
    service(db.clone())
        .decline_resume(seed.user_id, seed.audio_item_id)
        .await
        .unwrap();

    let state = db.get_or_default(seed.user_id, link_id).await.unwrap();
    assert_eq!(state.furthest_ts, 40.0);
    assert_eq!(state.ebook_declined_at_ts, None);
    assert_eq!(state.audio_declined_at_ts, Some(40.0));
}

// ============================================================================
// GROUP F: HTTP handlers
// ============================================================================

#[derive(Clone)]
struct StubCrossFormatService;

impl CrossFormatService for StubCrossFormatService {
    async fn resume_prompt(
        &self,
        _user_id: i64,
        _library_item_id: i64,
        _current_ts: f64,
    ) -> Result<Option<ResumePrompt>, CrossFormatError> {
        Ok(None)
    }

    async fn anchors_for_item(
        &self,
        _user_id: i64,
        _library_item_id: i64,
    ) -> Result<Vec<AlignmentEntry>, CrossFormatError> {
        Err(CrossFormatError::NotLinked)
    }

    async fn decline_resume(
        &self,
        _user_id: i64,
        _library_item_id: i64,
    ) -> Result<(), CrossFormatError> {
        Err(CrossFormatError::NotLinked)
    }

    async fn sync_to_here(
        &self,
        _user_id: i64,
        _library_item_id: i64,
        _current_ts: f64,
    ) -> Result<(), CrossFormatError> {
        Err(CrossFormatError::NotLinked)
    }
}

#[derive(Clone)]
struct HandlerFileService {
    db: SqliteDb,
}

impl HandlerFileService {
    fn real(&self) -> FileServiceImpl<SqliteDb> {
        FileServiceImpl::new(self.db.clone())
    }
}

impl FileService for HandlerFileService {
    async fn list(&self, user_id: UserId) -> Result<Vec<LibraryItem>, FileServiceError> {
        self.real().list(user_id).await
    }

    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<LibraryItem>, i64), FileServiceError> {
        self.real().list_paginated(user_id, page, page_size).await
    }

    async fn get(&self, user_id: UserId, item_id: i64) -> Result<LibraryItem, FileServiceError> {
        self.real().get(user_id, item_id).await
    }

    async fn delete(&self, user_id: UserId, item_id: i64) -> Result<(), FileServiceError> {
        self.real().delete(user_id, item_id).await
    }

    async fn resolve_path(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<PathBuf, FileServiceError> {
        self.real().resolve_path(user_id, item_id).await
    }

    async fn prepare_email(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<EmailPayload, FileServiceError> {
        self.real().prepare_email(user_id, item_id).await
    }

    async fn get_progress(
        &self,
        user_id: UserId,
        item_id: i64,
    ) -> Result<Option<PlaybackProgress>, FileServiceError> {
        self.real().get_progress(user_id, item_id).await
    }

    async fn update_progress(
        &self,
        user_id: UserId,
        item_id: i64,
        position: &str,
        progress_pct: f64,
        kind: ProgressKind,
        cross_format_ts: Option<f64>,
    ) -> Result<(), FileServiceError> {
        self.real()
            .update_progress(
                user_id,
                item_id,
                position,
                progress_pct,
                kind,
                cross_format_ts,
            )
            .await
    }

    async fn get_progress_for_items(
        &self,
        user_id: UserId,
        library_item_ids: &[LibraryItemId],
    ) -> Result<Vec<ItemProgress>, FileServiceError> {
        self.real()
            .get_progress_for_items(user_id, library_item_ids)
            .await
    }
}

#[derive(Clone)]
struct HandlerTestState {
    cross_format_service: StubCrossFormatService,
    file_service: HandlerFileService,
}

impl HandlerTestState {
    fn new(db: SqliteDb) -> Self {
        Self {
            cross_format_service: StubCrossFormatService,
            file_service: HandlerFileService { db },
        }
    }
}

impl HasCrossFormatService for HandlerTestState {
    type CrossFormatSvc = StubCrossFormatService;

    fn cross_format_service(&self) -> &Self::CrossFormatSvc {
        &self.cross_format_service
    }
}

impl HasFileService for HandlerTestState {
    type FileSvc = HandlerFileService;

    fn file_service(&self) -> &Self::FileSvc {
        &self.file_service
    }
}

fn cross_format_router(db: SqliteDb) -> Router {
    Router::new()
        .route(
            "/workfile/{id}/cross-format/prompt",
            get(livrarr_handlers::cross_format::get_resume_prompt::<HandlerTestState>),
        )
        .route(
            "/workfile/{id}/cross-format/anchors",
            get(livrarr_handlers::cross_format::get_anchors::<HandlerTestState>),
        )
        .route(
            "/workfile/{id}/cross-format/decline",
            post(livrarr_handlers::cross_format::post_decline::<HandlerTestState>),
        )
        .route(
            "/workfile/{id}/cross-format/sync",
            post(livrarr_handlers::cross_format::post_sync_to_here::<HandlerTestState>),
        )
        .with_state(HandlerTestState::new(db))
}

fn workfile_progress_router(db: SqliteDb) -> Router {
    Router::new()
        .route(
            "/workfile/{id}/progress",
            put(livrarr_handlers::workfile::update_progress::<HandlerTestState>),
        )
        .with_state(HandlerTestState::new(db))
}

fn handler_auth_context() -> AuthContext {
    handler_auth_context_for_user(7)
}

fn handler_auth_context_for_user(user_id: i64) -> AuthContext {
    let now = chrono::Utc::now();
    AuthContext {
        user: User {
            id: user_id,
            username: "handler-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            api_key_hash: "api".to_string(),
            setup_pending: false,
            created_at: now,
            updated_at: now,
        },
        auth_type: AuthType::Session,
        session_token_hash: Some("token".to_string()),
    }
}

async fn call_cross_format_handler(app: &Router, method: Method, uri: &str) -> Response<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(handler_auth_context());

    app.clone().oneshot(request).await.unwrap()
}

async fn call_json_handler_as(
    app: &Router,
    method: Method,
    uri: &str,
    body: serde_json::Value,
    auth: AuthContext,
) -> Response<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(auth);

    app.clone().oneshot(request).await.unwrap()
}

async fn response_json(response: Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// handlers-cross-format: unlinked prompt is a successful null-body fallback.
#[tokio::test]
async fn cross_format_prompt_unlinked_returns_200_null() {
    let app = cross_format_router(create_test_db().await);

    let response = call_cross_format_handler(
        &app,
        Method::GET,
        "/workfile/42/cross-format/prompt?current_ts=0",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!(null));
}

/// handlers-cross-format: unlinked non-prompt operations map NotLinked to 404.
#[tokio::test]
async fn cross_format_unlinked_anchors_decline_and_sync_return_404() {
    let app = cross_format_router(create_test_db().await);

    let anchors =
        call_cross_format_handler(&app, Method::GET, "/workfile/42/cross-format/anchors").await;
    assert_eq!(anchors.status(), StatusCode::NOT_FOUND);

    let decline =
        call_cross_format_handler(&app, Method::POST, "/workfile/42/cross-format/decline").await;
    assert_eq!(decline.status(), StatusCode::NOT_FOUND);

    let sync = call_cross_format_handler(
        &app,
        Method::POST,
        "/workfile/42/cross-format/sync?current_ts=0",
    )
    .await;
    assert_eq!(sync.status(), StatusCode::NOT_FOUND);
}

/// handlers-cross-format: prompt rejects negative and non-finite current_ts.
#[tokio::test]
async fn cross_format_prompt_invalid_current_ts_returns_400() {
    let app = cross_format_router(create_test_db().await);

    let negative = call_cross_format_handler(
        &app,
        Method::GET,
        "/workfile/42/cross-format/prompt?current_ts=-1",
    )
    .await;
    assert_eq!(negative.status(), StatusCode::BAD_REQUEST);

    let nan = call_cross_format_handler(
        &app,
        Method::GET,
        "/workfile/42/cross-format/prompt?current_ts=NaN",
    )
    .await;
    assert_eq!(nan.status(), StatusCode::BAD_REQUEST);
}

/// handlers-cross-format: sync rejects negative and non-finite current_ts.
#[tokio::test]
async fn cross_format_sync_invalid_current_ts_returns_400() {
    let app = cross_format_router(create_test_db().await);

    let negative = call_cross_format_handler(
        &app,
        Method::POST,
        "/workfile/42/cross-format/sync?current_ts=-1",
    )
    .await;
    assert_eq!(negative.status(), StatusCode::BAD_REQUEST);

    let nan = call_cross_format_handler(
        &app,
        Method::POST,
        "/workfile/42/cross-format/sync?current_ts=NaN",
    )
    .await;
    assert_eq!(nan.status(), StatusCode::BAD_REQUEST);
}

/// R-005/REQ-003: legacy progress bodies omit kind/ts and default safely to Seek.
#[tokio::test]
async fn update_progress_legacy_body_defaults_to_seek_and_succeeds() {
    let db = create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "handler-route-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            api_key_hash: "handler-route-api".to_string(),
        })
        .await
        .unwrap();
    let author = db
        .create_author(CreateAuthorDbRequest {
            user_id: user.id,
            name: "Handler Author".to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .unwrap();
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id: user.id,
            title: "Handler Work".to_string(),
            author_name: "Handler Author".to_string(),
            normalized_title: "handler work".to_string(),
            normalized_author: "handler author".to_string(),
            author_id: Some(author.id),
            monitor_ebook: true,
            monitor_audiobook: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let root_row = db
        .create_root_folder(root.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id: user.id,
            work_id: work.id,
            root_folder_id: root_row.id,
            path: EBOOK_REL.to_string(),
            media_type: MediaType::Ebook,
            file_size: EPUB_BYTES.len() as i64,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap();
    let app = workfile_progress_router(db.clone());

    let response = call_json_handler_as(
        &app,
        Method::PUT,
        &format!("/workfile/{}/progress", item.id),
        json!({"position": "epubcfi(/6/2!/4/2)", "progress_pct": 0.5}),
        handler_auth_context_for_user(user.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, json!({"success": true}));
    let progress = db.get_progress(user.id, item.id).await.unwrap().unwrap();
    assert_eq!(progress.position, "epubcfi(/6/2!/4/2)");
    assert_eq!(progress.progress_pct, 0.5);
}
