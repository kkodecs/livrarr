use axum::http::Request;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tower_governor::key_extractor::KeyExtractor;

/// Extracts client IP for rate limiting. Only trusts proxy headers when the
/// TCP peer is in the configured trusted_proxies list.
/// Default (empty list): uses peer IP only — safe for direct exposure.
#[derive(Clone)]
pub struct SmartIpKeyExtractor {
    pub trusted_proxies: Arc<Vec<IpNet>>,
}

/// Simple CIDR network for trusted proxy matching.
#[derive(Clone, Debug)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix_len: u8,
}

impl IpNet {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(test)) => {
                if self.prefix_len >= 32 {
                    return net == test;
                }
                let mask = u32::MAX << (32 - self.prefix_len);
                (u32::from(net) & mask) == (u32::from(test) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(test)) => {
                if self.prefix_len >= 128 {
                    return net == test;
                }
                let net_bits = u128::from(net);
                let test_bits = u128::from(test);
                let mask = u128::MAX << (128 - self.prefix_len);
                (net_bits & mask) == (test_bits & mask)
            }
            _ => false,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        if let Some((addr_str, prefix_str)) = s.split_once('/') {
            let addr = addr_str.parse().ok()?;
            let prefix_len = prefix_str.parse().ok()?;
            Some(Self { addr, prefix_len })
        } else {
            let addr: IpAddr = s.parse().ok()?;
            let prefix_len = if addr.is_ipv4() { 32 } else { 128 };
            Some(Self { addr, prefix_len })
        }
    }
}

impl SmartIpKeyExtractor {
    pub fn new(trusted_proxies: Vec<IpNet>) -> Self {
        Self {
            trusted_proxies: Arc::new(trusted_proxies),
        }
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted_proxies.iter().any(|net| net.contains(ip))
    }

    fn extract_from_xff(&self, header_value: &str, peer_ip: IpAddr) -> IpAddr {
        // Walk right-to-left, stripping trusted proxies, return first untrusted.
        for entry in header_value.rsplit(',') {
            let trimmed = entry.trim();
            if let Ok(ip) = trimmed.parse::<IpAddr>() {
                if !self.is_trusted(ip) {
                    return ip;
                }
            }
        }
        peer_ip
    }
}

impl KeyExtractor for SmartIpKeyExtractor {
    type Key = IpAddr;

    fn extract<B>(&self, req: &Request<B>) -> Result<Self::Key, tower_governor::GovernorError> {
        let peer_ip = req
            .extensions()
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
            .ok_or(tower_governor::GovernorError::UnableToExtractKey)?;

        // Only trust proxy headers if the peer is in our trusted list.
        if !self.is_trusted(peer_ip) {
            return Ok(peer_ip);
        }

        // Peer is trusted — extract real client IP from headers.
        if let Some(val) = req.headers().get("x-real-ip") {
            if let Ok(s) = val.to_str() {
                if let Ok(ip) = s.trim().parse::<IpAddr>() {
                    return Ok(ip);
                }
            }
        }

        if let Some(val) = req.headers().get("x-forwarded-for") {
            if let Ok(s) = val.to_str() {
                return Ok(self.extract_from_xff(s, peer_ip));
            }
        }

        Ok(peer_ip)
    }
}
