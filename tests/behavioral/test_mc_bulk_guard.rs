//! Behavioral pins for metadata-correctness bulk refresh guard semantics.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_domain::services::{BulkRefreshGuard, WorkService};
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService = WorkServiceImpl<
    SqliteDb,
    livrarr_metadata::work_service::StubNoEnrichment,
    StubHttpFetcher,
    livrarr_metadata::work_service::StubNoLlm,
>;

fn service(db: SqliteDb) -> TestWorkService {
    WorkServiceImpl::without_enrichment(
        db,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

#[tokio::test]
async fn try_start_rejects_duplicate_until_guard_drops() {
    // REQ-016/AC-018: a genuine concurrent duplicate returns None, then the slot reopens on Drop.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let svc = service(db);

    let first = svc.try_start_bulk_refresh(42);
    assert_matches!(first, Some(_));
    assert_matches!(svc.try_start_bulk_refresh(42), None);

    drop(first);
    assert_matches!(svc.try_start_bulk_refresh(42), Some(_));
}

#[tokio::test]
async fn guard_moved_into_panicking_task_releases_slot() {
    // REQ-016/AC-018: panic unwind drops the guard and releases the bulk slot.
    let slots = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    slots.lock().expect("slot lock").insert(7);
    let guard = BulkRefreshGuard::new(slots.clone(), 7);

    let handle = tokio::spawn(async move {
        let _guard = guard;
        panic!("simulated bulk refresh panic");
    });
    let join = handle.await;
    assert!(join.is_err(), "the task should have panicked");

    let reopened = {
        let mut locked = slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locked.insert(7)
    };
    assert!(reopened, "panic should not leave the slot occupied");
}

#[tokio::test]
async fn guard_moved_into_aborted_task_releases_slot() {
    // REQ-016/AC-018: aborting a task that owns the guard still drops it.
    let slots = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    slots.lock().expect("slot lock").insert(8);
    let guard = BulkRefreshGuard::new(slots.clone(), 8);

    let handle = tokio::spawn(async move {
        let _guard = guard;
        futures::future::pending::<()>().await;
    });
    handle.abort();
    let _ = handle.await;

    let reopened = {
        let mut locked = slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locked.insert(8)
    };
    assert!(reopened, "abort should not leave the slot occupied");
}

#[tokio::test]
async fn poisoned_mutex_peer_does_not_wedge_release() {
    // REQ-016/AC-018: BulkRefreshGuard::drop takes poisoned locks through into_inner.
    let slots = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    slots.lock().expect("slot lock").insert(9);

    let poison_slots = slots.clone();
    let poisoner = std::thread::spawn(move || {
        let _locked = poison_slots.lock().expect("poison lock");
        panic!("poison the mutex while holding it");
    });
    let _ = poisoner.join();

    let guard = BulkRefreshGuard::new(slots.clone(), 9);
    drop(guard);

    let reopened = {
        let mut locked = slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locked.insert(9)
    };
    assert!(reopened, "poisoned mutex should not wedge guard release");
}
