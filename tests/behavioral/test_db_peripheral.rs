use librarr_db::{
    test_helpers::{
        new_config_test_db, new_history_test_db, new_notification_test_db, test_user_id,
    },
    ConfigDb, CreateHistoryEventDbRequest, CreateNotificationDbRequest, HistoryDb, HistoryFilter,
    NotificationDb, UpdateMediaManagementConfigRequest, UpdateMetadataConfigRequest,
    UpdateProwlarrConfigRequest,
};
use librarr_domain::{EventType, LlmProvider, NotificationType};
use serde_json::json;

fn setup_notification_db() -> impl NotificationDb {
    new_notification_test_db()
}

fn setup_history_db() -> impl HistoryDb {
    new_history_test_db()
}

fn setup_config_db() -> impl ConfigDb {
    new_config_test_db()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn notif_req(
    ntype: NotificationType,
    ref_key: Option<&str>,
    msg: &str,
) -> CreateNotificationDbRequest {
    CreateNotificationDbRequest {
        user_id: test_user_id(),
        notification_type: ntype,
        ref_key: ref_key.map(String::from),
        message: msg.to_string(),
        data: json!({}),
    }
}

fn empty_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn history_req(event_type: EventType, work_id: Option<i64>) -> CreateHistoryEventDbRequest {
    CreateHistoryEventDbRequest {
        user_id: test_user_id(),
        work_id,
        event_type,
        data: json!({}),
    }
}

// =============================================================================
// NotificationDb — AUTHOR-003, AUTHOR-005
// =============================================================================

#[tokio::test]
async fn test_db_notification_nominal_crud() {
    // Satisfies: AUTHOR-003, AUTHOR-005 — full lifecycle: create, list, mark read, dismiss
    let db = setup_notification_db();
    let uid = test_user_id();

    let created = db
        .create_notification(notif_req(
            NotificationType::NewWorkDetected,
            Some("work:100"),
            "New work",
        ))
        .await
        .unwrap();

    // After create: visible, unread, not dismissed
    let list = db.list_notifications(uid, false).await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(!list[0].read);
    assert!(!list[0].dismissed);

    // Mark read: still visible in all-list, gone from unread-only
    db.mark_notification_read(uid, created.id).await.unwrap();
    assert!(db.list_notifications(uid, true).await.unwrap().is_empty());
    assert_eq!(db.list_notifications(uid, false).await.unwrap().len(), 1);

    // Dismiss: gone from both lists (permanent)
    db.dismiss_notification(uid, created.id).await.unwrap();
    assert!(db.list_notifications(uid, false).await.unwrap().is_empty());
    assert!(db.list_notifications(uid, true).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_db_notification_dedup_active_duplicate() {
    // Satisfies: AUTHOR-003 — second create with same (user, type, ref_key) is a no-op
    let db = setup_notification_db();
    let uid = test_user_id();

    let first = db
        .create_notification(notif_req(
            NotificationType::MetadataUpdated,
            Some("work:200"),
            "First",
        ))
        .await
        .unwrap();

    let second = db
        .create_notification(notif_req(
            NotificationType::MetadataUpdated,
            Some("work:200"),
            "Duplicate",
        ))
        .await
        .unwrap();

    assert_eq!(second.id, first.id);
    assert_eq!(db.list_notifications(uid, false).await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_db_notification_dedup_dismissed_duplicate() {
    // Satisfies: AUTHOR-003, AUTHOR-005 — dismissed blocks re-creation permanently
    let db = setup_notification_db();
    let uid = test_user_id();

    let created = db
        .create_notification(notif_req(
            NotificationType::WorkAutoAdded,
            Some("work:300"),
            "Original",
        ))
        .await
        .unwrap();
    db.dismiss_notification(uid, created.id).await.unwrap();

    let retry = db
        .create_notification(notif_req(
            NotificationType::WorkAutoAdded,
            Some("work:300"),
            "Retry",
        ))
        .await
        .unwrap();

    assert_eq!(retry.id, created.id);
    assert!(db.list_notifications(uid, false).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_db_notification_dismiss_all_and_block_recreation() {
    // Satisfies: AUTHOR-003, AUTHOR-005 — dismiss_all is permanent per ref_key
    let db = setup_notification_db();
    let uid = test_user_id();

    db.create_notification(notif_req(
        NotificationType::NewWorkDetected,
        Some("work:401"),
        "A",
    ))
    .await
    .unwrap();
    db.create_notification(notif_req(
        NotificationType::MetadataUpdated,
        Some("work:402"),
        "B",
    ))
    .await
    .unwrap();
    assert_eq!(db.list_notifications(uid, false).await.unwrap().len(), 2);

    db.dismiss_all_notifications(uid).await.unwrap();
    assert!(db.list_notifications(uid, false).await.unwrap().is_empty());

    // Re-creation blocked by dedup on dismissed rows
    db.create_notification(notif_req(
        NotificationType::NewWorkDetected,
        Some("work:401"),
        "Retry",
    ))
    .await
    .unwrap();
    assert!(db.list_notifications(uid, false).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_db_notification_unread_filter() {
    // Satisfies: AUTHOR-005 — unread_only=true excludes read notifications
    let db = setup_notification_db();
    let uid = test_user_id();

    let n1 = db
        .create_notification(notif_req(
            NotificationType::NewWorkDetected,
            Some("work:501"),
            "Stays unread",
        ))
        .await
        .unwrap();
    let n2 = db
        .create_notification(notif_req(
            NotificationType::MetadataUpdated,
            Some("work:502"),
            "Will be read",
        ))
        .await
        .unwrap();

    db.mark_notification_read(uid, n2.id).await.unwrap();

    let unread = db.list_notifications(uid, true).await.unwrap();
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].id, n1.id);
    assert_eq!(db.list_notifications(uid, false).await.unwrap().len(), 2);
}

#[tokio::test]
async fn test_db_notification_null_ref_key() {
    // Satisfies: AUTHOR-003 — NULL ref_key dedup depends on DB UNIQUE semantics.
    // Contract: both creates succeed without error.
    let db = setup_notification_db();

    let first = db
        .create_notification(notif_req(
            NotificationType::BulkEnrichmentComplete,
            None,
            "First",
        ))
        .await
        .unwrap();
    let second = db
        .create_notification(notif_req(
            NotificationType::BulkEnrichmentComplete,
            None,
            "Second",
        ))
        .await
        .unwrap();

    assert!(first.id > 0);
    assert!(second.id > 0);
}

// =============================================================================
// HistoryDb — spec Section 7
// =============================================================================

#[tokio::test]
async fn test_db_history_create_and_list() {
    // Satisfies: spec Section 7 — append-only event log
    let db = setup_history_db();
    let uid = test_user_id();

    db.create_history_event(CreateHistoryEventDbRequest {
        user_id: uid,
        work_id: Some(1001),
        event_type: EventType::Imported,
        data: json!({ "source": "test" }),
    })
    .await
    .unwrap();

    let events = db.list_history(uid, empty_filter()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].user_id, uid);
    assert_eq!(events[0].work_id, Some(1001));
    assert_eq!(events[0].event_type, EventType::Imported);
}

#[tokio::test]
async fn test_db_history_filter_by_event_type() {
    // Satisfies: spec Section 7
    let db = setup_history_db();
    let uid = test_user_id();

    db.create_history_event(history_req(EventType::Grabbed, Some(1101)))
        .await
        .unwrap();
    db.create_history_event(history_req(EventType::Imported, Some(1102)))
        .await
        .unwrap();

    let events = db
        .list_history(
            uid,
            HistoryFilter {
                event_type: Some(EventType::Imported),
                ..empty_filter()
            },
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::Imported);
}

#[tokio::test]
async fn test_db_history_filter_by_work_id() {
    // Satisfies: spec Section 7
    let db = setup_history_db();
    let uid = test_user_id();

    db.create_history_event(history_req(EventType::DownloadCompleted, Some(1201)))
        .await
        .unwrap();
    db.create_history_event(history_req(EventType::DownloadCompleted, Some(1202)))
        .await
        .unwrap();

    let events = db
        .list_history(
            uid,
            HistoryFilter {
                work_id: Some(1202),
                ..empty_filter()
            },
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].work_id, Some(1202));
}

#[tokio::test]
async fn test_db_history_empty_result_for_non_matching_filter() {
    // Satisfies: spec Section 7
    let db = setup_history_db();
    let uid = test_user_id();

    db.create_history_event(history_req(EventType::Imported, Some(1301)))
        .await
        .unwrap();

    let events = db
        .list_history(
            uid,
            HistoryFilter {
                event_type: Some(EventType::DownloadFailed),
                work_id: Some(999999),
                start_date: None,
                end_date: None,
            },
        )
        .await
        .unwrap();

    assert!(events.is_empty());
}

// =============================================================================
// ConfigDb — CONFIG-001..005, AUTH-004
// =============================================================================

#[tokio::test]
async fn test_db_config_naming_config_read_only_seeded() {
    // Satisfies: CONFIG-001, CONFIG-005, AUTH-004 — naming config is read-only, seeded by migration
    let db = setup_config_db();
    let config = db.get_naming_config().await.unwrap();
    assert!(!config.author_folder_format.is_empty());
    assert!(!config.book_folder_format.is_empty());
}

#[tokio::test]
async fn test_db_config_media_management_round_trip() {
    // Satisfies: CONFIG-002, CONFIG-005, AUTH-004
    let db = setup_config_db();

    let updated = db
        .update_media_management_config(UpdateMediaManagementConfigRequest {
            cwa_ingest_path: Some("/tmp/librarr-ingest".into()),
            preferred_ebook_formats: vec![],
            preferred_audiobook_formats: vec![],
        })
        .await
        .unwrap();
    let re_read = db.get_media_management_config().await.unwrap();

    assert_eq!(updated.cwa_ingest_path, Some("/tmp/librarr-ingest".into()));
    assert_eq!(re_read.cwa_ingest_path, updated.cwa_ingest_path);
}

#[tokio::test]
async fn test_db_config_prowlarr_round_trip() {
    // Satisfies: CONFIG-003, CONFIG-005, AUTH-004
    let db = setup_config_db();

    let updated = db
        .update_prowlarr_config(UpdateProwlarrConfigRequest {
            url: Some("http://localhost:9696".into()),
            api_key: Some("test-key".into()),
            enabled: Some(true),
        })
        .await
        .unwrap();
    let re_read = db.get_prowlarr_config().await.unwrap();

    assert_eq!(updated.url, Some("http://localhost:9696".into()));
    assert!(updated.enabled);
    assert_eq!(re_read.url, updated.url);
    assert_eq!(re_read.api_key, updated.api_key);
    assert_eq!(re_read.enabled, updated.enabled);
}

#[tokio::test]
async fn test_db_config_metadata_round_trip() {
    // Satisfies: CONFIG-004, CONFIG-005, AUTH-004
    let db = setup_config_db();

    let updated = db
        .update_metadata_config(UpdateMetadataConfigRequest {
            hardcover_api_token: Some("hc-token".into()),
            llm_provider: Some(LlmProvider::Openai),
            llm_endpoint: Some("https://api.openai.example".into()),
            llm_api_key: Some("llm-key".into()),
            llm_model: Some("gpt-test".into()),
            audnexus_url: Some("https://audnexus.example".into()),
            languages: Some(vec!["en".into(), "fr".into()]),
        })
        .await
        .unwrap();
    let re_read = db.get_metadata_config().await.unwrap();

    assert_eq!(updated.llm_provider, Some(LlmProvider::Openai));
    assert_eq!(updated.audnexus_url, "https://audnexus.example");
    assert_eq!(updated.languages, vec!["en", "fr"]);
    assert_eq!(re_read.hardcover_api_token, updated.hardcover_api_token);
    assert_eq!(re_read.llm_provider, updated.llm_provider);
    assert_eq!(re_read.llm_endpoint, updated.llm_endpoint);
    assert_eq!(re_read.audnexus_url, updated.audnexus_url);
    assert_eq!(re_read.languages, updated.languages);
}

#[tokio::test]
async fn test_db_config_metadata_partial_update_preserves_other_fields() {
    // Satisfies: CONFIG-004, CONFIG-005, AUTH-004
    let db = setup_config_db();

    // Establish baseline
    let baseline = db
        .update_metadata_config(UpdateMetadataConfigRequest {
            hardcover_api_token: Some("baseline-token".into()),
            llm_provider: Some(LlmProvider::Gemini),
            llm_endpoint: Some("https://baseline-llm.example".into()),
            llm_api_key: Some("baseline-key".into()),
            llm_model: Some("baseline-model".into()),
            audnexus_url: Some("https://baseline-audnexus.example".into()),
            languages: Some(vec!["en".into(), "de".into()]),
        })
        .await
        .unwrap();

    // Partial update: only audnexus_url
    let updated = db
        .update_metadata_config(UpdateMetadataConfigRequest {
            hardcover_api_token: None,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            languages: None,
            audnexus_url: Some("https://changed.example".into()),
        })
        .await
        .unwrap();

    assert_eq!(updated.audnexus_url, "https://changed.example");
    assert_eq!(updated.hardcover_api_token, baseline.hardcover_api_token);
    assert_eq!(updated.llm_provider, baseline.llm_provider);
    assert_eq!(updated.llm_endpoint, baseline.llm_endpoint);
    assert_eq!(updated.languages, baseline.languages);

    let re_read = db.get_metadata_config().await.unwrap();
    assert_eq!(re_read.audnexus_url, "https://changed.example");
    assert_eq!(re_read.llm_provider, baseline.llm_provider);
}
