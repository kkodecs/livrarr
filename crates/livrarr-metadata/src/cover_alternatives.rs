use std::collections::HashMap;
use std::time::Duration;

use livrarr_domain::{
    CoverCandidateSource, CoverMediaType, InternalCoverCandidate, MetadataProvider, Work,
};

use crate::cover_resolution::should_reject_cover;
use crate::provider_client::ProviderClient;
use crate::{
    EnrichmentContext, EnrichmentMode, NormalizedWorkDetail, ProviderOutcome, RequestPriority,
};

const ALTERNATIVE_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

fn is_english_work(work: &Work) -> bool {
    work.language.as_deref() == Some("en") || work.language.is_none()
}

fn eligible_providers(work: &Work) -> Vec<MetadataProvider> {
    if is_english_work(work) {
        vec![
            MetadataProvider::Hardcover,
            MetadataProvider::Goodreads,
            MetadataProvider::OpenLibrary,
            MetadataProvider::Audnexus,
        ]
    } else {
        vec![MetadataProvider::Audnexus]
    }
}

fn extract_cover_info(
    _provider: MetadataProvider,
    outcome: ProviderOutcome<NormalizedWorkDetail>,
) -> Option<(String, Option<String>)> {
    match outcome {
        ProviderOutcome::Success(detail) => {
            let url = detail
                .cover_url
                .as_deref()
                .filter(|u| !u.is_empty())?
                .to_string();
            let edition_title = detail.title.clone();
            Some((url, edition_title))
        }
        _ => None,
    }
}

fn media_type_for_provider(provider: MetadataProvider) -> CoverMediaType {
    match provider {
        MetadataProvider::Audnexus => CoverMediaType::Audiobook,
        _ => CoverMediaType::Ebook,
    }
}

pub async fn fetch_internal_alternatives(
    work: &Work,
    clients: &HashMap<MetadataProvider, ProviderClient>,
    http: &livrarr_http::HttpClient,
) -> Vec<InternalCoverCandidate> {
    let eligible = eligible_providers(work);
    let mut candidates = Vec::new();

    let ctx = EnrichmentContext {
        priority: RequestPriority::Normal,
        mode: EnrichmentMode::Manual,
    };

    // Query providers in parallel with timeout
    let mut futures = Vec::new();
    for &provider in &eligible {
        if let Some(client) = clients.get(&provider) {
            let client = client.clone();
            let work_clone = work.clone();
            let ctx_clone = ctx.clone();
            futures.push(async move {
                let result = tokio::time::timeout(
                    ALTERNATIVE_FETCH_TIMEOUT,
                    client.fetch(&work_clone, &ctx_clone),
                )
                .await;
                match result {
                    Ok(outcome) => (provider, extract_cover_info(provider, outcome)),
                    Err(_) => {
                        tracing::debug!(?provider, "cover alternatives: provider timed out");
                        (provider, None)
                    }
                }
            });
        }
    }

    let results = futures::future::join_all(futures).await;

    for (provider, info) in results {
        if let Some((url, edition_title)) = info {
            let media_type = media_type_for_provider(provider);
            if media_type == CoverMediaType::Ebook
                && should_reject_cover(edition_title.as_deref(), &work.title)
            {
                tracing::debug!(?provider, "cover alternatives: rejected by title filter");
                continue;
            }
            candidates.push(InternalCoverCandidate {
                source: CoverCandidateSource::Provider(provider),
                url,
                media_type,
                edition_title,
            });
        }
    }

    // ISBN-based covers (English: OL→Amazon, Foreign: CdL→Amazon)
    let isbn = work.isbn_13.as_deref();
    if is_english_work(work) {
        if let Some(url) = crate::cover::resolve_cover_english(http, isbn).await {
            candidates.push(InternalCoverCandidate {
                source: CoverCandidateSource::IsbnOl,
                url,
                media_type: CoverMediaType::Ebook,
                edition_title: None,
            });
        }
    } else if let Some(url) = crate::cover::resolve_cover_foreign(http, isbn).await {
        candidates.push(InternalCoverCandidate {
            source: CoverCandidateSource::IsbnAmazon,
            url,
            media_type: CoverMediaType::Ebook,
            edition_title: None,
        });
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_english_includes_all_providers() {
        let work = Work {
            language: Some("en".to_string()),
            ..Default::default()
        };
        let providers = eligible_providers(&work);
        assert_eq!(providers.len(), 4);
        assert!(providers.contains(&MetadataProvider::Hardcover));
        assert!(providers.contains(&MetadataProvider::Goodreads));
        assert!(providers.contains(&MetadataProvider::OpenLibrary));
        assert!(providers.contains(&MetadataProvider::Audnexus));
    }

    #[test]
    fn eligible_foreign_only_audnexus() {
        let work = Work {
            language: Some("ko".to_string()),
            ..Default::default()
        };
        let providers = eligible_providers(&work);
        assert_eq!(providers, vec![MetadataProvider::Audnexus]);
    }

    #[test]
    fn eligible_no_language_defaults_english() {
        let work = Work::default();
        let providers = eligible_providers(&work);
        assert_eq!(providers.len(), 4);
    }

    #[test]
    fn media_type_audnexus_is_audiobook() {
        assert_eq!(
            media_type_for_provider(MetadataProvider::Audnexus),
            CoverMediaType::Audiobook
        );
    }

    #[test]
    fn media_type_others_are_ebook() {
        assert_eq!(
            media_type_for_provider(MetadataProvider::Hardcover),
            CoverMediaType::Ebook
        );
        assert_eq!(
            media_type_for_provider(MetadataProvider::Goodreads),
            CoverMediaType::Ebook
        );
        assert_eq!(
            media_type_for_provider(MetadataProvider::OpenLibrary),
            CoverMediaType::Ebook
        );
    }

    #[test]
    fn extract_cover_info_success_with_url() {
        let detail = NormalizedWorkDetail {
            cover_url: Some("https://example.test/cover.jpg".to_string()),
            title: Some("Some Book".to_string()),
            ..Default::default()
        };
        let outcome = ProviderOutcome::Success(Box::new(detail));
        let (url, title) = extract_cover_info(MetadataProvider::Hardcover, outcome).unwrap();
        assert_eq!(url, "https://example.test/cover.jpg");
        assert_eq!(title.as_deref(), Some("Some Book"));
    }

    #[test]
    fn extract_cover_info_success_no_url() {
        let detail = NormalizedWorkDetail::default();
        let outcome = ProviderOutcome::Success(Box::new(detail));
        assert!(extract_cover_info(MetadataProvider::Hardcover, outcome).is_none());
    }

    #[test]
    fn extract_cover_info_not_found() {
        assert!(
            extract_cover_info(MetadataProvider::Hardcover, ProviderOutcome::NotFound).is_none()
        );
    }
}
