use std::sync::Arc;
use std::time::Duration;

use tracing::warn;
use url::Url;

use crate::{
    LlmClient, LlmError, LlmMessage, LlmRole, MetadataError, MetadataProvider,
    ProviderAuthorResult, ProviderSearchResult, ProviderWorkDetail,
};
use livrarr_external_data::normalize::nfc;
use livrarr_external_data::provider_util::{
    clean_html_for_llm, is_anti_bot_page, validate_cover_url,
};
use livrarr_http::HttpClient;

/// HTTP status codes for anti-bot detection (avoids direct reqwest dep).
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;

// =============================================================================
// LLM Response Parsing
// =============================================================================

#[derive(serde::Deserialize)]
struct LlmBookResult {
    title: Option<String>,
    author: Option<String>,
    year: Option<i32>,
    cover_url: Option<String>,
    detail_url: Option<String>,
}

/// Validate a year value is reasonable (1000–2100).
fn is_valid_year(year: i32) -> bool {
    (1000..=2100).contains(&year)
}

// =============================================================================
// LLM Extraction Prompt
// =============================================================================

const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are a metadata extraction tool. Extract book search results from the provided HTML page.

Return ONLY a JSON array of objects. Each object must have exactly these fields:
- "title": string (the book title, in the original language)
- "author": string (the primary author name)
- "year": integer or null (publication year, if visible)
- "cover_url": string or null (full URL of the cover image, resolve relative URLs)
- "detail_url": string or null (full URL of the book's detail/product page, resolve relative URLs)

Rules:
- Return ONLY the JSON array, no markdown fences, no explanation
- If a field is not visible on the page, use null
- Do NOT invent or guess missing data
- Extract all distinct book results visible on the page
- For relative image URLs, prepend the site's base URL
- For relative detail URLs, prepend the site's base URL
- If there are no book results, return an empty array: []"#;

fn build_user_prompt(cleaned_html: &str, base_url: &str) -> String {
    format!(
        "Extract book search results from this page (base URL: {}):\n\n{}",
        base_url, cleaned_html
    )
}

// =============================================================================
// LlmScraperProvider
// =============================================================================

pub struct LlmScraperConfig {
    /// Provider name for attribution (e.g., "lubimyczytac.pl")
    pub name: String,
    /// Search URL template. `{query}` is replaced with URL-encoded search term.
    pub search_url_template: String,
    /// Language code this provider serves.
    pub language: String,
}

pub struct LlmScraperProvider<L> {
    config: LlmScraperConfig,
    llm: Arc<L>,
    http: HttpClient,
}

impl<L: LlmClient> LlmScraperProvider<L> {
    pub fn new(config: LlmScraperConfig, llm: Arc<L>, http: HttpClient) -> Self {
        Self { config, llm, http }
    }

    fn build_url(&self, query: &str) -> String {
        let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
        self.config.search_url_template.replace("{query}", &encoded)
    }

    /// Extract the base URL (scheme + host) from the search URL template.
    fn base_url(&self) -> String {
        if let Ok(parsed) = Url::parse(&self.config.search_url_template.replace("{query}", "x")) {
            format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
        } else {
            String::new()
        }
    }

    async fn search_works_inner(
        &self,
        query: &str,
    ) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        let url = self.build_url(query);
        let base_url = self.base_url();

        // HTTP GET the search page — use a browser UA to avoid bot-detection on retail sites.
        let resp = self
            .http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| MetadataError::RequestFailed(format!("HTTP fetch failed: {e}")))?;

        let status = resp.status();

        // Check for 403/429 → anti-bot
        if status.as_u16() == HTTP_FORBIDDEN || status.as_u16() == HTTP_TOO_MANY_REQUESTS {
            return Err(MetadataError::AntiBotChallenge);
        }

        if !status.is_success() {
            return Err(MetadataError::RequestFailed(format!(
                "{} returned HTTP {}",
                self.config.name, status
            )));
        }

        let raw_html = resp
            .text()
            .await
            .map_err(|e| MetadataError::RequestFailed(format!("failed to read body: {e}")))?;

        // Check for anti-bot challenge in HTML body
        if is_anti_bot_page(&raw_html) {
            return Err(MetadataError::AntiBotChallenge);
        }

        // Clean HTML for LLM
        let cleaned = clean_html_for_llm(&raw_html);
        if cleaned.is_empty() {
            return Ok(vec![]);
        }

        // Build LLM messages
        let messages = vec![
            LlmMessage {
                role: LlmRole::System,
                content: EXTRACTION_SYSTEM_PROMPT.to_string(),
            },
            LlmMessage {
                role: LlmRole::User,
                content: build_user_prompt(&cleaned, &base_url),
            },
        ];

        // Send to LLM
        let llm_response = self
            .llm
            .chat_completion(messages)
            .await
            .map_err(|e| match e {
                LlmError::Timeout(d) => MetadataError::Timeout(d),
                LlmError::RateLimited => MetadataError::RateLimited,
                LlmError::NotConfigured => MetadataError::NotConfigured,
                _ => MetadataError::RequestFailed(format!("LLM error: {e}")),
            })?;

        // Parse JSON response from LLM.
        // Extract the JSON array robustly: find outermost [ ... ] bounds.
        // This handles conversational filler, markdown fences, and explanatory text.
        let trimmed = llm_response.trim();
        let json_str = trimmed
            .find('[')
            .and_then(|start| trimmed.rfind(']').map(|end| &trimmed[start..=end]))
            .unwrap_or(trimmed);

        let parsed: Vec<LlmBookResult> = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                let snippet: String = llm_response.chars().take(500).collect();
                warn!(
                    provider = %self.config.name,
                    error = %e,
                    response_snippet = %snippet,
                    "LLM returned malformed JSON — treating as zero results"
                );
                // Malformed LLM JSON → zero results, NOT "Not Responding"
                return Ok(vec![]);
            }
        };

        // Convert to ProviderSearchResult, validating and normalizing
        let mut results = Vec::new();
        for item in parsed {
            let title = match item.title {
                Some(t) if !t.trim().is_empty() => t,
                _ => continue, // Skip entries without a title
            };

            let author_name = item.author.filter(|a| !a.trim().is_empty());

            let year = item.year.filter(|&y| is_valid_year(y));

            let cover_url = item
                .cover_url
                .as_deref()
                .and_then(|u| validate_cover_url(u, &base_url));

            // Validate detail URL: must be HTTPS and resolve relative paths.
            let detail_url = item.detail_url.and_then(|u| {
                let trimmed = u.trim();
                if trimmed.is_empty() {
                    return None;
                }
                // Resolve relative URLs against base
                if trimmed.starts_with('/') {
                    Some(format!("{}{}", base_url.trim_end_matches('/'), trimmed))
                } else if trimmed.starts_with("https://") {
                    Some(trimmed.to_string())
                } else {
                    None
                }
            });

            results.push(ProviderSearchResult {
                provider_key: String::new(),
                title: nfc(&title),
                author_name: author_name.map(|a| nfc(&a)),
                year,
                cover_url,
                isbn: None,
                publisher: None,
                source: self.config.name.clone(),
                source_type: "llm".to_string(),
                language: self.config.language.clone(),
                detail_url,
            });
        }

        Ok(results)
    }
}

impl<L: LlmClient + 'static> MetadataProvider for LlmScraperProvider<L> {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn search_works(&self, query: &str) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        tokio::time::timeout(Duration::from_secs(60), self.search_works_inner(query))
            .await
            .map_err(|_| MetadataError::Timeout(Duration::from_secs(60)))?
    }

    async fn search_authors(
        &self,
        _query: &str,
    ) -> Result<Vec<ProviderAuthorResult>, MetadataError> {
        Ok(vec![])
    }

    async fn fetch_work_detail(
        &self,
        _provider_key: &str,
    ) -> Result<ProviderWorkDetail, MetadataError> {
        Err(MetadataError::UnsupportedOperation)
    }
}

// =============================================================================
// Site Configs
// =============================================================================

/// Build LLM scraper configs for the scraped sites.
/// OPAC SBN (Italian) removed — site is client-rendered (Liferay CSR), not SSR.
/// Deferred until render proxy is available (same as Skoob/Brazil).
pub fn build_llm_scraper_configs() -> Vec<LlmScraperConfig> {
    let goodreads_url = "https://www.goodreads.com/search?q={query}";
    let goodreads_languages = ["fr", "de", "es", "nl", "it", "ja", "ko", "pl"];

    goodreads_languages
        .iter()
        .map(|lang| LlmScraperConfig {
            name: "Web Search".to_string(),
            search_url_template: goodreads_url.to_string(),
            language: lang.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_validation() {
        assert!(is_valid_year(2024));
        assert!(is_valid_year(1000));
        assert!(is_valid_year(2100));
        assert!(!is_valid_year(999));
        assert!(!is_valid_year(2101));
        assert!(!is_valid_year(0));
        assert!(!is_valid_year(-1));
    }

    #[test]
    fn llm_json_parsing_valid() {
        let json = r#"[{"title":"Wiedźmin","author":"Andrzej Sapkowski","year":1990,"cover_url":"https://example.com/cover.jpg"}]"#;
        let parsed: Vec<LlmBookResult> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title.as_deref(), Some("Wiedźmin"));
        assert_eq!(parsed[0].year, Some(1990));
    }

    #[test]
    fn llm_json_parsing_with_nulls() {
        let json = r#"[{"title":"Book","author":"Author","year":null,"cover_url":null}]"#;
        let parsed: Vec<LlmBookResult> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].year.is_none());
        assert!(parsed[0].cover_url.is_none());
    }

    #[test]
    fn llm_json_parsing_malformed() {
        let json = "This is not JSON at all";
        let result: Result<Vec<LlmBookResult>, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn build_url_encodes_query() {
        let config = LlmScraperConfig {
            name: "test".to_string(),
            search_url_template: "https://example.com/search?q={query}".to_string(),
            language: "pl".to_string(),
        };
        let provider = LlmScraperProvider {
            config,
            llm: Arc::new(MockLlm),
            http: HttpClient::builder().build().unwrap(),
        };
        let url = provider.build_url("wiedźmin");
        assert!(url.contains("wied%C5%BAmin"));
        assert!(!url.contains("{query}"));
    }

    #[test]
    fn configs_cover_llm_sites() {
        let configs = build_llm_scraper_configs();
        assert_eq!(configs.len(), 8);
        let langs: Vec<&str> = configs.iter().map(|c| c.language.as_str()).collect();
        assert!(langs.contains(&"fr"));
        assert!(langs.contains(&"de"));
        assert!(langs.contains(&"es"));
        assert!(langs.contains(&"nl"));
        assert!(langs.contains(&"it"));
        assert!(langs.contains(&"ja"));
        assert!(langs.contains(&"ko"));
        assert!(langs.contains(&"pl"));
    }

    struct MockLlm;
    impl LlmClient for MockLlm {
        async fn chat_completion(&self, _messages: Vec<LlmMessage>) -> Result<String, LlmError> {
            Ok("[]".to_string())
        }
    }
}
