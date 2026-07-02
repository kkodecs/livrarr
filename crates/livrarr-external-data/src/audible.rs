use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::text_norm;
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;
use serde::Deserialize;
use std::time::Duration;

use crate::types::ProviderFetchError;
use crate::NormalizedWorkDetail;

const BASE_URL: &str = "https://api.audible.com/1.0/catalog/products";
const RESPONSE_GROUPS: &str =
    "product_desc,product_attrs,contributors,series,media,product_extended_attrs";

// ─── Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AudibleProduct {
    #[serde(default)]
    pub asin: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub runtime_length_min: Option<i32>,
    #[serde(default)]
    pub authors: Option<Vec<AudibleContributor>>,
    #[serde(default)]
    pub narrators: Option<Vec<AudibleContributor>>,
    #[serde(default)]
    pub series: Option<Vec<AudibleSeries>>,
    #[serde(default)]
    pub product_images: Option<AudibleImages>,
    #[serde(default)]
    pub publisher_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudibleContributor {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudibleSeries {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub sequence: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudibleImages {
    #[serde(rename = "500", default)]
    pub large: Option<String>,
    #[serde(rename = "252", default)]
    pub medium: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AudibleSearchResponse {
    #[serde(default)]
    products: Vec<AudibleProduct>,
}

// ─── Client ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AudibleCatalogClient {
    pub fetcher: livrarr_http::fetcher::HttpFetcherImpl,
    pub retry_backoff_secs: i64,
    #[allow(dead_code)] // read at green: REQ-001 record emission
    call_sink: Option<std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>>,
}

impl AudibleCatalogClient {
    pub fn new(fetcher: livrarr_http::fetcher::HttpFetcherImpl, retry_backoff_secs: i64) -> Self {
        Self {
            fetcher,
            retry_backoff_secs,
            call_sink: None,
        }
    }

    /// Inject the call-record sink (REQ-001).
    pub fn with_call_sink(
        mut self,
        sink: std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>,
    ) -> Self {
        self.call_sink = Some(sink);
        self
    }

    pub(crate) fn sink_ref(
        &self,
    ) -> Option<&std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>> {
        self.call_sink.as_ref()
    }

    /// Anchor-only fetch (REQ-006): catalog lookup by ASIN. The stored anchor
    /// is canonical identity, so no title rescoring applies here — the scoring
    /// in the seeded fetch guards wrong-ASIN adoption on the text path, which
    /// does not exist on this surface.
    pub async fn fetch_by_asin(&self, asin: &str) -> crate::ProviderOutcome<NormalizedWorkDetail> {
        match lookup_audible_by_asin(&self.fetcher, asin).await {
            Ok(Some(product)) => {
                crate::ProviderOutcome::Success(Box::new(map_audible_to_detail(&product)))
            }
            Ok(None) => crate::ProviderOutcome::NotFound,
            Err(ProviderFetchError::CircuitOpen(retry_after)) => circuit_open_outcome(retry_after),
            Err(_) => crate::ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: chrono::Utc::now()
                    + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }

    pub async fn fetch(
        &self,
        work: &livrarr_domain::Work,
    ) -> crate::ProviderOutcome<NormalizedWorkDetail> {
        // ASIN direct lookup
        if let Some(asin) = work.asin.as_deref().filter(|s| !s.is_empty()) {
            match lookup_audible_by_asin(&self.fetcher, asin).await {
                Ok(Some(product)) => {
                    let author = product
                        .authors
                        .as_ref()
                        .and_then(|a| a.first())
                        .and_then(|a| a.name.clone())
                        .unwrap_or_default();
                    // Score every title variant (raw, series-stripped, subtitle)
                    // so a series-prefixed catalog title still matches the seed.
                    let candidates: Vec<(String, String)> = audible_title_variants(&product)
                        .into_iter()
                        .map(|t| (t, author.clone()))
                        .collect();
                    if score_provider_candidates(
                        &work.title,
                        &work.author_name,
                        &candidates,
                        0.75,
                        1,
                    )
                    .is_some()
                    {
                        return crate::ProviderOutcome::Success(Box::new(map_audible_to_detail(
                            &product,
                        )));
                    }
                    tracing::debug!(asin = %asin, "Audible ASIN lookup title mismatch, falling through to search");
                }
                Ok(None) => {
                    tracing::debug!(asin = %asin, "Audible ASIN lookup: not found");
                }
                Err(ProviderFetchError::CircuitOpen(retry_after)) => {
                    return circuit_open_outcome(retry_after);
                }
                Err(_) => {
                    return crate::ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        next_attempt_at: chrono::Utc::now()
                            + chrono::Duration::seconds(self.retry_backoff_secs),
                    };
                }
            }
        }

        // Title+author search
        match search_audible(&self.fetcher, &work.title, &work.author_name, 10).await {
            Ok(products) if products.is_empty() => crate::ProviderOutcome::NotFound,
            Ok(products) => {
                // Feed multiple title variants per product (raw, series-stripped,
                // subtitle) into the strict matcher so a series-prefixed catalog
                // title still matches a bare seed title; track which product each
                // variant came from to map the winner back.
                let mut candidates: Vec<(String, String)> = Vec::new();
                let mut variant_to_product: Vec<usize> = Vec::new();
                for (pi, p) in products.iter().enumerate() {
                    let author = p
                        .authors
                        .as_ref()
                        .and_then(|au| au.first())
                        .and_then(|au| au.name.clone())
                        .unwrap_or_default();
                    for title in audible_title_variants(p) {
                        candidates.push((title, author.clone()));
                        variant_to_product.push(pi);
                    }
                }

                if let Some(vidx) =
                    score_provider_candidates(&work.title, &work.author_name, &candidates, 0.75, 1)
                {
                    let pi = variant_to_product[vidx];
                    return crate::ProviderOutcome::Success(Box::new(map_audible_to_detail(
                        &products[pi],
                    )));
                }

                crate::ProviderOutcome::NotFound
            }
            Err(ProviderFetchError::CircuitOpen(retry_after)) => circuit_open_outcome(retry_after),
            Err(_) => crate::ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: chrono::Utc::now()
                    + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }
}

/// Common `WillRetry { CircuitOpen }` mapping (R-11), local to this module —
/// mirrors `provider_client::circuit_open_outcome` for the one client
/// (`AudibleCatalogClient`) whose enrichment-surface methods live outside
/// `provider_client.rs`.
fn circuit_open_outcome(retry_after: Duration) -> crate::ProviderOutcome<NormalizedWorkDetail> {
    crate::ProviderOutcome::WillRetry {
        reason: livrarr_domain::WillRetryReason::CircuitOpen,
        next_attempt_at: chrono::Utc::now()
            + chrono::Duration::from_std(retry_after)
                .unwrap_or_else(|_| chrono::Duration::seconds(60)),
    }
}

// ─── API functions ───────────────────────────────────────────────────────

/// The fixed transport parameters every Audible API request carries. Audible
/// has no auth and no existing rate-limit-specific outcome discrimination —
/// any non-success status (including a fetcher-intercepted HTTP 429) maps to
/// the same generic `Err(String)` the pre-fetcher code already produced for
/// ANY non-success status, so no special `FetchError` translation is needed
/// here (unlike Goodreads/OpenLibrary/GoogleBooks/Audnexus).
fn audible_request(url: String) -> FetchRequest {
    FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: Duration::from_secs(30),
        rate_bucket: RateBucket::Audible,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    }
}

pub async fn search_audible<F: HttpFetcher>(
    fetcher: &F,
    title: &str,
    author: &str,
    max_results: u32,
) -> Result<Vec<AudibleProduct>, ProviderFetchError> {
    let url = format!(
        "{}?title={}&author={}&num_results={}&products_sort_by=Relevance&response_groups={}",
        BASE_URL,
        urlencoding::encode(title),
        urlencoding::encode(author),
        max_results,
        urlencoding::encode(RESPONSE_GROUPS),
    );

    let resp = match fetcher.fetch(audible_request(url)).await {
        Ok(r) => r,
        Err(FetchError::CircuitOpen { retry_after }) => {
            return Err(ProviderFetchError::CircuitOpen(retry_after));
        }
        Err(e) => {
            return Err(ProviderFetchError::Other(format!(
                "Audible search failed: {e}"
            )))
        }
    };

    if !(200..300).contains(&resp.status) {
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Failure);
        }
        return Err(ProviderFetchError::Other(format!(
            "Audible returned {}",
            resp.status
        )));
    }

    let data: AudibleSearchResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| ProviderFetchError::Other(format!("Audible parse error: {e}")))?;
    outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Success);

    Ok(data.products)
}

pub async fn lookup_audible_by_asin<F: HttpFetcher>(
    fetcher: &F,
    asin: &str,
) -> Result<Option<AudibleProduct>, ProviderFetchError> {
    let url = format!(
        "{}/{}?response_groups={}",
        BASE_URL,
        urlencoding::encode(asin),
        urlencoding::encode(RESPONSE_GROUPS),
    );

    let resp = match fetcher.fetch(audible_request(url)).await {
        Ok(r) => r,
        Err(FetchError::CircuitOpen { retry_after }) => {
            return Err(ProviderFetchError::CircuitOpen(retry_after));
        }
        Err(e) => {
            return Err(ProviderFetchError::Other(format!(
                "Audible ASIN lookup failed: {e}"
            )))
        }
    };

    if !(200..300).contains(&resp.status) {
        if (500..600).contains(&resp.status) {
            outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Failure);
        }
        return Err(ProviderFetchError::Other(format!(
            "Audible returned {}",
            resp.status
        )));
    }

    let data: AudibleSearchResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| ProviderFetchError::Other(format!("Audible parse error: {e}")))?;
    outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Success);

    Ok(data.products.into_iter().next())
}

// ─── Scoring helper ──────────────────────────────────────────────────────

pub fn score_provider_candidates(
    seed_title: &str,
    seed_author: &str,
    candidates: &[(String, String)],
    min_title_jaccard: f64,
    min_author_overlap: u32,
) -> Option<usize> {
    let seed_title_tokens = text_norm::title_tokens(seed_title);
    let seed_author_tokens = text_norm::author_tokens(seed_author);

    candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, (title, author))| {
            let title_jaccard =
                text_norm::jaccard(&seed_title_tokens, &text_norm::title_tokens(title));
            let author_overlap = seed_author_tokens
                .intersection(&text_norm::author_tokens(author))
                .count() as u32;
            if title_jaccard >= min_title_jaccard && author_overlap >= min_author_overlap {
                Some((idx, title_jaccard, author_overlap))
            } else {
                None
            }
        })
        .max_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.2.cmp(&b.2))
        })
        .map(|(idx, _, _)| idx)
}

/// Title variants Audible may use for a product, fed to the (strict, shared)
/// matcher so a series-prefixed catalog title still matches a bare seed title.
/// Produces: the series-stripped title (when the title begins with the
/// structured series name), the raw title, and the subtitle — deduped, in that
/// preference order. The shared scorer stays strict, so a sequel like "Dune
/// Messiah" still fails to match a "Dune" seed (no variant is contained-only).
fn audible_title_variants(product: &AudibleProduct) -> Vec<String> {
    let mut variants: Vec<String> = Vec::new();
    let raw = product.title.clone().unwrap_or_default();

    // Series-prefix strip: Audible titles series books as "<series>: <title>"
    // or "<series>, Book N: <title>". When the title begins with the structured
    // series name, drop everything up to and including the first ": ".
    if let Some(series) = product
        .series
        .as_ref()
        .and_then(|s| s.first())
        .and_then(|s| s.title.as_deref())
        .filter(|s| !s.is_empty())
    {
        if raw.to_lowercase().starts_with(&series.to_lowercase()) {
            if let Some(pos) = raw.find(": ") {
                let stripped = raw[pos + 2..].trim().to_string();
                if !stripped.is_empty() {
                    variants.push(stripped);
                }
            }
        }
    }

    for candidate in [Some(raw), product.subtitle.clone()].into_iter().flatten() {
        let candidate = candidate.trim().to_string();
        if !candidate.is_empty() && !variants.contains(&candidate) {
            variants.push(candidate);
        }
    }

    variants
}

// ─── Mapping ─────────────────────────────────────────────────────────────

pub fn map_audible_to_detail(product: &AudibleProduct) -> NormalizedWorkDetail {
    let (series_name, series_position) = product
        .series
        .as_ref()
        .and_then(|s| s.first())
        .map(|entry| {
            (
                entry.title.clone(),
                entry
                    .sequence
                    .as_deref()
                    .and_then(|s| s.parse::<f64>().ok()),
            )
        })
        .unwrap_or((None, None));

    let cover_url = product
        .product_images
        .as_ref()
        .and_then(|i| i.large.as_ref().or(i.medium.as_ref()))
        .map(|url| {
            if url.starts_with("http://") {
                url.replacen("http://", "https://", 1)
            } else {
                url.clone()
            }
        });

    let narrator = product
        .narrators
        .as_ref()
        .map(|ns| {
            ns.iter()
                .filter_map(|n| n.name.as_deref())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    NormalizedWorkDetail {
        title: product.title.clone(),
        author_name: product
            .authors
            .as_ref()
            .and_then(|a| a.first())
            .and_then(|a| a.name.clone()),
        narrator,
        duration_seconds: product.runtime_length_min.map(|m| m * 60),
        series_name,
        series_position,
        cover_url,
        publisher: None,
        asin: product.asin.clone(),
        language: product
            .language
            .as_deref()
            .map(livrarr_domain::normalize_language),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Door-routing: search_audible / lookup_audible_by_asin go through the
    // HttpFetcher trait with the Audible rate bucket, GET, no auth.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn search_audible_sends_audible_bucket_get_with_query_params() {
        let canned = serde_json::json!({"products": []});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let products = search_audible(&fetcher, "Dune", "Frank Herbert", 10)
            .await
            .unwrap();

        assert!(products.is_empty());
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert!(req.url.starts_with(BASE_URL));
        assert!(req.url.contains("title=Dune"));
        assert!(req.url.contains("num_results=10"));
        assert_eq!(req.rate_bucket, RateBucket::Audible);
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.headers.is_empty());
        assert!(matches!(req.user_agent, UserAgentProfile::Server));
        assert!(!req.anti_bot_check);
        assert_eq!(req.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn lookup_audible_by_asin_sends_audible_bucket_get_asin_path() {
        let canned = serde_json::json!({"products": [{"asin": "B000FC0PBC", "title": "Dune"}]});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let product = lookup_audible_by_asin(&fetcher, "B000FC0PBC")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(product.asin.as_deref(), Some("B000FC0PBC"));
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].url.starts_with(&format!("{BASE_URL}/B000FC0PBC")));
        assert_eq!(reqs[0].rate_bucket, RateBucket::Audible);
    }

    #[tokio::test]
    async fn lookup_audible_by_asin_returns_none_for_empty_products() {
        let canned = serde_json::json!({"products": []});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let product = lookup_audible_by_asin(&fetcher, "B0MISSING").await.unwrap();

        assert!(product.is_none());
    }

    // -------------------------------------------------------------------
    // Error mapping: any non-success status (including a fetcher-
    // intercepted 429) maps to a generic `Err(String)` — Audible has no
    // rate-limit-specific outcome, matching the pre-fetcher behavior.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn search_audible_maps_http_500_to_err() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);

        let err = search_audible(&fetcher, "Dune", "Frank Herbert", 10)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Audible returned 500");
    }

    #[tokio::test]
    async fn search_audible_maps_fetcher_rate_limited_to_err() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::RateLimited,
        );

        let err = search_audible(&fetcher, "Dune", "Frank Herbert", 10)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Audible search failed"));
    }

    #[tokio::test]
    async fn lookup_audible_by_asin_maps_network_error_to_err() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::Connection("refused".to_string()),
        );

        let err = lookup_audible_by_asin(&fetcher, "B000FC0PBC")
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Audible ASIN lookup failed"));
    }
}
