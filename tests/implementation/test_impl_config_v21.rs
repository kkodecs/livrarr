use librarr::config::{validate_config, AppConfig, ConfigError, LogFormat, LogLevel};

fn parse_config(input: &str) -> AppConfig {
    toml::from_str::<AppConfig>(input).expect("TOML should deserialize")
}

// Boundary condition: lowest valid non-zero port should validate.
#[test]
fn test_impl_config_v21_port_one_is_valid() {
    let config = parse_config("[server]\nport = 1");
    assert_eq!(config.server.port, 1);
    assert!(validate_config(&config).is_ok());
}

// Deserialization edge case: u16 overflow should fail during TOML deserialization, not validation.
#[test]
fn test_impl_config_v21_port_overflow_fails_deserialization() {
    let err = toml::from_str::<AppConfig>("[server]\nport = 65536").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("port") || msg.contains("65536") || msg.contains("integer"));
}

// url_base edge case: multiple slashes are accepted (validation only checks start/end).
#[test]
fn test_impl_config_v21_url_base_multiple_slashes_is_valid() {
    let config = parse_config("[server]\nurl_base = \"//api//v1\"");
    assert_eq!(config.server.url_base, "//api//v1");
    assert!(validate_config(&config).is_ok());
}

// url_base edge case: deeply nested path is valid.
#[test]
fn test_impl_config_v21_url_base_deeply_nested_path_is_valid() {
    let config = parse_config("[server]\nurl_base = \"/a/b/c/d/e/f/g/h/i/j\"");
    assert!(validate_config(&config).is_ok());
}

// url_base edge case: unicode path segments are accepted.
#[test]
fn test_impl_config_v21_url_base_unicode_is_valid() {
    let config = parse_config("[server]\nurl_base = \"/路径/ß\"");
    assert!(validate_config(&config).is_ok());
}

// Deserialization edge case: leading whitespace in url_base string is preserved and fails validation.
#[test]
fn test_impl_config_v21_url_base_whitespace_preserved_and_fails_validation() {
    let config = parse_config("[server]\nurl_base = \" /api\"");
    assert_eq!(config.server.url_base, " /api");
    let err = validate_config(&config).unwrap_err();
    assert!(matches!(
        err,
        ConfigError::InvalidValue { ref field, .. } if field == "server.url_base"
    ));
}

// CIDR parsing: bare IPv4 address is accepted by validation.
#[test]
fn test_impl_config_v21_trusted_proxy_bare_ipv4_is_valid() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"192.168.1.10\"]");
    assert!(validate_config(&config).is_ok());
}

// CIDR parsing: bare IPv6 address is accepted by validation.
#[test]
fn test_impl_config_v21_trusted_proxy_bare_ipv6_is_valid() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"2001:db8::1\"]");
    assert!(validate_config(&config).is_ok());
}

// CIDR boundary: IPv4 /0 is valid.
#[test]
fn test_impl_config_v21_trusted_proxy_ipv4_prefix_zero_is_valid() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"0.0.0.0/0\"]");
    assert!(validate_config(&config).is_ok());
}

// CIDR boundary: IPv6 /0 is valid.
#[test]
fn test_impl_config_v21_trusted_proxy_ipv6_prefix_zero_is_valid() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"::/0\"]");
    assert!(validate_config(&config).is_ok());
}

// CIDR boundary: IPv6 /128 is valid.
#[test]
fn test_impl_config_v21_trusted_proxy_ipv6_prefix_128_is_valid() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"2001:db8::/128\"]");
    assert!(validate_config(&config).is_ok());
}

// CIDR boundary: IPv6 prefix 129 exceeds max and fails.
#[test]
fn test_impl_config_v21_trusted_proxy_ipv6_prefix_129_fails_validation() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"2001:db8::/129\"]");
    let err = validate_config(&config).unwrap_err();
    assert!(matches!(
        err,
        ConfigError::InvalidValue { ref field, .. } if field == "auth.trusted_proxies"
    ));
}

// CIDR boundary: IPv4 /32 is valid.
#[test]
fn test_impl_config_v21_trusted_proxy_ipv4_prefix_32_is_valid() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"10.0.0.1/32\"]");
    assert!(validate_config(&config).is_ok());
}

// CIDR boundary: IPv4 prefix 33 exceeds max and fails.
#[test]
fn test_impl_config_v21_trusted_proxy_ipv4_prefix_33_fails_validation() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"10.0.0.1/33\"]");
    let err = validate_config(&config).unwrap_err();
    assert!(matches!(
        err,
        ConfigError::InvalidValue { ref field, .. } if field == "auth.trusted_proxies"
    ));
}

// Error: CIDR with missing prefix length after slash.
#[test]
fn test_impl_config_v21_trusted_proxy_missing_prefix_fails_validation() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"10.0.0.1/\"]");
    assert!(validate_config(&config).is_err());
}

// Error: negative prefix fails u8 parsing.
#[test]
fn test_impl_config_v21_trusted_proxy_negative_prefix_fails_validation() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"10.0.0.1/-1\"]");
    assert!(validate_config(&config).is_err());
}

// Error: numeric overflow in prefix (>255) fails u8 parsing.
#[test]
fn test_impl_config_v21_trusted_proxy_prefix_numeric_overflow_fails_validation() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"10.0.0.1/999\"]");
    assert!(validate_config(&config).is_err());
}

// Error: empty string in trusted_proxies fails validation.
#[test]
fn test_impl_config_v21_trusted_proxy_empty_string_fails_validation() {
    let config = parse_config("[auth]\ntrusted_proxies = [\"\"]");
    assert!(validate_config(&config).is_err());
}

// Deserialization: extra whitespace in enum string value fails.
#[test]
fn test_impl_config_v21_log_level_with_extra_whitespace_fails_deserialization() {
    let err = toml::from_str::<AppConfig>("[log]\nlevel = \" info \"").unwrap_err();
    assert!(!err.to_string().is_empty());
}

// Deserialization: enum values are case-sensitive (lowercase only).
#[test]
fn test_impl_config_v21_log_level_case_sensitive_fails_deserialization() {
    assert!(toml::from_str::<AppConfig>("[log]\nlevel = \"INFO\"").is_err());
}

// Deserialization: log format is also case-sensitive.
#[test]
fn test_impl_config_v21_log_format_case_sensitive_fails_deserialization() {
    assert!(toml::from_str::<AppConfig>("[log]\nformat = \"JSON\"").is_err());
}

// Validation ordering: port error reported before url_base when both invalid.
#[test]
fn test_impl_config_v21_validation_returns_port_error_first_when_multiple_invalid() {
    let config = parse_config("[server]\nport = 0\nurl_base = \"bad\"");
    let err = validate_config(&config).unwrap_err();
    assert!(matches!(
        err,
        ConfigError::InvalidValue { ref field, .. } if field == "server.port"
    ));
}

// Validation ordering: url_base error reported before proxy error when port valid.
#[test]
fn test_impl_config_v21_validation_returns_url_base_error_before_proxy_error() {
    let config = parse_config(
        "[server]\nport = 8080\nurl_base = \"bad\"\n[auth]\ntrusted_proxies = [\"not-a-cidr\"]",
    );
    let err = validate_config(&config).unwrap_err();
    assert!(matches!(
        err,
        ConfigError::InvalidValue { ref field, .. } if field == "server.url_base"
    ));
}

// Validation: only first invalid trusted proxy is surfaced.
#[test]
fn test_impl_config_v21_validation_returns_first_invalid_trusted_proxy_only() {
    let config =
        parse_config("[auth]\ntrusted_proxies = [\"10.0.0.1/24\", \"bad-cidr\", \"also-bad\"]");
    let err = validate_config(&config).unwrap_err();
    match err {
        ConfigError::InvalidValue { ref message, .. } => {
            assert!(message.contains("bad-cidr"));
            assert!(!message.contains("also-bad"));
        }
        _ => panic!("expected InvalidValue"),
    }
}

// Deserialization: whitespace in trusted proxy strings is preserved and causes validation failure.
#[test]
fn test_impl_config_v21_trusted_proxy_whitespace_preserved_and_fails() {
    let config = parse_config("[auth]\ntrusted_proxies = [\" 127.0.0.1 \"]");
    assert_eq!(config.auth.trusted_proxies, vec![" 127.0.0.1 ".to_string()]);
    assert!(validate_config(&config).is_err());
}

// Multiple valid proxies: mix of bare IPs and CIDRs should all validate.
#[test]
fn test_impl_config_v21_multiple_mixed_valid_proxies() {
    let config = parse_config(
        "[auth]\ntrusted_proxies = [\"10.0.0.1/24\", \"192.168.1.1\", \"::1\", \"2001:db8::/32\"]",
    );
    assert!(validate_config(&config).is_ok());
}

// Deserialization: TOML whitespace around assignment doesn't affect enum parsing.
#[test]
fn test_impl_config_v21_enum_values_parse_with_toml_whitespace() {
    let config: AppConfig =
        toml::from_str("[log]\nlevel   =   \"debug\"\nformat  =   \"json\"").expect("valid TOML");
    assert_eq!(config.log.level, LogLevel::Debug);
    assert_eq!(config.log.format, LogFormat::Json);
}
