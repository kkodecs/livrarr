//! Process-global outbound rate-limit queue.
//!
//! One wait-then-send line per [`RateBucket`]: every outbound provider call takes a
//! number and waits its turn, paced per bucket, with a bounded number of in-flight
//! sends. Nothing is dropped — the wait is unbounded. This is the process-wide floor
//! that keeps outbound traffic under each book site's rate limit so the server does
//! not get banned.
//!
//! Per-bucket intervals carry over from the existing fetcher limiter (OpenLibrary /
//! Goodreads / Hardcover / GoogleBooks 1s, Audnexus 2s, Audible 150ms, Indexer 500ms,
//! None 0).
//!
//! Design of record: `docs/metadata-remediation-phase3-queue-design.md` (LOCKED v4).
//! The queue STATE is process-global — shared by every `HttpFetcherImpl` via
//! [`shared`], so a second fetcher reaches the SAME per-bucket queues (the M-009
//! fix). The per-bucket dispatcher TASK is spawned ON DEMAND and self-exits when its
//! queue drains empty; it is never a static long-lived task (that hangs `cargo test`'s
//! multi-runtime model — a task spawned on one test's runtime dies with it).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use livrarr_domain::services::RateBucket;
use livrarr_domain::RequestPriority;
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use crate::breaker::{self, BreakerSignal, BreakerState, CircuitState};

/// Maximum concurrent in-flight sends per bucket.
///
/// Separate control from pacing: pacing spaces the *start* of each send; this caps
/// how many may be *in flight* at once when a response outlives the interval. This
/// is the sole authority on in-flight concurrency per bucket, for every caller.
pub const OUTBOUND_IN_FLIGHT_CAP: usize = 2;

/// RAII "your turn to send" signal handed to a caller when the dispatcher releases it.
///
/// Holds the in-flight permit; dropping it — on send completion OR on caller
/// cancellation — frees the in-flight slot. Opaque: hold it across the HTTP send and
/// body read, then let it drop. There is nothing to call on it. A bypass call
/// (`RateBucket::None`) holds no permit (`None`).
#[derive(Debug)]
pub struct QueuePermit {
    _permit: Option<OwnedSemaphorePermit>,
}

/// What the dispatcher hands a queued caller: a granted turn (the in-flight
/// permit), or a breaker-open rejection carrying the time remaining until the
/// open window elapses (R-3: no HTTP happens on this path).
type TurnResult = Result<OwnedSemaphorePermit, Duration>;

/// Process-monotonic sequence number. Assigned once per `acquire` call so same-
/// priority items dispatch in arrival order.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// One caller's place in a bucket's queue.
///
/// `Ord`/`PartialOrd`/`Eq`/`PartialEq` are hand-implemented over `(priority, seq)` —
/// `turn` is a `oneshot::Sender`, which implements none of those traits, so they
/// cannot be derived. The ordering makes a max-heap (`BinaryHeap`) pop the highest
/// priority first, and within equal priority, the lowest sequence number (earliest
/// arrival) first.
struct QueuedItem {
    priority: RequestPriority,
    seq: u64,
    turn: oneshot::Sender<TurnResult>,
}

impl PartialEq for QueuedItem {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}

impl Eq for QueuedItem {}

impl PartialOrd for QueuedItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// A bucket's queue state: pending callers plus the pacing clock. Guarded by a
/// `std::sync::Mutex` — never held across an `.await`.
struct BucketState {
    heap: BinaryHeap<QueuedItem>,
    dispatcher_running: bool,
    last_dispatch: Instant,
}

/// Shared handle to one bucket's queue state, its in-flight permit pool, and
/// (for breaker-tracked buckets) its circuit breaker. Cheap to clone — every
/// field is an `Arc` (or, for `breaker`, an `Option<Arc<_>>`).
#[derive(Clone)]
struct BucketHandle {
    state: Arc<Mutex<BucketState>>,
    semaphore: Arc<Semaphore>,
    /// `None` for `RateBucket::None` (R-5) and any future pace-only
    /// aggregate bucket. `Some` for every breaker-tracked bucket — the six
    /// book-provider APIs plus `Indexer(_)`, which is single-host (origin-
    /// keyed) and so carries a breaker like the rest.
    breaker: Option<Arc<Mutex<BreakerState>>>,
}

impl BucketHandle {
    fn new(bucket: &RateBucket, interval: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(BucketState {
                heap: BinaryHeap::new(),
                dispatcher_running: false,
                // Backdated so the bucket's first-ever dispatch does not wait a full
                // interval.
                last_dispatch: Instant::now() - interval,
            })),
            semaphore: Arc::new(Semaphore::new(OUTBOUND_IN_FLIGHT_CAP)),
            breaker: breaker::breaker_tracked(bucket)
                .then(|| Arc::new(Mutex::new(BreakerState::new(breaker::config_for(bucket))))),
        }
    }
}

/// Resets `dispatcher_running` to `false` if the dispatcher loop exits ABNORMALLY — a
/// panic, or the task being dropped on runtime teardown (a dispatcher spawned on one
/// test's runtime dies with it). The clean drain-empty exit clears the flag itself,
/// atomically under the empty-check lock, and disarms this guard. Without the guard a
/// dead dispatcher would leave the flag stuck `true` and every later call for that
/// bucket would wait forever.
struct DispatcherGuard {
    state: Arc<Mutex<BucketState>>,
    armed: bool,
}

impl DispatcherGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DispatcherGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Poison-tolerant: even if a panic poisoned the lock, still clear the flag so
        // the bucket can respawn a dispatcher rather than wedge forever.
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.dispatcher_running = false;
    }
}

/// Minimum interval between dispatches for a bucket — the single source of
/// per-provider pacing now that `do_fetch` routes through this queue.
fn interval_for(bucket: &RateBucket) -> Duration {
    match bucket {
        RateBucket::OpenLibrary | RateBucket::Hardcover | RateBucket::GoogleBooks => {
            Duration::from_secs(1)
        }
        // Goodreads is an anti-bot-hostile scrape target — paced slower than
        // the API-backed providers.
        RateBucket::Goodreads => Duration::from_millis(1500),
        RateBucket::Audnexus => Duration::from_secs(2),
        RateBucket::Audible => Duration::from_millis(150),
        // OL's ISBN-cover endpoint limit is ~100 requests/IP/5min ≈ 1 per 3s.
        // Pace-only (R-6) — never added to `breaker::breaker_tracked`.
        RateBucket::OpenLibraryCovers => Duration::from_secs(3),
        RateBucket::Indexer(_) => Duration::from_millis(500),
        RateBucket::None => Duration::ZERO,
    }
}

/// One bucket's dispatcher loop: pace sends by `interval`, cap in-flight sends via
/// the bucket's semaphore, and grant queued callers their turn in `(priority DESC,
/// seq ASC)` order. Exits — self-cleaning via [`DispatcherGuard`] — once the queue
/// drains empty; the next `acquire` call on this bucket respawns it.
async fn run_dispatcher(handle: BucketHandle, interval: Duration) {
    let mut guard = DispatcherGuard {
        state: Arc::clone(&handle.state),
        armed: true,
    };

    loop {
        let next_allowed = {
            let mut state = handle
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.heap.is_empty() {
                // Reset the flag ATOMICALLY with the empty check (same lock hold) so a
                // concurrent `acquire` cannot observe a stale `true`, skip spawning, and
                // orphan its item while this dispatcher exits. The guard is disarmed
                // because this clean path already cleared the flag.
                state.dispatcher_running = false;
                guard.disarm();
                return;
            }
            state.last_dispatch + interval
        };

        // Breaker gate (R-3), checked BEFORE pacing/semaphore: an Open breaker
        // rejects the top queued item immediately — no HTTP, no pacing slot
        // consumed, no in-flight permit touched. `handle.breaker` is `None`
        // for exempt buckets (`None`/`Indexer(_)`), which always fall through
        // to a normal grant below.
        if let Some(breaker) = &handle.breaker {
            let retry_after = {
                let mut b = breaker
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match b.current() {
                    CircuitState::Open => Some(b.retry_after()),
                    CircuitState::Closed | CircuitState::HalfOpen => None,
                }
            };
            if let Some(retry_after) = retry_after {
                let mut state = handle
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let item = state.heap.pop().expect(
                    "dispatcher is the sole consumer; heap was non-empty at the last check",
                );
                drop(state);
                let _ = item.turn.send(Err(retry_after));
                continue;
            }
        }

        if next_allowed > Instant::now() {
            tokio::time::sleep_until(next_allowed).await;
        }

        let permit = handle
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("outbound queue semaphore is never closed");

        let mut state = handle
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let item = state
            .heap
            .pop()
            .expect("dispatcher is the sole consumer; heap was non-empty at the last check");

        // Grant the turn. `send` is the commit point: advance the pacing clock ONLY on a
        // successful hand-off. If the caller cancelled, its receiver is gone and `send`
        // returns the permit in `Err` — it drops here, freeing the in-flight slot, and no
        // pacing is consumed. A cancelled wait must never burn a slot or an interval.
        if item.turn.send(Ok(permit)).is_ok() {
            state.last_dispatch = Instant::now();
        }
        drop(state);
    }
}

/// A per-bucket wait-then-send queue.
///
/// Cheap to clone — state is shared via `Arc`. Production uses the ONE process-global
/// instance from [`shared`] so every `HttpFetcherImpl` reaches the SAME per-bucket
/// queues (the M-009 fix); tests build isolated instances via [`OutboundQueue::new`].
#[derive(Clone)]
pub struct OutboundQueue {
    registry: Arc<Mutex<HashMap<RateBucket, BucketHandle>>>,
}

impl OutboundQueue {
    /// A fresh, isolated queue with its own per-bucket state. For tests.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return the bucket's handle, creating it (and its dispatcher-less initial
    /// state) on first use.
    fn bucket_handle(&self, bucket: &RateBucket) -> BucketHandle {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry
            .entry(bucket.clone())
            .or_insert_with(|| BucketHandle::new(bucket, interval_for(bucket)))
            .clone()
    }

    /// Enqueue an outbound call for `bucket` at `priority` and await your turn.
    ///
    /// Resolves to `Ok(QueuePermit)` when it is this caller's turn: the dispatcher has
    /// paced the bucket (interval since the last ACTUAL dispatch) and acquired an
    /// in-flight permit. Ordering is `(priority DESC, enqueue_sequence ASC)` — highest
    /// priority first, FIFO within a priority via a process-monotonic enqueue
    /// sequence. The wait is UNBOUNDED; nothing is ever dropped. `RateBucket::None`
    /// bypasses pacing and the in-flight cap (immediate turn).
    ///
    /// Resolves to `Err(retry_after)` when the bucket's breaker is Open at the
    /// moment a turn would have been granted (R-3): no permit, no HTTP. `retry_after`
    /// is the time remaining until the breaker's open window elapses.
    ///
    /// Cancel-safe by construction: a caller dropped while still queued is skipped and
    /// does NOT consume a pacing slot; a caller dropped after dispatch releases its
    /// permit on drop. The pacing clock advances only on an actual dispatch.
    pub async fn acquire(
        &self,
        bucket: RateBucket,
        priority: RequestPriority,
    ) -> Result<QueuePermit, Duration> {
        if bucket == RateBucket::None {
            return Ok(QueuePermit { _permit: None });
        }

        let seq = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let handle = self.bucket_handle(&bucket);
        let (turn_tx, turn_rx) = oneshot::channel();

        {
            let mut state = handle
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.heap.push(QueuedItem {
                priority,
                seq,
                turn: turn_tx,
            });
            if !state.dispatcher_running {
                state.dispatcher_running = true;
                tokio::spawn(run_dispatcher(handle.clone(), interval_for(&bucket)));
            }
        }

        let result = turn_rx
            .await
            .expect("dispatcher dropped a queued item without granting its turn");
        result.map(|permit| QueuePermit {
            _permit: Some(permit),
        })
    }

    /// Report a dispatched call's outcome to `bucket`'s breaker (R-8/R-12/R-14).
    /// `None`/`Indexer(_)` buckets carry no breaker state — this is a no-op for
    /// them. O(1), a brief lock, never held across an `.await`.
    pub fn report_outcome(&self, bucket: RateBucket, outcome: BreakerSignal) {
        let handle = self.bucket_handle(&bucket);
        if let Some(breaker) = &handle.breaker {
            breaker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .apply(outcome);
        }
    }

    /// Test-only: reset `bucket`'s breaker to a fresh Closed state.
    ///
    /// [`shared`] is a process-global singleton (M-009) — every test in a
    /// binary that drives a real `HttpFetcherImpl` against a real bucket
    /// shares the SAME breaker state. A test that deliberately trips a
    /// breaker (anti-bot, a 5xx run) must reset it afterward so sibling
    /// tests in the same process don't inherit an Open breaker.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn reset_breaker_for_tests(&self, bucket: RateBucket) {
        let handle = self.bucket_handle(&bucket);
        if let Some(breaker) = &handle.breaker {
            *breaker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                BreakerState::new(breaker::config_for(&bucket));
        }
    }

    /// Test-only: replace `bucket`'s breaker with one built from a caller-supplied
    /// config (e.g. a short `open_duration_secs` so an Open→HalfOpen transition
    /// doesn't require a real wall-clock wait). Mirrors the enrichment queue's
    /// own `with_initial_circuit_state` test seam. A no-op for exempt buckets
    /// (`None`/`Indexer(_)`) — they carry no breaker to replace.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn set_breaker_config_for_tests(
        &self,
        bucket: RateBucket,
        config: breaker::CircuitBreakerConfig,
    ) {
        let handle = self.bucket_handle(&bucket);
        if let Some(breaker) = &handle.breaker {
            *breaker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = BreakerState::new(config);
        }
    }
}

impl Default for OutboundQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-global shared queue.
///
/// Every production `HttpFetcherImpl` routes through this ONE instance, so per-bucket
/// pacing is coordinated across the whole process (M-009). Backed by a `OnceLock`.
pub fn shared() -> OutboundQueue {
    static SHARED: OnceLock<OutboundQueue> = OnceLock::new();
    SHARED.get_or_init(OutboundQueue::new).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::{advance, timeout, Duration};

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn same_priority_waiters_dispatch_fifo_within_a_bucket() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_first = Arc::clone(&queue);
        let tx_first = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_first
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await
                .unwrap();
            tx_first.send(0usize).unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(0));

        let queue_second = Arc::clone(&queue);
        let tx_second = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_second
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await
                .unwrap();
            tx_second.send(1usize).unwrap();
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let queue_third = Arc::clone(&queue);
        let tx_third = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_third
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await
                .unwrap();
            tx_third.send(2usize).unwrap();
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(1));

        advance(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(2));
    }

    #[tokio::test(start_paused = true)]
    async fn same_bucket_dispatches_are_paced_by_bucket_interval() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_first = Arc::clone(&queue);
        let tx_first = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_first
                .acquire(RateBucket::GoogleBooks, RequestPriority::Normal)
                .await
                .unwrap();
            tx_first.send(0usize).unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(0));

        let queue_second = Arc::clone(&queue);
        let tx_second = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_second
                .acquire(RateBucket::GoogleBooks, RequestPriority::Normal)
                .await
                .unwrap();
            tx_second.send(1usize).unwrap();
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_millis(999)).await;
        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(1));
    }

    /// B3: `OpenLibraryCovers` paces at 3s — OL's ISBN-cover limit (~100/IP/5min
    /// ≈ 1 per 3s), a different interval from the 1s book-metadata buckets.
    #[tokio::test(start_paused = true)]
    async fn openlibrary_covers_bucket_is_paced_at_3_seconds() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_first = Arc::clone(&queue);
        let tx_first = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_first
                .acquire(RateBucket::OpenLibraryCovers, RequestPriority::Normal)
                .await
                .unwrap();
            tx_first.send(0usize).unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(0));

        let queue_second = Arc::clone(&queue);
        let tx_second = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_second
                .acquire(RateBucket::OpenLibraryCovers, RequestPriority::Normal)
                .await
                .unwrap();
            tx_second.send(1usize).unwrap();
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_millis(2999)).await;
        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(1));
    }

    #[tokio::test(start_paused = true)]
    async fn different_buckets_do_not_block_each_other_pacing() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_open_library = Arc::clone(&queue);
        let tx_open_library = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_open_library
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await
                .unwrap();
            tx_open_library.send(0usize).unwrap();
        });

        let queue_audnexus = Arc::clone(&queue);
        let tx_audnexus = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_audnexus
                .acquire(RateBucket::Audnexus, RequestPriority::Normal)
                .await
                .unwrap();
            tx_audnexus.send(1usize).unwrap();
        });

        settle().await;

        let mut granted = Vec::new();
        while let Ok(id) = rx.try_recv() {
            granted.push(id);
        }
        granted.sort_unstable();

        assert_eq!(granted, vec![0, 1]);
    }

    #[tokio::test(start_paused = true)]
    async fn higher_priority_waiter_beats_older_lower_priority_at_dispatch_time() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_first = Arc::clone(&queue);
        let tx_first = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_first
                .acquire(RateBucket::Hardcover, RequestPriority::Normal)
                .await
                .unwrap();
            tx_first.send("first").unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some("first"));

        let queue_low = Arc::clone(&queue);
        let tx_low = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_low
                .acquire(RateBucket::Hardcover, RequestPriority::Low)
                .await
                .unwrap();
            tx_low.send("low").unwrap();
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let queue_interactive = Arc::clone(&queue);
        let tx_interactive = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_interactive
                .acquire(RateBucket::Hardcover, RequestPriority::Interactive)
                .await
                .unwrap();
            tx_interactive.send("interactive").unwrap();
        });

        settle().await;

        advance(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some("interactive"));

        advance(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some("low"));
    }

    async fn assert_goodreads_in_flight_cap_blocks_third_until_holder_released() {
        let queue = Arc::new(OutboundQueue::new());
        let (granted_tx, mut granted_rx) = mpsc::unbounded_channel();
        let mut releases: Vec<oneshot::Sender<()>> = Vec::new();

        for id in 0usize..3 {
            let queue_holder = Arc::clone(&queue);
            let granted_tx_holder = granted_tx.clone();
            let (release_tx, release_rx) = oneshot::channel();
            releases.push(release_tx);

            tokio::spawn(async move {
                let permit = queue_holder
                    .acquire(RateBucket::Goodreads, RequestPriority::Normal)
                    .await
                    .unwrap();
                granted_tx_holder.send(id).unwrap();
                let _ = release_rx.await;
                drop(permit);
            });
        }

        settle().await;
        assert_eq!(granted_rx.try_recv().ok(), Some(0));
        assert!(matches!(
            granted_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_millis(1500)).await;
        settle().await;
        assert_eq!(granted_rx.try_recv().ok(), Some(1));
        assert!(matches!(
            granted_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        // Advance past the pacing interval: the third grant is still blocked,
        // proving the in-flight cap (not pacing) is what holds it.
        advance(Duration::from_millis(1500)).await;
        settle().await;
        assert!(matches!(
            granted_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let release_first_holder = releases.remove(0);
        let _ = release_first_holder.send(());

        settle().await;
        assert_eq!(granted_rx.try_recv().ok(), Some(2));

        for release in releases {
            let _ = release.send(());
        }
        settle().await;
    }

    #[tokio::test(start_paused = true)]
    async fn bounded_in_flight_blocks_third_until_a_permit_is_dropped() {
        assert_goodreads_in_flight_cap_blocks_third_until_holder_released().await;
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_dispatched_permit_frees_in_flight_slot() {
        assert_goodreads_in_flight_cap_blocks_third_until_holder_released().await;
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_queued_does_not_consume_pacing_slot_or_wedge_queue() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_first = Arc::clone(&queue);
        let tx_first = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_first
                .acquire(RateBucket::Audible, RequestPriority::Normal)
                .await
                .unwrap();
            tx_first.send("first").unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some("first"));

        let (cancel_tx, cancel_rx) = oneshot::channel();
        let queue_cancelled = Arc::clone(&queue);
        let cancelled = tokio::spawn(async move {
            tokio::select! {
                _permit = queue_cancelled.acquire(RateBucket::Audible, RequestPriority::Normal) => {
                    panic!("cancelled queued acquire should never dispatch");
                }
                _ = cancel_rx => {}
            }
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let queue_followup = Arc::clone(&queue);
        let tx_followup = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_followup
                .acquire(RateBucket::Audible, RequestPriority::Normal)
                .await
                .unwrap();
            tx_followup.send("followup").unwrap();
        });

        settle().await;
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let _ = cancel_tx.send(());
        settle().await;
        cancelled.await.unwrap();

        advance(Duration::from_millis(150)).await;
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some("followup"));
    }

    #[tokio::test]
    async fn shared_queue_acquire_completes_on_this_runtime_openlibrary() {
        let result = timeout(
            Duration::from_secs(5),
            shared().acquire(RateBucket::OpenLibrary, RequestPriority::Normal),
        )
        .await;

        assert!(
            result.is_ok(),
            "shared queue acquire must not hang across runtimes"
        );
    }

    #[tokio::test]
    async fn shared_queue_acquire_completes_on_this_runtime_audnexus() {
        let result = timeout(
            Duration::from_secs(5),
            shared().acquire(RateBucket::Audnexus, RequestPriority::Normal),
        )
        .await;

        assert!(
            result.is_ok(),
            "shared queue acquire must not hang across runtimes"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn none_bucket_bypasses_pacing_and_in_flight_cap() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        // Spawn 5 simultaneous None-bucket requests that hold their permits (park on a
        // long sleep). If None were paced or capped, only some would be granted; the
        // bypass means all 5 dispatch immediately.
        for id in 0..5 {
            let queue_holder = Arc::clone(&queue);
            let tx_holder = tx.clone();
            tokio::spawn(async move {
                let permit = queue_holder
                    .acquire(RateBucket::None, RequestPriority::Normal)
                    .await
                    .unwrap();
                tx_holder.send(id).unwrap();
                tokio::time::sleep(Duration::from_secs(10)).await;
                drop(permit);
            });
        }

        settle().await;
        let mut granted = Vec::new();
        while let Ok(id) = rx.try_recv() {
            granted.push(id);
        }
        granted.sort_unstable();
        assert_eq!(
            granted,
            vec![0, 1, 2, 3, 4],
            "all None-bucket bypass requests must be granted immediately"
        );
    }

    // -------------------------------------------------------------------
    // B2: per-bucket circuit breaker.
    // -------------------------------------------------------------------

    /// Below the default failure_threshold (5), the breaker stays Closed and
    /// every reported failure counts exactly once — 4 reports must not trip
    /// it; the 5th must (the double-count rule: N reports of Failure produce
    /// exactly N counted failures, never more).
    #[tokio::test]
    async fn breaker_stays_closed_below_threshold_and_opens_exactly_at_the_fifth_failure() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::OpenLibrary;

        for _ in 0..4 {
            queue.report_outcome(bucket.clone(), BreakerSignal::Failure);
        }
        queue
            .acquire(bucket.clone(), RequestPriority::Normal)
            .await
            .expect("4 failures must not trip a 5-failure threshold");

        queue.report_outcome(bucket.clone(), BreakerSignal::Failure);
        let retry_after = queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect_err("the 5th failure must trip Closed -> Open");
        assert!(
            retry_after > Duration::ZERO && retry_after <= Duration::from_secs(60),
            "retry_after should be within the default 60s open window, got {retry_after:?}"
        );
    }

    /// `TripImmediately` opens the breaker on a single report, bypassing the
    /// failure-threshold count entirely (used for a hard block like an
    /// anti-bot interstitial or a GB quota 403 — R-8/R-9).
    #[tokio::test]
    async fn trip_immediately_opens_on_a_single_report_bypassing_the_threshold() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::Hardcover;

        queue.report_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately { open_for: None },
        );

        queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect_err("a single TripImmediately report must open the breaker");
    }

    /// `TripImmediately { open_for: Some(d) }` overrides the bucket's
    /// configured open duration — the queue itself computes nothing
    /// provider-specific; the caller (e.g. GB's Pacific-midnight reset)
    /// supplies the window.
    #[tokio::test]
    async fn trip_immediately_open_for_override_is_respected() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::GoogleBooks;

        queue.report_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_secs(42)),
            },
        );

        let retry_after = queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect_err("an Open breaker must reject the acquire");
        assert!(
            retry_after > Duration::from_secs(40) && retry_after <= Duration::from_secs(42),
            "retry_after should reflect the 42s override, not the bucket's default, got {retry_after:?}"
        );
    }

    /// Goodreads is an anti-bot-hostile scrape target — its default open
    /// duration is 3600s (R-1/R-9), not the 60s every other breaker-tracked
    /// bucket uses.
    #[tokio::test]
    async fn goodreads_bucket_default_open_duration_is_3600_seconds() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::Goodreads;

        queue.report_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately { open_for: None },
        );

        let retry_after = queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect_err("an Open breaker must reject the acquire");
        assert!(
            retry_after > Duration::from_secs(3599) && retry_after <= Duration::from_secs(3600),
            "Goodreads' default open window is 3600s, got {retry_after:?}"
        );
    }

    /// `None` and (B3) `OpenLibraryCovers` are exempt from breaker tracking
    /// (R-5/R-6): no amount of reported failure ever trips them.
    /// `OpenLibraryCovers` is pace-only — it must NOT be in
    /// `breaker::breaker_tracked`'s allowlist. `Indexer(_)` is NOT exempt
    /// (see `indexer_bucket_is_breaker_tracked_and_trips_at_the_failure_threshold`
    /// below) — it is single-host via origin keying, so it carries a breaker
    /// like the six book-provider buckets.
    #[tokio::test]
    async fn none_and_ol_covers_buckets_never_trip_regardless_of_reported_failures() {
        let queue = OutboundQueue::new();
        let ol_covers = RateBucket::OpenLibraryCovers;

        for _ in 0..20 {
            queue.report_outcome(RateBucket::None, BreakerSignal::Failure);
            queue.report_outcome(ol_covers.clone(), BreakerSignal::Failure);
        }
        queue.report_outcome(
            RateBucket::None,
            BreakerSignal::TripImmediately { open_for: None },
        );
        queue.report_outcome(
            ol_covers.clone(),
            BreakerSignal::TripImmediately { open_for: None },
        );

        queue
            .acquire(RateBucket::None, RequestPriority::Normal)
            .await
            .expect("RateBucket::None must never trip");
        queue
            .acquire(ol_covers, RequestPriority::Normal)
            .await
            .expect("RateBucket::OpenLibraryCovers must never trip (pace-only, R-6)");
    }

    /// `Indexer(_)` is breaker-tracked: a plain reported `Failure` trips it
    /// via the normal failure-threshold path, exactly like the six book-
    /// provider buckets — origin keying makes each indexer bucket single-
    /// host, so tracking it honors the per-host principle instead of
    /// violating it. The 429-triggers-`TripImmediately`-with-a-30-minute-
    /// cooldown behavior is covered end-to-end by
    /// `crates/livrarr-http/tests/indexer_breaker_pins.rs`.
    #[tokio::test]
    async fn indexer_bucket_is_breaker_tracked_and_trips_at_the_failure_threshold() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::Indexer("unit-test-indexer-origin".to_string());

        for _ in 0..4 {
            queue.report_outcome(bucket.clone(), BreakerSignal::Failure);
        }
        queue
            .acquire(bucket.clone(), RequestPriority::Normal)
            .await
            .expect("4 failures must not trip a 5-failure threshold");

        queue.report_outcome(bucket.clone(), BreakerSignal::Failure);
        queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect_err("the 5th failure must trip an indexer bucket Closed -> Open");
    }

    /// Open → HalfOpen → Closed: once the open window elapses, the next
    /// acquire is granted as a probe; a reported Success on that probe closes
    /// the breaker (`half_open_probe_count: 1`), and the following acquire is
    /// granted normally — no waiting out another open window.
    #[tokio::test]
    async fn breaker_recovers_via_a_successful_half_open_probe() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::Audible;
        queue.set_breaker_config_for_tests(
            bucket.clone(),
            breaker::CircuitBreakerConfig {
                failure_threshold: 5,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        // A tiny explicit open_for gets to HalfOpen without a 60s real wait —
        // TripImmediately's override, not the queue computing anything
        // provider-specific.
        queue.report_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_millis(5)),
            },
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        let probe = queue
            .acquire(bucket.clone(), RequestPriority::Normal)
            .await
            .expect("HalfOpen must grant a probe turn once the open window elapses");
        queue.report_outcome(bucket.clone(), BreakerSignal::Success);
        drop(probe);

        queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect("breaker must be Closed again after the probe succeeded");
    }

    /// A reported Failure on a HalfOpen probe reopens the breaker (fresh
    /// `open_duration_secs` window) rather than closing it.
    #[tokio::test]
    async fn half_open_probe_failure_reopens_the_breaker() {
        let queue = OutboundQueue::new();
        let bucket = RateBucket::Audible;
        queue.set_breaker_config_for_tests(
            bucket.clone(),
            breaker::CircuitBreakerConfig {
                failure_threshold: 5,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        queue.report_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_millis(5)),
            },
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        let probe = queue
            .acquire(bucket.clone(), RequestPriority::Normal)
            .await
            .expect("HalfOpen must grant a probe turn once the open window elapses");
        queue.report_outcome(bucket.clone(), BreakerSignal::Failure);
        drop(probe);

        // Reopened with the configured 60s window (not another 5ms override):
        // the very next acquire, made with no further wait, must see Open.
        queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect_err("a HalfOpen probe failure must reopen the breaker");
    }

    #[tokio::test]
    async fn poisoned_bucket_state_lock_does_not_panic_dispatcher() {
        let handle = BucketHandle::new(&RateBucket::OpenLibrary, Duration::ZERO);
        let state = Arc::clone(&handle.state);

        let poison = std::thread::spawn(move || {
            let _guard = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            panic!("poison isolated bucket state");
        })
        .join();
        assert!(poison.is_err());
        assert!(handle.state.is_poisoned());

        let result = tokio::spawn(run_dispatcher(handle, Duration::ZERO)).await;
        assert!(
            result.is_ok(),
            "dispatcher should tolerate a poisoned bucket state lock"
        );
    }
}
