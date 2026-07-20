//! Typed history-event constructors — the single sanctioned payload builder
//! for every history writer.
//!
//! Each written event shape has exactly one constructor here. Work-attached
//! constructors require the work-title snapshot (`work_title`, plus
//! `work_author` where sensible) so rows stay identifiable after their work
//! row is deleted (`history.work_id` is `ON DELETE SET NULL`). Grab-family
//! constructors keep the existing `title` key meaning the RELEASE title —
//! never the work title. Backfill variants stamp `backfilled: true` and carry
//! the caller-supplied historical fact date.
//!
//! Satisfies: work-history REQ-013 (payload identity), REQ-010 (backfill).

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::{EnrichmentStatus, EventType, WorkId};

/// A fully-formed history event awaiting insertion. Writers hand this to the
/// write chokepoint: the DB layer's `record_history` helper, or
/// [`crate::services::HistoryService::record`] from behind the compile wall.
#[derive(Debug, Clone)]
pub struct HistoryDraft {
    /// `None` for events that end unattached by design (`workDeleted`).
    pub work_id: Option<WorkId>,
    pub event_type: EventType,
    pub data: Value,
    /// `None` = write-time now (live writers); `Some` = the historical fact
    /// date (backfill variants).
    pub date: Option<DateTime<Utc>>,
}

/// The creation door that built a work candidate (the `added` event's source
/// label). Stamped exclusively by the per-door `seed_*` constructors in
/// [`crate::seed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkAddSource {
    Search,
    ListImport,
    Readarr,
    AuthorMonitor,
    SeriesMonitor,
    FileImport,
}

impl WorkAddSource {
    /// The payload string form of the door label.
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkAddSource::Search => "search",
            WorkAddSource::ListImport => "list-import",
            WorkAddSource::Readarr => "readarr",
            WorkAddSource::AuthorMonitor => "author-monitor",
            WorkAddSource::SeriesMonitor => "series-monitor",
            WorkAddSource::FileImport => "file-import",
        }
    }
}

/// Inserts `key` only when a value is present — optional payload keys are
/// omitted, never written as JSON null.
fn set_opt(data: &mut Value, key: &str, value: Option<&str>) {
    if let Some(v) = value {
        data[key] = json!(v);
    }
}

/// `added` — keys: `work_title`, `work_author`?, `source`.
pub fn added(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    source: WorkAddSource,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "source": source.as_str(),
    });
    set_opt(&mut data, "work_author", work_author);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Added,
        data,
        date: None,
    }
}

/// Backfilled `added` — keys: `work_title`, `work_author`?, `backfilled: true`.
/// The door is not reconstructible for old works, so `source` is omitted.
pub fn added_backfilled(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    date: DateTime<Utc>,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "backfilled": true,
    });
    set_opt(&mut data, "work_author", work_author);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Added,
        data,
        date: Some(date),
    }
}

/// `enriched` — keys: `work_title`, `work_author`?, `changed`, `status`,
/// `tags_written`.
pub fn enriched(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    changed: bool,
    status: &EnrichmentStatus,
    tags_written: bool,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "changed": changed,
        "status": status,
        "tags_written": tags_written,
    });
    set_opt(&mut data, "work_author", work_author);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Enriched,
        data,
        date: None,
    }
}

/// Backfilled `enriched` — keys: `work_title`, `work_author`?, `source`?,
/// `backfilled: true`.
pub fn enriched_backfilled(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    source: Option<String>,
    date: DateTime<Utc>,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "backfilled": true,
    });
    set_opt(&mut data, "work_author", work_author);
    set_opt(&mut data, "source", source.as_deref());
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Enriched,
        data,
        date: Some(date),
    }
}

/// `enrichmentFailed` — keys: `work_title`, `work_author`?, `reason`.
pub fn enrichment_failed(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    reason: &str,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "reason": reason,
    });
    set_opt(&mut data, "work_author", work_author);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::EnrichmentFailed,
        data,
        date: None,
    }
}

/// Manual/adopt per-file `imported` — keys: `work_title`, `path`, `media_type`.
/// A successful import always has a classified media type.
pub fn imported_manual(
    work_id: WorkId,
    work_title: &str,
    path: &str,
    media_type: &str,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Imported,
        data: json!({
            "work_title": work_title,
            "path": path,
            "media_type": media_type,
        }),
        date: None,
    }
}

/// Work-attached manual `importFailed` — keys: `work_title`, `path`,
/// `media_type`?, `error`.
pub fn import_failed_manual(
    work_id: WorkId,
    work_title: &str,
    path: &str,
    media_type: Option<&str>,
    error: &str,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "path": path,
        "error": error,
    });
    set_opt(&mut data, "media_type", media_type);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::ImportFailed,
        data,
        date: None,
    }
}

/// Unattached manual `importFailed` (`work_id` NULL) — keys: `path`,
/// `media_type`?, `error`. The early manual-import failures (unrecognized
/// media, missing root folder, work-creation failure) have no work to
/// snapshot; the row is identified by its path.
pub fn import_failed_unattached(path: &str, media_type: Option<&str>, error: &str) -> HistoryDraft {
    let mut data = json!({
        "path": path,
        "error": error,
    });
    set_opt(&mut data, "media_type", media_type);
    HistoryDraft {
        work_id: None,
        event_type: EventType::ImportFailed,
        data,
        date: None,
    }
}

/// Grab-road batch `imported`/`importFailed` — keys: `title` (RELEASE title,
/// meaning preserved), `imported`, `failed`, `skipped`, `work_title`,
/// `work_author`?. `success` is the caller's existing
/// `final_status == GrabStatus::Imported` ternary: `true` → `Imported`,
/// `false` → `ImportFailed`.
#[allow(clippy::too_many_arguments)] // 16 fixed shapes, one flat fn per shape (IR v2 constructors-not-builder)
pub fn imported_batch(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    release_title: &str,
    success: bool,
    imported: usize,
    failed: usize,
    skipped: usize,
) -> HistoryDraft {
    let mut data = json!({
        "title": release_title,
        "imported": imported,
        "failed": failed,
        "skipped": skipped,
        "work_title": work_title,
    });
    set_opt(&mut data, "work_author", work_author);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: if success {
            EventType::Imported
        } else {
            EventType::ImportFailed
        },
        data,
        date: None,
    }
}

/// Backfilled per-file `imported` — keys: `work_title`, `path`, `media_type`,
/// `backfilled: true`.
pub fn imported_backfilled(
    work_id: WorkId,
    work_title: &str,
    path: &str,
    media_type: &str,
    date: DateTime<Utc>,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Imported,
        data: json!({
            "work_title": work_title,
            "path": path,
            "media_type": media_type,
            "backfilled": true,
        }),
        date: Some(date),
    }
}

/// Backfilled `importFailed` (from `GrabStatus::ImportFailed`) — keys: `title`
/// (release), `guid`, `error`?, `work_title`, `backfilled: true`. `guid` is the
/// grab's persisted release id: the backfill's per-fact failure dedup reads it,
/// because a release title alone cannot distinguish the same-titled release
/// grabbed from two indexers.
pub fn import_failed_backfilled(
    work_id: WorkId,
    work_title: &str,
    release_title: &str,
    guid: &str,
    import_error: Option<&str>,
    date: DateTime<Utc>,
) -> HistoryDraft {
    let mut data = json!({
        "title": release_title,
        "guid": guid,
        "work_title": work_title,
        "backfilled": true,
    });
    set_opt(&mut data, "error", import_error);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::ImportFailed,
        data,
        date: Some(date),
    }
}

/// Backfilled `downloadFailed` (from `GrabStatus::Failed`) — keys: `title`
/// (release), `guid`, `error`?, `work_title`, `backfilled: true`. `guid` as in
/// [`import_failed_backfilled`]: the per-fact failure dedup key.
pub fn download_failed_backfilled(
    work_id: WorkId,
    work_title: &str,
    release_title: &str,
    guid: &str,
    import_error: Option<&str>,
    date: DateTime<Utc>,
) -> HistoryDraft {
    let mut data = json!({
        "title": release_title,
        "guid": guid,
        "work_title": work_title,
        "backfilled": true,
    });
    set_opt(&mut data, "error", import_error);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::DownloadFailed,
        data,
        date: Some(date),
    }
}

/// Per-work-per-pass `tagWritten` — keys: `work_title`, `attempted`,
/// `succeeded`.
pub fn tag_written(
    work_id: WorkId,
    work_title: &str,
    attempted: usize,
    succeeded: usize,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::TagWritten,
        data: json!({
            "work_title": work_title,
            "attempted": attempted,
            "succeeded": succeeded,
        }),
        date: None,
    }
}

/// `tagWriteFailed` (zero files succeeded) — keys: `work_title`, `attempted`,
/// `error` (first error).
pub fn tag_write_failed(
    work_id: WorkId,
    work_title: &str,
    attempted: usize,
    first_error: &str,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::TagWriteFailed,
        data: json!({
            "work_title": work_title,
            "attempted": attempted,
            "error": first_error,
        }),
        date: None,
    }
}

/// Convergence-recovery single-file `tagWritten` — keys: `work_title`, `path`.
pub fn tag_written_item(work_id: WorkId, work_title: &str, path: &str) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::TagWritten,
        data: json!({
            "work_title": work_title,
            "path": path,
        }),
        date: None,
    }
}

/// `fileDeleted` — keys: `work_title`, `path`, `media_type`, `undo` (key
/// present only when `true`, on the Readarr-undo road).
pub fn file_deleted(
    work_id: WorkId,
    work_title: &str,
    path: &str,
    media_type: &str,
    undo: bool,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "path": path,
        "media_type": media_type,
    });
    if undo {
        data["undo"] = json!(true);
    }
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::FileDeleted,
        data,
        date: None,
    }
}

/// `workDeleted` (composite; `work_id` NULL — the work row is already gone) —
/// keys: `work_title`, `work_author`?, `files_removed`, `undo`?.
pub fn work_deleted(
    work_title: &str,
    work_author: Option<&str>,
    files_removed: usize,
    undo: bool,
) -> HistoryDraft {
    let mut data = json!({
        "work_title": work_title,
        "files_removed": files_removed,
    });
    set_opt(&mut data, "work_author", work_author);
    if undo {
        data["undo"] = json!(true);
    }
    HistoryDraft {
        work_id: None,
        event_type: EventType::WorkDeleted,
        data,
        date: None,
    }
}

/// `worksMerged` on the survivor — keys: `work_title` (survivor snapshot),
/// `merged_title`, `merged_work_id`.
pub fn works_merged(
    survivor_work_id: WorkId,
    survivor_title: &str,
    merged_title: &str,
    merged_work_id: WorkId,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(survivor_work_id),
        event_type: EventType::WorksMerged,
        data: json!({
            "work_title": survivor_title,
            "merged_title": merged_title,
            "merged_work_id": merged_work_id,
        }),
        date: None,
    }
}

/// `grabbed` — keys: `title` (RELEASE), `indexer`, `guid`,
/// `download_client_id` (all four preserved verbatim — the backfill's per-fact
/// grab dedup reads `guid`), plus `work_title`, `work_author`?.
pub fn grabbed(
    work_id: WorkId,
    work_title: &str,
    work_author: Option<&str>,
    release_title: &str,
    indexer: &str,
    guid: &str,
    download_client_id: i64,
) -> HistoryDraft {
    let mut data = json!({
        "title": release_title,
        "indexer": indexer,
        "guid": guid,
        "download_client_id": download_client_id,
        "work_title": work_title,
    });
    set_opt(&mut data, "work_author", work_author);
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Grabbed,
        data,
        date: None,
    }
}

/// Backfilled `grabbed` — keys: `title` (release), `indexer`, `guid`,
/// `work_title`, `backfilled: true`.
pub fn grabbed_backfilled(
    work_id: WorkId,
    work_title: &str,
    release_title: &str,
    indexer: &str,
    guid: &str,
    date: DateTime<Utc>,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::Grabbed,
        data: json!({
            "title": release_title,
            "indexer": indexer,
            "guid": guid,
            "work_title": work_title,
            "backfilled": true,
        }),
        date: Some(date),
    }
}

/// `downloadFailed` — keys: `title` (release, preserved), `error`,
/// `work_title`.
pub fn download_failed(
    work_id: WorkId,
    work_title: &str,
    release_title: &str,
    error: &str,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::DownloadFailed,
        data: json!({
            "title": release_title,
            "error": error,
            "work_title": work_title,
        }),
        date: None,
    }
}

/// `identityResolved` — keys: `work_title`, `action`, `identity` (the chosen
/// identity in human-readable form, e.g. "Title — Author (gr_key 123)").
pub fn identity_resolved(
    work_id: WorkId,
    work_title: &str,
    action: &str,
    identity: String,
) -> HistoryDraft {
    HistoryDraft {
        work_id: Some(work_id),
        event_type: EventType::IdentityResolved,
        data: json!({
            "work_title": work_title,
            "action": action,
            "identity": identity,
        }),
        date: None,
    }
}
