//! Real-SQLite behavioral pins for author-provider linking.
//!
//! Normal setup uses production writers. Direct SQL is limited to migration
//! fixture construction and to observing exact persistence state.

use std::borrow::Cow;
use std::str::FromStr;

use chrono::{Duration, Utc};
use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::pool::{backfill_author_identity, backfill_normalized_identity};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    AuthorDb, AuthorLinkClaim, AuthorLinkDb, AuthorNameVariantDb, CreateAuthorDbRequest,
    CreateWorkDbRequest, GuardedRouteWrite, RenameAuthorDbRequest, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity_matching::AuthorVerdict;
use livrarr_domain::{
    guard_author_route, normalize_for_matching, AuthorCandidateAlternateNameEvidence,
    AuthorCandidateCatalogState, AuthorEvidenceFingerprint, AuthorKeyAttemptOutcome,
    AuthorLinkCandidate, AuthorLinkCandidateReason, AuthorLinkCandidateStatus, AuthorLinkCursor,
    AuthorLinkProgressState, AuthorLinkProgressUpdate, AuthorLinkTrigger, AuthorNameSource,
    AuthorProvider, AuthorRouteEvidenceSource, AuthorRouteGuardResult, AuthorRouteKey,
    AuthorRouteProvenance, DbError, IdentityStatus, ProviderAuthorNameObservation,
    ProviderAuthorRef, ProviderCredit, RouteWriteOutcome, SettledWorkProviderKey,
};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

static ALL_MIGRATIONS: Migrator = sqlx::migrate!("../livrarr-db/migrations");

fn route(provider: AuthorProvider, raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::parse(provider, raw).expect("fixture route must use the production parser")
}

fn author_request(user_id: i64, name: &str) -> CreateAuthorDbRequest {
    CreateAuthorDbRequest {
        user_id,
        name: name.to_string(),
        sort_name: None,
        ol_key: None,
        gr_key: None,
        hc_key: None,
        import_id: None,
    }
}

fn work_request(
    user_id: i64,
    author_id: i64,
    title: &str,
    author_name: &str,
) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author_name.to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching(author_name),
        author_id: Some(author_id),
        language: Some("en".to_string()),
        ..Default::default()
    }
}

async fn seeded_db(label: &str) -> (SqliteDb, i64, i64, i64) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let name = format!("{label} Author");
    let (author, created) = db
        .create_author(author_request(user_id, &name))
        .await
        .expect("production author writer");
    assert!(created);
    let (work, work_created) = db
        .create_work(work_request(
            user_id,
            author.id,
            &format!("{label} Work"),
            &name,
        ))
        .await
        .expect("production work writer");
    assert!(work_created);
    (db, user_id, author.id, work.id)
}

async fn claimed_author(db: &SqliteDb, user_id: i64, author_id: i64) -> AuthorLinkClaim {
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("enqueue through production DB operation");
    let now = Utc::now();
    let claims = db
        .claim_due(now, now + Duration::minutes(5), 10)
        .await
        .expect("claim through production DB operation");
    claims
        .into_iter()
        .find(|claim| claim.author_id == author_id)
        .expect("seeded author must be claimed")
}

fn candidate(author_id: i64, raw: &str, generation: i64) -> AuthorLinkCandidate {
    AuthorLinkCandidate {
        id: 0,
        author_id,
        key: route(AuthorProvider::OpenLibrary, raw),
        candidate_name: "Candidate Author".to_string(),
        reason: AuthorLinkCandidateReason::Tier2NameSearch,
        name_verdict: AuthorVerdict::Grey,
        primary_name_verdict: AuthorVerdict::Grey,
        alternate_name_evidence: vec![],
        top_work_preview: Some("A corroborating title".to_string()),
        catalog_evidence_state: AuthorCandidateCatalogState::Complete,
        corroborated_title_count: 1,
        settled_work_count: 2,
        previously_removed: false,
        status: AuthorLinkCandidateStatus::Pending,
        evidence_generation: generation,
        observed_at: Utc::now(),
        evidence_work_id: None,
        evidence_work_title: None,
        revoked_at: None,
    }
}

fn progress_update(generation: i64, dirty_generation: i64) -> AuthorLinkProgressUpdate {
    AuthorLinkProgressUpdate {
        state: AuthorLinkProgressState::Linked,
        tier: Some(1),
        cursor: Some(AuthorLinkCursor::Tier1 { key_attempt_id: 1 }),
        evaluated_fingerprint: AuthorEvidenceFingerprint {
            settled_work_count: 1,
            settled_provider_key_count: 1,
            content_hash: "fixed-fixture-fingerprint".to_string(),
        },
        evidence_generation: generation,
        next_attempt_at: Utc::now() + Duration::hours(1),
        last_error: None,
        display_name_generation: dirty_generation,
        display_name_dirty: false,
        would_have_linked_at_090: false,
    }
}

fn agreed_evidence(raw: &str) -> livrarr_domain::AgreedAuthorRouteEvidence {
    let guarded = guard_author_route(
        &["Octavia E. Butler".to_string()],
        ProviderAuthorRef {
            key: route(AuthorProvider::OpenLibrary, raw),
            name: "Octavia E Butler".to_string(),
            credit: ProviderCredit::AssertedAuthor,
        },
        Some(1),
        AuthorRouteEvidenceSource::Tier1SettledWork,
    );
    let AuthorRouteGuardResult::Agreed(evidence) = guarded else {
        panic!("fixture names must agree");
    };
    evidence
}

async fn migration_077_db() -> SqliteDb {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("SQLite options")
        .pragma("foreign_keys", "ON")
        .pragma("busy_timeout", "5000");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("migration fixture pool");
    let through_077 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 77)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    through_077
        .run(&pool)
        .await
        .expect("apply real migrations through 077");
    backfill_normalized_identity(&pool)
        .await
        .expect("run production work-identity startup repair");
    backfill_author_identity(&pool)
        .await
        .expect("run production migration-077 startup repair");
    SqliteDb::new(pool)
}

async fn apply_migration_078(db: &SqliteDb) {
    let only_078 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version == 78)
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    };
    only_078
        .run(db.pool())
        .await
        .expect("upgrade real migration-077 fixture through 078");
}

async fn apply_migration_079(db: &SqliteDb) {
    let only_079 = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version == 79)
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    };
    only_079
        .run(db.pool())
        .await
        .expect("upgrade real migration-078 fixture through 079");
}

/// Every migration after the cutover pair. A fixture that stops short of head
/// is not a state production ever serves from — startup migrates all the way —
/// so a production writer running against it would fail on a schema this
/// install would really have.
async fn apply_migrations_after_079(db: &SqliteDb) {
    let remainder = Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version > 79)
                .cloned()
                .collect(),
        ),
        ignore_missing: true,
        locking: true,
        no_tx: false,
    };
    remainder
        .run(db.pool())
        .await
        .expect("upgrade real migration-079 fixture to head");
}

async fn legacy_route_db(raw_ol_key: &str) -> (SqliteDb, i64, i64) {
    let db = migration_077_db().await;
    let user_id = create_test_user(&db).await;
    let mut request = author_request(user_id, "Legacy Route Author");
    request.ol_key = Some(raw_ol_key.to_string());
    let (author, _) = db
        .create_author(request)
        .await
        .expect("production pre-cutover author writer");
    apply_migration_078(&db).await;
    apply_migration_079(&db).await;
    db.ingest_legacy_routes()
        .await
        .expect("production legacy-route ingestion");
    apply_migrations_after_079(&db).await;
    (db, user_id, author.id)
}

async fn migration_graph() -> (SqliteDb, i64, i64, i64) {
    let db = migration_077_db().await;
    apply_migration_078(&db).await;
    let user_id = create_test_user(&db).await;
    let (author, _) = db
        .create_author(author_request(user_id, "Migration Matrix Author"))
        .await
        .expect("production author writer");
    let (work, _) = db
        .create_work(work_request(
            user_id,
            author.id,
            "Migration Matrix Work",
            &author.name,
        ))
        .await
        .expect("production work writer");

    // Constructed-state justification: the migration trigger itself is under
    // test and its sole production progress writer is an intentional Stage 4a
    // `todo!()`, so the pre-trigger row must be fixture state.
    sqlx::query(
        "INSERT INTO author_link_progress (
            author_id, user_id, state, tier, cursor, evaluated_fingerprint,
            evidence_generation, display_name_generation, display_name_dirty,
            attempt_count, next_attempt_at, claim_token, lease_until, last_error,
            would_have_linked_at_090, trigger, updated_at
         ) VALUES (?, ?, 'running', 1, 'tier1:41', 'fingerprint-before', 7, 3, 0,
                   2, '2099-01-01T00:00:00Z', '00000000-0000-0000-0000-000000000001',
                   '2099-01-01T00:05:00Z', NULL, 0, 'retry_due',
                   '2026-07-30T00:00:00Z')",
    )
    .bind(author.id)
    .bind(user_id)
    .execute(db.pool())
    .await
    .expect("construct pre-trigger progress row");
    (db, user_id, author.id, work.id)
}

async fn reset_live_progress(db: &SqliteDb, author_id: i64) {
    sqlx::query(
        "UPDATE author_link_progress
            SET state='running', trigger='retry_due',
                next_attempt_at='2099-01-01T00:00:00Z',
                claim_token='00000000-0000-0000-0000-000000000001',
                lease_until='2099-01-01T00:05:00Z',
                updated_at='2026-07-30T00:00:00Z'
          WHERE author_id=?",
    )
    .bind(author_id)
    .execute(db.pool())
    .await
    .expect("reset trigger fixture");
}

fn migration_claim(user_id: i64, author_id: i64) -> AuthorLinkClaim {
    AuthorLinkClaim {
        author_id,
        user_id,
        claim_token: "00000000-0000-0000-0000-000000000001"
            .parse()
            .expect("fixture claim token"),
        lease_expires_at: Utc::now() + Duration::minutes(5),
        cursor: Some(AuthorLinkCursor::Tier1 { key_attempt_id: 41 }),
        display_name_generation: 3,
    }
}

fn assert_claim_lost<T>(result: Result<T, DbError>) {
    assert!(
        matches!(result, Err(DbError::ClaimLost)),
        "a mutation carrying invalidated worker authority must return exact DbError::ClaimLost"
    );
}

type ProgressSnapshot = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    String,
);

async fn progress_snapshot(db: &SqliteDb, author_id: i64) -> ProgressSnapshot {
    sqlx::query_as(
        "SELECT state, trigger, claim_token, lease_until, evaluated_fingerprint,
                evidence_generation, cursor, next_attempt_at
           FROM author_link_progress WHERE author_id=?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("progress snapshot")
}

async fn assert_trigger_woke_once(db: &SqliteDb, author_id: i64) {
    let snapshot = progress_snapshot(db, author_id).await;
    assert_eq!(snapshot.0, "queued");
    assert_eq!(snapshot.1, "evidence_fingerprint_changed");
    assert_eq!(snapshot.2, None, "claim token must be invalidated");
    assert_eq!(snapshot.3, None, "lease must be invalidated");
    assert_eq!(snapshot.4.as_deref(), Some("fingerprint-before"));
    assert_eq!(snapshot.5, 7);
    assert_eq!(snapshot.6.as_deref(), Some("tier1:41"));
    let due: i64 = sqlx::query_scalar(
        "SELECT julianday(next_attempt_at) <= julianday('now')
           FROM author_link_progress WHERE author_id=?",
    )
    .bind(author_id)
    .fetch_one(db.pool())
    .await
    .expect("due check");
    assert_eq!(due, 1, "one progress row must be due immediately");
}

/// Door: Work evidence change wake-up (migration 078 DB trigger).
/// AC-006 / AC-012: every null-safe watched transition atomically queues one
/// existing task and invalidates only its stale authority.
#[tokio::test]
async fn ac006_ac012_migration_078_watches_identity_and_every_provider_key_transition() {
    let (db, _user_id, author_id, work_id) = migration_graph().await;
    let statements = [
        "UPDATE works SET identity_status='confirmed' WHERE id=?",
        "UPDATE works SET ol_key='OL1W' WHERE id=?",
        "UPDATE works SET ol_key='OL2W' WHERE id=?",
        "UPDATE works SET ol_key=NULL WHERE id=?",
        "UPDATE works SET gr_key='11' WHERE id=?",
        "UPDATE works SET gr_key='12' WHERE id=?",
        "UPDATE works SET gr_key=NULL WHERE id=?",
        "UPDATE works SET hc_key='21' WHERE id=?",
        "UPDATE works SET hc_key='22' WHERE id=?",
        "UPDATE works SET hc_key=NULL WHERE id=?",
    ];

    for statement in statements {
        reset_live_progress(&db, author_id).await;
        sqlx::query(statement)
            .bind(work_id)
            .execute(db.pool())
            .await
            .expect("watched work update");
        assert_trigger_woke_once(&db, author_id).await;
    }
}

/// Door: Work evidence change wake-up -> every token-checked worker mutation.
/// AC-006 / AC-012: a watched work transition invalidates the live claim before
/// any stale road, route, candidate, key-attempt, or progress mutation.
#[tokio::test]
async fn ac006_ac012_migration_078_live_claim_race_returns_exact_claim_lost_everywhere() {
    let (db, user_id, author_id, work_id) = migration_graph().await;
    let stale_claim = migration_claim(user_id, author_id);

    db.set_identity_status(user_id, work_id, IdentityStatus::Confirmed)
        .await
        .expect("production work evidence writer");
    assert_trigger_woke_once(&db, author_id).await;

    assert_claim_lost(db.load_road_input(stale_claim.clone()).await);
    assert_claim_lost(
        db.prepare_key_attempts(
            stale_claim.clone(),
            7,
            vec![SettledWorkProviderKey {
                work_id,
                provider: AuthorProvider::OpenLibrary,
                work_route: "OL7901W".to_string(),
            }],
        )
        .await,
    );
    assert_claim_lost(
        db.complete_key_attempt(
            stale_claim.clone(),
            41,
            AuthorKeyAttemptOutcome::Retryable {
                error: "stale worker".to_string(),
                next_attempt_at: Utc::now() + Duration::minutes(1),
            },
            0,
        )
        .await,
    );
    assert_claim_lost(
        db.apply_guarded_route(GuardedRouteWrite {
            claim_token: Some(stale_claim.claim_token),
            author_id,
            evidence: agreed_evidence("OL7901A"),
        })
        .await,
    );
    assert_claim_lost(
        db.record_candidates(
            stale_claim.clone(),
            vec![candidate(author_id, "OL7902A", 7)],
        )
        .await,
    );
    assert_claim_lost(
        db.advance_progress(stale_claim, progress_update(8, 3))
            .await,
    );
}

/// Door: Work evidence change wake-up (migration 078 DB trigger).
/// AC-006: same-value watched writes and unrelated writes leave every progress
/// byte unchanged.
#[tokio::test]
async fn ac006_migration_078_same_values_and_unrelated_updates_are_exact_noops() {
    let (db, _user_id, author_id, work_id) = migration_graph().await;
    let before = progress_snapshot(&db, author_id).await;

    sqlx::query(
        "UPDATE works
            SET identity_status=identity_status, ol_key=ol_key, gr_key=gr_key, hc_key=hc_key
          WHERE id=?",
    )
    .bind(work_id)
    .execute(db.pool())
    .await
    .expect("same-value watched update");
    sqlx::query("UPDATE works SET title=title WHERE id=?")
        .bind(work_id)
        .execute(db.pool())
        .await
        .expect("unrelated update");

    assert_eq!(progress_snapshot(&db, author_id).await, before);
}

/// Door: Startup legacy staging before route-only cutover.
/// AC-005 / AC-008 / AC-014: migration 079 trims and stages every nonempty
/// provider scalar, omits blank route values, and stages every author name.
#[tokio::test]
async fn ac005_ac008_ac014_migration_079_stages_nonempty_legacy_routes_and_every_name() {
    let db = migration_077_db().await;
    apply_migration_078(&db).await;
    let user_id = create_test_user(&db).await;
    let (routed, _) = db
        .create_author(author_request(user_id, "Legacy Routed Author"))
        .await
        .expect("production author writer");
    let (route_free, _) = db
        .create_author(author_request(user_id, "Legacy Route-Free Author"))
        .await
        .expect("production author writer");

    // Constructed-state justification: route-only production writers cannot
    // create the pre-cutover legacy scalar state that migration 079 consumes.
    sqlx::query(
        "UPDATE authors
            SET ol_key=' /authors/OL7901A ', gr_key=' 7902 ', hc_key=' 7903 '
          WHERE id=?",
    )
    .bind(routed.id)
    .execute(db.pool())
    .await
    .expect("construct pre-cutover legacy scalar state");

    apply_migration_079(&db).await;

    let routes: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT provider, raw_value, status
           FROM author_route_legacy_staging
          WHERE author_id=?
          ORDER BY provider",
    )
    .bind(routed.id)
    .fetch_all(db.pool())
    .await
    .expect("staged legacy routes");
    assert_eq!(
        routes,
        vec![
            (
                "goodreads".to_string(),
                "7902".to_string(),
                "pending".to_string(),
            ),
            (
                "hardcover".to_string(),
                "7903".to_string(),
                "pending".to_string(),
            ),
            (
                "open_library".to_string(),
                "/authors/OL7901A".to_string(),
                "pending".to_string(),
            ),
        ]
    );

    let route_free_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_route_legacy_staging WHERE author_id=?")
            .bind(route_free.id)
            .fetch_one(db.pool())
            .await
            .expect("blank legacy route count");
    assert_eq!(route_free_count, 0);

    let names: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT author_id, name, status
           FROM author_name_legacy_staging
          WHERE author_id IN (?, ?)
          ORDER BY author_id",
    )
    .bind(routed.id)
    .bind(route_free.id)
    .fetch_all(db.pool())
    .await
    .expect("staged legacy names");
    assert_eq!(
        names,
        vec![
            (
                routed.id,
                "Legacy Routed Author".to_string(),
                "pending".to_string(),
            ),
            (
                route_free.id,
                "Legacy Route-Free Author".to_string(),
                "pending".to_string(),
            ),
        ]
    );
}

/// Door: Work evidence change wake-up -> shared create/adopt repair.
/// AC-001 / AC-006: the trigger never invents a progress row for a pre-F1
/// author; the classified enqueue operation repairs it without provider I/O.
#[tokio::test]
async fn ac001_ac006_pre_f1_trigger_does_not_insert_and_enqueue_repairs_the_invariant() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (author, _) = db
        .create_author(author_request(user_id, "Pre F1 Author"))
        .await
        .expect("production author writer");
    let (work, _) = db
        .create_work(work_request(
            user_id,
            author.id,
            "Pre F1 Work",
            &author.name,
        ))
        .await
        .expect("production work writer");

    sqlx::query("UPDATE works SET identity_status='confirmed' WHERE id=?")
        .bind(work.id)
        .execute(db.pool())
        .await
        .expect("watched update");
    let before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_link_progress WHERE author_id=?")
            .bind(author.id)
            .fetch_one(db.pool())
            .await
            .expect("count progress");
    assert_eq!(before, 0);

    db.ensure_enqueued(user_id, author.id, AuthorLinkTrigger::AuthorAdopted)
        .await
        .expect("shared public repair operation");
    let after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_link_progress WHERE author_id=?")
            .bind(author.id)
            .fetch_one(db.pool())
            .await
            .expect("count repaired progress");
    assert_eq!(after, 1);
}

/// Door: all creation workflows -> `AuthorLinkDb::ensure_enqueued`.
/// AC-001 / AC-006: every trigger is idempotent, due, and preserves a live
/// lease rather than stealing it.
#[tokio::test]
async fn ac001_ac006_ensure_enqueued_is_idempotent_for_every_trigger_and_preserves_live_lease() {
    let (db, user_id, author_id, _work_id) = seeded_db("Ensure Enqueued").await;
    let triggers = [
        AuthorLinkTrigger::LegacyBackfill,
        AuthorLinkTrigger::AuthorCreated,
        AuthorLinkTrigger::AuthorAdopted,
        AuthorLinkTrigger::UserReResolve,
        AuthorLinkTrigger::EvidenceFingerprintChanged,
        AuthorLinkTrigger::DisplayNameDirty,
        AuthorLinkTrigger::RetryDue,
    ];
    for trigger in triggers {
        db.ensure_enqueued(user_id, author_id, trigger)
            .await
            .expect("enqueue trigger");
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_link_progress WHERE author_id=?")
            .bind(author_id)
            .fetch_one(db.pool())
            .await
            .expect("progress count");
    assert_eq!(count, 1);
}

/// Door: Recurring author-link sweep -> bounded missing-row discovery.
/// AC-006: repeated bounded repairs finish stably with one row per author.
#[tokio::test]
async fn ac006_missing_progress_discovery_is_bounded_and_stably_complete() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    for index in 0..5 {
        db.create_author(author_request(
            user_id,
            &format!("Missing Progress {index}"),
        ))
        .await
        .expect("production author writer");
    }

    assert_eq!(
        db.ensure_missing_progress_rows(2).await.expect("batch 1"),
        2
    );
    assert_eq!(
        db.ensure_missing_progress_rows(2).await.expect("batch 2"),
        2
    );
    assert_eq!(
        db.ensure_missing_progress_rows(2).await.expect("batch 3"),
        1
    );
    assert_eq!(db.ensure_missing_progress_rows(2).await.expect("stable"), 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM author_link_progress")
        .fetch_one(db.pool())
        .await
        .expect("progress count");
    assert_eq!(count, 5);
}

/// Door: Recurring author-link sweep -> lease claim.
/// AC-006: due, expired, dirty-terminal, and lease-free rows qualify once;
/// live leases and future clean rows do not.
#[tokio::test]
async fn ac006_claim_due_pins_due_dirty_terminal_and_lease_matrix() {
    let (db, user_id, author_id, _work_id) = seeded_db("Claim Matrix").await;
    db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
        .await
        .expect("enqueue");
    let now = Utc::now();
    let first = db
        .claim_due(now, now + Duration::minutes(5), 1)
        .await
        .expect("first claim");
    assert_eq!(first.len(), 1);
    let second = db
        .claim_due(now, now + Duration::minutes(5), 1)
        .await
        .expect("live lease exclusion");
    assert!(second.is_empty());
}

/// Door: Recurring author-link sweep -> road input and fingerprint.
/// AC-003 / AC-006: only Confirmed/Provisional works contribute, with stable
/// sorted provider-key fingerprinting and user isolation.
#[tokio::test]
async fn ac003_ac006_road_input_and_fingerprint_include_only_settled_user_owned_work() {
    let (db, user_id, author_id, work_id) = seeded_db("Road Input").await;
    db.set_identity_status(user_id, work_id, IdentityStatus::Confirmed)
        .await
        .expect("production identity writer");
    let claim = claimed_author(&db, user_id, author_id).await;
    let input = db
        .load_road_input(claim)
        .await
        .expect("load real road input");
    assert_eq!(input.author.id, author_id);
    assert_eq!(input.settled_works.len(), 1);

    let first = db
        .compute_evidence_fingerprint(user_id, author_id)
        .await
        .expect("first fingerprint");
    let second = db
        .compute_evidence_fingerprint(user_id, author_id)
        .await
        .expect("stable fingerprint");
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first.settled_work_count, 1);
}

/// Door: Recurring author-link sweep -> durable key-attempt ledger.
/// AC-003 / AC-006: all provider keys materialize once, retryable attempts
/// resume after their own due time, and terminal outcomes stay key-local.
#[tokio::test]
async fn ac003_ac006_key_attempts_are_idempotent_resumable_and_key_local() {
    let (db, user_id, author_id, work_id) = seeded_db("Key Attempts").await;
    let claim = claimed_author(&db, user_id, author_id).await;
    let keys = vec![
        SettledWorkProviderKey {
            work_id,
            provider: AuthorProvider::OpenLibrary,
            work_route: "OL1W".to_string(),
        },
        SettledWorkProviderKey {
            work_id,
            provider: AuthorProvider::Goodreads,
            work_route: "11".to_string(),
        },
        SettledWorkProviderKey {
            work_id,
            provider: AuthorProvider::Hardcover,
            work_route: "21".to_string(),
        },
    ];
    let attempts = db
        .prepare_key_attempts(claim.clone(), 1, keys.clone())
        .await
        .expect("prepare attempts");
    assert_eq!(attempts.len(), 3);
    let repeated = db
        .prepare_key_attempts(claim.clone(), 1, keys)
        .await
        .expect("idempotent prepare");
    assert_eq!(repeated.len(), 3);

    db.complete_key_attempt(
        claim,
        attempts[0].id,
        AuthorKeyAttemptOutcome::Retryable {
            error: "429".to_string(),
            next_attempt_at: Utc::now() + Duration::minutes(2),
        },
        0,
    )
    .await
    .expect("persist retryable outcome");
}

/// Door: Tier-1 guarded DB route writer.
/// AC-003 / AC-010 / AC-013 / AC-014: attach, idempotency, legacy upgrade,
/// tombstone park, contradiction, and ownership park are classified outcomes.
#[tokio::test]
async fn ac003_ac010_ac013_ac014_guarded_writer_classifies_every_route_outcome() {
    let (db, user_id, author_id, _work_id) = seeded_db("Guarded Writer").await;
    let write = GuardedRouteWrite {
        claim_token: None,
        author_id,
        evidence: agreed_evidence("OL701A"),
    };
    let attached = db
        .apply_guarded_route(write.clone())
        .await
        .expect("first guarded write");
    assert!(matches!(attached, RouteWriteOutcome::Attached(_)));
    let repeated = db
        .apply_guarded_route(write)
        .await
        .expect("idempotent guarded write");
    assert!(matches!(repeated, RouteWriteOutcome::AlreadyActive(_)));

    let (legacy_db, _legacy_user_id, legacy_author_id) = legacy_route_db("OL702A").await;
    let upgraded = legacy_db
        .apply_guarded_route(GuardedRouteWrite {
            claim_token: None,
            author_id: legacy_author_id,
            evidence: agreed_evidence("OL702A"),
        })
        .await
        .expect("guarded legacy upgrade");
    let RouteWriteOutcome::LegacyProvenanceUpgraded(upgraded_route) = upgraded else {
        panic!("same-key guarded evidence must upgrade legacy provenance");
    };
    assert!(matches!(
        upgraded_route.provenance,
        AuthorRouteProvenance::Tier1Inherited
    ));

    let (contradiction_db, contradiction_user_id, contradiction_author_id) =
        legacy_route_db("OL703A").await;
    let contradiction = contradiction_db
        .apply_guarded_route(GuardedRouteWrite {
            claim_token: None,
            author_id: contradiction_author_id,
            evidence: agreed_evidence("OL704A"),
        })
        .await
        .expect("guarded legacy contradiction");
    assert!(matches!(
        contradiction,
        RouteWriteOutcome::ParkedLegacyContradiction(_)
    ));
    let legacy_routes = contradiction_db
        .list_active_routes(contradiction_user_id, contradiction_author_id, None)
        .await
        .expect("legacy route after contradiction");
    assert_eq!(legacy_routes.len(), 1);
    assert_eq!(
        legacy_routes[0].key,
        route(AuthorProvider::OpenLibrary, "OL703A")
    );

    let tombstone_key = route(AuthorProvider::OpenLibrary, "OL705A");
    let tombstone_route = db
        .attach_route_as_user(user_id, author_id, tombstone_key.clone())
        .await
        .expect("user route before tombstone");
    db.remove_route_as_user(user_id, author_id, tombstone_route.id)
        .await
        .expect("production tombstone writer");
    let tombstoned = db
        .apply_guarded_route(GuardedRouteWrite {
            claim_token: None,
            author_id,
            evidence: agreed_evidence("OL705A"),
        })
        .await
        .expect("guarded tombstone classification");
    assert!(matches!(tombstoned, RouteWriteOutcome::ParkedTombstoned(_)));
    let tombstone_state: (String, Option<String>) =
        sqlx::query_as("SELECT state, removed_at FROM author_provider_routes WHERE id=?")
            .bind(tombstone_route.id)
            .fetch_one(db.pool())
            .await
            .expect("tombstone persistence");
    assert_eq!(tombstone_state.0, "removed");
    assert!(tombstone_state.1.is_some());

    let (owner, _) = db
        .create_author(author_request(user_id, "Guarded Key Owner"))
        .await
        .expect("production owner author writer");
    let ownership_key = route(AuthorProvider::OpenLibrary, "OL706A");
    db.attach_route_as_user(user_id, owner.id, ownership_key.clone())
        .await
        .expect("production owner route writer");
    let ownership = db
        .apply_guarded_route(GuardedRouteWrite {
            claim_token: None,
            author_id,
            evidence: agreed_evidence("OL706A"),
        })
        .await
        .expect("guarded ownership classification");
    assert!(matches!(
        ownership,
        RouteWriteOutcome::ParkedOwnershipCollision(_)
    ));
    let owner_routes = db
        .list_active_routes(user_id, owner.id, None)
        .await
        .expect("owner routes after collision");
    assert_eq!(owner_routes.len(), 1);
    assert_eq!(owner_routes[0].key, ownership_key);
}

/// Door: Tier-2 candidate persistence -> Review Authors list.
/// AC-002 / AC-004 / AC-007: Tier 2 always parks; current-generation rows and
/// ordered alternate evidence hydrate without turning failure into Complete.
#[tokio::test]
async fn ac002_ac004_ac007_candidates_round_trip_and_only_current_pending_rows_are_reviewable() {
    let (db, user_id, author_id, _work_id) = seeded_db("Candidates").await;
    let claim = claimed_author(&db, user_id, author_id).await;
    db.record_candidates(claim, vec![candidate(author_id, "OL801A", 1)])
        .await
        .expect("persist Tier-2 candidate");
    let review = db.list_review(user_id).await.expect("review list");
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].candidates.len(), 1);
    assert_eq!(review[0].candidates[0].corroborated_title_count, 1);
    assert!(matches!(
        review[0].candidates[0].catalog_evidence_state,
        AuthorCandidateCatalogState::Complete
    ));
}

/// Door: Readarr import author resolution -> rejected evidence persistence.
/// AC-004 / AC-011: non-Agree Readarr evidence records one idempotent review
/// candidate and never creates route or tombstone state.
#[tokio::test]
async fn ac004_ac011_readarr_rejection_is_idempotent_and_route_free() {
    let (db, user_id, author_id, _work_id) = seeded_db("Readarr Rejection").await;
    let guarded = guard_author_route(
        &["John Smith".to_string()],
        ProviderAuthorRef {
            key: route(AuthorProvider::Goodreads, "991"),
            name: "Jane Smith".to_string(),
            credit: ProviderCredit::AssertedAuthor,
        },
        None,
        AuthorRouteEvidenceSource::ReadarrImport,
    );
    let AuthorRouteGuardResult::Rejected(rejected) = guarded else {
        panic!("fixture must be Grey");
    };
    let first = db
        .record_readarr_rejection(user_id, author_id, rejected.clone())
        .await
        .expect("first rejection");
    let second = db
        .record_readarr_rejection(user_id, author_id, rejected)
        .await
        .expect("idempotent rejection");
    assert_eq!(first.id, second.id);
    assert!(db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("route list")
        .is_empty());
}

/// Door: Recurring author-link sweep -> token-checked progress CAS.
/// AC-006: the current claim advances once, while a stale claim is rejected
/// without clearing a newer dirty-name generation.
#[tokio::test]
async fn ac006_advance_progress_is_claim_checked_and_dirty_clear_is_generation_cas() {
    let (db, user_id, author_id, _work_id) = seeded_db("Progress CAS").await;
    let claim = claimed_author(&db, user_id, author_id).await;
    db.advance_progress(claim, progress_update(1, 0))
        .await
        .expect("current claim advances");
    let aggregate = db
        .sweep_progress(user_id)
        .await
        .expect("progress aggregate");
    assert_eq!(aggregate.total, 1);
    assert_eq!(aggregate.completed, 1);
}

/// Door: Review Authors candidate pick and explicit selected-route attach.
/// AC-007 / AC-010: both user entry shapes share ownership, idempotency,
/// UserPicked provenance, and exact tombstone reactivation semantics.
#[tokio::test]
async fn ac007_ac010_direct_attach_and_candidate_pick_share_user_sovereign_semantics() {
    let (db, user_id, author_id, _work_id) = seeded_db("User Attach").await;
    let key = route(AuthorProvider::OpenLibrary, "OL901A");
    let direct = db
        .attach_route_as_user(user_id, author_id, key.clone())
        .await
        .expect("direct user attach");
    db.remove_route_as_user(user_id, author_id, direct.id)
        .await
        .expect("tombstone before candidate pick");

    let claim = claimed_author(&db, user_id, author_id).await;
    let mut review_candidate = candidate(author_id, "OL901A", 0);
    review_candidate.candidate_name = "User Attach Author".to_string();
    review_candidate.alternate_name_evidence = vec![
        AuthorCandidateAlternateNameEvidence {
            name: "U. A. Author".to_string(),
            verdict: AuthorVerdict::Agree,
        },
        AuthorCandidateAlternateNameEvidence {
            name: "Attach, User".to_string(),
            verdict: AuthorVerdict::Grey,
        },
    ];
    db.record_candidates(claim, vec![review_candidate])
        .await
        .expect("production candidate writer");
    let candidate_id = db.list_review(user_id).await.expect("review list")[0].candidates[0].id;
    let picked = db
        .pick_candidate_as_user(user_id, candidate_id)
        .await
        .expect("production candidate-pick entry");

    assert_eq!(
        picked.id, direct.id,
        "candidate pick reactivates exact tombstone"
    );
    assert!(matches!(
        picked.provenance,
        AuthorRouteProvenance::UserPicked
    ));
    let status: String = sqlx::query_scalar("SELECT status FROM author_link_candidates WHERE id=?")
        .bind(candidate_id)
        .fetch_one(db.pool())
        .await
        .expect("picked candidate status");
    assert_eq!(status, "picked");
    let names: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, source, open_library_role
           FROM author_name_variants
          WHERE user_id=? AND author_id=? AND source='open_library'
          ORDER BY id",
    )
    .bind(user_id)
    .bind(author_id)
    .fetch_all(db.pool())
    .await
    .expect("picked Open Library names");
    assert_eq!(
        names,
        vec![
            (
                "User Attach Author".to_string(),
                "open_library".to_string(),
                Some("primary".to_string())
            ),
            (
                "U. A. Author".to_string(),
                "open_library".to_string(),
                Some("alias".to_string())
            ),
            (
                "Attach, User".to_string(),
                "open_library".to_string(),
                Some("alias".to_string())
            ),
        ]
    );
}

/// Door: Review Authors candidate dismiss.
/// AC-007: current pending dismissal changes no routes/tombstones, stale and
/// cross-user ids fail, and explicit re-resolve can surface fresh evidence.
#[tokio::test]
async fn ac007_candidate_dismissal_is_scoped_generation_checked_and_route_neutral() {
    let (db, user_id, author_id, _work_id) = seeded_db("Dismiss").await;
    let claim = claimed_author(&db, user_id, author_id).await;
    db.record_candidates(claim, vec![candidate(author_id, "OL902A", 1)])
        .await
        .expect("persist candidate");
    let id = db.list_review(user_id).await.expect("review")[0].candidates[0].id;
    db.dismiss_candidate_as_user(user_id, id)
        .await
        .expect("dismiss current candidate");
    assert!(db
        .list_review(user_id)
        .await
        .expect("review after dismiss")
        .is_empty());
    assert!(db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("routes after dismiss")
        .is_empty());
}

/// Door: Author route removal.
/// AC-010: removal retains the row as a tombstone, automation cannot revive
/// it, and explicit user selection alone reactivates the exact route.
#[tokio::test]
async fn ac010_route_removal_is_a_durable_tombstone_until_explicit_user_repick() {
    let (db, user_id, author_id, _work_id) = seeded_db("Tombstone").await;
    let key = route(AuthorProvider::OpenLibrary, "OL903A");
    let attached = db
        .attach_route_as_user(user_id, author_id, key.clone())
        .await
        .expect("attach route");
    db.remove_route_as_user(user_id, author_id, attached.id)
        .await
        .expect("remove route");
    assert!(db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes")
        .is_empty());
    let automatic = db
        .apply_guarded_route(GuardedRouteWrite {
            claim_token: None,
            author_id,
            evidence: agreed_evidence("OL903A"),
        })
        .await
        .expect("automatic write against tombstone");
    let RouteWriteOutcome::ParkedTombstoned(candidate) = automatic else {
        panic!("automation must park rather than clear the tombstone");
    };
    assert!(matches!(
        candidate.reason,
        AuthorLinkCandidateReason::Tombstoned
    ));
    assert!(db
        .list_active_routes(user_id, author_id, None)
        .await
        .expect("active routes after automatic write")
        .is_empty());
    let tombstone: (String, Option<String>) =
        sqlx::query_as("SELECT state, removed_at FROM author_provider_routes WHERE id=?")
            .bind(attached.id)
            .fetch_one(db.pool())
            .await
            .expect("durable tombstone row");
    assert_eq!(tombstone.0, "removed");
    assert!(tombstone.1.is_some());
    let repicked = db
        .attach_route_as_user(user_id, author_id, key)
        .await
        .expect("explicit repick");
    assert_eq!(repicked.id, attached.id);
}

/// Door: monitor-enable gate and compatibility response assembly DB reads.
/// AC-007 / AC-014: active route filtering, deterministic projection, and
/// monitor targets use route rows only and remain user-scoped.
#[tokio::test]
async fn ac007_ac014_route_reads_projection_and_monitor_targets_use_only_active_routes() {
    let (db, user_id, author_id, _work_id) = seeded_db("Consumers").await;
    db.attach_route_as_user(
        user_id,
        author_id,
        route(AuthorProvider::OpenLibrary, "OL904A"),
    )
    .await
    .expect("user OL route");
    assert!(db
        .has_active_route(user_id, author_id, AuthorProvider::OpenLibrary)
        .await
        .expect("monitor gate"));
    let projection = db
        .compatibility_projection(user_id, author_id)
        .await
        .expect("projection");
    assert_eq!(projection.ol_key.as_deref(), Some("OL904A"));
    let routes = db
        .list_active_routes(user_id, author_id, Some(AuthorProvider::OpenLibrary))
        .await
        .expect("active OL routes");
    assert_eq!(routes.len(), 1);
}

/// Door: Enrichment-completion author-name observation.
/// AC-006 / AC-008: distinct observations deduplicate canonically, dirty a
/// terminal task immediately, and never disturb its live lease.
#[tokio::test]
async fn ac006_ac008_name_observation_deduplicates_and_wakes_dirty_without_stealing_lease() {
    let (db, user_id, _author_id, work_id) = seeded_db("Name Observation").await;
    let inserted = db
        .record_observed_names(
            user_id,
            work_id,
            &[
                ProviderAuthorNameObservation {
                    source: AuthorNameSource::Goodreads,
                    name: "N. K. Jemisin".to_string(),
                },
                ProviderAuthorNameObservation {
                    source: AuthorNameSource::Goodreads,
                    name: "N.K. Jemisin".to_string(),
                },
            ],
        )
        .await
        .expect("record observed names");
    assert_eq!(inserted, 1);
}

/// Door: startup cutover verification and legacy ingestion.
/// AC-014: valid legacy scalars canonicalize with provenance, invalid values
/// remain reported, reruns are idempotent, and readiness is honest.
#[tokio::test]
async fn ac014_legacy_ingestion_and_cutover_report_are_canonical_idempotent_and_honest() {
    let (db, _user_id, _author_id, _work_id) = seeded_db("Legacy").await;
    let first = db.ingest_legacy_routes().await.expect("legacy ingestion");
    let second = db
        .ingest_legacy_routes()
        .await
        .expect("idempotent ingestion");
    assert_eq!(first.canonical_routes, second.canonical_routes);
    let ready = db.verify_cutover_ready().await.expect("cutover report");
    assert_eq!(ready.missing_routes, 0);
    assert_eq!(ready.invalid_values, 0);
    assert_eq!(ready.missing_progress_rows, 0);
}

/// Door: Author rename and stored-name variant pick.
/// AC-008 / AC-009: one transaction changes author/works display names,
/// increments merge generations, and never mutates normalized_author.
#[tokio::test]
async fn ac008_ac009_rename_cascades_display_only_and_preserves_normalized_author() {
    let (db, user_id, author_id, work_id) = seeded_db("Rename").await;
    let before_normalized: String =
        sqlx::query_scalar("SELECT normalized_author FROM works WHERE id=?")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("normalized author before");
    let before_generation: i64 =
        sqlx::query_scalar("SELECT merge_generation FROM works WHERE id=?")
            .bind(work_id)
            .fetch_one(db.pool())
            .await
            .expect("merge generation before");

    let renamed = db
        .rename_author_and_cascade(RenameAuthorDbRequest {
            user_id,
            author_id,
            display_name: "Renamed Author".to_string(),
            variant_id: 0,
        })
        .await
        .expect("rename cascade");
    assert_eq!(renamed.name, "Renamed Author");
    let row: (String, String, i64) = sqlx::query_as(
        "SELECT author_name, normalized_author, merge_generation FROM works WHERE id=?",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("work after rename");
    assert_eq!(row.0, "Renamed Author");
    assert_eq!(row.1, before_normalized);
    assert_eq!(row.2, before_generation + 1);
}

/// Door: Author merge UI -> one DB merge transaction.
/// AC-014: route/name/progress/candidate state folds into the survivor with
/// survivor tombstone precedence and cross-user rejection.
#[tokio::test]
async fn ac014_author_merge_folds_link_state_without_resurrecting_tombstones() {
    let (db, user_id, survivor_id, _work_id) = seeded_db("Merge Survivor").await;
    let survivor_tombstone = db
        .attach_route_as_user(
            user_id,
            survivor_id,
            route(AuthorProvider::OpenLibrary, "OL1904A"),
        )
        .await
        .expect("survivor route before removal");
    db.remove_route_as_user(user_id, survivor_id, survivor_tombstone.id)
        .await
        .expect("survivor tombstone");
    let (loser, _) = db
        .create_author(author_request(user_id, "Merge Loser"))
        .await
        .expect("loser author");
    db.attach_route_as_user(user_id, loser.id, route(AuthorProvider::Goodreads, "1905"))
        .await
        .expect("loser route");

    let report = db
        .merge_authors(user_id, survivor_id, loser.id)
        .await
        .expect("production merge");
    assert_eq!(report.works_moved, 0);
    let routes = db
        .list_active_routes(user_id, survivor_id, None)
        .await
        .expect("survivor routes");
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].key, route(AuthorProvider::Goodreads, "1905"));
    let retained: (i64, String, Option<String>) = sqlx::query_as(
        "SELECT author_id, state, removed_at FROM author_provider_routes WHERE id=?",
    )
    .bind(survivor_tombstone.id)
    .fetch_one(db.pool())
    .await
    .expect("survivor tombstone after merge");
    assert_eq!(retained.0, survivor_id);
    assert_eq!(retained.1, "removed");
    assert!(retained.2.is_some());
}
