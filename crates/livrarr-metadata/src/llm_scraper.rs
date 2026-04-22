use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use std::sync::LazyLock;
use tracing::warn;
use url::Url;

use crate::normalize::nfc;
use crate::{
    LlmClient, LlmError, LlmMessage, LlmRole, MetadataError, MetadataProvider,
    ProviderAuthorResult, ProviderSearchResult, ProviderWorkDetail,
};
use livrarr_http::HttpClient;

/// HTTP status codes for anti-bot detection (avoids direct reqwest dep).
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;

// =============================================================================
// HTML Cleaner
// =============================================================================

/// Maximum size for cleaned HTML sent to LLM (~100KB).
const MAX_HTML_BYTES: usize = 100_000;

static RE_COMMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?si)<!--.*?-->").unwrap());
static RE_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<script[^>]*>.*?</script>").unwrap());
static RE_STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<style[^>]*>.*?</style>").unwrap());
static RE_NAV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?si)<nav[^>]*>.*?</nav>").unwrap());
static RE_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<header[^>]*>.*?</header>").unwrap());
static RE_FOOTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<footer[^>]*>.*?</footer>").unwrap());
/// Extract src/data-src from img tags, stripping other attributes.
static RE_IMG_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<img\s[^>]*?>"#).unwrap());
static RE_IMG_SRC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:data-)?src="([^"]*)""#).unwrap());
/// Extract href from anchor opening tags, preserving links for detail URL extraction.
static RE_A_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<a\s[^>]*>"#).unwrap());
static RE_A_HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="([^"]*)""#).unwrap());
/// Strip all attributes from non-img opening tags.
static RE_ATTRS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<(\w+)\s+[^>]*>").unwrap());
static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());

/// Anti-bot challenge indicators in HTML body.
const ANTI_BOT_INDICATORS: &[&str] = &[
    "cf-browser-verification",
    "cf-challenge-platform",
    "challenge-form",
    "recaptcha",
    "hcaptcha",
    "g-recaptcha",
    "cdn-cgi/challenge-platform",
    "just a moment",
    "checking your browser",
];

/// Strip HTML to essential content for LLM extraction.
/// Removes scripts, styles, nav, header, footer, comments.
/// Preserves `src` on `<img>` tags and `href` on `<a>` tags.
/// Removes all other attributes. Collapses whitespace.
/// Truncates at ~100KB at nearest closing tag.
pub fn clean_html_for_llm(raw_html: &str) -> String {
    let mut html = RE_COMMENTS.replace_all(raw_html, "").into_owned();
    html = RE_SCRIPT.replace_all(&html, "").into_owned();
    html = RE_STYLE.replace_all(&html, "").into_owned();
    html = RE_NAV.replace_all(&html, "").into_owned();
    html = RE_HEADER.replace_all(&html, "").into_owned();
    html = RE_FOOTER.replace_all(&html, "").into_owned();

    // Simplify img tags: keep only the src/data-src URL for cover extraction.
    // Use a placeholder to protect from the subsequent attribute stripping pass.
    let mut img_counter = 0u32;
    let mut img_map: Vec<String> = Vec::new();
    html = RE_IMG_TAG
        .replace_all(&html, |caps: &regex::Captures| {
            if let Some(src) = RE_IMG_SRC.captures(&caps[0]) {
                let placeholder = format!("__IMG{img_counter}__");
                img_map.push(format!(r#"<img src="{}">"#, &src[1]));
                img_counter += 1;
                placeholder
            } else {
                String::new()
            }
        })
        .into_owned();
    // Simplify anchor tags: keep only href for detail URL extraction.
    let mut a_counter = 0u32;
    let mut a_map: Vec<String> = Vec::new();
    html = RE_A_OPEN
        .replace_all(&html, |caps: &regex::Captures| {
            if let Some(href) = RE_A_HREF.captures(&caps[0]) {
                let placeholder = format!("__LINK{a_counter}__");
                a_map.push(format!(r#"<a href="{}">"#, &href[1]));
                a_counter += 1;
                placeholder
            } else {
                "<a>".to_string()
            }
        })
        .into_owned();
    // Strip all attributes from remaining tags.
    html = RE_ATTRS.replace_all(&html, "<$1>").into_owned();
    // Restore img and anchor tags from placeholders.
    for (i, img_html) in img_map.iter().enumerate() {
        html = html.replace(&format!("__IMG{i}__"), img_html);
    }
    for (i, a_html) in a_map.iter().enumerate() {
        html = html.replace(&format!("__LINK{i}__"), a_html);
    }

    // Collapse whitespace
    html = RE_WHITESPACE.replace_all(&html, " ").into_owned();
    html = html.trim().to_string();

    // Truncate at ~100KB at nearest closing tag.
    // Use floor_char_boundary to avoid panic on multi-byte UTF-8 (CJK, Polish, etc).
    if html.len() > MAX_HTML_BYTES {
        let safe_len = html.floor_char_boundary(MAX_HTML_BYTES);
        if let Some(pos) = html[..safe_len].rfind("</") {
            if let Some(end) = html[pos..].find('>') {
                html.truncate(pos + end + 1);
            } else {
                html.truncate(safe_len);
            }
        } else {
            html.truncate(safe_len);
        }
    }

    html
}

// =============================================================================
// Anti-bot Detection
// =============================================================================

/// Returns true if the HTML body looks like an anti-bot challenge page.
/// Only triggers for small pages (< 10KB) where the indicator dominates,
/// not for large pages that happen to contain the phrase incidentally.
pub fn is_anti_bot_page(html: &str) -> bool {
    if html.len() > 10_000 {
        return false;
    }
    let lower = html.to_lowercase();
    ANTI_BOT_INDICATORS
        .iter()
        .any(|indicator| lower.contains(indicator))
}

// =============================================================================
// Cover URL Validation
// =============================================================================

/// Validate and resolve a cover URL. Returns None if invalid or SSRF risk.
/// Public so it can be reused in the add-work path.
pub fn validate_cover_url(raw_url: &str, base_url: &str) -> Option<String> {
    // Resolve relative URLs against base
    let resolved = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
        raw_url.to_string()
    } else {
        let base = Url::parse(base_url).ok()?;
        base.join(raw_url).ok()?.to_string()
    };

    // Parse and validate
    let parsed = Url::parse(&resolved).ok()?;

    // Must be http or https
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }

    // SSRF prevention: block private/loopback/link-local IPs using typed host parsing.
    // This handles decimal, octal, hex IP encodings via the url crate's normalization.
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => {
            if ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                // 100.64.0.0/10 (CGNAT)
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64)
            {
                return None;
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            if ip.is_loopback() || ip.is_unspecified() {
                return None;
            }
            // Block ULA (fc00::/7), link-local (fe80::/10)
            let segs = ip.segments();
            if (segs[0] & 0xFE00) == 0xFC00 || (segs[0] & 0xFFC0) == 0xFE80 {
                return None;
            }
            // Block IPv4-mapped IPv6 (::ffff:x.x.x.x) — extract the inner IPv4
            // and run it through the same private/loopback/link-local checks.
            if let Some(v4) = ip.to_ipv4_mapped() {
                if v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                {
                    return None;
                }
            }
        }
        Some(url::Host::Domain(d)) => {
            let lower = d.to_lowercase();
            if lower == "localhost" || lower.ends_with(".local") {
                return None;
            }
        }
        None => return None,
    }

    Some(resolved)
}

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
    let goodreads_languages = ["fr", "de", "es", "nl", "it", "ja", "ko"];

    let mut configs: Vec<LlmScraperConfig> = goodreads_languages
        .iter()
        .map(|lang| LlmScraperConfig {
            name: "Web Search".to_string(),
            search_url_template: goodreads_url.to_string(),
            language: lang.to_string(),
        })
        .collect();

    // Native-language sites with distinct routing
    configs.push(LlmScraperConfig {
        name: "lubimyczytac.pl".to_string(),
        search_url_template: "https://lubimyczytac.pl/szukaj/ksiazki?phrase={query}".to_string(),
        language: "pl".to_string(),
    });

    configs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_html_strips_scripts_and_styles() {
        let html = r#"<html><head><style>body{color:red}</style></head><body>
            <script>alert('xss')</script>
            <div class="content">Hello World</div>
        </body></html>"#;
        let cleaned = clean_html_for_llm(html);
        assert!(!cleaned.contains("alert"));
        assert!(!cleaned.contains("color:red"));
        assert!(cleaned.contains("Hello World"));
    }

    #[test]
    fn clean_html_strips_nav_header_footer() {
        let html = r#"<nav id="menu">Navigation</nav>
            <header>Site Header</header>
            <main>Content</main>
            <footer>Site Footer</footer>"#;
        let cleaned = clean_html_for_llm(html);
        assert!(!cleaned.contains("Navigation"));
        assert!(!cleaned.contains("Site Header"));
        assert!(!cleaned.contains("Site Footer"));
        assert!(cleaned.contains("Content"));
    }

    #[test]
    fn clean_html_removes_attributes_from_non_img() {
        let html = r#"<div class="foo" id="bar" data-x="y">Text</div>"#;
        let cleaned = clean_html_for_llm(html);
        assert!(cleaned.contains("<div>"));
        assert!(!cleaned.contains("class="));
        assert!(cleaned.contains("Text"));
    }

    #[test]
    fn clean_html_preserves_img_src() {
        let html = r#"<img class="cover" src="https://example.com/cover.jpg" alt="book">"#;
        let cleaned = clean_html_for_llm(html);
        assert!(cleaned.contains("src=\"https://example.com/cover.jpg\""));
        assert!(!cleaned.contains("class="));
        assert!(!cleaned.contains("alt="));
    }

    #[test]
    fn clean_html_collapses_whitespace() {
        let html = "Hello     \n\n\n   World";
        let cleaned = clean_html_for_llm(html);
        assert_eq!(cleaned, "Hello World");
    }

    #[test]
    fn clean_html_truncates_at_100kb() {
        let mut html = String::new();
        for i in 0..20_000 {
            html.push_str(&format!("<div>Entry {i}</div>"));
        }
        let cleaned = clean_html_for_llm(&html);
        assert!(cleaned.len() <= MAX_HTML_BYTES + 10); // small margin for closing tag
        assert!(cleaned.ends_with("</div>"));
    }

    #[test]
    fn clean_html_strips_comments() {
        let html = "<!-- secret comment --><p>Visible</p>";
        let cleaned = clean_html_for_llm(html);
        assert!(!cleaned.contains("secret"));
        assert!(cleaned.contains("Visible"));
    }

    #[test]
    fn anti_bot_detects_cloudflare() {
        assert!(is_anti_bot_page(
            "<html><body>Checking your browser before accessing</body></html>"
        ));
        assert!(is_anti_bot_page(
            "<div id=\"cf-browser-verification\">Please wait</div>"
        ));
    }

    #[test]
    fn anti_bot_passes_normal_html() {
        assert!(!is_anti_bot_page(
            "<html><body><div>Book results</div></body></html>"
        ));
    }

    #[test]
    fn validate_cover_url_allows_https() {
        let result = validate_cover_url("https://example.com/cover.jpg", "https://example.com");
        assert_eq!(result, Some("https://example.com/cover.jpg".to_string()));
    }

    #[test]
    fn validate_cover_url_resolves_relative() {
        let result = validate_cover_url("/images/cover.jpg", "https://example.com");
        assert_eq!(
            result,
            Some("https://example.com/images/cover.jpg".to_string())
        );
    }

    #[test]
    fn validate_cover_url_blocks_localhost() {
        assert!(validate_cover_url("http://localhost/img.jpg", "https://example.com").is_none());
        assert!(validate_cover_url("http://127.0.0.1/img.jpg", "https://example.com").is_none());
    }

    #[test]
    fn validate_cover_url_blocks_private_ips() {
        assert!(validate_cover_url("http://192.168.1.1/img.jpg", "https://example.com").is_none());
        assert!(validate_cover_url("http://10.0.0.1/img.jpg", "https://example.com").is_none());
        // Full 172.16.0.0/12 range
        assert!(validate_cover_url("http://172.20.0.1/img.jpg", "https://example.com").is_none());
        assert!(
            validate_cover_url("http://172.31.255.255/img.jpg", "https://example.com").is_none()
        );
        // AWS metadata endpoint (link-local)
        assert!(validate_cover_url(
            "http://169.254.169.254/latest/meta-data/",
            "https://example.com"
        )
        .is_none());
    }

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
