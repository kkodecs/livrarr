//! Identity upgrade in place (spec-identity-upgrade-inplace.md AC-001..AC-016).
//!
//! Every upgrade case starts from the committed alpha6 library: a migration-073
//! database created and last opened by the alpha6 binary itself, so it carries
//! alpha6's runtime `idx_works_identity` and runtime metadata. Its rows are
//! synthetic (scripts/gen-alpha6-upgrade-fixture.py). Startup is always the
//! Cargo-built `livrarr` binary as a child process. Faults are SQLite triggers in
//! the database: the in-process failpoints are thread-local and never reach the
//! child. No case plants a ready report or the active marker, and no case uses
//! the empty-to-head test database helper, which would skip migration 091.
//!
//! Setups that run production code in process use its real entry points: SQLx's
//! migrator over the real historical files, the manual cutover ceremony, the
//! startup repair functions and `run_migrations`. Six setups write state
//! directly, each standing in for an actor the harness cannot run: SQL edits and
//! deletions at migration 073 are what an alpha6 user's edit or delete writes; a
//! leftover `.partial` file is what a crash during the copy leaves behind; a
//! pending file naming a recorded copy is what a crash between recording that
//! copy and removing the pending file leaves behind (nothing observable lies
//! between the two to freeze the child on); extra
//! `pre-migrate` files are copies a user or an earlier release left in the data
//! directory; a foreign `startup_generation` value is what an older release of
//! this code leaves behind (one binary cannot be two releases); and a cleared
//! Goodreads cover-repair marker beside a machine Goodreads cover is the library
//! as a release before that repair left it.
//!
//! The title-policy and dedup-residue repairs are off while work merging is
//! unavailable and never stamp their markers, so "the repair ran" is asserted
//! only for enabled repairs.
//!
//! Process interruptions are real: the child is SIGKILLed, so no error handler
//! runs. Two barriers expose the boundaries without supplying any outcome: the
//! test holds SQLite's write lock so the child blocks at its first database write
//! (after the copy is published, before anything is recorded), and a large
//! library makes the copy slow enough to kill while its work file is written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use livrarr_db::identity_layer::IdentityCutoverMode;
use livrarr_db::pool::{create_sqlite_pool, run_migrations};
use livrarr_db::sqlite::SqliteDb;
use livrarr_domain::normalization::{normalize_asin, normalize_gr_key, normalize_isbn13, AsinNorm};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha384};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Row, SqlitePool};
use tempfile::TempDir;

const FIXTURE_SQL: &str = include_str!("fixtures/alpha6_library_073/livrarr.sql");
const ALPHA6_MIGRATIONS: &str = include_str!("fixtures/alpha6_migrations_073.sha384");
const FIXTURE_COVERS: [(&str, &[u8]); 4] = [
    (
        "1/1.jpg",
        include_bytes!("fixtures/alpha6_library_073/covers/1/1.jpg"),
    ),
    (
        "1/1_audio.jpg",
        include_bytes!("fixtures/alpha6_library_073/covers/1/1_audio.jpg"),
    ),
    (
        "1/7_audio.jpg",
        include_bytes!("fixtures/alpha6_library_073/covers/1/7_audio.jpg"),
    ),
    (
        "1/8_audio.jpg",
        include_bytes!("fixtures/alpha6_library_073/covers/1/8_audio.jpg"),
    ),
];
const USERNAME: &str = "alpha6-admin";
const PASSWORD: &str = "alpha6-fixture-password";
const FIXTURE_BOOKS: i64 = 20;
const ALPHA6_LAST_MIGRATION: i64 = 73;
const LAST_PUBLISHED_MIGRATION: i64 = 90;
const UPGRADE_MIGRATION: i64 = 91;
const COPY_PREFIX: &str = "livrarr.db.pre-migrate-";
/// Names the copy an upgrade attempt is publishing until its record commits.
const PENDING_COPY_FILE: &str = "livrarr.db.upgrade-copy-pending";
const UPGRADE_ACTOR: &str = "identity-upgrade";
const AUTHORITY_MARKER: &str = "identity_authority_v2";
const COVER_REPAIR_MARKER: &str = "identity_round15_gr_cover_reselect";
const TITLE_REPAIR_MARKER: &str = "identity_title_policy_generation";
const LAST_SYNC_REPAIR_MARKER: &str = "identity_round21_goodreads_book_namespace_heal";
const SYNC_REPAIR_MARKERS: [&str; 7] = [
    "identity_review_dismissal_adoption_v1",
    "identity_sweep_heal_generation",
    "identity_dedup_residue_heal_generation",
    "identity_round10_residue_heal_generation",
    "identity_round11_attempt_reheal",
    "identity_round15_search_ledger_reset",
    LAST_SYNC_REPAIR_MARKER,
];
/// Repairs that return before reading or stamping their markers while work
/// merging is unavailable (`heal_identity_title_policy`,
/// `heal_identity_dedup_residue`).
const MERGE_GATED_REPAIR_MARKERS: [&str; 2] = [
    TITLE_REPAIR_MARKER,
    "identity_dedup_residue_heal_generation",
];

/// True when production runs the repair that stamps `marker`.
fn repair_enabled(marker: &str) -> bool {
    livrarr_domain::identity_layer::WORK_MERGING_AVAILABLE
        || !MERGE_GATED_REPAIR_MARKERS.contains(&marker)
}

/// The three ways the background Goodreads cover pass ends (jobs/cover_startup.rs).
const COVER_PASS_ENDS: [&str; 3] = [
    "identity round-15 Goodreads cover reselect complete",
    "identity round-15 Goodreads cover reselect partial",
    "identity round-15 Goodreads cover reselect failed",
];
const COVER_CHOICES_KEY: &str = "upgrade_audiobook_cover_user_choices";
const WORK_WITH_USER_AUDIOBOOK_COVER: i64 = 7;
const WORK_WITH_MACHINE_GOODREADS_COVER: i64 = 8;

// ── child-process server ────────────────────────────────────────────────────

// Provider clients honour reqwest's proxy environment; this local endpoint
// refuses every request, so no case reaches a real provider.
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
        std::fs::write(
            data.join("config.toml"),
            format!(
                "[server]\nbind_address = '127.0.0.1'\nport = {port}\n\
                 [log]\nlevel = 'info'\n\
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
        strip_ansi(&std::fs::read_to_string(&self.log).unwrap_or_default())
    }

    async fn ready(&mut self, client: &Client, expectation: &str) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!(
                    "{expectation}: the server exited before serving ({status}):\n{}",
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
                "{expectation}: the server did not serve within 60 s:\n{}",
                self.logs()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn observe_refusal(&mut self, client: &Client) -> (Option<ExitStatus>, bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
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

    /// The Goodreads cover repair runs as a background pass after serving
    /// starts; its end is observable only in the log.
    async fn wait_for_cover_pass(&mut self, expectation: &str) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while !COVER_PASS_ENDS.iter().any(|end| self.logs().contains(end)) {
            assert!(
                Instant::now() < deadline,
                "{expectation}: the Goodreads cover startup pass did not finish:\n{}",
                self.logs()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
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

fn strip_ansi(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            clean.push(c);
        }
    }
    clean
}

async fn read_work(server: &Server, client: &Client, work_id: i64) -> Value {
    let login = client
        .post(format!("{}/auth/login", server.base))
        .json(&json!({"username": USERNAME, "password": PASSWORD, "rememberMe": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        login.status(),
        StatusCode::OK,
        "the alpha6 account must still log in after the upgrade:\n{}",
        server.logs()
    );
    let token = login.json::<Value>().await.unwrap()["token"]
        .as_str()
        .expect("login session token")
        .to_owned();
    let response = client
        .get(format!("{}/work/{work_id}", server.base))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "authenticated work read");
    let body: Value = response.json().await.unwrap();
    if body.get("work").is_some_and(Value::is_object) {
        body["work"].clone()
    } else {
        body
    }
}

// ── the library under test ──────────────────────────────────────────────────

struct Library {
    data: TempDir,
    logs: TempDir,
    starts: AtomicUsize,
}

impl Library {
    fn empty() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            logs: TempDir::new().unwrap(),
            starts: AtomicUsize::new(0),
        }
    }

    /// A fresh copy of the committed alpha6 library and its cover files.
    async fn alpha6() -> Self {
        let library = Self::empty();
        library.write_fixture().await;
        library
    }

    fn dir(&self) -> &Path {
        self.data.path()
    }

    fn db(&self) -> PathBuf {
        self.dir().join("livrarr.db")
    }

    async fn write_fixture(&self) {
        let mut connection = SqliteConnectOptions::new()
            .filename(self.db())
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        sqlx::raw_sql(FIXTURE_SQL)
            .execute(&mut connection)
            .await
            .expect("load the committed alpha6 library");
        connection.close().await.unwrap();
        for (relative, bytes) in FIXTURE_COVERS {
            let path = self.dir().join("covers").join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    async fn pool(&self) -> SqlitePool {
        create_sqlite_pool(self.dir()).await.unwrap()
    }

    async fn execute(&self, sql: &str) {
        let pool = self.pool().await;
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        pool.close().await;
    }

    async fn start(&self) -> Server {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        if start > 1 {
            // Copy names carry a one-second timestamp; never restart within it.
            tokio::time::sleep(Duration::from_millis(1100)).await;
        }
        Server::start(
            self.dir(),
            self.logs.path().join(format!("start-{start}.log")),
        )
        .await
    }

    async fn serve(&self, client: &Client, expectation: &str) -> Server {
        let mut server = self.start().await;
        server.ready(client, expectation).await;
        server
    }

    /// Serve, let the background cover pass finish, stop; returns the log.
    async fn serve_through_cover_pass(&self, client: &Client, expectation: &str) -> String {
        let mut server = self.serve(client, expectation).await;
        server.wait_for_cover_pass(expectation).await;
        server.stop();
        server.logs()
    }

    /// Start, require an unsuccessful exit without serving; returns the log.
    async fn refuse(&self, client: &Client, expectation: &str) -> String {
        let mut server = self.start().await;
        let (status, served) = server.observe_refusal(client).await;
        server.stop();
        let log = server.logs();
        assert!(
            !served && status.is_some_and(|status| !status.success()),
            "{expectation}: the start must be refused without serving; status={status:?}, served={served}\n{log}"
        );
        log
    }

    /// Completed copies that are not in `protected`: the current attempt's.
    fn attempt_copies(&self, protected: &BTreeSet<String>) -> Vec<String> {
        self.copies()
            .iter()
            .map(|path| name_of(path))
            .filter(|name| !protected.contains(name))
            .collect()
    }

    /// Boundary: after publication, before the association transaction begins.
    /// The test holds SQLite's write lock, so the child's first database write
    /// blocks (its pool waits up to 5 s); REQ-005 orders "VACUUM INTO .partial,
    /// rename, then record", so that write follows publication. Acknowledgement:
    /// with `require_new`, a copy that did not exist before this start is
    /// complete on disk; otherwise the child stays blocked with the attempt's
    /// copy on disk. No acknowledgement within the deadline is a setup failure.
    /// Returns the attempt's copy seen at the kill.
    async fn kill_after_publication(
        &self,
        protected: &BTreeSet<String>,
        require_new: bool,
    ) -> String {
        let pool = self.pool().await;
        let live_migration = max_migration(&pool).await;
        let mut lock = pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *lock)
            .await
            .unwrap();
        let before: BTreeSet<String> = self.copies().iter().map(|path| name_of(path)).collect();
        let mut server = self.start().await;
        let started = Instant::now();
        let observed = loop {
            let new: Vec<String> = self
                .attempt_copies(protected)
                .into_iter()
                .filter(|name| !before.contains(name))
                .collect();
            if let Some(name) = new.first() {
                break name.clone();
            }
            if !require_new && started.elapsed() > Duration::from_millis(1500) {
                let attempt = self.attempt_copies(protected);
                assert_eq!(
                    attempt.len(),
                    1,
                    "setup: the blocked attempt has its copy on disk: {attempt:?}"
                );
                break attempt[0].clone();
            }
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "setup: the child exited ({status}) before publishing a copy:\n{}",
                    server.logs()
                );
            }
            assert!(
                started.elapsed() < Duration::from_millis(4000),
                "setup: no copy was published before the child's first database write:\n{}",
                server.logs()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "setup: the child must still be blocked when it is killed:\n{}",
            server.logs()
        );
        server.child.kill().unwrap();
        server.stop();
        sqlx::query("ROLLBACK").execute(&mut *lock).await.unwrap();
        drop(lock);
        pool.close().await;
        let (_copy_dir, copy) = open_copy(&self.dir().join(&observed)).await;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&copy)
            .await
            .unwrap();
        assert_eq!(integrity, "ok", "the published copy {observed} is complete");
        assert_eq!(
            max_migration(&copy).await,
            live_migration,
            "the published copy {observed} is this library"
        );
        copy.close().await;
        observed
    }

    /// Boundary: inside the association transaction, before it commits. A
    /// trigger stalls the `upgrade_snapshot` write indefinitely (it supplies no
    /// value and cannot commit). Acknowledgement: the attempt's copy is on disk
    /// and the child holds SQLite's write lock without a break for 600 ms, which
    /// only the stalled write can do. No acknowledgement is a setup failure.
    async fn kill_before_association_commit(&self, protected: &BTreeSet<String>) -> String {
        stall_meta_write(self, "upgrade_snapshot").await;
        let mut probe = SqliteConnectOptions::new()
            .filename(self.db())
            .busy_timeout(Duration::ZERO)
            .connect()
            .await
            .unwrap();
        let mut server = self.start().await;
        let started = Instant::now();
        let mut held_since: Option<Instant> = None;
        let observed = loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "setup: the child exited ({status}) before reaching the association write:\n{}",
                    server.logs()
                );
            }
            let held = match sqlx::query("BEGIN IMMEDIATE").execute(&mut probe).await {
                Ok(_) => {
                    sqlx::query("ROLLBACK").execute(&mut probe).await.unwrap();
                    false
                }
                Err(_) => true,
            };
            let copy_on_disk = self.attempt_copies(protected).len() == 1;
            held_since = if held && copy_on_disk {
                held_since.or(Some(Instant::now()))
            } else {
                None
            };
            if held_since.is_some_and(|since| since.elapsed() > Duration::from_millis(600)) {
                break self.attempt_copies(protected)[0].clone();
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "setup: the child never stalled in the association write after publishing:\n{}",
                server.logs()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        server.child.kill().unwrap();
        server.stop();
        probe.close().await.unwrap();
        clear_faults(self).await;
        observed
    }

    /// Boundary: the copy is complete in its `.partial` work file and not yet
    /// published. The library must be large enough that copying and syncing
    /// take a few hundred milliseconds. While the attempt's pending file and
    /// one work file exist and no new copy is published, the child is frozen
    /// (SIGSTOP) and the freeze is acknowledged (stopped state in
    /// `/proc/<pid>/stat`) before anything is inspected. The frozen state must
    /// then show that phase afresh, a work file that is a complete copy of this
    /// library, and no `upgrade_snapshot` record; only then is the child
    /// killed. A work file still being written is resumed (SIGCONT) and frozen
    /// again. Returns false (a miss the caller retries on a fresh library) when
    /// a copy is published first or the phase is never caught.
    async fn kill_before_publication(&self) -> bool {
        let pool = self.pool().await;
        let expected = (
            seeded_library_signature(&pool).await,
            count(&pool, "SELECT COUNT(*) FROM provider_response_cache").await,
        );
        pool.close().await;
        let before: BTreeSet<PathBuf> = self.files_with_prefix().into_iter().collect();
        let pending = self.dir().join(PENDING_COPY_FILE);
        let mut server = self.start().await;
        let pid = server.child.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut freezes = 0;
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "setup: the child exited ({status}) before publishing a copy:\n{}",
                    server.logs()
                );
            }
            let published = self
                .copies()
                .into_iter()
                .any(|path| !before.contains(&path));
            if published || Instant::now() >= deadline {
                eprintln!(
                    "before-publication barrier missed after {freezes} freezes (published={published})"
                );
                server.stop();
                return false;
            }
            if !(pending.exists() && self.partials().len() == 1) {
                tokio::time::sleep(Duration::from_millis(1)).await;
                continue;
            }
            // SAFETY: signals the child this test spawned and still owns.
            unsafe { libc::kill(pid, libc::SIGSTOP) };
            freezes += 1;
            wait_until_stopped(&mut server).await;
            let partials = self.partials();
            let published = self
                .copies()
                .into_iter()
                .any(|path| !before.contains(&path));
            let complete = !published
                && pending.exists()
                && partials.len() == 1
                && is_complete_copy(&partials[0], &expected).await;
            if !complete {
                // SAFETY: as above.
                unsafe { libc::kill(pid, libc::SIGCONT) };
                tokio::time::sleep(Duration::from_millis(2)).await;
                continue;
            }
            let pool = self.pool().await;
            let record = meta(&pool, "upgrade_snapshot").await;
            pool.close().await;
            assert_eq!(record, None, "the frozen attempt has recorded no copy");
            let partial = name_of(&partials[0]);
            assert_eq!(
                std::fs::read_to_string(&pending).unwrap(),
                partial.trim_end_matches(".partial"),
                "the pending file names the copy being published"
            );
            eprintln!("before-publication barrier held after {freezes} freezes");
            server.stop();
            return true;
        }
    }

    fn files_with_prefix(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.dir())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_string_lossy();
                name.starts_with(COPY_PREFIX)
                    && !name.ends_with("-wal")
                    && !name.ends_with("-shm")
                    && !name.ends_with("-journal")
            })
            .collect();
        files.sort();
        files
    }

    /// Completed pre-upgrade copies (never the `.partial` work files).
    fn copies(&self) -> Vec<PathBuf> {
        self.files_with_prefix()
            .into_iter()
            .filter(|path| !name_of(path).ends_with(".partial"))
            .collect()
    }

    fn partials(&self) -> Vec<PathBuf> {
        self.files_with_prefix()
            .into_iter()
            .filter(|path| name_of(path).ends_with(".partial"))
            .collect()
    }

    fn copy_checksums(&self) -> BTreeMap<String, String> {
        self.copies()
            .iter()
            .map(|path| (name_of(path), sha256_file(path)))
            .collect()
    }

    fn plant(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.dir().join(name), bytes).unwrap();
    }

    /// Replace the live database with a copy, as a user restoring a backup does.
    fn restore(&self, copy: &Path) {
        for sidecar in ["livrarr.db-wal", "livrarr.db-shm"] {
            let _ = std::fs::remove_file(self.dir().join(sidecar));
        }
        std::fs::copy(copy, self.db()).unwrap();
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn sha256_file(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

/// The child's scheduler state from `/proc/<pid>/stat` ('T' when stopped).
fn process_state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.trim_start().chars().next()
}

/// Acknowledge a SIGSTOP: wait until the child is reported stopped.
async fn wait_until_stopped(server: &mut Server) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(process_state(server.child.id()), Some('T' | 't')) {
        if let Some(status) = server.child.try_wait().unwrap() {
            panic!(
                "setup: the child exited ({status}) instead of stopping:\n{}",
                server.logs()
            );
        }
        assert!(
            Instant::now() < deadline,
            "setup: the child never acknowledged SIGSTOP"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Whether a frozen work file is a complete copy of the library: its header
/// declares exactly the file's length, it passes quick_check, and it holds
/// the seeded ledger and works and every provider-cache row. Read in place
/// with `immutable=1`, which writes nothing beside it.
async fn is_complete_copy(path: &Path, expected: &(Vec<String>, i64)) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if bytes.len() < 100 || !bytes.starts_with(b"SQLite format 3\0") {
        return false;
    }
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let page_size = match u16::from_be_bytes([bytes[16], bytes[17]]) {
        1 => 65_536,
        size => u64::from(size),
    };
    if word(92) != word(24) || u64::from(word(28)) * page_size != bytes.len() as u64 {
        return false;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .immutable(true);
    let Ok(copy) = SqlitePool::connect_with(options).await else {
        return false;
    };
    let quick: Result<String, _> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&copy)
        .await;
    let cache: Result<i64, _> = sqlx::query_scalar("SELECT COUNT(*) FROM provider_response_cache")
        .fetch_one(&copy)
        .await;
    let complete = quick.is_ok_and(|result| result == "ok")
        && cache.is_ok_and(|rows| rows == expected.1)
        && seeded_library_signature(&copy).await == expected.0;
    copy.close().await;
    complete
}

/// Open a pre-upgrade copy without writing beside it in the data directory.
async fn open_copy(copy: &Path) -> (TempDir, SqlitePool) {
    let dir = TempDir::new().unwrap();
    std::fs::copy(copy, dir.path().join("livrarr.db")).unwrap();
    let pool = create_sqlite_pool(dir.path()).await.unwrap();
    (dir, pool)
}

// ── database-level faults (they reach the child binary) ─────────────────────

async fn fail_migration(library: &Library, version: i64) {
    library
        .execute(&format!(
            "CREATE TRIGGER upgrade_test_fault_migration_{version} \
             BEFORE INSERT ON _sqlx_migrations WHEN NEW.version = {version} \
             BEGIN SELECT RAISE(ABORT, 'upgrade-test fault at migration {version}'); END;"
        ))
        .await;
}

/// Reject every write of one `_livrarr_meta` key, whether inserted or updated.
async fn fail_meta_write(library: &Library, key: &str) {
    library
        .execute(&format!(
            "CREATE TRIGGER upgrade_test_fault_insert_{key} \
             BEFORE INSERT ON _livrarr_meta WHEN NEW.key = '{key}' \
             BEGIN SELECT RAISE(ABORT, 'upgrade-test fault on {key}'); END; \
             CREATE TRIGGER upgrade_test_fault_update_{key} \
             BEFORE UPDATE ON _livrarr_meta WHEN NEW.key = '{key}' \
             BEGIN SELECT RAISE(ABORT, 'upgrade-test fault on {key}'); END;"
        ))
        .await;
}

/// Stall every write of one `_livrarr_meta` key: its trigger runs a cross join
/// that does not finish, so the writing transaction stays open before commit.
/// It supplies no value; the stalled write can only be killed.
async fn stall_meta_write(library: &Library, key: &str) {
    library
        .execute(&format!(
            "CREATE TABLE upgrade_test_fault_stall_rows (x INTEGER); \
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000) \
             INSERT INTO upgrade_test_fault_stall_rows SELECT i FROM n; \
             CREATE TRIGGER upgrade_test_fault_stall_insert_{key} \
             BEFORE INSERT ON _livrarr_meta WHEN NEW.key = '{key}' \
             BEGIN SELECT count(*) FROM upgrade_test_fault_stall_rows a, \
                   upgrade_test_fault_stall_rows b, upgrade_test_fault_stall_rows c; END; \
             CREATE TRIGGER upgrade_test_fault_stall_update_{key} \
             BEFORE UPDATE ON _livrarr_meta WHEN NEW.key = '{key}' \
             BEGIN SELECT count(*) FROM upgrade_test_fault_stall_rows a, \
                   upgrade_test_fault_stall_rows b, upgrade_test_fault_stall_rows c; END;"
        ))
        .await;
}

async fn clear_faults(library: &Library) {
    let pool = library.pool().await;
    let faults: Vec<(String, String)> = sqlx::query_as(
        "SELECT type, name FROM sqlite_master \
          WHERE type IN ('trigger', 'table') AND name LIKE 'upgrade_test_fault_%' \
          ORDER BY type DESC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (kind, name) in faults {
        sqlx::query(&format!("DROP {kind} {name}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
}

// ── queries ─────────────────────────────────────────────────────────────────

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn meta(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn max_migration(pool: &SqlitePool) -> i64 {
    count(
        pool,
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1",
    )
    .await
}

async fn has_migration(pool: &SqlitePool, version: i64) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ?")
        .bind(version)
        .fetch_one(pool)
        .await
        .unwrap()
        == 1
}

async fn index_exists(pool: &SqlitePool, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
        == 1
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    id: i64,
    mode: String,
    branch: String,
    status: String,
    report_json: String,
}

async fn runs(pool: &SqlitePool) -> Vec<Run> {
    sqlx::query(
        "SELECT id, mode, branch, status, report_json FROM identity_cutover_runs ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|row| Run {
        id: row.get("id"),
        mode: row.get("mode"),
        branch: row.get("branch"),
        status: row.get("status"),
        report_json: row.get("report_json"),
    })
    .collect()
}

async fn automatic_runs(pool: &SqlitePool) -> Vec<Run> {
    runs(pool)
        .await
        .into_iter()
        .filter(|run| run.branch == "automatic")
        .collect()
}

async fn rows(pool: &SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

/// The identity graph and the rows a conversion or activation writes.
async fn identity_tables(pool: &SqlitePool) -> BTreeMap<&'static str, Vec<String>> {
    let mut tables = BTreeMap::new();
    for (name, sql) in [
        ("routes", "SELECT json_array(id, user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, provider_scoped_id, state, provenance, user_confirmed, observed_at) FROM identity_routes ORDER BY id"),
        ("editions", "SELECT json_array(id, user_id, work_id, format, language, subtitle, source_provider, provider_edition_id, state) FROM editions ORDER BY id"),
        ("contributors", "SELECT json_array(user_id, work_id, author_id, ordinal) FROM work_contributors ORDER BY user_id, work_id, ordinal"),
        ("review_cards", "SELECT json_array(id, user_id, work_id, kind, generation, status, resolved_at) FROM identity_review_cards ORDER BY id"),
        ("route_clashes", "SELECT json_array(id, user_id, current_work_id, status, resolution) FROM identity_conflicts_v2 ORDER BY id"),
        ("runs", "SELECT json_array(id, mode, branch, status, report_json, created_at, updated_at) FROM identity_cutover_runs ORDER BY id"),
        ("reports", "SELECT json_array(id, run_id, mapped_route_count, edition_count, blocker_count, index_ready, trivially_empty) FROM identity_cutover_reports ORDER BY id"),
        ("work_keys", "SELECT json_array(id, normalized_identity_main, normalized_identity_subtitle, normalized_identity_volume, primary_author_id, text_distinction, identity_status_v2) FROM works ORDER BY id"),
        ("upgrade_audit", "SELECT json_array(id, user_id, work_id, event_kind, payload) FROM identity_audit_events WHERE actor = 'identity-upgrade' ORDER BY id"),
    ] {
        tables.insert(name, rows(pool, sql).await);
    }
    tables
}

async fn graph_counts(pool: &SqlitePool) -> [i64; 4] {
    [
        count(pool, "SELECT COUNT(*) FROM identity_routes").await,
        count(pool, "SELECT COUNT(*) FROM editions").await,
        count(pool, "SELECT COUNT(*) FROM work_contributors").await,
        count(
            pool,
            "SELECT COUNT(*) FROM identity_audit_events WHERE actor = 'identity-upgrade'",
        )
        .await,
    ]
}

/// Each identity-upgrade audit event for a work, as "<event kind> <payload>".
async fn upgrade_audit_for(pool: &SqlitePool, work_id: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT event_kind || ' ' || payload FROM identity_audit_events \
          WHERE actor = ? AND work_id = ? ORDER BY id",
    )
    .bind(UPGRADE_ACTOR)
    .bind(work_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn mentions_number(text: &str, number: i64) -> bool {
    text.split(|c: char| !c.is_ascii_digit())
        .any(|part| part == number.to_string())
}

#[derive(Debug, PartialEq)]
struct Route {
    user_id: i64,
    owner_type: String,
    work_id: Option<i64>,
    edition_id: Option<i64>,
    resolved_work_id: i64,
    provider: String,
    kind: String,
    state: String,
    provenance: String,
    user_confirmed: i64,
}

async fn routes_with_value(pool: &SqlitePool, value: &str) -> Vec<Route> {
    sqlx::query(
        "SELECT user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
                state, provenance, user_confirmed \
           FROM identity_routes WHERE provider_scoped_id = ? ORDER BY id",
    )
    .bind(value)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|row| Route {
        user_id: row.get("user_id"),
        owner_type: row.get("owner_type"),
        work_id: row.get("work_id"),
        edition_id: row.get("edition_id"),
        resolved_work_id: row.get("resolved_work_id"),
        provider: row.get("provider"),
        kind: row.get("kind"),
        state: row.get("state"),
        provenance: row.get("provenance"),
        user_confirmed: row.get("user_confirmed"),
    })
    .collect()
}

/// The manual staging's column mapping in its production encoding
/// (crates/livrarr-db/src/identity_layer.rs, stage_legacy_identity_rows):
/// provider, route kind, and whether an edition owns the route.
fn legacy_mapping(column: &str) -> (String, String, bool) {
    use livrarr_domain::identity_layer::{IdentityProvider as P, RouteKind as K};
    let (provider, kind, edition) = match column {
        "ol_key" => (P::OpenLibrary, K::OpenLibraryWork, false),
        "hc_key" => (P::Hardcover, K::HardcoverWork, false),
        "gr_key" => (P::Goodreads, K::GoodreadsBookEdition, true),
        "isbn_13" => (P::IsbnRegistry, K::Isbn13Edition, true),
        "asin" => (P::Amazon, K::AsinEdition, true),
        other => panic!("no legacy identifier column {other}"),
    };
    (
        serde_json::to_string(&provider).unwrap(),
        serde_json::to_string(&kind).unwrap(),
        edition,
    )
}

/// One converted legacy identifier: its route carries the staging's provider,
/// kind, owner and provenance; an edition-owned route joins exactly one
/// same-user, same-work edition of format Unknown for that provider and value.
async fn assert_converted_route(
    pool: &SqlitePool,
    user_id: i64,
    work_id: i64,
    column: &str,
    value: &str,
    confirmed: bool,
) {
    let routes: Vec<Route> = routes_with_value(pool, value)
        .await
        .into_iter()
        .filter(|route| route.user_id == user_id)
        .collect();
    assert_eq!(
        routes.len(),
        1,
        "work {work_id} {column}={value} becomes one route: {routes:?}"
    );
    let route = &routes[0];
    let (provider, kind, edition_owned) = legacy_mapping(column);
    assert_eq!(
        (
            route.resolved_work_id,
            route.state.as_str(),
            route.provider.as_str(),
            route.kind.as_str()
        ),
        (work_id, "active", provider.as_str(), kind.as_str()),
        "work {work_id} {column}: owner work, state, provider and kind"
    );
    let provenance: Value = serde_json::from_str(&route.provenance).unwrap();
    assert_eq!(
        provenance,
        json!({"Migrated": {"legacy_field": column}}),
        "work {work_id} {column} route provenance"
    );
    assert_eq!(
        route.user_confirmed,
        i64::from(confirmed),
        "work {work_id} {column}: confirmed only by the user's own pick of this value"
    );
    if !edition_owned {
        assert_eq!(
            (route.owner_type.as_str(), route.work_id, route.edition_id),
            ("work", Some(work_id), None),
            "work {work_id} {column}: a work-owned route"
        );
        return;
    }
    assert_eq!(
        (route.owner_type.as_str(), route.work_id),
        ("edition", None),
        "work {work_id} {column}: an edition-owned route"
    );
    let unknown =
        serde_json::to_string(&livrarr_domain::identity_layer::EditionFormat::Unknown).unwrap();
    let editions: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM editions WHERE user_id = ? AND work_id = ? AND format = ? \
            AND source_provider = ? AND provider_edition_id = ? AND state = 'active'",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(&unknown)
    .bind(&provider)
    .bind(value)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        editions.len(),
        1,
        "work {work_id} {column}: one Unknown-format edition for {value}"
    );
    assert_eq!(
        route.edition_id,
        Some(editions[0]),
        "work {work_id} {column}: the route is owned by that edition"
    );
}

async fn work_status(pool: &SqlitePool, work_id: i64) -> String {
    sqlx::query_scalar("SELECT identity_status_v2 FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Works, ledger and cover rows as seeded, for comparing a pre-upgrade copy.
async fn seeded_library_signature(pool: &SqlitePool) -> Vec<String> {
    let mut signature = rows(
        pool,
        "SELECT json_array(version, hex(checksum)) FROM _sqlx_migrations ORDER BY version",
    )
    .await;
    signature.extend(
        rows(
            pool,
            "SELECT json_array(id, user_id, title, author_name, ol_key, gr_key, isbn_13, asin, \
                    audiobook_cover_url, audiobook_cover_trust) FROM works ORDER BY id",
        )
        .await,
    );
    signature
}

/// At 073, switch the one machine-chosen Goodreads audiobook cover to an Audible
/// cover, so the Goodreads cover repair has nothing to do and completes on its
/// first pass.
async fn without_machine_goodreads_cover(library: &Library) {
    library
        .execute(&format!(
            "UPDATE works SET audiobook_cover_source = 'audible' WHERE id = {WORK_WITH_MACHINE_GOODREADS_COVER}"
        ))
        .await;
}

/// The six counts the upgrade summary reports (REQ-006), by meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SummaryCount {
    Books,
    Identifiers,
    Confirmed,
    LookAlikes,
    Shared,
    ClosedQuestions,
}

const SUMMARY_TOPIC: &str = "identity upgrade";

/// A count's meaning from its label: a prose phrase ("2 look-alike books kept
/// separate"), a structured field name or a stored JSON key. Wording is free
/// (REQ-006 calls it illustrative); the meaning is not.
fn summary_meaning(label: &str) -> Option<SummaryCount> {
    let label = label.to_lowercase().replace(['-', '_'], " ");
    let has = |word: &str| label.contains(word);
    if has("look alike") || has("lookalike") || has("separat") {
        Some(SummaryCount::LookAlikes)
    } else if has("shared") {
        Some(SummaryCount::Shared)
    } else if has("question") {
        Some(SummaryCount::ClosedQuestions)
    } else if has("confirm") {
        Some(SummaryCount::Confirmed)
    } else if has("identifier") || has("route") {
        Some(SummaryCount::Identifiers)
    } else if has("book") || has("work") {
        Some(SummaryCount::Books)
    } else {
        None
    }
}

/// The six counts reported after the summary topic, as prose ("20 books, 26
/// identifiers (8 you had confirmed); ...") or as `name=value` fields; None
/// unless every count is present exactly once.
fn summary_counts(line: &str) -> Option<BTreeMap<SummaryCount, i64>> {
    let lower = line.to_lowercase();
    let tail = &lower[lower.find(SUMMARY_TOPIC)? + SUMMARY_TOPIC.len()..];
    let fields = regex::Regex::new(r"([a-z_]+)=(\d+)").unwrap();
    let labelled: Vec<(String, i64)> = if fields.is_match(tail) {
        fields
            .captures_iter(tail)
            .map(|field| (field[1].to_owned(), field[2].parse().unwrap()))
            .collect()
    } else {
        tail.split([',', ';', ':', '(', ')'])
            .filter_map(|segment| {
                let numbers: Vec<i64> = segment
                    .split(|c: char| !c.is_ascii_digit())
                    .filter_map(|part| part.parse().ok())
                    .collect();
                (numbers.len() == 1).then(|| (segment.to_owned(), numbers[0]))
            })
            .collect()
    };
    let mut counts = BTreeMap::new();
    for (label, value) in labelled {
        if let Some(meaning) = summary_meaning(&label) {
            if counts.insert(meaning, value).is_some() {
                return None;
            }
        }
    }
    (counts.len() == 6).then_some(counts)
}

/// The same six counts from the automatic run's stored JSON summary, keyed by
/// field-name meaning.
fn stored_summary_counts(report_json: &str) -> BTreeMap<SummaryCount, i64> {
    fn walk(key: &str, value: &Value, counts: &mut BTreeMap<SummaryCount, i64>) {
        match value {
            Value::Object(fields) => fields
                .iter()
                .for_each(|(key, value)| walk(key, value, counts)),
            Value::Number(number) => {
                if let (Some(meaning), Some(number)) = (summary_meaning(key), number.as_i64()) {
                    assert!(
                        counts.insert(meaning, number).is_none(),
                        "the stored summary has one field per count: {key}"
                    );
                }
            }
            _ => {}
        }
    }
    let summary: Value =
        serde_json::from_str(report_json).expect("the automatic run stores a JSON summary");
    let mut counts = BTreeMap::new();
    walk("", &summary, &mut counts);
    counts
}

/// What a keep-3 prune by name must leave: every protected copy (never
/// deleted) plus the three newest names; `.partial` files are never counted.
/// The scenarios keep protected copies older than the others, so this is the
/// same whether or not protected copies count toward the three.
fn retained_after_prune(
    completed: &BTreeSet<String>,
    protected: &BTreeSet<String>,
) -> BTreeSet<String> {
    let newest: BTreeSet<String> = completed.iter().rev().take(3).cloned().collect();
    protected.union(&newest).cloned().collect()
}

fn copy_names(library: &Library) -> BTreeSet<String> {
    library.copy_checksums().into_keys().collect()
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../livrarr-db/migrations")
}

/// The real historical migration files through `last`, run by SQLx's own
/// migrator. Unlike `run_migrations`, this never reaches migration 091.
async fn migrate_historically(library: &Library, last: i64) {
    let historical = TempDir::new().unwrap();
    for entry in std::fs::read_dir(migrations_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = name_of(&path);
        let version: i64 = name.split('_').next().unwrap().parse().unwrap();
        if name.ends_with(".sql") && version <= last {
            std::fs::copy(&path, historical.path().join(&name)).unwrap();
        }
    }
    let pool = library.pool().await;
    sqlx::migrate::Migrator::new(historical.path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .expect("historical migrations apply to the alpha6 library");
    assert_eq!(max_migration(&pool).await, last);
    pool.close().await;
}

/// Identity tables of the same alpha6 library taken to 090 by the historical
/// migrator: the state a failed 091 must leave behind.
async fn identity_tables_at_090() -> BTreeMap<&'static str, Vec<String>> {
    let reference = Library::alpha6().await;
    migrate_historically(&reference, LAST_PUBLISHED_MIGRATION).await;
    let pool = reference.pool().await;
    let tables = identity_tables(&pool).await;
    pool.close().await;
    tables
}

// ── the fixture itself ──────────────────────────────────────────────────────

#[tokio::test]
async fn alpha6_fixture_is_a_legal_alpha6_library() {
    let library = Library::alpha6().await;
    let pool = library.pool().await;

    // Applied migrations: exactly alpha6's 001-073, with alpha6's checksums,
    // which are also today's files (published migrations are immutable).
    let ledger: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT version, lower(hex(checksum)), success FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let manifest: Vec<(i64, String, i64)> = ALPHA6_MIGRATIONS
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            let version = name.split('_').next().unwrap().parse().unwrap();
            (version, digest.to_owned(), 1)
        })
        .collect();
    assert_eq!(ledger, manifest, "the ledger is alpha6's migration history");
    assert_eq!(max_migration(&pool).await, ALPHA6_LAST_MIGRATION);
    for line in ALPHA6_MIGRATIONS.lines() {
        let (digest, name) = line.split_once("  ").unwrap();
        let current = format!(
            "{:x}",
            Sha384::digest(std::fs::read(migrations_dir().join(name)).unwrap())
        );
        assert_eq!(current, digest, "{name} is byte-identical to alpha6's");
    }

    // alpha6's runtime-installed work index (never created by a migration),
    // and none of the identity layer yet.
    let index_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_works_identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("alpha6's runtime idx_works_identity");
    assert_eq!(
        index_sql,
        "CREATE UNIQUE INDEX idx_works_identity ON works(user_id, normalized_title, normalized_author)"
    );
    assert!(!index_exists(&pool, "idx_works_identity_v2").await);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'identity_routes'"
        )
        .await,
        0
    );
    assert_eq!(
        meta(&pool, "identity_key_generation").await.as_deref(),
        Some("1")
    );
    assert_eq!(meta(&pool, AUTHORITY_MARKER).await, None);

    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    assert_eq!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .len(),
        0
    );

    // Work rows: values alpha6's readers accept and writers produce; the keys
    // are alpha6's own identity_key recipe (generation 1).
    let works = sqlx::query(
        "SELECT id, title, author_name, normalized_title, normalized_author, enrichment_status, \
                identity_status, cover_trust, cover_manual, audiobook_cover_trust FROM works",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(works.len() as i64, FIXTURE_BOOKS);
    for work in &works {
        let id: i64 = work.get("id");
        let (title, author) = livrarr_domain::identity_matching::identity_key(
            work.get::<String, _>("title").as_str(),
            work.get::<String, _>("author_name").as_str(),
        );
        assert_eq!(
            (title, author),
            (
                work.get::<String, _>("normalized_title"),
                work.get::<String, _>("normalized_author")
            ),
            "work {id} keys are alpha6's own"
        );
        assert!(["unenriched", "enriched", "thin", "failed"]
            .contains(&work.get::<String, _>("enrichment_status").as_str()));
        assert!([
            "pending",
            "confirmed",
            "provisional",
            "conflict",
            "needs_review",
            "not_found"
        ]
        .contains(&work.get::<String, _>("identity_status").as_str()));
        let trust: String = work.get("cover_trust");
        let audio_trust: String = work.get("audiobook_cover_trust");
        for value in [&trust, &audio_trust] {
            assert!(["user", "validated", "unvalidated"].contains(&value.as_str()));
        }
        assert_eq!(
            work.get::<i64, _>("cover_manual") == 1,
            trust == "user",
            "work {id}: alpha6 set cover_manual exactly when the ebook cover trust was user"
        );
    }

    // Confirmed anchors pass alpha6's anchor writer (canonical typed values;
    // OpenLibrary and Hardcover only need a non-empty value).
    let anchors: Vec<(String, String)> = sqlx::query_as(
        "SELECT anchor_type, anchor_value FROM work_identity_anchors WHERE confidence = 'confirmed'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (kind, value) in &anchors {
        let accepted = match kind.as_str() {
            "isbn_13" => normalize_isbn13(value).as_deref() == Some(value.as_str()),
            "gr_work" => normalize_gr_key(value).as_deref() == Some(value.as_str()),
            "asin" => matches!(normalize_asin(value), AsinNorm::Asin(asin) if &asin == value),
            "ol_work" | "hc_work" => !value.trim().is_empty(),
            _ => false,
        };
        assert!(accepted, "alpha6 accepts confirmed {kind} anchor {value:?}");
    }

    let conflicts = sqlx::query(
        "SELECT kind, raised_by, status, resolution_action, incoming_payload_json \
           FROM work_identity_conflicts",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for conflict in &conflicts {
        assert_eq!(
            conflict.get::<String, _>("kind"),
            "incoming_different_ol_key"
        );
        assert_eq!(conflict.get::<String, _>("raised_by"), "refresh");
        assert!(["open", "resolved", "dismissed"]
            .contains(&conflict.get::<String, _>("status").as_str()));
        assert!(matches!(
            conflict
                .get::<Option<String>, _>("resolution_action")
                .as_deref(),
            None | Some("keep_existing" | "accept_separate" | "replace_anchor" | "merge")
        ));
        let payload: Value =
            serde_json::from_str(&conflict.get::<String, _>("incoming_payload_json")).unwrap();
        for field in [
            "ol_key",
            "gr_key",
            "hc_key",
            "isbn_13",
            "asin",
            "title",
            "author_name",
            "year",
            "cover_url",
            "top_candidates",
        ] {
            assert!(payload.get(field).is_some(), "alpha6 payload field {field}");
        }
    }

    // The states the upgrade tests rely on are present as alpha6 wrote them:
    // a subtitle with a non-integer series position; machine guesses that live
    // only in the anchor ledger (alpha6's pending writer never syncs columns);
    // the user's empty "still pending" marker; one book with mixed setters.
    let series: (Option<String>, Option<i64>, Option<f64>) =
        sqlx::query_as("SELECT subtitle, series_id, series_position FROM works WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        series,
        (
            Some("The First Book of Earthsea".to_owned()),
            Some(1),
            Some(1.5)
        )
    );
    let guesses: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.anchor_type, a.anchor_value FROM work_identity_anchors a JOIN works w ON w.id = a.work_id \
          WHERE a.confidence = 'pending' AND a.setter = 'auto_search' AND a.anchor_value <> '' \
            AND a.anchor_value NOT IN (COALESCE(w.ol_key, ''), COALESCE(w.hc_key, ''), \
                COALESCE(w.gr_key, ''), COALESCE(w.isbn_13, ''), COALESCE(w.asin, '')) \
          ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        guesses,
        [
            ("gr_work".to_owned(), "900055".to_owned()),
            ("ol_work".to_owned(), "/works/OL900055W".to_owned())
        ]
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM work_identity_anchors WHERE work_id = 5 AND anchor_value = '' \
                AND confidence = 'pending' AND setter = 'user'"
        )
        .await,
        1
    );
    let mixed: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT setter FROM work_identity_anchors WHERE work_id = 9 ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(mixed, ["auto_search", "import", "user"]);
    for (relative, bytes) in FIXTURE_COVERS {
        assert_eq!(
            std::fs::read(library.dir().join("covers").join(relative)).unwrap(),
            bytes
        );
    }
    pool.close().await;
}

// ── AC-001 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn alpha6_library_upgrades_on_first_start() {
    let library = Library::alpha6().await;
    let client = client();
    let seeded = {
        let pool = library.pool().await;
        let signature = seeded_library_signature(&pool).await;
        pool.close().await;
        signature
    };

    let mut server = library
        .serve(
            &client,
            "an alpha6 library must convert and serve on its first start",
        )
        .await;
    let work = read_work(&server, &client, 1).await;
    server.stop();
    let first_log = server.logs();

    let pool = library.pool().await;
    assert_eq!(
        meta(&pool, AUTHORITY_MARKER).await.as_deref(),
        Some("active"),
        "the first start activates the identity layer"
    );
    let all_runs = runs(&pool).await;
    assert_eq!(all_runs.len(), 1, "exactly one cutover run: {all_runs:?}");
    assert_eq!(
        (all_runs[0].branch.as_str(), all_runs[0].status.as_str()),
        ("automatic", "activated"),
        "the one run is the automatic conversion, activated"
    );
    assert_eq!(
        work["olKey"], "/works/OL900001W",
        "the work read shows the identifier projected from the converted route: {work}"
    );
    assert_eq!(
        work["identityStatus"], "confirmed",
        "the work read shows the green Confirmed badge: {work}"
    );

    let copies = library.copies();
    assert_eq!(copies.len(), 1, "exactly one pre-upgrade copy: {copies:?}");
    let (_copy_dir, copy) = open_copy(&copies[0]).await;
    assert_eq!(
        seeded_library_signature(&copy).await,
        seeded,
        "the pre-upgrade copy opens as the seeded alpha6 library"
    );
    copy.close().await;

    let stored = stored_summary_counts(&all_runs[0].report_json);
    assert_eq!(
        stored.len(),
        6,
        "the stored summary holds books, identifiers, confirmed identifiers, separated \
         look-alikes, shared identifiers and closed questions: {}",
        all_runs[0].report_json
    );
    // Summary events are selected by topic before any parsing, so an
    // incomplete or malformed second summary counts and fails.
    let events: Vec<&str> = first_log
        .lines()
        .filter(|line| line.to_lowercase().contains(SUMMARY_TOPIC))
        .collect();
    assert_eq!(
        events.len(),
        1,
        "the first start logs exactly one upgrade summary:\n{first_log}"
    );
    let summary = summary_counts(events[0]).unwrap_or_else(|| {
        panic!(
            "the summary reports each of the six counts exactly once: {}",
            events[0]
        )
    });
    assert_eq!(
        summary, stored,
        "each summary count equals its stored field (stored: {})",
        all_runs[0].report_json
    );
    assert_eq!(stored[&SummaryCount::Books], FIXTURE_BOOKS);
    pool.close().await;

    let checksums = library.copy_checksums();
    let mut again = library
        .serve(&client, "a converted library keeps serving")
        .await;
    again.stop();
    let second_log = again.logs();
    assert!(
        !second_log.to_lowercase().contains(SUMMARY_TOPIC),
        "a second start logs no upgrade summary:\n{second_log}"
    );
    assert_eq!(
        library.copy_checksums(),
        checksums,
        "a second start writes no copy"
    );
}

// ── AC-002 ──────────────────────────────────────────────────────────────────

const PRESERVED_WORK_FIELDS: &str =
    "SELECT json_array(id, user_id, title, subtitle, author_name, author_id, \
     series_id, series_name, series_position, \
     monitor_ebook, monitor_audiobook, cover_url, cover_source, cover_width, cover_height, \
     cover_manual, audiobook_cover_url, audiobook_cover_source, audiobook_cover_width, \
     audiobook_cover_height) FROM works WHERE id NOT IN (7, 8) ORDER BY id";
/// Books carrying all five identifier kinds, and which of their identifiers
/// the user picked: book 1 all five, book 9 two, books 2-4 none (import,
/// automatic matches, pending guesses copied into the columns).
const FIVE_IDENTIFIER_BOOKS: [(i64, &[&str]); 5] = [
    (1, &["ol_key", "hc_key", "gr_key", "isbn_13", "asin"]),
    (2, &[]),
    (3, &[]),
    (4, &[]),
    (9, &["ol_key", "isbn_13"]),
];
const PRESERVED_AUTHORS: &str = "SELECT json_array(id, user_id, name) FROM authors ORDER BY id";
const PRESERVED_PROVENANCE: &str = "SELECT json_array(user_id, work_id, field, source, set_at, \
     setter, cleared) FROM work_metadata_provenance ORDER BY work_id, field";

#[tokio::test]
async fn upgrade_keeps_user_picks_and_status() {
    let library = Library::alpha6().await;
    let client = client();
    let pool = library.pool().await;
    let before = [
        rows(&pool, PRESERVED_WORK_FIELDS).await,
        rows(&pool, PRESERVED_AUTHORS).await,
        rows(&pool, PRESERVED_PROVENANCE).await,
    ];
    let mut identifiers = Vec::new();
    for (work_id, picked) in FIVE_IDENTIFIER_BOOKS {
        let values: (String, String, String, String, String) =
            sqlx::query_as("SELECT ol_key, hc_key, gr_key, isbn_13, asin FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        identifiers.push((work_id, picked, values));
    }
    pool.close().await;

    library
        .serve_through_cover_pass(&client, "an alpha6 library must convert and serve")
        .await;

    let pool = library.pool().await;
    for (work_id, picked, (ol, hc, gr, isbn, asin)) in &identifiers {
        for (column, value) in [
            ("ol_key", ol),
            ("hc_key", hc),
            ("gr_key", gr),
            ("isbn_13", isbn),
            ("asin", asin),
        ] {
            assert_converted_route(&pool, 1, *work_id, column, value, picked.contains(&column))
                .await;
        }
    }
    // Machine guesses alpha6 kept only in its anchor ledger, and the user's
    // empty "still pending" marker, make nothing.
    for guess in ["/works/OL900055W", "900055"] {
        assert!(
            routes_with_value(&pool, guess).await.is_empty(),
            "an anchor-only pending guess {guess} makes no route"
        );
        let editions = count(
            &pool,
            &format!("SELECT COUNT(*) FROM editions WHERE provider_edition_id = '{guess}'"),
        )
        .await;
        assert_eq!(
            editions, 0,
            "an anchor-only pending guess {guess} makes no edition"
        );
    }
    assert_eq!(
        count(
            &pool,
            "SELECT (SELECT COUNT(*) FROM identity_routes WHERE resolved_work_id = 5) \
                  + (SELECT COUNT(*) FROM editions WHERE work_id = 5)"
        )
        .await,
        0,
        "work 5's pending guesses and empty marker make no route or edition"
    );
    // Book 1: a subtitle and a series position; its identity key uses the
    // manual staging's mapping (main title key, lower-cased trimmed subtitle,
    // position as text, the book's author).
    let key: (String, String, String, Option<i64>) = sqlx::query_as(
        "SELECT normalized_identity_main, normalized_identity_subtitle, \
                normalized_identity_volume, primary_author_id FROM works WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        key,
        (
            "wizard of earthsea".to_owned(),
            "the first book of earthsea".to_owned(),
            "1.5".to_owned(),
            Some(1)
        ),
        "book 1's identity key"
    );
    for (work_id, status) in [
        (1, "user_confirmed"),
        (2, "connected"),
        (3, "connected"),
        (4, "connected"),
        (5, "not_connected"),
        (6, "connected"),
        (9, "user_confirmed"),
    ] {
        assert_eq!(
            work_status(&pool, work_id).await,
            status,
            "work {work_id} status"
        );
    }
    let contributors: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT w.id, COUNT(c.work_id), COUNT(CASE WHEN c.ordinal = 0 THEN 1 END), \
                COUNT(CASE WHEN c.ordinal = 0 AND c.author_id = w.author_id THEN 1 END) \
           FROM works w LEFT JOIN work_contributors c ON c.user_id = w.user_id AND c.work_id = w.id \
          GROUP BY w.id ORDER BY w.id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (work_id, all, first, primary) in contributors {
        assert_eq!(
            (all, first, primary),
            (1, 1, 1),
            "work {work_id}: one ordinal-0 contributor, the book's author"
        );
    }
    assert_eq!(
        [
            rows(&pool, PRESERVED_WORK_FIELDS).await,
            rows(&pool, PRESERVED_AUTHORS).await,
            rows(&pool, PRESERVED_PROVENANCE).await,
        ],
        before,
        "titles, subtitles, authors, series, monitor flags, metadata provenance and both \
         cover slots are unchanged"
    );
    for (relative, bytes) in &FIXTURE_COVERS[..2] {
        assert_eq!(
            std::fs::read(library.dir().join("covers").join(relative)).unwrap(),
            *bytes,
            "cover file {relative} unchanged"
        );
    }

    let second_user = routes_with_value(&pool, "/works/OL900002W").await;
    assert_eq!(
        second_user
            .iter()
            .map(|route| (route.user_id, route.resolved_work_id))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([(1, 2), (2, 20)]),
        "the second user's identical identifier stays with that user's book"
    );
    assert!(upgrade_audit_for(&pool, 20).await.is_empty());
    pool.close().await;
}

// ── AC-003 ──────────────────────────────────────────────────────────────────

type AudiobookCover = (Option<String>, Option<String>, Option<i64>, Option<i64>);

async fn audiobook_cover_slot(pool: &SqlitePool, work_id: i64) -> AudiobookCover {
    sqlx::query_as(
        "SELECT audiobook_cover_url, audiobook_cover_source, audiobook_cover_width, \
                audiobook_cover_height FROM works WHERE id = ?",
    )
    .bind(work_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn audiobook_cover(pool: &SqlitePool, work_id: i64) -> (AudiobookCover, i64) {
    let manual = sqlx::query_scalar("SELECT audiobook_cover_manual FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(pool)
        .await
        .unwrap();
    (audiobook_cover_slot(pool, work_id).await, manual)
}

async fn assert_user_audiobook_cover_kept(library: &Library, expected: &AudiobookCover) {
    let pool = library.pool().await;
    let (cover, manual) = audiobook_cover(&pool, WORK_WITH_USER_AUDIOBOOK_COVER).await;
    assert_eq!(
        manual, 1,
        "alpha6's audiobook cover choice becomes audiobook_cover_manual=1"
    );
    assert_eq!(
        &cover, expected,
        "the chosen audiobook cover's URL, source and size survive the cover pass"
    );
    assert_eq!(
        std::fs::read(library.dir().join("covers/1/7_audio.jpg")).ok(),
        Some(FIXTURE_COVERS[2].1.to_vec()),
        "the chosen audiobook cover file survives the cover pass"
    );
    let (_, control_manual) = audiobook_cover(&pool, WORK_WITH_MACHINE_GOODREADS_COVER).await;
    assert_eq!(
        control_manual, 0,
        "a machine audiobook cover stays machine-owned"
    );
    pool.close().await;
}

#[tokio::test]
async fn upgrade_keeps_audiobook_cover_choice() {
    let library = Library::alpha6().await;
    let client = client();
    let pool = library.pool().await;
    let chosen = audiobook_cover_slot(&pool, WORK_WITH_USER_AUDIOBOOK_COVER).await;
    pool.close().await;
    library
        .serve_through_cover_pass(&client, "an alpha6 library must convert and serve")
        .await;
    assert_user_audiobook_cover_kept(&library, &chosen).await;

    // A start interrupted after 084 dropped the old column must not lose it.
    let interrupted = Library::alpha6().await;
    fail_migration(&interrupted, 88).await;
    let log = interrupted
        .refuse(&client, "a fault at migration 088 must stop startup")
        .await;
    assert!(
        log.contains("upgrade-test fault at migration 88"),
        "the refusal is the migration-088 fault:\n{log}"
    );
    let pool = interrupted.pool().await;
    assert_eq!(max_migration(&pool).await, 87);
    let old_column = count(
        &pool,
        "SELECT COUNT(*) FROM pragma_table_info('works') WHERE name = 'audiobook_cover_trust'",
    )
    .await;
    assert_eq!(
        old_column, 0,
        "migration 084 has dropped the old trust column"
    );
    let saved = meta(&pool, COVER_CHOICES_KEY).await.unwrap_or_else(|| {
        panic!("the audiobook cover choices are saved before migration 084 drops them")
    });
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert!(
        saved
            .as_array()
            .is_some_and(|pairs| pairs.contains(&json!({"u": 1, "w": 7}))),
        "the saved list names user 1's work 7: {saved}"
    );
    pool.close().await;
    clear_faults(&interrupted).await;
    interrupted
        .serve_through_cover_pass(&client, "the retried upgrade must convert and serve")
        .await;
    assert_user_audiobook_cover_kept(&interrupted, &chosen).await;
}

// ── AC-004 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn upgrade_trims_identifiers_like_the_app() {
    let library = Library::alpha6().await;
    let client = client();
    let mut server = library
        .serve(&client, "an alpha6 library must convert and serve")
        .await;
    server.stop();
    let pool = library.pool().await;

    // Work 15: NBSP-padded OpenLibrary id with a matching NBSP-padded user pick;
    // work 16: the same id padded with a tab and a newline.
    let value = "/works/OL900015W";
    let routes = routes_with_value(&pool, value).await;
    assert_eq!(
        routes
            .iter()
            .map(|route| (route.resolved_work_id, route.user_confirmed))
            .collect::<Vec<_>>(),
        vec![(15, 1)],
        "one trimmed route on the lower book, still confirmed by its padded user pick"
    );
    let padded = count(
        &pool,
        "SELECT COUNT(*) FROM identity_routes WHERE resolved_work_id IN (15, 16, 17)",
    )
    .await;
    assert_eq!(padded, 1, "no route for the padded copies");
    let audit = upgrade_audit_for(&pool, 16).await;
    assert_eq!(audit.len(), 1, "one shared-identifier audit for work 16");
    assert!(
        audit[0].contains(value) && mentions_number(&audit[0], 15),
        "the audit names the trimmed value and the book that kept it: {}",
        audit[0]
    );
    // Work 17: whitespace-only OpenLibrary id and an ideographic-space ASIN.
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM editions WHERE work_id = 17").await,
        0,
        "whitespace-only identifiers make no edition"
    );
    assert_eq!(work_status(&pool, 17).await, "not_connected");
    pool.close().await;
}

// ── AC-005 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn upgrade_keeps_lookalikes_separate_and_first_book_keeps_shared_id() {
    let library = Library::alpha6().await;
    let client = client();
    let mut server = library
        .serve(
            &client,
            "an alpha6 library with look-alike books must convert and serve",
        )
        .await;
    server.stop();
    let pool = library.pool().await;

    let trio: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, text_distinction FROM works WHERE id IN (10, 11, 12) ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        trio.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![10, 11, 12],
        "no book is merged away"
    );
    assert_eq!(trio[0].1, "common", "the lowest id keeps the common key");
    assert!(
        trio[1].1 != "common" && trio[2].1 != "common" && trio[1].1 != trio[2].1,
        "the other two get their own distinctions: {trio:?}"
    );
    assert!(
        index_exists(&pool, "idx_works_identity_v2").await,
        "the activation unique index exists"
    );
    let shared = routes_with_value(&pool, "900013").await;
    assert_eq!(
        shared
            .iter()
            .map(|route| route.resolved_work_id)
            .collect::<Vec<_>>(),
        vec![13],
        "the shared Goodreads id is routed to the lower book only"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM editions WHERE work_id = 14 OR \
                (provider_edition_id = '900013' AND work_id <> 13)"
        )
        .await,
        0,
        "the book that did not keep the shared id gets no edition for it"
    );
    let loser = upgrade_audit_for(&pool, 14).await;
    assert_eq!(
        loser.len(),
        1,
        "one audit row for the book that did not keep it"
    );
    assert!(
        loser[0].contains("900013") && mentions_number(&loser[0], 13),
        "the audit names the shared value and the book that kept it: {}",
        loser[0]
    );
    assert!(
        upgrade_audit_for(&pool, 10).await.is_empty(),
        "the look-alike that keeps the common key gets no separation event"
    );
    for separated in [11, 12] {
        let events = upgrade_audit_for(&pool, separated).await;
        assert_eq!(
            events.len(),
            1,
            "one separation event for look-alike work {separated}: {events:?}"
        );
        assert!(
            events[0].to_lowercase().contains("separate") && mentions_number(&events[0], 10),
            "work {separated}'s event records that it was kept separate from work 10: {}",
            events[0]
        );
    }
    pool.close().await;
}

// ── AC-006 ──────────────────────────────────────────────────────────────────

const CLOSED_CONFLICTS: &str = "SELECT json_array(id, user_id, existing_work_id, kind, \
     incoming_payload_json, raised_at, raised_by, status, resolved_at, resolution_action, \
     resolution_notes) FROM work_identity_conflicts WHERE id IN (2, 3) ORDER BY id";

#[tokio::test]
async fn upgrade_closes_old_match_questions() {
    let library = Library::alpha6().await;
    let client = client();
    let pool = library.pool().await;
    let closed_before = rows(&pool, CLOSED_CONFLICTS).await;
    let open_payload: String = sqlx::query_scalar(
        "SELECT incoming_payload_json FROM work_identity_conflicts WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    pool.close().await;

    let mut server = library
        .serve(
            &client,
            "an open legacy match question must not stop the upgraded server",
        )
        .await;
    server.stop();
    let pool = library.pool().await;
    let (status, resolved_at, action, notes): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT status, resolved_at, resolution_action, resolution_notes \
               FROM work_identity_conflicts WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "dismissed");
    assert!(resolved_at.is_some());
    assert_eq!(action, None, "no user decision is invented");
    assert_eq!(
        notes.as_deref(),
        Some("closed by identity upgrade; existing match kept")
    );
    let audits: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM identity_audit_events WHERE actor = ? AND user_id = 1",
    )
    .bind(UPGRADE_ACTOR)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        audits
            .iter()
            .filter(|payload| **payload == open_payload)
            .count(),
        1,
        "one audit event carries the closed question's payload"
    );
    assert_eq!(
        rows(&pool, CLOSED_CONFLICTS).await,
        closed_before,
        "already-closed questions are untouched"
    );
    let kept = routes_with_value(&pool, "/works/OL900018W").await;
    assert_eq!(
        kept.iter()
            .map(|route| (route.resolved_work_id, route.state.as_str()))
            .collect::<Vec<_>>(),
        vec![(18, "active")],
        "the book keeps its existing key"
    );
    assert!(routes_with_value(&pool, "/works/OL900098W")
        .await
        .is_empty());
    pool.close().await;
}

// ── AC-007 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn upgrade_finishes_an_abandoned_manual_attempt() {
    let library = Library::alpha6().await;
    let client = client();
    migrate_historically(&library, LAST_PUBLISHED_MIGRATION).await;
    let pool = library.pool().await;
    let manual = SqliteDb::new(pool.clone())
        .run_identity_cutover(IdentityCutoverMode::Apply, None)
        .await;
    let latest = runs(&pool).await.pop();
    assert_eq!(
        latest
            .as_ref()
            .map(|run| (run.mode.as_str(), run.status.as_str())),
        Some(("apply", "blocked")),
        "precondition: the manual apply is blocked ({manual:?})"
    );
    let pending: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM identity_review_cards WHERE status = 'pending'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!pending.is_empty(), "precondition: a pending review card");
    pool.close().await;

    let mut server = library
        .serve(
            &client,
            "an abandoned manual attempt must be finished by the upgrade and serve",
        )
        .await;
    server.stop();
    let pool = library.pool().await;
    for card in pending {
        let status: String =
            sqlx::query_scalar("SELECT status FROM identity_review_cards WHERE id = ?")
                .bind(card)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "cancelled", "card {card} from the manual attempt");
    }
    pool.close().await;
}

// ── AC-008 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fresh_install_writes_no_copy_and_no_run() {
    let library = Library::empty();
    let client = client();
    let mut server = library.serve(&client, "a fresh install must serve").await;
    server.stop();
    let pool = library.pool().await;
    assert!(
        automatic_runs(&pool).await.is_empty(),
        "a fresh install records no automatic conversion"
    );
    pool.close().await;
    assert_eq!(
        library.files_with_prefix(),
        Vec::<PathBuf>::new(),
        "a fresh install writes no pre-upgrade copy"
    );
    let mut again = library.serve(&client, "a fresh install restarts").await;
    again.stop();
    assert_eq!(
        library.files_with_prefix(),
        Vec::<PathBuf>::new(),
        "a second start of a fresh install writes no copy"
    );
}

// ── AC-009 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn active_library_skips_091() {
    let library = Library::alpha6().await;
    let client = client();
    // At 073 the user dismisses the open question and deletes the books the
    // manual ceremony cannot activate (look-alikes and shared identifiers).
    library
        .execute(
            "UPDATE work_identity_conflicts SET status = 'dismissed', \
                    resolved_at = '2026-06-04T08:00:00+00:00' WHERE status = 'open'; \
             DELETE FROM works WHERE id IN (11, 12, 14, 16);",
        )
        .await;
    migrate_historically(&library, LAST_PUBLISHED_MIGRATION).await;
    let pool = library.pool().await;
    let db = SqliteDb::new(pool.clone());
    db.run_identity_cutover(IdentityCutoverMode::Apply, None)
        .await
        .expect("precondition: the manual apply stages the library");
    db.ensure_identity_authority_ready()
        .await
        .expect("precondition: the manual ceremony activates the library");
    assert_eq!(
        meta(&pool, AUTHORITY_MARKER).await.as_deref(),
        Some("active")
    );
    // The production startup repairs, as the previous start would have run them.
    livrarr_db::pool::adopt_identity_review_dismissals(&pool)
        .await
        .unwrap();
    livrarr_db::pool::heal_identity_title_policy(&pool)
        .await
        .unwrap();
    livrarr_db::pool::heal_identity_sweep_findings(&pool)
        .await
        .unwrap();
    livrarr_db::identity_layer::heal_identity_dedup_residue(&pool)
        .await
        .unwrap();
    livrarr_db::identity_layer::heal_identity_round10_residue(&pool)
        .await
        .unwrap();
    livrarr_db::identity_layer::heal_identity_round11_attempt_residue(&pool)
        .await
        .unwrap();
    livrarr_db::identity_layer::heal_identity_round15_search_ledger(&pool)
        .await
        .unwrap();
    livrarr_db::identity_layer::heal_identity_round21_goodreads_namespace(&pool)
        .await
        .unwrap();
    assert!(!has_migration(&pool, UPGRADE_MIGRATION).await);
    let before = identity_tables(&pool).await;
    pool.close().await;

    let mut server = library
        .serve(&client, "an activated library must serve")
        .await;
    server.stop();
    let pool = library.pool().await;
    assert!(
        has_migration(&pool, UPGRADE_MIGRATION).await,
        "migration 091 is recorded on the active library"
    );
    assert_eq!(
        identity_tables(&pool).await,
        before,
        "on an active library 091 changes no identity table or run row"
    );
    assert!(automatic_runs(&pool).await.is_empty());
    pool.close().await;
}

// ── AC-010 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn failed_091_rolls_back_and_retries() {
    let library = Library::alpha6().await;
    let client = client();
    let at_090 = identity_tables_at_090().await;
    fail_migration(&library, UPGRADE_MIGRATION).await;
    let log = library
        .refuse(&client, "a fault inside migration 091 must stop startup")
        .await;
    assert!(
        log.contains("Migration failed") && log.contains("upgrade-test fault at migration 91"),
        "the refusal is migration 091's failure:\n{log}"
    );
    let pool = library.pool().await;
    assert!(
        !has_migration(&pool, UPGRADE_MIGRATION).await,
        "091 is not recorded"
    );
    assert_eq!(max_migration(&pool).await, LAST_PUBLISHED_MIGRATION);
    assert!(automatic_runs(&pool).await.is_empty(), "no automatic run");
    assert_eq!(
        identity_tables(&pool).await,
        at_090,
        "091's changes roll back: the identity tables are as at 090"
    );
    pool.close().await;
    let copies = library.copy_checksums();
    assert_eq!(copies.len(), 1, "one pre-upgrade copy: {copies:?}");

    clear_faults(&library).await;
    let mut server = library
        .serve(&client, "the retried upgrade must convert and serve")
        .await;
    server.stop();
    let pool = library.pool().await;
    assert_eq!(
        automatic_runs(&pool)
            .await
            .iter()
            .map(|run| run.status.as_str())
            .collect::<Vec<_>>(),
        vec!["activated"],
        "the retry converts"
    );
    pool.close().await;
    assert_eq!(
        library.copy_checksums(),
        copies,
        "still one copy, with the same checksum"
    );
}

// ── AC-011 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn retry_between_upgrade_and_activation() {
    let library = Library::alpha6().await;
    let client = client();
    fail_meta_write(&library, AUTHORITY_MARKER).await;
    let log = library
        .refuse(&client, "an activation fault after 091 must stop startup")
        .await;
    assert!(
        log.contains("upgrade-test fault on identity_authority_v2"),
        "the refusal is the activation fault:\n{log}"
    );
    let pool = library.pool().await;
    assert!(
        has_migration(&pool, UPGRADE_MIGRATION).await,
        "091 committed before the activation fault"
    );
    let ready = automatic_runs(&pool).await;
    assert_eq!(
        ready
            .iter()
            .map(|run| run.status.as_str())
            .collect::<Vec<_>>(),
        vec!["ready"],
        "the automatic run is ready, not activated"
    );
    assert_eq!(meta(&pool, AUTHORITY_MARKER).await, None);
    assert!(!index_exists(&pool, "idx_works_identity_v2").await);
    let counts = graph_counts(&pool).await;
    pool.close().await;
    let copies = library.copy_checksums();
    assert_eq!(copies.len(), 1, "one pre-upgrade copy: {copies:?}");

    clear_faults(&library).await;
    let mut server = library
        .serve(
            &client,
            "the next start must activate the same run and serve",
        )
        .await;
    server.stop();
    let pool = library.pool().await;
    let activated = automatic_runs(&pool).await;
    assert_eq!(
        activated
            .iter()
            .map(|run| (run.id, run.status.as_str()))
            .collect::<Vec<_>>(),
        vec![(ready[0].id, "activated")],
        "the same run is activated; one automatic run"
    );
    assert_eq!(
        graph_counts(&pool).await,
        counts,
        "route, edition, contributor and upgrade-audit counts are unchanged"
    );
    pool.close().await;
    assert_eq!(
        library.copy_checksums(),
        copies,
        "the original copy is kept"
    );
}

// ── AC-012 ──────────────────────────────────────────────────────────────────

const PLANTED_COPIES: [&str; 4] = [
    "livrarr.db.pre-migrate-v999-20990101-000001",
    "livrarr.db.pre-migrate-v999-20990101-000002",
    "livrarr.db.pre-migrate-v999-20990101-000003",
    "livrarr.db.pre-migrate-v999-20990101-000004",
];

#[tokio::test]
async fn original_copy_survives_partial_progress_prune_and_restore() {
    let library = Library::alpha6().await;
    let client = client();
    // A crash during an earlier copy left this work file behind.
    library.plant(
        "livrarr.db.pre-migrate-v073-20260101-000000.partial",
        b"interrupted copy",
    );

    fail_migration(&library, 78).await;
    library
        .refuse(&client, "a fault at migration 078 must stop startup")
        .await;
    assert!(
        library.partials().is_empty(),
        "a stale partial copy is removed: {:?}",
        library.partials()
    );
    let original = library.copy_checksums();
    assert_eq!(original.len(), 1, "one pre-upgrade copy: {original:?}");
    let original_name = original.keys().next().unwrap().clone();
    for version in [83, UPGRADE_MIGRATION] {
        clear_faults(&library).await;
        fail_migration(&library, version).await;
        library
            .refuse(&client, "a later migration fault must stop startup")
            .await;
        assert_eq!(
            library.copy_checksums(),
            original,
            "after the fault at {version}: still the one original copy"
        );
    }

    clear_faults(&library).await;
    for name in PLANTED_COPIES {
        library.plant(name, name.as_bytes());
    }
    library.plant(
        "livrarr.db.pre-migrate-v999-20990101-000009.partial",
        b"work file",
    );
    let before_prune = copy_names(&library);
    library
        .serve_through_cover_pass(&client, "the retried upgrade must convert and serve")
        .await;
    let after = library.copy_checksums();
    assert_eq!(
        after.get(&original_name),
        original.get(&original_name),
        "the prune keeps the original copy: {after:?}"
    );
    // Original (protected) + four newer copies: the prune keeps the original
    // and the three newest, removes the oldest eligible copy, and never counts
    // the work file.
    let expected = retained_after_prune(&before_prune, &BTreeSet::from([original_name.clone()]));
    assert_eq!(
        after.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "the prune removes eligible copies beyond three and keeps the protected original"
    );
    assert!(
        !after.contains_key(PLANTED_COPIES[0]),
        "the oldest eligible copy is removed: {after:?}"
    );

    // Refused starts after the upgrade finished add no copy.
    library
        .execute("UPDATE authors SET gr_key = 'not-a-goodreads-author' WHERE id = 1")
        .await;
    for _ in 0..2 {
        library
            .refuse(&client, "an invalid author key must stop startup")
            .await;
        assert_eq!(
            library.copy_checksums(),
            after,
            "a refused start adds no copy"
        );
    }

    // Restore the original, edit a book as alpha6 would, upgrade again.
    let original_path = library.dir().join(&original_name);
    library.restore(&original_path);
    clear_faults(&library).await;
    // The restored copy's attempt crashed after recording it and before
    // removing its pending file: recovery must keep the copy the restored
    // database names as its source, not reclaim it as that file's orphan.
    library.plant(PENDING_COPY_FILE, original_name.as_bytes());
    library
        .execute(
            "UPDATE works SET title = 'A Wizard of Earthsea (edited after restore)' WHERE id = 1",
        )
        .await;
    // Also at 073: no machine Goodreads cover is left, so the restored
    // upgrade's cover repair can complete once its marker may be stamped.
    without_machine_goodreads_cover(&library).await;
    fail_migration(&library, UPGRADE_MIGRATION).await;
    library
        .refuse(&client, "the second upgrade stops at the 091 fault")
        .await;
    let during = library.copy_checksums();
    assert_eq!(
        during.get(&original_name),
        original.get(&original_name),
        "the earlier copy stays until the new upgrade finishes"
    );
    let new_copies: Vec<&String> = during
        .keys()
        .filter(|name| !after.contains_key(*name))
        .collect();
    assert_eq!(
        new_copies.len(),
        1,
        "the second upgrade takes one new copy: {during:?}"
    );
    assert!(
        !library.dir().join(PENDING_COPY_FILE).exists(),
        "recovery clears the stale pending file"
    );
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    pool.close().await;
    assert!(
        record.contains(new_copies[0].as_str()) && record.contains(&original_name),
        "the new attempt is recorded with the restored copy as its previous: {record:?}"
    );
    let (_copy_dir, copy) = open_copy(&library.dir().join(new_copies[0])).await;
    let title: String = sqlx::query_scalar("SELECT title FROM works WHERE id = 1")
        .fetch_one(&copy)
        .await
        .unwrap();
    assert_eq!(
        title, "A Wizard of Earthsea (edited after restore)",
        "the new copy contains the edit"
    );
    copy.close().await;
    let edited = new_copies[0].clone();

    // The restored upgrade continues with its cover repair held incomplete,
    // under prune pressure from three newer eligible copies: the earlier
    // original stays protected beside the edited attempt's copy.
    clear_faults(&library).await;
    fail_meta_write(&library, COVER_REPAIR_MARKER).await;
    for name in [
        "livrarr.db.pre-migrate-v999-20990201-000001",
        "livrarr.db.pre-migrate-v999-20990201-000002",
        "livrarr.db.pre-migrate-v999-20990201-000003",
    ] {
        library.plant(name, name.as_bytes());
    }
    for start in 1..=2 {
        library
            .serve_through_cover_pass(
                &client,
                "the restored upgrade must serve with its cover repair pending",
            )
            .await;
        let now = library.copy_checksums();
        assert_eq!(
            now.get(&original_name),
            original.get(&original_name),
            "start {start}: the earlier original survives until the restored upgrade finishes: {now:?}"
        );
        assert_eq!(
            now.get(&edited),
            during.get(&edited),
            "start {start}: the edited attempt's copy is kept unchanged: {now:?}"
        );
        let pool = library.pool().await;
        let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
        pool.close().await;
        assert!(
            record.contains(&edited) && record.contains("in_progress"),
            "start {start}: the restored upgrade is still in progress: {record:?}"
        );
    }

    // The upgrade finishes once the cover marker can be stamped; only then is
    // the earlier original released to the ordinary prune.
    clear_faults(&library).await;
    library
        .serve_through_cover_pass(&client, "the restored upgrade completes its cover repair")
        .await;
    let mut settle = library
        .serve(&client, "the restored upgrade finishes")
        .await;
    settle.stop();
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    pool.close().await;
    assert!(
        is_complete(&record) && record.contains(&edited),
        "the restored upgrade finishes: {record:?}"
    );
    let finished = library.copy_checksums();
    assert_eq!(
        finished.get(&edited),
        during.get(&edited),
        "the edited attempt's copy is the rollback copy: {finished:?}"
    );
    assert!(
        !finished.contains_key(&original_name),
        "once the restored upgrade finished, the earlier original is released and pruned: {finished:?}"
    );
}

// ── AC-013 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn startup_repair_gets_one_copy() {
    let library = Library::alpha6().await;
    let client = client();
    let pool = library.pool().await;
    run_migrations(&pool)
        .await
        .expect("the library migrates to head through the real migration entry");
    assert_eq!(
        meta(&pool, "startup_generation").await,
        None,
        "precondition: no startup generation recorded"
    );
    pool.close().await;

    fail_meta_write(&library, LAST_SYNC_REPAIR_MARKER).await;
    let log = library
        .refuse(
            &client,
            "a fault on the last startup repair must stop startup",
        )
        .await;
    let pool = library.pool().await;
    assert_eq!(
        meta(&pool, SYNC_REPAIR_MARKERS[0]).await.as_deref(),
        Some("1"),
        "the first start converts and commits the early startup repairs before the fault:\n{log}"
    );
    pool.close().await;
    let copies = library.copy_checksums();
    assert_eq!(copies.len(), 1, "one copy before the repairs: {copies:?}");
    let (_copy_dir, copy) = open_copy(&library.dir().join(copies.keys().next().unwrap())).await;
    for marker in SYNC_REPAIR_MARKERS {
        assert_eq!(
            meta(&copy, marker).await,
            None,
            "the copy predates repair {marker}"
        );
    }
    copy.close().await;

    library
        .refuse(&client, "the repair fault still stops startup")
        .await;
    assert_eq!(
        library.copy_checksums(),
        copies,
        "a refused start after repairs committed reuses the copy"
    );

    clear_faults(&library).await;
    let mut server = library
        .serve(&client, "the repaired library must serve")
        .await;
    server.stop();
    let pool = library.pool().await;
    for marker in SYNC_REPAIR_MARKERS
        .into_iter()
        .filter(|marker| repair_enabled(marker))
    {
        assert!(meta(&pool, marker).await.is_some(), "repair {marker} ran");
    }
    pool.close().await;
    assert_eq!(library.copy_checksums(), copies, "still the one copy");
    let mut again = library
        .serve(&client, "a fully current library must serve")
        .await;
    again.stop();
    assert_eq!(
        library.copy_checksums(),
        copies,
        "a fully current library writes no copy"
    );
}

// ── AC-014 (R2-01) ──────────────────────────────────────────────────────────

/// Files a restart after an interruption may hold: at most one completed copy
/// beyond `protected` and one work file, the same after every repetition.
fn assert_bounded(
    library: &Library,
    protected: &BTreeMap<String, String>,
    bounded: &mut Option<(usize, usize)>,
    what: &str,
) {
    let now = library.copy_checksums();
    for (name, checksum) in protected {
        assert_eq!(
            now.get(name),
            Some(checksum),
            "{what}: the previous protected copy is unchanged"
        );
    }
    let files = (now.len() - protected.len(), library.partials().len());
    assert!(
        files.0 <= 1 && files.1 <= 1,
        "{what}: at most one unrecorded copy and one work file: {:?}",
        library.files_with_prefix()
    );
    assert_eq!(
        *bounded.get_or_insert(files),
        files,
        "{what}: repeated interruptions add no files"
    );
}

async fn assert_nothing_ran_before_association(library: &Library, what: &str) {
    let pool = library.pool().await;
    assert_eq!(
        max_migration(&pool).await,
        ALPHA6_LAST_MIGRATION,
        "{what}: no migration runs before the copy is recorded"
    );
    assert_eq!(
        meta(&pool, SYNC_REPAIR_MARKERS[0]).await,
        None,
        "{what}: no repair runs before the copy is recorded"
    );
    assert_eq!(meta(&pool, AUTHORITY_MARKER).await, None);
    pool.close().await;
}

/// After a kill at a named boundary: the observed copy is not recorded, the
/// protected copies are unchanged, files stay bounded and, on a first upgrade,
/// no migration or repair ran.
async fn assert_interrupted(
    library: &Library,
    observed: &str,
    protected: &BTreeMap<String, String>,
    bounded: &mut Option<(usize, usize)>,
    what: &str,
) {
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    pool.close().await;
    assert!(
        !record.contains(observed),
        "{what}: the killed attempt committed no association for {observed}: {record:?}"
    );
    if protected.is_empty() {
        assert_nothing_ran_before_association(library, what).await;
    }
    assert_bounded(library, protected, bounded, what);
}

/// After the restart that records an attempt: each copy observed at a kill
/// was either recovered (it is the recorded copy) or reclaimed (it is gone).
fn assert_recovered_or_reclaimed(library: &Library, observed: &[String], recorded: &str) {
    for name in observed {
        assert!(
            name == recorded || !library.dir().join(name).exists(),
            "the interrupted copy {name} is recovered or reclaimed (recorded: {recorded})"
        );
    }
}

/// Killed while the copy is written, before publication: the next start
/// reclaims the work file and records one complete copy of the library.
async fn copy_killed_before_publication_is_reclaimed(client: &Client) {
    for _ in 0..3 {
        let library = Library::alpha6().await;
        // A large provider-response cache (legal alpha6 data) makes copying and
        // syncing take a few hundred milliseconds, long enough to freeze the
        // child between a complete copy and its publication.
        library
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4096) \
                 INSERT INTO provider_response_cache (provider, anchor_type, anchor, payload, fetched_at) \
                 SELECT 'hardcover', 'hc_work', 'upgrade-test-bulk-' || i, hex(randomblob(32768)), \
                        '2026-06-01T12:00:00+00:00' FROM n",
            )
            .await;
        if library.kill_before_publication().await {
            assert_nothing_ran_before_association(&library, "kill before publication").await;
            return copy_killed_before_publication_restarts(&library, client).await;
        }
    }
    panic!("setup: three fresh libraries could not freeze a complete copy before its publication");
}

async fn copy_killed_before_publication_restarts(library: &Library, client: &Client) {
    let pool = library.pool().await;
    let seeded = seeded_library_signature(&pool).await;
    pool.close().await;
    fail_migration(library, UPGRADE_MIGRATION).await;
    library
        .refuse(client, "the restarted upgrade stops at the 091 fault")
        .await;
    assert!(
        library.partials().is_empty(),
        "the interrupted work file is reclaimed: {:?}",
        library.partials()
    );
    assert!(
        !library.dir().join(PENDING_COPY_FILE).exists(),
        "no copy is left pending once the restarted attempt is recorded"
    );
    let copies = library.copies();
    assert_eq!(copies.len(), 1, "one completed copy: {copies:?}");
    let (_copy_dir, copy) = open_copy(&copies[0]).await;
    assert_eq!(
        seeded_library_signature(&copy).await,
        seeded,
        "the recorded copy is complete and opens as the seeded library"
    );
    copy.close().await;
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    assert!(
        record.contains(&name_of(&copies[0])),
        "the attempt's record names its copy: {record:?}"
    );
    pool.close().await;
}

#[tokio::test]
async fn interrupted_snapshot_publication_stays_bounded_and_associated() {
    let client = client();
    copy_killed_before_publication_is_reclaimed(&client).await;

    let library = Library::alpha6().await;
    without_machine_goodreads_cover(&library).await;
    // A crash during an earlier copy left this work file behind.
    library.plant(
        "livrarr.db.pre-migrate-v073-20260101-000000.partial",
        b"interrupted copy",
    );
    // Killed after publication (before the association transaction begins)
    // and inside the association transaction (before it commits), twice each.
    let unprotected = BTreeMap::new();
    let mut observed = Vec::new();
    let mut bounded = None;
    for round in 1..=2 {
        let what = format!("round {round}, kill after publication");
        let copy = library
            .kill_after_publication(&BTreeSet::new(), round == 1)
            .await;
        assert_interrupted(&library, &copy, &unprotected, &mut bounded, &what).await;
        observed.push(copy);
        let what = format!("round {round}, kill before the association commits");
        let copy = library
            .kill_before_association_commit(&BTreeSet::new())
            .await;
        assert_interrupted(&library, &copy, &unprotected, &mut bounded, &what).await;
        observed.push(copy);
    }
    // The record's commit rejected on several consecutive starts.
    fail_meta_write(&library, "upgrade_snapshot").await;
    let mut bounded = None;
    for attempt in 1..=3 {
        library
            .refuse(&client, "an unrecorded pre-upgrade copy must stop startup")
            .await;
        let pool = library.pool().await;
        assert_eq!(
            max_migration(&pool).await,
            ALPHA6_LAST_MIGRATION,
            "attempt {attempt}: no migration runs before the copy is recorded"
        );
        assert_eq!(meta(&pool, SYNC_REPAIR_MARKERS[0]).await, None);
        assert_eq!(meta(&pool, AUTHORITY_MARKER).await, None);
        pool.close().await;
        let files = (library.copies().len(), library.partials().len());
        assert!(
            files.0 <= 1 && files.1 <= 1,
            "attempt {attempt}: at most one unrecorded copy and one work file: {:?}",
            library.files_with_prefix()
        );
        assert_eq!(
            *bounded.get_or_insert(files),
            files,
            "attempt {attempt}: repeated rejections add no files"
        );
    }

    clear_faults(&library).await;
    fail_migration(&library, UPGRADE_MIGRATION).await;
    library
        .refuse(&client, "the upgrade stops at the 091 fault")
        .await;
    let copies = library.copy_checksums();
    assert_eq!(
        copies.len(),
        1,
        "the orphan is recovered or reclaimed: {copies:?}"
    );
    assert!(library.partials().is_empty(), "no work file is left");
    let name = copies.keys().next().unwrap().clone();
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    assert!(
        record.contains(&name),
        "the attempt's record names its copy {name}: {record:?}"
    );
    pool.close().await;
    assert_recovered_or_reclaimed(&library, &observed, &name);
    clear_faults(&library).await;
    library
        .serve_through_cover_pass(&client, "the recorded upgrade must convert and serve")
        .await;
    let mut settle = library.serve(&client, "the converted library serves").await;
    settle.stop();
    assert_eq!(
        library.copy_checksums(),
        copies,
        "the recorded copy is reused"
    );

    // A later release: the previous rollback copy must survive the same kills
    // and rejections.
    library
        .execute(
            "INSERT INTO _livrarr_meta (key, value) VALUES ('startup_generation', 'upgrade-test-older-release') \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .await;
    let protected: BTreeSet<String> = copies.keys().cloned().collect();
    let mut later_observed = Vec::new();
    let mut bounded = None;
    let what = "later release, kill after publication";
    let copy = library.kill_after_publication(&protected, true).await;
    assert_interrupted(&library, &copy, &copies, &mut bounded, what).await;
    later_observed.push(copy);
    let what = "later release, kill before the association commits";
    let copy = library.kill_before_association_commit(&protected).await;
    assert_interrupted(&library, &copy, &copies, &mut bounded, what).await;
    later_observed.push(copy);
    fail_meta_write(&library, "upgrade_snapshot").await;
    let mut bounded = None;
    for attempt in 1..=3 {
        library
            .refuse(
                &client,
                "an unrecorded copy for the later upgrade must stop startup",
            )
            .await;
        let what = format!("later release, rejected record {attempt}");
        assert_bounded(&library, &copies, &mut bounded, &what);
    }
    clear_faults(&library).await;
    let mut server = library
        .serve(&client, "the later upgrade must record its copy and serve")
        .await;
    server.stop();
    let now = library.copy_checksums();
    assert_eq!(now.get(&name), copies.get(&name));
    let newest: Vec<&String> = now.keys().filter(|key| **key != name).collect();
    assert_eq!(newest.len(), 1, "one copy for the later upgrade: {now:?}");
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    assert!(
        record.contains(newest[0].as_str()),
        "the later attempt names its copy: {record:?}"
    );
    pool.close().await;
    assert_recovered_or_reclaimed(&library, &later_observed, newest[0]);
}

// ── AC-015 (R2-02) ──────────────────────────────────────────────────────────

#[tokio::test]
async fn missing_original_copy_stops_the_upgrade() {
    let library = Library::alpha6().await;
    let client = client();
    fail_migration(&library, UPGRADE_MIGRATION).await;
    library
        .refuse(&client, "the upgrade stops at the 091 fault")
        .await;
    let copies = library.copies();
    assert_eq!(copies.len(), 1, "precondition: one original copy");
    let original = name_of(&copies[0]);
    let pool = library.pool().await;
    assert_eq!(max_migration(&pool).await, LAST_PUBLISHED_MIGRATION);
    pool.close().await;

    // The library moves on without its sibling copy.
    std::fs::remove_file(&copies[0]).unwrap();
    clear_faults(&library).await;
    let log = library
        .refuse(
            &client,
            "an upgrade whose original copy is gone after progress must stop",
        )
        .await;
    assert_eq!(
        library.files_with_prefix(),
        Vec::<PathBuf>::new(),
        "no later state is saved as the pre-upgrade copy"
    );
    let pool = library.pool().await;
    assert!(
        !has_migration(&pool, UPGRADE_MIGRATION).await,
        "no further migration runs"
    );
    assert_eq!(
        meta(&pool, SYNC_REPAIR_MARKERS[0]).await,
        None,
        "no repair runs"
    );
    assert_eq!(meta(&pool, AUTHORITY_MARKER).await, None);
    pool.close().await;
    let lower = log.to_lowercase();
    assert!(
        log.contains(&original)
            || (lower.contains("original")
                && (lower.contains("missing") || lower.contains("unavailable"))),
        "the refusal names the missing original copy {original}:\n{log}"
    );

    // Separately: before any upgrade change commits, a retry retakes the copy.
    let untouched = Library::alpha6().await;
    fail_migration(&untouched, 74).await;
    untouched
        .refuse(&client, "a fault at the first new migration stops startup")
        .await;
    let pool = untouched.pool().await;
    assert_eq!(max_migration(&pool).await, ALPHA6_LAST_MIGRATION);
    pool.close().await;
    clear_faults(&untouched).await;
    untouched
        .execute("UPDATE works SET title = 'Dune (edited before retry)' WHERE id = 2")
        .await;
    fail_migration(&untouched, UPGRADE_MIGRATION).await;
    untouched
        .refuse(&client, "the retried upgrade stops at the 091 fault")
        .await;
    let retaken = untouched.copies();
    assert_eq!(
        retaken.len(),
        1,
        "the copy is retaken, not added: {retaken:?}"
    );
    assert!(untouched.partials().is_empty());
    let (_copy_dir, copy) = open_copy(&retaken[0]).await;
    let title: String = sqlx::query_scalar("SELECT title FROM works WHERE id = 2")
        .fetch_one(&copy)
        .await
        .unwrap();
    assert_eq!(
        title, "Dune (edited before retry)",
        "the retaken copy has the edit"
    );
    copy.close().await;
}

// ── AC-016 (R2-03) ──────────────────────────────────────────────────────────

#[tokio::test]
async fn partial_cover_repair_keeps_originals_protected() {
    let library = Library::alpha6().await;
    let client = client();
    let pool = library.pool().await;
    let control_before = audiobook_cover_row(&pool).await;
    pool.close().await;
    // The Goodreads cover repair commits its queue and slot changes but can
    // never stamp its completion marker.
    fail_meta_write(&library, COVER_REPAIR_MARKER).await;
    library
        .serve_through_cover_pass(&client, "the upgraded library must serve")
        .await;
    let pool = library.pool().await;
    assert_eq!(meta(&pool, COVER_REPAIR_MARKER).await, None);
    let queued = count(
        &pool,
        "SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue",
    )
    .await;
    assert!(
        queued > 0 || audiobook_cover_row(&pool).await != control_before,
        "precondition: the partial cover pass committed durable progress"
    );
    pool.close().await;
    let original = library.copy_checksums();
    assert_eq!(original.len(), 1, "one pre-upgrade copy: {original:?}");

    library
        .serve_through_cover_pass(&client, "the library restarts with the repair pending")
        .await;
    assert_eq!(
        library.copy_checksums(),
        original,
        "a restart does not retake the original from the mutated database"
    );

    // A later release upgrades while the cover repair is still incomplete.
    library
        .execute(
            "INSERT INTO _livrarr_meta (key, value) VALUES ('startup_generation', 'upgrade-test-older-release') \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .await;
    for name in PLANTED_COPIES {
        library.plant(name, name.as_bytes());
    }
    let before_later = copy_names(&library);
    library
        .serve_through_cover_pass(&client, "the later upgrade must serve")
        .await;
    // The copies this attempt requires, captured as soon as they exist.
    let required: BTreeMap<String, String> = library
        .copy_checksums()
        .into_iter()
        .filter(|(name, _)| !PLANTED_COPIES.contains(&name.as_str()))
        .collect();
    for (name, checksum) in &original {
        assert_eq!(
            required.get(name),
            Some(checksum),
            "the original stays protected while its repair is incomplete: {required:?}"
        );
    }
    let required_names: BTreeSet<String> = required.keys().cloned().collect();
    let expected = retained_after_prune(
        &before_later.union(&required_names).cloned().collect(),
        &required_names,
    );
    assert_eq!(
        copy_names(&library),
        expected,
        "the prune keeps every required copy and the three newest, and removes the rest"
    );
    library
        .serve_through_cover_pass(&client, "the later upgrade retries")
        .await;
    let retried = library.copy_checksums();
    assert_eq!(
        retried.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "a retry within the attempt adds no copy and removes no required one"
    );
    for (name, checksum) in &required {
        assert_eq!(
            retried.get(name),
            Some(checksum),
            "required copy {name} is never retaken"
        );
    }

    cover_only_attempt_keeps_its_snapshot(&client).await;

    // The permanently disabled title repair never blocks completion.
    let complete = Library::alpha6().await;
    without_machine_goodreads_cover(&complete).await;
    let log = complete
        .serve_through_cover_pass(&client, "the upgraded library must serve")
        .await;
    let mut settle = complete
        .serve(&client, "the upgraded library restarts")
        .await;
    settle.stop();
    let pool = complete.pool().await;
    assert!(
        meta(&pool, COVER_REPAIR_MARKER).await.is_some(),
        "precondition: the cover repair completed:\n{log}"
    );
    assert_eq!(
        meta(&pool, TITLE_REPAIR_MARKER).await,
        None,
        "precondition: the title repair is disabled"
    );
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    assert!(
        record.contains("complete") && !record.contains("in_progress"),
        "the upgrade completes without the disabled title repair: {record:?}"
    );
    pool.close().await;
}

async fn sync_marker_values(pool: &SqlitePool) -> Vec<Option<String>> {
    let mut values = Vec::new();
    for marker in SYNC_REPAIR_MARKERS {
        values.push(meta(pool, marker).await);
    }
    values
}

fn is_complete(record: &str) -> bool {
    record.contains("complete") && !record.contains("in_progress")
}

/// R2-03 isolated: after a completed upgrade, a release whose only startup
/// change is the Goodreads cover repair. Its attempt moves no migration and no
/// tracked marker while the repair commits queue and slot changes, and its
/// copy must still be kept, not retaken from the changed library.
async fn cover_only_attempt_keeps_its_snapshot(client: &Client) {
    let library = Library::alpha6().await;
    without_machine_goodreads_cover(&library).await;
    library
        .serve_through_cover_pass(client, "the upgraded library must serve")
        .await;
    let mut settle = library.serve(client, "the upgraded library restarts").await;
    settle.stop();
    let rollback = library.copy_checksums();
    assert_eq!(rollback.len(), 1, "precondition: one rollback copy");
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    assert!(
        is_complete(&record),
        "precondition: the first upgrade completed: {record:?}"
    );
    pool.close().await;

    // The library as a release before the Goodreads cover repair left it: no
    // repair marker, a machine-chosen Goodreads audiobook cover and that
    // release's own generation.
    library
        .execute(&format!(
            "DELETE FROM _livrarr_meta WHERE key = '{COVER_REPAIR_MARKER}'; \
             UPDATE works SET audiobook_cover_url = 'https://images.gr-assets.com/books/900008l.jpg', \
                    audiobook_cover_source = 'goodreads', audiobook_cover_width = 500, \
                    audiobook_cover_height = 500, audiobook_cover_manual = 0 \
              WHERE id = {WORK_WITH_MACHINE_GOODREADS_COVER}; \
             INSERT INTO _livrarr_meta (key, value) VALUES ('startup_generation', 'upgrade-test-older-release') \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value;"
        ))
        .await;
    std::fs::write(
        library.dir().join("covers/1/8_audio.jpg"),
        FIXTURE_COVERS[3].1,
    )
    .unwrap();
    fail_meta_write(&library, COVER_REPAIR_MARKER).await;
    let pool = library.pool().await;
    let migrations = max_migration(&pool).await;
    let markers = sync_marker_values(&pool).await;
    let slot = audiobook_cover_row(&pool).await;
    pool.close().await;

    library
        .serve_through_cover_pass(client, "the cover-repair-only upgrade must serve")
        .await;
    let pool = library.pool().await;
    assert_eq!(
        max_migration(&pool).await,
        migrations,
        "no migration progress"
    );
    assert_eq!(
        sync_marker_values(&pool).await,
        markers,
        "no tracked repair marker moves"
    );
    assert_eq!(meta(&pool, COVER_REPAIR_MARKER).await, None);
    let queued = count(
        &pool,
        "SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue",
    )
    .await;
    assert!(
        queued > 0 || audiobook_cover_row(&pool).await != slot,
        "precondition: the cover pass committed unmarked progress"
    );
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    pool.close().await;
    let copies = library.copy_checksums();
    for (name, checksum) in &rollback {
        assert_eq!(
            copies.get(name),
            Some(checksum),
            "the prior rollback copy stays protected: {copies:?}"
        );
    }
    let attempt: Vec<&String> = copies
        .keys()
        .filter(|name| !rollback.contains_key(*name))
        .collect();
    assert_eq!(
        attempt.len(),
        1,
        "the cover-repair-only upgrade takes one copy: {copies:?}"
    );
    let snapshot = attempt[0].clone();
    assert!(
        record.contains(&snapshot) && record.contains("in_progress"),
        "the attempt names its copy and stays incomplete while cover work remains: {record:?}"
    );
    let (_copy_dir, copy) = open_copy(&library.dir().join(&snapshot)).await;
    assert_eq!(max_migration(&copy).await, migrations);
    assert_eq!(sync_marker_values(&copy).await, markers);
    assert_eq!(
        audiobook_cover_row(&copy).await,
        slot,
        "the copy holds the pre-repair cover"
    );
    assert_eq!(
        count(
            &copy,
            "SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue"
        )
        .await,
        0,
        "the copy predates the repair's queue"
    );
    copy.close().await;

    library
        .serve_through_cover_pass(client, "the library restarts with the cover repair pending")
        .await;
    assert_eq!(
        library.copy_checksums(),
        copies,
        "a restart keeps the same copy and checksum and the prior rollback copy"
    );
    let pool = library.pool().await;
    let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
    assert!(
        record.contains(&snapshot) && record.contains("in_progress"),
        "still the same incomplete attempt: {record:?}"
    );
    pool.close().await;

    // Prune pressure while both roles are required: the previous rollback copy
    // and the current attempt's copy, captured by role before any pruning, plus
    // four eligible copies whose names sort after both.
    let required = copies.clone();
    let required_names: BTreeSet<String> = required.keys().cloned().collect();
    assert_eq!(required_names.len(), 2, "two required roles: {required:?}");
    for name in PLANTED_COPIES {
        assert!(required_names.iter().all(|role| role.as_str() < name));
        library.plant(name, name.as_bytes());
    }
    let expected = retained_after_prune(&copy_names(&library), &required_names);
    assert!(!expected.contains(PLANTED_COPIES[0]));
    for attempt in 1..=2 {
        library
            .serve_through_cover_pass(client, "the library restarts under prune pressure")
            .await;
        let now = library.copy_checksums();
        assert_eq!(
            now.keys().cloned().collect::<BTreeSet<_>>(),
            expected,
            "start {attempt}: the prune keeps both required copies and the three newest, and \
             removes the oldest eligible copy"
        );
        for (name, checksum) in &required {
            assert_eq!(
                now.get(name),
                Some(checksum),
                "start {attempt}: required copy {name} is never retaken"
            );
        }
        let pool = library.pool().await;
        let record = meta(&pool, "upgrade_snapshot").await.unwrap_or_default();
        assert!(
            record.contains(&snapshot) && record.contains("in_progress"),
            "start {attempt}: the attempt stays incomplete while cover work remains: {record:?}"
        );
        pool.close().await;
    }
}

async fn audiobook_cover_row(pool: &SqlitePool) -> Vec<String> {
    rows(
        pool,
        "SELECT json_array(audiobook_cover_url, audiobook_cover_source, audiobook_cover_width, \
                audiobook_cover_height) FROM works WHERE id = 8",
    )
    .await
}
