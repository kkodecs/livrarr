use livrarr_db::{ProvenanceDb, SetFieldProvenanceRequest};
use livrarr_domain::{ProvenanceSetter, Work, WorkField};

/// Write provenance records for all non-empty identity fields on a work at add-time.
/// Parameterized by `setter` so callers can distinguish user-initiated adds from auto-adds.
pub async fn write_addtime_provenance<D: ProvenanceDb>(
    db: &D,
    user_id: i64,
    work: &Work,
    setter: ProvenanceSetter,
) {
    let mut reqs: Vec<SetFieldProvenanceRequest> = Vec::new();
    let push = |reqs: &mut Vec<SetFieldProvenanceRequest>, field: WorkField| {
        reqs.push(SetFieldProvenanceRequest {
            user_id,
            work_id: work.id,
            field,
            source: None,
            setter,
            cleared: false,
        });
    };
    if !work.title.is_empty() {
        push(&mut reqs, WorkField::Title);
    }
    if !work.author_name.is_empty() {
        push(&mut reqs, WorkField::AuthorName);
    }
    if work.ol_key.is_some() {
        push(&mut reqs, WorkField::OlKey);
    }
    if work.gr_key.is_some() {
        push(&mut reqs, WorkField::GrKey);
    }
    if work.language.is_some() {
        push(&mut reqs, WorkField::Language);
    }
    if work.year.is_some() {
        push(&mut reqs, WorkField::Year);
    }
    if work.series_name.is_some() {
        push(&mut reqs, WorkField::SeriesName);
    }
    if work.series_position.is_some() {
        push(&mut reqs, WorkField::SeriesPosition);
    }
    if reqs.is_empty() {
        return;
    }
    if let Err(e) = db.set_field_provenance_batch(reqs).await {
        tracing::warn!(
            work_id = work.id,
            ?setter,
            "write_addtime_provenance failed: {e}"
        );
    }
}
