#![allow(dead_code)]

//! RED behavioral tests for work-history deletion events.

mod common;

use common::create_test_db;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateImportDbRequest, CreateLibraryItemDbRequest,
    CreateUserDbRequest, CreateWorkDbRequest, HistoryDb, HistoryFilter, ImportDb, LibraryItemDb,
    RootFolderDb, TagStatus, UserDb, WorkDbCreate,
};
use livrarr_domain::services::{FileService, ManualImportService, WorkService};
use livrarr_domain::{EventType, HistoryEvent, MediaType, UserId, UserRole, WorkId};
use livrarr_library::file_service::FileServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::manual_import_service::ManualImportServiceImpl;

const WORK_TITLE: &str = "History Delete Work";
const WORK_AUTHOR: &str = "History Delete Author";

async fn seed_user(db: &livrarr_db::sqlite::SqliteDb) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: "wh-user".to_string(),
        password_hash: "hash".to_string(),
        role: UserRole::Admin,
        api_key_hash: "wh-api-key".to_string(),
    })
    .await
    .unwrap()
    .id
}

fn work_req(user_id: UserId, title: &str, import_id: Option<&str>) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: WORK_AUTHOR.to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(WORK_AUTHOR),
        import_id: import_id.map(str::to_string),
        ..Default::default()
    }
}

async fn seed_work(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, title: &str) -> WorkId {
    db.create_work(work_req(user_id, title, None))
        .await
        .unwrap()
        .0
        .id
}

async fn seed_library_item(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    path: &str,
    media_type: MediaType,
    import_id: Option<&str>,
) -> i64 {
    let root_path = format!("/tmp/livrarr-wh-{}", path.replace('/', "-"));
    // root_folders.media_type is UNIQUE — a second item of the same type must
    // reuse the existing root instead of creating another.
    let root = match db.create_root_folder(&root_path, media_type).await {
        Ok(root) => root,
        Err(_) => db
            .list_root_folders()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.media_type == media_type)
            .expect("existing root folder of this media type"),
    };
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: root.id,
        path: path.to_string(),
        media_type,
        file_size: 1024,
        import_id: import_id.map(str::to_string),
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap()
    .id
}

async fn seed_prior_history(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, work_id: WorkId) {
    db.create_history_event(CreateHistoryEventDbRequest {
        user_id,
        work_id: Some(work_id),
        event_type: EventType::Imported,
        data: serde_json::json!({
            "work_title": WORK_TITLE,
            "path": "prior.epub",
            "media_type": "ebook"
        }),
        date: None,
    })
    .await
    .unwrap();
}

async fn history(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId) -> Vec<HistoryEvent> {
    db.list_history(
        user_id,
        HistoryFilter {
            event_type: None,
            work_id: None,
            start_date: None,
            end_date: None,
        },
    )
    .await
    .unwrap()
}

fn events_of(events: &[HistoryEvent], event_type: EventType) -> Vec<&HistoryEvent> {
    events
        .iter()
        .filter(|event| event.event_type == event_type)
        .collect()
}

fn assert_one_file_deleted(events: &[HistoryEvent], path: &str, media_type: &str) {
    let deleted = events_of(events, EventType::FileDeleted);
    assert_eq!(deleted.len(), 1, "expected exactly one fileDeleted event");
    let event = deleted[0];
    assert_eq!(event.data["path"].as_str(), Some(path));
    assert_eq!(event.data["media_type"].as_str(), Some(media_type));
    assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
    assert!(
        event.data.get("undo").is_none(),
        "non-undo delete payload must omit undo"
    );
}

#[tokio::test]
async fn wh_file_service_delete_records_one_file_deleted() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let work_id = seed_work(&db, user_id, WORK_TITLE).await;
    let library = tempfile::tempdir().expect("library root folder");
    std::fs::create_dir_all(library.path().join("library-road")).unwrap();
    std::fs::write(library.path().join("library-road/book.epub"), b"epub").unwrap();
    let root = db
        .create_root_folder(library.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .unwrap();
    let item_id = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id,
            root_folder_id: root.id,
            path: "library-road/book.epub".to_string(),
            media_type: MediaType::Ebook,
            file_size: 1024,
            import_id: None,
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .unwrap()
        .id;

    FileServiceImpl::new(db.clone())
        .delete(user_id, item_id)
        .await
        .unwrap();

    let events = history(&db, user_id).await;
    assert_one_file_deleted(&events, "library-road/book.epub", "ebook");
    assert!(
        matches!(
            FileServiceImpl::new(db.clone()).get(user_id, item_id).await,
            Err(livrarr_domain::services::FileServiceError::NotFound)
        ),
        "the library item record is gone"
    );
}

#[tokio::test]
async fn wh_manual_import_delete_library_item_records_one_file_deleted() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let work_id = seed_work(&db, user_id, WORK_TITLE).await;
    let item_id = seed_library_item(
        &db,
        user_id,
        work_id,
        "manual-road/book.m4b",
        MediaType::Audiobook,
        None,
    )
    .await;

    ManualImportServiceImpl::new(db.clone())
        .delete_library_item(user_id, item_id)
        .await
        .unwrap();

    let events = history(&db, user_id).await;
    assert_one_file_deleted(&events, "manual-road/book.m4b", "audiobook");
}

// REQ-005 door (c) "secondary API delete" — NO TEST, BY DISPOSITION (tests-review
// r1, google R-1 CONFIRMED and extended): the test as authored drove the raw
// `SqliteDb::delete_library_item` seam and asserted a fileDeleted event there,
// which the design forbids (writers live at the doors, never inside DB methods —
// ir-v2 D-WRITE-PATH). Deeper: `api_secondary_impl` is `#[cfg(test)]` in
// livrarr-server's lib.rs, `SecondaryApiImpl` has zero production references, and
// no other type implements `LibraryFileApi` — the "door" has no production
// surface today (same shape as the proven-empty REQ-008 door (d)). No writer is
// built for it and no event can be pinned; flagged to the PO as a spec-level
// stale enumeration. Any future productionization of the secondary API must add
// the fileDeleted writer, its doors row, and this test in the same change.

#[tokio::test]
async fn wh_work_delete_records_one_unattached_work_deleted_and_preserves_prior_history() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let work_id = seed_work(&db, user_id, WORK_TITLE).await;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "work-delete/one.epub",
        MediaType::Ebook,
        None,
    )
    .await;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "work-delete/two.epub",
        MediaType::Ebook,
        None,
    )
    .await;
    seed_prior_history(&db, user_id, work_id).await;

    let svc = WorkServiceImpl::new(
        db.clone(),
        livrarr_behavioral::stubs::StubEnrichmentWorkflow::succeeding(),
        livrarr_behavioral::stubs::StubHttpFetcher::new(),
        tempfile::tempdir().expect("test data dir").keep(),
    );
    svc.delete(user_id, work_id, false).await.unwrap();

    let events = history(&db, user_id).await;
    assert_eq!(
        events_of(&events, EventType::FileDeleted).len(),
        0,
        "whole-work delete is composite-only"
    );

    let work_deleted = events_of(&events, EventType::WorkDeleted);
    assert_eq!(work_deleted.len(), 1, "expected one workDeleted event");
    let event = work_deleted[0];
    assert_eq!(event.work_id, None, "workDeleted row must end unattached");
    assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
    assert_eq!(event.data["work_author"].as_str(), Some(WORK_AUTHOR));
    assert_eq!(
        event.data["files_removed"].as_u64(),
        Some(0),
        "files_removed counts files actually removed; neither item has a file on disk"
    );
    assert!(event.data.get("undo").is_none());

    let prior = events
        .iter()
        .find(|event| event.event_type == EventType::Imported)
        .expect("prior history remains listable after ON DELETE SET NULL");
    assert_eq!(prior.work_id, None);
    assert_eq!(prior.data["work_title"].as_str(), Some(WORK_TITLE));
}

#[tokio::test]
async fn wh_readarr_import_undo_marks_file_and_orphan_work_deletions_as_undo() {
    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let import_id = "wh-undo-import-1";

    db.create_import(CreateImportDbRequest {
        id: import_id.to_string(),
        user_id,
        source: "readarr".to_string(),
        source_url: None,
        target_root_folder_id: None,
    })
    .await
    .unwrap();
    // create_import always lands as "running"; undo refuses a running import.
    db.update_import_status(import_id, "completed")
        .await
        .unwrap();

    let work_id = db
        .create_work(work_req(user_id, WORK_TITLE, Some(import_id)))
        .await
        .unwrap()
        .0
        .id;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "undo/one.epub",
        MediaType::Ebook,
        Some(import_id),
    )
    .await;
    seed_library_item(
        &db,
        user_id,
        work_id,
        "undo/two.m4b",
        MediaType::Audiobook,
        Some(import_id),
    )
    .await;

    let service = livrarr_server::readarr_import_service::LiveReadarrImportService::new(db.clone());
    let tmp = tempfile::tempdir().expect("test data dir");

    let response = livrarr_server::readarr_import_workflow::undo_import(
        &service,
        tmp.path(),
        &db,
        user_id,
        import_id,
    )
    .await
    .unwrap();

    assert_eq!(
        response.works_deleted, 1,
        "fixture-reality: 1 orphan work removed"
    );

    let events = history(&db, user_id).await;

    let file_deleted = events_of(&events, EventType::FileDeleted);
    assert_eq!(
        file_deleted.len(),
        2,
        "expected one fileDeleted per undone item"
    );
    for event in &file_deleted {
        let path = event.data["path"].as_str().expect("path");
        let expected_media_type = match path {
            "undo/one.epub" => "ebook",
            "undo/two.m4b" => "audiobook",
            other => panic!("unexpected path in fileDeleted event: {other}"),
        };
        assert_eq!(event.data["media_type"].as_str(), Some(expected_media_type));
        assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
        assert_eq!(event.data["undo"].as_bool(), Some(true));
    }

    let work_deleted = events_of(&events, EventType::WorkDeleted);
    assert_eq!(
        work_deleted.len(),
        1,
        "expected one workDeleted for the orphaned work"
    );
    let event = work_deleted[0];
    assert_eq!(event.work_id, None, "workDeleted row must end unattached");
    assert_eq!(event.data["work_title"].as_str(), Some(WORK_TITLE));
    assert_eq!(event.data["files_removed"].as_u64(), Some(0));
    assert_eq!(event.data["undo"].as_bool(), Some(true));

    assert_eq!(
        (response.files_deleted, response.files_skipped),
        (0, 2),
        "fixture-reality: neither item's root folder exists on disk, so both files are skipped"
    );
}

// ---------------------------------------------------------------------------
// Readarr import undo removes files only inside each item's own root folder
// ---------------------------------------------------------------------------

/// A completed Readarr import with the given target root folder.
async fn completed_import(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    import_id: &str,
    target_root_folder_id: Option<i64>,
) {
    db.create_import(CreateImportDbRequest {
        id: import_id.to_string(),
        user_id,
        source: "readarr".to_string(),
        source_url: None,
        target_root_folder_id,
    })
    .await
    .unwrap();
    db.update_import_status(import_id, "completed")
        .await
        .unwrap();
}

async fn import_work(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    import_id: &str,
    title: &str,
) -> WorkId {
    db.create_work(work_req(user_id, title, Some(import_id)))
        .await
        .unwrap()
        .0
        .id
}

/// A library item of `import_id` under the root folder `root_id`.
async fn seed_item_under(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    root_id: i64,
    path: &str,
    media_type: MediaType,
    import_id: &str,
) {
    db.create_library_item(CreateLibraryItemDbRequest {
        user_id,
        work_id,
        root_folder_id: root_id,
        path: path.to_string(),
        media_type,
        file_size: 1024,
        import_id: Some(import_id.to_string()),
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    })
    .await
    .unwrap();
}

async fn undo(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    import_id: &str,
) -> livrarr_domain::readarr::ReadarrUndoResponse {
    let service = livrarr_server::readarr_import_service::LiveReadarrImportService::new(db.clone());
    let data_dir = tempfile::tempdir().expect("test data dir");
    livrarr_server::readarr_import_workflow::undo_import(
        &service,
        data_dir.path(),
        db,
        user_id,
        import_id,
    )
    .await
    .unwrap()
}

fn write_file(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"fixture").unwrap();
}

/// Present on disk without following a final link.
fn on_disk(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Collects every failed check so one run reports all of them.
#[derive(Default)]
struct Findings(Vec<String>);

impl Findings {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if !ok {
            self.0.push(what());
        }
    }

    fn finish(self, case: &str) {
        assert!(
            self.0.is_empty(),
            "{case}: {} check(s) failed:\n- {}",
            self.0.len(),
            self.0.join("\n- ")
        );
    }
}

/// Captured WARN lines that mention `path`.
fn warn_lines_naming(path: &str) -> Vec<String> {
    let buf = tracing_test::internal::global_buf().lock().unwrap().clone();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| line.contains(" WARN ") && line.contains(path))
        .map(str::to_owned)
        .collect()
}

#[cfg(unix)]
#[tokio::test]
#[tracing_test::traced_test]
async fn readarr_undo_removes_only_regular_files_inside_the_item_root() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("lib").join("ebooks");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(root.join("1")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let inside = root.join("1/ac201-inside.epub");
    let absolute_inside = root.join("1/ac201-absolute-inside.epub");
    let dotdot_target = tmp.path().join("lib/outside/ac201-dotdot.epub");
    let absolute_outside = outside.join("ac201-absolute.epub");
    let link_target = outside.join("ac201-link-target.epub");
    let link = root.join("1/ac201-link.epub");
    let via_parent_target = outside.join("ac201-via-parent.epub");
    let folder = root.join("1/ac201-folder.epub");
    for file in [
        &inside,
        &absolute_inside,
        &dotdot_target,
        &absolute_outside,
        &link_target,
        &via_parent_target,
    ] {
        write_file(file);
    }
    symlink(&link_target, &link).unwrap();
    symlink(&outside, root.join("1/linkdir")).unwrap();
    std::fs::create_dir_all(folder.join("keep")).unwrap();

    let absolute_inside_path = absolute_inside.to_string_lossy().into_owned();
    let absolute_outside_path = absolute_outside.to_string_lossy().into_owned();
    // (stored path, reason a skipped path is logged with)
    let deleted: [&str; 3] = [
        "1/ac201-inside.epub",
        &absolute_inside_path,
        "1/ac201-missing.epub",
    ];
    let skipped: [(&str, &str); 5] = [
        (
            "../outside/ac201-dotdot.epub",
            "resolves outside the root folder",
        ),
        (&absolute_outside_path, "resolves outside the root folder"),
        ("1/ac201-link.epub", "is a link, not a regular file"),
        (
            "1/linkdir/ac201-via-parent.epub",
            "resolves outside the root folder",
        ),
        ("1/ac201-folder.epub", "is a folder, not a regular file"),
    ];

    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let root_folder = db
        .create_root_folder(&root.to_string_lossy(), MediaType::Ebook)
        .await
        .unwrap();
    let import_id = "ac201-undo-shapes";
    completed_import(&db, user_id, import_id, Some(root_folder.id)).await;
    let work_id = import_work(&db, user_id, import_id, WORK_TITLE).await;
    for path in deleted
        .iter()
        .copied()
        .chain(skipped.iter().map(|(p, _)| *p))
    {
        seed_item_under(
            &db,
            user_id,
            work_id,
            root_folder.id,
            path,
            MediaType::Ebook,
            import_id,
        )
        .await;
    }

    let response = undo(&db, user_id, import_id).await;

    let mut findings = Findings::default();
    for (name, path) in [
        ("../ target outside the root", &dotdot_target),
        ("absolute path outside the root", &absolute_outside),
        ("link target outside the root", &link_target),
        ("file behind a linked parent folder", &via_parent_target),
    ] {
        findings.check(path.exists(), || {
            format!("{name} {} still exists", path.display())
        });
    }
    findings.check(on_disk(&link), || {
        "the link entry itself is kept".to_string()
    });
    findings.check(folder.is_dir(), || "the folder entry is kept".to_string());
    findings.check(!on_disk(&inside), || {
        "the inside file is removed".to_string()
    });
    findings.check(!on_disk(&absolute_inside), || {
        "the absolute path to a file inside the root is removed".to_string()
    });
    findings.check(
        (response.files_deleted, response.files_skipped) == (3, 5),
        || {
            format!(
                "reply counts filesDeleted 3, filesSkipped 5; got {}, {}",
                response.files_deleted, response.files_skipped
            )
        },
    );
    for (path, reason) in skipped {
        let lines = warn_lines_naming(path);
        findings.check(lines.len() == 1 && lines[0].contains(reason), || {
            format!("one WARN line naming {path:?} with reason {reason:?}; got {lines:?}")
        });
    }
    findings.finish("AC-201");
}

/// A folder under the process working folder, removed when dropped.
struct WorkingFolderFixture(std::path::PathBuf);

impl Drop for WorkingFolderFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn readarr_undo_without_an_import_root_uses_each_item_root_and_never_the_working_folder() {
    let unique = format!(
        "zz-ac202-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let stored = format!("{unique}/ac202-book.epub");
    let cwd = std::env::current_dir().unwrap();
    let _guard = WorkingFolderFixture(cwd.join(&unique));
    let working_folder_copy = cwd.join(&stored);
    write_file(&working_folder_copy);

    let tmp = tempfile::tempdir().unwrap();
    let ebook_root = tmp.path().join("ebooks");
    let inside = ebook_root.join(&stored);
    write_file(&inside);
    let missing_audio_root = tmp.path().join("audiobooks-not-on-disk");

    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let ebook = db
        .create_root_folder(&ebook_root.to_string_lossy(), MediaType::Ebook)
        .await
        .unwrap();
    let audio = db
        .create_root_folder(&missing_audio_root.to_string_lossy(), MediaType::Audiobook)
        .await
        .unwrap();
    let import_id = "ac202-no-import-root";
    completed_import(&db, user_id, import_id, None).await;
    let work_id = import_work(&db, user_id, import_id, WORK_TITLE).await;
    seed_item_under(
        &db,
        user_id,
        work_id,
        ebook.id,
        &stored,
        MediaType::Ebook,
        import_id,
    )
    .await;
    seed_item_under(
        &db,
        user_id,
        work_id,
        audio.id,
        &format!("{unique}/ac202-book.m4b"),
        MediaType::Audiobook,
        import_id,
    )
    .await;

    let response = undo(&db, user_id, import_id).await;

    let mut findings = Findings::default();
    findings.check(!on_disk(&inside), || {
        format!(
            "the file under the item's root {} is removed",
            inside.display()
        )
    });
    findings.check(working_folder_copy.exists(), || {
        format!(
            "the file at the same relative path under the working folder {} still exists",
            working_folder_copy.display()
        )
    });
    findings.check(
        (response.files_deleted, response.files_skipped) == (1, 1),
        || {
            format!(
                "the item whose root folder is missing on disk is skipped: expected \
                 filesDeleted 1, filesSkipped 1; got {}, {}",
                response.files_deleted, response.files_skipped
            )
        },
    );
    findings.finish("AC-202");
}

#[tokio::test]
async fn readarr_undo_uses_each_item_own_root_not_the_import_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root_a = tmp.path().join("ebooks");
    let root_b = tmp.path().join("audiobooks");
    let p1 = "1/ac205-ebook.epub";
    let p2 = "1/ac205-audio.m4b";
    for file in [
        root_a.join(p1),
        root_b.join(p2),
        root_a.join(p2),
        root_b.join(p1),
    ] {
        write_file(&file);
    }

    let db = create_test_db().await;
    let user_id = seed_user(&db).await;
    let a = db
        .create_root_folder(&root_a.to_string_lossy(), MediaType::Ebook)
        .await
        .unwrap();
    let b = db
        .create_root_folder(&root_b.to_string_lossy(), MediaType::Audiobook)
        .await
        .unwrap();
    let import_id = "ac205-two-roots";
    completed_import(&db, user_id, import_id, Some(a.id)).await;
    let work_id = import_work(&db, user_id, import_id, WORK_TITLE).await;
    seed_item_under(&db, user_id, work_id, a.id, p1, MediaType::Ebook, import_id).await;
    seed_item_under(
        &db,
        user_id,
        work_id,
        b.id,
        p2,
        MediaType::Audiobook,
        import_id,
    )
    .await;

    let response = undo(&db, user_id, import_id).await;

    let mut findings = Findings::default();
    findings.check(!on_disk(&root_a.join(p1)), || "A/p1 is removed".to_string());
    findings.check(!on_disk(&root_b.join(p2)), || "B/p2 is removed".to_string());
    findings.check(root_a.join(p2).exists(), || {
        "decoy A/p2 still exists".to_string()
    });
    findings.check(root_b.join(p1).exists(), || {
        "decoy B/p1 still exists".to_string()
    });
    findings.check(
        (response.files_deleted, response.files_skipped) == (2, 0),
        || {
            format!(
                "expected filesDeleted 2, filesSkipped 0; got {}, {}",
                response.files_deleted, response.files_skipped
            )
        },
    );
    findings.finish("AC-205");
}
