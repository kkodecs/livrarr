mod mappers;

pub use mappers::{BneFieldMapper, BnfFieldMapper, DnbFieldMapper, KbFieldMapper, NdlFieldMapper};

use crate::cover::resolve_cover_foreign;
use crate::normalize::nfc;
use crate::{
    MetadataError, MetadataProvider, ProviderAuthorResult, ProviderSearchResult, ProviderWorkDetail,
};
use livrarr_http::HttpClient;
use std::time::Duration;

/// Extracts structured metadata fields from SRU XML response bytes.
pub trait FieldMapper: Send + Sync {
    fn name(&self) -> &str;
    fn parse_search_results(&self, xml: &[u8]) -> Result<Vec<ProviderSearchResult>, MetadataError>;
}

pub struct SruConfig {
    pub name: String,
    pub base_url: String,
    pub query_template: String,
    pub max_records: u32,
    pub language: String,
    /// SRU protocol version. Most use "1.2", DNB requires "1.1".
    pub sru_version: String,
}

pub struct SruProvider {
    config: SruConfig,
    mapper: Box<dyn FieldMapper>,
    http: HttpClient,
}

impl SruProvider {
    pub fn new(config: SruConfig, mapper: Box<dyn FieldMapper>, http: HttpClient) -> Self {
        Self {
            config,
            mapper,
            http,
        }
    }

    fn build_url(&self, query: &str) -> String {
        let cql = self.config.query_template.replace("{query}", query);
        let encoded_cql = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("version", &self.config.sru_version)
            .append_pair("operation", "searchRetrieve")
            .append_pair("query", &cql)
            .append_pair("maximumRecords", &self.config.max_records.to_string())
            .finish();
        format!("{}?{}", self.config.base_url, encoded_cql)
    }
}

impl MetadataProvider for SruProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn search_works(&self, query: &str) -> Result<Vec<ProviderSearchResult>, MetadataError> {
        let url = self.build_url(query);

        let resp = tokio::time::timeout(Duration::from_secs(10), self.http.get(&url).send())
            .await
            .map_err(|_| MetadataError::Timeout(Duration::from_secs(10)))?
            .map_err(|e| MetadataError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(MetadataError::RequestFailed(format!(
                "SRU returned HTTP {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| MetadataError::RequestFailed(format!("failed to read body: {e}")))?;

        let mut results = self.mapper.parse_search_results(&bytes)?;

        // NFC normalize all string fields and resolve covers
        for result in &mut results {
            result.title = nfc(&result.title);
            if let Some(ref a) = result.author_name {
                result.author_name = Some(nfc(a));
            }
            if let Some(ref p) = result.publisher {
                result.publisher = Some(nfc(p));
            }

            // Resolve cover: Amazon direct ISBN URL
            if result.cover_url.is_none() {
                result.cover_url = resolve_cover_foreign(&self.http, result.isbn.as_deref()).await;
            }
        }

        Ok(results)
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

/// Build SRU configs for all five national libraries.
pub fn build_sru_configs() -> Vec<(SruConfig, Box<dyn FieldMapper>)> {
    vec![
        (
            SruConfig {
                name: "BNE".to_string(),
                base_url: "https://catalogo.bne.es/view/sru/34BNE_INST".to_string(),
                query_template: "alma.title all \"{query}\"".to_string(),
                max_records: 20,
                language: "es".to_string(),
                sru_version: "1.2".to_string(),
            },
            Box::new(BneFieldMapper),
        ),
        (
            SruConfig {
                name: "BnF".to_string(),
                base_url: "https://catalogue.bnf.fr/api/SRU".to_string(),
                query_template: "bib.title all \"{query}\"".to_string(),
                max_records: 20,
                language: "fr".to_string(),
                sru_version: "1.2".to_string(),
            },
            Box::new(BnfFieldMapper),
        ),
        (
            SruConfig {
                name: "DNB".to_string(),
                base_url: "https://services.dnb.de/sru/dnb".to_string(),
                query_template: "title={query}".to_string(),
                max_records: 20,
                language: "de".to_string(),
                sru_version: "1.1".to_string(),
            },
            Box::new(DnbFieldMapper),
        ),
        // KB (Dutch) removed — returns news bulletins, not books. Replaced with Amazon.nl LLM scrape.
        (
            SruConfig {
                name: "NDL".to_string(),
                base_url: "https://ndlsearch.ndl.go.jp/api/sru".to_string(),
                query_template: "title=\"{query}\"".to_string(),
                max_records: 20,
                language: "ja".to_string(),
                sru_version: "1.2".to_string(),
            },
            Box::new(NdlFieldMapper),
        ),
    ]
}
