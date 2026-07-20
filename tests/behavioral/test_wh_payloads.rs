mod common;

use std::panic::{catch_unwind, AssertUnwindSafe};

use chrono::{TimeZone, Utc};
use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::{CreateHistoryEventDbRequest, HistoryDb};
use livrarr_domain::history_events::{self, HistoryDraft, WorkAddSource};
use livrarr_domain::{EnrichmentStatus, EventType, HistoryFilter, WorkId};
use serde_json::{json, Value};

const WORK_ID: WorkId = 4242;
const MERGED_WORK_ID: WorkId = 4243;
const WORK_TITLE: &str = "Work Title Snapshot";
const WORK_AUTHOR: &str = "Work Author Snapshot";
const RELEASE_TITLE: &str = "Release Title Snapshot";

fn no_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

fn draft_from(name: &str, build: impl FnOnce() -> HistoryDraft) -> HistoryDraft {
    catch_unwind(AssertUnwindSafe(build))
        .unwrap_or_else(|_| panic!("{name} constructor should return a HistoryDraft"))
}

fn assert_work_title(name: &str, data: &Value) {
    assert_eq!(
        data.get("work_title"),
        Some(&json!(WORK_TITLE)),
        "{name} must carry the stable work_title snapshot"
    );
}

fn assert_release_title(name: &str, data: &Value) {
    assert_eq!(
        data.get("title"),
        Some(&json!(RELEASE_TITLE)),
        "{name} must keep title as the release title"
    );
    assert_ne!(
        data.get("title"),
        data.get("work_title"),
        "{name} must not alias release title to work_title"
    );
}

fn assert_backfilled(name: &str, draft: &HistoryDraft, fact_date: chrono::DateTime<Utc>) {
    assert_eq!(
        draft.data.get("backfilled"),
        Some(&json!(true)),
        "{name} must mark synthesized rows"
    );
    assert_eq!(
        draft.date,
        Some(fact_date),
        "{name} must carry the fact date"
    );
}

#[test]
fn wh_constructors_put_work_title_on_every_work_attached_payload_except_unattached_import_failure()
{
    let fact_date = Utc.with_ymd_and_hms(2024, 5, 6, 7, 8, 9).unwrap();

    let work_attached: Vec<(&str, HistoryDraft)> = vec![
        (
            "added",
            draft_from("added", || {
                history_events::added(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    WorkAddSource::Search,
                )
            }),
        ),
        (
            "added_backfilled",
            draft_from("added_backfilled", || {
                history_events::added_backfilled(WORK_ID, WORK_TITLE, Some(WORK_AUTHOR), fact_date)
            }),
        ),
        (
            "enriched",
            draft_from("enriched", || {
                history_events::enriched(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    true,
                    &EnrichmentStatus::Enriched,
                    false,
                )
            }),
        ),
        (
            "enriched_backfilled",
            draft_from("enriched_backfilled", || {
                history_events::enriched_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    Some("readarr".to_string()),
                    fact_date,
                )
            }),
        ),
        (
            "enrichment_failed",
            draft_from("enrichment_failed", || {
                history_events::enrichment_failed(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    "provider failed",
                )
            }),
        ),
        (
            "imported_manual",
            draft_from("imported_manual", || {
                history_events::imported_manual(WORK_ID, WORK_TITLE, "/books/work.epub", "ebook")
            }),
        ),
        (
            "import_failed_manual",
            draft_from("import_failed_manual", || {
                history_events::import_failed_manual(
                    WORK_ID,
                    WORK_TITLE,
                    "/books/bad.epub",
                    Some("ebook"),
                    "bad file",
                )
            }),
        ),
        (
            "imported_batch",
            draft_from("imported_batch", || {
                history_events::imported_batch(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    RELEASE_TITLE,
                    true,
                    1,
                    0,
                    0,
                )
            }),
        ),
        (
            "imported_backfilled",
            draft_from("imported_backfilled", || {
                history_events::imported_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    "/books/backfill.epub",
                    "ebook",
                    fact_date,
                )
            }),
        ),
        (
            "import_failed_backfilled",
            draft_from("import_failed_backfilled", || {
                history_events::import_failed_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "guid-backfilled",
                    Some("import failed"),
                    fact_date,
                )
            }),
        ),
        (
            "download_failed_backfilled",
            draft_from("download_failed_backfilled", || {
                history_events::download_failed_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "guid-backfilled",
                    Some("download failed"),
                    fact_date,
                )
            }),
        ),
        (
            "tag_written",
            draft_from("tag_written", || {
                history_events::tag_written(WORK_ID, WORK_TITLE, 2, 2)
            }),
        ),
        (
            "tag_write_failed",
            draft_from("tag_write_failed", || {
                history_events::tag_write_failed(WORK_ID, WORK_TITLE, 2, "first error")
            }),
        ),
        (
            "tag_written_item",
            draft_from("tag_written_item", || {
                history_events::tag_written_item(WORK_ID, WORK_TITLE, "/books/item.epub")
            }),
        ),
        (
            "file_deleted",
            draft_from("file_deleted", || {
                history_events::file_deleted(
                    WORK_ID,
                    WORK_TITLE,
                    "/books/item.epub",
                    "ebook",
                    false,
                )
            }),
        ),
        (
            "work_deleted",
            draft_from("work_deleted", || {
                history_events::work_deleted(WORK_TITLE, Some(WORK_AUTHOR), 3, false)
            }),
        ),
        (
            "works_merged",
            draft_from("works_merged", || {
                history_events::works_merged(WORK_ID, WORK_TITLE, "Merged Work", MERGED_WORK_ID)
            }),
        ),
        (
            "grabbed",
            draft_from("grabbed", || {
                history_events::grabbed(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    RELEASE_TITLE,
                    "Indexer",
                    "guid-1",
                    9,
                )
            }),
        ),
        (
            "grabbed_backfilled",
            draft_from("grabbed_backfilled", || {
                history_events::grabbed_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "Indexer",
                    "guid-2",
                    fact_date,
                )
            }),
        ),
        (
            "download_failed",
            draft_from("download_failed", || {
                history_events::download_failed(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "download failed",
                )
            }),
        ),
        (
            "identity_resolved",
            draft_from("identity_resolved", || {
                history_events::identity_resolved(
                    WORK_ID,
                    WORK_TITLE,
                    "AcceptSeparate",
                    "Work Title Snapshot - Work Author Snapshot (gr_key 1)".to_string(),
                )
            }),
        ),
    ];

    for (name, draft) in work_attached {
        assert_work_title(name, &draft.data);
    }

    let unattached = draft_from("import_failed_unattached", || {
        history_events::import_failed_unattached("/incoming/unknown.bin", None, "unrecognized")
    });
    assert!(
        unattached.data.get("work_title").is_none(),
        "import_failed_unattached is the one payload shape with no work_title"
    );
}

#[test]
fn wh_grab_family_keeps_release_title_and_backfills_stamp_fact_dates() {
    let fact_date = Utc.with_ymd_and_hms(2024, 7, 8, 9, 10, 11).unwrap();

    let grab_family = vec![
        (
            "grabbed",
            draft_from("grabbed", || {
                history_events::grabbed(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    RELEASE_TITLE,
                    "Indexer",
                    "guid-live",
                    5,
                )
            }),
            false,
        ),
        (
            "grabbed_backfilled",
            draft_from("grabbed_backfilled", || {
                history_events::grabbed_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "Indexer",
                    "guid-backfilled",
                    fact_date,
                )
            }),
            true,
        ),
        (
            "download_failed",
            draft_from("download_failed", || {
                history_events::download_failed(WORK_ID, WORK_TITLE, RELEASE_TITLE, "failed")
            }),
            false,
        ),
        (
            "download_failed_backfilled",
            draft_from("download_failed_backfilled", || {
                history_events::download_failed_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "guid-backfilled",
                    Some("failed"),
                    fact_date,
                )
            }),
            true,
        ),
        (
            "import_failed_backfilled",
            draft_from("import_failed_backfilled", || {
                history_events::import_failed_backfilled(
                    WORK_ID,
                    WORK_TITLE,
                    RELEASE_TITLE,
                    "guid-backfilled",
                    Some("failed"),
                    fact_date,
                )
            }),
            true,
        ),
        (
            "imported_batch",
            draft_from("imported_batch", || {
                history_events::imported_batch(
                    WORK_ID,
                    WORK_TITLE,
                    Some(WORK_AUTHOR),
                    RELEASE_TITLE,
                    true,
                    1,
                    0,
                    0,
                )
            }),
            false,
        ),
    ];

    for (name, draft, is_backfill) in grab_family {
        assert_work_title(name, &draft.data);
        assert_release_title(name, &draft.data);
        if is_backfill {
            assert_backfilled(name, &draft, fact_date);
        }
    }

    let added_backfilled = draft_from("added_backfilled", || {
        history_events::added_backfilled(WORK_ID, WORK_TITLE, Some(WORK_AUTHOR), fact_date)
    });
    assert_backfilled("added_backfilled", &added_backfilled, fact_date);

    let enriched_backfilled = draft_from("enriched_backfilled", || {
        history_events::enriched_backfilled(WORK_ID, WORK_TITLE, None, None, fact_date)
    });
    assert_backfilled("enriched_backfilled", &enriched_backfilled, fact_date);

    let imported_backfilled = draft_from("imported_backfilled", || {
        history_events::imported_backfilled(
            WORK_ID,
            WORK_TITLE,
            "/books/old.epub",
            "ebook",
            fact_date,
        )
    });
    assert_backfilled("imported_backfilled", &imported_backfilled, fact_date);
}

#[tokio::test]
async fn wh_event_type_serde_and_sqlite_history_maps_agree_for_all_kinds() {
    let db = common::create_test_db().await;
    let user_id = create_test_user(&db).await;

    let cases = [
        (EventType::Grabbed, "grabbed"),
        (EventType::DownloadCompleted, "downloadCompleted"),
        (EventType::DownloadFailed, "downloadFailed"),
        (EventType::Imported, "imported"),
        (EventType::ImportFailed, "importFailed"),
        (EventType::Enriched, "enriched"),
        (EventType::EnrichmentFailed, "enrichmentFailed"),
        (EventType::TagWritten, "tagWritten"),
        (EventType::TagWriteFailed, "tagWriteFailed"),
        (EventType::FileDeleted, "fileDeleted"),
        (EventType::Added, "added"),
        (EventType::WorkDeleted, "workDeleted"),
        (EventType::WorksMerged, "worksMerged"),
        (EventType::IdentityResolved, "identityResolved"),
    ];

    // Exhaustiveness gate (ir-v2 tri-map directive, r2 fold): one combined arm
    // naming every variant, NO wildcard — adding a future EventType variant
    // makes this match non-exhaustive and fails compilation here, forcing the
    // case list above to be extended in the same change.
    for (kind, _) in &cases {
        match kind {
            EventType::Grabbed
            | EventType::DownloadCompleted
            | EventType::DownloadFailed
            | EventType::Imported
            | EventType::ImportFailed
            | EventType::Enriched
            | EventType::EnrichmentFailed
            | EventType::TagWritten
            | EventType::TagWriteFailed
            | EventType::FileDeleted
            | EventType::Added
            | EventType::WorkDeleted
            | EventType::WorksMerged
            | EventType::IdentityResolved => {}
        }
    }
    let distinct: std::collections::HashSet<_> = cases
        .iter()
        .map(|(k, _)| std::mem::discriminant(k))
        .collect();
    assert_eq!(
        distinct.len(),
        cases.len(),
        "every variant appears exactly once in the case list"
    );

    for (kind, expected) in cases {
        assert_eq!(
            serde_json::to_value(kind).expect("event type should serialize"),
            json!(expected),
            "serde representation must be the public camelCase kind string"
        );

        db.create_history_event(CreateHistoryEventDbRequest {
            user_id,
            work_id: None,
            event_type: kind,
            data: json!({ "kind": expected }),
            date: None,
        })
        .await
        .expect("history insert should succeed");

        let rows = db
            .list_history(
                user_id,
                HistoryFilter {
                    event_type: Some(kind),
                    ..no_filter()
                },
            )
            .await
            .expect("history list should succeed");

        assert!(
            rows.iter().any(|row| row.event_type == kind),
            "a row inserted as {expected} must list back as {kind:?}"
        );
    }
}
