//! RED behavioral suite for design-author-dedup.
//!
//! Coverage map:
//! - U-2 / design §3.2: real `WorkService::add` create-gate adoption, exact fast path,
//!   monotonic author key fill, and ambiguous-compatible variants creating a visible split.
//! - U-2 / design §3.2: real `AuthorService::add` exact-hit and adoption arms preserve stored
//!   OL keys when the request carries a conflicting key.
//! - U-3 / design §1 and §3.3: real `AuthorDb::merge_authors`, `AuthorService::merge`, and
//!   the real axum handler route `POST /author/{id}/merge`.
//! - Readarr-shaped work-service door pins: `ReadarrImportWorkflow::process_authors` is
//!   private behind the crate-local `ImportRunner`, while `LiveReadarrImportWorkflow::new`
//!   is tied to production `LiveWorkService`/HTTP/import composition. The behavioral crate
//!   cannot reach the real batch-list seam without a production visibility/API change.
//!
//! DEFERRED-PIN:
//! - When implementing author-dedup in `readarr_import_workflow.rs`, add direct
//!   `process_authors` batch-list tests in that module or through a deliberately exposed
//!   test seam. They MUST pin: two in-batch spelling variants absent from the DB converge
//!   to one Livrarr author with both Readarr ids mapped, and a later in-batch variant of a
//!   first-row-adopted author still adopts rather than creating or tripping raw duplicate
//!   match counts.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use chrono::{TimeZone, Utc};
use livrarr_behavioral::stubs::{
    create_second_test_user, create_test_user, StubEnrichmentWorkflow, StubHttpFetcher,
    StubLlmCaller,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{AuthorDb, CreateAuthorDbRequest, CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{
    CapturedIdentity, IdentityMethod, IdentityState, PendingReason, WorkCandidate,
};
use livrarr_domain::identity_matching::identity_key;
use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};
use livrarr_domain::services::{
    AddAuthorRequest, AuthorService, AuthorServiceError, SourceProviderData, WorkService,
};
use livrarr_domain::{
    AuthType, Author, AuthorId, DbError, ProvenanceSetter, User, UserId, UserRole, Work, WorkId,
};
use livrarr_handlers::context::HasAuthorService;
use livrarr_handlers::AuthContext;
use livrarr_metadata::author_service::AuthorServiceImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use serde_json::json;
use sqlx::Row;
use tower::ServiceExt;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;
type TestAuthorService = AuthorServiceImpl<SqliteDb, StubHttpFetcher, StubLlmCaller>;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-author-dedup-{}", std::process::id()))
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

fn seed_input(title: &str, author: &str, author_ol_key: Option<&str>) -> SeedInput {
    SeedInput {
        title: title.to_string(),
        author_name: author.to_string(),
        language: SeedLanguage::resolve(Some("en"), "en"),
        author_ol_key: author_ol_key.map(str::to_string),
        year: Some(2024),
        cover_url: None,
        detail_url: None,
        description: None,
        series_name: None,
        series_position: None,
    }
}

fn captured_identity(title: &str, author: &str, ol_key: Option<&str>) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: title.to_string(),
        author_name: author.to_string(),
        language: Some("en".to_string()),
    }
}

fn confirmed_candidate(
    title: &str,
    author: &str,
    ol_key: &str,
    author_ol_key: Option<&str>,
) -> WorkCandidate {
    seed_add_box(
        seed_input(title, author, author_ol_key),
        IdentityState::Confirmed {
            anchors: captured_identity(title, author, Some(ol_key)),
            method: IdentityMethod::UserSelected,
            score: None,
        },
        None,
        false,
    )
}

fn pending_readarr_candidate(
    title: &str,
    author: &str,
    author_ol_key: Option<&str>,
) -> WorkCandidate {
    let mut candidate = seed_add_box(
        seed_input(title, author, author_ol_key),
        IdentityState::Pending {
            reason: PendingReason::NoCandidates,
            seed_anchors: Some(captured_identity(title, author, None)),
            top_candidates: vec![],
        },
        None,
        false,
    );
    candidate.provenance_setter = Some(ProvenanceSetter::Import);
    candidate.source_provider_data = Some(SourceProviderData {
        description: Some("Readarr seed".to_string()),
        ..SourceProviderData::default()
    });
    candidate
}

async fn create_author(db: &SqliteDb, user_id: UserId, name: &str, ol_key: Option<&str>) -> Author {
    db.create_author(CreateAuthorDbRequest {
        user_id,
        name: name.to_string(),
        sort_name: None,
        ol_key: ol_key.map(str::to_string),
        gr_key: None,
        hc_key: None,
        import_id: None,
    })
    .await
    .expect("seed author")
}

async fn author_count(db: &SqliteDb, user_id: UserId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count authors")
}

#[tokio::test]
async fn work_add_author_gate_adopts_variants_preserves_key_policy_and_keeps_ambiguous_splits_visible(
) {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = work_service(db.clone());

    let existing = create_author(&db, user_id, "Robert A. Heinlein", None).await;
    let before = author_count(&db, user_id).await;
    let adopted = svc
        .add(
            user_id,
            confirmed_candidate(
                "The Moon Is a Harsh Mistress",
                "Robert Heinlein",
                "OLMOONW",
                Some("OLHEINLEINA"),
            ),
        )
        .await
        .expect("add variant work");

    assert_eq!(adopted.author_id, Some(existing.id));
    assert_eq!(author_count(&db, user_id).await, before);
    assert_eq!(
        db.get_author(user_id, existing.id)
            .await
            .expect("adopted author")
            .ol_key
            .as_deref(),
        Some("OLHEINLEINA"),
        "adoption must fill a missing stored ol_key"
    );

    let populated = create_author(&db, user_id, "J.K. Rowling", Some("OL_STORED")).await;
    let no_overwrite = svc
        .add(
            user_id,
            confirmed_candidate(
                "Harry Potter and the Test Gate",
                "JK Rowling",
                "OLHPW",
                Some("OL_CONFLICTING"),
            ),
        )
        .await
        .expect("add variant work with conflicting author key");
    assert_eq!(no_overwrite.author_id, Some(populated.id));
    assert_eq!(
        db.get_author(user_id, populated.id)
            .await
            .expect("stored key author")
            .ol_key
            .as_deref(),
        Some("OL_STORED"),
        "adoption must never overwrite a populated stored ol_key"
    );

    let fast_path = svc
        .add(
            user_id,
            confirmed_candidate(
                "Harry Potter Exact Reuse",
                "J.K. Rowling",
                "OLHP2W",
                Some("OL_IGNORED"),
            ),
        )
        .await
        .expect("add exact spelling work");
    assert_eq!(
        fast_path.author_id,
        Some(populated.id),
        "exact spelling fast path must still self-match"
    );

    create_author(&db, user_id, "John Smith", None).await;
    create_author(&db, user_id, "Jane Smith", None).await;
    let ambiguous_before = author_count(&db, user_id).await;
    let ambiguous = svc
        .add(
            user_id,
            confirmed_candidate("Ambiguous Initials", "J. Smith", "OLAMBIGW", None),
        )
        .await
        .expect("add ambiguous variant");
    assert!(ambiguous.author_created);
    assert_eq!(author_count(&db, user_id).await, ambiguous_before + 1);
}

#[tokio::test]
async fn standalone_author_add_exact_and_adoption_hits_preserve_conflicting_stored_ol_key() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = author_service(db.clone());

    let exact = create_author(&db, user_id, "J.K. Rowling", Some("OL_STORED_EXACT")).await;
    let exact_result = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "J.K. Rowling".to_string(),
                sort_name: Some("Rowling, J.K.".to_string()),
                ol_key: Some("OL_CONFLICT_EXACT".to_string()),
                monitored: false,
            },
        )
        .await
        .expect("exact add hit");
    assert_eq!(exact_result.author().id, exact.id);
    assert_eq!(
        db.get_author(user_id, exact.id)
            .await
            .expect("exact author")
            .ol_key
            .as_deref(),
        Some("OL_STORED_EXACT")
    );

    let adopt = create_author(&db, user_id, "W. E. B. Griffin", Some("OL_STORED_ADOPT")).await;
    let adopt_result = svc
        .add(
            user_id,
            AddAuthorRequest {
                name: "W.E.B. Griffin".to_string(),
                sort_name: None,
                ol_key: Some("OL_CONFLICT_ADOPT".to_string()),
                monitored: false,
            },
        )
        .await
        .expect("adoption add hit");
    assert_eq!(adopt_result.author().id, adopt.id);
    assert_eq!(
        author_count(&db, user_id).await,
        2,
        "adoption arm must not create a duplicate author row"
    );
    assert_eq!(
        db.get_author(user_id, adopt.id)
            .await
            .expect("adopted author")
            .ol_key
            .as_deref(),
        Some("OL_STORED_ADOPT")
    );
}

fn work_req(
    user_id: UserId,
    title: &str,
    author_name: &str,
    author_id: AuthorId,
    series_id: Option<i64>,
) -> CreateWorkDbRequest {
    let (normalized_title, normalized_author) = identity_key(title, author_name);
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author_name.to_string(),
        normalized_title,
        normalized_author,
        author_id: Some(author_id),
        series_id,
        language: Some("en".to_string()),
        monitor_ebook: false,
        monitor_audiobook: false,
        ..CreateWorkDbRequest::default()
    }
}

async fn create_work(
    db: &SqliteDb,
    user_id: UserId,
    title: &str,
    author_name: &str,
    author_id: AuthorId,
    series_id: Option<i64>,
) -> Work {
    db.create_work(work_req(user_id, title, author_name, author_id, series_id))
        .await
        .expect("seed work")
        .0
}

struct SeriesSeed<'a> {
    user_id: UserId,
    author_id: AuthorId,
    name: &'a str,
    gr_key: &'a str,
    monitor_ebook: bool,
    monitor_audiobook: bool,
    monitor_language: Option<&'a str>,
}

async fn insert_series(db: &SqliteDb, seed: SeriesSeed<'_>) -> i64 {
    sqlx::query(
        "INSERT INTO series \
         (user_id, author_id, name, gr_key, monitor_ebook, monitor_audiobook, work_count, monitor_language) \
         VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
    )
    .bind(seed.user_id)
    .bind(seed.author_id)
    .bind(seed.name)
    .bind(seed.gr_key)
    .bind(seed.monitor_ebook)
    .bind(seed.monitor_audiobook)
    .bind(seed.monitor_language)
    .execute(db.pool())
    .await
    .expect("insert series")
    .last_insert_rowid()
}

async fn insert_roster(db: &SqliteDb, series_id: i64, marker: &str) {
    sqlx::query("INSERT INTO series_roster (series_id, entries, fetched_at) VALUES (?, ?, ?)")
        .bind(series_id)
        .bind(format!(r#"[{{"marker":"{marker}"}}]"#))
        .bind(Utc::now().to_rfc3339())
        .execute(db.pool())
        .await
        .expect("insert roster");
}

async fn insert_root_and_synced_item(
    db: &SqliteDb,
    user_id: UserId,
    work_id: WorkId,
    old_generation: i64,
) {
    let root_id = sqlx::query(
        "INSERT INTO root_folders (path, media_type) VALUES (?, 'ebook') \
         ON CONFLICT(media_type) DO UPDATE SET path = excluded.path",
    )
    .bind(format!("/tmp/livrarr-author-dedup-{user_id}"))
    .execute(db.pool())
    .await
    .expect("insert root folder")
    .last_insert_rowid();

    sqlx::query(
        "INSERT INTO library_items \
         (user_id, work_id, root_folder_id, path, media_type, file_size, imported_at, tag_status, tagged_at_generation) \
         VALUES (?, ?, ?, ?, 'ebook', 1, ?, 'synced', ?)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(root_id)
    .bind(format!("work-{work_id}.epub"))
    .bind(Utc::now().to_rfc3339())
    .bind(old_generation)
    .execute(db.pool())
    .await
    .expect("insert library item");
}

async fn set_work_flags(db: &SqliteDb, work_id: WorkId, ebook: bool, audiobook: bool) {
    sqlx::query("UPDATE works SET monitor_ebook = ?, monitor_audiobook = ? WHERE id = ?")
        .bind(ebook)
        .bind(audiobook)
        .bind(work_id)
        .execute(db.pool())
        .await
        .expect("set work flags");
}

async fn build_merge_fixture(
    db: &SqliteDb,
    user_id: UserId,
) -> (Author, Author, Work, i64, i64, i64, i64) {
    let survivor = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Survivor Name".to_string(),
            sort_name: Some("Survivor Sort".to_string()),
            ol_key: None,
            gr_key: Some("SURVIVOR_GR".to_string()),
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("seed survivor");
    sqlx::query(
        "INSERT INTO imports (id, user_id, source, status, started_at) VALUES (?, ?, 'readarr', 'completed', ?)",
    )
    .bind("LOSER_IMPORT")
    .bind(user_id)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("seed loser import FK target");
    let loser = db
        .create_author(CreateAuthorDbRequest {
            user_id,
            name: "Loser Name".to_string(),
            sort_name: Some("Loser Sort".to_string()),
            ol_key: Some("LOSER_OL".to_string()),
            gr_key: Some("LOSER_GR".to_string()),
            hc_key: Some("LOSER_HC".to_string()),
            import_id: Some("LOSER_IMPORT".to_string()),
        })
        .await
        .expect("seed loser");

    let early = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let late = Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap();
    sqlx::query(
        "UPDATE authors SET monitored = 1, monitor_new_items = 0, monitor_since = ?, monitor_language = NULL WHERE id = ?",
    )
    .bind(late.to_rfc3339())
    .bind(survivor.id)
    .execute(db.pool())
    .await
    .expect("monitor survivor");
    sqlx::query(
        "UPDATE authors SET monitored = 1, monitor_new_items = 1, monitor_since = ?, monitor_language = NULL WHERE id = ?",
    )
    .bind(early.to_rfc3339())
    .bind(loser.id)
    .execute(db.pool())
    .await
    .expect("monitor loser");

    let fold_changed_survivor = insert_series(
        db,
        SeriesSeed {
            user_id,
            author_id: survivor.id,
            name: "Fold Changed",
            gr_key: "GR-FOLD-CHANGED",
            monitor_ebook: false,
            monitor_audiobook: false,
            monitor_language: None,
        },
    )
    .await;
    let fold_changed_loser = insert_series(
        db,
        SeriesSeed {
            user_id,
            author_id: loser.id,
            name: "Fold Changed",
            gr_key: "GR-FOLD-CHANGED",
            monitor_ebook: true,
            monitor_audiobook: false,
            monitor_language: None,
        },
    )
    .await;
    let fold_unchanged_survivor = insert_series(
        db,
        SeriesSeed {
            user_id,
            author_id: survivor.id,
            name: "Fold Unchanged",
            gr_key: "GR-FOLD-UNCHANGED",
            monitor_ebook: true,
            monitor_audiobook: true,
            monitor_language: Some("de"),
        },
    )
    .await;
    let fold_unchanged_loser = insert_series(
        db,
        SeriesSeed {
            user_id,
            author_id: loser.id,
            name: "Fold Unchanged",
            gr_key: "GR-FOLD-UNCHANGED",
            monitor_ebook: false,
            monitor_audiobook: false,
            monitor_language: None,
        },
    )
    .await;
    let moved_loser = insert_series(
        db,
        SeriesSeed {
            user_id,
            author_id: loser.id,
            name: "Moved Series",
            gr_key: "GR-MOVED",
            monitor_ebook: false,
            monitor_audiobook: true,
            monitor_language: Some("fr"),
        },
    )
    .await;

    insert_roster(db, fold_changed_loser, "loser-only").await;
    insert_roster(db, fold_unchanged_survivor, "survivor-kept").await;
    insert_roster(db, fold_unchanged_loser, "loser-dropped").await;

    let survivor_changed_work = create_work(
        db,
        user_id,
        "Survivor Changed Series Work",
        &survivor.name,
        survivor.id,
        Some(fold_changed_survivor),
    )
    .await;
    let loser_changed_work = create_work(
        db,
        user_id,
        "Loser Changed Series Work",
        &loser.name,
        loser.id,
        Some(fold_changed_loser),
    )
    .await;
    let survivor_unchanged_work = create_work(
        db,
        user_id,
        "Survivor Unchanged Series Work",
        &survivor.name,
        survivor.id,
        Some(fold_unchanged_survivor),
    )
    .await;
    let loser_unchanged_work = create_work(
        db,
        user_id,
        "Loser Unchanged Series Work",
        &loser.name,
        loser.id,
        Some(fold_unchanged_loser),
    )
    .await;
    let moved_work = create_work(
        db,
        user_id,
        "Moved Series Work",
        &loser.name,
        loser.id,
        Some(moved_loser),
    )
    .await;
    let unseries_work =
        create_work(db, user_id, "Unseries Work", &loser.name, loser.id, None).await;

    set_work_flags(db, survivor_changed_work.id, false, false).await;
    set_work_flags(db, loser_changed_work.id, false, false).await;
    set_work_flags(db, survivor_unchanged_work.id, false, false).await;
    set_work_flags(db, loser_unchanged_work.id, false, false).await;
    set_work_flags(db, moved_work.id, false, false).await;

    sqlx::query("UPDATE works SET merge_generation = 7 WHERE id = ?")
        .bind(unseries_work.id)
        .execute(db.pool())
        .await
        .expect("set old generation");
    insert_root_and_synced_item(db, user_id, unseries_work.id, 7).await;

    sqlx::query(
        "INSERT INTO author_series_cache (author_id, entries, fetched_at) VALUES (?, '[]', ?)",
    )
    .bind(loser.id)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("seed loser series cache");
    sqlx::query(
        "INSERT INTO author_bibliography (author_id, entries, fetched_at) VALUES (?, '[]', ?)",
    )
    .bind(loser.id)
    .bind(Utc::now().to_rfc3339())
    .execute(db.pool())
    .await
    .expect("seed loser bibliography");

    (
        survivor,
        loser,
        unseries_work,
        fold_changed_survivor,
        fold_unchanged_survivor,
        moved_loser,
        survivor_unchanged_work.id,
    )
}

#[tokio::test]
async fn merge_authors_repoints_works_folds_and_moves_series_preserves_state_and_reports_counts() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (
        survivor,
        loser,
        generation_work,
        fold_changed_survivor,
        fold_unchanged_survivor,
        moved_loser,
        survivor_unchanged_work_id,
    ) = build_merge_fixture(&db, user_id).await;
    let generation_before: (String, i64) =
        sqlx::query_as("SELECT normalized_author, merge_generation FROM works WHERE id = ?")
            .bind(generation_work.id)
            .fetch_one(db.pool())
            .await
            .expect("generation state before merge");

    let report = db
        .merge_authors(user_id, survivor.id, loser.id)
        .await
        .expect("merge authors");

    assert_eq!(report.works_moved, 4);
    assert_eq!(report.series_moved, 1);
    assert_eq!(report.series_folded, 2);

    let moved = db
        .get_work(user_id, generation_work.id)
        .await
        .expect("generation work");
    let generation_after: (String, i64) =
        sqlx::query_as("SELECT normalized_author, merge_generation FROM works WHERE id = ?")
            .bind(generation_work.id)
            .fetch_one(db.pool())
            .await
            .expect("generation state after merge");
    assert_eq!(moved.author_id, Some(survivor.id));
    assert_eq!(moved.author_name, survivor.name);
    assert_eq!(generation_after.0, generation_before.0);
    assert_eq!(generation_after.1, generation_before.1 + 1);

    let tagged_generation: i64 =
        sqlx::query_scalar("SELECT tagged_at_generation FROM library_items WHERE work_id = ?")
            .bind(generation_work.id)
            .fetch_one(db.pool())
            .await
            .expect("tagged generation");
    assert!(
        tagged_generation < generation_after.1,
        "old synced library item must become eligible for tag convergence"
    );

    let changed_series = sqlx::query("SELECT author_id, monitor_ebook, monitor_audiobook, monitor_language FROM series WHERE id = ?")
        .bind(fold_changed_survivor)
        .fetch_one(db.pool())
        .await
        .expect("changed survivor series");
    assert_eq!(changed_series.get::<i64, _>("author_id"), survivor.id);
    assert!(changed_series.get::<bool, _>("monitor_ebook"));
    assert!(!changed_series.get::<bool, _>("monitor_audiobook"));
    assert_eq!(changed_series.get::<String, _>("monitor_language"), "en");

    let changed_work_flags: Vec<(bool, bool)> = sqlx::query_as(
        "SELECT monitor_ebook, monitor_audiobook FROM works WHERE series_id = ? ORDER BY title",
    )
    .bind(fold_changed_survivor)
    .fetch_all(db.pool())
    .await
    .expect("changed linked work flags");
    assert_eq!(changed_work_flags, vec![(true, false), (true, false)]);

    let loser_roster_series: i64 =
        sqlx::query_scalar("SELECT series_id FROM series_roster WHERE entries LIKE '%loser-only%'")
            .fetch_one(db.pool())
            .await
            .expect("loser-only roster");
    assert_eq!(loser_roster_series, fold_changed_survivor);

    let unchanged_repointed: Vec<(i64, bool, bool)> = sqlx::query_as(
        "SELECT id, monitor_ebook, monitor_audiobook FROM works WHERE series_id = ? ORDER BY title",
    )
    .bind(fold_unchanged_survivor)
    .fetch_all(db.pool())
    .await
    .expect("unchanged linked work flags");
    let survivor_preexisting = unchanged_repointed
        .iter()
        .find(|(id, _, _)| *id == survivor_unchanged_work_id)
        .expect("pre-existing survivor work");
    assert_eq!(
        *survivor_preexisting,
        (survivor_unchanged_work_id, false, false)
    );
    assert!(
        unchanged_repointed
            .iter()
            .any(|(id, ebook, audiobook)| *id != survivor_unchanged_work_id
                && *ebook
                && *audiobook),
        "only the repointed loser work should be stamped when folded flags did not change"
    );
    let survivor_roster_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM series_roster WHERE entries LIKE '%survivor-kept%'",
    )
    .fetch_one(db.pool())
    .await
    .expect("survivor roster count");
    let loser_roster_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM series_roster WHERE entries LIKE '%loser-dropped%'",
    )
    .fetch_one(db.pool())
    .await
    .expect("loser roster count");
    assert_eq!(survivor_roster_count, 1);
    assert_eq!(loser_roster_count, 0);

    let moved_series_author: i64 = sqlx::query_scalar("SELECT author_id FROM series WHERE id = ?")
        .bind(moved_loser)
        .fetch_one(db.pool())
        .await
        .expect("moved series author");
    assert_eq!(moved_series_author, survivor.id);

    let merged_author = db
        .get_author(user_id, survivor.id)
        .await
        .expect("merged survivor author");
    assert!(merged_author.monitored);
    assert!(merged_author.monitor_new_items);
    assert_eq!(
        merged_author.monitor_since,
        Some(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap())
    );
    assert_eq!(merged_author.monitor_language.as_deref(), Some("en"));
    assert_eq!(merged_author.sort_name.as_deref(), Some("Survivor Sort"));
    assert_eq!(merged_author.ol_key.as_deref(), Some("LOSER_OL"));
    assert_eq!(merged_author.gr_key.as_deref(), Some("SURVIVOR_GR"));
    assert_eq!(merged_author.hc_key.as_deref(), Some("LOSER_HC"));
    assert_eq!(merged_author.import_id.as_deref(), Some("LOSER_IMPORT"));

    assert!(matches!(
        db.get_author(user_id, loser.id).await,
        Err(DbError::NotFound { .. })
    ));
    let loser_cache_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_series_cache WHERE author_id = ?")
            .bind(loser.id)
            .fetch_one(db.pool())
            .await
            .expect("loser cache count");
    let loser_biblio_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM author_bibliography WHERE author_id = ?")
            .bind(loser.id)
            .fetch_one(db.pool())
            .await
            .expect("loser bibliography count");
    assert_eq!(loser_cache_count, 0);
    assert_eq!(loser_biblio_count, 0);
}

#[tokio::test]
async fn merge_authors_loser_only_monitoring_carries_since_and_language() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let survivor = create_author(&db, user_id, "Survivor", None).await;
    let loser = create_author(&db, user_id, "Loser", None).await;
    let since = Utc.with_ymd_and_hms(2021, 5, 6, 0, 0, 0).unwrap();
    sqlx::query(
        "UPDATE authors SET monitored = 1, monitor_new_items = 1, monitor_since = ?, monitor_language = 'de' WHERE id = ?",
    )
    .bind(since.to_rfc3339())
    .bind(loser.id)
    .execute(db.pool())
    .await
    .expect("mark loser monitored");

    db.merge_authors(user_id, survivor.id, loser.id)
        .await
        .expect("merge loser-only monitored");

    let merged = db
        .get_author(user_id, survivor.id)
        .await
        .expect("merged author");
    assert!(merged.monitored);
    assert!(merged.monitor_new_items);
    assert_eq!(merged.monitor_since, Some(since));
    assert_eq!(merged.monitor_language.as_deref(), Some("de"));
}

#[tokio::test]
async fn merge_authors_error_arms_leave_first_merge_state_intact() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let other_user_id = create_second_test_user(&db).await;
    let survivor = create_author(&db, user_id, "Survivor", None).await;
    let loser = create_author(&db, user_id, "Loser", None).await;
    let other = create_author(&db, other_user_id, "Other User", None).await;

    assert!(db
        .merge_authors(user_id, survivor.id, survivor.id)
        .await
        .is_err());
    assert!(db
        .merge_authors(user_id, survivor.id, other.id)
        .await
        .is_err());
    assert!(db
        .merge_authors(user_id, survivor.id, 9_999_999)
        .await
        .is_err());

    let report = db
        .merge_authors(user_id, survivor.id, loser.id)
        .await
        .expect("first merge succeeds");
    assert_eq!(report.works_moved, 0);
    assert!(db
        .merge_authors(user_id, survivor.id, loser.id)
        .await
        .is_err());
    assert!(db.get_author(user_id, survivor.id).await.is_ok());
    assert!(matches!(
        db.get_author(user_id, loser.id).await,
        Err(DbError::NotFound { .. })
    ));
}

#[tokio::test]
async fn author_service_merge_delegates_to_db_merge() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let survivor = create_author(&db, user_id, "Survivor", None).await;
    let loser = create_author(&db, user_id, "Loser", None).await;
    let svc = author_service(db.clone());

    let report = svc
        .merge(user_id, survivor.id, loser.id)
        .await
        .expect("service merge");

    assert_eq!(report.works_moved, 0);
    assert!(matches!(
        svc.get(user_id, loser.id).await,
        Err(AuthorServiceError::NotFound)
    ));
}

#[derive(Clone)]
struct RouteState {
    author_service: Arc<TestAuthorService>,
}

impl HasAuthorService for RouteState {
    type AuthorSvc = TestAuthorService;

    fn author_service(&self) -> &Self::AuthorSvc {
        &self.author_service
    }
}

fn auth_context(user_id: UserId) -> AuthContext {
    let now = Utc::now();
    AuthContext {
        user: User {
            id: user_id,
            username: "route-user".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api".to_string(),
            setup_pending: false,
            created_at: now,
            updated_at: now,
        },
        auth_type: AuthType::Session,
        session_token_hash: None,
    }
}

#[tokio::test]
async fn post_author_merge_route_returns_report_json() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let survivor = create_author(&db, user_id, "Survivor", None).await;
    let loser = create_author(&db, user_id, "Loser", None).await;
    let state = RouteState {
        author_service: Arc::new(author_service(db)),
    };
    let app = Router::new()
        .route(
            "/author/{id}/merge",
            post(livrarr_handlers::author::merge::<RouteState>),
        )
        .with_state(state);

    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/author/{}/merge", survivor.id))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "loser_id": loser.id }).to_string()))
        .expect("build request");
    request.extensions_mut().insert(auth_context(user_id));

    let response = app.oneshot(request).await.expect("route response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("report json");
    assert_eq!(json["worksMoved"], 0);
    assert_eq!(json["seriesMoved"], 0);
    assert_eq!(json["seriesFolded"], 0);
}

#[tokio::test]
async fn work_service_door_readarr_shaped_variants_converge_to_one_livrarr_author() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let svc = work_service(db.clone());

    let first = svc
        .add(
            user_id,
            pending_readarr_candidate("Readarr Book One", "JK Rowling", None),
        )
        .await
        .expect("first Readarr-shaped add");
    let second = svc
        .add(
            user_id,
            pending_readarr_candidate("Readarr Book Two", "J.K. Rowling", None),
        )
        .await
        .expect("second Readarr-shaped add");

    assert_eq!(first.author_id, second.author_id);
    assert_eq!(author_count(&db, user_id).await, 1);
}

#[tokio::test]
async fn work_service_door_readarr_shaped_later_variant_adopts_preexisting_author() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let existing = create_author(&db, user_id, "J.K. Rowling", None).await;
    let svc = work_service(db.clone());

    let first = svc
        .add(
            user_id,
            pending_readarr_candidate("Readarr Existing One", "JK Rowling", None),
        )
        .await
        .expect("first Readarr-shaped add adopts existing");
    let second = svc
        .add(
            user_id,
            pending_readarr_candidate("Readarr Existing Two", "Joanne K. Rowling", None),
        )
        .await
        .expect("later Readarr-shaped add adopts same existing");

    assert_eq!(first.author_id, Some(existing.id));
    assert_eq!(second.author_id, Some(existing.id));
    assert_eq!(author_count(&db, user_id).await, 1);
}
