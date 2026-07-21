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
    /// The per-indexer rate-limit breaker for this item's FULL bucket value,
    /// resolved once at enqueue time (`Some` only for a fully-resolved
    /// `Indexer { indexer: Some(_) }` bucket). The dispatcher gates on this
    /// independently of the lane's shared transport breaker (`BucketHandle.
    /// breaker`); it is not part of the heap ordering.
    rl_breaker: Option<Arc<Mutex<BreakerState>>>,
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
    /// The lane's TRANSPORT-level breaker, keyed by pace key (origin for
    /// indexers). `None` for `RateBucket::None` / `OpenLibraryCovers` and any
    /// future pace-only aggregate bucket. `Some` for every breaker-tracked
    /// pace bucket — the six book-provider APIs plus every `Indexer` origin
    /// lane. It answers "is this host up?" (connection errors / timeouts),
    /// never sees a 429, and is shared by all indexers on one origin. The
    /// per-indexer rate-limit breaker is separate (`OutboundQueue::
    /// rate_limit_breakers`) and lives on each `QueuedItem` as `rl_breaker`.
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
        RateBucket::Indexer { .. } => Duration::from_millis(500),
        RateBucket::None => Duration::ZERO,
    }
}

/// Project a bucket down to its PACING key — the key the registry (pacing +
/// in-flight cap + transport breaker) is stored under. Identity for every
/// variant except `Indexer`, whose per-indexer id is erased so all indexers
/// proxied through one origin share a single pace lane and one transport-level
/// breaker (politeness is to the machine). The per-indexer rate-limit breaker
/// is keyed by the FULL bucket value elsewhere (`rate_limit_breakers`), not by
/// this projection.
fn pace_key(bucket: &RateBucket) -> RateBucket {
    match bucket {
        RateBucket::Indexer { origin, .. } => RateBucket::Indexer {
            origin: origin.clone(),
            indexer: None,
        },
        other => other.clone(),
    }
}

/// If `breaker` is `Some` and currently Open, the time remaining in its open
/// window; otherwise `None`. `current()` transitions Open→HalfOpen internally
/// once the window elapses, so a HalfOpen breaker returns `None` and a probe is
/// admitted. Mutates the breaker (the window transition), so callers hold the
/// state lock across it — the breaker is a leaf lock, taken state→breaker.
fn breaker_open_retry(breaker: Option<&Arc<Mutex<BreakerState>>>) -> Option<Duration> {
    let breaker = breaker?;
    let mut b = breaker
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match b.current() {
        CircuitState::Open => Some(b.retry_after()),
        CircuitState::Closed | CircuitState::HalfOpen => None,
    }
}

/// One pace lane's dispatcher loop: pace sends by `interval`, cap in-flight
/// sends via the lane's semaphore, and grant queued callers their turn in
/// `(priority DESC, seq ASC)` order. Two breaker levels gate a grant — the
/// item's per-indexer rate-limit breaker (`QueuedItem::rl_breaker`) and the
/// lane's shared transport breaker (`handle.breaker`); an Open breaker on
/// either rejects the item with `Err(retry_after)` and no HTTP. Exits —
/// self-cleaning via [`DispatcherGuard`] — once the queue drains empty; the
/// next `acquire` call on this lane respawns it.
async fn run_dispatcher(handle: BucketHandle, interval: Duration) {
    let mut guard = DispatcherGuard {
        state: Arc::clone(&handle.state),
        armed: true,
    };

    loop {
        // Phase 1 — early drain + breaker gate, under ONE state-lock hold
        // (peek/check/pop share the hold, so there is no rl-breaker TOCTOU):
        // reject each top item whose rate-limit breaker OR the lane's transport
        // breaker is Open — popped and errored with no pacing slot and no
        // in-flight permit consumed — until the top item is grantable or the
        // heap drains empty.
        let next_allowed = {
            let mut state = handle
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                if state.heap.is_empty() {
                    // Clear the flag ATOMICALLY with the empty check (same lock
                    // hold) so a concurrent `acquire` cannot observe a stale
                    // `true`, skip spawning, and orphan its item. Disarm the
                    // guard — this clean path already cleared the flag.
                    state.dispatcher_running = false;
                    guard.disarm();
                    return;
                }
                // Clone the top item's rl_breaker handle so the immutable peek
                // borrow is released before any mutable pop.
                let top_rl = state.heap.peek().and_then(|item| item.rl_breaker.clone());
                let retry_after = breaker_open_retry(top_rl.as_ref())
                    .or_else(|| breaker_open_retry(handle.breaker.as_ref()));
                match retry_after {
                    Some(retry_after) => {
                        let item = state.heap.pop().expect(
                            "dispatcher is the sole consumer; heap was non-empty at the last check",
                        );
                        let _ = item.turn.send(Err(retry_after));
                        // Re-peek the new top under the same lock hold.
                    }
                    None => break state.last_dispatch + interval,
                }
            }
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

        // Phase 3 — grant-time re-check: the item now on top may differ from
        // the one Phase 1 cleared (a higher-priority item could have arrived
        // during the pacing sleep / semaphore wait) and either breaker may have
        // tripped in that window. Pop and re-check BOTH before granting.
        let mut state = handle
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let item = state
            .heap
            .pop()
            .expect("dispatcher is the sole consumer; heap was non-empty at the last check");
        let retry_after = breaker_open_retry(item.rl_breaker.as_ref())
            .or_else(|| breaker_open_retry(handle.breaker.as_ref()));
        if let Some(retry_after) = retry_after {
            // Tripped during the wait (or the heap reordered to a tripped
            // item): reject, free the permit, and DO NOT advance the pacing
            // clock — only a successful hand-off is a dispatch.
            drop(state);
            drop(permit);
            let _ = item.turn.send(Err(retry_after));
            continue;
        }

        // Grant the turn. `send` is the commit point: advance the pacing clock
        // ONLY on a successful hand-off. If the caller cancelled, its receiver
        // is gone and `send` returns the permit in `Err` — it drops here,
        // freeing the in-flight slot, and no pacing is consumed. A cancelled
        // wait must never burn a slot or an interval.
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
    /// The second breaker level (issue #130): per-INDEXER rate-limit breakers,
    /// keyed by the FULL `Indexer { origin, indexer: Some(id) }` bucket value —
    /// distinct from `registry`, which is keyed by the pace projection (origin
    /// only). Created lazily on first use of a resolvable indexer bucket and
    /// bounded by the number of configured indexers. A 429 for one indexer
    /// trips only its own entry; its neighbours on the same origin are
    /// untouched.
    rate_limit_breakers: Arc<Mutex<HashMap<RateBucket, Arc<Mutex<BreakerState>>>>>,
}

impl OutboundQueue {
    /// A fresh, isolated queue with its own per-bucket state. For tests.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(HashMap::new())),
            rate_limit_breakers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return the pace lane's handle (pacing, in-flight cap, transport breaker),
    /// creating it on first use. Keyed by the PACE projection, so all indexers
    /// on one origin share a handle.
    fn bucket_handle(&self, bucket: &RateBucket) -> BucketHandle {
        let key = pace_key(bucket);
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry
            .entry(key.clone())
            .or_insert_with(|| BucketHandle::new(&key, interval_for(&key)))
            .clone()
    }

    /// The per-indexer rate-limit breaker for `bucket`, created on first use.
    /// `Some` only for a fully-resolved indexer bucket (`Indexer { indexer:
    /// Some(_) }`); `None` for everything else — provider buckets (whose only
    /// breaker is the transport one), `Indexer { indexer: None }`, `None`, and
    /// covers. Keyed by the full bucket value, so two indexers on one origin
    /// get two distinct breakers.
    fn rate_limit_breaker(&self, bucket: &RateBucket) -> Option<Arc<Mutex<BreakerState>>> {
        if !matches!(
            bucket,
            RateBucket::Indexer {
                indexer: Some(_),
                ..
            }
        ) {
            return None;
        }
        let mut map = self
            .rate_limit_breakers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(
            map.entry(bucket.clone())
                .or_insert_with(|| {
                    Arc::new(Mutex::new(BreakerState::new(breaker::config_for(bucket))))
                })
                .clone(),
        )
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
        // Resolve the per-indexer rate-limit breaker ONCE, before taking the
        // state lock (the map lock is a leaf lock released here). `None` for
        // every non-`Indexer{Some}` bucket.
        let rl_breaker = self.rate_limit_breaker(&bucket);
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
                rl_breaker,
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

    /// Report a dispatched call's outcome to `bucket`'s TRANSPORT-level breaker
    /// (R-8/R-12/R-14) — the pace-lane breaker, keyed by pace key. This is the
    /// reporter the six provider clients call (their bucket's only breaker) and
    /// the one `do_fetch` uses for transport failures and host-alive successes
    /// on every bucket, indexers included. `None`/`OpenLibraryCovers` carry no
    /// breaker — a no-op for them. The per-indexer rate-limit level has its own
    /// reporter (`report_rate_limit_outcome`). O(1), a brief lock, never held
    /// across an `.await`.
    pub fn report_outcome(&self, bucket: RateBucket, outcome: BreakerSignal) {
        let handle = self.bucket_handle(&bucket);
        if let Some(breaker) = &handle.breaker {
            breaker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .apply(outcome);
        }
    }

    /// Report an outcome to `bucket`'s per-INDEXER rate-limit breaker (issue
    /// #130). A no-op unless `bucket` is a fully-resolved `Indexer { indexer:
    /// Some(_) }`. `do_fetch` calls this with `TripImmediately` on a 429 (that
    /// one indexer is rate-limited) and `Success` on any completed non-429
    /// response (which closes a half-open probe). Disjoint from the transport
    /// level: this breaker never sees a transport failure. O(1), a brief lock,
    /// never held across an `.await`.
    pub fn report_rate_limit_outcome(&self, bucket: RateBucket, outcome: BreakerSignal) {
        if let Some(breaker) = self.rate_limit_breaker(&bucket) {
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
    /// own `with_initial_circuit_state` test seam. Operates on the TRANSPORT
    /// breaker (pace key); a no-op for pace-only buckets
    /// (`None`/`OpenLibraryCovers`) — they carry no breaker to replace.
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

    // -------------------------------------------------------------------
    // Two-level indexer breaker (issue #130): a shared TRANSPORT breaker per
    // origin ("is the host up?") and a per-INDEXER rate-limit breaker
    // ("is this one indexer 429ing?"). The two levels see disjoint signals.
    // -------------------------------------------------------------------

    fn indexer_bucket(origin: &str, id: &str) -> RateBucket {
        RateBucket::Indexer {
            origin: origin.to_string(),
            indexer: Some(id.to_string()),
        }
    }

    /// An indexer lane's TRANSPORT breaker trips on the normal failure
    /// threshold, exactly like the six provider buckets: 5 reported `Failure`s
    /// (host down / timeouts) open it. `report_outcome` targets the transport
    /// level (keyed by origin).
    #[tokio::test]
    async fn indexer_transport_breaker_trips_at_the_failure_threshold() {
        let queue = OutboundQueue::new();
        let bucket = indexer_bucket("unit-test-indexer-origin", "unit-test-indexer-id");

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
            .expect_err("the 5th transport failure must trip the indexer lane Closed -> Open");
    }

    /// #130 regression (load-bearing): two indexers on ONE origin. Tripping A's
    /// per-indexer rate-limit breaker rejects A but leaves sibling B free —
    /// with no extra pacing wait, since A's rejection consumes no dispatch.
    #[tokio::test]
    async fn rate_limit_trip_on_one_indexer_does_not_block_a_same_origin_sibling() {
        let queue = OutboundQueue::new();
        let a = indexer_bucket("origin-130", "id-a");
        let b = indexer_bucket("origin-130", "id-b");

        queue.report_rate_limit_outcome(
            a.clone(),
            BreakerSignal::TripImmediately { open_for: None },
        );

        queue
            .acquire(a, RequestPriority::Normal)
            .await
            .expect_err("indexer A's own rate-limit breaker is open");
        queue
            .acquire(b, RequestPriority::Normal)
            .await
            .expect("sibling indexer B on the same origin must not be blocked by A's 429 (#130)");
    }

    /// An OPEN high-priority indexer must not starve a CLOSED low-priority
    /// sibling on the same origin: both queued at once, the open one is
    /// rejected and the closed one is still granted.
    #[tokio::test]
    async fn open_high_priority_indexer_does_not_starve_a_closed_low_priority_sibling() {
        let queue = Arc::new(OutboundQueue::new());
        let a = indexer_bucket("origin-mixed", "id-a");
        let b = indexer_bucket("origin-mixed", "id-b");
        queue.report_rate_limit_outcome(
            a.clone(),
            BreakerSignal::TripImmediately { open_for: None },
        );

        let qa = Arc::clone(&queue);
        let a_task =
            tokio::spawn(async move { qa.acquire(a, RequestPriority::Interactive).await.is_err() });
        let qb = Arc::clone(&queue);
        let b_task = tokio::spawn(async move { qb.acquire(b, RequestPriority::Low).await.is_ok() });

        assert!(
            a_task.await.unwrap(),
            "open high-priority indexer A must be rejected"
        );
        assert!(
            b_task.await.unwrap(),
            "closed low-priority sibling B must still be granted"
        );
    }

    /// Grant-time re-check: a rate-limit breaker that trips WHILE its item is in
    /// the pacing sleep is caught when the permit is granted — the item is
    /// rejected, the permit released, and the pacing clock not advanced.
    #[tokio::test(start_paused = true)]
    async fn rate_limit_trip_during_pacing_sleep_is_caught_at_grant_time() {
        let queue = Arc::new(OutboundQueue::new());
        let bucket = indexer_bucket("origin-race", "id-race");

        // First grant advances the pacing clock so the SECOND item must wait a
        // full 500ms interval — the window in which we trip the breaker.
        let first = queue
            .acquire(bucket.clone(), RequestPriority::Normal)
            .await
            .expect("first grant");
        drop(first);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let q2 = Arc::clone(&queue);
        let b2 = bucket.clone();
        tokio::spawn(async move {
            let res = q2.acquire(b2, RequestPriority::Normal).await;
            tx.send(res.is_err()).unwrap();
        });
        settle().await;
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "second item must still be in the pacing sleep, not yet granted"
        );

        // Trip the rate-limit breaker mid-sleep, then let the sleep elapse.
        queue.report_rate_limit_outcome(bucket, BreakerSignal::TripImmediately { open_for: None });
        advance(Duration::from_millis(500)).await;
        settle().await;
        assert_eq!(
            rx.try_recv().ok(),
            Some(true),
            "an item whose breaker tripped mid-sleep must be rejected at grant time"
        );
    }

    /// The transport breaker is shared per ORIGIN: transport failures open the
    /// whole lane (every indexer on that origin), but a different origin is
    /// untouched.
    #[tokio::test]
    async fn transport_failures_trip_the_whole_origin_lane_but_not_other_origins() {
        let queue = OutboundQueue::new();
        let x_a = indexer_bucket("origin-x", "id-a");
        let x_b = indexer_bucket("origin-x", "id-b");
        let y = indexer_bucket("origin-y", "id-a");

        for _ in 0..5 {
            queue.report_outcome(x_a.clone(), BreakerSignal::Failure);
        }

        queue
            .acquire(x_a, RequestPriority::Normal)
            .await
            .expect_err("origin X's transport breaker is open");
        queue
            .acquire(x_b, RequestPriority::Normal)
            .await
            .expect_err("a sibling indexer on origin X shares the open transport breaker");
        queue
            .acquire(y, RequestPriority::Normal)
            .await
            .expect("a different origin must be unaffected");
    }

    /// A 429 STORM (rate-limit trip + transport success, mirroring `do_fetch`'s
    /// 429 path) trips only the indexer's rate-limit breaker and never moves the
    /// shared transport breaker — a sibling on the same origin keeps flowing.
    #[tokio::test]
    async fn a_429_storm_trips_the_indexer_but_leaves_the_transport_lane_closed() {
        let queue = OutboundQueue::new();
        let a = indexer_bucket("origin-429storm", "id-a");
        let b = indexer_bucket("origin-429storm", "id-b");

        for _ in 0..10 {
            queue.report_rate_limit_outcome(
                a.clone(),
                BreakerSignal::TripImmediately { open_for: None },
            );
            queue.report_outcome(a.clone(), BreakerSignal::Success);
        }

        queue
            .acquire(a, RequestPriority::Normal)
            .await
            .expect_err("indexer A's rate-limit breaker is open after its 429s");
        queue
            .acquire(b, RequestPriority::Normal)
            .await
            .expect("the transport lane stayed closed under a 429 storm — sibling B proceeds");
    }

    /// `Indexer { indexer: None }` (unresolved-identity fallback) has NO
    /// rate-limit breaker — a rate-limit trip report is a no-op — but it IS
    /// still subject to its origin's transport gate.
    #[tokio::test]
    async fn indexer_with_no_id_has_no_rate_limit_breaker_but_shares_the_transport_gate() {
        let queue = OutboundQueue::new();
        let none_id = RateBucket::Indexer {
            origin: "origin-noid".to_string(),
            indexer: None,
        };

        queue.report_rate_limit_outcome(
            none_id.clone(),
            BreakerSignal::TripImmediately { open_for: None },
        );
        queue
            .acquire(none_id.clone(), RequestPriority::Normal)
            .await
            .expect("indexer:None has no rate-limit breaker to trip");

        for _ in 0..5 {
            queue.report_outcome(none_id.clone(), BreakerSignal::Failure);
        }
        queue
            .acquire(none_id, RequestPriority::Normal)
            .await
            .expect_err("indexer:None is still subject to the transport lane gate");
    }

    /// The rate-limit breaker recovers per its own level: once its cooldown
    /// elapses it admits a probe, and a Success (a completed non-429 response)
    /// closes it.
    #[tokio::test]
    async fn rate_limit_breaker_recovers_via_a_successful_probe() {
        let queue = OutboundQueue::new();
        let bucket = indexer_bucket("origin-halfopen", "id-a");

        // Tiny open window so HalfOpen is reached without a long real wait.
        queue.report_rate_limit_outcome(
            bucket.clone(),
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_millis(5)),
            },
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        queue
            .acquire(bucket.clone(), RequestPriority::Normal)
            .await
            .expect("HalfOpen must admit a probe once the cooldown elapses");
        queue.report_rate_limit_outcome(bucket.clone(), BreakerSignal::Success);
        queue
            .acquire(bucket, RequestPriority::Normal)
            .await
            .expect("the rate-limit breaker must be closed after a successful probe");
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
