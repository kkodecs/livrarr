//! Bounded fire-and-forget persistence for provider call records (REQ-001).
//!
//! `SqliteCallSink::record` never blocks the instrumented call path: it
//! `try_send`s into a bounded channel and counts drops when the channel is
//! full. A writer task batches records (up to [`BATCH_MAX`] or
//! [`BATCH_WINDOW`], whichever first) into `record_provider_calls`; a db
//! error drops the batch with a warning — telemetry must never take down
//! enrichment. On cancellation the channel is drained and flushed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::ProviderCallRecordDb;
use livrarr_domain::services::{ProviderCallRecord, ProviderCallSink};

const CHANNEL_BOUND: usize = 4096;
const BATCH_MAX: usize = 64;
const BATCH_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone)]
pub struct SqliteCallSink {
    tx: mpsc::Sender<ProviderCallRecord>,
    dropped: Arc<AtomicU64>,
}

impl ProviderCallSink for SqliteCallSink {
    fn record(&self, rec: ProviderCallRecord) {
        if self.tx.try_send(rec).is_err() {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            // Rate-limited: first drop and every 1000th thereafter.
            if n == 1 || n.is_multiple_of(1000) {
                tracing::warn!(dropped = n, "call-record sink full; dropping records");
            }
        }
    }
}

impl SqliteCallSink {
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Spawn the writer task and return the sink plus its join handle. The
/// caller awaits the handle after signalling `cancel` so the final drain
/// completes before process exit.
pub fn spawn_call_sink(
    db: SqliteDb,
    cancel: CancellationToken,
) -> (SqliteCallSink, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_BOUND);
    let sink = SqliteCallSink {
        tx,
        dropped: Arc::new(AtomicU64::new(0)),
    };
    let handle = tokio::spawn(writer_loop(db, rx, cancel));
    (sink, handle)
}

async fn writer_loop(
    db: SqliteDb,
    mut rx: mpsc::Receiver<ProviderCallRecord>,
    cancel: CancellationToken,
) {
    loop {
        // Wait for the first record of a batch (or shutdown).
        let first = tokio::select! {
            _ = cancel.cancelled() => break,
            rec = rx.recv() => match rec {
                Some(rec) => rec,
                None => break,
            },
        };

        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + BATCH_WINDOW;
        while batch.len() < BATCH_MAX {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep_until(deadline) => break,
                rec = rx.recv() => match rec {
                    Some(rec) => batch.push(rec),
                    None => break,
                },
            }
        }
        flush(&db, batch).await;
    }

    // Shutdown drain: persist whatever is still queued.
    rx.close();
    let mut batch = Vec::with_capacity(BATCH_MAX);
    while let Ok(rec) = rx.try_recv() {
        batch.push(rec);
        if batch.len() >= BATCH_MAX {
            flush(&db, std::mem::take(&mut batch)).await;
        }
    }
    flush(&db, batch).await;
}

async fn flush(db: &SqliteDb, batch: Vec<ProviderCallRecord>) {
    if batch.is_empty() {
        return;
    }
    let n = batch.len();
    if let Err(e) = db.record_provider_calls(batch).await {
        tracing::warn!("failed to persist {n} provider call records: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_domain::services::{CallOperation, CallOutcomeClass};

    fn make_rec() -> ProviderCallRecord {
        ProviderCallRecord {
            provider: "google_books".to_string(),
            operation: CallOperation::Enrich,
            work_id: None,
            started_at: chrono::Utc::now(),
            duration_ms: 5,
            outcome: CallOutcomeClass::Success,
            detail: None,
        }
    }

    /// IR directive: full channel → record() returns immediately, the drop
    /// counter increments, no deadlock. The receiver is held but never
    /// consumed — a wedged persistence backend.
    #[tokio::test]
    async fn record_never_blocks_when_backend_wedged() {
        let (tx, _rx) = mpsc::channel(2);
        let sink = SqliteCallSink {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
        };

        for _ in 0..10 {
            sink.record(make_rec());
        }

        assert_eq!(sink.dropped_count(), 8);
    }

    /// Records sent before shutdown are drained and persisted on cancel.
    #[tokio::test]
    async fn writer_flushes_and_drains_on_cancel() {
        let db = livrarr_db::create_test_db().await;
        let cancel = CancellationToken::new();
        let (sink, handle) = spawn_call_sink(db.clone(), cancel.clone());

        for _ in 0..3 {
            sink.record(make_rec());
        }
        cancel.cancel();
        handle.await.expect("writer task join");

        let stats = db.query_provider_stats_24h().await.expect("stats query");
        let gb = stats
            .iter()
            .find(|s| s.provider == "google_books")
            .expect("google_books row");
        assert_eq!(gb.calls_24h, 3);
    }
}
