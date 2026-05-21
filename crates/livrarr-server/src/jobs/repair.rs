use livrarr_db::sqlite::SqliteDb;
use livrarr_db::WorkDb;
use livrarr_domain::identity::AnchorSetter;
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::{UserId, WorkId};

#[derive(Debug)]
pub struct RepairReport {
    pub total_processed: usize,
    pub repaired: usize,
    pub identity_pending: usize,
    pub errors: usize,
}

pub async fn run_repair(db: &SqliteDb, user_id: UserId, work_ids: Vec<WorkId>) -> RepairReport {
    let mut report = RepairReport {
        total_processed: 0,
        repaired: 0,
        identity_pending: 0,
        errors: 0,
    };

    for work_id in work_ids {
        report.total_processed += 1;

        let work = match db.get_work(user_id, work_id).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(work_id, "repair: failed to fetch work: {e}");
                report.errors += 1;
                continue;
            }
        };

        if work.language.as_deref() != Some("en") {
            continue;
        }

        if work.ol_key.is_some() {
            let anchors = db.list_anchors(work_id).await.unwrap_or_default();
            let has_confirmed = anchors
                .iter()
                .any(|a| a.confidence == livrarr_domain::identity::AnchorConfidence::Confirmed);
            if !has_confirmed {
                if let Some(ref ol_key) = work.ol_key {
                    match db
                        .confirm_ol_anchor(work_id, ol_key, AnchorSetter::Import)
                        .await
                    {
                        Ok(()) => {
                            report.repaired += 1;
                            tracing::info!(
                                work_id,
                                ol_key,
                                "repair: backfilled anchor from works.ol_key"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(work_id, "repair: anchor backfill failed: {e}");
                            report.errors += 1;
                        }
                    }
                }
            }
        } else {
            report.identity_pending += 1;
        }
    }

    report
}
