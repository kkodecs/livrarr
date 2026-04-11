//! SSRF (Server-Side Request Forgery) protection.
//!
//! Provides URL validation that rejects requests to private, loopback,
//! link-local, and other reserved IP ranges. Use `validate_url()` on any
//! user-supplied URL before fetching it server-side.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Errors from SSRF validation.
#[derive(Debug, thiserror::Error)]
pub enum SsrfError {
    #[error("URL scheme must be http or https")]
    InvalidScheme,
    #[error("URL has no host")]
    NoHost,
    #[error("URL resolves to a private or reserved IP address")]
    PrivateIp,
    #[error("DNS resolution failed: {0}")]
    DnsError(String),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
}

/// Check whether an IP address is private, loopback, link-local, or otherwise
/// reserved and should not be reachable from user-supplied URLs.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                // 127.0.0.0/8
            || v4.is_private()              // 10/8, 172.16/12, 192.168/16
            || v4.is_link_local()           // 169.254.0.0/16
            || v4.is_broadcast()            // 255.255.255.255
            || v4.is_unspecified()          // 0.0.0.0
            // CGNAT (100.64.0.0/10)
            || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
            // IETF protocol assignments (192.0.0.0/24)
            || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0)
            // Benchmarking (198.18.0.0/15)
            || (v4.octets()[0] == 198 && (v4.octets()[1] & 0xFE) == 18)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                // ::1
            || v6.is_unspecified()          // ::
            // Unique local (fc00::/7)
            || (v6.segments()[0] & 0xfe00) == 0xfc00
            // Link-local (fe80::/10)
            || (v6.segments()[0] & 0xffc0) == 0xfe80
            // IPv4-mapped (::ffff:0:0/96) — check the embedded v4
            || v6.to_ipv4_mapped().is_some_and(|v4| is_private_ip(IpAddr::V4(v4)))
        }
    }
}

/// Validate a URL for SSRF safety before making a server-side request.
///
/// 1. Scheme must be `http` or `https`
/// 2. Host must be present
/// 3. Literal IP hosts are checked directly
/// 4. Hostnames are resolved via DNS; rejected if *any* resolved IP is private
///    (prevents dual-stack DNS rebinding)
pub async fn validate_url(url: &str) -> Result<(), SsrfError> {
    // Allow magnet links through — they're not HTTP fetches.
    if url.starts_with("magnet:") {
        return Ok(());
    }

    let parsed = reqwest::Url::parse(url).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;

    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(SsrfError::InvalidScheme),
    }

    let host = parsed.host_str().ok_or(SsrfError::NoHost)?;
    let port = parsed.port_or_known_default().unwrap_or(80);

    // Literal IP — check directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_private_ip(ip) {
            Err(SsrfError::PrivateIp)
        } else {
            Ok(())
        };
    }

    // Hostname — resolve and check every address.
    let addrs: Vec<_> = tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| SsrfError::DnsError(e.to_string()))?
        .collect();

    if addrs.is_empty() {
        return Err(SsrfError::DnsError("no addresses resolved".into()));
    }

    // Reject if ANY resolved address is private. An attacker can add one
    // public A record alongside a private one (dual-stack rebinding).
    for addr in &addrs {
        if is_private_ip(addr.ip()) {
            return Err(SsrfError::PrivateIp);
        }
    }

    Ok(())
}

/// Custom DNS resolver that rejects private/reserved IPs at connection time.
///
/// Wraps the standard resolver and filters results. This prevents:
/// - DNS rebinding (TOCTOU between validate_url and connect)
/// - Redirect-based SSRF (reqwest re-resolves on redirect targets)
///
/// Attach to a `reqwest::ClientBuilder` via `.dns_resolver(SsrfSafeResolver::new())`.
pub struct SsrfSafeResolver;

impl SsrfSafeResolver {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl reqwest::dns::Resolve for SsrfSafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str();
            // Resolve via standard DNS.
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host(format!("{host}:0"))
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
                .collect();

            if addrs.is_empty() {
                return Err("DNS resolution returned no addresses".into());
            }

            // Filter out private IPs. If ALL are private, reject.
            let safe: Vec<SocketAddr> = addrs
                .into_iter()
                .filter(|a| !is_private_ip(a.ip()))
                .collect();

            if safe.is_empty() {
                return Err("all resolved addresses are private/reserved".into());
            }

            Ok(Box::new(safe.into_iter()) as Box<dyn Iterator<Item = SocketAddr> + Send>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn private_ipv4() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
    }

    #[test]
    fn public_ipv4() {
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
    }

    #[test]
    fn private_ipv6() {
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        // fc00::/7
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        // fe80::/10
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        // IPv4-mapped ::ffff:127.0.0.1
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001
        ))));
    }

    #[tokio::test]
    async fn rejects_private_ip_urls() {
        assert!(matches!(
            validate_url("http://127.0.0.1/secret").await,
            Err(SsrfError::PrivateIp)
        ));
        assert!(matches!(
            validate_url("http://169.254.169.254/latest/meta-data/").await,
            Err(SsrfError::PrivateIp)
        ));
        assert!(matches!(
            validate_url("http://192.168.1.1/admin").await,
            Err(SsrfError::PrivateIp)
        ));
        assert!(matches!(
            validate_url("http://10.0.0.1:8080/").await,
            Err(SsrfError::PrivateIp)
        ));
    }

    #[tokio::test]
    async fn rejects_bad_scheme() {
        assert!(matches!(
            validate_url("ftp://example.com/file").await,
            Err(SsrfError::InvalidScheme)
        ));
        assert!(matches!(
            validate_url("file:///etc/passwd").await,
            Err(SsrfError::InvalidScheme)
        ));
        assert!(matches!(
            validate_url("gopher://example.com").await,
            Err(SsrfError::InvalidScheme)
        ));
    }

    #[tokio::test]
    async fn allows_magnet_links() {
        assert!(validate_url("magnet:?xt=urn:btih:abc123").await.is_ok());
    }
}
