//! Fresh-author setup contract at 98d35a37 (spec v2 / PM decisions).
//!
//! Registered on the server crate to use Cargo's actual `livrarr` binary. Fresh
//! cases never seed schema/authors or call create_test_db. Historical cases run
//! every real migration through 088 before constructing explicitly old data.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use livrarr_db::pool::{check_version_gate, create_sqlite_pool, run_migrations};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    AuthorDb, AuthorLinkDb, CreateAuthorDbRequest, CreateAuthorGateRequest,
    CreateDownloadClientDbRequest, CreateGrabDbRequest, CreateIndexerDbRequest,
    CreateWorkDbRequest, DownloadClientDb, GrabDb, IndexerDb, WorkDbCreate,
};
use livrarr_domain::author_link::{AuthorLinkTrigger, AuthorNameSource};
use livrarr_domain::{DownloadClientImplementation, GrabStatus, MediaType};
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
        Self::launch(data, log, config, "").await
    }

    /// Starts the binary as `start_with` does, except that requests to
    /// `localhost` and `127.0.0.1` bypass the refusing proxy, so the binary
    /// reaches local fake servers while every other outbound request still
    /// meets the proxy.
    async fn start_local(data: &Path, log: PathBuf, config: impl FnOnce(u16) -> String) -> Self {
        Self::launch(data, log, config, "localhost,127.0.0.1").await
    }

    async fn launch(
        data: &Path,
        log: PathBuf,
        config: impl FnOnce(u16) -> String,
        no_proxy: &str,
    ) -> Self {
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
            .env("NO_PROXY", no_proxy);
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

/// The startup warning for a `trusted_proxies` entry that is not an address
/// or range.
fn ignored_proxy_warning(entry: &str) -> String {
    format!(
        "Ignored [server] trusted_proxies entry \"{entry}\": use an IP address or range such \
         as 172.18.0.0/16; host names and ports are not supported"
    )
}

#[tokio::test]
async fn startup_warns_once_about_a_host_name_in_trusted_proxies() {
    let (mut server, _data) = ready_with_config("proxy-warning.log", |port| {
        format!(
            "[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
             trusted_proxies = ['nginx']\n\
             [log]\nlevel = 'info'\nformat = 'text'\n\
             [convergence]\nenabled = false\n\
             [author_link]\nenabled = false\n"
        )
    })
    .await;
    server.stop();
    let stdout = strip_ansi(&server.stdout());
    assert!(
        stdout.contains(STARTUP_EVENT),
        "control: startup logged to stdout:\n{stdout}"
    );
    let expected = ignored_proxy_warning("nginx");
    let lines: Vec<&str> = stdout.lines().filter(|l| l.contains(&expected)).collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one stdout line carries {expected:?}; found {lines:?}\n{stdout}"
    );
    assert!(
        lines[0].contains(" WARN "),
        "the proxy warning is logged at WARN: {:?}",
        lines[0]
    );
}

/// The `config` / `warning` messages of an admin health reply; empty when the
/// body is not a list of items.
fn config_warning_messages(body: &str) -> Vec<String> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item["source"] == "config" && item["checkType"] == "warning")
        .filter_map(|item| item["message"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn startup_keeps_unknown_keys_for_the_admin_health_list() {
    let secret_key = toml_key("x?apikey=ac712Marker");
    let (mut server, _data) = ready_with_config("unknown-keys-admin.log", |port| {
        format!(
            "nginx = 1\n{secret_key} = 1\n\
             [server]\nbind_address = '127.0.0.1'\nport = {port}\n\
             api_kye = \"sk-live-ac711Marker\"\n\
             [log]\nlevel = 'info'\nformat = 'text'\n\
             [convergence]\nenabled = false\n\
             [author_link]\nenabled = false\n\
             [foo]\n"
        )
    })
    .await;
    let client = client();
    let (token, _) = setup_owner(&server, &client).await;
    let (status, body) = api(
        &server,
        &client,
        &token,
        reqwest::Method::GET,
        "/system/health",
        None,
    )
    .await;
    server.stop();

    let logged = unknown_config_keys(&server.logs());
    for key in ["foo", "nginx", "server.api_kye"] {
        assert!(
            logged.iter().any(|k| k == key),
            "control: startup logged unknown key {key:?}; logged {logged:?}"
        );
    }

    let messages = config_warning_messages(&body);
    let mut findings = Findings::default();
    for key in ["foo", "nginx", "server.api_kye"] {
        let row = format!("Unknown config key: {key}");
        findings.check(messages.contains(&row), || {
            format!("no warning row {row:?}; rows {messages:?}; status {status}, body {body}")
        });
    }
    let masked: Vec<&String> = messages
        .iter()
        .filter(|m| m.starts_with("Unknown config key: x?apikey="))
        .collect();
    findings.check(
        masked.len() == 1 && masked[0].contains("[REDACTED]"),
        || format!("expected one masked row for the secret-shaped key; got {masked:?}"),
    );
    findings.check(status == StatusCode::OK, || {
        format!("admin health answers 200; got {status}")
    });
    for marker in ["ac711Marker", "ac712Marker"] {
        findings.check(!body.contains(marker), || {
            format!("the reply carries {marker:?}: {body}")
        });
    }
    findings.finish("unknown keys reach the admin health list");
}

// ---------------------------------------------------------------------------
// Log file retention and log file failure through the built binary
// ---------------------------------------------------------------------------

/// Runs `attempt` with the UTC date it starts on and returns its result when
/// it also ends on that date. An attempt that spans UTC midnight is discarded
/// and run once more, on a fresh data folder and the new date.
async fn within_one_utc_day<T, F, Fut>(mut attempt: F) -> T
where
    F: FnMut(chrono::NaiveDate) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    for _ in 0..2 {
        let day = chrono::Utc::now().date_naive();
        let outcome = attempt(day).await;
        if chrono::Utc::now().date_naive() == day {
            return outcome;
        }
    }
    panic!("the UTC date changed during both attempts");
}

/// The log file name for `day`.
fn log_name_for(day: chrono::NaiveDate) -> String {
    format!("livrarr.log.{}", day.format("%Y-%m-%d"))
}

fn folder_entries(dir: &Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("read folder")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[tokio::test]
async fn startup_keeps_todays_log_file_and_the_30_newest_of_85() {
    let (expected, actual) = within_one_utc_day(|today| async move {
        let data = TempDir::new().unwrap();
        let logs = data.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        let mut seeded = Vec::new();
        for days in (1..=85).rev() {
            let name = log_name_for(today - chrono::Duration::days(days));
            std::fs::write(logs.join(&name), b"seeded\n").unwrap();
            seeded.push(name);
            std::thread::sleep(Duration::from_millis(15));
        }
        assert_eq!(folder_entries(&logs).len(), 85, "control: seeded");

        let client = client();
        let mut server = Server::start(data.path(), data.path().join("ac317.log")).await;
        server.ready(&client).await;
        server.stop();

        let mut expected: std::collections::BTreeSet<String> =
            seeded.iter().rev().take(30).cloned().collect();
        expected.insert(log_name_for(today));
        (expected, folder_entries(&logs))
    })
    .await;

    assert_eq!(
        actual,
        expected,
        "logs/ holds today's file and the 30 newest: expected {} entries, found {}; \
         unexpected {:?}",
        expected.len(),
        actual.len(),
        actual.difference(&expected).collect::<Vec<_>>()
    );
}

/// What one start with today's log file name blocked produced.
struct BlockedStart {
    /// `Err` holds why the binary did not serve.
    served: Result<(), String>,
    status: Option<(StatusCode, String)>,
    stderr: String,
}

#[tokio::test]
async fn startup_with_todays_log_file_blocked_serves_and_reports_the_error() {
    const BUILD_ERROR: &str = "failed to create initial log file";
    let outcome = within_one_utc_day(|today| async move {
        let data = TempDir::new().unwrap();
        let logs = data.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        std::fs::create_dir(logs.join(log_name_for(today))).unwrap();

        let client = client();
        let mut server = Server::start(data.path(), data.path().join("ac318.log")).await;
        let served = match server.observe_refusal(&client).await {
            (_, true) => Ok(()),
            (Some(exit), false) => Err(format!("exited before serving ({exit})")),
            (None, false) => Err("did not serve within the deadline".to_owned()),
        };
        let mut status = None;
        if served.is_ok() {
            server.ready(&client).await;
            let (token, _) = setup_owner(&server, &client).await;
            status = Some(
                api(
                    &server,
                    &client,
                    &token,
                    reqwest::Method::GET,
                    "/system/status",
                    None,
                )
                .await,
            );
        }
        server.stop();
        BlockedStart {
            served,
            status,
            stderr: server.stderr(),
        }
    })
    .await;

    let stderr = &outcome.stderr;
    if let Err(why) = &outcome.served {
        panic!("the binary serves GET /api/v1/health: it {why}\n--- stderr:\n{stderr}");
    }
    let (status, body) = outcome.status.expect("system status was read");
    assert_eq!(status, StatusCode::OK, "control: system status: {body}");
    let reply: Value = serde_json::from_str(&body).expect("system status is JSON");
    let init_error = reply["logInitError"].as_str().unwrap_or_default();
    assert!(
        init_error.contains(BUILD_ERROR),
        "logInitError names the failed file creation: {body}"
    );
    assert!(
        stderr.contains(BUILD_ERROR),
        "stderr names the failed file creation:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "the log file failure is reported, not a panic:\n{stderr}"
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

// ---------------------------------------------------------------------------
// Secrets in the log sinks, Retry content-path lookups and release-search
// warnings through the built binary
// ---------------------------------------------------------------------------

/// One request a fake server received.
#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    /// Path and query, as sent.
    target: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// What a fake server answers.
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl Reply {
    fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// A local HTTP server on `127.0.0.1` that records every request it receives.
struct Fake {
    port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fake {
    async fn start(respond: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake server");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let respond = Arc::new(respond);
        let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let seen = seen.clone();
            let respond = respond.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, usize::MAX)
                    .await
                    .unwrap_or_default();
                let recorded = Recorded {
                    method: parts.method.to_string(),
                    target: parts
                        .uri
                        .path_and_query()
                        .map(|p| p.to_string())
                        .unwrap_or_default(),
                    headers: parts
                        .headers
                        .iter()
                        .map(|(n, v)| {
                            (
                                n.to_string(),
                                String::from_utf8_lossy(v.as_bytes()).into_owned(),
                            )
                        })
                        .collect(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                };
                let reply = respond(&recorded);
                seen.lock().unwrap().push(recorded);
                let mut response = axum::response::Response::builder().status(reply.status);
                for (name, value) in reply.headers {
                    response = response.header(name, value);
                }
                response.body(axum::body::Body::from(reply.body)).unwrap()
            }
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve fake server");
        });
        Self {
            port,
            requests,
            task,
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A loopback port with nothing listening on it.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    fn name(self) -> &'static str {
        match self {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        }
    }
}

/// The harness's standard `config.toml` with a log level, a log format and
/// `root_keys` (already TOML) ahead of the first section.
fn logged_config(port: u16, level: &str, format: LogFormat, root_keys: &str) -> String {
    format!(
        "{root_keys}[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
         [log]\nlevel = '{level}'\nformat = '{}'\n\
         [convergence]\nenabled = false\n\
         [author_link]\nenabled = false\n",
        format.name()
    )
}

/// `text` as a quoted TOML key.
fn toml_key(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Completes first-run setup; returns the session token and the user's API key.
async fn setup_owner(server: &Server, client: &Client) -> (String, String) {
    let response = client
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
        response.status(),
        StatusCode::OK,
        "real account setup failed:\n{}",
        server.logs()
    );
    let body: Value = response.json().await.unwrap();
    (
        body["token"].as_str().expect("session token").to_owned(),
        body["apiKey"].as_str().expect("user API key").to_owned(),
    )
}

/// One authenticated API call; returns the status and the body as sent.
async fn api(
    server: &Server,
    client: &Client,
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut request = client
        .request(method, format!("{}{path}", server.base))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (status, response.text().await.unwrap())
}

/// The `GET /system/logs/tail?lines=200` reply body, as sent.
async fn read_tail(server: &Server, client: &Client, token: &str) -> String {
    let (status, body) = api(
        server,
        client,
        token,
        reqwest::Method::GET,
        "/system/logs/tail?lines=200",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "logs tail: {body}");
    body
}

/// What the binary wrote to its log sinks during one case.
struct Sinks {
    format: LogFormat,
    stdout: String,
    stderr: String,
    file: String,
    /// Each logs-tail reply body, as sent.
    tails: Vec<String>,
}

impl Sinks {
    fn last_tail_lines(&self) -> Vec<String> {
        self.tails
            .last()
            .map(|t| serde_json::from_str(t).expect("logs tail reply is a JSON array of lines"))
            .unwrap_or_default()
    }

    /// For stdout, the file and the last tail reply: the text of each log
    /// event containing `marker`. A text event is its line without colour
    /// codes; a JSON event is its `fields` values joined by spaces.
    fn events(&self, marker: &str) -> [(&'static str, Vec<String>); 3] {
        let json = self.format == LogFormat::Json;
        [
            ("stdout", events_in(&self.stdout, json, marker)),
            ("file", events_in(&self.file, json, marker)),
            (
                "tail",
                self.last_tail_lines()
                    .into_iter()
                    .filter(|line| line.contains(marker))
                    .collect(),
            ),
        ]
    }
}

fn events_in(sink: &str, json: bool, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in sink.lines() {
        if json {
            if let Ok(event) = serde_json::from_str::<Value>(line) {
                let Some(fields) = event.get("fields").and_then(Value::as_object) else {
                    continue;
                };
                let text = fields
                    .values()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.contains(marker) {
                    out.push(text);
                }
                continue;
            }
        }
        let line = strip_ansi(line);
        if line.contains(marker) {
            out.push(line);
        }
    }
    out
}

const STARTUP_EVENT: &str = "Livrarr starting — data directory:";
const LLM_WARNING: &str = "LLM test endpoint returned non-success";

/// `text` as it is written inside a JSON string.
fn json_escaped(text: &str) -> String {
    let quoted = serde_json::to_string(text).unwrap();
    quoted[1..quoted.len() - 1].to_owned()
}

/// Every string in `value`: member names and string values, at any depth.
fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(members) => {
            for (name, v) in members {
                out.push(name.clone());
                collect_strings(v, out);
            }
        }
        _ => {}
    }
}

/// The strings of `value`, plus the strings of each of those that parses as
/// JSON once more.
fn decoded_strings(value: &Value) -> Vec<String> {
    let mut first = Vec::new();
    collect_strings(value, &mut first);
    let mut all = first.clone();
    for s in &first {
        if let Ok(inner) = serde_json::from_str::<Value>(s) {
            collect_strings(&inner, &mut all);
        }
    }
    all
}

/// In a text line holding the LLM test warning: the strings of its `body=`
/// value (the last field), parsed as JSON where it parses.
fn llm_body_strings(line: &str) -> Vec<String> {
    let line = strip_ansi(line);
    if !line.contains(LLM_WARNING) {
        return Vec::new();
    }
    let Some(at) = line.rfind(" body=") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(body) = serde_json::from_str::<Value>(line[at + " body=".len()..].trim_end()) {
        collect_strings(&body, &mut out);
    }
    out
}

fn reveals(text: &str, forms: &[String]) -> bool {
    forms.iter().any(|form| text.contains(form.as_str()))
}

fn text_sink_reveals(text: &str, forms: &[String]) -> bool {
    reveals(text, forms)
        || reveals(&strip_ansi(text), forms)
        || text
            .lines()
            .any(|line| llm_body_strings(line).iter().any(|s| reveals(s, forms)))
}

/// The sinks from which `secret` can be recovered: the secret or its
/// JSON-escaped form is a substring of a text sink, of a string decoded from a
/// JSON-format line or the logs-tail reply (and once more where that string
/// is JSON), or of a string decoded from the LLM warning's `body=` value.
fn recoverable_in(sinks: &Sinks, secret: &str) -> Vec<String> {
    let forms = vec![secret.to_owned(), json_escaped(secret)];
    let mut found = Vec::new();
    for (name, text) in [("stdout", &sinks.stdout), ("file", &sinks.file)] {
        let hit = match sinks.format {
            LogFormat::Text => text_sink_reveals(text, &forms),
            LogFormat::Json => text
                .lines()
                .any(|line| match serde_json::from_str::<Value>(line) {
                    Ok(event) => decoded_strings(&event).iter().any(|s| reveals(s, &forms)),
                    Err(_) => text_sink_reveals(line, &forms),
                }),
        };
        if hit {
            found.push(name.to_owned());
        }
    }
    if text_sink_reveals(&sinks.stderr, &forms) {
        found.push("stderr".to_owned());
    }
    for (i, tail) in sinks.tails.iter().enumerate() {
        let parsed: Vec<String> = serde_json::from_str::<Value>(tail)
            .map(|v| decoded_strings(&v))
            .unwrap_or_default();
        if reveals(tail, &forms)
            || parsed.iter().any(|s| reveals(s, &forms))
            || parsed
                .iter()
                .any(|line| llm_body_strings(line).iter().any(|s| reveals(s, &forms)))
        {
            found.push(format!("logs tail read {}", i + 1));
        }
    }
    found
}

/// JSON-format log lines that do not parse as JSON.
fn unparseable_json_lines(sinks: &Sinks) -> Vec<String> {
    if sinks.format != LogFormat::Json {
        return Vec::new();
    }
    let file = sinks.file.lines().filter(|l| !l.trim().is_empty());
    let stdout = sinks.stdout.lines().filter(|l| l.starts_with('{'));
    file.chain(stdout)
        .filter(|line| serde_json::from_str::<Value>(line).is_err())
        .map(str::to_owned)
        .collect()
}

/// Collects every failed check of a case so one run reports all of them.
#[derive(Default)]
struct Findings(Vec<String>);

impl Findings {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if !ok {
            self.0.push(what());
        }
    }

    fn not_recoverable(&mut self, sinks: &Sinks, label: &str, secret: &str) {
        let found = recoverable_in(sinks, secret);
        self.check(found.is_empty(), || {
            format!("{label}: expected in no sink, recoverable from {found:?}")
        });
    }

    fn json_lines_parse(&mut self, sinks: &Sinks) {
        let bad = unparseable_json_lines(sinks);
        self.check(bad.is_empty(), || {
            format!("every JSON-format line parses; these do not: {bad:?}")
        });
    }

    fn finish(self, case: &str) {
        assert!(
            self.0.is_empty(),
            "{case}: {} check(s) failed:\n- {}",
            self.0.len(),
            self.0.join("\n- ")
        );
    }
}

/// Control: `marker` names at least one event in stdout, the file and the
/// last logs-tail reply.
fn assert_event_in_every_sink(sinks: &Sinks, marker: &str) {
    for (sink, events) in sinks.events(marker) {
        assert!(
            !events.is_empty(),
            "control: an event containing {marker:?} is in {sink}\n--- stdout:\n{}\n--- tail:\n{:?}",
            sinks.stdout,
            sinks.last_tail_lines()
        );
    }
    println!("control passed: {marker:?} is in stdout, the file and the logs tail");
}

// AC-108: an emitter with no call-site redaction, a secret-shaped unknown key
// in config.toml, logged at startup.

async fn secret_shaped_unknown_keys_are_masked(format: LogFormat) {
    let first = "ac108Value1x7Kq9Zr";
    let second = "ac108Value2m4Tp8Wn";
    let root_keys = format!(
        "{} = 1\n{} = 1\n",
        toml_key(&format!("x?apikey={first}")),
        toml_key(&format!("y access_token={second}"))
    );
    let data = TempDir::new().unwrap();
    let client = client();
    let mut server = Server::start_with(data.path(), data.path().join("ac108.log"), |port| {
        logged_config(port, "info", format, &root_keys)
    })
    .await;
    server.ready(&client).await;
    let token = setup_account(&server, &client).await;
    let tail = read_tail(&server, &client, &token).await;
    server.stop();
    let sinks = Sinks {
        format,
        stdout: server.stdout(),
        stderr: server.stderr(),
        file: file_log_text(data.path()),
        tails: vec![tail],
    };

    assert_event_in_every_sink(&sinks, STARTUP_EVENT);
    let markers = ["Unknown config key: x?", "Unknown config key: y "];
    for marker in markers {
        assert_event_in_every_sink(&sinks, marker);
    }

    let mut findings = Findings::default();
    for marker in markers {
        for (sink, events) in sinks.events(marker) {
            findings.check(events.iter().all(|e| e.contains("[REDACTED]")), || {
                format!("{marker:?} event in {sink}: expected [REDACTED], got {events:?}")
            });
        }
    }
    findings.not_recoverable(&sinks, "apikey value of the first unknown key", first);
    findings.not_recoverable(
        &sinks,
        "access_token value of the second unknown key",
        second,
    );
    findings.json_lines_parse(&sinks);
    findings.finish(&format!("AC-108 ({} format)", format.name()));
}

#[tokio::test]
async fn secret_shaped_unknown_config_keys_are_masked_in_every_text_sink() {
    secret_shaped_unknown_keys_are_masked(LogFormat::Text).await;
}

#[tokio::test]
async fn secret_shaped_unknown_config_keys_are_masked_in_every_json_sink() {
    secret_shaped_unknown_keys_are_masked(LogFormat::Json).await;
}

// AC-102: every pattern shape of the log cleanser, carried to the sinks by
// the unknown-config-key emitter (its text reaches each sink verbatim).

/// A cleanser shape: the unknown key's text, the marker that finds its event,
/// and the secret parts that must not survive. `text_only` shapes are written
/// with plain quotes, which the JSON sinks re-escape into a spelling the
/// cleanser rules do not name.
struct Shape {
    marker: String,
    text: String,
    secrets: Vec<String>,
    text_only: bool,
}

fn shape(n: u32, body: &str, text_only: bool) -> Shape {
    let marker = format!("shape{n:02}");
    let secret = format!("SECRET{n:02}x9k2m4p7");
    Shape {
        text: format!("{marker} {}", body.replace("{S}", &secret)),
        marker,
        secrets: vec![secret],
        text_only,
    }
}

fn cleanser_shapes() -> Vec<Shape> {
    let mut shapes = vec![
        shape(1, "http://h/x?apikey={S}", false),
        shape(2, "http://h/x?a=1&API_KEY={S}", false),
        shape(3, "http://h/x?sab_apikey={S}", false),
        shape(4, "http://h/x?Access_Token={S}&b=2", false),
        shape(5, "refresh_token={S}", false),
        shape(6, "http://h/x?TOKEN={S}", false),
        shape(7, "http://h/x?b=2&PassKey={S}", false),
        shape(8, "password={S}", false),
        shape(9, "http://h/x?passwd={S}", false),
        shape(10, "http://h/x?AuthKey={S}", false),
        shape(11, "http://h/x?auth={S}", false),
        shape(12, "NZB_KEY={S}", false),
        shape(14, "http://admin:{S}@h/x", false),
        shape(15, "X-Api-Key: {S}", false),
        shape(16, "X-Goog-Api-Key: {S}", false),
        shape(17, "Cookie: SID={S}", false),
        shape(18, "Authorization: Bearer {S}", false),
        shape(19, "Authorization: Basic {S}", false),
        shape(20, "upstream said Bearer {S} was refused", false),
        shape(21, r#"headers: [("X-Api-Key", "{S}")]"#, true),
        shape(22, r#"headers: [("Authorization", "Bearer {S}")]"#, true),
        shape(23, r#"headers: [("X-Goog-Api-Key", "{S}")]"#, true),
        shape(24, r#"headers: [("Cookie", "SID={S}")]"#, true),
        shape(25, r#"headers: [("authorization", "Basic {S}")]"#, true),
        shape(26, r#"{"apikey":"{S}"}"#, false),
        shape(27, r#"{"user_api_key":"{S}","id":7}"#, false),
        shape(28, r#"{"access_token":"{S}"}"#, false),
        shape(29, r#"{"Password":"{S}"}"#, false),
        shape(30, r#"{"passkey":"{S}"}"#, false),
        shape(31, r#"{"client_secret":"{S}"}"#, false),
        shape(32, r#"{"nzb_key":"{S}"}"#, false),
        shape(33, r#"{\"api_key\":\"{S}\"}"#, true),
        shape(34, "first line\nshape34b http://h/x?apikey={S}", false),
        shape(
            35,
            "error sending request for url (http://10.0.0.1:9696/2/api?t=search&apikey={S})",
            false,
        ),
        shape(36, "upstream said Basic {S} was refused", false),
    ];
    // A URL-encoded value: both sides of the escape must go.
    shapes.push(Shape {
        marker: "shape13".into(),
        text: "shape13 http://h/x?api_key=SECRET13pre%2BSECRET13post".into(),
        secrets: vec!["SECRET13pre".into(), "SECRET13post".into()],
        text_only: false,
    });
    shapes
}

/// An LLM reply carrying secret shapes other than the key that was sent.
const FIELD_REPLY: &str = "upstream refused http://h/v1?api_key=FIELDSECRET01x9k2 \
                           with Authorization: Bearer FIELDSECRET02x9k2";
const FIELD_SECRETS: [&str; 2] = ["FIELDSECRET01x9k2", "FIELDSECRET02x9k2"];

/// A secret-shaped field written as the console writes a structured field:
/// real ANSI codes between the name and `=`, then its undecorated twin. The
/// `body` field value reaches the text sinks with its bytes unescaped.
const ANSI_FIELD_REPLY: &str = " then password\u{1b}[0m\u{1b}[2m=\u{1b}[0mANSIFIELD01x9k2 \
                                and password=ANSIFIELD02x9k2 \
                                with shade\u{1b}[0m\u{1b}[2m=\u{1b}[0mgrey";

/// An ordinary ANSI-decorated field in the same reply; its bytes in a sink
/// show that the reply reached it unescaped.
const ANSI_ORDINARY_FIELD: &str = "shade\u{1b}[0m\u{1b}[2m=\u{1b}[0mgrey";
const ANSI_FIELD_SECRETS: [&str; 2] = ["ANSIFIELD01x9k2", "ANSIFIELD02x9k2"];

/// The LLM reply for one run: the ANSI-decorated field in text format only,
/// where the console and file writers carry its bytes as they are.
fn field_reply(format: LogFormat) -> String {
    match format {
        LogFormat::Text => format!("{FIELD_REPLY}{ANSI_FIELD_REPLY}"),
        LogFormat::Json => FIELD_REPLY.to_owned(),
    }
}

fn field_secrets(format: LogFormat) -> Vec<&'static str> {
    let mut secrets = FIELD_SECRETS.to_vec();
    if format == LogFormat::Text {
        secrets.extend(ANSI_FIELD_SECRETS);
    }
    secrets
}

/// A login username carrying secret shapes; a failed login logs it.
const LOGIN_NAME: &str = "probe?apikey=LOGINSECRET01x9k2 Authorization: Bearer LOGINSECRET02x9k2";
const LOGIN_SECRETS: [&str; 2] = ["LOGINSECRET01x9k2", "LOGINSECRET02x9k2"];

/// Ordinary text that the cleanser leaves byte-identical.
const ORDINARY_KEYS: [&str; 4] = [
    "plain01 http://host:9696/api?t=search&cat=7000",
    "plain02 http://host:9696/path",
    "plain03 http://h/x?key=OL123W&q=dune",
    "plain04 Ordinary text: Dune (1965), 412 pages.",
];

async fn every_cleanser_shape_is_masked(format: LogFormat) {
    let shapes: Vec<Shape> = cleanser_shapes()
        .into_iter()
        .filter(|s| format == LogFormat::Text || !s.text_only)
        .collect();
    let mut root_keys = String::new();
    for text in shapes.iter().map(|s| s.text.as_str()).chain(ORDINARY_KEYS) {
        root_keys.push_str(&format!("{} = 1\n", toml_key(text)));
    }
    // A structured field: the LLM test logs the reply as its `body` field.
    // The reply never echoes the key, so only the sink cleanser can mask it.
    let reply = field_reply(format);
    if format == LogFormat::Text {
        assert!(
            reply.contains("password\u{1b}[0m\u{1b}[2m=\u{1b}[0mANSIFIELD01x9k2"),
            "fixture: the reply carries real ANSI bytes between the name and `=`"
        );
    }
    let llm = Fake::start(move |_| Reply::new(401, reply.clone())).await;
    let data = TempDir::new().unwrap();
    let client = client();
    let mut server = Server::start_local(data.path(), data.path().join("ac102.log"), |port| {
        logged_config(port, "info", format, &root_keys)
    })
    .await;
    server.ready(&client).await;
    let token = setup_account(&server, &client).await;
    let (status, body) = api(
        &server,
        &client,
        &token,
        reqwest::Method::PUT,
        "/config/metadata",
        Some(json!({
            "llmEndpoint": llm_endpoint(&llm),
            "llmApiKey": "ac102-unechoed-llm-key",
            "llmModel": "m",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "saving the LLM settings: {body}");
    let (status, body) = api(
        &server,
        &client,
        &token,
        reqwest::Method::POST,
        "/config/metadata/test/llm",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "LLM test: {body}");
    // Another structured field: a failed login logs the username it was given.
    let (status, body) = login_as(&server, &client, "10.0.7.1", LOGIN_NAME, PASSWORD).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "unknown-user login: {body}"
    );
    let tail = read_tail(&server, &client, &token).await;
    server.stop();
    let sinks = Sinks {
        format,
        stdout: server.stdout(),
        stderr: server.stderr(),
        file: file_log_text(data.path()),
        tails: vec![tail],
    };

    assert_event_in_every_sink(&sinks, STARTUP_EVENT);
    for marker in shapes
        .iter()
        .map(|s| s.marker.as_str())
        .chain(ORDINARY_KEYS.iter().map(|k| &k[..7]))
    {
        assert_event_in_every_sink(&sinks, &format!("Unknown config key: {marker}"));
    }

    assert_eq!(
        llm.requests().len(),
        1,
        "control: the fake LLM recorded the test"
    );
    assert_event_in_every_sink(&sinks, "upstream refused");
    if format == LogFormat::Text {
        let tail = sinks.last_tail_lines().join("\n");
        for (sink, text) in [
            ("stdout", &sinks.stdout),
            ("file", &sinks.file),
            ("tail", &tail),
        ] {
            assert!(
                text.contains(ANSI_ORDINARY_FIELD),
                "control: the ordinary ANSI-decorated field reaches {sink} with its bytes as sent"
            );
        }
        println!("control passed: the ANSI-decorated field reaches stdout, the file and the tail");
    }

    let mut findings = Findings::default();
    for (sink, events) in sinks.events("upstream refused") {
        findings.check(events.iter().all(|e| e.contains("[REDACTED]")), || {
            format!("structured body field in {sink}: expected [REDACTED], got {events:?}")
        });
    }
    assert_event_in_every_sink(&sinks, "login failed: user not found");
    for (sink, events) in sinks.events("login failed: user not found") {
        findings.check(events.iter().all(|e| e.contains("[REDACTED]")), || {
            format!("structured username field in {sink}: expected [REDACTED], got {events:?}")
        });
    }
    for secret in LOGIN_SECRETS {
        findings.not_recoverable(
            &sinks,
            &format!("structured username field secret {secret}"),
            secret,
        );
    }
    for secret in field_secrets(format) {
        findings.not_recoverable(
            &sinks,
            &format!("structured body field secret {secret}"),
            secret,
        );
    }
    for shape in &shapes {
        let marker = if shape.marker == "shape34" {
            "shape34b".to_owned()
        } else {
            format!("Unknown config key: {}", shape.marker)
        };
        for (sink, events) in sinks.events(&marker) {
            findings.check(
                !events.is_empty() && events.iter().all(|e| e.contains("[REDACTED]")),
                || {
                    format!(
                        "{} in {sink}: expected [REDACTED], got {events:?}",
                        shape.marker
                    )
                },
            );
        }
        for secret in &shape.secrets {
            findings.not_recoverable(
                &sinks,
                &format!("{} ({:?})", shape.marker, shape.text),
                secret,
            );
        }
    }
    for text in ORDINARY_KEYS {
        let expected = format!("Unknown config key: {text}");
        for (sink, events) in sinks.events(&expected) {
            findings.check(
                !events.is_empty() && events.iter().all(|e| e.trim_end().ends_with(&expected)),
                || format!("{text:?} in {sink}: expected byte-identical, got {events:?}"),
            );
        }
    }
    findings.json_lines_parse(&sinks);
    findings.finish(&format!("AC-102 ({} format)", format.name()));
}

#[tokio::test]
async fn log_cleanser_masks_every_secret_shape_in_text_sinks() {
    every_cleanser_shape_is_masked(LogFormat::Text).await;
}

#[tokio::test]
async fn log_cleanser_masks_every_secret_shape_in_json_sinks() {
    every_cleanser_shape_is_masked(LogFormat::Json).await;
}

// AC-107: the LLM connection test blanks the key it sent out of the reply,
// however the reply spells it.

/// The LLM endpoint as the settings API accepts it: a host name, never a
/// literal private address.
fn llm_endpoint(fake: &Fake) -> String {
    format!("http://localhost:{}/v1", fake.port)
}

fn llm_key_echo_cases() -> Vec<(&'static str, &'static str, String)> {
    let quote_key = r#"probe"quote\back-1234"#;
    vec![
        (
            "(i) seven-character key in prose",
            "hunter2",
            "Incorrect API key provided: hunter2. Check your settings.".to_owned(),
        ),
        (
            "(ii) quote and backslash, serialiser escapes",
            quote_key,
            serde_json::to_string(&json!({
                "error": {"message": format!("Incorrect API key provided: {quote_key}")}
            }))
            .unwrap(),
        ),
        (
            "(iii) & written as \\u0026",
            "probe&key-1234",
            r#"{"error":{"message":"Incorrect API key provided: probe\u0026key-1234"}}"#.to_owned(),
        ),
        (
            "(iv) / written as \\/",
            "probe/key-1234",
            r#"{"error":{"message":"Incorrect API key provided: probe\/key-1234"}}"#.to_owned(),
        ),
        (
            "(v) key written as a JSON number",
            "12345678901234",
            r#"{"error":{"code":12345678901234,"message":"Incorrect API key provided"}}"#
                .to_owned(),
        ),
        (
            "(vi) quote and backslash, JSON-escaped in prose",
            PROSE_ESCAPED_KEY,
            format!(
                "Incorrect API key provided: {}. Check your settings.",
                json_escaped(PROSE_ESCAPED_KEY)
            ),
        ),
    ]
}

const PROSE_ESCAPED_KEY: &str = r#"probe"prose\back-5678"#;

/// Fixture checks: each reply spells its key the way its case names.
fn assert_llm_reply_fixtures(cases: &[(&str, &str, String)]) {
    let reply = |prefix: &str| {
        cases
            .iter()
            .find(|(label, _, _)| label.starts_with(prefix))
            .map(|(_, _, reply)| reply.as_str())
            .unwrap()
    };
    let unicode = reply("(iii)");
    assert!(
        unicode.contains(r"probe\u0026key-1234") && !unicode.contains("probe&key-1234"),
        "fixture: (iii) spells the key with the six characters \\u0026: {unicode}"
    );
    let decoded: Value = serde_json::from_str(unicode).expect("fixture: (iii) is JSON");
    assert_eq!(
        decoded["error"]["message"], "Incorrect API key provided: probe&key-1234",
        "fixture: (iii) decodes to the key as sent"
    );
    let prose = reply("(vi)");
    assert!(
        serde_json::from_str::<Value>(prose).is_err(),
        "fixture: (vi) is not a JSON document: {prose}"
    );
    assert!(
        prose.contains(r#"probe\"prose\\back-5678"#) && !prose.contains(PROSE_ESCAPED_KEY),
        "fixture: (vi) holds only the JSON-escaped spelling of its key: {prose}"
    );
    println!("fixture checks passed: (iii) spells \\u0026; (vi) is prose with the escaped key");
}

async fn llm_test_blanks_the_sent_key(format: LogFormat) {
    let cases = llm_key_echo_cases();
    assert_llm_reply_fixtures(&cases);
    let replies: Vec<(String, String)> = cases
        .iter()
        .map(|(_, key, reply)| (format!("Bearer {key}"), reply.clone()))
        .collect();
    let llm = Fake::start(move |request| {
        let auth = request.header("authorization").unwrap_or_default();
        let body = replies
            .iter()
            .find(|(sent, _)| sent == auth)
            .map(|(_, reply)| reply.clone())
            .unwrap_or_else(|| "unexpected key".to_owned());
        Reply::new(401, body).with_header("content-type", "application/json")
    })
    .await;

    let data = TempDir::new().unwrap();
    let client = client();
    let mut server = Server::start_local(data.path(), data.path().join("ac107.log"), |port| {
        logged_config(port, "info", format, "")
    })
    .await;
    server.ready(&client).await;
    let token = setup_account(&server, &client).await;
    let mut tails = Vec::new();
    for (label, key, _) in &cases {
        let (status, body) = api(
            &server,
            &client,
            &token,
            reqwest::Method::PUT,
            "/config/metadata",
            Some(json!({"llmEndpoint": llm_endpoint(&llm), "llmApiKey": key, "llmModel": "m"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{label}: saving the LLM key: {body}"
        );
        let (status, body) = api(
            &server,
            &client,
            &token,
            reqwest::Method::POST,
            "/config/metadata/test/llm",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{label}: LLM test: {body}");
        tails.push(read_tail(&server, &client, &token).await);
    }
    server.stop();
    let sinks = Sinks {
        format,
        stdout: server.stdout(),
        stderr: server.stderr(),
        file: file_log_text(data.path()),
        tails,
    };

    let sent: Vec<String> = llm
        .requests()
        .iter()
        .filter_map(|r| r.header("authorization").map(str::to_owned))
        .collect();
    let expected: Vec<String> = cases
        .iter()
        .map(|(_, key, _)| format!("Bearer {key}"))
        .collect();
    assert_eq!(
        sent, expected,
        "control: the fake LLM recorded each key as sent"
    );
    println!("control passed: the fake LLM recorded Authorization: Bearer <key> for each case");
    assert_event_in_every_sink(&sinks, STARTUP_EVENT);
    for (sink, events) in sinks.events(LLM_WARNING) {
        assert_eq!(
            events.len(),
            cases.len(),
            "control: one LLM warning per case in {sink}: {events:?}"
        );
    }
    println!("control passed: one LLM warning per case in stdout, the file and the logs tail");

    let mut findings = Findings::default();
    for (sink, events) in sinks.events(LLM_WARNING) {
        for ((label, _, _), event) in cases.iter().zip(&events) {
            findings.check(
                event.contains("Incorrect API key provided") && event.contains("[REDACTED]"),
                || {
                    format!(
                        "{label} in {sink}: expected the body to keep \"Incorrect API key \
                         provided\" and show [REDACTED], got {event:?}"
                    )
                },
            );
        }
    }
    for (label, key, _) in &cases {
        findings.not_recoverable(&sinks, &format!("{label} key {key:?}"), key);
    }
    findings.json_lines_parse(&sinks);
    findings.finish(&format!("AC-107 ({} format)", format.name()));
}

#[tokio::test]
async fn llm_test_blanks_the_sent_key_in_every_spelling_in_text_sinks() {
    llm_test_blanks_the_sent_key(LogFormat::Text).await;
}

#[tokio::test]
async fn llm_test_blanks_the_sent_key_in_every_spelling_in_json_sinks() {
    llm_test_blanks_the_sent_key(LogFormat::Json).await;
}

// Rows the binary reads at startup, written while it is stopped with the real
// `SqliteDb` writers.

const FIXTURE_USER: i64 = 1;

async fn seed_work_row(db: &SqliteDb, title: &str, author: &str) -> i64 {
    db.create_work(CreateWorkDbRequest {
        user_id: FIXTURE_USER,
        title: title.to_owned(),
        author_name: author.to_owned(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(author),
        ..Default::default()
    })
    .await
    .unwrap()
    .0
    .id
}

struct ClientRow<'a> {
    implementation: DownloadClientImplementation,
    host: &'a str,
    port: u16,
    username: Option<&'a str>,
    password: Option<&'a str>,
    api_key: Option<&'a str>,
}

async fn seed_client_row(db: &SqliteDb, name: &str, row: ClientRow<'_>) -> i64 {
    db.create_download_client(CreateDownloadClientDbRequest {
        name: name.to_owned(),
        implementation: row.implementation,
        host: row.host.to_owned(),
        port: row.port,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: row.username.map(str::to_owned),
        password: row.password.map(str::to_owned),
        category: "books".to_owned(),
        download_dir: None,
        enabled: true,
        api_key: row.api_key.map(str::to_owned),
    })
    .await
    .unwrap()
    .id
}

/// A grab whose import failed before its content path was saved, as the
/// Queue page offers it for Retry.
async fn seed_failed_grab(db: &SqliteDb, work_id: i64, client_id: i64, download_id: &str) -> i64 {
    db.upsert_grab(CreateGrabDbRequest {
        user_id: FIXTURE_USER,
        work_id,
        download_client_id: client_id,
        title: "Retry Fixture Book".to_owned(),
        indexer: "fixture".to_owned(),
        guid: format!("guid-{download_id}"),
        size: None,
        download_url: "http://indexer.invalid/download/1".to_owned(),
        download_id: Some(download_id.to_owned()),
        status: GrabStatus::ImportFailed,
        media_type: Some(MediaType::Ebook),
    })
    .await
    .unwrap()
    .id
}

async fn seed_indexer_row(db: &SqliteDb, name: &str, url: String, api_key: Option<&str>) -> i64 {
    db.create_indexer(CreateIndexerDbRequest {
        name: name.to_owned(),
        protocol: "torrent".to_owned(),
        url,
        api_path: "/api".to_owned(),
        api_key: api_key.map(str::to_owned),
        categories: vec![7020],
        priority: 25,
        enable_automatic_search: true,
        enable_interactive_search: true,
        enable_rss: false,
        enabled: true,
    })
    .await
    .unwrap()
    .id
}

/// A fresh install, set up by its owner and stopped, ready for rows.
/// Returns the setup reply's session token and API key.
async fn installed_then_stopped(
    data: &Path,
    config: impl FnOnce(u16) -> String,
) -> (String, String, Server) {
    let client = client();
    let mut server = Server::start_local(data, data.join("install.log"), config).await;
    server.ready(&client).await;
    let (session, api_key) = setup_owner(&server, &client).await;
    server.stop();
    (session, api_key, server)
}

/// Restarts a stopped install after its rows were written. Backup file names
/// have one-second resolution, so a restart waits past the first start's.
async fn restart_local(data: &Path, name: &str, config: impl FnOnce(u16) -> String) -> Server {
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut server = Server::start_local(data, data.join(name), config).await;
    server.ready(&client()).await;
    server
}

async fn stored_grab(data: &Path, grab_id: i64) -> livrarr_domain::Grab {
    let pool = create_sqlite_pool(data).await.unwrap();
    let grab = SqliteDb::new(pool.clone())
        .get_grab(FIXTURE_USER, grab_id)
        .await
        .unwrap();
    pool.close().await;
    grab
}

// AC-301, AC-302, AC-303: manual import Retry asks the download client for the
// content path of a grab that has none.

/// A qBittorrent fake that logs in anyone and knows one torrent's content
/// path, answered only to a lookup by that torrent's hash.
async fn fake_qbittorrent(hash: &'static str, content_path: &'static str) -> Fake {
    Fake::start(move |request| {
        if request.target.starts_with("/api/v2/auth/login") {
            return Reply::new(200, "Ok.").with_header("set-cookie", "SID=fixture-sid; path=/");
        }
        if request.target.starts_with("/api/v2/torrents/info")
            && request.target.contains(&format!("hashes={hash}"))
        {
            return Reply::new(
                200,
                json!([{"hash": hash, "content_path": content_path}]).to_string(),
            )
            .with_header("content-type", "application/json");
        }
        Reply::new(200, "[]").with_header("content-type", "application/json")
    })
    .await
}

struct RetryOutcome {
    status: StatusCode,
    body: String,
    grab: livrarr_domain::Grab,
    logs: String,
}

/// Seeds one failed grab on `client`, restarts the binary and presses Retry.
async fn press_retry(row: ClientRow<'_>, download_id: &str) -> RetryOutcome {
    let data = TempDir::new().unwrap();
    let (_, _, _first) = installed_then_stopped(data.path(), standard_config).await;
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    let db = SqliteDb::new(pool.clone());
    let work_id = seed_work_row(&db, "Retry Fixture Book", "Retry Fixture Author").await;
    let client_id = seed_client_row(&db, "Retry Fixture Client", row).await;
    let grab_id = seed_failed_grab(&db, work_id, client_id, download_id).await;
    pool.close().await;

    let mut server = restart_local(data.path(), "retry.log", standard_config).await;
    let http = client();
    let token = login(&server, &http).await;
    let (status, body) = api(
        &server,
        &http,
        &token,
        reqwest::Method::POST,
        &format!("/grab/{grab_id}/retry"),
        None,
    )
    .await;
    server.stop();
    let grab = stored_grab(data.path(), grab_id).await;
    RetryOutcome {
        status,
        body,
        grab,
        logs: server.logs(),
    }
}

async fn retry_saves_the_qbittorrent_content_path(host: &str) {
    let hash = "ac301fixturehash0000000000000000000000ab";
    let path = "/downloads/ac301-book";
    let qbit = fake_qbittorrent(hash, path).await;
    let outcome = press_retry(
        ClientRow {
            implementation: DownloadClientImplementation::QBittorrent,
            host,
            port: qbit.port,
            username: Some("fixture-user"),
            password: Some("fixture-password"),
            api_key: None,
        },
        hash,
    )
    .await;
    println!(
        "control passed: Retry replied {} with {}",
        outcome.status, outcome.body
    );

    let mut findings = Findings::default();
    findings.check(outcome.grab.content_path.as_deref() == Some(path), || {
        format!(
            "host {host}: expected content_path {path:?}, got {:?} (grab error {:?}; reply {} {})",
            outcome.grab.content_path, outcome.grab.import_error, outcome.status, outcome.body
        )
    });
    let lookups = qbit
        .requests()
        .iter()
        .filter(|r| r.target.contains(&format!("hashes={hash}")))
        .count();
    findings.check(lookups >= 1, || {
        format!(
            "host {host}: expected the fake qBittorrent to record the lookup by hash, recorded {:?}",
            qbit.requests().iter().map(|r| r.target.clone()).collect::<Vec<_>>()
        )
    });
    findings.finish(&format!("AC-301 (qBittorrent at {host})\n{}", outcome.logs));
}

#[tokio::test]
async fn retry_reaches_a_qbittorrent_client_named_localhost() {
    retry_saves_the_qbittorrent_content_path("localhost").await;
}

#[tokio::test]
async fn retry_reaches_a_qbittorrent_client_at_a_literal_loopback_address() {
    retry_saves_the_qbittorrent_content_path("127.0.0.1").await;
}

#[tokio::test]
async fn retry_follows_a_sabnzbd_history_redirect_as_the_poller_does() {
    let nzo_id = "SABnzbd_nzo_ac302fixture";
    let storage = "/downloads/ac302-book";
    let landing = Fake::start(move |_| {
        Reply::new(
            200,
            json!({"history": {"slots": [{"nzo_id": nzo_id, "storage": storage}]}}).to_string(),
        )
        .with_header("content-type", "application/json")
    })
    .await;
    let landing_url = format!(
        "http://localhost:{}/api?mode=history&output=json",
        landing.port
    );
    let sab =
        Fake::start(move |_| Reply::new(302, "").with_header("location", landing_url.clone()))
            .await;
    let outcome = press_retry(
        ClientRow {
            implementation: DownloadClientImplementation::SABnzbd,
            host: "127.0.0.1",
            port: sab.port,
            username: None,
            password: None,
            api_key: Some("ac302sabapikey71c"),
        },
        nzo_id,
    )
    .await;

    let history = sab
        .requests()
        .iter()
        .filter(|r| r.target.contains("mode=history") && r.target.contains("apikey="))
        .count();
    assert!(
        history >= 1,
        "control: the first SABnzbd fake recorded the history request: {:?}\n{}",
        sab.requests(),
        outcome.logs
    );
    println!(
        "control passed: the first SABnzbd fake recorded the history request and answered 302"
    );

    let mut findings = Findings::default();
    findings.check(
        outcome.grab.content_path.as_deref() == Some(storage),
        || {
            format!(
                "expected content_path {storage:?} from the redirect's landing server, got {:?} \
             (grab error {:?}; reply {} {})",
                outcome.grab.content_path, outcome.grab.import_error, outcome.status, outcome.body
            )
        },
    );
    findings.check(!landing.requests().is_empty(), || {
        "expected the redirect's landing server to record the history request".to_owned()
    });
    findings.finish(&format!("AC-302\n{}", outcome.logs));
}

#[tokio::test]
async fn retry_against_a_closed_qbittorrent_port_reports_the_login_failure() {
    let outcome = press_retry(
        ClientRow {
            implementation: DownloadClientImplementation::QBittorrent,
            host: "localhost",
            port: closed_port(),
            username: Some("fixture-user"),
            password: Some("fixture-password"),
            api_key: None,
        },
        "ac303fixturehash000000000000000000000000",
    )
    .await;
    assert_eq!(
        outcome.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}\n{}",
        outcome.body,
        outcome.logs
    );
    assert_eq!(outcome.grab.status, GrabStatus::ImportFailed);
    let error = outcome.grab.import_error.unwrap_or_default();
    assert!(
        error.contains("qBittorrent login failed"),
        "the grab's error names the login failure: {error:?}"
    );
}

// AC-106: a release-search warning keeps today's text; only the log copies are
// cleansed.

#[tokio::test]
async fn release_search_warning_keeps_its_returned_text() {
    let item = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel><title>fixture</title>
<item><title>Release Fixture Author - Release Fixture Book (epub)</title><guid>ac106-guid-1</guid>
<link>http://indexer.invalid/download/1</link><size>1048576</size>
<enclosure url="http://indexer.invalid/download/1" length="1048576" type="application/x-bittorrent"/>
<torznab:attr name="seeders" value="3"/></item></channel></rss>"#;
    let error = r#"<?xml version="1.0" encoding="UTF-8"?><error code="100" description="access_token=abc"/>"#;
    let good =
        Fake::start(move |_| Reply::new(200, item).with_header("content-type", "application/xml"))
            .await;
    let failing =
        Fake::start(move |_| Reply::new(200, error).with_header("content-type", "application/xml"))
            .await;

    let data = TempDir::new().unwrap();
    let (_, _, _first) = installed_then_stopped(data.path(), standard_config).await;
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    let db = SqliteDb::new(pool.clone());
    let work_id = seed_work_row(&db, "Release Fixture Book", "Release Fixture Author").await;
    seed_indexer_row(
        &db,
        "Fixture Good",
        format!("http://127.0.0.1:{}", good.port),
        None,
    )
    .await;
    seed_indexer_row(
        &db,
        "Fixture Failing",
        format!("http://127.0.0.1:{}", failing.port),
        None,
    )
    .await;
    pool.close().await;

    let mut server = restart_local(data.path(), "release.log", standard_config).await;
    let http = client();
    let token = login(&server, &http).await;
    let (status, body) = api(
        &server,
        &http,
        &token,
        reqwest::Method::GET,
        &format!("/release?workId={work_id}"),
        None,
    )
    .await;
    server.stop();
    assert_eq!(status, StatusCode::OK, "{body}\n{}", server.logs());

    for (name, fake) in [("good", &good), ("failing", &failing)] {
        let searches = fake
            .requests()
            .iter()
            .filter(|r| r.target.contains("t=search"))
            .count();
        assert_eq!(
            searches,
            1,
            "control: the {name} fake recorded one search request: {:?}",
            fake.requests()
        );
    }
    println!("control passed: each fake indexer recorded one search request");

    let body: Value = serde_json::from_str(&body).unwrap();
    let warning = body["warnings"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["indexer"] == "Fixture Failing"))
        .unwrap_or_else(|| panic!("a warning for the failing indexer: {body}"));
    assert_eq!(warning["error"], "error 100: access_token=abc");
}

// AC-101: no stored secret, session token or API key reaches a log sink in
// the flows that handle them. Only the LLM test writes one today.

const QBIT_PASSWORD: &str = "ac101-qbit-password-7f3a";
const SAB_API_KEY: &str = "ac101sabapikey9d2c71";
const INDEXER_API_KEY: &str = "ac101indexerkey4b8e05";
const PROWLARR_API_KEY: &str = "ac101prowlarrkey6a1f93";
const HARDCOVER_TOKEN: &str = "ac101hardcovertoken2e7d";
const LLM_API_KEY: &str = "sk-ac101-llm-key-5c9b28";
const GOOGLE_BOOKS_KEY: &str = "ac101googlebookskey8f4a";
const SMTP_PASSWORD: &str = "ac101-smtp-password-3d6e";
const QBIT_USERNAME: &str = "ac101-qbit-user";

fn assert_status(flow: &str, status: StatusCode, expected: StatusCode, body: &str) {
    assert_eq!(
        status, expected,
        "control: {flow} replies {expected}: {body}"
    );
    println!("control passed: {flow} replied {status}");
}

async fn no_secret_reaches_a_log_sink(format: LogFormat) {
    let llm = Fake::start(|request| {
        let key = request
            .header("authorization")
            .unwrap_or_default()
            .trim_start_matches("Bearer ")
            .to_owned();
        Reply::new(
            401,
            json!({"error": {"message": format!("Incorrect API key provided: {key}")}}).to_string(),
        )
        .with_header("content-type", "application/json")
    })
    .await;
    let indexer =
        Fake::start(|request| Reply::new(401, format!("Invalid API key in {}", request.target)))
            .await;
    let prowlarr = Fake::start(|request| {
        Reply::new(
            500,
            format!(
                "Unknown key {}",
                request.header("x-api-key").unwrap_or_default()
            ),
        )
    })
    .await;
    let qbit = Fake::start(|_| Reply::new(200, "Fails.")).await;
    let sab = Fake::start(|request| {
        let key = request
            .target
            .split("apikey=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or_default()
            .to_owned();
        Reply::new(
            200,
            json!({"error": format!("API Key Incorrect: {key}")}).to_string(),
        )
        .with_header("content-type", "application/json")
    })
    .await;
    let config = |port| logged_config(port, "debug", format, "");

    let data = TempDir::new().unwrap();
    let (setup_session, api_key, first) = installed_then_stopped(data.path(), config).await;
    let pool = create_sqlite_pool(data.path()).await.unwrap();
    let db = SqliteDb::new(pool.clone());
    let work_id = seed_work_row(&db, "Secrets Fixture Book", "Secrets Fixture Author").await;
    let closed_sab = seed_client_row(
        &db,
        "Closed SABnzbd",
        ClientRow {
            implementation: DownloadClientImplementation::SABnzbd,
            host: "127.0.0.1",
            port: closed_port(),
            username: None,
            password: None,
            api_key: Some(SAB_API_KEY),
        },
    )
    .await;
    seed_client_row(
        &db,
        "Fixture qBittorrent",
        ClientRow {
            implementation: DownloadClientImplementation::QBittorrent,
            host: "127.0.0.1",
            port: qbit.port,
            username: Some(QBIT_USERNAME),
            password: Some(QBIT_PASSWORD),
            api_key: None,
        },
    )
    .await;
    seed_indexer_row(
        &db,
        "Fixture Indexer",
        format!("http://127.0.0.1:{}", indexer.port),
        Some(INDEXER_API_KEY),
    )
    .await;
    let grab_id = seed_failed_grab(&db, work_id, closed_sab, "SABnzbd_nzo_ac101").await;
    pool.close().await;

    let mut server = restart_local(data.path(), "flows.log", config).await;
    let http = client();
    let mut tails = Vec::new();

    // (f) login, then requests with the session token and with X-Api-Key.
    let session = login(&server, &http).await;
    for (name, header) in [
        (
            "session token",
            ("Authorization", format!("Bearer {session}")),
        ),
        ("X-Api-Key", ("X-Api-Key", api_key.clone())),
    ] {
        let response = http
            .get(format!("{}/auth/me", server.base))
            .header(header.0, header.1)
            .send()
            .await
            .unwrap();
        assert_status(
            &format!("(f) GET /auth/me with the {name}"),
            response.status(),
            StatusCode::OK,
            "",
        );
    }
    tails.push(read_tail(&server, &http, &session).await);

    // Every stored secret of the settings holds a distinct value.
    let saves = [
        (
            "/config/metadata",
            json!({
                "hardcoverApiToken": HARDCOVER_TOKEN,
                "llmEndpoint": llm_endpoint(&llm),
                "llmApiKey": LLM_API_KEY,
                "llmModel": "m",
                "googleBooksApiKey": GOOGLE_BOOKS_KEY,
            }),
        ),
        (
            "/config/prowlarr",
            json!({
                "url": format!("http://127.0.0.1:{}", prowlarr.port),
                "apiKey": PROWLARR_API_KEY,
                "enabled": true,
            }),
        ),
        (
            "/config/email",
            json!({
                "enabled": true,
                "smtpHost": "127.0.0.1",
                "smtpPort": closed_port(),
                "encryption": "none",
                "username": "ac101-smtp-user",
                "password": SMTP_PASSWORD,
                "fromAddress": "livrarr@example.com",
                "recipientEmail": "reader@example.com",
            }),
        ),
    ];
    for (path, body) in saves {
        let (status, reply) = api(
            &server,
            &http,
            &session,
            reqwest::Method::PUT,
            path,
            Some(body),
        )
        .await;
        assert_status(&format!("PUT {path}"), status, StatusCode::OK, &reply);
    }
    tails.push(read_tail(&server, &http, &session).await);

    // (a) the LLM connection test against a fake that echoes the key.
    let (status, reply) = api(
        &server,
        &http,
        &session,
        reqwest::Method::POST,
        "/config/metadata/test/llm",
        None,
    )
    .await;
    assert_status("(a) LLM test", status, StatusCode::BAD_GATEWAY, &reply);
    tails.push(read_tail(&server, &http, &session).await);

    // (b) indexer Test and Prowlarr import against fakes that echo the key.
    let (status, reply) = api(
        &server,
        &http,
        &session,
        reqwest::Method::POST,
        "/indexer/test",
        Some(json!({
            "url": format!("http://127.0.0.1:{}", indexer.port),
            "apiPath": "/api",
            "apiKey": INDEXER_API_KEY,
        })),
    )
    .await;
    assert_status("(b) indexer Test", status, StatusCode::BAD_GATEWAY, &reply);
    let (status, reply) = api(
        &server,
        &http,
        &session,
        reqwest::Method::POST,
        "/indexer/import/prowlarr",
        Some(json!({"url": "", "apiKey": ""})),
    )
    .await;
    assert_status(
        "(b) Prowlarr import",
        status,
        StatusCode::BAD_GATEWAY,
        &reply,
    );
    tails.push(read_tail(&server, &http, &session).await);

    // (c) qBittorrent and SABnzbd Test.
    let client_test = |implementation: &str, port: u16| {
        json!({
            "name": "Test",
            "implementation": implementation,
            "host": "127.0.0.1",
            "port": port,
            "useSsl": false,
            "skipSslValidation": false,
            "urlBase": null,
            "username": QBIT_USERNAME,
            "password": QBIT_PASSWORD,
            "category": "books",
            "downloadDir": null,
            "enabled": true,
            "apiKey": SAB_API_KEY,
        })
    };
    for (name, body) in [
        ("qBittorrent", client_test("qBittorrent", qbit.port)),
        ("SABnzbd", client_test("sabnzbd", sab.port)),
    ] {
        let (status, reply) = api(
            &server,
            &http,
            &session,
            reqwest::Method::POST,
            "/downloadclient/test",
            Some(body),
        )
        .await;
        assert_status(
            &format!("(c) {name} Test"),
            status,
            StatusCode::BAD_GATEWAY,
            &reply,
        );
    }
    tails.push(read_tail(&server, &http, &session).await);

    // (d) Retry against a SABnzbd client whose port refuses connection.
    let (status, reply) = api(
        &server,
        &http,
        &session,
        reqwest::Method::POST,
        &format!("/grab/{grab_id}/retry"),
        None,
    )
    .await;
    assert_status(
        "(d) Retry",
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        &reply,
    );
    tails.push(read_tail(&server, &http, &session).await);

    // (e) Hardcover Test through the refusing proxy.
    let (status, reply) = api(
        &server,
        &http,
        &session,
        reqwest::Method::POST,
        "/config/metadata/test/hardcover",
        None,
    )
    .await;
    assert_status(
        "(e) Hardcover Test",
        status,
        StatusCode::BAD_GATEWAY,
        &reply,
    );
    assert!(
        reply.contains("Hardcover connection failed"),
        "control: (e) {reply}"
    );
    tails.push(read_tail(&server, &http, &session).await);

    // (g) e-mail Test to a closed SMTP port.
    let (status, reply) = api(
        &server,
        &http,
        &session,
        reqwest::Method::POST,
        "/config/email/test",
        None,
    )
    .await;
    assert_status("(g) e-mail Test", status, StatusCode::BAD_REQUEST, &reply);
    tails.push(read_tail(&server, &http, &session).await);
    server.stop();

    let sinks = Sinks {
        format,
        stdout: format!("{}{}", first.stdout(), server.stdout()),
        stderr: format!("{}{}", first.stderr(), server.stderr()),
        file: file_log_text(data.path()),
        tails,
    };

    // Reachability controls.
    assert_event_in_every_sink(&sinks, STARTUP_EVENT);
    assert_event_in_every_sink(&sinks, "login successful");
    assert!(
        llm.requests()
            .iter()
            .any(|r| r.header("authorization") == Some(&format!("Bearer {LLM_API_KEY}"))),
        "control: (a) the fake LLM recorded the key"
    );
    assert_event_in_every_sink(&sinks, LLM_WARNING);
    assert!(
        indexer
            .requests()
            .iter()
            .any(|r| r.target.contains(&format!("apikey={INDEXER_API_KEY}"))),
        "control: (b) the fake indexer recorded the apikey query: {:?}",
        indexer.requests()
    );
    assert!(
        prowlarr
            .requests()
            .iter()
            .any(|r| r.header("x-api-key") == Some(PROWLARR_API_KEY)),
        "control: (b) the fake Prowlarr recorded the X-Api-Key header"
    );
    assert!(
        qbit.requests().iter().any(|r| r.method == "POST"
            && r.target.starts_with("/api/v2/auth/login")
            && r.body.contains(&format!("password={QBIT_PASSWORD}"))),
        "control: (c) the fake qBittorrent recorded the login form: {:?}",
        qbit.requests()
    );
    assert!(
        sab.requests()
            .iter()
            .any(|r| r.target.contains(&format!("apikey={SAB_API_KEY}"))),
        "control: (c) the fake SABnzbd recorded the apikey query"
    );
    println!("control passed: every fake recorded the request carrying its secret");
    for (sink, events) in sinks.events("internal error:") {
        assert!(
            events
                .iter()
                .any(|e| e.contains("SABnzbd history request failed")),
            "control: (d) the internal error event names the SABnzbd history request in {sink}: \
             {events:?}"
        );
    }
    println!(
        "control passed: (d) the internal error event is in stdout, the file and the logs tail"
    );

    let mut findings = Findings::default();
    for (sink, events) in sinks.events(LLM_WARNING) {
        findings.check(events.iter().all(|e| e.contains("[REDACTED]")), || {
            format!(
                "(a, red) LLM warning in {sink}: expected [REDACTED] in its body, got {events:?}"
            )
        });
    }
    findings.not_recoverable(&sinks, "(a, red) LLM key", LLM_API_KEY);
    for (label, secret) in [
        ("(guard) download-client password", QBIT_PASSWORD),
        ("(guard) download-client API key", SAB_API_KEY),
        ("(guard) indexer API key", INDEXER_API_KEY),
        ("(guard) Prowlarr API key", PROWLARR_API_KEY),
        ("(guard) Hardcover token", HARDCOVER_TOKEN),
        ("(guard) Google Books key", GOOGLE_BOOKS_KEY),
        ("(guard) SMTP password", SMTP_PASSWORD),
        ("(guard) user password", PASSWORD),
        ("(guard) setup session token", setup_session.as_str()),
        ("(guard) login session token", session.as_str()),
        ("(guard) user API key", api_key.as_str()),
    ] {
        findings.not_recoverable(&sinks, label, secret);
    }
    findings.json_lines_parse(&sinks);
    findings.finish(&format!("AC-101 ({} format)", format.name()));
}

#[tokio::test]
async fn no_secret_reaches_a_text_log_sink_in_the_flows_that_handle_one() {
    no_secret_reaches_a_log_sink(LogFormat::Text).await;
}

#[tokio::test]
async fn no_secret_reaches_a_json_log_sink_in_the_flows_that_handle_one() {
    no_secret_reaches_a_log_sink(LogFormat::Json).await;
}
