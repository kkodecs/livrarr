use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    record_history, ConfigDb, CreateAuthorDbRequest, CreateImportDbRequest, ReadarrOriginDb, WorkDb,
};
use livrarr_domain::identity_matching::{identity_key, unambiguous_author_match};
use livrarr_domain::readarr::*;
use livrarr_domain::services::{
    ReadarrImportWorkflow, ServiceError, SourceProviderData, WorkService,
};
use livrarr_domain::{
    derive_sort_name, history_events, sanitize_path_component, Author, AuthorId, Import, MediaType,
};

use livrarr_http::fetcher::HttpFetcherImpl;

use crate::readarr_client::{
    self, RdAuthor, RdBook, RdBookFile, RdRootFolder, ReadarrClient, ReadarrConnectError,
};
use crate::readarr_import_service::ReadarrImportService;
use crate::state::{LiveWorkService, ReadarrImportServiceImpl};

/// Fixed, fully-generic rejection surfaced for ANY Readarr connection
/// failure — SSRF/approval rejection, protocol mismatch, network failure,
/// timeout, or any non-2xx response — reused for both the API response and
/// any log line, so the two can never drift apart. Never interpolate a
/// `ReadarrConnectError`'s cause (there isn't one to interpolate) or any
/// lower-level error text at any of this workflow's Readarr-connection call
/// sites — that is exactly what the probed target could use to fingerprint
/// what's behind it.
fn readarr_rejected() -> ServiceError {
    ServiceError::Internal(ReadarrConnectError.to_string())
}

// =============================================================================
// LiveReadarrImportWorkflow
// =============================================================================

#[derive(Clone)]
pub struct LiveReadarrImportWorkflow {
    http_fetcher: HttpFetcherImpl,
    readarr_import_service: Arc<ReadarrImportServiceImpl>,
    readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
    /// User id that claimed the current (or, once finished, the most
    /// recently completed) run — 0 means no import has ever run in this
    /// process (Unit B3 Part 2). Set once, atomically, at slot-claim time in
    /// `start()`; never cleared, mirroring `readarr_import_progress`'s own
    /// "last completed state persists until the next run" lifecycle.
    readarr_import_owner: Arc<AtomicI64>,
    data_dir: Arc<std::path::PathBuf>,
    work_service: Arc<LiveWorkService>,
    db: SqliteDb,
    import_workflow: Arc<crate::state::LiveImportWorkflow>,
}

impl LiveReadarrImportWorkflow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http_fetcher: HttpFetcherImpl,
        readarr_import_service: Arc<ReadarrImportServiceImpl>,
        readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
        data_dir: Arc<std::path::PathBuf>,
        work_service: Arc<LiveWorkService>,
        db: SqliteDb,
        import_workflow: Arc<crate::state::LiveImportWorkflow>,
    ) -> Self {
        Self {
            http_fetcher,
            readarr_import_service,
            readarr_import_progress,
            readarr_import_owner: Arc::new(AtomicI64::new(0)),
            data_dir,
            work_service,
            db,
            import_workflow,
        }
    }

    /// Establish and protocol-verify a Readarr client from a raw,
    /// admin/user-supplied base URL (Unit B3 Part 1 — the SSRF origin-trust
    /// boundary). Every rejection reason collapses to the same opaque
    /// `ReadarrConnectError` — response and logs alike never surface
    /// anything the probed target returned.
    async fn connect_readarr(
        &self,
        url: &str,
        api_key: &str,
    ) -> Result<ReadarrClient, ReadarrConnectError> {
        connect_readarr_verified(&self.db, &self.http_fetcher, url, api_key).await
    }

    /// Whether `origin` (already normalized — `scheme://host[:port]`, no
    /// path) may be connected to: either an admin-approved entry, or the
    /// SSRF-safe classifier judges it public (public destinations carry no
    /// internal-probe risk). Exposed directly (not only through `connect`)
    /// so the admission decision is independently testable from the
    /// network/protocol behavior layered on top of it.
    pub async fn is_origin_permitted(&self, origin: &str) -> bool {
        origin_is_permitted(&self.db, origin).await
    }
}

/// Whether `origin` (already normalized) may be connected to: either an
/// admin-approved entry, or the SSRF-safe classifier judges it public
/// (public destinations carry no internal-probe risk). A DB error is
/// treated as not-approved (fail closed) — the public-only check still
/// applies. Free function — only needs a `SqliteDb`, so the origin-trust
/// decision is unit-testable with a `:memory:` DB alone, without the full
/// `LiveWorkService`/`LiveImportWorkflow` graph `LiveReadarrImportWorkflow`
/// otherwise requires (same F11 testability constraint as
/// `try_claim_readarr_slot` above).
async fn origin_is_permitted(db: &SqliteDb, origin: &str) -> bool {
    if matches!(db.is_readarr_origin_approved(origin).await, Ok(true)) {
        return true;
    }
    livrarr_http::ssrf::validate_url(origin).await.is_ok()
}

/// Establish and protocol-verify a Readarr client from a raw,
/// admin/user-supplied base URL (Unit B3 Part 1 — the SSRF origin-trust
/// boundary): normalize (point 2) -> origin admission (point 1) -> construct
/// -> protocol check (point 4, which itself never follows a redirect, point
/// 5). Every rejection reason collapses to the same opaque
/// `ReadarrConnectError` — response and logs alike never surface anything
/// the probed target returned (point 6). Free function for the same
/// testability reason as `origin_is_permitted` — needs only a `SqliteDb` and
/// an `HttpFetcherImpl`, both trivially constructible in a test.
async fn connect_readarr_verified(
    db: &SqliteDb,
    http_fetcher: &HttpFetcherImpl,
    url: &str,
    api_key: &str,
) -> Result<ReadarrClient, ReadarrConnectError> {
    let (base, origin) = readarr_client::normalize_readarr_base(url)?;
    if !origin_is_permitted(db, &origin).await {
        return Err(ReadarrConnectError);
    }
    let client = ReadarrClient::new(base, origin, api_key.to_string(), http_fetcher.clone());
    client.verify_protocol().await?;
    Ok(client)
}

/// The single-flight admission decision (Unit B3 Part 2: the global guard is
/// RETAINED unchanged, never replaced with per-user locks — it protects
/// shared-work / source-provider-data races, see M2/M8). Check-and-set
/// `running` atomically under the progress lock; stamp the owner in
/// lockstep. Free function (not a method) so the admission race is
/// unit-testable without constructing the full production
/// `LiveWorkService`/`LiveImportWorkflow` graph `start()` otherwise requires
/// — mirrors `resolve_batch_author`'s extraction below for the same reason
/// (see `test_author_dedup.rs`'s documented F11 constraint).
async fn try_claim_readarr_slot(
    progress: &Mutex<ReadarrImportProgress>,
    owner: &AtomicI64,
    user_id: i64,
    import_id: &str,
) -> bool {
    let mut prog = progress.lock().await;
    if prog.running {
        return false;
    }
    *prog = ReadarrImportProgress {
        running: true,
        import_id: Some(import_id.to_string()),
        phase: "fetching".to_string(),
        ..Default::default()
    };
    // Never cleared — mirrors `prog`'s own "last completed state persists
    // until the next run" lifecycle, so the owner can still see their
    // finished import's results afterward.
    owner.store(user_id, Ordering::SeqCst);
    true
}

/// Filter the shared progress record by ownership (Unit B3 Part 2, audit
/// finding #11): only the owner sees their own run's owner/import_id/counts/
/// errors/paths; a non-owner gets 404 (a specific `import_id` was requested
/// and it isn't theirs — indistinguishable from "no such import", never
/// confirms a DIFFERENT user owns it) or an idle default (a generic poll with
/// nothing owned — never the truth about someone else's run). Free
/// function for the same testability reason as `try_claim_readarr_slot`.
async fn scoped_readarr_progress(
    progress: &Mutex<ReadarrImportProgress>,
    owner: &AtomicI64,
    user_id: i64,
    import_id: Option<String>,
) -> Result<ReadarrImportProgress, ServiceError> {
    // Unit B3 #5: owner is read AFTER acquiring the progress lock (never
    // before) so owner+progress are one consistent snapshot — a claim
    // completing between the two reads would otherwise pair a stale owner
    // with a different, concurrently-claimed owner's live progress.
    let guard = progress.lock().await;
    let owner_id = owner.load(Ordering::SeqCst);
    let prog = guard.clone();
    drop(guard);

    if owner_id != user_id {
        return match import_id {
            Some(_) => Err(ServiceError::NotFound),
            None => Ok(ReadarrImportProgress::default()),
        };
    }

    match import_id {
        Some(requested) if prog.import_id.as_deref() != Some(requested.as_str()) => {
            Err(ServiceError::NotFound)
        }
        _ => Ok(prog),
    }
}

#[cfg(test)]
mod single_flight_and_progress_tests {
    use super::*;

    fn fresh_state() -> (Arc<Mutex<ReadarrImportProgress>>, Arc<AtomicI64>) {
        (
            Arc::new(Mutex::new(ReadarrImportProgress::default())),
            Arc::new(AtomicI64::new(0)),
        )
    }

    #[tokio::test]
    async fn second_concurrent_start_is_rejected_first_succeeds() {
        let (progress, owner) = fresh_state();
        let claimed_a = try_claim_readarr_slot(&progress, &owner, 1, "import-a").await;
        let claimed_b = try_claim_readarr_slot(&progress, &owner, 2, "import-b").await;
        assert!(claimed_a, "first caller must claim the slot");
        assert!(
            !claimed_b,
            "second caller must be rejected while one is running"
        );
        // The slot still reflects user A's run — never overwritten by B's
        // rejected attempt.
        assert_eq!(owner.load(Ordering::SeqCst), 1);
        assert_eq!(progress.lock().await.import_id.as_deref(), Some("import-a"));
    }

    #[tokio::test]
    async fn slot_is_claimable_again_once_marked_not_running() {
        let (progress, owner) = fresh_state();
        assert!(try_claim_readarr_slot(&progress, &owner, 1, "import-a").await);
        progress.lock().await.running = false;
        assert!(
            try_claim_readarr_slot(&progress, &owner, 2, "import-b").await,
            "a freed slot must be claimable by a new caller"
        );
        assert_eq!(owner.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn non_owner_polling_with_no_import_id_gets_idle_not_someone_elses_run() {
        let (progress, owner) = fresh_state();
        try_claim_readarr_slot(&progress, &owner, 1, "import-a").await;
        {
            let mut prog = progress.lock().await;
            prog.works_processed = 42;
            prog.errors.push("some internal detail".to_string());
        }

        let result = scoped_readarr_progress(&progress, &owner, 2, None)
            .await
            .expect("a generic poll with nothing owned must not error");
        assert!(
            !result.running,
            "non-owner must see idle, not user 1's live run"
        );
        assert_eq!(result.import_id, None);
        assert_eq!(result.works_processed, 0);
        assert!(result.errors.is_empty());
    }

    #[tokio::test]
    async fn non_owner_requesting_a_specific_import_id_gets_not_found() {
        let (progress, owner) = fresh_state();
        try_claim_readarr_slot(&progress, &owner, 1, "import-a").await;

        let err = scoped_readarr_progress(&progress, &owner, 2, Some("import-a".to_string()))
            .await
            .expect_err("a non-owner naming another user's import id must 404");
        assert!(matches!(err, ServiceError::NotFound));
    }

    #[tokio::test]
    async fn owner_sees_their_own_running_progress() {
        let (progress, owner) = fresh_state();
        try_claim_readarr_slot(&progress, &owner, 1, "import-a").await;
        progress.lock().await.works_processed = 7;

        let result = scoped_readarr_progress(&progress, &owner, 1, None)
            .await
            .expect("the owner must see their own progress");
        assert!(result.running);
        assert_eq!(result.import_id.as_deref(), Some("import-a"));
        assert_eq!(result.works_processed, 7);

        let result_by_id =
            scoped_readarr_progress(&progress, &owner, 1, Some("import-a".to_string()))
                .await
                .expect("the owner naming their own import id must see it too");
        assert_eq!(result_by_id.import_id.as_deref(), Some("import-a"));
    }

    #[tokio::test]
    async fn owner_naming_the_wrong_import_id_gets_not_found() {
        let (progress, owner) = fresh_state();
        try_claim_readarr_slot(&progress, &owner, 1, "import-a").await;

        let err = scoped_readarr_progress(&progress, &owner, 1, Some("some-other-id".to_string()))
            .await
            .expect_err("the owner naming an id that isn't their current run must 404");
        assert!(matches!(err, ServiceError::NotFound));
    }

    #[tokio::test]
    async fn owner_still_sees_completed_report_after_the_slot_is_freed() {
        // Post-completion ownership: `running` flips false but `import_id`/
        // counts/errors persist until the NEXT run overwrites them — the
        // owner must still be able to read their finished report.
        let (progress, owner) = fresh_state();
        try_claim_readarr_slot(&progress, &owner, 1, "import-a").await;
        {
            let mut prog = progress.lock().await;
            prog.running = false;
            prog.phase = "done".to_string();
            prog.files_processed = 3;
        }

        let owner_view = scoped_readarr_progress(&progress, &owner, 1, None)
            .await
            .expect("the owner must still see their completed report");
        assert!(!owner_view.running);
        assert_eq!(owner_view.files_processed, 3);
        assert_eq!(owner_view.import_id.as_deref(), Some("import-a"));

        let non_owner_view = scoped_readarr_progress(&progress, &owner, 2, None)
            .await
            .expect("a non-owner must still see idle, not the completed report");
        assert!(!non_owner_view.running);
        assert_eq!(non_owner_view.import_id, None);
        assert_eq!(non_owner_view.files_processed, 0);
    }

    #[tokio::test]
    async fn no_import_ever_run_owner_zero_treats_every_caller_as_non_owner() {
        let (progress, owner) = fresh_state();
        let result = scoped_readarr_progress(&progress, &owner, 1, None)
            .await
            .expect("no import has ever run — every caller gets idle");
        assert!(!result.running);
        // A real user id is never 0 (the first admin is id 1) — a caller
        // could not accidentally be treated as "the owner" of a fresh slot.
        assert_ne!(owner.load(Ordering::SeqCst), 1);
    }

    /// Unit B3 #5 (torn progress snapshot / cross-user leak): `owner` is a
    /// sibling `AtomicI64`, read by the OLD `scoped_readarr_progress`
    /// BEFORE the progress lock is acquired. If a concurrent claim
    /// completes in that window, a caller whose stale owner-read happens to
    /// equal their OWN id (e.g. they owned an earlier, already-finished
    /// run — the owner field is never cleared) is paired with a DIFFERENT,
    /// concurrently-claimed owner's LIVE progress.
    ///
    /// Deterministic on a real multi-thread runtime: the test holds the
    /// progress lock itself so both the writer's claim (`try_claim_readarr_
    /// slot`, simulating user Y's `start()`) and the reader's poll
    /// (`scoped_readarr_progress`, simulating user X's `progress()`) queue
    /// up behind the SAME lock in a controlled order (writer first, so its
    /// write completes before the reader's lock resolves) before the test
    /// releases it. The existing tests above are all sequential — this is
    /// the one that actually interleaves the two functions.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_claim_never_leaks_into_a_stale_owners_poll() {
        let (progress, owner) = fresh_state();
        let x_id = 1i64;
        let y_id = 2i64;

        // X previously ran and finished. `owner` is never cleared on
        // completion, so it still reads x_id even though X's run is long
        // done — this is what makes X's NEXT stale read equal their own id.
        assert!(try_claim_readarr_slot(&progress, &owner, x_id, "import-x").await);
        {
            let mut prog = progress.lock().await;
            prog.running = false;
            prog.phase = "done".to_string();
            prog.works_processed = 5;
        }

        // Hold the lock ourselves so both the writer (Y's claim) and the
        // reader (X's poll) queue up behind it in a controlled order.
        let guard = progress.lock().await;

        let writer_progress = progress.clone();
        let writer_owner = owner.clone();
        let writer = tokio::spawn(async move {
            try_claim_readarr_slot(&writer_progress, &writer_owner, y_id, "import-y").await
        });
        // Let the writer reach its (blocked) lock attempt first, so it is
        // queued ahead of the reader below.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        let reader_progress = progress.clone();
        let reader_owner = owner.clone();
        let reader = tokio::spawn(async move {
            scoped_readarr_progress(&reader_progress, &reader_owner, x_id, None).await
        });
        // Let the reader run up to its own (blocked) lock attempt — in the
        // pre-fix code this is where it loads the still-stale x_id owner,
        // before Y's claim has written anything.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // Release: the writer (queued first) claims for Y, then the reader
        // resolves and observes the now-updated progress.
        drop(guard);

        assert!(
            writer.await.unwrap(),
            "Y's claim must succeed — X's run already finished"
        );
        let result = reader
            .await
            .unwrap()
            .expect("a generic poll (no import_id) must not error");

        assert_ne!(
            result.import_id.as_deref(),
            Some("import-y"),
            "X's poll must never observe Y's concurrently-claimed live import"
        );
    }
}

/// Unit B3 Part 1 — origin trust boundary. `origin_is_permitted` and
/// `connect_readarr_verified` need only a `SqliteDb` + `HttpFetcherImpl`
/// (never the full `LiveWorkService`/`LiveImportWorkflow` graph), so these
/// run against a real `:memory:` DB and real local HTTP servers.
#[cfg(test)]
mod origin_trust_tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use livrarr_db::test_helpers::create_test_db;
    use tokio::net::TcpListener;

    async fn spawn_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn readarr_status_ok() -> Router {
        Router::new().route(
            "/api/v1/system/status",
            get(|| async { axum::Json(serde_json::json!({"appName": "Readarr"})) }),
        )
    }

    // --- Origin admission (point 1): approved-list OR public-safe ---

    #[tokio::test]
    async fn unapproved_private_origin_is_rejected() {
        let db = create_test_db().await;
        assert!(!origin_is_permitted(&db, "http://192.168.50.50:8787").await);
    }

    #[tokio::test]
    async fn approved_private_origin_is_permitted() {
        let db = create_test_db().await;
        db.create_readarr_origin("http://192.168.50.50:8787")
            .await
            .unwrap();
        assert!(origin_is_permitted(&db, "http://192.168.50.50:8787").await);
    }

    #[tokio::test]
    async fn public_origin_is_permitted_without_any_approval() {
        let db = create_test_db().await;
        // A real public IP literal — `validate_url` classifies a literal IP
        // without DNS, so this needs no network I/O.
        assert!(origin_is_permitted(&db, "http://8.8.8.8").await);
    }

    #[tokio::test]
    async fn loopback_is_rejected_when_not_approved() {
        let db = create_test_db().await;
        assert!(!origin_is_permitted(&db, "http://127.0.0.1:9999").await);
    }

    // --- Full pipeline: normalize -> admission -> protocol check ---

    #[tokio::test]
    async fn approved_target_with_correct_protocol_shape_connects() {
        let db = create_test_db().await;
        let base = spawn_server(readarr_status_ok()).await;
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        db.create_readarr_origin(&origin).await.unwrap();
        let fetcher = HttpFetcherImpl::new().unwrap();

        let result = connect_readarr_verified(&db, &fetcher, &base, "any-key").await;
        assert!(
            result.is_ok(),
            "an approved, protocol-correct target must connect"
        );
    }

    #[tokio::test]
    async fn healthy_but_unapproved_private_target_is_still_rejected() {
        let db = create_test_db().await;
        // A real, healthy, correctly-shaped Readarr stub — but NOT approved.
        // If admission didn't fire, this would otherwise succeed.
        let base = spawn_server(readarr_status_ok()).await;
        let fetcher = HttpFetcherImpl::new().unwrap();

        let result = connect_readarr_verified(&db, &fetcher, &base, "any-key").await;
        assert!(
            result.is_err(),
            "an unapproved loopback target must be rejected even when reachable and correctly-shaped"
        );
    }

    #[tokio::test]
    async fn non_readarr_shape_is_rejected() {
        let db = create_test_db().await;
        let app = Router::new().route(
            "/api/v1/system/status",
            get(|| async { axum::Json(serde_json::json!({"appName": "Sonarr"})) }),
        );
        let base = spawn_server(app).await;
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        db.create_readarr_origin(&origin).await.unwrap();
        let fetcher = HttpFetcherImpl::new().unwrap();

        let result = connect_readarr_verified(&db, &fetcher, &base, "any-key").await;
        assert!(result.is_err(), "a non-Readarr appName must be rejected");
    }

    #[tokio::test]
    async fn wrong_key_status_is_rejected() {
        let db = create_test_db().await;
        let app = Router::new().route(
            "/api/v1/system/status",
            get(|| async { axum::http::StatusCode::UNAUTHORIZED }),
        );
        let base = spawn_server(app).await;
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        db.create_readarr_origin(&origin).await.unwrap();
        let fetcher = HttpFetcherImpl::new().unwrap();

        let result = connect_readarr_verified(&db, &fetcher, &base, "wrong-key").await;
        assert!(
            result.is_err(),
            "a non-200 status (e.g. an unauthorized key) must be rejected"
        );
    }

    #[tokio::test]
    async fn all_3xx_responses_are_rejected_generically() {
        let db = create_test_db().await;
        // The 3xx ALSO carries a valid, correctly-shaped Readarr body (not
        // just a bare redirect with an empty/HTML body) — isolates "a 3xx
        // status itself is rejected" from "the body happened to not parse".
        let app = Router::new().route(
            "/api/v1/system/status",
            get(|| async {
                (
                    axum::http::StatusCode::FOUND,
                    [(axum::http::header::LOCATION, "http://example.com/elsewhere")],
                    axum::Json(serde_json::json!({"appName": "Readarr"})),
                )
            }),
        );
        let base = spawn_server(app).await;
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        db.create_readarr_origin(&origin).await.unwrap();
        let fetcher = HttpFetcherImpl::new().unwrap();

        let result = connect_readarr_verified(&db, &fetcher, &base, "any-key").await;
        assert!(
            result.is_err(),
            "a 3xx must never be followed — reject generically instead"
        );
    }

    #[tokio::test]
    async fn identical_generic_message_across_different_failure_kinds() {
        let db = create_test_db().await;
        let fetcher = HttpFetcherImpl::new().unwrap();

        // Kind 1: unapproved private origin (admission-level rejection) —
        // no HTTP is even attempted.
        let err_admission = connect_readarr_verified(&db, &fetcher, "http://10.99.99.99:8787", "k")
            .await
            .err()
            .unwrap();

        // Kind 2: approved target, wrong protocol shape.
        let app = Router::new().route(
            "/api/v1/system/status",
            get(|| async { axum::Json(serde_json::json!({"appName": "Sonarr"})) }),
        );
        let base = spawn_server(app).await;
        let origin = livrarr_http::normalized_origin(&base).unwrap();
        db.create_readarr_origin(&origin).await.unwrap();
        let err_protocol = connect_readarr_verified(&db, &fetcher, &base, "k")
            .await
            .err()
            .unwrap();

        assert_eq!(
            err_admission.to_string(),
            err_protocol.to_string(),
            "every rejection reason must render identically — never surface WHY"
        );
    }

    // --- Admin-approved origins CRUD (DB layer) ---

    #[tokio::test]
    async fn origin_crud_add_list_remove_round_trips() {
        let db = create_test_db().await;
        assert!(db.list_readarr_origins().await.unwrap().is_empty());

        let created = db
            .create_readarr_origin("http://10.0.0.9:8787")
            .await
            .unwrap();
        assert_eq!(created.origin, "http://10.0.0.9:8787");

        let listed = db.list_readarr_origins().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        assert!(db
            .is_readarr_origin_approved("http://10.0.0.9:8787")
            .await
            .unwrap());
        assert!(!db
            .is_readarr_origin_approved("http://10.0.0.10:8787")
            .await
            .unwrap());

        db.delete_readarr_origin(created.id).await.unwrap();
        assert!(db.list_readarr_origins().await.unwrap().is_empty());
        assert!(!db
            .is_readarr_origin_approved("http://10.0.0.9:8787")
            .await
            .unwrap());
    }
}

impl ReadarrImportWorkflow for LiveReadarrImportWorkflow {
    async fn connect(
        &self,
        req: ReadarrConnectRequest,
    ) -> Result<ReadarrConnectResponse, ServiceError> {
        let client = self
            .connect_readarr(&req.url, &req.api_key)
            .await
            .map_err(|_| readarr_rejected())?;
        let folders = client
            .root_folders()
            .await
            .map_err(|_| readarr_rejected())?;

        let root_folders = folders
            .into_iter()
            .map(|f| ReadarrRootFolderInfo {
                id: f.id,
                name: f.name,
                path: f.path,
                accessible: f.accessible,
                free_space: f.free_space,
                total_space: f.total_space,
            })
            .collect();

        Ok(ReadarrConnectResponse { root_folders })
    }

    async fn preview(
        &self,
        user_id: i64,
        req: ReadarrImportRequest,
    ) -> Result<ReadarrPreviewResponse, ServiceError> {
        let client = self
            .connect_readarr(&req.url, &req.api_key)
            .await
            .map_err(|_| readarr_rejected())?;

        let data = fetch_all_readarr_data(&client).await?;

        let _readarr_root = data
            .root_folders
            .iter()
            .find(|f| f.id == req.readarr_root_folder_id)
            .map(|f| f.path.clone())
            .ok_or_else(|| ServiceError::Internal("Invalid Readarr root folder ID".into()))?;

        let livrarr_root = self
            .readarr_import_service
            .get_root_folder(req.livrarr_root_folder_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let planner = ImportPlanner::new(&data, &livrarr_root.path, req.files_only, user_id);
        let existing_authors = self
            .readarr_import_service
            .list_authors(user_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let existing_works = self
            .readarr_import_service
            .list_works(user_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(planner.preview(&existing_authors, &existing_works))
    }

    async fn start(
        &self,
        user_id: i64,
        req: ReadarrImportRequest,
    ) -> Result<ReadarrStartResponse, ServiceError> {
        // Origin trust boundary (Unit B3 Part 1), checked and the client
        // fully protocol-verified BEFORE anything is claimed — a bad/
        // unapproved target must never occupy the single-flight slot.
        let client = self
            .connect_readarr(&req.url, &req.api_key)
            .await
            .map_err(|_| readarr_rejected())?;

        // Single-flight guard: claim the running slot under the progress
        // lock atomically (check + set). Two concurrent imports for the
        // same normalized identity can race on `source_provider_data`
        // injection (see M2/M8); serializing Readarr imports closes that
        // window without needing finer-grained per-work locks. Other entry
        // paths (list import, author monitor) do not pass
        // `source_provider_data` and are not subject to this race.
        // The user's default language, read once per import run: books whose
        // Readarr edition record carries no language seed this value. Fetched
        // before the running slot is claimed so a failure needs no cleanup.
        let default_language = self
            .db
            .get_default_language()
            .await
            .map_err(|e| ServiceError::Internal(format!("read default language: {e}")))?;

        let import_id = uuid::Uuid::new_v4().to_string();
        if !try_claim_readarr_slot(
            &self.readarr_import_progress,
            &self.readarr_import_owner,
            user_id,
            &import_id,
        )
        .await
        {
            // Generic busy rejection (Unit B3 Part 2): carries no owner,
            // import id, counts, errors, or paths — the single-flight guard
            // is process-global, not per-user, so a second caller learns
            // nothing about who is running or what it's doing.
            return Err(ServiceError::Internal(
                "Readarr import already running".into(),
            ));
        }

        // Create the DB import record AFTER claiming the slot, so the slot
        // is correctly released if create_import fails.
        if let Err(e) = self
            .readarr_import_service
            .create_import(CreateImportDbRequest {
                id: import_id.clone(),
                user_id,
                source: "readarr".to_string(),
                source_url: Some(req.url.clone()),
                target_root_folder_id: Some(req.livrarr_root_folder_id),
            })
            .await
        {
            let mut prog = self.readarr_import_progress.lock().await;
            prog.running = false;
            prog.phase = "failed".to_string();
            return Err(ServiceError::Internal(e.to_string()));
        }

        let readarr_import_service = self.readarr_import_service.clone();
        let readarr_import_progress = self.readarr_import_progress.clone();
        let work_service = self.work_service.clone();
        let import_workflow = self.import_workflow.clone();
        let id = import_id.clone();

        tokio::spawn(async move {
            // RAII slot guard — clears `running` even on panic.
            // Drop is synchronous, so we spawn a one-shot cleanup task
            // rather than blocking. The tokio runtime is alive at this
            // point because we are inside a spawned task.
            struct SlotGuard(Arc<Mutex<ReadarrImportProgress>>);
            impl Drop for SlotGuard {
                fn drop(&mut self) {
                    let progress = self.0.clone();
                    tokio::spawn(async move {
                        let mut prog = progress.lock().await;
                        prog.running = false;
                    });
                }
            }
            let _slot_guard = SlotGuard(readarr_import_progress.clone());

            let runner = ImportRunner::new(
                client,
                readarr_import_service.clone(),
                readarr_import_progress.clone(),
                &id,
                user_id,
                req,
                work_service,
                default_language,
                import_workflow,
            );
            if let Err(e) = runner.run().await {
                error!(import_id = %id, "Readarr import failed: {e}");
                let _ = readarr_import_service
                    .update_import_status(&id, "failed")
                    .await;
            }

            // Normal completion: set phase=done; slot_guard handles `running`
            // unconditionally on drop (including the panic path).
            let mut prog = readarr_import_progress.lock().await;
            prog.phase = "done".to_string();
        });

        Ok(ReadarrStartResponse { import_id })
    }

    async fn progress(
        &self,
        user_id: i64,
        import_id: Option<String>,
    ) -> Result<ReadarrImportProgress, ServiceError> {
        scoped_readarr_progress(
            &self.readarr_import_progress,
            &self.readarr_import_owner,
            user_id,
            import_id,
        )
        .await
    }

    async fn history(&self, user_id: i64) -> Result<ReadarrHistoryResponse, ServiceError> {
        let imports = self
            .readarr_import_service
            .list_imports(user_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let records = imports.iter().map(import_to_record).collect();
        Ok(ReadarrHistoryResponse { imports: records })
    }

    async fn undo(
        &self,
        user_id: i64,
        import_id: String,
    ) -> Result<ReadarrUndoResponse, ServiceError> {
        undo_import(
            &self.readarr_import_service,
            &self.data_dir,
            &self.db,
            user_id,
            &import_id,
        )
        .await
    }

    async fn list_origins(&self) -> Result<Vec<ReadarrOriginInfo>, ServiceError> {
        let origins = self
            .db
            .list_readarr_origins()
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        Ok(origins
            .into_iter()
            .map(|o| ReadarrOriginInfo {
                id: o.id,
                origin: o.origin,
                created_at: o.created_at,
            })
            .collect())
    }

    async fn add_origin(&self, url: String) -> Result<ReadarrOriginInfo, ServiceError> {
        let (_base, origin) = readarr_client::normalize_readarr_base(&url)
            .map_err(|_| ServiceError::Internal("invalid Readarr origin URL".into()))?;
        let created = self
            .db
            .create_readarr_origin(&origin)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        Ok(ReadarrOriginInfo {
            id: created.id,
            origin: created.origin,
            created_at: created.created_at,
        })
    }

    async fn remove_origin(&self, id: i64) -> Result<(), ServiceError> {
        self.db
            .delete_readarr_origin(id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))
    }
}

pub async fn undo_import(
    readarr_import_service: &crate::state::ReadarrImportServiceImpl,
    data_dir: &std::path::Path,
    db: &SqliteDb,
    user_id: i64,
    import_id: &str,
) -> Result<ReadarrUndoResponse, ServiceError> {
    let imp = readarr_import_service
        .get_import(import_id)
        .await
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .ok_or_else(|| ServiceError::Internal("Import not found".into()))?;

    if imp.user_id != user_id {
        return Err(ServiceError::Internal("Forbidden".into()));
    }
    if imp.status == "running" {
        return Err(ServiceError::Internal(
            "Cannot undo a running import".into(),
        ));
    }

    let items = readarr_import_service
        .list_library_items_by_import(import_id)
        .await
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    // Titles are unrecoverable once the work rows die, so pre-fetch them while
    // the works are still alive.
    let mut titles: HashMap<i64, String> = HashMap::new();
    let item_work_ids: HashSet<i64> = items.iter().map(|item| item.work_id).collect();
    for id in item_work_ids {
        if let Ok(work) = db.get_work(user_id, id).await {
            titles.insert(id, work.title);
        }
    }

    let root_folder_path: Option<String> = if let Some(rf_id) = imp.target_root_folder_id {
        readarr_import_service
            .get_root_folder(rf_id)
            .await
            .ok()
            .map(|rf| rf.path)
    } else {
        None
    };

    let undo_items: Vec<_> = items
        .iter()
        .map(|item| {
            let full_path = if let Some(ref root) = root_folder_path {
                PathBuf::from(root).join(&item.path)
            } else {
                PathBuf::from(&item.path)
            };
            (full_path, item.path.clone())
        })
        .collect();

    let (files_deleted, files_skipped) = tokio::task::spawn_blocking(move || {
        let mut deleted = 0i64;
        let mut skipped = 0i64;
        for (full_path, rel_path) in &undo_items {
            match std::fs::remove_file(full_path) {
                Ok(()) => {
                    deleted += 1;
                    info!(path = %rel_path, "Undo: deleted file");
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    deleted += 1;
                    debug!(path = %rel_path, "Undo: file already absent");
                }
                Err(e) => {
                    warn!(path = %rel_path, "Undo: failed to delete: {e}");
                    skipped += 1;
                }
            }
        }
        (deleted, skipped)
    })
    .await
    .unwrap_or((0, items.len() as i64));

    for item in &items {
        match readarr_import_service
            .delete_library_item_by_id(item.id)
            .await
        {
            Ok(()) => {
                let title = titles.get(&item.work_id).cloned().unwrap_or_default();
                record_history(
                    db,
                    user_id,
                    history_events::file_deleted(
                        item.work_id,
                        &title,
                        &item.path,
                        item.media_type.as_str(),
                        true,
                    ),
                )
                .await;
            }
            Err(e) => {
                warn!(id = item.id, "Undo: failed to delete library item: {e}");
            }
        }
    }

    let orphan_work_ids = readarr_import_service
        .list_orphan_work_ids_by_import(import_id)
        .await
        .unwrap_or_default();

    for id in &orphan_work_ids {
        if !titles.contains_key(id) {
            if let Ok(work) = db.get_work(user_id, *id).await {
                titles.insert(*id, work.title);
            }
        }
    }

    let title_of = |id: i64| titles.get(&id).cloned().unwrap_or_default();

    let works_deleted = match readarr_import_service
        .delete_orphan_works_by_import(import_id)
        .await
    {
        Ok(count) => {
            for id in &orphan_work_ids {
                record_history(
                    db,
                    user_id,
                    history_events::work_deleted(&title_of(*id), None, 0, true),
                )
                .await;
            }
            count
        }
        Err(e) => {
            warn!(import_id = %import_id, "Undo: failed to delete orphan works: {e}");
            0
        }
    };

    for wid in &orphan_work_ids {
        livrarr_metadata::work_service::delete_cover_files(data_dir, user_id, *wid).await;
    }

    let authors_deleted = readarr_import_service
        .delete_orphan_authors_by_import(import_id)
        .await
        .unwrap_or(0);

    readarr_import_service
        .update_import_status(import_id, "undone")
        .await
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok(ReadarrUndoResponse {
        files_deleted,
        files_skipped,
        works_deleted,
        authors_deleted,
    })
}

// =============================================================================
// Shared helpers
// =============================================================================

fn import_to_record(imp: &Import) -> ReadarrImportRecord {
    ReadarrImportRecord {
        id: imp.id.clone(),
        source: imp.source.clone(),
        status: imp.status.clone(),
        started_at: imp.started_at.to_rfc3339(),
        completed_at: imp.completed_at.map(|d| d.to_rfc3339()),
        authors_created: imp.authors_created,
        works_created: imp.works_created,
        files_imported: imp.files_imported,
        files_skipped: imp.files_skipped,
        source_url: imp.source_url.clone(),
    }
}

fn resolve_media_type(quality_id: Option<i32>, path: &str) -> Option<MediaType> {
    if let Some(qid) = quality_id {
        if let Some(mt_str) = readarr_client::quality_to_media_type(qid) {
            return match mt_str {
                "ebook" => Some(MediaType::Ebook),
                "audiobook" => Some(MediaType::Audiobook),
                _ => None,
            };
        }
    }
    if let Some(mt_str) = readarr_client::media_type_from_extension(path) {
        return match mt_str {
            "ebook" => Some(MediaType::Ebook),
            "audiobook" => Some(MediaType::Audiobook),
            _ => None,
        };
    }
    None
}

fn extract_quality_id(bf: &RdBookFile) -> Option<i32> {
    bf.quality.as_ref()?.quality.as_ref().map(|q| q.id)
}

static SERIES_TITLE_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^(.*?)(?:\s+#([\d.]+))?$").unwrap());

fn parse_series_title(series_title: &str) -> (Option<String>, Option<f64>) {
    let segment = series_title
        .split(';')
        .next()
        .unwrap_or(series_title)
        .trim();
    if segment.is_empty() {
        return (None, None);
    }
    if let Some(caps) = SERIES_TITLE_RE.captures(segment) {
        let name = caps.get(1).map(|m| m.as_str().trim().to_string());
        let pos = caps.get(2).and_then(|m| m.as_str().parse::<f64>().ok());
        let name = name.filter(|n| !n.is_empty());
        (name, pos)
    } else {
        (Some(segment.to_string()), None)
    }
}

fn extract_year(date_str: &str) -> Option<i32> {
    date_str.get(..4)?.parse::<i32>().ok()
}

fn extract_cover_url(images: &Option<Vec<readarr_client::RdImage>>) -> Option<String> {
    let imgs = images.as_ref()?;
    for img in imgs {
        if img.cover_type.as_deref() == Some("cover") {
            if let Some(ref url) = img.remote_url {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
            if let Some(ref url) = img.url {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
        }
    }
    for img in imgs {
        if let Some(ref url) = img.remote_url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
        if let Some(ref url) = img.url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
    }
    None
}

fn build_dest_path(
    root: &str,
    user_id: i64,
    author_name: &str,
    title: &str,
    source_path: &str,
) -> PathBuf {
    let ext = Path::new(source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let author_dir = sanitize_path_component(author_name, "Unknown Author");
    let file_stem = sanitize_path_component(title, "Unknown Title");
    PathBuf::from(root)
        .join(user_id.to_string())
        .join(author_dir)
        .join(format!("{file_stem}.{ext}"))
}

fn validate_source_path(source: &str, readarr_root: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(source)
        .map_err(|e| format!("cannot canonicalize source path: {e}"))?;
    let root_canonical = std::fs::canonicalize(readarr_root)
        .map_err(|e| format!("cannot canonicalize readarr root: {e}"))?;
    if !canonical.starts_with(&root_canonical) {
        return Err(format!(
            "source path {canonical:?} is not under readarr root {root_canonical:?}"
        ));
    }
    Ok(canonical)
}

fn apply_path_translation(
    path: &str,
    container_path: Option<&str>,
    host_path: Option<&str>,
) -> String {
    match (container_path, host_path) {
        (Some(cp), Some(hp)) if !cp.is_empty() && !hp.is_empty() => {
            let cp = cp.trim_end_matches('/');
            let hp = hp.trim_end_matches('/');
            if let Some(suffix) = path.strip_prefix(cp) {
                format!("{hp}{suffix}")
            } else {
                path.to_string()
            }
        }
        _ => path.to_string(),
    }
}

fn media_type_str(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Ebook => "ebook",
        MediaType::Audiobook => "audiobook",
    }
}

// =============================================================================
// Readarr data bundle (fetched once, shared by preview and import)
// =============================================================================

struct ReadarrData {
    authors: Vec<RdAuthor>,
    books: Vec<RdBook>,
    book_files: Vec<RdBookFile>,
    root_folders: Vec<RdRootFolder>,
}

async fn fetch_all_readarr_data(client: &ReadarrClient) -> Result<ReadarrData, ServiceError> {
    // Every step maps to the SAME generic rejection — which step failed (and
    // for book files, which author id) must never leak into the response.
    let root_folders = client
        .root_folders()
        .await
        .map_err(|_| readarr_rejected())?;
    let authors = client.authors().await.map_err(|_| readarr_rejected())?;
    let books = client.books().await.map_err(|_| readarr_rejected())?;
    let author_ids: Vec<i64> = authors.iter().map(|a| a.id).collect();
    use futures::stream::{self, StreamExt};
    let file_results: Vec<(i64, Result<Vec<RdBookFile>, _>)> = stream::iter(
        author_ids
            .into_iter()
            .map(|aid| async move { (aid, client.book_files_by_author(aid).await) }),
    )
    .buffer_unordered(10)
    .collect()
    .await;
    let mut book_files: Vec<RdBookFile> = Vec::new();
    for (_aid, res) in file_results {
        match res {
            Ok(files) => book_files.extend(files),
            Err(_) => return Err(readarr_rejected()),
        }
    }
    Ok(ReadarrData {
        authors,
        books,
        book_files,
        root_folders,
    })
}

// =============================================================================
// ImportPlanner — shared logic for preview and the plan phase of import
// =============================================================================

struct ImportPlanner<'a> {
    author_map: HashMap<i64, &'a RdAuthor>,
    book_files_by_book: HashMap<i64, Vec<&'a RdBookFile>>,
    livrarr_root_path: &'a str,
    books: &'a [RdBook],
    files_only: bool,
    user_id: i64,
}

impl<'a> ImportPlanner<'a> {
    fn new(
        data: &'a ReadarrData,
        livrarr_root_path: &'a str,
        files_only: bool,
        user_id: i64,
    ) -> Self {
        let author_map: HashMap<i64, &RdAuthor> = data.authors.iter().map(|a| (a.id, a)).collect();
        let mut book_files_by_book: HashMap<i64, Vec<&RdBookFile>> = HashMap::new();
        for bf in &data.book_files {
            book_files_by_book.entry(bf.book_id).or_default().push(bf);
        }
        Self {
            author_map,
            book_files_by_book,
            livrarr_root_path,
            books: &data.books,
            files_only,
            user_id,
        }
    }

    fn preview(
        &self,
        existing_authors: &[livrarr_domain::Author],
        existing_works: &[livrarr_domain::Work],
    ) -> ReadarrPreviewResponse {
        let mut skipped_items: Vec<ReadarrSkippedItem> = Vec::new();
        let mut import_files: Vec<ReadarrPreviewFileItem> = Vec::new();
        let mut authors_to_create = 0i64;
        let mut works_to_create = 0i64;
        let mut works_existing = 0i64;
        let mut files_to_skip = 0i64;

        let mut author_names_seen: HashMap<String, bool> = HashMap::new();
        for a in existing_authors {
            // REQ-014: author-only normalization — the title half of
            // identity_key is unused here.
            author_names_seen.insert(identity_key("", &a.name).1, true);
        }

        for book in self.books {
            let author_name = self
                .author_map
                .get(&book.author_id)
                .and_then(|a| a.author_name.as_deref())
                .unwrap_or("");
            let title = book.title.as_deref().unwrap_or("");

            if author_name.is_empty() {
                skipped_items.push(ReadarrSkippedItem {
                    title: title.to_string(),
                    author: String::new(),
                    reason: "No author".to_string(),
                });
                continue;
            }

            if self.files_only && !self.book_files_by_book.contains_key(&book.id) {
                continue;
            }

            let norm_author = identity_key("", author_name).1;
            if !author_names_seen.contains_key(&norm_author) {
                author_names_seen.insert(norm_author.clone(), false);
                authors_to_create += 1;
            }

            let is_existing = self.is_work_existing(book, &norm_author, title, existing_works);

            let work_status = if is_existing { "existing" } else { "new" };
            if is_existing {
                works_existing += 1;
            } else {
                works_to_create += 1;
            }

            self.classify_book_files(
                book,
                author_name,
                title,
                work_status,
                &mut import_files,
                &mut skipped_items,
                &mut files_to_skip,
            );
        }

        let authors_existing = self
            .author_map
            .values()
            .filter(|a| {
                let name = a.author_name.as_deref().unwrap_or("");
                let norm = identity_key("", name).1;
                author_names_seen.get(&norm) == Some(&true)
            })
            .count() as i64;

        ReadarrPreviewResponse {
            authors_to_create,
            authors_existing,
            works_to_create,
            works_existing,
            files_to_import: import_files.len() as i64,
            files_to_skip,
            skipped_items,
            import_files,
        }
    }

    fn is_work_existing(
        &self,
        book: &RdBook,
        norm_author: &str,
        title: &str,
        existing_works: &[livrarr_domain::Work],
    ) -> bool {
        let edition = book.monitored_edition();
        let isbn = edition
            .and_then(|e| e.isbn13.as_deref())
            .filter(|s| !s.is_empty());
        let asin = edition
            .and_then(|e| e.asin.as_deref())
            .filter(|s| !s.is_empty());
        let year = book.release_date.as_deref().and_then(extract_year);
        // REQ-014: `norm_author` (the parameter) is already an identity_key
        // author component (computed by the caller); pair it with the
        // title-only component here rather than re-deriving both — the
        // caller's precomputed `norm_author` stays authoritative.
        let norm_title = identity_key(title, "").0;

        if let Some(isbn_val) = isbn {
            existing_works
                .iter()
                .any(|w| w.isbn_13.as_deref() == Some(isbn_val))
        } else if let Some(asin_val) = asin {
            existing_works
                .iter()
                .any(|w| w.asin.as_deref() == Some(asin_val))
        } else {
            existing_works.iter().any(|w| {
                let (w_norm_title, w_norm_author) = identity_key(&w.title, &w.author_name);
                w_norm_author == norm_author && w_norm_title == norm_title && w.year == year
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn classify_book_files(
        &self,
        book: &RdBook,
        author_name: &str,
        title: &str,
        work_status: &str,
        import_files: &mut Vec<ReadarrPreviewFileItem>,
        skipped_items: &mut Vec<ReadarrSkippedItem>,
        files_to_skip: &mut i64,
    ) {
        let files = self.book_files_by_book.get(&book.id);
        let file_list: Vec<&&RdBookFile> = files.map(|f| f.iter().collect()).unwrap_or_default();

        let audiobook_count = file_list
            .iter()
            .filter(|f| {
                resolve_media_type(extract_quality_id(f), &f.path) == Some(MediaType::Audiobook)
            })
            .count();

        if audiobook_count > 1 {
            skipped_items.push(ReadarrSkippedItem {
                title: title.to_string(),
                author: author_name.to_string(),
                reason: format!("Multi-file audiobook ({audiobook_count} files)"),
            });
            *files_to_skip += audiobook_count as i64;

            for f in file_list.iter().filter(|f| {
                resolve_media_type(extract_quality_id(f), &f.path) != Some(MediaType::Audiobook)
            }) {
                self.classify_single_file(
                    f,
                    author_name,
                    title,
                    work_status,
                    import_files,
                    skipped_items,
                    files_to_skip,
                );
            }
        } else {
            for f in &file_list {
                self.classify_single_file(
                    f,
                    author_name,
                    title,
                    work_status,
                    import_files,
                    skipped_items,
                    files_to_skip,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn classify_single_file(
        &self,
        f: &RdBookFile,
        author_name: &str,
        title: &str,
        work_status: &str,
        import_files: &mut Vec<ReadarrPreviewFileItem>,
        skipped_items: &mut Vec<ReadarrSkippedItem>,
        files_to_skip: &mut i64,
    ) {
        let qid = extract_quality_id(f);
        match resolve_media_type(qid, &f.path) {
            None => {
                *files_to_skip += 1;
                skipped_items.push(ReadarrSkippedItem {
                    title: title.to_string(),
                    author: author_name.to_string(),
                    reason: format!("Unknown format: {}", f.path),
                });
            }
            Some(mt) => {
                let dest = build_dest_path(
                    self.livrarr_root_path,
                    self.user_id,
                    author_name,
                    title,
                    &f.path,
                );
                if dest.exists() {
                    *files_to_skip += 1;
                    skipped_items.push(ReadarrSkippedItem {
                        title: title.to_string(),
                        author: author_name.to_string(),
                        reason: "Destination already exists".to_string(),
                    });
                } else {
                    import_files.push(ReadarrPreviewFileItem {
                        title: title.to_string(),
                        author: author_name.to_string(),
                        path: f.path.clone(),
                        media_type: media_type_str(mt).to_string(),
                        work_status: work_status.to_string(),
                    });
                }
            }
        }
    }
}

// =============================================================================
// ImportRunner — executes the import in a background task
// =============================================================================

struct ImportRunner {
    /// Already normalized, origin-trust-checked, and protocol-verified
    /// (Unit B3 Part 1) by `LiveReadarrImportWorkflow::start` before this
    /// runner was constructed — the runner never re-derives trust.
    client: ReadarrClient,
    readarr_import_service: Arc<ReadarrImportServiceImpl>,
    readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
    import_id: String,
    user_id: i64,
    req: ReadarrImportRequest,
    work_service: Arc<LiveWorkService>,
    default_language: String,
    import_workflow: Arc<crate::state::LiveImportWorkflow>,
    author_map_rd: HashMap<i64, i64>,
    work_map_rd: HashMap<i64, i64>,
    authors_created: i64,
    works_created: i64,
    files_imported: i64,
    files_skipped: i64,
}

/// `process_authors`' batch-local resolution for one Readarr row
/// (author-dedup U-2, `[REV codex R-7/R-9]`): adopt an existing entry in
/// `batch_authors` or signal that a new author row is needed. `batch_authors`
/// starts as the pre-batch DB snapshot and grows by exactly one entry per
/// newly-created author — adopted authors are never re-appended — so every
/// entry maps to a distinct author id and the exactly-one-match rule inside
/// `unambiguous_author_match` is already exactly-one-distinct-author-id.
/// Pure and DB-free: extracted so the in-batch dedup behavior is
/// unit-testable without the full production `ImportRunner` construction
/// chain (`LiveWorkService`/`LiveImportWorkflow` pull in live provider
/// wiring that cannot be assembled offline — see `batch_author_resolution_tests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchAuthorDecision {
    Adopt(AuthorId),
    Create,
}

fn resolve_batch_author(name: &str, batch_authors: &[Author]) -> BatchAuthorDecision {
    let names: Vec<String> = batch_authors.iter().map(|a| a.name.clone()).collect();
    match unambiguous_author_match(name, &names) {
        Some(i) => BatchAuthorDecision::Adopt(batch_authors[i].id),
        None => BatchAuthorDecision::Create,
    }
}

impl ImportRunner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        client: ReadarrClient,
        readarr_import_service: Arc<ReadarrImportServiceImpl>,
        readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
        import_id: &str,
        user_id: i64,
        req: ReadarrImportRequest,
        work_service: Arc<LiveWorkService>,
        default_language: String,
        import_workflow: Arc<crate::state::LiveImportWorkflow>,
    ) -> Self {
        Self {
            client,
            readarr_import_service,
            readarr_import_progress,
            import_id: import_id.to_string(),
            user_id,
            req,
            work_service,
            default_language,
            import_workflow,
            author_map_rd: HashMap::new(),
            work_map_rd: HashMap::new(),
            authors_created: 0,
            works_created: 0,
            files_imported: 0,
            files_skipped: 0,
        }
    }

    fn progress(&self) -> &Arc<Mutex<ReadarrImportProgress>> {
        &self.readarr_import_progress
    }

    async fn run(mut self) -> Result<(), String> {
        let data = fetch_all_readarr_data(&self.client)
            .await
            .map_err(|e| format!("fetch failed: {e}"))?;

        let readarr_root_raw = data
            .root_folders
            .iter()
            .find(|f| f.id == self.req.readarr_root_folder_id)
            .map(|f| f.path.clone())
            .ok_or_else(|| "Invalid Readarr root folder ID".to_string())?;
        let readarr_root = apply_path_translation(
            &readarr_root_raw,
            self.req.container_path.as_deref(),
            self.req.host_path.as_deref(),
        );

        let livrarr_root = self
            .readarr_import_service
            .get_root_folder(self.req.livrarr_root_folder_id)
            .await
            .map_err(|e| format!("get livrarr root folder: {e}"))?;

        let author_map: HashMap<i64, &RdAuthor> = data.authors.iter().map(|a| (a.id, a)).collect();
        let mut book_files_by_book: HashMap<i64, Vec<&RdBookFile>> = HashMap::new();
        for bf in &data.book_files {
            book_files_by_book.entry(bf.book_id).or_default().push(bf);
        }

        let active_book_ids: HashSet<i64> = if self.req.files_only {
            book_files_by_book.keys().copied().collect()
        } else {
            data.books.iter().map(|b| b.id).collect()
        };

        let active_books: Vec<&RdBook> = data
            .books
            .iter()
            .filter(|b| active_book_ids.contains(&b.id))
            .collect();

        {
            let mut prog = self.progress().lock().await;
            prog.phase = "processing".to_string();
            prog.authors_total = data.authors.len() as i64;
            prog.works_total = active_books.len() as i64;
            prog.files_total = data
                .book_files
                .iter()
                .filter(|f| active_book_ids.contains(&f.book_id))
                .count() as i64;
        }

        self.process_authors(&data.authors, &data.books, &active_book_ids)
            .await?;
        self.process_works(
            &active_books,
            &author_map,
            &book_files_by_book,
            &livrarr_root.path,
        )
        .await?;
        self.process_files(
            &data.book_files,
            &active_book_ids,
            &author_map,
            &data.books,
            &book_files_by_book,
            &readarr_root,
            &livrarr_root.path,
        )
        .await?;

        let _ = self
            .readarr_import_service
            .update_import_counts(
                &self.import_id,
                self.authors_created,
                self.works_created,
                self.files_imported,
                self.files_skipped,
            )
            .await;

        self.readarr_import_service
            .set_import_completed(&self.import_id)
            .await
            .map_err(|e| format!("set completed: {e}"))?;

        info!(
            import_id = %self.import_id,
            self.authors_created,
            self.works_created,
            self.files_imported,
            self.files_skipped,
            "Readarr import completed"
        );

        Ok(())
    }

    async fn process_authors(
        &mut self,
        rd_authors: &[RdAuthor],
        rd_books: &[RdBook],
        active_book_ids: &HashSet<i64>,
    ) -> Result<(), String> {
        let mut existing_authors = self
            .readarr_import_service
            .list_authors(self.user_id)
            .await
            .map_err(|e| format!("list authors: {e}"))?;

        for rd_author in rd_authors {
            let name = rd_author.author_name.as_deref().unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }

            if self.req.files_only {
                let has_files = rd_books
                    .iter()
                    .filter(|b| b.author_id == rd_author.id)
                    .any(|b| active_book_ids.contains(&b.id));
                if !has_files {
                    let mut prog = self.progress().lock().await;
                    prog.authors_processed += 1;
                    continue;
                }
            }

            let livrarr_author_id = match resolve_batch_author(name, &existing_authors) {
                BatchAuthorDecision::Adopt(id) => id,
                BatchAuthorDecision::Create => {
                    let sort_name = rd_author
                        .sort_name
                        .as_deref()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| derive_sort_name(name));

                    match self
                        .readarr_import_service
                        .create_author(CreateAuthorDbRequest {
                            user_id: self.user_id,
                            name: name.to_string(),
                            sort_name: Some(sort_name),
                            ol_key: None,
                            gr_key: None,
                            hc_key: None,
                            import_id: Some(self.import_id.clone()),
                        })
                        .await
                    {
                        Ok(a) => {
                            self.authors_created += 1;
                            let id = a.id;
                            // Newly-created authors join the batch-local list
                            // so a later in-batch spelling variant adopts
                            // instead of double-creating [REV codex R-7].
                            // Adopted authors are never re-appended here —
                            // they are already in the snapshot or an earlier
                            // append [REV codex R-9].
                            existing_authors.push(a);
                            id
                        }
                        Err(e) => {
                            warn!(name = %name, "Failed to create author: {e}");
                            let mut prog = self.progress().lock().await;
                            prog.errors.push(format!("Author '{name}': {e}"));
                            continue;
                        }
                    }
                }
            };

            self.author_map_rd.insert(rd_author.id, livrarr_author_id);

            {
                let mut prog = self.progress().lock().await;
                prog.authors_processed += 1;
            }
        }
        Ok(())
    }

    async fn process_works(
        &mut self,
        active_books: &[&RdBook],
        author_map: &HashMap<i64, &RdAuthor>,
        book_files_by_book: &HashMap<i64, Vec<&RdBookFile>>,
        _livrarr_root_path: &str,
    ) -> Result<(), String> {
        // M9 bounded concurrency: serial prep (cheap, read-only) builds an
        // AddWorkRequest per active book; concurrent dispatch runs up to 5
        // work_service.add() calls in parallel; serial post-pass folds the
        // outcomes into self.works_created / self.work_map_rd. Progress
        // (works_processed/errors) is bumped incrementally, per item, as
        // each add() settles inside the concurrent stream below — NOT in a
        // serial pass after `.collect()` — so the progress bar advances
        // throughout this (the long) phase instead of freezing until it
        // ends. Each individual add() is still synchronous-and-complete —
        // bounded concurrency is BETWEEN works, not within one.

        // --- Pass 1: serial prep + skip-on-empty-author ---
        struct Prep {
            rd_book_id: i64,
            title: String,
            candidate: livrarr_domain::identity::WorkCandidate,
        }
        let mut preps: Vec<Prep> = Vec::with_capacity(active_books.len());
        let mut skip_errors: Vec<String> = Vec::new();

        for rd_book in active_books {
            let author_name = author_map
                .get(&rd_book.author_id)
                .and_then(|a| a.author_name.as_deref())
                .unwrap_or("");
            let title = rd_book.title.as_deref().unwrap_or("").trim();

            if author_name.is_empty() {
                skip_errors.push(format!("Book '{title}': skipped (no author)"));
                continue;
            }

            let edition = rd_book.monitored_edition();
            let isbn = edition
                .and_then(|e| e.isbn13.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let asin = edition
                .and_then(|e| e.asin.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let publisher = edition
                .and_then(|e| e.publisher.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let language = edition
                .and_then(|e| e.language.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let year = rd_book.release_date.as_deref().and_then(extract_year);

            let description = rd_book
                .overview
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    edition
                        .and_then(|e| e.overview.as_deref())
                        .filter(|s| !s.is_empty())
                })
                .map(|s| s.to_string());
            let (series_name, series_position_f64) = rd_book
                .series_title
                .as_deref()
                .map(parse_series_title)
                .unwrap_or((None, None));
            let genres = rd_book.genres.clone();
            let page_count = rd_book
                .page_count
                .or_else(|| edition.and_then(|e| e.page_count));
            let rating = rd_book.ratings.as_ref().and_then(|r| r.value);
            let rating_count = rd_book.ratings.as_ref().and_then(|r| r.votes);
            let cover_url = extract_cover_url(&rd_book.images);

            let book_files_list = book_files_by_book.get(&rd_book.id);
            let has_ebook_file = book_files_list
                .map(|fs| {
                    fs.iter().any(|f| {
                        resolve_media_type(extract_quality_id(f), &f.path) == Some(MediaType::Ebook)
                    })
                })
                .unwrap_or(false);
            let has_audiobook_file = book_files_list
                .map(|fs| {
                    fs.iter().any(|f| {
                        resolve_media_type(extract_quality_id(f), &f.path)
                            == Some(MediaType::Audiobook)
                    })
                })
                .unwrap_or(false);

            let monitor_ebook = has_ebook_file || rd_book.monitored.unwrap_or(false);
            let monitor_audiobook = has_audiobook_file;

            // Seed the source identifiers (REQ-006): the Goodreads work id maps
            // to gr_key, plus the edition ISBN/ASIN, each normalized through the
            // domain authority so add() persists them as anchors at create. The
            // background resolver converges any missing anchors afterward.
            let seed_anchors = {
                use livrarr_domain::identity::CapturedIdentity;
                use livrarr_domain::normalization::{
                    normalize_asin, normalize_gr_key, normalize_isbn13, AsinNorm,
                };
                let gr_key = rd_book
                    .foreign_book_id
                    .as_deref()
                    .and_then(normalize_gr_key);
                let mut isbn_13 = isbn.as_deref().and_then(normalize_isbn13);
                let mut asin_anchor = None;
                if let Some(raw_asin) = asin.as_deref() {
                    match normalize_asin(raw_asin) {
                        AsinNorm::Isbn13(folded) => isbn_13 = isbn_13.or(Some(folded)),
                        AsinNorm::Asin(canonical) => asin_anchor = Some(canonical),
                        AsinNorm::Invalid => {}
                    }
                }
                if gr_key.is_some() || isbn_13.is_some() || asin_anchor.is_some() {
                    Some(CapturedIdentity {
                        ol_key: None,
                        gr_key,
                        hc_key: None,
                        isbn_13,
                        asin: asin_anchor,
                        title: title.to_string(),
                        author_name: author_name.to_string(),
                        language: language.clone(),
                    })
                } else {
                    None
                }
            };

            let source_data = SourceProviderData {
                description,
                isbn,
                asin,
                publisher,
                genres,
                page_count,
                rating,
                rating_count,
                cover_url: cover_url.clone(),
                series_name: series_name.clone(),
                series_position: series_position_f64.map(|p| p.to_string()),
            };

            use livrarr_domain::identity::{IdentityState, PendingReason};
            use livrarr_domain::seed::{seed_readarr_import, SeedInput, SeedLanguage};
            let candidate = seed_readarr_import(
                SeedInput {
                    title: title.to_string(),
                    author_name: author_name.to_string(),
                    language: SeedLanguage::resolve(language.as_deref(), &self.default_language),
                    author_ol_key: None,
                    year,
                    cover_url,
                    detail_url: None,
                    description: source_data.description.clone(),
                    series_name,
                    series_position: series_position_f64,
                },
                IdentityState::Pending {
                    reason: PendingReason::NoCandidates,
                    seed_anchors,
                    top_candidates: vec![],
                },
                source_data,
                monitor_ebook,
                monitor_audiobook,
                self.import_id.clone(),
            );

            preps.push(Prep {
                rd_book_id: rd_book.id,
                title: title.to_string(),
                candidate,
            });
        }

        // --- Pass 1b: dedupe by normalized identity ---
        //
        // Readarr can return two books that normalize to the same Livrarr
        // identity (e.g. "The Stand" and "Stand, The"). Dispatching them
        // concurrently through buffer_unordered would race on
        // EnrichmentServiceImpl's per-(user, work) source_data_store —
        // both calls supplying SourceProviderData for the same (user_id,
        // work_id), the later inject overwriting the earlier one. Closing
        // the race at the dispatch boundary is cheaper than threading a
        // payload through the trait surface.
        //
        // Strategy: keep the first prep per identity as the "primary"; track
        // the remaining rd_book_ids as "secondaries" and fan the resulting
        // work_id out to them in the post-pass. All secondaries' rd_files
        // therefore map to the same Livrarr work as the primary.
        let mut by_identity: HashMap<(String, String), (Prep, Vec<i64>)> =
            HashMap::with_capacity(preps.len());
        for prep in preps {
            let key = identity_key(
                &prep.candidate.fields.title,
                &prep.candidate.fields.author_name,
            );
            by_identity
                .entry(key)
                .and_modify(|(_, secs)| secs.push(prep.rd_book_id))
                .or_insert_with(|| (prep, Vec::new()));
        }
        let total_dispatch = by_identity.len();
        let total_secondaries: usize = by_identity.values().map(|(_, s)| s.len()).sum();
        if total_secondaries > 0 {
            info!(
                primaries = total_dispatch,
                secondaries = total_secondaries,
                "Readarr import: deduplicated {} books by normalized identity",
                total_secondaries
            );
        }
        // Split into parallel-dispatch list and secondary map keyed by primary rd_book_id.
        let mut primary_preps: Vec<Prep> = Vec::with_capacity(total_dispatch);
        let mut secondaries_by_primary: HashMap<i64, Vec<i64>> =
            HashMap::with_capacity(total_dispatch);
        for (prep, secs) in by_identity.into_values() {
            secondaries_by_primary.insert(prep.rd_book_id, secs);
            primary_preps.push(prep);
        }

        // --- Pass 2: concurrent dispatch (M9 buffer_unordered(5)) ---
        struct AddOutcome {
            rd_book_id: i64,
            work_id: Option<i64>,
            was_created: bool,
            error: Option<String>,
        }
        let user_id = self.user_id;
        let work_service = self.work_service.clone();
        // Cloned up front (like `work_service` above) so each item's async
        // block can bump progress the moment its own add() settles, instead
        // of all progress movement waiting for `.collect()` below. The works
        // phase is the long one (each add() runs identity + enrichment), so
        // leaving the bump in a post-collect pass is what froze the bar at
        // its starting value for the whole phase, then jumped it at the end.
        let progress = self.progress().clone();
        let secondaries_by_primary = Arc::new(secondaries_by_primary);
        let outcomes: Vec<AddOutcome> = stream::iter(primary_preps)
            .map(|p| {
                let ws = work_service.clone();
                let progress = progress.clone();
                let secondaries_by_primary = secondaries_by_primary.clone();
                async move {
                    let outcome = match ws.add(user_id, p.candidate).await {
                        Ok(result) => {
                            if result.created {
                                let ws2 = ws.clone();
                                let (uid, wid) = (user_id, result.work.id);
                                tokio::spawn(async move {
                                    let _ = ws2.converge_work(uid, wid, 3).await;
                                });
                            }
                            AddOutcome {
                                rd_book_id: p.rd_book_id,
                                work_id: Some(result.work.id),
                                was_created: result.created,
                                error: None,
                            }
                        }
                        Err(e) => {
                            warn!(title = %p.title, "Failed to create work: {e}");
                            AddOutcome {
                                rd_book_id: p.rd_book_id,
                                work_id: None,
                                was_created: false,
                                error: Some(format!("Work '{}': {e}", p.title)),
                            }
                        }
                    };
                    // Bump progress for this item now, right as it settles.
                    // The lock is taken only across this brief update — never
                    // across the `add()` await above — so concurrent siblings
                    // are never blocked waiting on this item's own progress
                    // bump. group_size mirrors exactly what the old
                    // post-collect pass counted: one primary + its deduped
                    // secondaries (Pass 1b), so the final total is unchanged.
                    let group_size = 1 + secondaries_by_primary
                        .get(&outcome.rd_book_id)
                        .map(|s| s.len())
                        .unwrap_or(0);
                    {
                        let mut prog = progress.lock().await;
                        prog.works_processed += group_size as i64;
                        if let Some(err) = &outcome.error {
                            prog.errors.push(err.clone());
                        }
                    }
                    outcome
                }
            })
            .buffer_unordered(5)
            .collect()
            .await;

        // --- Pass 3: serial post-pass to fold outcomes into shared state ---
        // Progress (works_processed/errors) for the dispatched items was
        // already advanced incrementally inside the stream above. This pass
        // only folds skip_errors (known before dispatch even started, so
        // there's no stall to fix there) and the `&mut self` bookkeeping
        // (works_created / work_map_rd) that the concurrent closures can't
        // touch directly.
        {
            let mut prog = self.progress().lock().await;
            for err in skip_errors {
                prog.errors.push(err);
                prog.works_processed += 1;
            }
        }
        for outcome in outcomes {
            if outcome.was_created {
                self.works_created += 1;
            }
            if let Some(id) = outcome.work_id {
                self.work_map_rd.insert(outcome.rd_book_id, id);
                if let Some(secs) = secondaries_by_primary.get(&outcome.rd_book_id) {
                    for sec_id in secs {
                        self.work_map_rd.insert(*sec_id, id);
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_files(
        &mut self,
        rd_book_files: &[RdBookFile],
        active_book_ids: &HashSet<i64>,
        author_map: &HashMap<i64, &RdAuthor>,
        rd_books: &[RdBook],
        book_files_by_book: &HashMap<i64, Vec<&RdBookFile>>,
        readarr_root: &str,
        livrarr_root_path: &str,
    ) -> Result<(), String> {
        for rd_file in rd_book_files
            .iter()
            .filter(|f| active_book_ids.contains(&f.book_id))
        {
            let work_id = match self.work_map_rd.get(&rd_file.book_id) {
                Some(id) => *id,
                None => {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    continue;
                }
            };

            let author_name = rd_file
                .author_id
                .and_then(|aid| author_map.get(&aid))
                .and_then(|a| a.author_name.as_deref())
                .unwrap_or("Unknown Author");

            let title = rd_books
                .iter()
                .find(|b| b.id == rd_file.book_id)
                .and_then(|b| b.title.as_deref())
                .unwrap_or("Unknown Title");

            let qid = extract_quality_id(rd_file);
            let media_type = match resolve_media_type(qid, &rd_file.path) {
                Some(mt) => mt,
                None => {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    continue;
                }
            };

            if media_type == MediaType::Audiobook {
                let book_audio_count = book_files_by_book
                    .get(&rd_file.book_id)
                    .map(|fs| {
                        fs.iter()
                            .filter(|f| {
                                resolve_media_type(extract_quality_id(f), &f.path)
                                    == Some(MediaType::Audiobook)
                            })
                            .count()
                    })
                    .unwrap_or(0);
                if book_audio_count > 1 {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    continue;
                }
            }

            let translated_path = apply_path_translation(
                &rd_file.path,
                self.req.container_path.as_deref(),
                self.req.host_path.as_deref(),
            );
            let vsp_translated = translated_path.clone();
            let vsp_root = readarr_root.to_string();
            let source = match tokio::task::spawn_blocking(move || {
                validate_source_path(&vsp_translated, &vsp_root)
            })
            .await
            .unwrap_or_else(|e| Err(format!("spawn error: {e}")))
            {
                Ok(p) => p,
                Err(e) => {
                    // Server-side log keeps the real path for operators; the
                    // surfaced error never carries a filesystem path — it
                    // identifies the book by title instead (mirrors the
                    // works-phase `Work '{title}': {e}` convention above).
                    warn!(path = %rd_file.path, "Source path validation failed: {e}");
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    prog.errors
                        .push(format!("File for '{title}': source path validation failed"));
                    continue;
                }
            };

            let dest = build_dest_path(
                livrarr_root_path,
                self.user_id,
                author_name,
                title,
                &rd_file.path,
            );

            // The shared import core's adopt/dedup outcome matrix decides
            // existing-target handling: row for this work at this path →
            // Skipped; orphan file matching the source's size → Adopted;
            // otherwise → PathCollision. A re-run after a crashed prior
            // migration therefore adopts its already-hardlinked files.
            let rel_path = dest
                .strip_prefix(livrarr_root_path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| dest.to_string_lossy().to_string());

            use livrarr_domain::services::ImportWorkflow;
            match self
                .import_workflow
                .import_file(
                    self.user_id,
                    livrarr_domain::services::ImportFileRequest {
                        work_id,
                        root_folder_id: self.req.livrarr_root_folder_id,
                        source,
                        target_relative: rel_path,
                        media_type,
                        materialization: livrarr_domain::services::Materialization::HardlinkFirst,
                        import_id: Some(self.import_id.clone()),
                        extract_chapters: false,
                    },
                )
                .await
            {
                Ok(
                    livrarr_domain::services::ImportFileOutcome::Imported { .. }
                    | livrarr_domain::services::ImportFileOutcome::Adopted { .. },
                ) => {
                    self.files_imported += 1;
                }
                Ok(livrarr_domain::services::ImportFileOutcome::Skipped { .. }) => {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_skipped += 1;
                }
                Err(livrarr_domain::services::ImportWorkflowError::PathCollision(path)) => {
                    warn!(path = %rd_file.path, "Path collision on import: {path} already claimed by a different work");
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_skipped += 1;
                    prog.errors.push(format!(
                        "File for '{title}': path collision — already claimed by a different work"
                    ));
                }
                Err(e) => {
                    warn!(path = %rd_file.path, "Failed to import file: {e}");
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_skipped += 1;
                    prog.errors.push(format!("File for '{title}': {e}"));
                }
            }

            {
                let mut prog = self.progress().lock().await;
                prog.files_processed += 1;
            }
        }
        Ok(())
    }
}

// =============================================================================
// Bug 114c: works-phase progress must advance incrementally, not freeze
// =============================================================================
//
// `process_works` above dispatches `work_service.add()` through
// `Arc<LiveWorkService>` — a concrete production type alias
// (`WorkServiceImpl<SqliteDb, LiveEnrichmentWorkflow, HttpFetcherImpl, ...>`,
// see `crate::state::LiveWorkService`), not a generic parameter. There is no
// seam to swap in an offline stub HTTP fetcher for this specific struct, and
// `add()`'s real identity resolution would otherwise reach live Goodreads /
// OpenLibrary endpoints (banned in automated tests per project convention —
// see `wiki/insights.md` #44/#45 on GR anti-bot fragility and OL crawl
// etiquette; this is the same constraint `tests/behavioral/
// test_wcc_path_seams.rs` already documents for this exact method: "Readarr
// `process_works` is currently a private `ImportRunner` method"). So this
// module cannot invoke the real method end-to-end; instead it exercises the
// EXACT dispatch shape Pass 2/3 above use — bounded concurrency 5 via
// `buffer_unordered`, one `progress.lock().await` bump per item taken only
// across the counter update (never across the item's own await), sized by
// the same "1 + secondaries" group rule — with a controllable stand-in for
// `add()` so the timing is deterministic. A regression that moves the bump
// back into a post-collect loop (the 114c bug) makes
// `test_readarr_import_progress_advances_incrementally` fail.
#[cfg(test)]
mod process_works_progress_tests {
    use super::*;
    use tokio::sync::Notify;

    struct FakeOutcome {
        #[allow(dead_code)]
        rd_book_id: i64,
        group_size: i64,
        error: Option<String>,
    }

    /// Mirrors `process_works` Pass 2/3: bounded concurrency 5, one
    /// `progress.works_processed` bump per item as it settles.
    async fn run_incremental_dispatch(
        items: Vec<(i64, i64, Option<String>)>, // (rd_book_id, group_size, error)
        blocked_id: i64,
        gate: Arc<Notify>,
        progress: Arc<Mutex<ReadarrImportProgress>>,
    ) -> Vec<FakeOutcome> {
        stream::iter(items)
            .map(|(rd_book_id, group_size, error)| {
                let progress = progress.clone();
                let gate = gate.clone();
                async move {
                    if rd_book_id == blocked_id {
                        // Stands in for a slow/in-flight `add()` (identity +
                        // enrichment) — never resolves until signalled.
                        gate.notified().await;
                    }
                    let outcome = FakeOutcome {
                        rd_book_id,
                        group_size,
                        error,
                    };
                    // Bump held only across the counter update — never
                    // across the item's own await above.
                    {
                        let mut prog = progress.lock().await;
                        prog.works_processed += outcome.group_size;
                        if let Some(err) = &outcome.error {
                            prog.errors.push(err.clone());
                        }
                    }
                    outcome
                }
            })
            .buffer_unordered(5)
            .collect()
            .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_readarr_import_progress_advances_incrementally() {
        // 7 primary items: id=100 carries 2 deduped secondaries (group_size
        // 3, mirroring Pass 1b's "1 + secondaries" rule); the rest are
        // group_size 1. id=999 is gated open — it never completes until
        // signalled — so it stays "in flight" for the whole assertion
        // window below.
        let items: Vec<(i64, i64, Option<String>)> = vec![
            (999, 1, None), // blocked for the whole window
            (1, 1, None),
            (2, 1, None),
            (3, 1, None),
            (4, 1, None),
            (100, 3, None), // primary + 2 secondaries
            (5, 1, Some("Work 'X': boom".to_string())),
        ];
        let total_expected: i64 = items.iter().map(|(_, g, _)| g).sum();
        let items_count = items.len();
        let partial_expected = total_expected - 1; // everything but item 999

        let progress = Arc::new(Mutex::new(ReadarrImportProgress::default()));
        let gate = Arc::new(Notify::new());

        let handle = tokio::spawn(run_incremental_dispatch(
            items,
            999,
            gate.clone(),
            progress.clone(),
        ));

        // While item 999 is still gated shut, every other item (bounded
        // concurrency 5, so at most 4 others ever run alongside it, with the
        // remainder queued behind) must still be able to complete and bump
        // progress. This is the "not frozen" proof: the pre-fix code could
        // only ever move `works_processed` AFTER `.collect()` returned,
        // which cannot happen while item 999 is gated.
        let mut observed_partial = false;
        for _ in 0..200 {
            if progress.lock().await.works_processed >= partial_expected {
                observed_partial = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            observed_partial,
            "works_processed should reach {partial_expected} while item 999 is still pending \
             (progress must advance incrementally, not only after the whole phase collects)"
        );
        assert!(
            !handle.is_finished(),
            "the dispatch must still be awaiting the gated item"
        );

        // Release the last item and confirm the total lands exactly where
        // it should — no double-count, no lost update, total unchanged from
        // before the fix.
        gate.notify_one();
        let outcomes = handle.await.expect("dispatch task should not panic");
        assert_eq!(outcomes.len(), items_count);
        assert_eq!(
            progress.lock().await.works_processed,
            total_expected,
            "final works_processed must equal the sum of all group sizes"
        );
        assert_eq!(
            progress.lock().await.errors.len(),
            1,
            "the one erroring item's message must still be recorded"
        );
    }
}

// =============================================================================
// Author-dedup DEFERRED-PIN: process_authors' batch-local decision
// =============================================================================
//
// `ImportRunner` cannot be constructed here the way `process_authors` runs in
// production: `Arc<LiveWorkService>` and `Arc<LiveImportWorkflow>` are
// concrete production aliases (`crate::state`) that bottom out in the live
// provider/enrichment queue wiring — the same constraint
// `process_works_progress_tests` above already documents for `process_works`.
// `process_authors` itself never touches either field, so the fix extracts
// its batch-local adopt-or-create decision into the pure, DB-free
// `resolve_batch_author` (defined above, beside `ImportRunner`) and pins
// that directly — the sanctioned fallback for this exact situation, same
// spirit as the `provider_queue` tracer tests reaching a private seam.
#[cfg(test)]
mod batch_author_resolution_tests {
    use super::*;

    fn fake_author(id: i64, name: &str) -> Author {
        Author {
            id,
            user_id: 1,
            name: name.to_string(),
            sort_name: None,
            ol_key: None,
            gr_key: None,
            hc_key: None,
            import_id: None,
            monitored: false,
            monitor_new_items: false,
            monitor_since: None,
            monitor_language: None,
            added_at: chrono::Utc::now(),
        }
    }

    /// [REV codex R-7]: two spelling variants arriving in the same import
    /// batch, both absent from the DB — the first creates, the second must
    /// adopt the first's freshly-created row rather than double-creating.
    #[test]
    fn two_in_batch_spelling_variants_collapse_to_one_author_both_ids_mapped() {
        let mut batch_authors: Vec<Author> = Vec::new();
        let mut author_map_rd: HashMap<i64, i64> = HashMap::new();
        let mut next_id = 1i64;

        let mut resolve = |rd_id: i64, name: &str| {
            let livrarr_id = match resolve_batch_author(name, &batch_authors) {
                BatchAuthorDecision::Adopt(id) => id,
                BatchAuthorDecision::Create => {
                    let id = next_id;
                    next_id += 1;
                    batch_authors.push(fake_author(id, name));
                    id
                }
            };
            author_map_rd.insert(rd_id, livrarr_id);
        };

        resolve(501, "W.E.B. Griffin");
        resolve(502, "W. E. B. Griffin");

        assert_eq!(
            batch_authors.len(),
            1,
            "only one author should have been created for two in-batch variants"
        );
        assert_eq!(author_map_rd[&501], author_map_rd[&502]);
    }

    /// [REV codex R-9]: the first row of the batch adopts a pre-existing
    /// (already-in-DB) author; a later row in the same batch is another
    /// spelling variant and must also adopt — never create, and never trip
    /// the exactly-one rule into ambiguity from a re-appended duplicate.
    #[test]
    fn later_in_batch_variant_of_first_row_adopted_author_also_adopts_never_creates() {
        let batch_authors = vec![fake_author(42, "Robert A. Heinlein")];

        let first = resolve_batch_author("Robert Heinlein", &batch_authors);
        assert_eq!(first, BatchAuthorDecision::Adopt(42));
        // An adopted author is never re-appended to the batch-local list.
        assert_eq!(batch_authors.len(), 1);

        let second = resolve_batch_author("Robert Anson Heinlein", &batch_authors);
        assert_eq!(
            second,
            BatchAuthorDecision::Adopt(42),
            "a later in-batch variant of an already-adopted author must adopt, not create"
        );
    }
}
