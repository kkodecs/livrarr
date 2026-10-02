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
    data: PathBuf,
    log: PathBuf,
    stderr_log: PathBuf,
    proxy: tokio::task::JoinHandle<()>,
}

/// The harness's standard `config.toml`: transport, logging and recurring
/// background scheduling only.
fn standard_config(port: u16) -> String {
    format!(
        "[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
         [log]\nlevel = 'debug'\n\
         [convergence]\nenabled = false\n\
         [author_link]\nenabled = false\n"
    )
}

impl Server {
    async fn start(data: &Path, log: PathBuf) -> Self {
        Self::start_with(data, log, standard_config).await
    }

    /// Starts the binary with a `config.toml` built from the reserved port.
    async fn start_with(data: &Path, log: PathBuf, config: impl FnOnce(u16) -> String) -> Self {
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
        // Database/account setup remains the production executable's job.
        std::fs::write(data.join("config.toml"), config(port)).unwrap();
        let stderr_log = log.with_extension("stderr");
        let stdout = std::fs::File::create(&log).unwrap();
        let stderr = std::fs::File::create(&stderr_log).unwrap();
        drop(reservation);
        let mut command = Command::new(env!("CARGO_BIN_EXE_livrarr"));
        command
            .arg("--data")
            .arg(data)
            .env_clear()
            .env("HTTP_PROXY", &proxy_url)
            .env("HTTPS_PROXY", &proxy_url)
            .env("ALL_PROXY", &proxy_url)
            .env("NO_PROXY", "");
        // A coverage run's profile destination is the only inherited setting.
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let child = command
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("start Cargo-built livrarr binary");
        Self {
            child,
            base: format!("http://127.0.0.1:{port}/api/v1"),
            data: data.to_path_buf(),
            log,
            stderr_log,
            proxy,
        }
    }

    fn stdout(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr_log).unwrap_or_default()
    }

    /// Everything the process printed: stdout, then stderr.
    fn logs(&self) -> String {
        format!("{}{}", self.stdout(), self.stderr())
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

/// The contents of `{data}/setup-token` when that path is a readable file.
fn setup_token_if_present(data: &Path) -> Option<String> {
    std::fs::read_to_string(data.join("setup-token"))
        .ok()
        .map(|token| token.trim().to_owned())
}

async fn setup_account(server: &Server, client: &Client) -> String {
    let setup = client
        .post(format!("{}/setup", server.base))
        .json(&json!({
            "username": "fresh-author",
            "password": PASSWORD,
            "setupToken": setup_token_if_present(&server.data),
        }))
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

// ---------------------------------------------------------------------------
// Config warnings through the built binary
// ---------------------------------------------------------------------------

/// Removes ANSI colour sequences from console output.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Every key named by an `Unknown config key: <key>` line, in output order.
/// Each such line must be a WARN-level log line.
fn unknown_config_keys(output: &str) -> Vec<String> {
    const MARKER: &str = "Unknown config key: ";
    let mut keys = Vec::new();
    for line in strip_ansi(output).lines() {
        if let Some((_, rest)) = line.split_once(MARKER) {
            assert!(
                line.contains(" WARN "),
                "an unknown-key line must be logged at WARN: {line:?}"
            );
            let key: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            keys.push(key);
        }
    }
    keys
}

async fn ready_with_config(name: &str, config: impl FnOnce(u16) -> String) -> (Server, TempDir) {
    let data = TempDir::new().unwrap();
    let mut server = Server::start_with(data.path(), data.path().join(name), config).await;
    server.ready(&client()).await;
    (server, data)
}

#[tokio::test]
async fn config_warnings_name_each_unknown_key_once_and_no_valid_key() {
    let (mut server, _data) = ready_with_config("all-keys.log", |port| {
        format!(
            "[server]\nbind_address = '127.0.0.1'\nport = {port}\nurl_base = ''\n\
             trusted_proxies = ['127.0.0.1']\nnot_a_key = 1\n\
             [log]\nlevel = 'debug'\nformat = 'text'\n\
             [convergence]\nenabled = false\ninterval_secs = 3600\nbatch_size = 25\n\
             attempt_threshold = 3\n\
             [metadata_cache]\nttl_days = 7\nmax_rows = 100000\nttl_day = 3\n\
             [author_link]\nenabled = false\ninterval_secs = 900\nbatch_size = 25\n\
             [no_such_section]\nanything = true\n"
        )
    })
    .await;
    server.stop();
    let mut keys = unknown_config_keys(&server.logs());
    keys.sort();
    assert_eq!(
        keys,
        [
            "metadata_cache.ttl_day",
            "no_such_section",
            "server.not_a_key"
        ],
        "each unknown key is named exactly once and no valid key is named:\n{}",
        server.logs()
    );
}

#[tokio::test]
async fn config_warning_shows_at_the_default_log_level() {
    let (mut server, _data) = ready_with_config("default-level.log", |port| {
        format!(
            "[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
             [convergence]\nenabled = false\n\
             [metadata_cache]\nttl_day = 3\n\
             [author_link]\nenabled = false\n"
        )
    })
    .await;
    server.stop();
    assert_eq!(
        unknown_config_keys(&server.logs()),
        ["metadata_cache.ttl_day"],
        "with no [log] section the misspelled key is warned about once:\n{}",
        server.logs()
    );
}

#[tokio::test]
async fn removed_auth_section_warns_and_starts_while_url_base_stays_silent() {
    let (mut server, _data) = ready_with_config("removed-auth.log", |port| {
        format!(
            "[server]\nbind_address = '127.0.0.1'\nport = {port}\nurl_base = '/livrarr'\n\
             [log]\nlevel = 'debug'\n\
             [convergence]\nenabled = false\n\
             [author_link]\nenabled = false\n\
             [auth]\nexternal_header = 'X-Remote-User'\ntrusted_proxies = ['not-a-cidr']\n"
        )
    })
    .await;
    server.stop();
    assert_eq!(
        unknown_config_keys(&server.logs()),
        ["auth"],
        "a removed [auth] section is named once; url_base is not named:\n{}",
        server.logs()
    );
}

#[tokio::test]
async fn config_warning_shows_at_the_warn_log_level() {
    let (mut server, _data) = ready_with_config("warn-level.log", |port| {
        format!(
            "[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
             [log]\nlevel = 'warn'\n\
             [convergence]\nenabled = false\n\
             [metadata_cache]\nttl_day = 3\n\
             [author_link]\nenabled = false\n"
        )
    })
    .await;
    server.stop();
    assert_eq!(
        unknown_config_keys(&server.logs()),
        ["metadata_cache.ttl_day"],
        "at level warn the misspelled key is warned about once:\n{}",
        server.logs()
    );
}

#[tokio::test]
async fn url_base_alone_starts_and_names_no_unknown_key() {
    let (mut server, _data) = ready_with_config("url-base.log", |port| {
        format!(
            "[server]\nbind_address = '127.0.0.1'\nport = {port}\nurl_base = '/livrarr'\n\
             [log]\nlevel = 'debug'\n\
             [convergence]\nenabled = false\n\
             [author_link]\nenabled = false\n"
        )
    })
    .await;
    server.stop();
    assert!(
        unknown_config_keys(&server.logs()).is_empty(),
        "url_base is a known key:\n{}",
        server.logs()
    );
}

// ---------------------------------------------------------------------------
// First-run setup token through the built binary
// ---------------------------------------------------------------------------

const SETUP_TOKEN_MESSAGE: &str = "The setup token is missing or wrong. Find it in Livrarr's \
    startup output, or in the file setup-token in its data folder (/config/setup-token in Docker).";

/// The opening words of the console banner shown while setup is pending.
const BANNER_PHRASE: &str = "first-run setup";

/// A well-formed token that the server never generates for these cases.
const WRONG_TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// Standard config plus loopback as a trusted proxy, so `X-Real-IP` gives each
/// request its own client address for the per-IP limits.
fn proxied_config_at(port: u16, level: &str) -> String {
    format!(
        "[server]\nbind_address = '127.0.0.1'\nport = {port}\ntrusted_proxies = ['127.0.0.1']\n\
         [log]\nlevel = '{level}'\n\
         [convergence]\nenabled = false\n\
         [author_link]\nenabled = false\n"
    )
}

fn proxied_config(port: u16) -> String {
    proxied_config_at(port, "info")
}

fn token_path(data: &Path) -> PathBuf {
    data.join("setup-token")
}

fn is_token_shaped(word: &str) -> bool {
    word.len() == 32 && word.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Every 32-character lowercase-hex word in `text`.
fn token_shaped_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| is_token_shaped(word))
        .map(str::to_owned)
        .collect()
}

fn assert_no_banner(stdout: &str) {
    assert!(
        !stdout.to_lowercase().contains(BANNER_PHRASE),
        "no setup banner is printed:\n{stdout}"
    );
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
fn set_mode(path: &Path, bits: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap();
}

#[cfg(unix)]
fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).unwrap().ino()
}

/// Writes `{data}/setup-token` with `contents` and permission `bits`, as a user
/// or tool might leave it.
#[cfg(unix)]
fn prepare_token_file(data: &Path, contents: &str, bits: u32) {
    std::fs::write(token_path(data), contents).unwrap();
    set_mode(&token_path(data), bits);
}

/// A second hard link to the current `setup-token` file. It keeps the file's
/// identity, contents and mode observable after the path is replaced.
#[cfg(unix)]
fn link_witness(data: &Path) -> PathBuf {
    let witness = data.join("setup-token.witness");
    let _ = std::fs::remove_file(&witness);
    std::fs::hard_link(token_path(data), &witness).unwrap();
    witness
}

/// The active `setup-token` is a different file from `witness`, and the
/// witnessed file still has its original contents and mode.
#[cfg(unix)]
fn assert_replaced_not_rewritten(data: &Path, witness: &Path, contents: &str, bits: u32) {
    assert_ne!(
        inode(&token_path(data)),
        inode(witness),
        "setup-token is replaced by a new file, not rewritten in place"
    );
    assert_eq!(std::fs::read_to_string(witness).unwrap(), contents);
    assert_eq!(
        format!("{:o}", mode(witness)),
        format!("{bits:o}"),
        "the old file's mode is untouched"
    );
}

/// Reads `{data}/setup-token`, which must be a private regular file holding one
/// 32-character lowercase-hex token.
fn read_private_token(data: &Path) -> String {
    let path = token_path(data);
    let meta = std::fs::symlink_metadata(&path)
        .unwrap_or_else(|e| panic!("expected a setup-token file at {}: {e}", path.display()));
    assert!(
        meta.file_type().is_file(),
        "setup-token must be a regular file, found {:?}",
        meta.file_type()
    );
    #[cfg(unix)]
    assert_eq!(
        format!("{:o}", mode(&path)),
        "600",
        "setup-token must be owner-only"
    );
    let token = std::fs::read_to_string(&path).unwrap().trim().to_owned();
    assert!(
        is_token_shaped(&token),
        "setup-token must hold 32 lowercase hex characters, found {token:?}"
    );
    token
}

fn file_log_text(data: &Path) -> String {
    let mut text = String::new();
    let mut files = 0;
    for entry in std::fs::read_dir(data.join("logs")).expect("file logging is active") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("livrarr.log.") {
            files += 1;
            text.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    assert!(files > 0, "expected at least one logs/livrarr.log.* file");
    text
}

async fn post_setup(
    server: &Server,
    client: &Client,
    client_ip: &str,
    body: Value,
) -> (StatusCode, String) {
    let response = client
        .post(format!("{}/setup", server.base))
        .header("X-Real-IP", client_ip)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.text().await.unwrap())
}

/// The `GET /setup/status` body, as JSON and as sent.
async fn setup_status(server: &Server, client: &Client) -> (Value, String) {
    let response = client
        .get(format!("{}/setup/status", server.base))
        .header("X-Real-IP", "10.9.9.9")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    (serde_json::from_str(&body).unwrap(), body)
}

async fn login_as(
    server: &Server,
    client: &Client,
    client_ip: &str,
    username: &str,
    password: &str,
) -> (StatusCode, String) {
    let response = client
        .post(format!("{}/auth/login", server.base))
        .header("X-Real-IP", client_ip)
        .json(&json!({"username": username, "password": password, "rememberMe": false}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.text().await.unwrap())
}

async fn login_from(server: &Server, client: &Client, client_ip: &str) -> StatusCode {
    login_as(server, client, client_ip, "fresh-author", PASSWORD)
        .await
        .0
}

/// A valid owner account body; `None` omits `setupToken` entirely.
fn owner_setup(token: Option<&str>) -> Value {
    let mut body = json!({"username": "fresh-author", "password": PASSWORD});
    if let Some(token) = token {
        body["setupToken"] = Value::from(token);
    }
    body
}

async fn me_status(
    server: &Server,
    client: &Client,
    client_ip: &str,
    header: (&str, &str),
) -> (StatusCode, String) {
    let response = client
        .get(format!("{}/auth/me", server.base))
        .header("X-Real-IP", client_ip)
        .header(header.0, header.1)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.text().await.unwrap())
}

#[tokio::test]
async fn setup_refuses_missing_empty_blank_and_wrong_tokens_then_accepts_a_padded_right_one_once() {
    let data = TempDir::new().unwrap();
    let client = client();
    let mut server =
        Server::start_with(data.path(), data.path().join("matrix.log"), proxied_config).await;
    server.ready(&client).await;

    let intruder = |token: Option<&str>| {
        let mut body = json!({"username": "intruder", "password": "intruder-pass-1"});
        if let Some(token) = token {
            body["setupToken"] = Value::from(token);
        }
        body
    };
    let mut refusals = Vec::new();
    for (ip, case, body) in [
        ("10.0.4.1", "missing", intruder(None)),
        ("10.0.4.2", "empty", intruder(Some(""))),
        ("10.0.4.3", "whitespace", intruder(Some(" \t\n "))),
        ("10.0.4.4", "wrong", intruder(Some(WRONG_TOKEN))),
    ] {
        let (status, text) = post_setup(&server, &client, ip, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{case} token: {text}");
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed["message"], SETUP_TOKEN_MESSAGE,
            "{case} token: {text}"
        );
        refusals.push(text);
    }

    let token = read_private_token(data.path());
    assert_ne!(token, WRONG_TOKEN);
    for text in &refusals {
        assert!(
            !text.contains(&token),
            "a refusal never carries the token: {text}"
        );
    }
    assert_eq!(
        setup_status(&server, &client).await.0,
        json!({"setupRequired": true}),
        "refused attempts leave setup pending"
    );
    let (status, text) =
        login_as(&server, &client, "10.0.4.5", "intruder", "intruder-pass-1").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the intruder has no account: {text}"
    );

    let padded = format!(" \t{token}\n ");
    let (status, text) = post_setup(&server, &client, "10.0.4.6", owner_setup(Some(&padded))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the right token, padded, sets up: {text}"
    );
    assert!(
        !text.contains(&token),
        "the success body carries no setup token"
    );
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let mut keys: Vec<&str> = parsed
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["apiKey", "token"]);
    let bearer = format!("Bearer {}", parsed["token"].as_str().unwrap());
    let api_key = parsed["apiKey"].as_str().unwrap();
    for (ip, header) in [
        ("10.0.4.7", ("Authorization", bearer.as_str())),
        ("10.0.4.8", ("X-Api-Key", api_key)),
    ] {
        let (status, text) = me_status(&server, &client, ip, header).await;
        assert_eq!(status, StatusCode::OK, "{}: {text}", header.0);
        let me: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(me["user"]["username"], "fresh-author");
    }

    let (status, text) = post_setup(&server, &client, "10.0.4.9", owner_setup(Some(&token))).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a second setup is refused: {text}"
    );
    server.stop();
}

#[tokio::test]
async fn fresh_install_writes_private_setup_token_reused_on_restart_and_single_use() {
    let data = TempDir::new().unwrap();
    let client = client();

    let mut first =
        Server::start_with(data.path(), data.path().join("first.log"), proxied_config).await;
    first.ready(&client).await;
    let token = read_private_token(data.path());
    assert!(
        first.stdout().contains(&token),
        "the token is printed to stdout:\n{}",
        first.logs()
    );
    assert!(
        first.stdout().to_lowercase().contains(BANNER_PHRASE),
        "the banner says first-run setup needs the token:\n{}",
        first.stdout()
    );
    first.stop();
    assert!(
        !file_log_text(data.path()).contains(&token),
        "the token never reaches logs/livrarr.log.*"
    );

    #[cfg(unix)]
    let witness = link_witness(data.path());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut second =
        Server::start_with(data.path(), data.path().join("second.log"), proxied_config).await;
    second.ready(&client).await;
    assert_eq!(
        read_private_token(data.path()),
        token,
        "restart reuses the token"
    );
    #[cfg(unix)]
    assert_replaced_not_rewritten(data.path(), &witness, &token, 0o600);
    assert!(
        second.stdout().contains(&token),
        "restart prints the same token"
    );

    let (status, status_body) = setup_status(&second, &client).await;
    assert_eq!(status, json!({"setupRequired": true}));
    assert!(
        !status_body.contains(&token),
        "setup/status never carries the token"
    );

    assert_ne!(WRONG_TOKEN, token);
    for (ip, body) in [
        ("10.0.0.1", owner_setup(Some(WRONG_TOKEN))),
        ("10.0.0.2", owner_setup(None)),
    ] {
        let (status, text) = post_setup(&second, &client, ip, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "wrong or missing token: {text}"
        );
        assert!(
            !text.contains(&token),
            "a refusal never carries the token: {text}"
        );
    }

    let (status, text) = post_setup(&second, &client, "10.0.0.3", owner_setup(Some(&token))).await;
    assert_eq!(status, StatusCode::OK, "right token: {text}");
    assert!(
        !text.contains(&token),
        "the success body never carries the token"
    );
    assert!(
        std::fs::symlink_metadata(token_path(data.path())).is_err(),
        "the token file is gone after setup"
    );
    let session = serde_json::from_str::<Value>(&text).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let tail = client
        .get(format!("{}/system/logs/tail?lines=200", second.base))
        .header("X-Real-IP", "10.0.0.4")
        .bearer_auth(&session)
        .send()
        .await
        .unwrap();
    assert_eq!(tail.status(), StatusCode::OK);
    let tail: Vec<String> = tail.json().await.unwrap();
    assert!(
        tail.iter().any(|line| line.contains("Livrarr starting")),
        "the in-memory log still holds this start: {tail:?}"
    );
    assert!(
        tail.iter().all(|line| !line.contains(&token)),
        "the in-memory log never carries the token"
    );
    second.stop();
    assert!(!file_log_text(data.path()).contains(&token));

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut third =
        Server::start_with(data.path(), data.path().join("third.log"), proxied_config).await;
    third.ready(&client).await;
    third.stop();
    assert_no_banner(&third.stdout());
    assert!(!third.logs().contains(&token));
    assert!(std::fs::symlink_metadata(token_path(data.path())).is_err());
}

#[tokio::test]
async fn two_fresh_installs_get_different_setup_tokens() {
    let mut tokens = Vec::new();
    for name in ["install-a.log", "install-b.log"] {
        let data = TempDir::new().unwrap();
        let mut server =
            Server::start_with(data.path(), data.path().join(name), proxied_config).await;
        server.ready(&client()).await;
        tokens.push(read_private_token(data.path()));
        server.stop();
    }
    assert_ne!(tokens[0], tokens[1], "each install generates its own token");
}

#[tokio::test]
async fn setup_token_appears_only_in_the_stdout_banner_with_verbose_logging() {
    let data = TempDir::new().unwrap();
    let client = client();
    let mut server = Server::start_with(data.path(), data.path().join("verbose.log"), |port| {
        proxied_config_at(port, "debug")
    })
    .await;
    server.ready(&client).await;
    let token = read_private_token(data.path());

    post_setup(&server, &client, "10.0.5.1", owner_setup(Some(WRONG_TOKEN))).await;
    let padded = format!(" {token}\n");
    let (status, text) = post_setup(&server, &client, "10.0.5.2", owner_setup(Some(&padded))).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        login_from(&server, &client, "10.0.5.3").await,
        StatusCode::OK
    );
    let session = serde_json::from_str::<Value>(&text).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let tail = client
        .get(format!("{}/system/logs/tail?lines=200", server.base))
        .header("X-Real-IP", "10.0.5.4")
        .bearer_auth(&session)
        .send()
        .await
        .unwrap();
    assert_eq!(tail.status(), StatusCode::OK);
    let tail: Vec<String> = tail.json().await.unwrap();
    server.stop();

    let stdout = server.stdout();
    assert!(stdout.contains("Livrarr starting"), "stdout was captured");
    assert!(
        stdout.contains(&token),
        "the banner prints the token:\n{stdout}"
    );
    assert!(
        !server.stderr().contains(&token),
        "stderr never carries the token:\n{}",
        server.stderr()
    );
    let file_log = file_log_text(data.path());
    assert!(
        file_log.contains("Livrarr starting") && file_log.contains("login successful"),
        "the file log covers the start and the requests"
    );
    assert!(
        !file_log.contains(&token),
        "the file log never carries the token"
    );
    assert!(
        tail.iter().any(|line| line.contains("Livrarr starting")),
        "coverage lost: the in-memory log tail no longer reaches back to startup: {tail:?}"
    );
    assert!(
        tail.iter().any(|line| line.contains("login successful")),
        "the in-memory log covers the requests: {tail:?}"
    );
    assert!(
        tail.iter().all(|line| !line.contains(&token)),
        "the in-memory log never carries the token"
    );
}

/// Starts a fresh install whose data folder already holds `setup-token`
/// (a user or tool changed the folder before the first start).
async fn start_over_prepared_token_path(data: &Path, name: &str) -> Server {
    let mut server = Server::start_with(data, data.join(name), proxied_config).await;
    server.ready(&client()).await;
    server
}

#[cfg(unix)]
#[tokio::test]
async fn valid_world_readable_setup_token_is_reused_and_made_private() {
    let data = TempDir::new().unwrap();
    let existing = "00112233445566778899aabbccddeeff";
    prepare_token_file(data.path(), existing, 0o644);
    let witness = link_witness(data.path());
    let mut server = start_over_prepared_token_path(data.path(), "readable.log").await;
    let token = read_private_token(data.path());
    server.stop();
    assert_eq!(token, existing, "a valid token is reused");
    assert_replaced_not_rewritten(data.path(), &witness, existing, 0o644);
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_world_readable_setup_token_is_replaced_privately() {
    let data = TempDir::new().unwrap();
    prepare_token_file(data.path(), "not-a-token\n", 0o644);
    let witness = link_witness(data.path());
    let mut server = start_over_prepared_token_path(data.path(), "invalid.log").await;
    let token = read_private_token(data.path());
    server.stop();
    assert_ne!(token, "not-a-token");
    assert_replaced_not_rewritten(data.path(), &witness, "not-a-token\n", 0o644);
}

#[cfg(unix)]
#[tokio::test]
async fn setup_token_file_with_anything_after_the_token_is_replaced() {
    let prefix = "00112233445566778899aabbccddeeff";
    let cases = [
        (
            "trailing.log",
            format!("{prefix}{}INVALID-TRAILING-DATA", " ".repeat(64)),
        ),
        ("newline.log", format!("{prefix}\n")),
    ];
    for (log_name, contents) in cases {
        let data = TempDir::new().unwrap();
        prepare_token_file(data.path(), &contents, 0o644);
        let mut server = start_over_prepared_token_path(data.path(), log_name).await;
        let written = std::fs::read(token_path(data.path())).unwrap();
        let file_mode = mode(&token_path(data.path()));
        server.stop();
        assert!(
            written.len() == 32
                && written
                    .iter()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{log_name}: setup-token must hold exactly 32 lowercase hex bytes, found {:?}",
            String::from_utf8_lossy(&written)
        );
        assert_ne!(
            written,
            prefix.as_bytes(),
            "{log_name}: a file with more than the token is not reused"
        );
        assert_eq!(
            format!("{file_mode:o}"),
            "600",
            "{log_name}: setup-token must be owner-only"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn private_valid_setup_token_is_still_replaced_on_start() {
    let data = TempDir::new().unwrap();
    let existing = "a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5";
    prepare_token_file(data.path(), existing, 0o600);
    let witness = link_witness(data.path());
    let mut server = start_over_prepared_token_path(data.path(), "private.log").await;
    let token = read_private_token(data.path());
    server.stop();
    assert_eq!(token, existing, "a valid token is reused");
    assert_replaced_not_rewritten(data.path(), &witness, existing, 0o600);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_setup_token_is_replaced_without_touching_its_target() {
    let data = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    let target = elsewhere.path().join("target.txt");
    let target_text = "ffeeddccbbaa99887766554433221100";
    std::fs::write(&target, target_text).unwrap();
    set_mode(&target, 0o644);
    std::os::unix::fs::symlink(&target, token_path(data.path())).unwrap();
    let mut server = start_over_prepared_token_path(data.path(), "symlink.log").await;
    let token = read_private_token(data.path());
    server.stop();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), target_text);
    assert_eq!(
        format!("{:o}", mode(&target)),
        "644",
        "the link's target is unchanged"
    );
    assert_ne!(token, target_text, "nothing is read through a link");
}

#[tokio::test]
async fn setup_token_path_that_is_a_directory_stops_startup() {
    let data = TempDir::new().unwrap();
    let path = token_path(data.path());
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep"), "x").unwrap();
    let mut server =
        Server::start_with(data.path(), data.path().join("dir.log"), proxied_config).await;
    let (status, served) = server.observe_refusal(&client()).await;
    server.stop();
    let output = server.logs();
    assert!(
        !served && status.is_some_and(|s| s.code() == Some(1)),
        "the process exits 1 before readiness; status={status:?}, served={served}\n{output}"
    );
    let expected = format!("Cannot write the setup token file {}: ", path.display());
    assert!(
        server.stderr().contains(&expected)
            && server
                .stderr()
                .contains(". Fix the data folder and restart."),
        "stderr names the path and the fix:\n{output}"
    );
    assert!(
        token_shaped_words(&output).is_empty(),
        "no token is printed:\n{output}"
    );
}

fn replace_token_file_with_directory(data: &Path) {
    let path = token_path(data);
    if std::fs::symlink_metadata(&path).is_ok() {
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep"), "x").unwrap();
}

fn assert_directory_kept(data: &Path) {
    let path = token_path(data);
    assert!(
        path.is_dir() && path.join("keep").is_file(),
        "cleanup never deletes a directory recursively"
    );
}

/// WARN lines reporting a failure to remove the setup-token path: they name
/// the path, or name the file together with a removal.
fn token_cleanup_warnings(output: &str, path: &Path) -> Vec<String> {
    let shown = path.display().to_string();
    strip_ansi(output)
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            let names_file = lower.contains("setup-token") || lower.contains("setup token");
            let names_removal = ["remov", "delet", "clean"]
                .iter()
                .any(|word| lower.contains(word));
            line.contains(" WARN ") && (line.contains(&shown) || (names_file && names_removal))
        })
        .map(str::to_owned)
        .collect()
}

/// A fresh install whose data folder holds a valid token before the first
/// start, completed through HTTP with that token.
#[cfg(unix)]
async fn completed_install_with_prepared_token(data: &Path, token: &str) {
    prepare_token_file(data, token, 0o600);
    let client = client();
    let mut server = Server::start_with(data, data.join("setup.log"), proxied_config).await;
    server.ready(&client).await;
    let (status, text) = post_setup(&server, &client, "10.0.6.1", owner_setup(Some(token))).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    server.stop();
    tokio::time::sleep(Duration::from_millis(1100)).await;
}

#[tokio::test]
async fn setup_completes_when_the_token_file_cannot_be_removed() {
    let data = TempDir::new().unwrap();
    let client = client();
    let mut server =
        Server::start_with(data.path(), data.path().join("first.log"), proxied_config).await;
    server.ready(&client).await;
    let token = setup_token_if_present(data.path());
    replace_token_file_with_directory(data.path());

    let (status, text) =
        post_setup(&server, &client, "10.0.1.1", owner_setup(token.as_deref())).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        setup_status(&server, &client).await.0,
        json!({"setupRequired": false}),
        "the account is claimed"
    );
    let (status, text) =
        post_setup(&server, &client, "10.0.1.2", owner_setup(token.as_deref())).await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    server.stop();

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut restarted =
        Server::start_with(data.path(), data.path().join("restart.log"), proxied_config).await;
    restarted.ready(&client).await;
    assert_eq!(
        login_from(&restarted, &client, "10.0.1.3").await,
        StatusCode::OK
    );
    restarted.stop();
    assert_no_banner(&restarted.stdout());
}

#[cfg(unix)]
#[tokio::test]
async fn setup_warns_without_the_token_when_the_token_file_cannot_be_removed() {
    let data = TempDir::new().unwrap();
    let client = client();
    let token = "1f2e3d4c5b6a79881f2e3d4c5b6a7988";
    prepare_token_file(data.path(), token, 0o600);
    let mut server =
        Server::start_with(data.path(), data.path().join("first.log"), proxied_config).await;
    server.ready(&client).await;
    replace_token_file_with_directory(data.path());
    let path = token_path(data.path());
    let before = token_cleanup_warnings(&server.logs(), &path).len();

    let (status, text) = post_setup(&server, &client, "10.0.2.1", owner_setup(Some(token))).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    server.stop();
    let warnings = token_cleanup_warnings(&server.logs(), &path);
    assert!(
        warnings.len() > before,
        "a failed cleanup after setup logs a WARN:\n{}",
        server.logs()
    );
    assert!(warnings.iter().all(|line| !line.contains(token)));
    assert_directory_kept(data.path());
}

#[cfg(unix)]
#[tokio::test]
async fn restart_warns_when_a_stale_token_directory_cannot_be_removed() {
    let data = TempDir::new().unwrap();
    let client = client();
    let token = "2a3b4c5d6e7f80912a3b4c5d6e7f8091";
    completed_install_with_prepared_token(data.path(), token).await;
    replace_token_file_with_directory(data.path());

    let mut restarted = Server::start_with(
        data.path(),
        data.path().join("with-dir.log"),
        proxied_config,
    )
    .await;
    restarted.ready(&client).await;
    restarted.stop();
    let warnings = token_cleanup_warnings(&restarted.logs(), &token_path(data.path()));
    assert!(
        !warnings.is_empty(),
        "a restart that cannot remove the stale path logs a WARN:\n{}",
        restarted.logs()
    );
    assert!(
        !restarted.logs().contains(token),
        "the token is never printed after setup:\n{}",
        restarted.logs()
    );
    assert_directory_kept(data.path());
}

#[cfg(unix)]
#[tokio::test]
async fn restart_after_setup_removes_a_stale_token_file() {
    let data = TempDir::new().unwrap();
    let client = client();
    let token = "3c4d5e6f708192a33c4d5e6f708192a3";
    completed_install_with_prepared_token(data.path(), token).await;
    let path = token_path(data.path());
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, token).unwrap();
    assert!(path.is_file(), "the stale token file exists before launch");

    let mut recovered = Server::start_with(
        data.path(),
        data.path().join("recovered.log"),
        proxied_config,
    )
    .await;
    recovered.ready(&client).await;
    assert!(
        std::fs::symlink_metadata(&path).is_err(),
        "startup removes a stale token file once setup is complete"
    );
    assert_eq!(
        setup_status(&recovered, &client).await.0,
        json!({"setupRequired": false})
    );
    assert_eq!(
        login_from(&recovered, &client, "10.0.2.2").await,
        StatusCode::OK
    );
    recovered.stop();
    assert_no_banner(&recovered.stdout());
    assert!(!recovered.logs().contains(token));
}

#[tokio::test]
async fn finished_install_keeps_session_and_api_key_and_serves_no_token_after_restart() {
    let data = TempDir::new().unwrap();
    let client = client();
    let mut server =
        Server::start_with(data.path(), data.path().join("first.log"), proxied_config).await;
    server.ready(&client).await;
    let token = setup_token_if_present(data.path());
    let (status, text) =
        post_setup(&server, &client, "10.0.3.1", owner_setup(token.as_deref())).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    let session = body["token"].as_str().unwrap().to_owned();
    let api_key = body["apiKey"].as_str().unwrap().to_owned();
    server.stop();

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut restarted =
        Server::start_with(data.path(), data.path().join("restart.log"), proxied_config).await;
    restarted.ready(&client).await;
    assert!(std::fs::symlink_metadata(token_path(data.path())).is_err());
    let (status_json, status_body) = setup_status(&restarted, &client).await;
    assert_eq!(
        status_json,
        json!({"setupRequired": false}),
        "{status_body}"
    );
    let (status, text) = post_setup(&restarted, &client, "10.0.3.2", owner_setup(None)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    let bearer = format!("Bearer {session}");
    for (ip, header) in [
        ("10.0.3.3", ("Authorization", bearer.as_str())),
        ("10.0.3.4", ("X-Api-Key", api_key.as_str())),
    ] {
        let (status, text) = me_status(&restarted, &client, ip, header).await;
        assert_eq!(status, StatusCode::OK, "{}: {text}", header.0);
    }
    restarted.stop();
    assert_no_banner(&restarted.stdout());
    if let Some(token) = token {
        assert!(!restarted.logs().contains(&token));
    }
}
