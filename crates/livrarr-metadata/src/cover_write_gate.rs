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
//! Commit order (every step durable before the next is visible):
//! 1. write `{id}{suffix}.candidate.tmp` (bytes) + `.candidate.meta.json`
//!    (url/source/trust/dims) before anything else changes.
//! 2. the comparator decides accept/reject.
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
) -> KeyedMutexGuard {
    SLOT_LOCKS.lock((user_id, work_id, media_type)).await
}

/// The durable sidecar recording a pending candidate's intended DB values.
/// Crash recovery reads this to tell a committed-but-unfinished save apart
/// from an undecided or rejected one.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CandidateMeta {
    pub url: String,
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
    /// No candidate, User-locked, or trust disallows replacement — no-op.
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

    let work = match db.get_work(user_id, work_id).await {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(work_id, error = %e, "cover write gate: work row unreadable");
            return GateOutcome::NoOp;
        }
    };
    let current = current_state_for_slot(&work, media_type);
    let foreign = matches!(
        livrarr_external_data::language::provider_priority(work.language.as_deref()),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    let sibling_cover_url = work.audiobook_cover_url.clone();

    if current.trust == CoverTrust::User {
        return GateOutcome::NoOp;
    }
    if !current.trust.allows_replacement_by(resolution.trust) {
        return GateOutcome::NoOp;
    }

    let suffix = media_type.suffix();
    let final_path = final_cover_path(&covers_dir, work_id, suffix);

    if current.url.as_deref() == Some(resolution.url.as_str()) {
        let exists = tokio::fs::try_exists(&final_path).await.unwrap_or(false);
        if exists {
            return GateOutcome::AlreadyCurrent;
        }
    }

    if tokio::fs::create_dir_all(&covers_dir).await.is_err() {
        return GateOutcome::DownloadFailed;
    }

    let (bytes, dims) = match livrarr_materialize::fetch_and_decode_cover(
        http,
        &resolution.url,
        RequestPriority::Normal,
        false,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(work_id, error = %e, "cover write gate: candidate download failed");
            return GateOutcome::DownloadFailed;
        }
    };
    let (new_w, new_h) = dims.unwrap_or((0, 0));

    let tmp_path = candidate_tmp_path(&covers_dir, work_id, suffix);
    let meta_path = candidate_meta_path(&covers_dir, work_id, suffix);

    if write_candidate_files(&tmp_path, &meta_path, &bytes, &resolution, new_w, new_h)
        .await
        .is_err()
    {
        // Meta first, then tmp (see the REJECT cleanup below for why the
        // order matters): recovery's discriminator for "the accept path's
        // rename already ran" is "tmp is gone" — that must never be true
        // while an undecided/rejected candidate's meta still exists.
        let _ = tokio::fs::remove_file(&meta_path).await;
        let _ = tokio::fs::remove_file(&tmp_path).await;
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
        // Meta first, then tmp. Recovery treats "tmp gone, meta present" as
        // proof the ACCEPT path's atomic rename already ran (row was already
        // committed to match meta) — so a reject's two-step cleanup must
        // never pass through that same observable state, or a crash between
        // the two deletes would make recovery wrongly "heal" the row to a
        // candidate that was actually rejected. Deleting meta first means
        // the only interleaved state a crash can leave is "meta gone, tmp
        // still present" — invisible to recovery (which scans for meta
        // files), so the orphaned tmp is inert: never rewritten to, never
        // renamed to the served path, and physically distinct from the
        // final `.jpg` name.
        let _ = tokio::fs::remove_file(&meta_path).await;
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return GateOutcome::Rejected;
    }

    let db_result = match media_type {
        CoverMediaType::Ebook => {
            db.update_cover_metadata(
                user_id,
                work_id,
                Some(&resolution.url),
                &resolution.source,
                resolution.trust,
                new_w,
                new_h,
            )
            .await
        }
        CoverMediaType::Audiobook => {
            db.update_audiobook_cover_metadata(
                user_id,
                work_id,
                Some(&resolution.url),
                &resolution.source,
                resolution.trust,
                new_w,
                new_h,
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

    if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
        // DB already committed (the commit point). Recovery completes the
        // rename from meta+tmp at next startup/first-read.
        tracing::warn!(work_id, error = %e, "cover write gate: rename after commit failed");
        return GateOutcome::Accepted {
            bytes,
            width: new_w,
            height: new_h,
        };
    }
    let _ = tokio::fs::remove_file(&meta_path).await;

    invalidate_thumbnails(
        &covers_dir,
        work_id,
        suffix,
        media_type,
        sibling_cover_url.as_deref(),
    )
    .await;

    GateOutcome::Accepted {
        bytes,
        width: new_w,
        height: new_h,
    }
}

async fn write_candidate_files(
    tmp_path: &Path,
    meta_path: &Path,
    bytes: &[u8],
    resolution: &CoverResolution,
    width: i32,
    height: i32,
) -> std::io::Result<()> {
    let meta = CandidateMeta {
        url: resolution.url.clone(),
        source: resolution.source.clone(),
        trust: resolution.trust,
        width,
        height,
    };
    let meta_json = serde_json::to_vec(&meta)
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
        let audio_thumb = covers_dir.join(format!("{work_id}_audio_thumb.jpg"));
        let _ = tokio::fs::remove_file(&audio_thumb).await;
    }
}
