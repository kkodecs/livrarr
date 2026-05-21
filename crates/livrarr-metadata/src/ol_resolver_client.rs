use crate::english_identity_resolver::{CircuitState, OlError, OlSearchHit, OpenLibraryClient};
use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use std::time::Duration;

pub struct LiveOlResolverClient<H> {
    http: H,
}

impl<H> LiveOlResolverClient<H> {
    pub fn new(http: H) -> Self {
        Self { http }
    }
}

impl<H: HttpFetcher + Send + Sync> OpenLibraryClient for LiveOlResolverClient<H> {
    fn circuit_state(&self) -> CircuitState {
        CircuitState::Closed
    }

    async fn isbn_to_work(&self, isbn: &str) -> Result<Option<String>, OlError> {
        let url = format!("https://openlibrary.org/isbn/{isbn}.json");
        let resp = self
            .http
            .fetch(FetchRequest {
                url,
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(10),
                rate_bucket: RateBucket::OpenLibrary,
                max_body_bytes: 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
            })
            .await
            .map_err(|e| OlError::Transient(e.to_string()))?;

        if resp.status != 200 {
            return Ok(None);
        }

        let data: serde_json::Value =
            serde_json::from_slice(&resp.body).map_err(|e| OlError::Transient(e.to_string()))?;

        let ol_key = data
            .get("works")
            .and_then(|w| w.as_array())
            .and_then(|a| a.first())
            .and_then(|w| w.get("key"))
            .and_then(|k| k.as_str())
            .map(|k| k.trim_start_matches("/works/").to_string());

        Ok(ol_key)
    }

    async fn search_works(
        &self,
        title: &str,
        author: &str,
        limit: u32,
    ) -> Result<Vec<OlSearchHit>, OlError> {
        let q = format!("{title} {author}");
        let encoded = urlencoding::encode(&q);
        let url = format!(
            "https://openlibrary.org/search.json?q={encoded}&limit={limit}&fields=key,title,author_name,first_publish_year,isbn"
        );

        let resp = self
            .http
            .fetch(FetchRequest {
                url,
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(10),
                rate_bucket: RateBucket::OpenLibrary,
                max_body_bytes: 2 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
            })
            .await
            .map_err(|e| OlError::Transient(e.to_string()))?;

        if resp.status != 200 {
            return Err(OlError::Transient(format!("OL returned {}", resp.status)));
        }

        let data: serde_json::Value =
            serde_json::from_slice(&resp.body).map_err(|e| OlError::Transient(e.to_string()))?;

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let hits = docs
            .iter()
            .filter_map(|doc| {
                let key = doc.get("key")?.as_str()?;
                let ol_key = key.trim_start_matches("/works/").to_string();
                let title = doc
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let author_combined = doc
                    .get("author_name")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string();
                let first_publish_year = doc
                    .get("first_publish_year")
                    .and_then(|y| y.as_i64())
                    .map(|y| y as i32);
                let isbn = doc
                    .get("isbn")
                    .and_then(|i| i.as_array())
                    .and_then(|a| a.first())
                    .and_then(|i| i.as_str())
                    .map(|s| s.to_string());
                Some(OlSearchHit {
                    ol_key,
                    title,
                    author_combined,
                    first_publish_year,
                    isbn,
                })
            })
            .collect();

        Ok(hits)
    }
}
