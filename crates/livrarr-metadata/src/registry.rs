use std::collections::HashMap;
use std::sync::Arc;

use crate::http_llm::HttpLlmClient;
use crate::llm_scraper::{build_llm_scraper_configs, LlmScraperProvider};
use crate::sru::{build_sru_configs, SruProvider};
use crate::{MetadataError, MetadataProvider, ProviderSearchResult};
use livrarr_http::HttpClient;

/// A provider that can be SRU or LLM scraper.
/// Enum dispatch avoids the dyn-incompatibility of trait_variant async traits.
enum AnyProvider {
    Sru(SruProvider),
    LlmScraper(LlmScraperProvider<HttpLlmClient>),
}

impl AnyProvider {
    fn name(&self) -> &str {
        match self {
            AnyProvider::Sru(p) => p.name(),
            AnyProvider::LlmScraper(p) => p.name(),
        }
    }

    async fn search_works(&self, query: &str) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        match self {
            AnyProvider::Sru(p) => p.search_works(query).await,
            AnyProvider::LlmScraper(p) => p.search_works(query).await,
        }
    }
}

/// Maps language codes to metadata providers.
/// Built at startup, rebuilt atomically on config change.
pub struct ProviderRegistry {
    providers: HashMap<String, AnyProvider>,
    primary_language: String,
}

impl ProviderRegistry {
    /// Build registry from enabled languages and LLM availability.
    /// English is NOT in the registry — it uses the existing OL+Hardcover flow.
    pub fn build(
        languages: &[String],
        llm: Option<Arc<HttpLlmClient>>,
        http: HttpClient,
    ) -> Result<Self, MetadataError> {
        let primary = languages
            .first()
            .map(|s| s.as_str())
            .unwrap_or("en")
            .to_string();

        let mut providers: HashMap<String, AnyProvider> = HashMap::new();

        // Build SRU providers for enabled API languages
        for (config, mapper) in build_sru_configs() {
            let lang = config.language.clone();
            if languages.iter().any(|l| l == &lang) {
                providers.insert(
                    lang,
                    AnyProvider::Sru(SruProvider::new(config, mapper, http.clone())),
                );
            }
        }

        // Build LLM scraper providers for enabled LLM languages (only if LLM client available)
        if let Some(ref llm_client) = llm {
            for config in build_llm_scraper_configs() {
                let lang = config.language.clone();
                if languages.iter().any(|l| l == &lang) {
                    let provider =
                        LlmScraperProvider::new(config, Arc::clone(llm_client), http.clone());
                    providers.insert(lang, AnyProvider::LlmScraper(provider));
                }
            }
        }

        Ok(Self {
            providers,
            primary_language: primary,
        })
    }

    /// Search works for a specific language. Returns None for unknown/disabled or "en".
    pub async fn search(
        &self,
        lang: &str,
        query: &str,
    ) -> Option<Result<Vec<ProviderSearchResult>, MetadataError>> {
        let provider = self.providers.get(lang)?;
        Some(provider.search_works(query).await)
    }

    /// Get the provider name for a language. Returns None for unknown/disabled.
    pub fn provider_name(&self, lang: &str) -> Option<&str> {
        self.providers.get(lang).map(|p| p.name())
    }

    /// Returns true if a language has a registered provider.
    pub fn has_provider(&self, lang: &str) -> bool {
        self.providers.contains_key(lang)
    }

    /// Returns the primary language code (first element of the languages array).
    pub fn primary_language(&self) -> &str {
        &self.primary_language
    }

    /// List all registered language codes.
    pub fn languages(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

/// Returns true if LLM is fully configured (enabled + endpoint + key + model all non-empty).
pub fn is_llm_configured(
    llm_enabled: bool,
    llm_endpoint: Option<&str>,
    llm_api_key: Option<&str>,
    llm_model: Option<&str>,
) -> bool {
    llm_enabled
        && llm_endpoint.is_some_and(|s| !s.is_empty())
        && llm_api_key.is_some_and(|s| !s.is_empty())
        && llm_model.is_some_and(|s| !s.is_empty())
}

/// Build an LLM client from config fields, or None if not fully configured.
pub fn build_llm_client(
    http: &HttpClient,
    llm_enabled: bool,
    llm_endpoint: Option<&str>,
    llm_api_key: Option<&str>,
    llm_model: Option<&str>,
) -> Option<Arc<HttpLlmClient>> {
    if !is_llm_configured(llm_enabled, llm_endpoint, llm_api_key, llm_model) {
        return None;
    }
    Some(Arc::new(HttpLlmClient::new(
        http.clone(),
        llm_endpoint.unwrap().to_string(),
        llm_api_key.unwrap().to_string(),
        llm_model.unwrap().to_string(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_empty_for_english_only() {
        let http = HttpClient::builder().build().unwrap();
        let reg = ProviderRegistry::build(&["en".to_string()], None, http).unwrap();
        assert!(reg.provider_name("en").is_none());
        assert_eq!(reg.primary_language(), "en");
        assert!(reg.languages().is_empty());
    }

    #[test]
    fn registry_builds_sru_for_api_languages() {
        let http = HttpClient::builder().build().unwrap();
        let langs: Vec<String> = vec!["en", "fr", "de"]
            .into_iter()
            .map(String::from)
            .collect();
        let reg = ProviderRegistry::build(&langs, None, http).unwrap();
        assert!(reg.has_provider("fr"));
        assert!(reg.has_provider("de"));
        assert!(!reg.has_provider("en")); // English not in registry
        assert!(!reg.has_provider("pl")); // Not enabled
    }

    #[test]
    fn registry_skips_llm_langs_without_llm() {
        let http = HttpClient::builder().build().unwrap();
        let langs: Vec<String> = vec!["en", "pl", "fr"]
            .into_iter()
            .map(String::from)
            .collect();
        let reg = ProviderRegistry::build(&langs, None, http).unwrap();
        assert!(reg.has_provider("fr")); // SRU works
        assert!(!reg.has_provider("pl")); // LLM not available
    }

    #[test]
    fn primary_language_is_first() {
        let http = HttpClient::builder().build().unwrap();
        let langs: Vec<String> = vec!["fr", "en"].into_iter().map(String::from).collect();
        let reg = ProviderRegistry::build(&langs, None, http).unwrap();
        assert_eq!(reg.primary_language(), "fr");
    }

    #[test]
    fn is_llm_configured_checks_all_fields() {
        assert!(is_llm_configured(
            true,
            Some("http://localhost"),
            Some("key"),
            Some("model")
        ));
        assert!(!is_llm_configured(false, Some("x"), Some("x"), Some("x")));
        assert!(!is_llm_configured(true, None, Some("x"), Some("x")));
        assert!(!is_llm_configured(true, Some("x"), Some(""), Some("x")));
        assert!(!is_llm_configured(true, Some("x"), Some("x"), None));
    }
}
