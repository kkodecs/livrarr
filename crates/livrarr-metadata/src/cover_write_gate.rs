//! The consolidated cover save gate (S2). Every road that saves an automatic
//! cover — add-time enrichment, refresh, background retry, all funneling
//! through `work_service::run_unified_enrichment` — routes through
//! [`run_cover_write_gate`]. It wires the previously-dormant size/trust
//! rank comparator into the single chokepoint with a crash-safe commit protocol:
//! `works.cover_*` (and the audiobook twins) update ONLY on an accepted swap
//! or an initial save, atomically with the file write, never in the generic
//! enrichment field merge — so the row always describes the file actually on
//! disk (the binding invariant).
//!
//! A user's own choice (`select_cover`/`upload_cover`, via
//! [`run_user_cover_write`]) runs the identical crash-safe commit protocol —
//! same slot lock, same tmp+meta/DB/rename/cleanup steps — but skips the
//! enrichment-only guards: a user's pick is absolute, including replacing
//! their own earlier one. The two entry points share every stage from
//! "acquire candidate bytes" onward (R3).
//!
//! Commit order (every step durable before the next is visible):
//! 1. write `{id}{suffix}.candidate.tmp` (bytes) + `.candidate.meta.json`
//!    (url/source/manual/dims) before anything else changes.
//! 2. the comparator decides accept/reject (enrichment only — a user's own
//!    pick always accepts once its bytes are validated).
//! 3. reject: delete both candidate files, row and final file untouched.
//! 4. accept: update the DB row (url/source/trust/dims) — the commit point.
//! 5. rename tmp -> final (atomic, same filesystem).
//! 6. delete the meta sidecar.
//!
//! The whole protocol runs under a process-wide per-(user, work, slot) lock:
//! two concurrent runs for the same slot share the `{id}{suffix}.candidate.*`
//! paths, so unserialized they corrupt each other's protocol state (one run's
//! rename consuming the other's tmp, a commit landing against a decision made
//! from a stale snapshot). The incumbent's state is read from the work row
//! INSIDE the lock, so the second run always decides against the first run's
//! committed result, never a stale caller-captured snapshot. Crash recovery
//! (`cover_write_gate_recovery`) takes the same lock per candidate it
//! processes.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use livrarr_db::WorkDb;
use livrarr_domain::keyed_mutex::{KeyedMutex, KeyedMutexGuard};
use livrarr_domain::services::HttpFetcher;
use livrarr_domain::{CoverMediaType, CoverResolution, RequestPriority, UserId, Work, WorkId};
use livrarr_enrichment::cover_rank::CoverRankModel;
use livrarr_enrichment::cover_resolution::should_upgrade_same_tier;

/// One lock per (user, work, slot). Process-wide because the writers span
/// service instances: every `run_unified_enrichment` caller (add, refresh,
/// background retry, convergence) plus startup crash recovery, all writing
/// into one shared covers directory.
static SLOT_LOCKS: LazyLock<KeyedMutex<(UserId, WorkId, CoverMediaType)>> =
    LazyLock::new(KeyedMutex::new);

/// Acquire the write-gate lock for one cover slot. Held for the entire save
/// protocol; crash recovery holds it around each candidate it converges.
pub(crate) async fn lock_slot(
    user_id: UserId,
    work_id: WorkId,
    media_type: CoverMediaType,
) -> KeyedMutexGuard<(UserId, WorkId, CoverMediaType)> {
    SLOT_LOCKS.lock((user_id, work_id, media_type)).await
}

/// D3 #8 / R-5: `KeyedMutex::sweep()` is the backstop for permits `Drop`'s
/// opportunistic per-guard prune skips (only when the map is contended at
/// release) — it existed with zero production callers. This spawns a 300s
/// periodic sweep of `SLOT_LOCKS` for the life of the process. Call once
/// from the server composition root at startup. Returns `None` (never
/// panics) when no Tokio runtime is current; the returned handle exists so
/// tests can observe the task, production callers may discard it.
pub fn spawn_slot_locks_sweeper() -> Option<tokio::task::JoinHandle<()>> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    Some(handle.spawn(async {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            ticker.tick().await;
            SLOT_LOCKS.sweep().await;
        }
    }))
}

/// The durable sidecar recording a pending candidate's intended DB values.
/// Crash recovery reads this to tell a committed-but-unfinished save apart
/// from an undecided or rejected one. `url` is `None` for a user's byte
/// upload (there is no source URL to record); `#[serde(default)]` keeps
/// existing v1 sidecars — where `url` was a required `String` — parsing
/// unchanged (they deserialize as `Some(..)`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CandidateMeta {
    #[serde(default)]
    pub url: Option<String>,
    pub source: String,
    #[serde(default)]
    pub manual: bool,
    pub width: i32,
    pub height: i32,
}

pub(crate) fn candidate_tmp_path(covers_dir: &Path, work_id: WorkId, suffix: &str) -> PathBuf {
    covers_dir.join(format!("{work_id}{suffix}.candidate.tmp"))
}

pub(crate) fn candidate_meta_path(covers_dir: &Path, work_id: WorkId, suffix: &str) -> PathBuf {
    covers_dir.join(format!("{work_id}{suffix}.candidate.meta.json"))
}

pub(crate) fn final_cover_path(covers_dir: &Path, work_id: WorkId, suffix: &str) -> PathBuf {
    covers_dir.join(format!("{work_id}{suffix}.jpg"))
}

/// Observable result of offering one candidate to the gate.
#[derive(Debug)]
pub enum GateOutcome {
    /// No candidate or a manual ebook selection backed by an existing file.
    NoOp,
    /// Same URL already committed to disk — an unchanged pick on refresh
    /// must not re-download every pass.
    AlreadyCurrent,
    /// The candidate could not be fetched/decoded; incumbent untouched.
    DownloadFailed,
    /// The comparator rejected the candidate (AC-3): file and row untouched.
    Rejected,
    /// The candidate was accepted, committed, and is now the file on disk.
    Accepted {
        bytes: Vec<u8>,
        width: i32,
        height: i32,
    },
}

impl GateOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(self, GateOutcome::Accepted { .. })
    }
}

/// The incumbent's state for one cover slot, derived from the work row read
/// INSIDE the slot lock — never a caller-captured snapshot, which could be
/// stale by the time the lock is acquired.
struct CurrentCoverState {
    manual: bool,
    width: i32,
    height: i32,
    source: Option<String>,
    url: Option<String>,
}

fn current_state_for_slot(
    work: &Work,
    media_type: CoverMediaType,
    audiobook_manual: bool,
) -> CurrentCoverState {
    match media_type {
        CoverMediaType::Ebook => CurrentCoverState {
            manual: work.cover_manual,
            width: work.cover_width,
            height: work.cover_height,
            source: work.cover_source.clone(),
            url: work.cover_url.clone(),
        },
        CoverMediaType::Audiobook => CurrentCoverState {
            manual: audiobook_manual,
            width: work.audiobook_cover_width,
            height: work.audiobook_cover_height,
            source: work.audiobook_cover_source.clone(),
            url: work.audiobook_cover_url.clone(),
        },
    }
}

/// A manual ebook choice is absolute while its file exists. A stale manual
/// bit with no file must remain repairable by the normal ranked candidate.
fn manual_selection_blocks_candidate(current_manual: bool, file_exists: bool) -> bool {
    current_manual && file_exists
}

pub struct CoverWriteGateInput {
    pub covers_dir: PathBuf,
    pub work_id: WorkId,
    pub media_type: CoverMediaType,
    pub resolution: CoverResolution,
}

/// Offer one candidate cover to the save gate. `db`/`http` are the live
/// implementations; `user_id` scopes the slot lock, the DB write, and the
/// covers directory. The incumbent's trust/dims/source/url, the language
/// (foreign vs english rank order), and the sibling slot's URL (thumbnail
/// fallback rule) are all read from the work row under the lock.
pub async fn run_cover_write_gate<D, H>(
    db: &D,
    http: &H,
    user_id: UserId,
    input: CoverWriteGateInput,
) -> GateOutcome
where
    D: WorkDb + Sync,
    H: HttpFetcher,
{
    let CoverWriteGateInput {
        covers_dir,
        work_id,
        media_type,
        resolution,
    } = input;

    let _slot_guard = lock_slot(user_id, work_id, media_type).await;

    let work = match read_work_row(db, user_id, work_id).await {
        Ok(w) => w,
        Err(outcome) => return outcome,
    };
    let audiobook_manual = if media_type == CoverMediaType::Audiobook {
        match db.get_audiobook_cover_manual(user_id, work_id).await {
            Ok(manual) => manual,
            Err(error) => {
                tracing::warn!(
                    work_id,
                    error = %error,
                    "cover write gate: audiobook manual state unreadable"
                );
                return GateOutcome::NoOp;
            }
        }
    } else {
        false
    };
    let current = current_state_for_slot(&work, media_type, audiobook_manual);
    let foreign = matches!(
        livrarr_external_data::language::provider_priority(work.language.as_deref()),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    let sibling_cover_url = work.audiobook_cover_url.clone();

    let suffix = media_type.suffix();
    let final_path = final_cover_path(&covers_dir, work_id, suffix);

    // A manual selection is honored only when its cover actually exists on disk —
    // either the final .jpg, or the crash-safe protocol's committed-but-
    // unrenamed state (candidate meta sidecar present: the DB row is already
    // manual and startup recovery owns finishing the rename). A failed phase-1
    // add-time download leaves NOTHING on disk (0x0 dims, cover_url still set);
    // only that fully-empty shape is a damaged, replaceable slot. Only pay
    // for the existence checks when the row is actually manually selected.
    let locked_file_exists = if current.manual {
        tokio::fs::try_exists(&final_path).await.unwrap_or(false)
            || tokio::fs::try_exists(&candidate_meta_path(&covers_dir, work_id, suffix))
                .await
                .unwrap_or(false)
    } else {
        false
    };
    if manual_selection_blocks_candidate(current.manual, locked_file_exists) {
        return GateOutcome::NoOp;
    }

    if current.url.as_deref() == Some(resolution.url.as_str()) {
        let exists = tokio::fs::try_exists(&final_path).await.unwrap_or(false);
        if exists {
            return GateOutcome::AlreadyCurrent;
        }
    }

    if tokio::fs::create_dir_all(&covers_dir).await.is_err() {
        return GateOutcome::DownloadFailed;
    }

    let (bytes, new_w, new_h) = match fetch_candidate_bytes(http, work_id, &resolution.url).await {
        Ok(v) => v,
        Err(outcome) => return outcome,
    };

    let tmp_path = candidate_tmp_path(&covers_dir, work_id, suffix);
    let meta_path = candidate_meta_path(&covers_dir, work_id, suffix);
    let meta = CandidateMeta {
        url: Some(resolution.url.clone()),
        source: resolution.source.clone(),
        manual: false,
        width: new_w,
        height: new_h,
    };

    if write_candidate_files(&tmp_path, &meta_path, &bytes, &meta)
        .await
        .is_err()
    {
        // See `discard_candidate_files`'s doc for why the order matters.
        discard_candidate_files(&meta_path, &tmp_path).await;
        return GateOutcome::DownloadFailed;
    }

    let rank_model = CoverRankModel::for_media(media_type, foreign);
    let accept = should_upgrade_same_tier(
        current.width.max(0) as u32,
        current.height.max(0) as u32,
        new_w as u32,
        new_h as u32,
        current.source.as_deref(),
        &resolution.source,
        rank_model,
    );

    if !accept {
        // See `discard_candidate_files`'s doc for why the order matters.
        discard_candidate_files(&meta_path, &tmp_path).await;
        return GateOutcome::Rejected;
    }

    commit_and_finalize(
        db,
        user_id,
        work_id,
        media_type,
        &covers_dir,
        &tmp_path,
        &meta_path,
        &final_path,
        sibling_cover_url.as_deref(),
        meta,
        bytes,
    )
    .await
}

/// One user-chosen cover, offered to [`run_user_cover_write`].
pub struct UserCoverInput {
    pub covers_dir: PathBuf,
    pub work_id: WorkId,
    pub media_type: CoverMediaType,
    pub payload: UserCoverPayload,
}

/// A user's cover choice arrives either as a resolved URL (`select_cover`,
/// picking one of the alternatives already offered) or raw bytes
/// (`upload_cover`).
pub enum UserCoverPayload {
    Url { url: String, source: String },
    Bytes { data: Vec<u8> },
}

/// Failure detail a bare [`GateOutcome`] can't carry: a user's upload bytes
/// failed validation (size cap, format, dimensions), and the caller needs
/// the specific reason to surface to the user (HTTP 400) — the same
/// messages `cover_service.rs` used to construct directly before this
/// validation moved into the gate as the single validation site (AR-11).
#[derive(Debug)]
pub enum UserCoverError {
    Validation(String),
}

/// Offer one user-chosen cover (a `select_cover` pick or an `upload_cover`
/// byte payload) to the same crash-safe protocol [`run_cover_write_gate`]
/// uses — same slot lock, same tmp+meta commit/rename/cleanup steps — but
/// without the enrichment-only guards: a user's choice is absolute and must
/// be able to replace even their own earlier manual pick. Only a `Bytes`
/// payload can fail outright (validation); a resolved `Url` behaves like any
/// other candidate fetch and reports failure via `GateOutcome::DownloadFailed`.
pub async fn run_user_cover_write<D, H>(
    db: &D,
    http: &H,
    user_id: UserId,
    input: UserCoverInput,
) -> Result<GateOutcome, UserCoverError>
where
    D: WorkDb + Sync,
    H: HttpFetcher,
{
    let UserCoverInput {
        covers_dir,
        work_id,
        media_type,
        payload,
    } = input;

    let _slot_guard = lock_slot(user_id, work_id, media_type).await;

    let work = match read_work_row(db, user_id, work_id).await {
        Ok(w) => w,
        Err(outcome) => return Ok(outcome),
    };
    let sibling_cover_url = work.audiobook_cover_url.clone();

    let suffix = media_type.suffix();
    let final_path = final_cover_path(&covers_dir, work_id, suffix);

    if tokio::fs::create_dir_all(&covers_dir).await.is_err() {
        return Ok(GateOutcome::DownloadFailed);
    }

    let (bytes, width, height, meta_url, source) = match payload {
        UserCoverPayload::Url { url, source } => {
            match fetch_candidate_bytes(http, work_id, &url).await {
                Ok((bytes, w, h)) => (bytes, w, h, Some(url), source),
                Err(outcome) => return Ok(outcome),
            }
        }
        UserCoverPayload::Bytes { data } => {
            let (jpeg_bytes, w, h) = validate_and_reencode_upload(data)
                .await
                .map_err(UserCoverError::Validation)?;
            (jpeg_bytes, w, h, None, "user_upload".to_string())
        }
    };

    let tmp_path = candidate_tmp_path(&covers_dir, work_id, suffix);
    let meta_path = candidate_meta_path(&covers_dir, work_id, suffix);
    let meta = CandidateMeta {
        url: meta_url,
        source,
        manual: true,
        width,
        height,
    };

    if write_candidate_files(&tmp_path, &meta_path, &bytes, &meta)
        .await
        .is_err()
    {
        discard_candidate_files(&meta_path, &tmp_path).await;
        return Ok(GateOutcome::DownloadFailed);
    }

    Ok(commit_and_finalize(
        db,
        user_id,
        work_id,
        media_type,
        &covers_dir,
        &tmp_path,
        &meta_path,
        &final_path,
        sibling_cover_url.as_deref(),
        meta,
        bytes,
    )
    .await)
}

/// Read the work row inside the slot lock. Both entry points treat an
/// unreadable row identically: there's nothing to compare against or
/// update, so the safest response is a no-op.
async fn read_work_row<D: WorkDb + Sync>(
    db: &D,
    user_id: UserId,
    work_id: WorkId,
) -> Result<Work, GateOutcome> {
    db.get_work(user_id, work_id).await.map_err(|e| {
        tracing::warn!(work_id, error = %e, "cover write gate: work row unreadable");
        GateOutcome::NoOp
    })
}

/// Fetch and decode one candidate's bytes via the SSRF-safe fetcher — the
/// acquisition step shared by both entry points for URL-sourced candidates
/// (a provider/enrichment pick and a user's `select_cover`). Dimensions
/// default to `(0, 0)` when the format can't be measured, matching
/// `fetch_and_decode_cover`'s non-fatal passthrough policy.
async fn fetch_candidate_bytes<H: HttpFetcher>(
    http: &H,
    work_id: WorkId,
    url: &str,
) -> Result<(Vec<u8>, i32, i32), GateOutcome> {
    match livrarr_materialize::fetch_and_decode_cover(http, url, RequestPriority::Normal, false)
        .await
    {
        Ok((bytes, dims)) => {
            let (w, h) = dims.unwrap_or((0, 0));
            Ok((bytes, w, h))
        }
        Err(e) => {
            tracing::debug!(work_id, error = %e, "cover write gate: candidate download failed");
            Err(GateOutcome::DownloadFailed)
        }
    }
}

/// Upload validation moved verbatim from `cover_service.rs`'s old
/// `upload_cover` (AR-11 — the gate is now the single validation site): a
/// 5MB size cap, a magic-byte sniff (JPEG/PNG/WebP), a decode, a maximum
/// dimension cap (8000x8000), and an always-re-encode-to-JPEG step. Runs in
/// a blocking thread — decode/encode are CPU-bound.
async fn validate_and_reencode_upload(data: Vec<u8>) -> Result<(Vec<u8>, i32, i32), String> {
    const MAX_UPLOAD_SIZE: usize = 5 * 1024 * 1024;

    if data.len() > MAX_UPLOAD_SIZE {
        return Err(format!(
            "file too large: {} bytes (max {})",
            data.len(),
            MAX_UPLOAD_SIZE
        ));
    }

    if !(data.starts_with(&[0xFF, 0xD8]) // JPEG
        || data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) // PNG
        || (data.len() >= 12 && &data[8..12] == b"WEBP"))
    // WebP
    {
        return Err("unsupported format: must be JPEG, PNG, or WebP".into());
    }

    tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, i32, i32), String> {
        let img =
            image::load_from_memory(&data).map_err(|e| format!("image decode failed: {e}"))?;

        if img.width() > 8000 || img.height() > 8000 {
            return Err(format!(
                "image too large: {}x{} (max 8000x8000)",
                img.width(),
                img.height()
            ));
        }
        let (width, height) = (img.width() as i32, img.height() as i32);

        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg)
            .map_err(|e| format!("JPEG encode failed: {e}"))?;
        Ok((buf.into_inner(), width, height))
    })
    .await
    .map_err(|e| format!("spawn error: {e}"))?
}

async fn write_candidate_files(
    tmp_path: &Path,
    meta_path: &Path,
    bytes: &[u8],
    meta: &CandidateMeta,
) -> std::io::Result<()> {
    let meta_json = serde_json::to_vec(meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let tmp_owned = tmp_path.to_path_buf();
    let meta_owned = meta_path.to_path_buf();
    let bytes_owned = bytes.to_vec();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_owned)?;
        f.write_all(&bytes_owned)?;
        f.sync_all()?;
        drop(f);
        let mut mf = std::fs::File::create(&meta_owned)?;
        mf.write_all(&meta_json)?;
        mf.sync_all()?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Discard an undecided/rejected candidate's on-disk state. Meta first, then
/// tmp: recovery treats "tmp gone, meta present" as proof the ACCEPT path's
/// atomic rename already ran (row was already committed to match meta) — so
/// this two-step cleanup must never pass through that same observable state,
/// or a crash between the two deletes would make recovery wrongly "heal" the
/// row to a candidate that was actually rejected/never decided. Deleting
/// meta first means the only interleaved state a crash can leave is "meta
/// gone, tmp still present" — invisible to recovery (which scans for meta
/// files), so the orphaned tmp is inert: never rewritten to, never renamed
/// to the served path, and physically distinct from the final `.jpg` name.
async fn discard_candidate_files(meta_path: &Path, tmp_path: &Path) {
    let _ = tokio::fs::remove_file(meta_path).await;
    let _ = tokio::fs::remove_file(tmp_path).await;
}

/// Commit an accepted candidate: DB write (the commit point), rename tmp ->
/// final, delete the meta sidecar, invalidate thumbnails. Shared by both
/// entry points — the protocol from here on is identical whether the
/// candidate came from enrichment or a user's own pick.
#[allow(clippy::too_many_arguments)]
async fn commit_and_finalize<D: WorkDb + Sync>(
    db: &D,
    user_id: UserId,
    work_id: WorkId,
    media_type: CoverMediaType,
    covers_dir: &Path,
    tmp_path: &Path,
    meta_path: &Path,
    final_path: &Path,
    sibling_cover_url: Option<&str>,
    meta: CandidateMeta,
    bytes: Vec<u8>,
) -> GateOutcome {
    let db_result = match media_type {
        CoverMediaType::Ebook => {
            db.update_cover_metadata(
                user_id,
                work_id,
                meta.url.as_deref(),
                &meta.source,
                meta.manual,
                meta.width,
                meta.height,
            )
            .await
        }
        CoverMediaType::Audiobook => {
            db.update_audiobook_cover_metadata(
                user_id,
                work_id,
                meta.url.as_deref(),
                &meta.source,
                meta.manual,
                meta.width,
                meta.height,
            )
            .await
        }
    };
    if let Err(e) = db_result {
        // Leave tmp+meta in place — the next startup recovery pass discards
        // them (row != meta, since the commit never happened).
        tracing::warn!(work_id, error = %e, "cover write gate: DB commit failed");
        return GateOutcome::DownloadFailed;
    }

    if let Err(e) = tokio::fs::rename(tmp_path, final_path).await {
        // DB already committed (the commit point). Recovery completes the
        // rename from meta+tmp at next startup/first-read.
        tracing::warn!(work_id, error = %e, "cover write gate: rename after commit failed");
        return GateOutcome::Accepted {
            bytes,
            width: meta.width,
            height: meta.height,
        };
    }
    let _ = tokio::fs::remove_file(meta_path).await;

    invalidate_thumbnails(
        covers_dir,
        work_id,
        media_type.suffix(),
        media_type,
        sibling_cover_url,
    )
    .await;

    GateOutcome::Accepted {
        bytes,
        width: meta.width,
        height: meta.height,
    }
}

/// Remove the slot's thumbnail after its cover file changed, plus the
/// audiobook fallback thumb when an ebook change is also what the audiobook
/// route renders (no dedicated audiobook cover). Shared with crash recovery,
/// which replays this step for renames the crashed writer never got to
/// finish bookkeeping for.
pub(crate) async fn invalidate_thumbnails(
    covers_dir: &Path,
    work_id: WorkId,
    suffix: &str,
    media_type: CoverMediaType,
    sibling_cover_url: Option<&str>,
) {
    let thumb_path = covers_dir.join(format!("{work_id}{suffix}_thumb.jpg"));
    let _ = tokio::fs::remove_file(&thumb_path).await;

    if media_type == CoverMediaType::Ebook && sibling_cover_url.is_none() {
        let audio_thumb = final_cover_path(covers_dir, work_id, "_audio_thumb");
        let _ = tokio::fs::remove_file(&audio_thumb).await;
    }
}

#[cfg(test)]
mod manual_selection_blocks_candidate_tests {
    use super::*;

    #[test]
    fn user_lock_with_missing_file_does_not_block() {
        assert!(!manual_selection_blocks_candidate(true, false));
    }

    #[test]
    fn user_lock_with_existing_file_blocks() {
        assert!(manual_selection_blocks_candidate(true, true));
    }

    #[test]
    fn automatic_cover_never_blocks_by_itself() {
        assert!(!manual_selection_blocks_candidate(false, false));
        assert!(!manual_selection_blocks_candidate(false, true));
    }
}

#[cfg(test)]
mod slot_locks_sweeper_tests {
    use super::*;

    /// D3 #8 / R-5: `sweep()` existed with zero production callers. This
    /// proves `spawn_slot_locks_sweeper` actually spawns a task (not a no-op)
    /// under a live Tokio runtime, and that the loop is a recurring sweep —
    /// not a one-shot — surviving past the real 300s production interval
    /// (via tokio's mock clock — no real waiting).
    #[tokio::test(start_paused = true)]
    async fn spawns_a_sweeper_that_survives_past_one_full_interval_without_dying() {
        let handle = spawn_slot_locks_sweeper();
        assert!(
            handle.is_some(),
            "must spawn a sweep task under a live Tokio runtime"
        );
        let handle = handle.unwrap();
        assert!(
            !handle.is_finished(),
            "the sweep loop must not exit immediately"
        );

        tokio::time::advance(std::time::Duration::from_secs(301)).await;
        // Let the woken task actually run its tick and re-arm the next one.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert!(
            !handle.is_finished(),
            "the sweep loop must still be alive (looping, not one-shot) \
             after a full interval elapses"
        );
    }

    /// Defensive guard regression test: calling this outside any Tokio
    /// runtime must never panic — it must return `None` instead.
    #[test]
    fn returns_none_outside_a_tokio_runtime() {
        assert!(
            spawn_slot_locks_sweeper().is_none(),
            "no runtime is current here, so no sweep task should be spawned"
        );
    }
}
