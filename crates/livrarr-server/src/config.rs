//! Bootstrap configuration (TOML).
//!
//! Satisfies: RUNTIME-CONFIG-001, RUNTIME-CONFIG-002, RUNTIME-CONFIG-003,
//!            RUNTIME-COMPOSE-004, RUNTIME-LOG-001, RUNTIME-LOG-002

use serde::Deserialize;

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
    pub log: LogConfig,

    #[serde(default)]
    pub convergence: ConvergenceConfig,

    #[serde(default)]
    pub metadata_cache: MetadataCacheConfig,

    #[serde(default)]
    pub author_link: AuthorLinkConfig,

    /// The keys in `config.toml` that the loader does not read, as
    /// `unknown_keys` reports them. Set by `load_config`, never read from the
    /// file.
    #[serde(skip)]
    pub unknown_keys: Vec<String>,
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

    /// The trace viewer's OTLP-over-HTTP endpoint, such as
    /// `http://127.0.0.1:4318`. Missing or empty means tracing is off;
    /// `trace_endpoint()` reads it.
    #[serde(default)]
    pub trace_endpoint: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
            trace_endpoint: None,
        }
    }
}

/// What `[log] trace_endpoint` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEndpoint {
    /// Missing or empty: no exporter is built.
    Off,
    /// The OTLP/HTTP traces URL request spans are sent to: the endpoint with
    /// `/v1/traces` appended.
    Export(String),
    /// A value that is not an `http://` or `https://` address with a host and
    /// no query or fragment, as written. No exporter is built.
    Malformed(String),
}

impl LogConfig {
    /// The one reading of `trace_endpoint`, shared by tracing start-up and the
    /// config warnings.
    pub fn trace_endpoint(&self) -> TraceEndpoint {
        let Some(raw) = self.trace_endpoint.as_deref() else {
            return TraceEndpoint::Off;
        };
        let value = raw.trim();
        if value.is_empty() {
            return TraceEndpoint::Off;
        }
        match reqwest::Url::parse(value) {
            Ok(url)
                if matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some_and(|host| !host.is_empty())
                    && url.query().is_none()
                    && url.fragment().is_none() =>
            {
                TraceEndpoint::Export(format!("{}/v1/traces", url.as_str().trim_end_matches('/')))
            }
            _ => TraceEndpoint::Malformed(raw.to_owned()),
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

// ---------------------------------------------------------------------------
// Unknown key detection
// ---------------------------------------------------------------------------

/// Every section the loader reads, with every key it reads in that section.
const KNOWN_KEYS: &[(&str, &[&str])] = &[
    (
        "server",
        &["bind_address", "port", "url_base", "trusted_proxies"],
    ),
    ("log", &["level", "format", "trace_endpoint"]),
    (
        "convergence",
        &[
            "enabled",
            "interval_secs",
            "batch_size",
            "attempt_threshold",
        ],
    ),
    ("metadata_cache", &["ttl_days", "max_rows"]),
    ("author_link", &["enabled", "interval_secs", "batch_size"]),
];

/// The keys in a parsed `config.toml` that the loader does not read: an unknown
/// root key as `key`, an unknown key inside a known section as `section.key`.
///
/// Satisfies: RUNTIME-CONFIG-003
pub fn unknown_keys(table: &toml::Table) -> Vec<String> {
    let mut unknown = Vec::new();
    for (key, value) in table {
        match KNOWN_KEYS.iter().find(|(section, _)| section == key) {
            None => unknown.push(key.clone()),
            Some((section, known)) => {
                if let Some(children) = value.as_table() {
                    unknown.extend(
                        children
                            .keys()
                            .filter(|child| !known.contains(&child.as_str()))
                            .map(|child| format!("{section}.{child}")),
                    );
                }
            }
        }
    }
    unknown
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Reads and validates `{data_dir}/config.toml`, returning the config and the
/// keys in it that the loader does not read. The config also carries those
/// keys in `unknown_keys`. An absent or blank file is the default config.
pub fn load_config(data_dir: &std::path::Path) -> Result<(AppConfig, Vec<String>), String> {
    let config_path = data_dir.join("config.toml");

    let mut unknown = Vec::new();
    let mut config: AppConfig = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("failed to read config.toml: {e}"))?;

        if raw.trim().is_empty() {
            AppConfig::default()
        } else {
            if let Ok(table) = raw.parse::<toml::Table>() {
                unknown = unknown_keys(&table);
            }

            toml::from_str(&raw).map_err(|e| format!("failed to parse config.toml: {e}"))?
        }
    } else {
        AppConfig::default()
    };

    validate_config(&config).map_err(|e| e.to_string())?;
    config.unknown_keys = unknown.clone();
    Ok((config, unknown))
}

// ---------------------------------------------------------------------------
// Warnings
// ---------------------------------------------------------------------------

/// Every warning the loaded config gives, cleansed for display: first each
/// rejected `[server] trusted_proxies` entry in file order, then a malformed
/// `[log] trace_endpoint`, then each unknown key in the loader's order.
/// Startup logs them; the admin health list shows them.
pub fn config_warnings(config: &AppConfig) -> Vec<String> {
    let (_, rejected) = crate::rate_limit::parse_trusted_proxies(&config.server.trusted_proxies);
    let proxies = rejected.iter().map(|entry| {
        format!(
            "Ignored [server] trusted_proxies entry \"{entry}\": use an IP address or range \
             such as 172.18.0.0/16; host names and ports are not supported"
        )
    });
    let trace = match config.log.trace_endpoint() {
        TraceEndpoint::Malformed(value) => Some(format!(
            "Ignored [log] trace_endpoint \"{value}\": use an http:// or https:// address \
             such as http://127.0.0.1:4318; tracing is off"
        )),
        TraceEndpoint::Off | TraceEndpoint::Export(_) => None,
    };
    let keys = config
        .unknown_keys
        .iter()
        .map(|key| format!("Unknown config key: {key}"));
    proxies
        .chain(trace)
        .chain(keys)
        .map(|message| livrarr_domain::cleanse_log_line(&message))
        .collect()
}
