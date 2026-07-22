//! The consolidated cover save gate (S2). Every road that saves a non-User
//! cover — add-time enrichment, refresh, background retry, all funneling
//! through `work_service::run_unified_enrichment` — routes through
//! [`run_cover_write_gate`]. It wires the previously-dormant size/trust
//! comparator into the single chokepoint with a crash-safe commit protocol:
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
//!    (url/source/trust/dims) before anything else changes.
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
use livrarr_domain::{
    CoverMediaType, CoverResolution, CoverTrust, RequestPriority, UserId, Work, WorkId,
};
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
    pub trust: CoverTrust,
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
    /// No candidate, a User lock backed by an existing file, or trust
    /// disallows replacement — no-op.
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
    trust: CoverTrust,
    width: i32,
    height: i32,
    source: Option<String>,
    url: Option<String>,
}

fn current_state_for_slot(work: &Work, media_type: CoverMediaType) -> CurrentCoverState {
    match media_type {
        CoverMediaType::Ebook => CurrentCoverState {
            trust: work.cover_trust,
            width: work.cover_width,
            height: work.cover_height,
            source: work.cover_source.clone(),
            url: work.cover_url.clone(),
        },
        CoverMediaType::Audiobook => CurrentCoverState {
            trust: work.audiobook_cover_trust,
            width: work.audiobook_cover_width,
            height: work.audiobook_cover_height,
            source: work.audiobook_cover_source.clone(),
            url: work.audiobook_cover_url.clone(),
        },
    }
}

/// Whether the incumbent's trust should block the candidate outright, before
/// any download is attempted. A User lock is absolute only while its file
/// still exists on disk (`locked_file_exists`) — a User-trust row with no
/// file is a damaged slot (e.g. a failed phase-1 add-time download, see
/// `addtime_cover_trust` in work_service.rs) and must not permanently refuse
/// every future candidate. Any other trust falls back to the usual
/// replacement rule. Pure and file-I/O-free so it's unit testable directly.
fn trust_blocks_candidate(
    current_trust: CoverTrust,
    incoming_trust: CoverTrust,
    locked_file_exists: bool,
) -> bool {
    if current_trust == CoverTrust::User {
        locked_file_exists
    } else {
        !current_trust.allows_replacement_by(incoming_trust)
    }
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
    let current = current_state_for_slot(&work, media_type);
    let foreign = matches!(
        livrarr_external_data::language::provider_priority(work.language.as_deref()),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    let sibling_cover_url = work.audiobook_cover_url.clone();

    let suffix = media_type.suffix();
    let final_path = final_cover_path(&covers_dir, work_id, suffix);

    // A User lock is honored only when its cover actually exists on disk —
    // either the final .jpg, or the crash-safe protocol's committed-but-
    // unrenamed state (candidate meta sidecar present: the DB row is already
    // User and startup recovery owns finishing the rename). A failed phase-1
    // add-time download stamps User trust with NOTHING on disk (0x0 dims,
    // cover_url still set — see `addtime_cover_trust` in work_service.rs);
    // only that fully-empty shape is a damaged, replaceable slot. Only pay
    // for the existence checks when the row is actually User-locked.
    let locked_file_exists = if current.trust == CoverTrust::User {
        tokio::fs::try_exists(&final_path).await.unwrap_or(false)
            || tokio::fs::try_exists(&candidate_meta_path(&covers_dir, work_id, suffix))
                .await
                .unwrap_or(false)
    } else {
        false
    };
    if trust_blocks_candidate(current.trust, resolution.trust, locked_file_exists) {
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
        trust: resolution.trust,
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

    let accept = if resolution.trust != current.trust {
        true // higher trust always wins (already gated by allows_replacement_by above)
    } else {
        let rank_model = CoverRankModel::for_media(media_type, foreign);
        should_upgrade_same_tier(
            current.width.max(0) as u32,
            current.height.max(0) as u32,
            new_w as u32,
            new_h as u32,
            current.source.as_deref(),
            &resolution.source,
            rank_model,
        )
    };

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
/// be able to replace even their own earlier User-trust pick, exactly the
/// case `run_cover_write_gate`'s User-NoOp guard blocks. Only a `Bytes`
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
        trust: CoverTrust::User,
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
                meta.trust,
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
                meta.trust,
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
mod trust_blocks_candidate_tests {
    use super::*;

    #[test]
    fn user_lock_with_missing_file_does_not_block() {
        // BUG: a User-trust slot with no file on disk (a failed phase-1
        // add-time download) must not permanently refuse every candidate.
        assert!(!trust_blocks_candidate(
            CoverTrust::User,
            CoverTrust::Validated,
            false,
        ));
    }

    #[test]
    fn user_lock_with_existing_file_blocks() {
        // A real user-chosen cover on disk stays absolute.
        assert!(trust_blocks_candidate(
            CoverTrust::User,
            CoverTrust::Validated,
            true,
        ));
    }

    #[test]
    fn validated_rejects_unvalidated_regardless_of_file_flag() {
        assert!(trust_blocks_candidate(
            CoverTrust::Validated,
            CoverTrust::Unvalidated,
            false,
        ));
        assert!(trust_blocks_candidate(
            CoverTrust::Validated,
            CoverTrust::Unvalidated,
            true,
        ));
    }

    #[test]
    fn unvalidated_never_blocks() {
        assert!(!trust_blocks_candidate(
            CoverTrust::Unvalidated,
            CoverTrust::Unvalidated,
            false,
        ));
        assert!(!trust_blocks_candidate(
            CoverTrust::Unvalidated,
            CoverTrust::User,
            false,
        ));
    }
}
