use chrono::Utc;
use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateHistoryEventDbRequest, CreateWorkDbRequest, HistoryDb, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    normalize_for_matching, EventType, HistoryFilter, IdentityStatus, UserId, Work, WorkId,
};

fn empty_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn work_req(user_id: UserId, title: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Robust Reads Author".to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching("Robust Reads Author"),
        language: Some("en".to_string()),
        monitor_ebook: true,
        monitor_audiobook: true,
        ..Default::default()
    }
}

async fn seed_work(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, title: &str) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title))
        .await
        .expect("seed work");
    assert!(created, "fixture work must be newly created");
    db.set_identity_status(user_id, work.id, IdentityStatus::Confirmed)
        .await
        .expect("seed identity status");
    work
}

async fn seed_event(
    db: &livrarr_db::sqlite::SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    event_type: EventType,
    title: &str,
) {
    db.create_history_event(CreateHistoryEventDbRequest {
        user_id,
        work_id: Some(work_id),
        event_type,
        data: serde_json::json!({"work_title": title}),
        date: None,
    })
    .await
    .expect("seed typed history row");
}

#[tokio::test]
async fn wh_unknown_history_event_type_is_skipped_without_breaking_lists() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let other_user = db
        .create_user(livrarr_db::CreateUserDbRequest {
            username: "robust-reads-other".into(),
            password_hash: "hash".into(),
            role: livrarr_domain::UserRole::Admin,
            api_key_hash: "robust-reads-other-key".into(),
        })
        .await
        .expect("create second user")
        .id;
    let work = seed_work(&db, user_id, "Robust Reads Work").await;
    let other_work = seed_work(&db, other_user, "Other User Work").await;

    seed_event(
        &db,
        user_id,
        work.id,
        EventType::Grabbed,
        "Robust Reads Work",
    )
    .await;
    seed_event(
        &db,
        user_id,
        work.id,
        EventType::Imported,
        "Robust Reads Work",
    )
    .await;
    seed_event(
        &db,
        user_id,
        work.id,
        EventType::Enriched,
        "Robust Reads Work",
    )
    .await;
    seed_event(
        &db,
        other_user,
        other_work.id,
        EventType::Grabbed,
        "Other User Work",
    )
    .await;

    sqlx::query(
        "INSERT INTO history (user_id, work_id, event_type, data, date) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(user_id)
    .bind(work.id)
    .bind("futureExperimentalKind")
    .bind(serde_json::json!({"work_title": "Robust Reads Work"}).to_string())
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("raw insert unknown event kind");

    let rows = db
        .list_history(user_id, empty_filter())
        .await
        .expect("unknown event_type rows must not fail list_history");
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.user_id == user_id));
    assert!(rows.iter().all(|row| row.work_id == Some(work.id)));
    assert!(rows
        .iter()
        .all(|row| row.event_type != EventType::IdentityResolved));

    let (page, total) = db
        .list_history_paginated(user_id, empty_filter(), 1, 20)
        .await
        .expect("unknown event_type rows must not fail list_history_paginated");
    assert_eq!(page.len(), 3);
    assert_eq!(
        total, 4,
        "paginated total intentionally uses raw COUNT(*), so the skipped unknown row still contributes"
    );
}
