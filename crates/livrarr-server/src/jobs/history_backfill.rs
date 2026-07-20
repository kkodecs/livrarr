//! One-time history backfill: synthesizes history events for existing
//! libraries from persisted facts only (`works.added_at`, grabs, item import
//! dates, last enrichment), marking every synthesized row `backfilled: true`.
//!
//! Runs as a non-blocking startup pass; completion is recorded in the
//! `_livrarr_meta` `history_backfill_generation` marker, written only after a
//! fully clean pass. Idempotency is non-destructive: a rerun never duplicates
//! rows (per-fact dedup) and never deletes anything.
//!
//! Satisfies: work-history REQ-010, REQ-013.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateHistoryEventDbRequest, GrabDb, HistoryDb, LibraryItemDb, UserDb, WorkDb};
use livrarr_domain::history_events;
use livrarr_domain::{DbError, EventType, GrabStatus, HistoryEvent, HistoryFilter, UserId, WorkId};

/// Grabs are paged at this size while scanning for backfill candidates.
const GRABS_PAGE_SIZE: u32 = 500;

/// The pass yields to the runtime after this many inserts, so it never holds
/// the single SQLite writer connection continuously.
const YIELD_EVERY: u32 = 50;

/// Per-user coverage, preloaded once from existing history rows so the pass
/// never synthesizes a fact that is already recorded.
struct Coverage {
    grabbed_guids: HashSet<String>,
    added_covered: HashSet<WorkId>,
    enriched_covered: HashSet<WorkId>,
    imported_events: Vec<(WorkId, DateTime<Utc>)>,
    failure_covered: HashSet<(WorkId, String)>,
    failure_guids: HashSet<String>,
}

/// Run the backfill to completion. Never returns an error: per-user failures
/// are warned and leave the completion marker unwritten so the next boot
/// retries additively.
pub async fn run_history_backfill(db: SqliteDb) {
    match marker_generation(&db).await {
        Ok(generation) if generation >= 1 => {
            tracing::debug!(generation, "history backfill: already complete");
            return;
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "history backfill: failed to read completion marker");
            return;
        }
    }

    let users = match db.list_users().await {
        Ok(users) => users,
        Err(e) => {
            tracing::warn!(error = %e, "history backfill: failed to list users");
            return;
        }
    };

    tracing::info!(count = users.len(), "history backfill: starting");

    let mut all_users_clean = true;
    let mut inserted: u32 = 0;

    for user in &users {
        if !process_user(&db, user.id, &mut inserted).await {
            all_users_clean = false;
        }
    }

    if all_users_clean {
        if let Err(e) = write_marker(&db).await {
            tracing::warn!(error = %e, "history backfill: failed to write completion marker");
        }
    }

    tracing::info!(inserted, all_users_clean, "history backfill: complete");
}

/// Reads the stored generation, or `0` when the marker has never been
/// written.
async fn marker_generation(db: &SqliteDb) -> Result<i64, sqlx::Error> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key = 'history_backfill_generation'",
    )
    .fetch_optional(db.pool())
    .await?;
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Marks the pass complete. Insert-or-update: unlike `identity_key_generation`
/// (seeded by migration 069), this key is never pre-seeded, so a bare UPDATE
/// would silently no-op on the first clean run.
async fn write_marker(db: &SqliteDb) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) VALUES ('history_backfill_generation', '1') \
         ON CONFLICT(key) DO UPDATE SET value = '1'",
    )
    .execute(db.pool())
    .await?;
    Ok(())
}

/// Runs every pass for one user. Returns `false` when a list query or an
/// insert failed for this user — the caller then skips the completion
/// marker. A list-query failure abandons the rest of this user's passes; an
/// insert failure is warned and the remaining facts are still attempted.
async fn process_user(db: &SqliteDb, user_id: UserId, inserted: &mut u32) -> bool {
    let coverage = match preload_coverage(db, user_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(user_id, error = %e, "history backfill: coverage preload failed");
            return false;
        }
    };

    let mut clean = true;

    let works = match db.list_works(user_id).await {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(user_id, error = %e, "history backfill: failed to list works");
            return false;
        }
    };

    let titles: HashMap<WorkId, (String, String)> = works
        .iter()
        .map(|w| (w.id, (w.title.clone(), w.author_name.clone())))
        .collect();
    // Never index the map directly — a work deleted mid-pass must degrade to
    // an empty title, not panic.
    let title_of = |work_id: WorkId| titles.get(&work_id).map(|(t, _)| t.as_str()).unwrap_or("");

    for work in &works {
        if !coverage.added_covered.contains(&work.id) {
            insert_fact(
                db,
                user_id,
                history_events::added_backfilled(
                    work.id,
                    &work.title,
                    Some(&work.author_name),
                    work.added_at,
                ),
                &mut clean,
                inserted,
            )
            .await;
        }
        if let Some(enriched_at) = work.enriched_at {
            if !coverage.enriched_covered.contains(&work.id) {
                insert_fact(
                    db,
                    user_id,
                    history_events::enriched_backfilled(
                        work.id,
                        &work.title,
                        Some(&work.author_name),
                        work.enrichment_source.clone(),
                        enriched_at,
                    ),
                    &mut clean,
                    inserted,
                )
                .await;
            }
        }
    }

    let mut page: u32 = 1;
    loop {
        let (grabs, total) = match db
            .list_grabs_paginated(user_id, page, GRABS_PAGE_SIZE)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(user_id, page, error = %e, "history backfill: failed to list grabs");
                return false;
            }
        };
        let fetched = grabs.len() as u32;

        for grab in grabs {
            // Sent/Confirmed/Importing/Removed are in-flight or discarded —
            // nothing is synthesized for them, not even the grab itself.
            if !matches!(
                grab.status,
                GrabStatus::Imported | GrabStatus::Failed | GrabStatus::ImportFailed
            ) {
                continue;
            }

            let work_title = title_of(grab.work_id);

            if !coverage.grabbed_guids.contains(&grab.guid) {
                insert_fact(
                    db,
                    user_id,
                    history_events::grabbed_backfilled(
                        grab.work_id,
                        work_title,
                        &grab.title,
                        &grab.indexer,
                        &grab.guid,
                        grab.grabbed_at,
                    ),
                    &mut clean,
                    inserted,
                )
                .await;
            }

            let already_failed = coverage.failure_guids.contains(&grab.guid)
                || coverage
                    .failure_covered
                    .contains(&(grab.work_id, grab.title.clone()));

            if grab.status == GrabStatus::Failed && !already_failed {
                insert_fact(
                    db,
                    user_id,
                    history_events::download_failed_backfilled(
                        grab.work_id,
                        work_title,
                        &grab.title,
                        &grab.guid,
                        grab.import_error.as_deref(),
                        grab.grabbed_at,
                    ),
                    &mut clean,
                    inserted,
                )
                .await;
            } else if grab.status == GrabStatus::ImportFailed && !already_failed {
                insert_fact(
                    db,
                    user_id,
                    history_events::import_failed_backfilled(
                        grab.work_id,
                        work_title,
                        &grab.title,
                        &grab.guid,
                        grab.import_error.as_deref(),
                        grab.grabbed_at,
                    ),
                    &mut clean,
                    inserted,
                )
                .await;
            }
        }

        if fetched < GRABS_PAGE_SIZE || i64::from(page) * i64::from(GRABS_PAGE_SIZE) >= total {
            break;
        }
        page += 1;
    }

    let items = match db.list_library_items(user_id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(user_id, error = %e, "history backfill: failed to list library items");
            return false;
        }
    };

    for item in items {
        let covered = coverage.imported_events.iter().any(|(work_id, date)| {
            *work_id == item.work_id
                && *date >= item.imported_at - Duration::seconds(5)
                && *date <= item.imported_at + Duration::hours(1)
        });
        if !covered {
            insert_fact(
                db,
                user_id,
                history_events::imported_backfilled(
                    item.work_id,
                    title_of(item.work_id),
                    &item.path,
                    item.media_type.as_str(),
                    item.imported_at,
                ),
                &mut clean,
                inserted,
            )
            .await;
        }
    }

    clean
}

/// One `list_history` call per event kind, folded into the sets and vector
/// the per-user passes dedup against. `ImportFailed` rows feed both
/// `imported_events` (coverage window) and `failure_covered` (failure dedup).
async fn preload_coverage(db: &SqliteDb, user_id: UserId) -> Result<Coverage, DbError> {
    let grabbed_rows = list_by_kind(db, user_id, EventType::Grabbed).await?;
    let added_rows = list_by_kind(db, user_id, EventType::Added).await?;
    let enriched_rows = list_by_kind(db, user_id, EventType::Enriched).await?;
    let imported_rows = list_by_kind(db, user_id, EventType::Imported).await?;
    let import_failed_rows = list_by_kind(db, user_id, EventType::ImportFailed).await?;
    let download_failed_rows = list_by_kind(db, user_id, EventType::DownloadFailed).await?;

    let grabbed_guids: HashSet<String> = grabbed_rows
        .iter()
        .filter_map(|event| event.data.get("guid").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    let added_covered: HashSet<WorkId> = added_rows
        .iter()
        .filter_map(|event| event.work_id)
        .collect();
    let enriched_covered: HashSet<WorkId> = enriched_rows
        .iter()
        .filter_map(|event| event.work_id)
        .collect();

    let imported_events: Vec<(WorkId, DateTime<Utc>)> = imported_rows
        .iter()
        .chain(import_failed_rows.iter())
        .filter_map(|event| event.work_id.map(|work_id| (work_id, event.date)))
        .collect();

    // Failure coverage is two-tier: a row carrying a `guid` (backfill-written)
    // covers exactly its own grab; a guid-less row (live writers persist no
    // guid on failure events) covers by (work, release title) — fuzzier, and
    // deliberately erring toward not synthesizing. Guid rows stay OUT of the
    // title tier so one indexer's failure never masks the same-titled release
    // grabbed from another indexer across a crash-rerun.
    let mut failure_guids: HashSet<String> = HashSet::new();
    let mut failure_covered: HashSet<(WorkId, String)> = HashSet::new();
    for event in download_failed_rows.iter().chain(import_failed_rows.iter()) {
        if let Some(guid) = event.data.get("guid").and_then(Value::as_str) {
            failure_guids.insert(guid.to_string());
            continue;
        }
        if let Some(work_id) = event.work_id {
            let title = event
                .data
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            failure_covered.insert((work_id, title.to_string()));
        }
    }

    Ok(Coverage {
        grabbed_guids,
        added_covered,
        enriched_covered,
        imported_events,
        failure_covered,
        failure_guids,
    })
}

async fn list_by_kind(
    db: &SqliteDb,
    user_id: UserId,
    event_type: EventType,
) -> Result<Vec<HistoryEvent>, DbError> {
    db.list_history(
        user_id,
        HistoryFilter {
            event_type: Some(event_type),
            work_id: None,
            start_date: None,
            end_date: None,
        },
    )
    .await
}

/// The backfill's local, fallible insert chokepoint. Deliberately not
/// `record_history`: that helper swallows errors for live writers protecting
/// a host operation, but a swallowed failure here would let the completion
/// marker land over a fact that was never actually written.
async fn insert_fact(
    db: &SqliteDb,
    user_id: UserId,
    draft: history_events::HistoryDraft,
    clean: &mut bool,
    inserted: &mut u32,
) {
    let event_type = draft.event_type;
    let work_id = draft.work_id;
    let req = CreateHistoryEventDbRequest {
        user_id,
        work_id,
        event_type,
        data: draft.data,
        date: draft.date,
    };
    if let Err(e) = db.create_history_event(req).await {
        tracing::warn!(user_id, ?event_type, ?work_id, error = %e, "history backfill: insert failed");
        *clean = false;
    }

    *inserted += 1;
    if inserted.is_multiple_of(YIELD_EVERY) {
        tokio::task::yield_now().await;
    }
}
