use crate::english_identity_resolver::{EnglishIdentityResolver, WorkSeed};
use futures::stream::{self, StreamExt};
use livrarr_domain::identity::Resolution;

pub async fn resolve_bulk<R: EnglishIdentityResolver>(
    resolver: &R,
    user_id: livrarr_domain::UserId,
    seeds: Vec<WorkSeed>,
    concurrency: usize,
) -> Vec<Resolution> {
    let cap = concurrency.max(1);
    let len = seeds.len();

    if len == 0 {
        return Vec::new();
    }

    let mut results: Vec<Option<Resolution>> = (0..len).map(|_| None).collect();

    let indexed_futures = seeds.into_iter().enumerate().map(|(idx, seed)| async move {
        let resolution = resolver
            .resolve(user_id, &seed, livrarr_domain::identity::LatencyTier::Bulk)
            .await
            .unwrap_or_else(|_| Resolution::Unresolved {
                captured: livrarr_domain::identity::CapturedIdentity {
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    isbn_13: None,
                    asin: None,
                    title: seed.title.clone().unwrap_or_default(),
                    author_name: seed.author_name.clone().unwrap_or_default(),
                    language: None,
                },
                reason: livrarr_domain::identity::PendingReason::OlUnavailable,
                candidate_id: None,
            });
        (idx, resolution)
    });

    let mut buffered = stream::iter(indexed_futures).buffer_unordered(cap);

    while let Some((idx, resolution)) = buffered.next().await {
        results[idx] = Some(resolution);
    }

    results.into_iter().map(|r| r.unwrap()).collect()
}
