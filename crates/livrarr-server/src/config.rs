//! Bootstrap configuration (TOML).
//!
//! Satisfies: RUNTIME-CONFIG-001, RUNTIME-CONFIG-002, RUNTIME-CONFIG-003,
//!            RUNTIME-COMPOSE-004, RUNTIME-LOG-001, RUNTIME-LOG-002

use serde::Deserialize;
use tracing::warn;

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

/// Bootstrap configuration read from {data-dir}/config.toml.
///
/// Satisfies: RUNTIME-CONFIG-001, RUNTIME-CONFIG-002, RUNTIME-CONFIG-003
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub log: LogConfig,

    #[serde(default)]
    pub convergence: ConvergenceConfig,

    #[serde(default)]
    pub metadata_cache: MetadataCacheConfig,

    #[serde(default)]
    pub author_link: AuthorLinkConfig,
}

// ---------------------------------------------------------------------------
// ServerConfig
// ---------------------------------------------------------------------------

/// [server] section.
///
/// Satisfies: RUNTIME-SERVER-003, RUNTIME-COMPOSE-004
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default)]
    pub url_base: String,

    #[serde(default = "default_trusted_proxies")]
    pub trusted_proxies: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
            url_base: String::new(),
            trusted_proxies: default_trusted_proxies(),
        }
    }
}

fn default_trusted_proxies() -> Vec<String> {
    Vec::new()
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8789
}

// ---------------------------------------------------------------------------
// AuthConfig
// ---------------------------------------------------------------------------

/// [auth] section.
///
/// Satisfies: AUTH-009 (external auth)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthConfig {
    pub external_header: Option<String>,

    #[serde(default)]
    pub trusted_proxies: Vec<String>,
}

// ---------------------------------------------------------------------------
// LogConfig, LogLevel, LogFormat
// ---------------------------------------------------------------------------

/// [log] section.
///
/// Satisfies: RUNTIME-LOG-001, RUNTIME-LOG-002
#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: LogLevel,

    #[serde(default = "default_log_format")]
    pub format: LogFormat,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

fn default_log_level() -> LogLevel {
    LogLevel::Info
}

fn default_log_format() -> LogFormat {
    LogFormat::Text
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

// ---------------------------------------------------------------------------
// ConvergenceConfig
// ---------------------------------------------------------------------------

/// [convergence] section — the background identity/enrichment completion sweep.
///
/// Enabled by default; `[convergence] enabled = false` opts out. Provider
/// calls ride the outbound queue at Low priority behind the circuit breakers,
/// so the sweep yields to interactive traffic (the REQ-007 volume guard).
/// `interval_secs` is both the tick cadence and the back-off applied
/// to a still-incomplete work's next due time; `batch_size` caps how many works
/// one tick processes per user; `attempt_threshold` is the per-anchor dead-end
/// limit (a missing anchor is abandoned after this many failed chase attempts).
#[derive(Debug, Clone, Deserialize)]
pub struct ConvergenceConfig {
    #[serde(default = "default_convergence_enabled")]
    pub enabled: bool,

    #[serde(default = "default_convergence_interval_secs")]
    pub interval_secs: u64,

    #[serde(default = "default_convergence_batch_size")]
    pub batch_size: i64,

    #[serde(default = "default_convergence_attempt_threshold")]
    pub attempt_threshold: u32,
}

impl Default for ConvergenceConfig {
    fn default() -> Self {
        Self {
            enabled: default_convergence_enabled(),
            interval_secs: default_convergence_interval_secs(),
            batch_size: default_convergence_batch_size(),
            attempt_threshold: default_convergence_attempt_threshold(),
        }
    }
}

fn default_convergence_enabled() -> bool {
    true
}

fn default_convergence_interval_secs() -> u64 {
    3600
}

fn default_convergence_batch_size() -> i64 {
    25
}

fn default_convergence_attempt_threshold() -> u32 {
    3
}

// ---------------------------------------------------------------------------
// MetadataCacheConfig
// ---------------------------------------------------------------------------

/// [metadata_cache] section — the persistent provider-response cache (REQ-009).
///
/// Background metadata flows (convergence, re-adds, list import, monitors)
/// serve provider detail payloads from this cache while fresh; the user's
/// per-work Refresh and Refresh All bypass it and overwrite entries. TOML
/// only — no environment-variable override path exists.
#[derive(Debug, Clone, Deserialize)]
pub struct MetadataCacheConfig {
    #[serde(default = "default_metadata_cache_ttl_days")]
    pub ttl_days: u64,

    #[serde(default = "default_metadata_cache_max_rows")]
    pub max_rows: i64,
}

impl Default for MetadataCacheConfig {
    fn default() -> Self {
        Self {
            ttl_days: default_metadata_cache_ttl_days(),
            max_rows: default_metadata_cache_max_rows(),
        }
    }
}

fn default_metadata_cache_ttl_days() -> u64 {
    7
}

fn default_metadata_cache_max_rows() -> i64 {
    100_000
}

// ---------------------------------------------------------------------------
// AuthorLinkConfig
// ---------------------------------------------------------------------------

/// [author_link] section — the background author-provider linking sweep.
///
/// Enabled by default; `[author_link] enabled = false` opts out and the recurring
/// tick then does nothing (user actions still enqueue, so the work is not lost).
/// Every provider call the sweep makes rides the outbound queue at Low priority,
/// so it yields to interactive traffic. `interval_secs` is the tick cadence;
/// `batch_size` caps how many authors one tick claims, which is what keeps a tick
/// interruptible and resumable. TOML only — no environment-variable override.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorLinkConfig {
    #[serde(default = "default_author_link_enabled")]
    pub enabled: bool,

    #[serde(default = "default_author_link_interval_secs")]
    pub interval_secs: u64,

    #[serde(default = "default_author_link_batch_size")]
    pub batch_size: i64,
}

impl Default for AuthorLinkConfig {
    fn default() -> Self {
        Self {
            enabled: default_author_link_enabled(),
            interval_secs: default_author_link_interval_secs(),
            batch_size: default_author_link_batch_size(),
        }
    }
}

fn default_author_link_enabled() -> bool {
    true
}

fn default_author_link_interval_secs() -> u64 {
    900
}

fn default_author_link_batch_size() -> i64 {
    25
}

// ---------------------------------------------------------------------------
// ConfigError
// ---------------------------------------------------------------------------

/// Config validation errors — fatal at startup.
///
/// Satisfies: RUNTIME-CONFIG-003
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file parse error: {message}")]
    ParseError { message: String },

    #[error("invalid config value: {field}: {message}")]
    InvalidValue { field: String, message: String },

    #[error("I/O error: {message}")]
    Io { message: String },
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate AppConfig after deserialization.
///
/// Satisfies: RUNTIME-CONFIG-003
pub fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
    // port must be 1..=65535 (u16 prevents >65535, but 0 is invalid)
    if config.server.port == 0 {
        return Err(ConfigError::InvalidValue {
            field: "server.port".to_string(),
            message: "port must be between 1 and 65535".to_string(),
        });
    }

    // url_base normalization rules (RUNTIME-COMPOSE-004):
    //   - Must start with "/" or be empty
    //   - Must not end with "/"
    //   - "/" is allowed (normalized to "" by caller, but valid at validation)
    let url_base = &config.server.url_base;
    if !url_base.is_empty() {
        if !url_base.starts_with('/') {
            return Err(ConfigError::InvalidValue {
                field: "server.url_base".to_string(),
                message: "url_base must start with '/' or be empty".to_string(),
            });
        }
        if url_base.len() > 1 && url_base.ends_with('/') {
            return Err(ConfigError::InvalidValue {
                field: "server.url_base".to_string(),
                message: "url_base must not end with '/'".to_string(),
            });
        }
    }

    // trusted_proxies must be valid CIDRs
    for cidr in &config.auth.trusted_proxies {
        if cidr.parse::<std::net::IpAddr>().is_err() && parse_cidr(cidr).is_err() {
            return Err(ConfigError::InvalidValue {
                field: "auth.trusted_proxies".to_string(),
                message: format!("invalid CIDR: {cidr}"),
            });
        }
    }

    // convergence values must be positive — a zero interval would busy-loop the
    // job runner; a non-positive batch or threshold is degenerate.
    if config.convergence.interval_secs == 0 {
        return Err(ConfigError::InvalidValue {
            field: "convergence.interval_secs".to_string(),
            message: "interval_secs must be at least 1".to_string(),
        });
    }
    if config.convergence.batch_size < 1 {
        return Err(ConfigError::InvalidValue {
            field: "convergence.batch_size".to_string(),
            message: "batch_size must be at least 1".to_string(),
        });
    }
    if config.convergence.attempt_threshold < 1 {
        return Err(ConfigError::InvalidValue {
            field: "convergence.attempt_threshold".to_string(),
            message: "attempt_threshold must be at least 1".to_string(),
        });
    }

    // Same reasoning as convergence: a zero interval busy-loops the job runner
    // and a non-positive batch claims nothing.
    if config.author_link.interval_secs == 0 {
        return Err(ConfigError::InvalidValue {
            field: "author_link.interval_secs".to_string(),
            message: "interval_secs must be at least 1".to_string(),
        });
    }
    if config.author_link.batch_size < 1 {
        return Err(ConfigError::InvalidValue {
            field: "author_link.batch_size".to_string(),
            message: "batch_size must be at least 1".to_string(),
        });
    }

    Ok(())
}

/// Minimal CIDR parsing — validates {ip}/{prefix_len} format.
fn parse_cidr(cidr: &str) -> Result<(), String> {
    let parts: Vec<&str> = cidr.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err("missing prefix length".to_string());
    }
    parts[0]
        .parse::<std::net::IpAddr>()
        .map_err(|e| e.to_string())?;
    let prefix_len: u8 = parts[1]
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    let is_v4 = parts[0].parse::<std::net::Ipv4Addr>().is_ok();
    let max = if is_v4 { 32 } else { 128 };
    if prefix_len > max {
        return Err(format!("prefix length {prefix_len} exceeds maximum {max}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unknown key detection
// ---------------------------------------------------------------------------

/// Detect and warn about unknown config keys.
///
/// Satisfies: RUNTIME-CONFIG-003
pub fn warn_unknown_keys(raw: &toml::Value) {
    const KNOWN_ROOT: &[&str] = &["server", "auth", "log", "convergence", "author_link"];
    const KNOWN_SERVER: &[&str] = &["bind_address", "port", "url_base"];
    const KNOWN_AUTH: &[&str] = &["external_header", "trusted_proxies"];
    const KNOWN_LOG: &[&str] = &["level", "format"];
    const KNOWN_CONVERGENCE: &[&str] = &[
        "enabled",
        "interval_secs",
        "batch_size",
        "attempt_threshold",
    ];
    const KNOWN_AUTHOR_LINK: &[&str] = &["enabled", "interval_secs", "batch_size"];

    if let Some(table) = raw.as_table() {
        for key in table.keys() {
            if !KNOWN_ROOT.contains(&key.as_str()) {
                warn!("Unknown config key: {key}");
            }
        }

        if let Some(server) = table.get("server").and_then(|v| v.as_table()) {
            for key in server.keys() {
                if !KNOWN_SERVER.contains(&key.as_str()) {
                    warn!("Unknown config key: server.{key}");
                }
            }
        }

        if let Some(auth) = table.get("auth").and_then(|v| v.as_table()) {
            for key in auth.keys() {
                if !KNOWN_AUTH.contains(&key.as_str()) {
                    warn!("Unknown config key: auth.{key}");
                }
            }
        }

        if let Some(log) = table.get("log").and_then(|v| v.as_table()) {
            for key in log.keys() {
                if !KNOWN_LOG.contains(&key.as_str()) {
                    warn!("Unknown config key: log.{key}");
                }
            }
        }

        if let Some(convergence) = table.get("convergence").and_then(|v| v.as_table()) {
            for key in convergence.keys() {
                if !KNOWN_CONVERGENCE.contains(&key.as_str()) {
                    warn!("Unknown config key: convergence.{key}");
                }
            }
        }

        if let Some(author_link) = table.get("author_link").and_then(|v| v.as_table()) {
            for key in author_link.keys() {
                if !KNOWN_AUTHOR_LINK.contains(&key.as_str()) {
                    warn!("Unknown config key: author_link.{key}");
                }
            }
        }
    }
}
