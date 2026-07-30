use livrarr_db::{sqlite::SqliteDb, AuthorRouteBackfillReport};
use livrarr_domain::AuthorRouteGuardResult;
use tokio_util::sync::CancellationToken;

use crate::readarr_client::RdAuthor;
use crate::state::AppState;

#[derive(Debug)]
pub struct StartupError;

pub async fn verify_author_link_cutover_before_serving(
    db: &SqliteDb,
) -> Result<AuthorRouteBackfillReport, StartupError> {
    todo!()
}

pub async fn author_link_sweep_tick(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    todo!()
}

pub fn readarr_author_route_evidence(
    rd_author: &RdAuthor,
    pre_observation_associated_names: &[String],
) -> Option<AuthorRouteGuardResult> {
    todo!()
}
