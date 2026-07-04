//! Per-import-run negative cache for phase1 cover-fetch hosts.
//!
//! A host that fails the phase1 direct-download connect attempt once is
//! skipped for the rest of the SAME import run — later books whose embedded
//! cover URL shares that host never re-attempt the doomed connect. The cache
//! is a `tokio::task_local`, not a global/static: it is installed once per
//! run via [`with_cover_host_cache`] (wrapping the manual-import batch loop)
//! and lives only as long as that async task runs. It is dropped when the
//! scope exits, so a host dead in one run is retried fresh in the next, and
//! two runs (different requests, different tasks, possibly different users)
//! never share state.
//!
//! Every call site that does NOT wrap itself in [`with_cover_host_cache`]
//! (direct add, list import, Readarr import, author monitor, background
//! retry — every `WorkService::add` caller other than manual import) reads a
//! task-local that was never installed. [`is_known_dead_host`] and
//! [`mark_cover_host_dead`] use `try_with`, so that's a silent, safe no-op —
//! those paths behave exactly as before this cache existed.

use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;

tokio::task_local! {
    static DEAD_COVER_HOSTS: RefCell<HashSet<String>>;
}

/// Run `body` with a fresh, empty dead-host set installed for the current
/// task. Call once per import batch, wrapping the whole loop over items —
/// every nested `.await` down to `fetch_phase1_cover` sees the same set as
/// long as nothing in between spawns a separate task (manual import's loop
/// does not).
pub async fn with_cover_host_cache<F: Future>(body: F) -> F::Output {
    DEAD_COVER_HOSTS
        .scope(RefCell::new(HashSet::new()), body)
        .await
}

/// True if `host` already failed a phase1 connect attempt earlier in the
/// current import run. Always `false` outside a [`with_cover_host_cache`]
/// scope.
pub fn is_known_dead_host(host: &str) -> bool {
    DEAD_COVER_HOSTS
        .try_with(|hosts| hosts.borrow().contains(host))
        .unwrap_or(false)
}

/// Record that `host` failed a phase1 connect attempt in the current import
/// run. No-op outside a [`with_cover_host_cache`] scope.
pub fn mark_cover_host_dead(host: &str) {
    let _ = DEAD_COVER_HOSTS.try_with(|hosts| {
        hosts.borrow_mut().insert(host.to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unset_outside_scope_is_always_not_dead_and_mark_is_a_noop() {
        assert!(!is_known_dead_host("example.com"));
        mark_cover_host_dead("example.com"); // must not panic
        assert!(!is_known_dead_host("example.com"));
    }

    #[tokio::test]
    async fn mark_then_check_within_the_same_scope_reports_dead() {
        with_cover_host_cache(async {
            assert!(!is_known_dead_host("dead.example.com"));
            mark_cover_host_dead("dead.example.com");
            assert!(is_known_dead_host("dead.example.com"));
            assert!(!is_known_dead_host("other.example.com"));
        })
        .await;
    }

    #[tokio::test]
    async fn separate_scopes_do_not_leak_into_each_other() {
        with_cover_host_cache(async {
            mark_cover_host_dead("dead.example.com");
            assert!(is_known_dead_host("dead.example.com"));
        })
        .await;

        // A fresh scope — the mark from the previous scope must not survive.
        with_cover_host_cache(async {
            assert!(!is_known_dead_host("dead.example.com"));
        })
        .await;
    }
}
