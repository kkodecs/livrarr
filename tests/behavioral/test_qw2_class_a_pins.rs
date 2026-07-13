use livrarr_db::{
    create_test_db, CreateImportDbRequest, CreateLibraryItemDbRequest, CreateUserDbRequest,
    CreateWorkDbRequest, ImportDb, LibraryItemDb, RootFolderDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{AnchorConfidence, AnchorSetter, AnchorType};
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::{MediaType, QueueStatus, TagStatus, UserRole};
use livrarr_download::classify_qbit_state;

async fn seeded_db() -> (livrarr_db::sqlite::SqliteDb, i64) {
    let db = create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "qw2-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api".to_string(),
        })
        .await
        .expect("user should be created");
    (db, user.id)
}

fn work_req(user_id: i64, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: title.to_ascii_lowercase(),
        normalized_author: author.to_ascii_lowercase(),
        language: Some("en".to_string()),
        ..CreateWorkDbRequest::default()
    }
}

#[tokio::test]
async fn qw2_readarr_import_read_preserves_tag_columns() {
    let (db, user_id) = seeded_db().await;
    let root = db
        .create_root_folder("/tmp/qw2", MediaType::Ebook)
        .await
        .expect("root folder should be created");
    db.create_import(CreateImportDbRequest {
        id: "qw2-imp-1".to_string(),
        user_id,
        source: "readarr".to_string(),
        source_url: None,
        target_root_folder_id: Some(root.id),
    })
    .await
    .expect("import should be created");
    let (work, _) = db
        .create_work(work_req(user_id, "Pinned Work", "Pinned Author"))
        .await
        .expect("work should be created");
    let item = db
        .create_library_item(CreateLibraryItemDbRequest {
            user_id,
            work_id: work.id,
            root_folder_id: root.id,
            path: "Pinned Work.epub".to_string(),
            media_type: MediaType::Ebook,
            file_size: 1234,
            import_id: Some("qw2-imp-1".to_string()),
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
        })
        .await
        .expect("library item should be created");

    db.update_library_item_tag_status(item.id, TagStatus::Synced, 42)
        .await
        .expect("tag status should update");

    let canonical = db
        .get_library_item(user_id, item.id)
        .await
        .expect("canonical read should succeed");
    assert_eq!(canonical.tag_status, TagStatus::Synced);
    assert_eq!(canonical.tagged_at_generation, 42);

    let by_import = db
        .list_library_items_by_import("qw2-imp-1")
        .await
        .expect("import read should succeed");
    assert_eq!(by_import.len(), 1);
    assert_eq!(by_import[0].tag_status, TagStatus::Synced);
    assert_eq!(by_import[0].tagged_at_generation, 42);
}

#[tokio::test]
async fn qw2_work_create_with_anchor_persists_work_and_anchor_rows() {
    let (db, user_id) = seeded_db().await;

    let (work, created) = db
        .create_work_with_anchor(
            work_req(user_id, "Anchored Work", "Anchored Author"),
            "OL999W",
            AnchorSetter::Import,
        )
        .await
        .expect("work and anchor should be created");

    assert!(created);
    let persisted = db
        .get_work(user_id, work.id)
        .await
        .expect("work row should exist");
    assert_eq!(persisted.ol_key.as_deref(), Some("OL999W"));

    let anchors = db
        .list_anchors(work.id)
        .await
        .expect("anchor rows should be readable");
    assert!(anchors.iter().any(|anchor| {
        anchor.anchor_type == AnchorType::new(AnchorType::OL_WORK)
            && anchor.anchor_value == "OL999W"
            && anchor.confidence == AnchorConfidence::Confirmed
    }));
}

#[test]
fn qw2a_classify_qbit_state_pins_ratified_truth_table_rows() {
    let rows = [
        ("downloading", QueueStatus::Downloading, false),
        ("stalledDL", QueueStatus::Downloading, false),
        ("forcedDL", QueueStatus::Downloading, false),
        ("metaDL", QueueStatus::Queued, false),
        ("forcedMetaDL", QueueStatus::Queued, false),
        ("allocating", QueueStatus::Queued, false),
        ("queuedDL", QueueStatus::Queued, false),
        ("checkingDL", QueueStatus::Queued, false),
        ("checkingResumeData", QueueStatus::Queued, false),
        ("pausedDL", QueueStatus::Paused, false),
        ("stoppedDL", QueueStatus::Paused, false),
        ("uploading", QueueStatus::Completed, true),
        ("stalledUP", QueueStatus::Completed, true),
        ("forcedUP", QueueStatus::Completed, true),
        ("queuedUP", QueueStatus::Completed, true),
        ("pausedUP", QueueStatus::Completed, true),
        ("stoppedUP", QueueStatus::Completed, true),
        ("checkingUP", QueueStatus::Queued, false),
        ("moving", QueueStatus::Downloading, false),
        ("missingFiles", QueueStatus::Warning, false),
        ("error", QueueStatus::Error, false),
    ];

    for (state, expected_ui, expected_import_safe) in rows {
        let classification = classify_qbit_state(state);
        assert_eq!(
            classification.ui_status, expected_ui,
            "state {state}: ui projection"
        );
        assert_eq!(
            classification.import_safe, expected_import_safe,
            "state {state}: import_safe projection"
        );
    }
}

#[test]
fn qw2a_classify_qbit_state_pins_fallback_and_case_sensitive_rows() {
    let rows = [
        ("unknown", QueueStatus::Warning, false),
        ("some-new-state", QueueStatus::Warning, false),
        ("", QueueStatus::Warning, false),
        ("DOWNLOADING", QueueStatus::Warning, false),
        ("Error", QueueStatus::Warning, false),
    ];

    for (state, expected_ui, expected_import_safe) in rows {
        let classification = classify_qbit_state(state);
        assert_eq!(
            classification.ui_status, expected_ui,
            "state {state}: ui projection"
        );
        assert_eq!(
            classification.import_safe, expected_import_safe,
            "state {state}: import_safe projection"
        );
    }
}
