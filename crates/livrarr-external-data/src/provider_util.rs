//! Stateless provider-substrate utilities (mechanism, not policy):
//! cover-URL upscaling, HTML cleaning for LLM extraction, anti-bot page
//! detection, and cover-URL SSRF validation. Relocated from cover.rs and
//! llm_scraper.rs; bound for livrarr-external-data.

use regex::Regex;
use std::sync::LazyLock;
use url::Url;

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

static RE_GR_SIZE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\._S[A-Z0-9_,]+_\.").unwrap());

pub fn upscale_cover_url(url: &str) -> String {
    if url.contains("gr-assets.com") || url.contains("goodreads.com") {
        RE_GR_SIZE.replace(url, ".").into_owned()
    } else {
        url.to_string()
    }
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
}
