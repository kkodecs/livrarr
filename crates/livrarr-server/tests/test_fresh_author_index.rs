//! Fresh-author setup contract at 98d35a37 (spec v2 / PM decisions).
//!
//! Registered on the server crate to use Cargo's actual `livrarr` binary. Fresh
//! cases never seed schema/authors or call create_test_db. Historical cases run
//! every real migration through 088 before constructing explicitly old data.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use livrarr_db::pool::{check_version_gate, create_sqlite_pool, run_migrations};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{AuthorDb, AuthorLinkDb, CreateAuthorDbRequest, CreateAuthorGateRequest};
use livrarr_domain::author_link::{AuthorLinkTrigger, AuthorNameSource};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha384};
use sqlx::{Row, SqlitePool};
use tempfile::TempDir;

const REPAIR_VERSION: i64 = 89;
const MIGRATION_CHECKSUMS: &str = include_str!("fixtures/fresh_author_migrations_088.sha384");
const PASSWORD: &str = "fresh-author-disposable-password";

// All fixed provider clients used by title/author-only Add honor reqwest's
// proxy environment. This local endpoint refuses both HTTP and CONNECT; it
// never forwards a request. There are no cover URLs, provider keys or configured
// infrastructure in these cases. The test's API client explicitly bypasses it.
// This is a bounded provider-unavailability fixture, not a general network jail.
struct Server {
    child: Child,
    base: String,
    log: PathBuf,
    proxy: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start(data: &Path, log: PathBuf) -> Self {
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local unavailable-provider fixture");
        let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
        let proxy = tokio::spawn(async move {
            axum::serve(
                proxy_listener,
                axum::Router::new().fallback(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            )
            .await
            .expect("serve local unavailable-provider fixture");
        });

        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        // Only transport, logging and recurring background scheduling are set.
        // Database/account setup remains the production executable's job.
        std::fs::write(
            data.join("config.toml"),
            format!(
                "[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
                 [log]\nlevel = 'debug'\n\
                 [convergence]\nenabled = false\n\
                 [author_link]\nenabled = false\n"
            ),
        )
        .unwrap();
        let stdout = std::fs::File::create(&log).unwrap();
        let stderr = stdout.try_clone().unwrap();
        drop(reservation);
        let child = Command::new(env!("CARGO_BIN_EXE_livrarr"))
            .arg("--data")
            .arg(data)
            .env_clear()
            .env("HTTP_PROXY", &proxy_url)
            .env("HTTPS_PROXY", &proxy_url)
            .env("ALL_PROXY", &proxy_url)
            .env("NO_PROXY", "")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("start Cargo-built livrarr binary");
        Self {
            child,
            base: format!("http://127.0.0.1:{port}/api/v1"),
            log,
            proxy,
        }
    }

    fn logs(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    async fn ready(&mut self, client: &Client) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!(
                    "server exited before HTTP readiness ({status}):\n{}",
                    self.logs()
                );
            }
            if let Ok(response) = client.get(format!("{}/health", self.base)).send().await {
                if response.status().is_success() {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "server readiness deadline:\n{}",
                self.logs()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn observe_refusal(&mut self, client: &Client) -> (Option<ExitStatus>, bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (Some(status), false);
            }
            // Any HTTP response means it served requests, including an error.
            if client
                .get(format!("{}/health", self.base))
                .send()
                .await
                .is_ok()
            {
                return (None, true);
            }
            if Instant::now() >= deadline {
                return (None, false);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn stop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().expect("stop private test server");
        }
        self.child.wait().expect("reap private test server");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.proxy.abort();
    }
}

fn client() -> Client {
    Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

async fn login(server: &Server, client: &Client) -> String {
    let response = client
        .post(format!("{}/auth/login", server.base))
        .json(&json!({"username": "fresh-author", "password": PASSWORD, "rememberMe": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "real login failed:\n{}",
        server.logs()
    );
    response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .expect("login session token")
        .to_owned()
}

async fn setup_account(server: &Server, client: &Client) -> String {
    let setup = client
        .post(format!("{}/setup", server.base))
        .json(&json!({"username": "fresh-author", "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        setup.status(),
        StatusCode::OK,
        "real account setup failed:\n{}",
        server.logs()
    );
    login(server, client).await
}

async fn add_work(server: &Server, client: &Client, token: &str, title: &str, author: &str) -> i64 {
    let response = client
        .post(format!("{}/work", server.base))
        .bearer_auth(token)
        .json(&json!({"title": title, "authorName": author}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "fresh authenticated Add must persist a new author and Work; body={body}\n{}",
        server.logs()
    );
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        body["created"], true,
        "Add must create the requested Work: {body}"
    );
    assert_eq!(
        body["authorCreated"], true,
        "Add must create the new author: {body}"
    );
    body["work"]["id"].as_i64().expect("persisted Work id")
}

async fn assert_author_index(pool: &SqlitePool) {
    let indexes = sqlx::query("PRAGMA index_list('authors')")
        .fetch_all(pool)
        .await
        .unwrap();
    let index = indexes
        .iter()
        .find(|row| row.get::<String, _>("name") == "idx_authors_identity")
        .expect("normal setup must create idx_authors_identity before Add");
    assert_eq!(index.get::<i64, _>("unique"), 1);
    assert_eq!(index.get::<i64, _>("partial"), 1);
    let columns: Vec<String> = sqlx::query("PRAGMA index_info('idx_authors_identity')")
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get("name"))
        .collect();
    assert_eq!(columns, ["user_id", "normalized_name"]);
    let sql: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name = 'idx_authors_identity'")
            .fetch_one(pool)
            .await
            .unwrap();
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    assert_eq!(
        normalized
            .split_once(" where ")
            .map(|(_, predicate)| predicate.trim_end_matches(';')),
        Some("normalized_name is not null"),
        "NULL keys must remain exempt: {sql}"
    );
}

async fn assert_repair_recorded(pool: &SqlitePool) {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ? AND success = 1",
    )
    .bind(REPAIR_VERSION)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        count, 1,
        "normal migration authority must record repair 089 exactly once"
    );
}

async fn persisted_work(pool: &SqlitePool, work_id: i64) -> (i64, String, String, Option<String>) {
    sqlx::query_as(
        "SELECT a.id, a.name, w.title, a.normalized_name FROM works w \
         JOIN authors a ON a.id = w.author_id AND a.user_id = w.user_id \
         WHERE w.id = ? AND w.user_id = 1",
    )
    .bind(work_id)
    .fetch_one(pool)
    .await
    .expect("durable Work linked to its user's author")
}

#[tokio::test]
async fn fresh_server_establishes_partial_author_index_before_add() {
    let data = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let client = client();
    assert!(!data.path().join("livrarr.db").exists());
    let mut server = Server::start(data.path(), logs.path().join("fresh-schema.log")).await;
    server.ready(&client).await;
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    assert_author_index(&pool).await;
    assert_repair_recorded(&pool).await;
    let token = setup_account(&server, &client).await;
    let work_id = add_work(&server, &client, &token, "Dune", "Frank Herbert").await;
    server.stop();
    let stored = persisted_work(&pool, work_id).await;
    assert_eq!(stored.1, "Frank Herbert");
    assert_eq!(stored.2, "Dune");
    pool.close().await;
}

#[tokio::test]
async fn fresh_server_account_login_add_and_restart_persist_authors_and_works() {
    let data = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let client = client();
    assert!(!data.path().join("livrarr.db").exists());
    let mut server = Server::start(data.path(), logs.path().join("first.log")).await;
    server.ready(&client).await;
    let token = setup_account(&server, &client).await;
    let unauthorized = client
        .post(format!("{}/work", server.base))
        .json(&json!({"title": "Dune", "authorName": "Frank Herbert"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    // The sibling case pins the index before any Add. Keep this real HTTP
    // regression independent so the missing index also produces the actual 500.
    let first_id = add_work(&server, &client, &token, "Dune", "Frank Herbert").await;
    server.stop();
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    let first = persisted_work(&pool, first_id).await;
    assert_eq!(
        (&first.1, &first.2),
        (&"Frank Herbert".to_string(), &"Dune".to_string())
    );
    assert!(first.3.is_some());
    pool.close().await;

    // Backup filenames have one-second resolution. Avoid the independent,
    // documented same-second restart collision without changing production.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut restarted = Server::start(data.path(), logs.path().join("restart.log")).await;
    restarted.ready(&client).await;
    let token = login(&restarted, &client).await;
    let second_id = add_work(
        &restarted,
        &client,
        &token,
        "A Wizard of Earthsea",
        "Ursula K. Le Guin",
    )
    .await;
    restarted.stop();
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    assert_eq!(
        persisted_work(&pool, first_id).await,
        first,
        "restart preserves the original author/Work"
    );
    let second = persisted_work(&pool, second_id).await;
    assert_eq!(second.1, "Ursula K. Le Guin");
    assert_eq!(second.2, "A Wizard of Earthsea");
    assert_ne!(first.0, second.0);
    let db = SqliteDb::new(pool.clone());
    let (adopted, created) = db
        .create_or_adopt_author(author_request(1, "FRANK HERBERT"))
        .await
        .expect("the real writer remains available after restart");
    assert!(!created);
    assert_eq!(adopted.id, first.0);
    let same_key_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authors WHERE user_id = 1 AND normalized_name = ?",
    )
    .bind(&first.3)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(same_key_rows, 1);
    assert_author_index(&pool).await;
    assert_repair_recorded(&pool).await;
    check_version_gate(&pool).await.unwrap();
    pool.close().await;
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../livrarr-db/migrations")
}

async fn historical_database(data: &Path) -> SqlitePool {
    let historical = TempDir::new().unwrap();
    // SQLx runs the complete real historical sequence, including checksum
    // bookkeeping. Do not model the old schema by hand or drop a current index.
    for line in MIGRATION_CHECKSUMS.lines() {
        let (_, name) = line.split_once("  ").unwrap();
        std::fs::copy(migrations_dir().join(name), historical.path().join(name)).unwrap();
    }
    let pool = create_sqlite_pool(data).await.unwrap();
    sqlx::migrate::Migrator::new(historical.path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    let last: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(last, 88);
    let index: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_authors_identity'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        index, 0,
        "the historical migrations alone never supplied this index"
    );
    pool
}

async fn seed_legacy_records(pool: &SqlitePool, duplicate: bool) {
    // These are historical rows, not claims about current writer behavior:
    // pre-077 authors can retain NULL keys; a corrupt/imported old DB can have
    // duplicate non-NULL keys. Today's writer requires the missing index and
    // cannot construct either unindexed fixture through its author insert gate.
    sqlx::query("INSERT INTO users (id, username, password_hash, role, api_key_hash, created_at, updated_at) \
        VALUES (2, 'other-user', 'unused', 'user', 'other-unused', '2026-01-01', '2026-01-01')")
        .execute(pool).await.unwrap();
    for (id, user, name, key) in [
        (10, 1, "Legacy Writer", Some("legacy writer")),
        (11, 2, "Legacy Writer", Some("legacy writer")),
        (12, 1, "Unkeyed Writer", None),
        (13, 1, "Unkeyed Writer", None),
    ] {
        sqlx::query("INSERT INTO authors (id, user_id, name, sort_name, normalized_name, added_at) VALUES (?, ?, ?, ?, ?, '2026-01-01T00:00:00Z')")
            .bind(id).bind(user).bind(name).bind(format!("Sort {id}")).bind(key).execute(pool).await.unwrap();
    }
    if duplicate {
        sqlx::query(
            "INSERT INTO authors (id, user_id, name, normalized_name, added_at) \
            VALUES (14, 1, 'LEGACY WRITER', 'legacy writer', '2026-01-02T00:00:00Z')",
        )
        .execute(pool)
        .await
        .unwrap();
    }
    // Each keyed author (including both conflicting rows) owns relationships.
    // A merge/delete/rename/key rewrite would change these selected observables.
    for id in if duplicate { vec![10, 14] } else { vec![10] } {
        sqlx::query(
            "INSERT INTO series (id, user_id, author_id, name, gr_key) VALUES (?, 1, ?, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(format!("Legacy series {id}"))
        .bind(format!("{id}"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO works (id, user_id, title, author_name, author_id, series_id, added_at) \
            VALUES (?, 1, ?, 'Legacy Writer', ?, ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("Legacy work {id}"))
        .bind(id)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO author_provider_routes (id, user_id, author_id, provider, route_value, state, provenance, evidence_work_id, created_at) \
            VALUES (?, 1, ?, 'goodreads', ?, 'active', 'user_picked', ?, '2026-01-01T00:00:00Z')")
            .bind(id).bind(id).bind(format!("{id}")).bind(id).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO author_name_variants (user_id, author_id, name, canonical_name, source, observed_at) \
        SELECT user_id, id, name, COALESCE(normalized_name, 'unkeyed writer'), 'legacy', added_at FROM authors")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO author_link_progress (author_id, user_id, state, next_attempt_at, trigger, updated_at) \
        SELECT id, user_id, 'parked_no_settled_work', '2099-01-01T00:00:00Z', 'legacy_backfill', added_at FROM authors")
        .execute(pool).await.unwrap();
}

async fn legacy_records(
    pool: &SqlitePool,
    last_author_id: i64,
) -> BTreeMap<&'static str, Vec<String>> {
    let mut records = BTreeMap::new();
    for (table, query) in [
        ("authors", "SELECT json_array(id, user_id, name, sort_name, normalized_name, ol_key, gr_key, hc_key, added_at) FROM authors WHERE id BETWEEN 10 AND ? ORDER BY id"),
        ("works", "SELECT json_array(id, user_id, title, author_name, author_id, series_id) FROM works WHERE id BETWEEN 10 AND ? ORDER BY id"),
        ("series", "SELECT json_array(id, user_id, author_id, name, gr_key) FROM series WHERE author_id BETWEEN 10 AND ? ORDER BY id"),
        ("routes", "SELECT json_array(id, user_id, author_id, provider, route_value, state, provenance, evidence_work_id) FROM author_provider_routes WHERE author_id BETWEEN 10 AND ? ORDER BY id"),
        ("variants", "SELECT json_array(id, user_id, author_id, name, canonical_name, source) FROM author_name_variants WHERE author_id BETWEEN 10 AND ? ORDER BY id"),
    ] {
        records.insert(table, sqlx::query_scalar(query).bind(last_author_id).fetch_all(pool).await.unwrap());
    }
    records
}

async fn migration_records(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT json_array(version, description, installed_on, success, hex(checksum), execution_time) \
        FROM _sqlx_migrations WHERE version <= 88 ORDER BY version")
        .fetch_all(pool).await.unwrap()
}

fn author_request(user_id: i64, name: &str) -> CreateAuthorGateRequest {
    CreateAuthorGateRequest {
        user_id,
        name: name.to_owned(),
        sort_name: None,
        import_id: None,
        initial_name_source: AuthorNameSource::User,
        trigger: AuthorLinkTrigger::AuthorCreated,
    }
}

async fn assert_live_writer_semantics(pool: &SqlitePool) {
    let db = SqliteDb::new(pool.clone());
    let (first, created) = db
        .create_or_adopt_author(author_request(1, "Octavia Butler"))
        .await
        .expect("production create/adopt writer must accept a new author after setup");
    assert!(created);
    let (adopted, created) = db
        .create_or_adopt_author(author_request(1, "OCTAVIA BUTLER"))
        .await
        .expect("same-user canonical key must converge through the actual writer");
    assert!(!created);
    assert_eq!(adopted.id, first.id);
    let key = livrarr_domain::identity_matching::canonical_author_key("Octavia Butler");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authors WHERE user_id = 1 AND normalized_name = ?",
    )
    .bind(&key)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 1);

    let (other_user, created) = db
        .create_or_adopt_author(author_request(2, "Octavia Butler"))
        .await
        .unwrap();
    assert!(created);
    assert_ne!(first.id, other_user.id, "uniqueness is scoped to the user");
    assert_eq!(other_user.user_id, 2);
    let (null_one, created) = db
        .create_or_adopt_author(author_request(1, "!!!"))
        .await
        .unwrap();
    assert!(created);
    let (null_two, created) = db
        .create_or_adopt_author(author_request(1, "!!!"))
        .await
        .unwrap();
    assert!(created);
    assert_ne!(null_one.id, null_two.id, "NULL keys remain exempt");
    let nulls: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authors WHERE id IN (?, ?) AND normalized_name IS NULL",
    )
    .bind(null_one.id)
    .bind(null_two.id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(nulls, 2);

    let (_, created) = db
        .create_author(CreateAuthorDbRequest {
            user_id: 1,
            name: "Legacy Writer Interface".to_owned(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
        })
        .await
        .expect("the older production author writer uses the same conflict target");
    assert!(created);
}

async fn upgrade_existing(already_indexed: bool) {
    let data = TempDir::new().unwrap();
    let pool = historical_database(data.path()).await;
    seed_legacy_records(&pool, false).await;
    if already_indexed {
        // Only this existing-index fixture installs the historical index. Fresh
        // and clean-unindexed paths never receive a test-created index.
        sqlx::query("CREATE UNIQUE INDEX idx_authors_identity ON authors(user_id, normalized_name) WHERE normalized_name IS NOT NULL")
            .execute(&pool).await.unwrap();
    }
    let before = legacy_records(&pool, 13).await;
    let migrations_before = migration_records(&pool).await;
    run_migrations(&pool)
        .await
        .expect("clean historical database must upgrade");
    assert_eq!(
        legacy_records(&pool, 13).await,
        before,
        "upgrade must preserve old authors and relationships"
    );
    assert_eq!(migration_records(&pool).await, migrations_before);
    check_version_gate(&pool).await.unwrap();
    assert_live_writer_semantics(&pool).await;
    assert_author_index(&pool).await;
    run_migrations(&pool)
        .await
        .expect("repeat setup must be idempotent");
    assert_eq!(legacy_records(&pool, 13).await, before);
    assert_eq!(migration_records(&pool).await, migrations_before);
    check_version_gate(&pool).await.unwrap();
    let db = SqliteDb::new(pool.clone());
    let (_, created) = db
        .create_or_adopt_author(author_request(1, "After Repeat Setup"))
        .await
        .unwrap();
    assert!(created, "writer remains usable after repeat setup");
    assert_repair_recorded(&pool).await;
    pool.close().await;
}

#[tokio::test]
async fn clean_unindexed_088_upgrade_preserves_legacy_data_and_writer_semantics() {
    upgrade_existing(false).await;
}

#[tokio::test]
async fn already_indexed_088_upgrade_preserves_legacy_data_and_writer_semantics() {
    upgrade_existing(true).await;
}

#[tokio::test]
async fn duplicate_same_user_keys_refuse_migration_without_mutation() {
    let data = TempDir::new().unwrap();
    let pool = historical_database(data.path()).await;
    seed_legacy_records(&pool, true).await;
    let before = legacy_records(&pool, 14).await;
    let migrations_before = migration_records(&pool).await;
    for _ in 0..2 {
        let result = run_migrations(&pool).await;
        assert_eq!(legacy_records(&pool, 14).await, before);
        assert_eq!(migration_records(&pool).await, migrations_before);
        let later_migrations: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version > 88")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            later_migrations, 0,
            "failed setup must leave no repair/dirty migration record"
        );
        let index: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_authors_identity'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(index, 0, "failed unique-index creation must roll back");
        let error = result
            .expect_err("production migration setup must refuse duplicate same-user author keys");
        match error {
            sqlx::migrate::MigrateError::ExecuteMigration(error, REPAIR_VERSION) => {
                assert!(
                    error
                        .as_database_error()
                        .is_some_and(|error| error.is_unique_violation()),
                    "repair refusal must be a uniqueness violation: {error}"
                );
            }
            error => panic!(
                "expected migration 089 uniqueness refusal, not another setup failure: {error}"
            ),
        }
    }
    pool.close().await;
}

#[tokio::test]
async fn duplicate_same_user_keys_refuse_real_startup_without_mutation() {
    let data = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let pool = historical_database(data.path()).await;
    // Establish normal identity readiness while empty, before adding the old
    // author records, so the unrelated inactive-library cutover gate cannot be
    // mistaken for the required unique-index setup refusal.
    SqliteDb::new(pool.clone())
        .ensure_identity_authority_ready()
        .await
        .unwrap();
    seed_legacy_records(&pool, true).await;
    let before = legacy_records(&pool, 14).await;
    let migrations_before = migration_records(&pool).await;
    pool.close().await;

    for attempt in 0..2 {
        let mut server = Server::start(
            data.path(),
            logs.path().join(format!("duplicate-{attempt}.log")),
        )
        .await;
        let (status, served) = server.observe_refusal(&client()).await;
        server.stop();
        let log = server.logs();
        let pool = create_sqlite_pool(data.path()).await.unwrap();
        assert_eq!(
            legacy_records(&pool, 14).await,
            before,
            "failed setup must not merge, rename, delete, rewrite keys or move relationships"
        );
        assert_eq!(
            migration_records(&pool).await,
            migrations_before,
            "prior successful migration state must survive refusal"
        );
        let repair_success: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ? AND success = 1",
        )
        .bind(REPAIR_VERSION)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            repair_success, 0,
            "failed repair must never be recorded successful"
        );
        let index: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_authors_identity'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(index, 0, "failed unique-index creation must roll back");
        pool.close().await;
        assert!(!served && status.is_some_and(|s| !s.success()),
            "duplicate author keys must make the real server exit unsuccessfully without serving; status={status:?}, served={served}\n{log}");
        let lower = log.to_lowercase();
        assert!(
            !lower.contains("listening on"),
            "refused setup must never announce HTTP readiness:\n{log}"
        );
        assert!(lower.contains("migration") && lower.contains("89") && lower.contains("unique") && lower.contains("authors"),
            "startup refusal must explain migration 089's author uniqueness failure, not an unrelated harness/gate error:\n{log}");
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
}

#[tokio::test]
async fn additive_setup_preserves_schema_83_data_1_compatibility_guards() {
    let data = TempDir::new().unwrap();
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    run_migrations(&pool).await.unwrap();
    for (key, supported, newer) in [("schema_version", "83", "84"), ("data_version", "1", "2")] {
        let value: String = sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = ?")
            .bind(key)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            value, supported,
            "migration number must not change the {key} contract"
        );
        check_version_gate(&pool)
            .await
            .expect("supported versions must remain accepted");
        sqlx::query("UPDATE _livrarr_meta SET value = ? WHERE key = ?")
            .bind(newer)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
        let error = check_version_gate(&pool)
            .await
            .expect_err("newer incompatible version must remain rejected");
        assert!(
            error.contains(key) && error.contains("newer"),
            "useful compatibility refusal: {error}"
        );
        sqlx::query("UPDATE _livrarr_meta SET value = ? WHERE key = ?")
            .bind(supported)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    check_version_gate(&pool).await.unwrap();
    pool.close().await;
}

#[test]
fn migrations_through_088_keep_their_shipped_checksums() {
    let mut actual = Vec::new();
    for entry in std::fs::read_dir(migrations_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            let name = path.file_name().unwrap().to_str().unwrap();
            let version: i64 = name.split('_').next().unwrap().parse().unwrap();
            if version <= 88 {
                let digest = Sha384::digest(std::fs::read(&path).unwrap());
                actual.push(format!("{digest:x}  {name}"));
            }
        }
    }
    actual.sort_by(|left, right| {
        left.split_once("  ")
            .unwrap()
            .1
            .cmp(right.split_once("  ").unwrap().1)
    });
    assert_eq!(
        actual.join("\n") + "\n",
        MIGRATION_CHECKSUMS,
        "shipped SQL migration files through 088 must stay byte-identical to 98d35a37"
    );
}
