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

/// Maximum concurrent in-flight sends per bucket.
///
/// Separate control from pacing: pacing spaces the *start* of each send; this caps
/// how many may be *in flight* at once when a response outlives the interval. Carries
/// over the enrichment layer's concurrency of 2 — confirm at review.
pub const OUTBOUND_IN_FLIGHT_CAP: usize = 2;

/// RAII "your turn to send" signal handed to a caller when the dispatcher releases it.
///
/// Holds the in-flight permit; dropping it — on send completion OR on caller
/// cancellation — frees the in-flight slot. Opaque: hold it across the HTTP send and
/// body read, then let it drop. There is nothing to call on it. A bypass call
/// (`RateBucket::None`) holds no permit (`None`).
pub struct QueuePermit {
    _permit: Option<OwnedSemaphorePermit>,
}

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
    turn: oneshot::Sender<OwnedSemaphorePermit>,
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

/// Shared handle to one bucket's queue state and its in-flight permit pool. Cheap to
/// clone — both fields are `Arc`s.
#[derive(Clone)]
struct BucketHandle {
    state: Arc<Mutex<BucketState>>,
    semaphore: Arc<Semaphore>,
}

impl BucketHandle {
    fn new(interval: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(BucketState {
                heap: BinaryHeap::new(),
                dispatcher_running: false,
                // Backdated so the bucket's first-ever dispatch does not wait a full
                // interval.
                last_dispatch: Instant::now() - interval,
            })),
            semaphore: Arc::new(Semaphore::new(OUTBOUND_IN_FLIGHT_CAP)),
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

/// Minimum interval between dispatches for a bucket.
///
/// Duplicates `fetcher.rs`'s `interval_for` on purpose for now — removed once
/// `do_fetch` adopts this queue.
fn interval_for(bucket: &RateBucket) -> Duration {
    match bucket {
        RateBucket::OpenLibrary
        | RateBucket::Goodreads
        | RateBucket::Hardcover
        | RateBucket::GoogleBooks => Duration::from_secs(1),
        RateBucket::Audnexus => Duration::from_secs(2),
        RateBucket::Audible => Duration::from_millis(150),
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
            let mut state = handle.state.lock().unwrap();
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

        if next_allowed > Instant::now() {
            tokio::time::sleep_until(next_allowed).await;
        }

        let permit = handle
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("outbound queue semaphore is never closed");

        let mut state = handle.state.lock().unwrap();
        let item = state
            .heap
            .pop()
            .expect("dispatcher is the sole consumer; heap was non-empty at the last check");

        // Grant the turn. `send` is the commit point: advance the pacing clock ONLY on a
        // successful hand-off. If the caller cancelled, its receiver is gone and `send`
        // returns the permit in `Err` — it drops here, freeing the in-flight slot, and no
        // pacing is consumed. A cancelled wait must never burn a slot or an interval.
        if item.turn.send(permit).is_ok() {
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
        let mut registry = self.registry.lock().unwrap();
        registry
            .entry(bucket.clone())
            .or_insert_with(|| BucketHandle::new(interval_for(bucket)))
            .clone()
    }

    /// Enqueue an outbound call for `bucket` at `priority` and await your turn.
    ///
    /// Resolves to a [`QueuePermit`] when it is this caller's turn: the dispatcher has
    /// paced the bucket (interval since the last ACTUAL dispatch) and acquired an
    /// in-flight permit. Ordering is `(priority DESC, enqueue_sequence ASC)` — highest
    /// priority first, FIFO within a priority via a process-monotonic enqueue
    /// sequence. The wait is UNBOUNDED; nothing is ever dropped. `RateBucket::None`
    /// bypasses pacing and the in-flight cap (immediate turn).
    ///
    /// Cancel-safe by construction: a caller dropped while still queued is skipped and
    /// does NOT consume a pacing slot; a caller dropped after dispatch releases its
    /// permit on drop. The pacing clock advances only on an actual dispatch.
    pub async fn acquire(&self, bucket: RateBucket, priority: RequestPriority) -> QueuePermit {
        if bucket == RateBucket::None {
            return QueuePermit { _permit: None };
        }

        let seq = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let handle = self.bucket_handle(&bucket);
        let (turn_tx, turn_rx) = oneshot::channel();

        {
            let mut state = handle.state.lock().unwrap();
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

        let permit = turn_rx
            .await
            .expect("dispatcher dropped a queued item without granting its turn");
        QueuePermit {
            _permit: Some(permit),
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
                .await;
            tx_first.send(0usize).unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(0));

        let queue_second = Arc::clone(&queue);
        let tx_second = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_second
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
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
                .await;
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
                .await;
            tx_first.send(0usize).unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(0));

        let queue_second = Arc::clone(&queue);
        let tx_second = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_second
                .acquire(RateBucket::GoogleBooks, RequestPriority::Normal)
                .await;
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

    #[tokio::test(start_paused = true)]
    async fn different_buckets_do_not_block_each_other_pacing() {
        let queue = Arc::new(OutboundQueue::new());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let queue_open_library = Arc::clone(&queue);
        let tx_open_library = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_open_library
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            tx_open_library.send(0usize).unwrap();
        });

        let queue_audnexus = Arc::clone(&queue);
        let tx_audnexus = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_audnexus
                .acquire(RateBucket::Audnexus, RequestPriority::Normal)
                .await;
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
                .await;
            tx_first.send("first").unwrap();
        });

        settle().await;
        assert_eq!(rx.try_recv().ok(), Some("first"));

        let queue_low = Arc::clone(&queue);
        let tx_low = tx.clone();
        tokio::spawn(async move {
            let _permit = queue_low
                .acquire(RateBucket::Hardcover, RequestPriority::Low)
                .await;
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
                .await;
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
                    .await;
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

        advance(Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(granted_rx.try_recv().ok(), Some(1));
        assert!(matches!(
            granted_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        advance(Duration::from_secs(1)).await;
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
                .await;
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
                .await;
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
                    .await;
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
}
