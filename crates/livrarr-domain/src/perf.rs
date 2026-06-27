//! Process self-measurement helpers for performance diagnostics.

use std::time::Instant;

/// Times one stage of the works pipeline and emits a single log line when
/// dropped. The line goes to the `livrarr::perf` tracing target at DEBUG —
/// **silent unless that target is raised to debug** (e.g.
/// `RUST_LOG=livrarr::perf=debug`), so it costs nothing in normal operation.
///
/// Usage: hold the guard for the scope you want to time:
/// ```ignore
/// let _span = StageTimer::start("enrich", work_id);
/// // ... work ...
/// // logs "stage=enrich work_id=… elapsed_ms=… rss_bytes=…" here, on drop
/// ```
pub struct StageTimer {
    stage: &'static str,
    work_id: i64,
    start: Instant,
}

impl StageTimer {
    pub fn start(stage: &'static str, work_id: i64) -> Self {
        Self {
            stage,
            work_id,
            start: Instant::now(),
        }
    }
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        tracing::debug!(
            target: "livrarr::perf",
            stage = self.stage,
            work_id = self.work_id,
            elapsed_ms = self.start.elapsed().as_millis() as u64,
            rss_bytes = process_rss_bytes(),
            "stage timing"
        );
    }
}

/// Current resident set size (physical RAM) of this process, in bytes.
///
/// Reads `VmRSS` from `/proc/self/status` (Linux). Returns `None` on any
/// platform without that file or on a read/parse failure — measurement never
/// fails the caller.
pub fn process_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            // Format: "VmRSS:\t   12345 kB"
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}
