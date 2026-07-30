use livrarr_db::{AuthorNameVariantDb, WorkDb};
use livrarr_domain::{ProviderAuthorNameObservation, UserId, WorkId};

pub struct AuthorNameVariantObserver;

impl AuthorNameVariantObserver {
    pub async fn record_observed_author_names(
        db: &(impl AuthorNameVariantDb + WorkDb),
        user_id: UserId,
        work_id: WorkId,
        observations: &[ProviderAuthorNameObservation],
    ) {
        todo!()
    }
}
