//! Release search service — business logic extracted from the release handler.
//!
//! Owns the search + grab pipeline so it can be tested without an Axum router.

use std::collections::HashMap;
use std::sync::Arc;

use livrarr_db::sqlite::SqliteDb;
use livrarr_domain::Indexer;
use tokio::task::JoinSet;

use crate::state::GrabSearchCache;
use crate::{ReleaseResponse, ReleaseSearchResponse, SearchWarning};

/// Drives release searching across all configured Torznab indexers.
#[derive(Clone)]
pub struct ReleaseService {
    pub db: SqliteDb,
    pub http_client: livrarr_http::HttpClient,
    pub grab_search_cache: Arc<GrabSearchCache>,
}

impl ReleaseService {
    pub fn new(
        db: SqliteDb,
        http_client: livrarr_http::HttpClient,
        grab_search_cache: Arc<GrabSearchCache>,
    ) -> Self {
        Self {
            db,
            http_client,
            grab_search_cache,
        }
    }

    /// Search all enabled indexers for a title + author combination.
    ///
    /// Returns a deduplicated, seeder-sorted result set with per-indexer warnings.
    pub async fn search(
        &self,
        title: &str,
        author: &str,
        indexers: Vec<Indexer>,
        refresh: bool,
        cache_only: bool,
    ) -> ReleaseSearchResponse {
        let total_indexers = indexers.len();
        let cache_title = title.to_lowercase();
        let cache_author = author.to_lowercase();

        let mut join_set = JoinSet::new();

        for indexer in indexers {
            let http = self.http_client.clone();
            let t = title.to_string();
            let a = author.to_string();
            let cache = self.grab_search_cache.clone();
            let ct = cache_title.clone();
            let ca = cache_author.clone();

            join_set.spawn(async move {
                let allowed_cats: std::collections::HashSet<i32> =
                    indexer.categories.iter().copied().collect();

                if !refresh {
                    if let Some((cached, age_secs)) = cache.get(&ct, &ca, indexer.id).await {
                        tracing::debug!(indexer = %indexer.name, age_secs, "grab search cache hit");
                        let filtered: Vec<_> = cached
                            .into_iter()
                            .filter(|r| {
                                r.categories.is_empty()
                                    || r.categories.iter().any(|c| allowed_cats.contains(c))
                            })
                            .collect();
                        return (
                            indexer.name.clone(),
                            indexer.priority,
                            Ok(filtered),
                            Some(age_secs),
                        );
                    }
                }

                if cache_only {
                    return (indexer.name.clone(), indexer.priority, Ok(vec![]), None);
                }

                let result =
                    crate::infra::release_helpers::search_indexer(&http, &indexer, &t, &a).await;
                if let Ok(ref items) = result {
                    cache.put(&ct, &ca, indexer.id, items.clone()).await;
                }
                let result = result.map(|items| {
                    items
                        .into_iter()
                        .filter(|r| {
                            r.categories.is_empty()
                                || r.categories.iter().any(|c| allowed_cats.contains(c))
                        })
                        .collect::<Vec<_>>()
                });
                (indexer.name.clone(), indexer.priority, result, Some(0u64))
            });
        }

        let mut all_results: Vec<(i32, ReleaseResponse)> = Vec::new();
        let mut warnings: Vec<SearchWarning> = Vec::new();
        let mut max_cache_age: Option<u64> = None;

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((_name, priority, Ok(items), age)) => {
                    if let Some(a) = age {
                        max_cache_age = Some(max_cache_age.map_or(a, |cur: u64| cur.max(a)));
                    }
                    for item in items {
                        all_results.push((priority, item));
                    }
                }
                Ok((name, _, Err(err), _)) => {
                    warnings.push(SearchWarning {
                        indexer: name,
                        error: err,
                    });
                }
                Err(e) => {
                    warnings.push(SearchWarning {
                        indexer: "unknown".into(),
                        error: format!("task panicked: {e}"),
                    });
                }
            }
        }

        // Dedup by guid: keep highest-priority (lowest number), break ties by seeders desc.
        let results_before_dedup = all_results.len();
        let mut by_guid: HashMap<String, (i32, ReleaseResponse)> = HashMap::new();
        for (priority, result) in all_results {
            let key = result.guid.clone();
            match by_guid.get(&key) {
                Some((existing_priority, existing)) => {
                    if priority < *existing_priority
                        || (priority == *existing_priority
                            && result.seeders.unwrap_or(0) > existing.seeders.unwrap_or(0))
                    {
                        by_guid.insert(key, (priority, result));
                    }
                }
                None => {
                    by_guid.insert(key, (priority, result));
                }
            }
        }

        let mut results: Vec<ReleaseResponse> = by_guid.into_values().map(|(_, r)| r).collect();
        results.sort_by(|a, b| b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0)));

        tracing::info!(
            indexers_total = total_indexers,
            indexers_succeeded = total_indexers - warnings.len(),
            indexers_failed = warnings.len(),
            results_before_dedup,
            results_after_dedup = results.len(),
            "release search complete"
        );

        let cache_age_seconds = max_cache_age.filter(|&a| a > 0);

        ReleaseSearchResponse {
            results,
            warnings,
            cache_age_seconds,
        }
    }

    /// Whether all indexers failed (used to decide 502 vs 200).
    pub fn all_failed(response: &ReleaseSearchResponse, total_indexers: usize) -> bool {
        response.warnings.len() == total_indexers
    }
}
