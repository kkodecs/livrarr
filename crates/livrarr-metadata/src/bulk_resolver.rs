use crate::english_identity_resolver::{EnglishIdentityResolver, EnglishSeed};
use livrarr_domain::identity::IdentityResolution;

pub async fn resolve_bulk<R: EnglishIdentityResolver>(
    _resolver: &R,
    _seeds: Vec<EnglishSeed>,
    _concurrency: usize,
) -> Vec<IdentityResolution> {
    todo!()
}
