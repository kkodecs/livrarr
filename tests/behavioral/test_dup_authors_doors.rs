//! Behavioral acceptance for issue #175 beyond the AC-001 repro
//! (`test_dup_authors_import_race.rs`): per-door race-loser honesty
//! (AC-002 a/b), the live junk-name NULL-key door (AC-004), and the rename
//! door's stored-key maintenance (AC-006). Spec:
//! `spec-bugfix-175-duplicate-authors.md`.
//!
//! Every test drives a real production seam — `WorkService::add`,
//! `AuthorService::add`, `AuthorService::update` — over the real `SqliteDb`
//! writer; no injected state. (The Readarr door's AC-002(c) coverage lives
//! with the extracted batch loop in `readarr_import_workflow.rs`; the
//! DB-level AC-004 enforcement and the AC-003/AC-005 repair suite live in
//! `livrarr-db/src/pool.rs`.)

use livrarr_behavioral::stubs::{
    create_test_user, StubEnrichmentWorkflow, StubHttpFetcher, StubLlmCaller,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::AuthorDb;
use livrarr_domain::identity::{CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate};
use livrarr_domain::identity_matching::canonical_author_key;
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{
    AddAuthorRequest, AuthorService, AuthorServiceError, UpdateAuthorRequest, WorkService,
};
use livrarr_metadata::author_service::AuthorServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;
type TestAuthorService = AuthorServiceImpl<SqliteDb, StubHttpFetcher, StubLlmCaller>;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-dup-authors-doors-{}", std::process::id()))
}

fn work_service(db: SqliteDb) -> TestWorkService {
    WorkServiceImpl::new(
        db,
        StubEnrichmentWorkflow::succeeding(),
        StubHttpFetcher::new(),
        test_data_dir(),
    )
}

fn author_service(db: SqliteDb) -> TestAuthorService {
    AuthorServiceImpl::new(db, StubHttpFetcher::new(), StubLlmCaller::not_configured())
}

fn seed_input(title: &str, author: &str) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: author.to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: None,
        year: Some(2024),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn confirmed_candidate(title: &str, author: &str, ol_key: &str) -> WorkCandidate {
    seed_add_box(
        seed_input(title, author),
        IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: Some(ol_key.to_string()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: title.to_string(),
                author_name: author.to_string(),
                language: Some("en".to_string()),
            },
            method: IdentityMethod::UserSelected,
            score: None,
        },
        None,
        false,
    )
}

fn add_req(name: &str) -> AddAuthorRequest {
    AddAuthorRequest {
        name: name.to_string(),
        sort_name: None,
        ol_key: None,
        monitored: false,
    }
}

fn rename_req(name: &str) -> UpdateAuthorRequest {
    UpdateAuthorRequest {
        name: Some(name.to_string()),
        sort_name: None,
        ol_key: None,
        gr_key: None,
        monitored: None,
        monitor_new_items: None,
        monitor_language: None,
    }
}

async fn stored_key(db: &SqliteDb, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT normalized_name FROM authors WHERE id = ?")
        .bind(id)
        .fetch_one(db.pool())
        .await
        .expect("read stored key")
}

/// AC-002(a): concurrent `work_service.add` candidates naming the same
/// author leave one author row; both results carry the winner's id and at
/// most one reports `author_created`.
#[tokio::test]
async fn ac002a_concurrent_work_adds_converge_on_one_author_with_one_created_signal() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = work_service(db.clone());

    let (r1, r2) = tokio::join!(
        svc.add(
            user_id,
            confirmed_candidate("Race Book One", "Anne Rice", "OLRB1W")
        ),
        svc.add(
            user_id,
            confirmed_candidate("Race Book Two", "Anne Rice", "OLRB2W")
        ),
    );
    let r1 = r1.expect("first concurrent add");
    let r2 = r2.expect("second concurrent add");

    let authors = db.list_authors(user_id).await.expect("list authors");
    let rice: Vec<_> = authors.iter().filter(|a| a.name == "Anne Rice").collect();
    assert_eq!(
        rice.len(),
        1,
        "exactly one Anne Rice row after concurrent adds; got ids {:?}",
        rice.iter().map(|a| a.id).collect::<Vec<_>>()
    );
    let winner = rice[0].id;
    assert_eq!(
        r1.author_id,
        Some(winner),
        "first add carries the winner id"
    );
    assert_eq!(
        r2.author_id,
        Some(winner),
        "second add carries the winner id"
    );
    assert!(
        !(r1.author_created && r2.author_created),
        "at most one add may report author_created (REQ-002)"
    );
}

/// AC-002(b): the manual author-add door racing the same name converges —
/// exactly one Created, the other the Updated/adopted shape, same row.
#[tokio::test]
async fn ac002b_concurrent_manual_adds_converge_one_created_one_updated() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = author_service(db.clone());

    let (r1, r2) = tokio::join!(
        svc.add(user_id, add_req("Diana Gabaldon")),
        svc.add(user_id, add_req("Diana Gabaldon")),
    );
    let r1 = r1.expect("first concurrent manual add");
    let r2 = r2.expect("second concurrent manual add");

    let authors = db.list_authors(user_id).await.expect("list authors");
    let matching: Vec<_> = authors
        .iter()
        .filter(|a| a.name == "Diana Gabaldon")
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "exactly one Diana Gabaldon row; got ids {:?}",
        matching.iter().map(|a| a.id).collect::<Vec<_>>()
    );
    let winner = matching[0].id;
    assert_eq!(r1.author().id, winner);
    assert_eq!(r2.author().id, winner);
    let created_count = [&r1, &r2].iter().filter(|r| r.is_created()).count();
    assert_eq!(
        created_count, 1,
        "exactly one Created; the loser reports the Updated shape, never a second Created"
    );
}

/// AC-004 (live door): a non-canonicalizable name creates through the real
/// manual door, stores a NULL key (never ""), and rejects nothing — and a
/// second junk name creates separately (NULL keys never converge).
#[tokio::test]
async fn ac004_junk_name_creates_through_the_real_door_storing_null_key() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = author_service(db.clone());

    let first = svc
        .add(user_id, add_req("(Editor)"))
        .await
        .expect("junk-named create must not be rejected");
    assert!(first.is_created());
    let first_id = first.author().id;
    assert_eq!(
        stored_key(&db, first_id).await,
        None,
        "the door computes NULL for a non-canonicalizable name — never \"\""
    );

    let second = svc
        .add(user_id, add_req("Jr."))
        .await
        .expect("second junk-named create must not be rejected");
    assert!(second.is_created());
    assert_ne!(
        second.author().id,
        first_id,
        "distinct junk-named authors stay separate rows (ST-010)"
    );
    assert_eq!(stored_key(&db, second.author().id).await, None);
}

/// AC-006: renaming recomputes the stored key in the same transaction —
/// verified by a subsequent create of the OLD name producing a new row and
/// a create of the NEW name converging on the renamed row.
#[tokio::test]
async fn ac006_rename_recomputes_key_old_name_vacates_new_name_converges() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = author_service(db.clone());

    let created = svc
        .add(user_id, add_req("Margaret Atwood"))
        .await
        .expect("seed author");
    let renamed_id = created.author().id;

    svc.update(user_id, renamed_id, rename_req("Robin Hobb"))
        .await
        .expect("rename");
    assert_eq!(
        stored_key(&db, renamed_id).await.as_deref(),
        Some(canonical_author_key("Robin Hobb").as_str()),
        "rename must recompute the stored key"
    );

    let old_again = svc
        .add(user_id, add_req("Margaret Atwood"))
        .await
        .expect("re-add of the old name");
    assert!(
        old_again.is_created(),
        "the old key was vacated by the rename, so the old name creates fresh"
    );
    assert_ne!(old_again.author().id, renamed_id);

    let new_again = svc
        .add(user_id, add_req("Robin Hobb"))
        .await
        .expect("add of the new name");
    assert!(
        !new_again.is_created(),
        "the new name converges, not creates"
    );
    assert_eq!(new_again.author().id, renamed_id);
}

/// AC-006: renaming onto a different author's key is rejected with a
/// validation error that names the colliding author, and changes no rows.
#[tokio::test]
async fn ac006_rename_onto_existing_key_rejected_naming_the_collider() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = author_service(db.clone());

    let rice = svc
        .add(user_id, add_req("Anne Rice"))
        .await
        .expect("seed collider");
    let rice_id = rice.author().id;
    let gabaldon = svc
        .add(user_id, add_req("Diana Gabaldon"))
        .await
        .expect("seed rename target");
    let gabaldon_id = gabaldon.author().id;

    let err = svc
        .update(user_id, gabaldon_id, rename_req("Anne Rice"))
        .await
        .expect_err("rename onto an existing key must be rejected");
    match err {
        AuthorServiceError::Validation { field, message } => {
            assert_eq!(field, "name");
            assert!(
                message.contains("Anne Rice") && message.contains(&format!("(id {rice_id})")),
                "the error must name the colliding author: {message}"
            );
        }
        other => panic!("expected Validation, got: {other:?}"),
    }

    let target = db
        .get_author(user_id, gabaldon_id)
        .await
        .expect("rename target still readable");
    assert_eq!(
        target.name, "Diana Gabaldon",
        "a rejected rename must change no rows"
    );
    let rice_rows = db
        .list_authors(user_id)
        .await
        .expect("list authors")
        .into_iter()
        .filter(|a| a.name == "Anne Rice")
        .count();
    assert_eq!(rice_rows, 1);
}

/// AC-006: renaming TO a non-canonicalizable name succeeds with the key
/// recomputed to NULL — including when another NULL-key row already exists
/// (no collision, ST-010 exemption).
#[tokio::test]
async fn ac006_rename_to_junk_stores_null_beside_existing_null_row() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = author_service(db.clone());

    svc.add(user_id, add_req("(Editor)"))
        .await
        .expect("pre-existing NULL-key row");
    let sagan = svc
        .add(user_id, add_req("Carl Sagan"))
        .await
        .expect("seed rename target");
    let sagan_id = sagan.author().id;

    let renamed = svc
        .update(user_id, sagan_id, rename_req("Jr."))
        .await
        .expect("rename to a junk name must succeed despite the existing NULL-key row");
    assert_eq!(renamed.name, "Jr.");
    assert_eq!(
        stored_key(&db, sagan_id).await,
        None,
        "the recomputed key for a junk name is NULL"
    );
}
