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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
            url_base: String::new(),
        }
    }
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8787
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
    const KNOWN_ROOT: &[&str] = &["server", "auth", "log"];
    const KNOWN_SERVER: &[&str] = &["bind_address", "port", "url_base"];
    const KNOWN_AUTH: &[&str] = &["external_header", "trusted_proxies"];
    const KNOWN_LOG: &[&str] = &["level", "format"];

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
    }
}
