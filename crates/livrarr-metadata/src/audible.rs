use livrarr_domain::text_norm;
use livrarr_http::HttpClient;
use serde::Deserialize;

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
    pub http: livrarr_http::HttpClient,
    pub retry_backoff_secs: i64,
}

impl AudibleCatalogClient {
    pub fn new(http: livrarr_http::HttpClient, retry_backoff_secs: i64) -> Self {
        Self {
            http,
            retry_backoff_secs,
        }
    }

    pub async fn fetch(
        &self,
        work: &livrarr_domain::Work,
        _ctx: &crate::EnrichmentContext,
    ) -> crate::ProviderOutcome<NormalizedWorkDetail> {
        // ASIN direct lookup
        if let Some(asin) = work.asin.as_deref().filter(|s| !s.is_empty()) {
            match lookup_audible_by_asin(&self.http, asin).await {
                Ok(Some(product)) => {
                    let title = product.title.clone().unwrap_or_default();
                    let author = product
                        .authors
                        .as_ref()
                        .and_then(|a| a.first())
                        .and_then(|a| a.name.clone())
                        .unwrap_or_default();
                    let candidates = vec![(title, author)];
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
        match search_audible(&self.http, &work.title, &work.author_name, 10).await {
            Ok(products) if products.is_empty() => crate::ProviderOutcome::NotFound,
            Ok(products) => {
                let candidates: Vec<(String, String)> = products
                    .iter()
                    .map(|p| {
                        let t = p.title.clone().unwrap_or_default();
                        let a = p
                            .authors
                            .as_ref()
                            .and_then(|au| au.first())
                            .and_then(|au| au.name.clone())
                            .unwrap_or_default();
                        (t, a)
                    })
                    .collect();

                if let Some(idx) =
                    score_provider_candidates(&work.title, &work.author_name, &candidates, 0.75, 1)
                {
                    return crate::ProviderOutcome::Success(Box::new(map_audible_to_detail(
                        &products[idx],
                    )));
                }

                crate::ProviderOutcome::NotFound
            }
            Err(_) => crate::ProviderOutcome::WillRetry {
                reason: livrarr_domain::WillRetryReason::ServerError,
                next_attempt_at: chrono::Utc::now()
                    + chrono::Duration::seconds(self.retry_backoff_secs),
            },
        }
    }
}

// ─── API functions ───────────────────────────────────────────────────────

pub async fn search_audible(
    http: &HttpClient,
    title: &str,
    author: &str,
    max_results: u32,
) -> Result<Vec<AudibleProduct>, String> {
    let url = format!(
        "{}?title={}&author={}&num_results={}&products_sort_by=Relevance&response_groups={}",
        BASE_URL,
        urlencoding::encode(title),
        urlencoding::encode(author),
        max_results,
        urlencoding::encode(RESPONSE_GROUPS),
    );

    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Audible search failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Audible returned {}", resp.status()));
    }

    let data: AudibleSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("Audible parse error: {e}"))?;

    Ok(data.products)
}

pub async fn lookup_audible_by_asin(
    http: &HttpClient,
    asin: &str,
) -> Result<Option<AudibleProduct>, String> {
    let url = format!(
        "{}/{}?response_groups={}",
        BASE_URL,
        urlencoding::encode(asin),
        urlencoding::encode(RESPONSE_GROUPS),
    );

    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Audible ASIN lookup failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Audible returned {}", resp.status()));
    }

    let data: AudibleSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("Audible parse error: {e}"))?;

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
