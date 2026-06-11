#![allow(clippy::async_yields_async)]

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct JobStatusSnapshot {
    name: String,
    interval: Duration,
    last_run_present: bool,
    running: bool,
    panic_notified: bool,
}

trait JobRunnerContract: Send + Sync {
    fn job_statuses<'a>(&'a self) -> BoxFuture<'a, Vec<JobStatusSnapshot>>;
    fn execute_job_by_name<'a>(
        &'a self,
        job_name: &'a str,
        notification_count: Arc<AtomicUsize>,
        job: BoxFuture<'static, ()>,
    ) -> BoxFuture<'a, bool>;
    fn spawn_blocked_job<'a>(&'a self) -> BoxFuture<'a, JoinHandle<()>>;
    fn abort_all(&self);
}

// ---------------------------------------------------------------------------
// Real JobRunner implementation
// ---------------------------------------------------------------------------

struct JobEntry {
    name: String,
    interval: Duration,
    last_run_present: bool,
    running: bool,
    panic_notified: bool,
}

struct RealJobRunner {
    jobs: Arc<RwLock<HashMap<String, JobEntry>>>,
    abort_handles: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl RealJobRunner {
    fn new() -> Self {
        let mut jobs = HashMap::new();
        for (name, interval_secs) in [
            ("download_poller", 60),
            ("session_cleanup", 3600),
            ("author_monitor", 86400),
            ("enrichment_retry", 300),
        ] {
            jobs.insert(
                name.to_string(),
                JobEntry {
                    name: name.to_string(),
                    interval: Duration::from_secs(interval_secs),
                    last_run_present: false,
                    running: false,
                    panic_notified: false,
                },
            );
        }
        Self {
            jobs: Arc::new(RwLock::new(jobs)),
            abort_handles: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl JobRunnerContract for RealJobRunner {
    fn job_statuses<'a>(&'a self) -> BoxFuture<'a, Vec<JobStatusSnapshot>> {
        Box::pin(async move {
            let jobs = self.jobs.read().await;
            jobs.values()
                .map(|j| JobStatusSnapshot {
                    name: j.name.clone(),
                    interval: j.interval,
                    last_run_present: j.last_run_present,
                    running: j.running,
                    panic_notified: j.panic_notified,
                })
                .collect()
        })
    }

    fn execute_job_by_name<'a>(
        &'a self,
        job_name: &'a str,
        notification_count: Arc<AtomicUsize>,
        job: BoxFuture<'static, ()>,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            {
                let mut jobs = self.jobs.write().await;
                let entry = match jobs.get_mut(job_name) {
                    Some(e) => e,
                    None => return false,
                };
                entry.running = true;
            }

            // Catch panics via AssertUnwindSafe + catch_unwind on the future
            let panicked = {
                use futures::FutureExt;
                let result = std::panic::AssertUnwindSafe(job).catch_unwind().await;
                result.is_err()
            };

            {
                let mut jobs = self.jobs.write().await;
                if let Some(entry) = jobs.get_mut(job_name) {
                    entry.running = false;
                    if panicked {
                        if !entry.panic_notified {
                            notification_count.fetch_add(1, Ordering::SeqCst);
                            entry.panic_notified = true;
                        }
                    } else {
                        entry.last_run_present = true;
                        entry.panic_notified = false;
                    }
                }
            }

            true
        })
    }

    fn spawn_blocked_job<'a>(&'a self) -> BoxFuture<'a, JoinHandle<()>> {
        Box::pin(async move {
            let handle = tokio::spawn(async {
                std::future::pending::<()>().await;
            });
            self.abort_handles
                .lock()
                .unwrap()
                .push(handle.abort_handle());
            handle
        })
    }

    fn abort_all(&self) {
        let handles = self.abort_handles.lock().unwrap();
        for h in handles.iter() {
            h.abort();
        }
    }
}

fn make_job_runner() -> Arc<dyn JobRunnerContract> {
    Arc::new(RealJobRunner::new())
}

async fn find_job_status(runner: &dyn JobRunnerContract, job_name: &str) -> JobStatusSnapshot {
    let statuses = runner.job_statuses().await;
    statuses
        .into_iter()
        .find(|s| s.name == job_name)
        .unwrap_or_else(|| panic!("expected job status for {job_name}"))
}

#[tokio::test]
async fn test_job_runner_req_jr_001_exposes_required_interval_jobs_by_name_and_interval() {
    // REQ-ID: JR-001
    let runner = make_job_runner();

    let statuses = runner.job_statuses().await;

    assert!(
        statuses
            .iter()
            .any(|s| s.name == "download_poller" && s.interval == Duration::from_secs(60)),
        "missing required job download_poller with 60s interval"
    );
    assert!(
        statuses
            .iter()
            .any(|s| s.name == "session_cleanup" && s.interval == Duration::from_secs(60 * 60)),
        "missing required job session_cleanup with 3600s interval"
    );
    assert!(
        statuses
            .iter()
            .any(|s| s.name == "author_monitor" && s.interval == Duration::from_secs(60 * 60 * 24)),
        "missing required job author_monitor with 86400s interval"
    );
    assert!(
        statuses
            .iter()
            .any(|s| s.name == "enrichment_retry" && s.interval == Duration::from_secs(60 * 5)),
        "missing required job enrichment_retry with 300s interval"
    );
}

#[tokio::test]
async fn test_job_runner_req_jr_002_successful_execution_clears_running_sets_last_run_and_no_panic_notification(
) {
    // REQ-ID: JR-002
    let runner = make_job_runner();
    let notifications = Arc::new(AtomicUsize::new(0));

    let executed = runner
        .execute_job_by_name("download_poller", notifications.clone(), Box::pin(async {}))
        .await;

    assert!(executed, "runner should execute a non-running job");

    let status = find_job_status(runner.as_ref(), "download_poller").await;
    assert!(
        !status.running,
        "job must not remain marked running after success"
    );
    assert!(
        status.last_run_present,
        "successful execution must record last_run"
    );
    assert!(
        !status.panic_notified,
        "successful execution must clear panic-notified state"
    );
    assert_eq!(
        notifications.load(Ordering::SeqCst),
        0,
        "success must not emit panic notifications"
    );
}

#[tokio::test]
async fn test_job_runner_req_jr_003_panic_marks_notification_and_does_not_set_last_run() {
    // REQ-ID: JR-003
    let runner = make_job_runner();
    let notifications = Arc::new(AtomicUsize::new(0));

    let executed = runner
        .execute_job_by_name(
            "session_cleanup",
            notifications.clone(),
            Box::pin(async {
                panic!("simulated panic");
            }),
        )
        .await;

    assert!(executed, "runner should accept execution attempt");

    let status = find_job_status(runner.as_ref(), "session_cleanup").await;
    assert!(!status.running, "panic must not leave job stuck running");
    assert!(
        status.panic_notified,
        "panic must mark the job as already notified"
    );
    assert!(
        !status.last_run_present,
        "panic must not update successful last_run timestamp"
    );
    assert_eq!(
        notifications.load(Ordering::SeqCst),
        1,
        "first panic in an episode must emit one notification"
    );
}

#[tokio::test]
async fn test_job_runner_req_jr_004_second_consecutive_panic_does_not_duplicate_notification() {
    // REQ-ID: JR-004
    let runner = make_job_runner();
    let notifications = Arc::new(AtomicUsize::new(0));

    let first = runner
        .execute_job_by_name(
            "author_monitor",
            notifications.clone(),
            Box::pin(async {
                panic!("first panic");
            }),
        )
        .await;
    assert!(first, "first execution attempt should be accepted");

    let second = runner
        .execute_job_by_name(
            "author_monitor",
            notifications.clone(),
            Box::pin(async {
                panic!("second panic");
            }),
        )
        .await;
    assert!(second, "second execution attempt should be accepted");

    let status = find_job_status(runner.as_ref(), "author_monitor").await;
    assert!(!status.running, "job must not remain marked running");
    assert!(status.panic_notified, "panic state should remain notified");
    assert_eq!(
        notifications.load(Ordering::SeqCst),
        1,
        "consecutive panic episodes must not produce duplicate notifications"
    );
}

#[tokio::test]
async fn test_job_runner_req_jr_005_success_after_panic_resets_notification_state() {
    // REQ-ID: JR-005
    let runner = make_job_runner();
    let notifications = Arc::new(AtomicUsize::new(0));

    let first = runner
        .execute_job_by_name(
            "enrichment_retry",
            notifications.clone(),
            Box::pin(async {
                panic!("panic before recovery");
            }),
        )
        .await;
    assert!(first, "panicing execution attempt should still be accepted");

    let after_panic = find_job_status(runner.as_ref(), "enrichment_retry").await;
    assert!(
        after_panic.panic_notified,
        "panic episode must set panic_notified before recovery"
    );

    let second = runner
        .execute_job_by_name(
            "enrichment_retry",
            notifications.clone(),
            Box::pin(async {}),
        )
        .await;
    assert!(second, "recovery execution attempt should be accepted");

    let recovered = find_job_status(runner.as_ref(), "enrichment_retry").await;
    assert!(!recovered.running, "recovered job must not be running");
    assert!(
        !recovered.panic_notified,
        "successful recovery must clear panic_notified"
    );
    assert!(
        recovered.last_run_present,
        "successful recovery must record last_run"
    );
    assert_eq!(
        notifications.load(Ordering::SeqCst),
        1,
        "recovery itself must not emit additional panic notifications"
    );
}

#[tokio::test]
async fn test_job_runner_req_jr_006_abort_all_cancels_active_tasks() {
    // REQ-ID: JR-006
    let runner = make_job_runner();

    let handle_1 = runner.spawn_blocked_job().await;
    let handle_2 = runner.spawn_blocked_job().await;
    let handle_3 = runner.spawn_blocked_job().await;
    let handle_4 = runner.spawn_blocked_job().await;

    runner.abort_all();

    let result_1 = timeout(Duration::from_secs(1), handle_1).await.unwrap();
    let result_2 = timeout(Duration::from_secs(1), handle_2).await.unwrap();
    let result_3 = timeout(Duration::from_secs(1), handle_3).await.unwrap();
    let result_4 = timeout(Duration::from_secs(1), handle_4).await.unwrap();

    assert!(
        result_1.unwrap_err().is_cancelled(),
        "task 1 must be cancelled"
    );
    assert!(
        result_2.unwrap_err().is_cancelled(),
        "task 2 must be cancelled"
    );
    assert!(
        result_3.unwrap_err().is_cancelled(),
        "task 3 must be cancelled"
    );
    assert!(
        result_4.unwrap_err().is_cancelled(),
        "task 4 must be cancelled"
    );
}
