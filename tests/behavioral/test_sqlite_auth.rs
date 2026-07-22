use chrono::{Duration, Utc};
use livrarr_db::pool::{create_sqlite_pool, run_migrations};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    CompleteSetupDbRequest, CreateUserDbRequest, DbError, Session, SessionDb, UpdateUserDbRequest,
    UserDb, UserRole,
};
use tempfile::TempDir;

async fn setup_test_db() -> (SqliteDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let pool = create_sqlite_pool(dir.path()).await.unwrap();
    run_migrations(&pool).await.unwrap();
    (SqliteDb::new(pool), dir)
}

fn assert_is_constraint(err: DbError) {
    match err {
        DbError::Constraint { .. } => {}
        other => panic!("expected DbError::Constraint, got: {:?}", other),
    }
}

fn assert_is_not_found(err: DbError) {
    match err {
        DbError::NotFound { .. } => {}
        other => panic!("expected DbError::NotFound, got: {:?}", other),
    }
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn migration_creates_placeholder_admin_pending_setup() {
    let (db, _dir) = setup_test_db().await;

    let user = db.get_user(1).await.unwrap();
    assert_eq!(user.id, 1);
    assert!(user.setup_pending);
    assert_eq!(user.role, UserRole::Admin);

    let admins = db.count_admins().await.unwrap();
    assert_eq!(admins, 1);

    let users = db.list_users().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, 1);
    assert!(users[0].setup_pending);
    assert_eq!(users[0].role, UserRole::Admin);
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn complete_setup_converts_placeholder_admin_and_clears_setup_pending() {
    let (db, _dir) = setup_test_db().await;

    let completed = db
        .complete_setup(CompleteSetupDbRequest {
            username: "admin".to_string(),
            password_hash: "pw_hash".to_string(),
            api_key_hash: "api_hash_admin".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(completed.id, 1);
    assert_eq!(completed.username, "admin");
    assert_eq!(completed.password_hash, "pw_hash");
    assert_eq!(completed.api_key_hash, "api_hash_admin");
    assert_eq!(completed.role, UserRole::Admin);
    assert!(!completed.setup_pending);

    let fetched = db.get_user(1).await.unwrap();
    assert_eq!(fetched.username, "admin");
    assert_eq!(fetched.password_hash, "pw_hash");
    assert_eq!(fetched.api_key_hash, "api_hash_admin");
    assert!(!fetched.setup_pending);
}

/// REQ-ID: AUTH-010
#[tokio::test]
async fn complete_setup_is_atomic_and_only_allowed_once() {
    let (db, _dir) = setup_test_db().await;

    let first = db
        .complete_setup(CompleteSetupDbRequest {
            username: "admin".to_string(),
            password_hash: "pw_hash".to_string(),
            api_key_hash: "api_hash_admin".to_string(),
        })
        .await
        .unwrap();
    assert!(!first.setup_pending);

    let err = db
        .complete_setup(CompleteSetupDbRequest {
            username: "admin2".to_string(),
            password_hash: "pw_hash_2".to_string(),
            api_key_hash: "api_hash_admin_2".to_string(),
        })
        .await
        .unwrap_err();
    assert_is_constraint(err);

    let fetched = db.get_user(1).await.unwrap();
    assert_eq!(fetched.username, "admin");
    assert_eq!(fetched.password_hash, "pw_hash");
    assert_eq!(fetched.api_key_hash, "api_hash_admin");
    assert!(!fetched.setup_pending);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn create_and_get_user_by_id_username_and_api_key_hash() {
    let (db, _dir) = setup_test_db().await;

    let created = db
        .create_user(CreateUserDbRequest {
            username: "alice".to_string(),
            password_hash: "alice_pw".to_string(),
            role: UserRole::User,
            api_key_hash: "alice_api".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(created.username, "alice");
    assert_eq!(created.password_hash, "alice_pw");
    assert_eq!(created.role, UserRole::User);
    assert_eq!(created.api_key_hash, "alice_api");
    assert!(!created.setup_pending);

    let by_id = db.get_user(created.id).await.unwrap();
    assert_eq!(by_id.id, created.id);
    assert_eq!(by_id.username, "alice");

    let by_username = db.get_user_by_username("alice").await.unwrap();
    assert_eq!(by_username.id, created.id);

    let by_api = db.get_user_by_api_key_hash("alice_api").await.unwrap();
    assert_eq!(by_api.id, created.id);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn username_lookup_is_case_insensitive() {
    let (db, _dir) = setup_test_db().await;

    let created = db
        .create_user(CreateUserDbRequest {
            username: "AliceCase".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "case_api".to_string(),
        })
        .await
        .unwrap();

    let lower = db.get_user_by_username("alicecase").await.unwrap();
    let upper = db.get_user_by_username("ALICECASE").await.unwrap();
    let mixed = db.get_user_by_username("AlIcEcAsE").await.unwrap();

    assert_eq!(lower.id, created.id);
    assert_eq!(upper.id, created.id);
    assert_eq!(mixed.id, created.id);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn create_user_rejects_duplicate_username_case_insensitively() {
    let (db, _dir) = setup_test_db().await;

    db.create_user(CreateUserDbRequest {
        username: "Alice".to_string(),
        password_hash: "pw1".to_string(),
        role: UserRole::User,
        api_key_hash: "api1".to_string(),
    })
    .await
    .unwrap();

    let err = db
        .create_user(CreateUserDbRequest {
            username: "alice".to_string(),
            password_hash: "pw2".to_string(),
            role: UserRole::User,
            api_key_hash: "api2".to_string(),
        })
        .await
        .unwrap_err();

    assert_is_constraint(err);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn create_user_rejects_duplicate_api_key_hash() {
    let (db, _dir) = setup_test_db().await;

    db.create_user(CreateUserDbRequest {
        username: "user1".to_string(),
        password_hash: "pw1".to_string(),
        role: UserRole::User,
        api_key_hash: "shared_api".to_string(),
    })
    .await
    .unwrap();

    let err = db
        .create_user(CreateUserDbRequest {
            username: "user2".to_string(),
            password_hash: "pw2".to_string(),
            role: UserRole::User,
            api_key_hash: "shared_api".to_string(),
        })
        .await
        .unwrap_err();

    assert_is_constraint(err);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn get_missing_user_and_lookups_return_not_found() {
    let (db, _dir) = setup_test_db().await;

    assert_is_not_found(db.get_user(9999).await.unwrap_err());
    assert_is_not_found(db.get_user_by_username("missing").await.unwrap_err());
    assert_is_not_found(
        db.get_user_by_api_key_hash("missing_api_hash")
            .await
            .unwrap_err(),
    );
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn list_users_returns_existing_users() {
    let (db, _dir) = setup_test_db().await;

    let before = db.list_users().await.unwrap();
    assert_eq!(before.len(), 1);

    let u1 = db
        .create_user(CreateUserDbRequest {
            username: "alice".to_string(),
            password_hash: "pw1".to_string(),
            role: UserRole::User,
            api_key_hash: "api1".to_string(),
        })
        .await
        .unwrap();

    let u2 = db
        .create_user(CreateUserDbRequest {
            username: "bob".to_string(),
            password_hash: "pw2".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api2".to_string(),
        })
        .await
        .unwrap();

    let users = db.list_users().await.unwrap();
    assert_eq!(users.len(), 3);
    assert!(users.iter().any(|u| u.id == 1));
    assert!(users.iter().any(|u| u.id == u1.id && u.username == "alice"));
    assert!(users.iter().any(|u| u.id == u2.id && u.username == "bob"));
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn update_user_changes_selected_fields_only() {
    let (db, _dir) = setup_test_db().await;

    let created = db
        .create_user(CreateUserDbRequest {
            username: "charlie".to_string(),
            password_hash: "pw_old".to_string(),
            role: UserRole::User,
            api_key_hash: "charlie_api".to_string(),
        })
        .await
        .unwrap();

    let updated = db
        .update_user(
            created.id,
            UpdateUserDbRequest {
                username: Some("charlie2".to_string()),
                password_hash: Some("pw_new".to_string()),
                role: Some(UserRole::Admin),
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.username, "charlie2");
    assert_eq!(updated.password_hash, "pw_new");
    assert_eq!(updated.role, UserRole::Admin);
    assert_eq!(updated.api_key_hash, "charlie_api");
    assert!(!updated.setup_pending);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn update_user_with_none_fields_leaves_values_unchanged() {
    let (db, _dir) = setup_test_db().await;

    let created = db
        .create_user(CreateUserDbRequest {
            username: "delta".to_string(),
            password_hash: "delta_pw".to_string(),
            role: UserRole::User,
            api_key_hash: "delta_api".to_string(),
        })
        .await
        .unwrap();

    let updated = db
        .update_user(
            created.id,
            UpdateUserDbRequest {
                username: None,
                password_hash: None,
                role: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.username, created.username);
    assert_eq!(updated.password_hash, created.password_hash);
    assert_eq!(updated.role, created.role);
    assert_eq!(updated.api_key_hash, created.api_key_hash);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn update_missing_user_returns_not_found() {
    let (db, _dir) = setup_test_db().await;

    let err = db
        .update_user(
            9999,
            UpdateUserDbRequest {
                username: Some("nobody".to_string()),
                password_hash: None,
                role: None,
            },
        )
        .await
        .unwrap_err();

    assert_is_not_found(err);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn update_api_key_hash_changes_lookup_target() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "gina".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "old_api".to_string(),
        })
        .await
        .unwrap();

    let before = db.get_user_by_api_key_hash("old_api").await.unwrap();
    assert_eq!(before.id, user.id);

    db.update_api_key_hash(user.id, "new_api").await.unwrap();

    assert_is_not_found(db.get_user_by_api_key_hash("old_api").await.unwrap_err());

    let after = db.get_user_by_api_key_hash("new_api").await.unwrap();
    assert_eq!(after.id, user.id);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn update_api_key_hash_missing_user_returns_not_found() {
    let (db, _dir) = setup_test_db().await;

    let err = db.update_api_key_hash(9999, "new_api").await.unwrap_err();
    assert_is_not_found(err);
}

/// REQ-ID: AUTH-011
#[tokio::test]
async fn delete_user_removes_user() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "jane".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "jane_api".to_string(),
        })
        .await
        .unwrap();

    db.delete_user(user.id).await.unwrap();

    assert_is_not_found(db.get_user(user.id).await.unwrap_err());
    assert_is_not_found(db.get_user_by_username("jane").await.unwrap_err());
    assert_is_not_found(db.get_user_by_api_key_hash("jane_api").await.unwrap_err());
}

/// REQ-ID: AUTH-011
#[tokio::test]
async fn delete_user_cascades_sessions() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "kate".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "kate_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session1 = Session {
        token_hash: "token1".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::hours(1),
    };
    let session2 = Session {
        token_hash: "token2".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: now,
        expires_at: now + Duration::days(30),
    };

    db.create_session(&session1).await.unwrap();
    db.create_session(&session2).await.unwrap();

    assert!(db.get_session("token1").await.unwrap().is_some());
    assert!(db.get_session("token2").await.unwrap().is_some());

    db.delete_user(user.id).await.unwrap();

    assert!(db.get_session("token1").await.unwrap().is_none());
    assert!(db.get_session("token2").await.unwrap().is_none());
}

/// REQ-ID: AUTH-011
#[tokio::test]
async fn delete_missing_user_returns_not_found() {
    let (db, _dir) = setup_test_db().await;

    let err = db.delete_user(9999).await.unwrap_err();
    assert_is_not_found(err);
}

/// REQ-ID: AUTH-007
#[tokio::test]
async fn count_admins_counts_current_admin_users() {
    let (db, _dir) = setup_test_db().await;

    assert_eq!(db.count_admins().await.unwrap(), 1);

    let user_admin = db
        .create_user(CreateUserDbRequest {
            username: "second_admin".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::Admin,
            api_key_hash: "second_admin_api".to_string(),
        })
        .await
        .unwrap();

    let regular = db
        .create_user(CreateUserDbRequest {
            username: "regular_user".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "regular_api".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(db.count_admins().await.unwrap(), 2);

    db.update_user(
        regular.id,
        UpdateUserDbRequest {
            username: None,
            password_hash: None,
            role: Some(UserRole::Admin),
        },
    )
    .await
    .unwrap();

    assert_eq!(db.count_admins().await.unwrap(), 3);

    db.delete_user(user_admin.id).await.unwrap();

    assert_eq!(db.count_admins().await.unwrap(), 2);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn create_and_get_non_persistent_session() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "leo".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "leo_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session = Session {
        token_hash: "session_non_persistent".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::hours(12),
    };

    db.create_session(&session).await.unwrap();

    let fetched = db
        .get_session("session_non_persistent")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.token_hash, session.token_hash);
    assert_eq!(fetched.user_id, user.id);
    assert!(!fetched.persistent);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn create_and_get_persistent_session() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "maya".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "maya_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session = Session {
        token_hash: "session_persistent".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: now,
        expires_at: now + Duration::days(30),
    };

    db.create_session(&session).await.unwrap();

    let fetched = db.get_session("session_persistent").await.unwrap().unwrap();
    assert_eq!(fetched.user_id, user.id);
    assert!(fetched.persistent);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn create_session_for_missing_user_fails() {
    let (db, _dir) = setup_test_db().await;

    let now = Utc::now();
    let session = Session {
        token_hash: "orphan_session".to_string(),
        user_id: 9999,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::hours(1),
    };

    let err = db.create_session(&session).await.unwrap_err();
    assert_is_constraint(err);
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn create_session_rejects_duplicate_token_hash() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "nick".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "nick_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session = Session {
        token_hash: "dup_token".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::hours(1),
    };

    db.create_session(&session).await.unwrap();

    let err = db.create_session(&session).await.unwrap_err();
    assert_is_constraint(err);
}

/// REQ-ID: AUTH-006
#[tokio::test]
async fn get_session_returns_none_when_missing() {
    let (db, _dir) = setup_test_db().await;

    let session = db.get_session("missing_token").await.unwrap();
    assert!(session.is_none());
}

/// REQ-ID: AUTH-006
#[tokio::test]
async fn get_session_returns_none_for_expired_session() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "olivia".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "olivia_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session = Session {
        token_hash: "expired_token".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now - Duration::hours(2),
        expires_at: now - Duration::minutes(1),
    };

    db.create_session(&session).await.unwrap();

    let fetched = db.get_session("expired_token").await.unwrap();
    assert!(fetched.is_none());
}

/// REQ-ID: AUTH-005
#[tokio::test]
async fn extend_session_updates_expiration() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "peter".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "peter_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session = Session {
        token_hash: "extend_me".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::minutes(10),
    };

    db.create_session(&session).await.unwrap();

    let new_expiry = now + Duration::hours(5);
    db.extend_session("extend_me", new_expiry).await.unwrap();

    let fetched = db.get_session("extend_me").await.unwrap().unwrap();
    assert_eq!(fetched.user_id, user.id);
}

/// REQ-ID: AUTH-006
#[tokio::test]
async fn delete_session_removes_existing_session() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "quinn".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "quinn_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let session = Session {
        token_hash: "delete_me".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::hours(1),
    };

    db.create_session(&session).await.unwrap();
    assert!(db.get_session("delete_me").await.unwrap().is_some());

    db.delete_session("delete_me").await.unwrap();

    assert!(db.get_session("delete_me").await.unwrap().is_none());
}

/// REQ-ID: AUTH-006
#[tokio::test]
async fn delete_missing_session_is_ok() {
    let (db, _dir) = setup_test_db().await;

    db.delete_session("missing_token").await.unwrap();
    assert!(db.get_session("missing_token").await.unwrap().is_none());
}

/// REQ-ID: AUTH-014
#[tokio::test]
async fn delete_expired_sessions_removes_only_expired_and_returns_count() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "riley".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "riley_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();

    let expired1 = Session {
        token_hash: "expired1".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now - Duration::hours(3),
        expires_at: now - Duration::hours(2),
    };
    let expired2 = Session {
        token_hash: "expired2".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: now - Duration::days(40),
        expires_at: now - Duration::days(1),
    };
    let active = Session {
        token_hash: "active1".to_string(),
        user_id: user.id,
        persistent: true,
        created_at: now,
        expires_at: now + Duration::days(10),
    };

    db.create_session(&expired1).await.unwrap();
    db.create_session(&expired2).await.unwrap();
    db.create_session(&active).await.unwrap();

    let deleted = db.delete_expired_sessions().await.unwrap();
    assert_eq!(deleted, 2);

    assert!(db.get_session("expired1").await.unwrap().is_none());
    assert!(db.get_session("expired2").await.unwrap().is_none());
    assert!(db.get_session("active1").await.unwrap().is_some());
}

/// REQ-ID: AUTH-014
#[tokio::test]
async fn delete_expired_sessions_returns_zero_when_nothing_expired() {
    let (db, _dir) = setup_test_db().await;

    let user = db
        .create_user(CreateUserDbRequest {
            username: "sara".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::User,
            api_key_hash: "sara_api".to_string(),
        })
        .await
        .unwrap();

    let now = Utc::now();
    let active = Session {
        token_hash: "active_only".to_string(),
        user_id: user.id,
        persistent: false,
        created_at: now,
        expires_at: now + Duration::hours(2),
    };

    db.create_session(&active).await.unwrap();

    let deleted = db.delete_expired_sessions().await.unwrap();
    assert_eq!(deleted, 0);

    assert!(db.get_session("active_only").await.unwrap().is_some());
}

// =============================================================================
// Sole-admin invariant (Unit B2) — demotion-only, atomic, disambiguated
// =============================================================================

/// Concurrent demotion of two admins must leave exactly one admin standing —
/// the loser sees the guard block it, not a generic failure.
#[tokio::test]
async fn concurrent_demote_leaves_exactly_one_admin() {
    let (db, _dir) = setup_test_db().await;

    let second_admin = db
        .create_user(CreateUserDbRequest {
            username: "second_admin_demote".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::Admin,
            api_key_hash: "second_admin_demote_api".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(db.count_admins().await.unwrap(), 2);

    let demote_first = db.update_user(
        1,
        UpdateUserDbRequest {
            username: None,
            password_hash: None,
            role: Some(UserRole::User),
        },
    );
    let demote_second = db.update_user(
        second_admin.id,
        UpdateUserDbRequest {
            username: None,
            password_hash: None,
            role: Some(UserRole::User),
        },
    );
    let (r1, r2) = tokio::join!(demote_first, demote_second);

    let (winner, loser) = if r1.is_ok() { (r1, r2) } else { (r2, r1) };
    assert!(
        winner.is_ok(),
        "exactly one demotion must succeed: {winner:?}"
    );
    assert!(
        matches!(loser, Err(DbError::LastAdmin)),
        "the losing demotion must fail with LastAdmin, got: {loser:?}"
    );

    assert_eq!(db.count_admins().await.unwrap(), 1);
}

/// Concurrent deletion of two admins must leave exactly one admin standing.
#[tokio::test]
async fn concurrent_delete_leaves_exactly_one_admin() {
    let (db, _dir) = setup_test_db().await;

    let second_admin = db
        .create_user(CreateUserDbRequest {
            username: "second_admin_delete".to_string(),
            password_hash: "pw".to_string(),
            role: UserRole::Admin,
            api_key_hash: "second_admin_delete_api".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(db.count_admins().await.unwrap(), 2);

    let delete_first = db.delete_user(1);
    let delete_second = db.delete_user(second_admin.id);
    let (r1, r2) = tokio::join!(delete_first, delete_second);

    let (winner, loser) = if r1.is_ok() { (r1, r2) } else { (r2, r1) };
    assert!(
        winner.is_ok(),
        "exactly one deletion must succeed: {winner:?}"
    );
    assert!(
        matches!(loser, Err(DbError::LastAdmin)),
        "the losing deletion must fail with LastAdmin, got: {loser:?}"
    );

    assert_eq!(db.count_admins().await.unwrap(), 1);
}

/// A plain username/password edit on the sole admin must succeed — the guard
/// only fires on a real admin -> user transition, never on an unrelated
/// field edit.
#[tokio::test]
async fn update_user_username_and_password_succeeds_for_sole_admin() {
    let (db, _dir) = setup_test_db().await;
    assert_eq!(db.count_admins().await.unwrap(), 1);

    let updated = db
        .update_user(
            1,
            UpdateUserDbRequest {
                username: Some("renamed_admin".to_string()),
                password_hash: Some("new_pw_hash".to_string()),
                role: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.username, "renamed_admin");
    assert_eq!(updated.password_hash, "new_pw_hash");
    assert_eq!(updated.role, UserRole::Admin);
    assert_eq!(db.count_admins().await.unwrap(), 1);
}

/// A missing user must report NotFound, even when the request looks like a
/// demotion — 0 rows affected must not be misread as LastAdmin when the row
/// simply doesn't exist.
#[tokio::test]
async fn update_missing_user_with_demotion_request_returns_not_found_not_last_admin() {
    let (db, _dir) = setup_test_db().await;

    let err = db
        .update_user(
            9999,
            UpdateUserDbRequest {
                username: None,
                password_hash: None,
                role: Some(UserRole::User),
            },
        )
        .await
        .unwrap_err();

    assert_is_not_found(err);
}

/// A rejected demotion must be all-or-nothing — a mixed request (new
/// password + role: user) on the sole admin must fail with LastAdmin AND
/// leave the password hash unchanged.
#[tokio::test]
async fn update_user_rejects_demotion_and_leaves_password_unchanged_when_mixed() {
    let (db, _dir) = setup_test_db().await;

    let before = db.get_user(1).await.unwrap();

    let err = db
        .update_user(
            1,
            UpdateUserDbRequest {
                username: None,
                password_hash: Some("attempted_new_hash".to_string()),
                role: Some(UserRole::User),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, DbError::LastAdmin));

    let after = db.get_user(1).await.unwrap();
    assert_eq!(after.password_hash, before.password_hash);
    assert_eq!(after.role, UserRole::Admin);
}
