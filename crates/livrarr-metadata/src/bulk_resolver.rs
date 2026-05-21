use crate::english_identity_resolver::{EnglishIdentityResolver, EnglishSeed};
use futures::stream::{self, StreamExt};
use livrarr_domain::identity::IdentityResolution;

pub async fn resolve_bulk<R: EnglishIdentityResolver>(
    resolver: &R,
    seeds: Vec<EnglishSeed>,
    concurrency: usize,
) -> Vec<IdentityResolution> {
    let cap = concurrency.max(1);
    let len = seeds.len();

    if len == 0 {
        return Vec::new();
    }

    let mut results: Vec<Option<IdentityResolution>> = (0..len).map(|_| None).collect();

    let indexed_futures = seeds
        .into_iter()
        .enumerate()
        .map(|(idx, seed)| async move { (idx, resolver.resolve(&seed).await) });

    let mut buffered = stream::iter(indexed_futures).buffer_unordered(cap);

    while let Some((idx, resolution)) = buffered.next().await {
        results[idx] = Some(resolution);
    }

    results.into_iter().map(|r| r.unwrap()).collect()
}
