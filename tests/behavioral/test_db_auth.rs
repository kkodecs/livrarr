use chrono::{Duration, Utc};
use librarr_db::{
    test_helpers::new_test_db, CompleteSetupDbRequest, CreateUserDbRequest, DbError, SessionDb,
    UpdateUserDbRequest, UserDb,
};
use librarr_domain::{Session, UserRole};

async fn fresh_db() -> impl UserDb + SessionDb {
    new_test_db().await
}

/// Fresh DB bootstrapped with a placeholder admin (setup_pending=true),
/// mimicking first-boot state per AUTH-010.
async fn fresh_db_with_placeholder() -> impl UserDb + SessionDb {
    librarr_db::test_helpers::new_test_db_with_placeholder().await
}

fn create_user_req(username: &str, role: UserRole) -> CreateUserDbRequest {
    CreateUserDbRequest {
        username: username.to_string(),
        password_hash: format!("pw-{username}"),
        role,
        api_key_hash: format!("api-{username}"),
    }
}

// === UserDb ===

#[tokio::test]
async fn test_db_auth_user_create_user_returns_user_with_generated_id() {
    // Satisfies: AUTH-013 — create user persists fields and enforces DB-side uniqueness contract surface
    let db = fresh_db().await;
    let before = Utc::now();

    let user = db
        .create_user(create_user_req("alice", UserRole::Admin))
        .await
        .unwrap();

    let after = Utc::now();
    assert!(user.id > 0);
    assert_eq!(user.username, "alice");
    assert_eq!(user.password_hash, "pw-alice");
    assert_eq!(user.role, UserRole::Admin);
    assert_eq!(user.api_key_hash, "api-alice");
    assert!(!user.setup_pending);
    assert!(user.created_at >= before && user.created_at <= after);
    assert!(user.updated_at >= before && user.updated_at <= after);
}

#[tokio::test]
async fn test_db_auth_user_get_user_retrieves_created_user_by_id() {
    // Satisfies: AUTH-005 — user/session ownership paths require stable user lookup by id
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    let fetched = db.get_user(created.id).await.unwrap();

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.username, created.username);
    assert_eq!(fetched.password_hash, created.password_hash);
    assert_eq!(fetched.role, created.role);
    assert_eq!(fetched.api_key_hash, created.api_key_hash);
    assert_eq!(fetched.setup_pending, created.setup_pending);
}

#[tokio::test]
async fn test_db_auth_user_get_user_nonexistent_returns_not_found() {
    // Satisfies: AUTH-011 — CRUD contract returns NotFound for missing user
    let db = fresh_db().await;

    match db.get_user(999_999).await {
        Err(DbError::NotFound) => {}
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_get_user_by_username_retrieves_case_insensitively() {
    // Satisfies: AUTH-012 — username lookup path used by auth/brute-force logic
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("Alice_User", UserRole::User))
        .await
        .unwrap();

    let fetched = db.get_user_by_username("alice_user").await.unwrap();

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.username, created.username);
}

#[tokio::test]
async fn test_db_auth_user_get_user_by_username_nonexistent_returns_not_found() {
    // Satisfies: AUTH-012 — missing username lookup returns NotFound
    let db = fresh_db().await;

    match db.get_user_by_username("missing").await {
        Err(DbError::NotFound) => {}
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_get_user_by_api_key_hash_retrieves_user() {
    // Satisfies: AUTH-007 — API key hash lookup returns owning user
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    let fetched = db.get_user_by_api_key_hash("api-alice").await.unwrap();

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.username, "alice");
}

#[tokio::test]
async fn test_db_auth_user_get_user_by_api_key_hash_nonexistent_returns_not_found() {
    // Satisfies: AUTH-007 — missing API key hash returns NotFound
    let db = fresh_db().await;

    match db.get_user_by_api_key_hash("missing-hash").await {
        Err(DbError::NotFound) => {}
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_list_users_returns_all_created_users() {
    // Satisfies: AUTH-011 — list users returns persisted user set
    let db = fresh_db().await;
    let u1 = db
        .create_user(create_user_req("alice", UserRole::Admin))
        .await
        .unwrap();
    let u2 = db
        .create_user(create_user_req("bob", UserRole::User))
        .await
        .unwrap();

    let users = db.list_users().await.unwrap();

    assert_eq!(users.len(), 2);
    assert!(users.iter().any(|u| u.id == u1.id && u.username == "alice"));
    assert!(users.iter().any(|u| u.id == u2.id && u.username == "bob"));
}

#[tokio::test]
async fn test_db_auth_user_list_users_empty_returns_empty_vec() {
    // Satisfies: AUTH-011 — empty user store lists as empty
    let db = fresh_db().await;

    let users = db.list_users().await.unwrap();

    assert!(users.is_empty());
}

#[tokio::test]
async fn test_db_auth_user_update_user_changes_some_fields_and_leaves_none_unchanged() {
    // Satisfies: AUTH-011 — partial user update semantics
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    let updated = db
        .update_user(
            created.id,
            UpdateUserDbRequest {
                username: Some("alice2".to_string()),
                password_hash: None,
                role: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.username, "alice2");
    assert_eq!(updated.password_hash, created.password_hash);
    assert_eq!(updated.role, created.role);
    assert_eq!(updated.api_key_hash, created.api_key_hash);
    assert!(updated.updated_at >= created.updated_at);
}

#[tokio::test]
async fn test_db_auth_user_update_user_changes_role() {
    // Satisfies: AUTH-011 — role updates persist
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    let updated = db
        .update_user(
            created.id,
            UpdateUserDbRequest {
                username: None,
                password_hash: None,
                role: Some(UserRole::Admin),
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.role, UserRole::Admin);
    assert_eq!(updated.username, created.username);
}

#[tokio::test]
async fn test_db_auth_user_update_user_nonexistent_returns_not_found() {
    // Satisfies: AUTH-011 — updating missing user returns NotFound
    let db = fresh_db().await;

    match db
        .update_user(
            999_999,
            UpdateUserDbRequest {
                username: Some("x".to_string()),
                password_hash: None,
                role: None,
            },
        )
        .await
    {
        Err(DbError::NotFound) => {}
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_delete_user_removes_user_and_subsequent_get_is_not_found() {
    // Satisfies: AUTH-011 — delete removes user record
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    db.delete_user(created.id).await.unwrap();

    match db.get_user(created.id).await {
        Err(DbError::NotFound) => {}
        other => panic!("expected NotFound after delete, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_delete_user_nonexistent_returns_not_found() {
    // Satisfies: AUTH-011 — deleting missing user returns NotFound
    let db = fresh_db().await;

    match db.delete_user(999_999).await {
        Err(DbError::NotFound) => {}
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_count_admins_returns_correct_count() {
    // Satisfies: AUTH-011 — admin counting supports last-admin protection at service layer
    let db = fresh_db().await;
    db.create_user(create_user_req("admin1", UserRole::Admin))
        .await
        .unwrap();
    db.create_user(create_user_req("admin2", UserRole::Admin))
        .await
        .unwrap();
    db.create_user(create_user_req("user1", UserRole::User))
        .await
        .unwrap();

    let count = db.count_admins().await.unwrap();

    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_db_auth_user_count_admins_returns_zero_when_no_admins_exist() {
    // Satisfies: AUTH-011 — admin count can be zero
    let db = fresh_db().await;
    db.create_user(create_user_req("user1", UserRole::User))
        .await
        .unwrap();

    let count = db.count_admins().await.unwrap();

    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_db_auth_user_complete_setup_updates_placeholder_admin() {
    // Satisfies: AUTH-010 — complete_setup atomically finalizes placeholder admin
    // The fresh DB is bootstrapped with a placeholder admin (setup_pending=true).
    // complete_setup finds that row and replaces credentials + clears setup_pending.
    let db = fresh_db_with_placeholder().await;

    // Verify the placeholder exists with setup_pending=true
    let users = db.list_users().await.unwrap();
    assert_eq!(users.len(), 1);
    assert!(users[0].setup_pending);

    let completed = db
        .complete_setup(CompleteSetupDbRequest {
            username: "realadmin".to_string(),
            password_hash: "pw-real".to_string(),
            api_key_hash: "api-real".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(completed.username, "realadmin");
    assert_eq!(completed.password_hash, "pw-real");
    assert_eq!(completed.api_key_hash, "api-real");
    assert_eq!(completed.role, UserRole::Admin);
    assert!(!completed.setup_pending);
}

#[tokio::test]
async fn test_db_auth_user_complete_setup_when_not_pending_returns_constraint() {
    // Satisfies: AUTH-010 — complete_setup only succeeds for setup_pending placeholder admin.
    // After setup is completed, a second call must fail with Constraint.
    let db = fresh_db_with_placeholder().await;
    db.complete_setup(CompleteSetupDbRequest {
        username: "admin".to_string(),
        password_hash: "pw-admin".to_string(),
        api_key_hash: "api-admin".to_string(),
    })
    .await
    .unwrap();

    match db
        .complete_setup(CompleteSetupDbRequest {
            username: "newadmin".to_string(),
            password_hash: "pw-new".to_string(),
            api_key_hash: "api-new".to_string(),
        })
        .await
    {
        Err(DbError::Constraint { .. }) => {}
        other => panic!("expected Constraint, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_user_update_api_key_hash_changes_lookup_hash() {
    // Satisfies: AUTH-007 — API key hash update changes stored lookup key
    let db = fresh_db().await;
    let created = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    db.update_api_key_hash(created.id, "api-alice-new")
        .await
        .unwrap();

    match db.get_user_by_api_key_hash("api-alice").await {
        Err(DbError::NotFound) => {}
        other => panic!("expected old hash to be missing, got {:?}", other),
    }

    let fetched = db.get_user_by_api_key_hash("api-alice-new").await.unwrap();
    assert_eq!(fetched.id, created.id);
}

#[tokio::test]
async fn test_db_auth_user_create_user_duplicate_username_returns_constraint() {
    // Satisfies: AUTH-013 — duplicate username violates DB constraint
    let db = fresh_db().await;
    db.create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    match db
        .create_user(create_user_req("alice", UserRole::Admin))
        .await
    {
        Err(DbError::Constraint { .. }) => {}
        other => panic!("expected Constraint, got {:?}", other),
    }
}

// === SessionDb ===

#[tokio::test]
async fn test_db_auth_session_create_and_get_round_trip() {
    // Satisfies: AUTH-005, AUTH-006 — session record stores hashed token and expiry metadata
    let db = fresh_db().await;
    let user = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();
    let session = Session {
        token_hash: "token-hash-1".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(24),
    };

    db.create_session(&session).await.unwrap();
    let fetched = db.get_session(&session.token_hash).await.unwrap().unwrap();

    assert_eq!(fetched.token_hash, session.token_hash);
    assert_eq!(fetched.user_id, session.user_id);
    assert_eq!(fetched.persistent, session.persistent);
    assert_eq!(fetched.expires_at, session.expires_at);
}

#[tokio::test]
async fn test_db_auth_session_get_nonexistent_returns_none() {
    // Satisfies: AUTH-006 — missing session token hash returns None
    let db = fresh_db().await;

    let session = db.get_session("missing-token").await.unwrap();

    assert!(session.is_none());
}

#[tokio::test]
async fn test_db_auth_session_delete_removes_session() {
    // Satisfies: AUTH-005 — session deletion invalidates stored session
    let db = fresh_db().await;
    let user = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();
    let session = Session {
        token_hash: "token-hash-2".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::days(30),
    };

    db.create_session(&session).await.unwrap();
    db.delete_session(&session.token_hash).await.unwrap();

    let fetched = db.get_session(&session.token_hash).await.unwrap();
    assert!(fetched.is_none());
}

#[tokio::test]
async fn test_db_auth_session_delete_nonexistent_is_idempotent() {
    // Satisfies: AUTH-014 — cleanup/delete path tolerates already-missing sessions
    let db = fresh_db().await;

    db.delete_session("missing-token").await.unwrap();
}

#[tokio::test]
async fn test_db_auth_session_extend_updates_expires_at() {
    // Satisfies: AUTH-005 — rolling persistent session expiry can be extended
    let db = fresh_db().await;
    let user = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();
    let original_expiry = Utc::now() + Duration::hours(24);
    let new_expiry = Utc::now() + Duration::days(30);
    let session = Session {
        token_hash: "token-hash-3".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: Utc::now(),
        expires_at: original_expiry,
    };

    db.create_session(&session).await.unwrap();
    db.extend_session(&session.token_hash, new_expiry)
        .await
        .unwrap();

    let fetched = db.get_session(&session.token_hash).await.unwrap().unwrap();
    assert_eq!(fetched.expires_at, new_expiry);
    assert_eq!(fetched.user_id, user.id);
}

#[tokio::test]
async fn test_db_auth_session_extend_nonexistent_returns_not_found_or_constraint() {
    // Satisfies: AUTH-005 — extending a missing session must fail
    let db = fresh_db().await;

    match db
        .extend_session("missing-token", Utc::now() + Duration::hours(1))
        .await
    {
        Err(DbError::NotFound) | Err(DbError::Constraint { .. }) => {}
        other => panic!("expected NotFound or Constraint, got {:?}", other),
    }
}

#[tokio::test]
async fn test_db_auth_session_delete_expired_sessions_removes_only_expired_and_returns_count() {
    // Satisfies: AUTH-014 — cleanup deletes expired sessions only
    let db = fresh_db().await;
    let user = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();

    let expired = Session {
        token_hash: "expired-token".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: Utc::now() - Duration::days(2),
        expires_at: Utc::now() - Duration::hours(1),
    };
    let active = Session {
        token_hash: "active-token".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::days(1),
    };

    db.create_session(&expired).await.unwrap();
    db.create_session(&active).await.unwrap();

    let deleted = db.delete_expired_sessions().await.unwrap();

    assert_eq!(deleted, 1);
    assert!(db.get_session("expired-token").await.unwrap().is_none());
    assert!(db.get_session("active-token").await.unwrap().is_some());
}

#[tokio::test]
async fn test_db_auth_session_delete_expired_sessions_with_none_expired_returns_zero() {
    // Satisfies: AUTH-014 — cleanup reports zero when nothing expired
    let db = fresh_db().await;
    let user = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();
    let active = Session {
        token_hash: "active-only-token".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(2),
    };

    db.create_session(&active).await.unwrap();

    let deleted = db.delete_expired_sessions().await.unwrap();

    assert_eq!(deleted, 0);
    assert!(db.get_session(&active.token_hash).await.unwrap().is_some());
}

#[tokio::test]
async fn test_db_auth_session_sessions_are_cleaned_up_when_user_deleted_via_cascade() {
    // Satisfies: AUTH-011, AUTH-014 — deleting user cascades to sessions
    let db = fresh_db().await;
    let user = db
        .create_user(create_user_req("alice", UserRole::User))
        .await
        .unwrap();
    let session = Session {
        token_hash: "cascade-token".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::days(30),
    };

    db.create_session(&session).await.unwrap();
    db.delete_user(user.id).await.unwrap();

    let fetched = db.get_session(&session.token_hash).await.unwrap();
    assert!(fetched.is_none());
}
