use std::collections::HashMap;
use std::time::Duration;

use livrarr_domain::services::{PreaddCoverCandidate, PreaddCoverError, PreaddCoverService};
use livrarr_domain::{MetadataProvider, RequestPriority, UserId, Work};

use livrarr_external_data::provider_client::ProviderClient;

#[derive(Clone)]
pub struct LivePreaddCoverService {
    clients: HashMap<MetadataProvider, ProviderClient>,
}

impl LivePreaddCoverService {
    pub fn new(clients: HashMap<MetadataProvider, ProviderClient>) -> Self {
        Self { clients }
    }
}

impl PreaddCoverService for LivePreaddCoverService {
    async fn fetch_cover_alternatives(
        &self,
        _user_id: UserId,
        title: &str,
        author: &str,
        lang: &str,
        isbn_13: Option<&str>,
    ) -> Result<Vec<PreaddCoverCandidate>, PreaddCoverError> {
        let temp_work = Work {
            title: title.to_string(),
            author_name: author.to_string(),
            language: Some(lang.to_string()),
            isbn_13: isbn_13.map(|s| s.to_string()),
            ..Default::default()
        };

        let eligible = crate::cover_alternatives::eligible_providers_for_work(&temp_work);

        let mut handles = Vec::new();
        for provider in &eligible {
            if let Some(client) = self.clients.get(provider) {
                let client = client.clone();
                let work = temp_work.clone();
                let provider = *provider;
                handles.push(tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(10),
                        client.fetch(&work, RequestPriority::High),
                    )
                    .await;
                    (provider, result)
                }));
            }
        }

        let mut candidates = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();

        for handle in handles {
            if let Ok((provider, Ok(crate::ProviderOutcome::Success(detail)))) = handle.await {
                if let Some(ref cover_url) = detail.cover_url {
                    let proxy_url = livrarr_domain::proxy_cover_url(cover_url);
                    if seen_urls.insert(proxy_url.clone()) {
                        candidates.push(PreaddCoverCandidate {
                            proxy_url,
                            source: provider_display_name(provider),
                            title: detail.title.clone().unwrap_or_default(),
                            author_name: detail.author_name.clone().unwrap_or_default(),
                        });
                    }
                }
            }
        }

        Ok(candidates)
    }
}

fn provider_display_name(provider: MetadataProvider) -> String {
    match provider {
        MetadataProvider::Hardcover => "Hardcover",
        MetadataProvider::OpenLibrary => "OpenLibrary",
        MetadataProvider::Goodreads => "Goodreads",
        MetadataProvider::Audnexus => "Audnexus",
        MetadataProvider::Audible => "Audiobook",
        MetadataProvider::GoogleBooks => "Google Books",
        MetadataProvider::Llm => "LLM",
        MetadataProvider::Readarr => "Readarr",
    }
    .to_string()
}
