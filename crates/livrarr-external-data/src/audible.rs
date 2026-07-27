use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
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
pub struct AudibleCatalogClient<F: HttpFetcher = livrarr_http::fetcher::HttpFetcherImpl> {
    pub fetcher: F,
    pub retry_backoff_secs: i64,
    call_sink: Option<std::sync::Arc<dyn livrarr_domain::services::ProviderCallSink>>,
}

impl<F: HttpFetcher> AudibleCatalogClient<F> {
    pub fn new(fetcher: F, retry_backoff_secs: i64) -> Self {
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
    pub async fn fetch_by_asin(
        &self,
        asin: &str,
        priority: RequestPriority,
    ) -> crate::ProviderOutcome<NormalizedWorkDetail> {
        match lookup_audible_by_asin(&self.fetcher, asin, priority).await {
            Ok(Some(product)) => {
                report_audible_success();
                crate::ProviderOutcome::Success(Box::new(map_audible_to_detail(&product)))
            }
            Ok(None) => {
                report_audible_success();
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

    pub async fn fetch(
        &self,
        work: &livrarr_domain::Work,
        priority: RequestPriority,
    ) -> crate::ProviderOutcome<NormalizedWorkDetail> {
        // ASIN direct lookup
        if let Some(asin) = work.asin.as_deref().filter(|s| !s.is_empty()) {
            match lookup_audible_by_asin(&self.fetcher, asin, priority).await {
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
                    if livrarr_domain::identity_matching::pick_best_candidate(
                        &work.title,
                        &work.author_name,
                        &candidates,
                        false,
                    )
                    .is_some()
                    {
                        report_audible_success();
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
        match search_audible(&self.fetcher, &work.title, &work.author_name, 10, priority).await {
            Ok(products) if products.is_empty() => {
                report_audible_success();
                crate::ProviderOutcome::NotFound
            }
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

                if let Some(vidx) = livrarr_domain::identity_matching::pick_best_candidate(
                    &work.title,
                    &work.author_name,
                    &candidates,
                    false,
                ) {
                    let pi = variant_to_product[vidx];
                    report_audible_success();
                    return crate::ProviderOutcome::Success(Box::new(map_audible_to_detail(
                        &products[pi],
                    )));
                }

                report_audible_success();
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

fn report_audible_success() {
    outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Success);
}

// ─── API functions ───────────────────────────────────────────────────────

/// The fixed transport parameters every Audible API request carries. Audible
/// has no auth and no existing rate-limit-specific outcome discrimination —
/// any non-success status (including a fetcher-intercepted HTTP 429) maps to
/// the same generic `Err(String)` the pre-fetcher code already produced for
/// ANY non-success status, so no special `FetchError` translation is needed
/// here (unlike Goodreads/OpenLibrary/GoogleBooks/Audnexus).
fn audible_request(url: String, priority: RequestPriority) -> FetchRequest {
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
        priority,
    }
}

pub async fn search_audible<F: HttpFetcher>(
    fetcher: &F,
    title: &str,
    author: &str,
    max_results: u32,
    priority: RequestPriority,
) -> Result<Vec<AudibleProduct>, ProviderFetchError> {
    let url = format!(
        "{}?title={}&author={}&num_results={}&products_sort_by=Relevance&response_groups={}",
        BASE_URL,
        urlencoding::encode(title),
        urlencoding::encode(author),
        max_results,
        urlencoding::encode(RESPONSE_GROUPS),
    );

    let resp = match fetcher.fetch(audible_request(url, priority)).await {
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
        // Any non-2xx is a provider-health signal here; reporting only 5xx left
        // a refusal invisible to the breaker. No 404/410 exemption on THIS
        // endpoint: a search has no "this book is absent" status — an empty
        // result set is a 200 with an empty product list — so a 404 means the
        // ROUTE moved or is blocked. The by-ASIN lookup below keeps the
        // exemption, where a 404 really does mean that ASIN is absent.
        outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Failure);
        return Err(ProviderFetchError::Other(format!(
            "Audible returned {}",
            resp.status
        )));
    }

    let data: AudibleSearchResponse = serde_json::from_slice(&resp.body).map_err(|e| {
        outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Failure);
        ProviderFetchError::Other(format!("Audible parse error: {e}"))
    })?;

    Ok(data.products)
}

pub async fn lookup_audible_by_asin<F: HttpFetcher>(
    fetcher: &F,
    asin: &str,
    priority: RequestPriority,
) -> Result<Option<AudibleProduct>, ProviderFetchError> {
    let url = format!(
        "{}/{}?response_groups={}",
        BASE_URL,
        urlencoding::encode(asin),
        urlencoding::encode(RESPONSE_GROUPS),
    );

    let resp = match fetcher.fetch(audible_request(url, priority)).await {
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
        // A 404/410 on this ITEM route is the catalog saying it does not carry
        // that ASIN. It is neither a provider-health event nor something to
        // keep retrying, so it leaves the breaker alone AND returns as an
        // absence — an `Err` here reads as a provider failure one layer up
        // (`fetch_by_asin` maps it to `WillRetry`) and burns retry budget
        // forever on a book that will never appear.
        if resp.status == 404 || resp.status == 410 {
            return Ok(None);
        }
        // Any other non-2xx is a provider-health signal; reporting only 5xx
        // left a refusal invisible to the breaker.
        outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Failure);
        return Err(ProviderFetchError::Other(format!(
            "Audible returned {}",
            resp.status
        )));
    }

    let data: AudibleSearchResponse = serde_json::from_slice(&resp.body).map_err(|e| {
        outbound_queue::shared().report_outcome(RateBucket::Audible, BreakerSignal::Failure);
        ProviderFetchError::Other(format!("Audible parse error: {e}"))
    })?;

    Ok(data.products.into_iter().next())
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

    async fn half_open_audible_probe() -> livrarr_http::outbound_queue::QueuePermit {
        use livrarr_http::breaker::CircuitBreakerConfig;

        let queue = outbound_queue::shared();
        queue.reset_breaker_for_tests(RateBucket::Audible);
        queue.set_breaker_config_for_tests(
            RateBucket::Audible,
            CircuitBreakerConfig {
                failure_threshold: 5,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );
        queue.report_outcome(
            RateBucket::Audible,
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_millis(5)),
            },
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        queue
            .acquire(RateBucket::Audible, RequestPriority::Normal)
            .await
            .expect("half-open Audible breaker must admit one probe")
    }

    #[test]
    fn audible_client_accepts_the_recording_fetcher_seam() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<AudibleCatalogClient<crate::test_support::RecordingHttpFetcher>>();
    }

    #[tokio::test]
    async fn seeded_asin_mismatch_cannot_erase_later_search_failures_at_production_threshold() {
        use livrarr_domain::services::FetchResponse;
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        let work = livrarr_domain::Work {
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            asin: Some("B0MISMATCH".to_string()),
            ..Default::default()
        };

        for unreadable_search in [false, true] {
            queue.reset_breaker_for_tests(RateBucket::Audible);
            queue.set_breaker_config_for_tests(
                RateBucket::Audible,
                CircuitBreakerConfig {
                    failure_threshold: 5,
                    evaluation_window_secs: 60,
                    open_duration_secs: 60,
                    half_open_probe_count: 1,
                },
            );

            let fetcher = crate::test_support::RecordingHttpFetcher::new();
            for _ in 0..5 {
                fetcher.push_response(Ok(FetchResponse {
                    status: 200,
                    headers: vec![],
                    body: br#"{"products":[{"asin":"B0MISMATCH","title":"Foundation","authors":[{"name":"Isaac Asimov"}]}]}"#.to_vec(),
                }));
                fetcher.push_response(Ok(FetchResponse {
                    status: if unreadable_search { 200 } else { 403 },
                    headers: vec![],
                    body: if unreadable_search {
                        b"not-json".to_vec()
                    } else {
                        vec![]
                    },
                }));
            }
            let client = AudibleCatalogClient::new(fetcher, 1);

            for _ in 0..5 {
                let outcome = client.fetch(&work, RequestPriority::Normal).await;
                assert!(matches!(
                    outcome,
                    crate::ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::ServerError,
                        ..
                    }
                ));
            }

            let open = matches!(
                queue
                    .acquire(RateBucket::Audible, RequestPriority::Normal)
                    .await,
                Err(AdmissionError::CircuitOpen { .. })
            );
            assert!(
                open,
                "five seeded operations must retain their later {} search Failures",
                if unreadable_search { "decode" } else { "HTTP" }
            );
        }
    }

    #[tokio::test]
    async fn search_helper_leaves_half_open_success_ownership_to_the_client() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Audible,
            CircuitBreakerConfig {
                failure_threshold: 5,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );
        queue.report_outcome(
            RateBucket::Audible,
            BreakerSignal::TripImmediately {
                open_for: Some(Duration::from_millis(5)),
            },
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        let probe = queue
            .acquire(RateBucket::Audible, RequestPriority::Normal)
            .await
            .expect("half-open Audible breaker must admit one probe");

        let fetcher = crate::test_support::RecordingHttpFetcher::new();
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 200,
            headers: vec![],
            body: br#"{"products":[]}"#.to_vec(),
        }));
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 403,
            headers: vec![],
            body: vec![],
        }));
        let products = search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
        .await
        .expect("decoded empty search");
        assert!(products.is_empty());
        assert!(search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
        .await
        .is_err());
        drop(probe);

        assert!(matches!(
            queue
                .acquire(RateBucket::Audible, RequestPriority::Normal)
                .await,
            Err(AdmissionError::CircuitOpen { .. })
        ));
    }

    #[tokio::test]
    async fn terminal_asin_absences_close_half_open_only_at_the_client_boundary() {
        use livrarr_domain::services::FetchResponse;

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();

        for (status, body) in [
            (404, Vec::new()),
            (410, Vec::new()),
            (200, br#"{"products":[]}"#.to_vec()),
        ] {
            let probe = half_open_audible_probe().await;
            let fetcher = crate::test_support::RecordingHttpFetcher::new();
            fetcher.push_response(Ok(FetchResponse {
                status,
                headers: vec![],
                body,
            }));
            fetcher.push_response(Ok(FetchResponse {
                status: 403,
                headers: vec![],
                body: vec![],
            }));
            let client = AudibleCatalogClient::new(fetcher.clone(), 1);

            let outcome = client
                .fetch_by_asin("B0MISSING", RequestPriority::Normal)
                .await;
            assert!(matches!(outcome, crate::ProviderOutcome::NotFound));
            assert!(search_audible(
                &fetcher,
                "Dune",
                "Frank Herbert",
                10,
                RequestPriority::Normal,
            )
            .await
            .is_err());
            drop(probe);

            queue
                .acquire(RateBucket::Audible, RequestPriority::Normal)
                .await
                .expect(
                    "terminal healthy ASIN absence must close half-open before a later failure",
                );
        }
    }

    #[tokio::test]
    async fn matching_seeded_asin_is_terminal_reports_success_and_skips_search() {
        use livrarr_domain::services::FetchResponse;

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        let probe = half_open_audible_probe().await;
        let fetcher = crate::test_support::RecordingHttpFetcher::new();
        fetcher.push_response(Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: br#"{"products":[{"asin":"B0DUNE","title":"Dune","authors":[{"name":"Frank Herbert"}]}]}"#.to_vec(),
        }));
        fetcher.push_response(Ok(FetchResponse {
            status: 403,
            headers: vec![],
            body: vec![],
        }));
        let client = AudibleCatalogClient::new(fetcher.clone(), 1);
        let work = livrarr_domain::Work {
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            asin: Some("B0DUNE".to_string()),
            ..Default::default()
        };

        let outcome = client.fetch(&work, RequestPriority::Normal).await;
        assert!(matches!(outcome, crate::ProviderOutcome::Success(_)));
        assert_eq!(fetcher.call_count(), 1, "a Same ASIN hit is terminal");
        assert!(search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
        .await
        .is_err());
        drop(probe);

        queue
            .acquire(RateBucket::Audible, RequestPriority::Normal)
            .await
            .expect("matching seeded ASIN must report one outer Success");
    }

    #[tokio::test]
    async fn terminal_search_hit_and_miss_report_outer_success() {
        use livrarr_domain::services::FetchResponse;

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        let work = livrarr_domain::Work {
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            ..Default::default()
        };

        for body in [
            br#"{"products":[]}"#.to_vec(),
            br#"{"products":[{"asin":"B0DUNE","title":"Dune","authors":[{"name":"Frank Herbert"}]}]}"#
                .to_vec(),
        ] {
            let probe = half_open_audible_probe().await;
            let fetcher = crate::test_support::RecordingHttpFetcher::new();
            fetcher.push_response(Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body,
            }));
            fetcher.push_response(Ok(FetchResponse {
                status: 403,
                headers: vec![],
                body: vec![],
            }));
            let client = AudibleCatalogClient::new(fetcher.clone(), 1);

            let outcome = client.fetch(&work, RequestPriority::Normal).await;
            assert!(matches!(
                outcome,
                crate::ProviderOutcome::Success(_) | crate::ProviderOutcome::NotFound
            ));
            assert!(search_audible(
                &fetcher,
                "Dune",
                "Frank Herbert",
                10,
                RequestPriority::Normal,
            )
            .await
            .is_err());
            drop(probe);

            queue
                .acquire(RateBucket::Audible, RequestPriority::Normal)
                .await
                .expect("terminal healthy search must report one outer Success");
        }
    }

    #[tokio::test]
    async fn seeded_asin_miss_then_broken_search_reopens_half_open_without_early_success() {
        use livrarr_domain::services::FetchResponse;
        use livrarr_http::outbound_queue::AdmissionError;

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        let probe = half_open_audible_probe().await;
        let fetcher = crate::test_support::RecordingHttpFetcher::new();
        fetcher.push_response(Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: br#"{"products":[]}"#.to_vec(),
        }));
        fetcher.push_response(Ok(FetchResponse {
            status: 403,
            headers: vec![],
            body: vec![],
        }));
        let client = AudibleCatalogClient::new(fetcher, 1);
        let work = livrarr_domain::Work {
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            asin: Some("B0MISSING".to_string()),
            ..Default::default()
        };

        let outcome = client.fetch(&work, RequestPriority::Normal).await;
        assert!(matches!(
            outcome,
            crate::ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                ..
            }
        ));
        drop(probe);
        assert!(matches!(
            queue
                .acquire(RateBucket::Audible, RequestPriority::Normal)
                .await,
            Err(AdmissionError::CircuitOpen { .. })
        ));
    }

    #[tokio::test]
    async fn search_audible_invalid_json_opens_threshold_one_breaker() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Audible,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"not json".to_vec());
        let result = search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
        .await;
        assert!(
            matches!(
                result,
                Err(ProviderFetchError::Other(message))
                    if message.starts_with("Audible parse error:")
            ),
            "the existing search parse error must be preserved"
        );

        let admission = queue
            .acquire(RateBucket::Audible, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "the unreadable completed Audible search response must open a threshold-one breaker"
        );
    }

    #[tokio::test]
    async fn lookup_audible_by_asin_invalid_json_opens_threshold_one_breaker() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::Audible,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"not json".to_vec());
        let result = lookup_audible_by_asin(&fetcher, "B0INVALID", RequestPriority::Normal).await;
        assert!(
            matches!(
                result,
                Err(ProviderFetchError::Other(message))
                    if message.starts_with("Audible parse error:")
            ),
            "the existing by-ASIN parse error must be preserved"
        );

        let admission = queue
            .acquire(RateBucket::Audible, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "the unreadable completed by-ASIN response must open a threshold-one breaker"
        );
    }

    /// C4, test-plan item 6: a refusal must reach this client's own bucket.
    /// And no 404/410 exemption on the SEARCH route — an empty result set there
    /// is a 200 with an empty product list, so a 404 means the route is gone.
    /// The by-ASIN lookup keeps the exemption and is asserted alongside, since
    /// that contrast is the whole rule.
    #[tokio::test]
    async fn a_search_refusal_and_a_dead_search_route_both_report_a_breaker_failure() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let queue = outbound_queue::shared();
        let one_strike = || CircuitBreakerConfig {
            failure_threshold: 1,
            evaluation_window_secs: 60,
            open_duration_secs: 60,
            half_open_probe_count: 1,
        };
        let is_open =
            |r: &Result<_, AdmissionError>| matches!(r, Err(AdmissionError::CircuitOpen { .. }));

        for status in [403u16, 404u16] {
            queue.set_breaker_config_for_tests(RateBucket::Audible, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(status, vec![]);
            let _ = search_audible(&fetcher, "t", "a", 5, RequestPriority::Normal).await;
            let tripped = {
                let admission = queue
                    .acquire(RateBucket::Audible, RequestPriority::Normal)
                    .await;
                is_open(&admission)
            };
            assert!(
                tripped,
                "Audible search returning {status} must report a breaker failure"
            );
        }

        // The contrast: a 404 from the by-ASIN ITEM lookup is a genuine
        // absence, not a provider-health event, so it must leave the breaker
        // closed. This asserts the ABSENCE of a signal, which only holds
        // because every emitter in this binary takes the same lock — a single
        // unguarded emitter landing concurrently would fail it.
        for status in [404u16, 410u16] {
            queue.reset_breaker_for_tests(RateBucket::Audible);
            queue.set_breaker_config_for_tests(RateBucket::Audible, one_strike());
            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(status, vec![]);
            let absent =
                lookup_audible_by_asin(&fetcher, "B0MISSING", RequestPriority::Normal).await;
            // The health signal is only half the contract: an absence must also
            // reach the caller AS an absence. Returning `Err` here is read as a
            // provider failure one layer up and burns retry budget forever on a
            // book Audible simply does not carry (design §C3 scope line).
            assert!(
                matches!(absent, Ok(None)),
                "a {status} from the by-ASIN item lookup must be a soft miss, got {absent:?}"
            );
            let tripped = {
                let admission = queue
                    .acquire(RateBucket::Audible, RequestPriority::Normal)
                    .await;
                is_open(&admission)
            };
            assert!(
                !tripped,
                "a {status} from the by-ASIN item lookup is a missing book, not a \
                 provider failure — it must not trip the breaker"
            );
        }
    }

    // -------------------------------------------------------------------
    // Door-routing: search_audible / lookup_audible_by_asin go through the
    // HttpFetcher trait with the Audible rate bucket, GET, no auth.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn search_audible_sends_audible_bucket_get_with_query_params() {
        // Drives a request through the shared Audible bucket, so it emits or
        // depends on breaker state (C4) — hold the lock.
        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let canned = serde_json::json!({"products": []});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let products = search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
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
        // Drives a request through the shared Audible bucket, so it emits or
        // depends on breaker state (C4) — hold the lock.
        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let canned = serde_json::json!({"products": [{"asin": "B000FC0PBC", "title": "Dune"}]});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let product = lookup_audible_by_asin(&fetcher, "B000FC0PBC", RequestPriority::Normal)
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
        // Drives a request through the shared Audible bucket, so it emits or
        // depends on breaker state (C4) — hold the lock.
        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let canned = serde_json::json!({"products": []});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let product = lookup_audible_by_asin(&fetcher, "B0MISSING", RequestPriority::Normal)
            .await
            .unwrap();

        assert!(product.is_none());
    }

    // -------------------------------------------------------------------
    // Error mapping: any non-success status (including a fetcher-
    // intercepted 429) maps to a generic `Err(String)` — Audible has no
    // rate-limit-specific outcome, matching the pre-fetcher behavior.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn search_audible_maps_http_500_to_err() {
        // Emits a Failure to the shared Audible breaker (C4), so it must hold
        // the lock or it lands mid-assertion in a sibling breaker test.
        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);

        let err = search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert_eq!(err.to_string(), "Audible returned 500");
    }

    #[tokio::test]
    async fn search_audible_maps_fetcher_rate_limited_to_err() {
        // Drives a request through the shared Audible bucket, so it emits or
        // depends on breaker state (C4) — hold the lock.
        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::RateLimited,
        );

        let err = search_audible(
            &fetcher,
            "Dune",
            "Frank Herbert",
            10,
            RequestPriority::Normal,
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("Audible search failed"));
    }

    #[tokio::test]
    async fn lookup_audible_by_asin_maps_network_error_to_err() {
        // Drives a request through the shared Audible bucket, so it emits or
        // depends on breaker state (C4) — hold the lock.
        let _guard = crate::test_support::lock_breaker(RateBucket::Audible).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::Connection("refused".to_string()),
        );

        let err = lookup_audible_by_asin(&fetcher, "B000FC0PBC", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Audible ASIN lookup failed"));
    }
}
