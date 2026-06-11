use toml::Value;

use librarr::config::{validate_config, AppConfig, ConfigError, LogFormat, LogLevel};

fn parse_app_config(input: &str) -> Result<AppConfig, toml::de::Error> {
    toml::from_str::<AppConfig>(input)
}

#[test]
fn test_config_v21_empty_toml_applies_all_defaults() {
    // REQ-ID: RUNTIME-CONFIG-001, RUNTIME-CONFIG-002
    // IR contract: AppConfig/ServerConfig/AuthConfig/LogConfig deserialization with serde defaults

    let cfg = parse_app_config("").expect("empty TOML should deserialize with defaults");

    assert_eq!(cfg.server.bind_address, "0.0.0.0");
    assert_eq!(cfg.server.port, 8787);
    assert_eq!(cfg.server.url_base, "");

    assert_eq!(cfg.auth.external_header, None);
    assert!(cfg.auth.trusted_proxies.is_empty());

    assert!(matches!(cfg.log.level, LogLevel::Info));
    assert!(matches!(cfg.log.format, LogFormat::Text));
}

#[test]
fn test_config_v21_full_toml_parses_all_values() {
    // REQ-ID: RUNTIME-CONFIG-002, AUTH-009, RUNTIME-LOG-001, RUNTIME-LOG-002
    // IR contract: AppConfig schema deserialization for server/auth/log sections

    let input = r#"
[server]
bind_address = "127.0.0.1"
port = 9090
url_base = "/librarr"

[auth]
external_header = "X-Remote-User"
trusted_proxies = ["10.0.0.0/8", "192.168.1.0/24"]

[log]
level = "debug"
format = "json"
"#;

    let cfg = parse_app_config(input).expect("valid full TOML should deserialize");

    assert_eq!(cfg.server.bind_address, "127.0.0.1");
    assert_eq!(cfg.server.port, 9090);
    assert_eq!(cfg.server.url_base, "/librarr");

    assert_eq!(cfg.auth.external_header.as_deref(), Some("X-Remote-User"));
    assert_eq!(
        cfg.auth.trusted_proxies,
        vec!["10.0.0.0/8".to_string(), "192.168.1.0/24".to_string()]
    );

    assert!(matches!(cfg.log.level, LogLevel::Debug));
    assert!(matches!(cfg.log.format, LogFormat::Json));
}

#[test]
fn test_config_v21_missing_auth_section_defaults() {
    // REQ-ID: RUNTIME-CONFIG-001, AUTH-009
    // IR contract: AppConfig.auth has #[serde(default)] and AuthConfig::default()

    let input = r#"
[server]
bind_address = "127.0.0.1"
"#;

    let cfg = parse_app_config(input).expect("missing auth section should default");

    assert_eq!(cfg.auth.external_header, None);
    assert!(cfg.auth.trusted_proxies.is_empty());
}

#[test]
fn test_config_v21_missing_nested_fields_use_defaults() {
    // REQ-ID: RUNTIME-CONFIG-001, RUNTIME-CONFIG-002, RUNTIME-LOG-001, RUNTIME-LOG-002
    // IR contract: field-level serde defaults for server and log fields

    let input = r#"
[server]
bind_address = "127.0.0.1"

[log]
level = "warn"
"#;

    let cfg = parse_app_config(input).expect("missing nested fields should use defaults");

    assert_eq!(cfg.server.bind_address, "127.0.0.1");
    assert_eq!(cfg.server.port, 8787);
    assert_eq!(cfg.server.url_base, "");

    assert!(matches!(cfg.log.level, LogLevel::Warn));
    assert!(matches!(cfg.log.format, LogFormat::Text));
}

#[test]
fn test_config_v21_validate_config_accepts_valid_input() {
    // REQ-ID: RUNTIME-CONFIG-003, RUNTIME-COMPOSE-004
    // IR contract: validate_config returns Ok for valid port, url_base, and trusted_proxies

    let input = r#"
[server]
bind_address = "0.0.0.0"
port = 8787
url_base = "/librarr"

[auth]
trusted_proxies = ["10.0.0.0/8", "172.16.0.0/12"]

[log]
level = "info"
format = "text"
"#;

    let cfg = parse_app_config(input).expect("valid TOML should deserialize");
    let result = validate_config(&cfg);

    assert!(result.is_ok(), "expected valid config to pass validation");
}

#[test]
fn test_config_v21_port_zero_fails_validation() {
    // REQ-ID: RUNTIME-CONFIG-003
    // IR contract: validate_config checks port in 1..=65535

    let input = r#"
[server]
port = 0
"#;

    let cfg = parse_app_config(input).expect("port=0 is valid TOML/u16 and should deserialize");
    let err = validate_config(&cfg).expect_err("port=0 must fail validation");

    match err {
        ConfigError::InvalidValue { field, .. } => {
            assert_eq!(field, "server.port");
        }
        other => panic!("expected InvalidValue for server.port, got {other:?}"),
    }
}

#[test]
fn test_config_v21_port_65535_is_valid() {
    // REQ-ID: RUNTIME-CONFIG-003
    // IR contract: validate_config accepts the upper valid boundary of u16 ports

    let input = r#"
[server]
port = 65535
"#;

    let cfg = parse_app_config(input).expect("port=65535 should deserialize");
    let result = validate_config(&cfg);

    assert!(result.is_ok(), "port=65535 should be valid");
}

#[test]
fn test_config_v21_malformed_cidr_fails_validation() {
    // REQ-ID: RUNTIME-CONFIG-003, AUTH-009
    // IR contract: validate_config checks trusted_proxies are valid CIDRs

    let input = r#"
[auth]
trusted_proxies = ["not-a-cidr"]
"#;

    let cfg = parse_app_config(input).expect("malformed CIDR string should still deserialize");
    let err = validate_config(&cfg).expect_err("malformed CIDR must fail validation");

    match err {
        ConfigError::InvalidValue { field, .. } => {
            assert_eq!(field, "auth.trusted_proxies");
        }
        other => panic!("expected InvalidValue for auth.trusted_proxies, got {other:?}"),
    }
}

#[test]
fn test_config_v21_bad_toml_returns_parse_error_from_toml_deserialization() {
    // REQ-ID: RUNTIME-CONFIG-003
    // IR contract: AppConfig is deserialized from TOML; malformed TOML must fail parse

    let input = r#"
[server
port = 8787
"#;

    let err = parse_app_config(input).expect_err("malformed TOML must not deserialize");
    let message = err.to_string().to_lowercase();

    assert!(
        message.contains("expected")
            || message.contains("invalid")
            || message.contains("unterminated")
            || message.contains("table"),
        "unexpected parse error message: {message}"
    );
}

#[test]
fn test_config_v21_invalid_log_level_fails_deserialization() {
    // REQ-ID: RUNTIME-LOG-001, RUNTIME-CONFIG-003
    // IR contract: LogLevel uses lowercase serde enum variants; invalid value must fail parse

    let input = r#"
[log]
level = "verbose"
"#;

    let err = parse_app_config(input).expect_err("invalid log level must fail deserialization");
    let message = err.to_string().to_lowercase();

    assert!(
        message.contains("unknown variant")
            || message.contains("invalid value")
            || message.contains("expected"),
        "unexpected enum parse error message: {message}"
    );
}

#[test]
fn test_config_v21_unknown_keys_are_ignored_by_deserialization() {
    // REQ-ID: RUNTIME-CONFIG-003
    // IR contract: Unknown keys warn and ignore; AppConfig deserialization should still succeed

    let input = r#"
unknown_root = "ignored"

[server]
port = 9999
extra_server_key = true

[auth]
external_header = "X-User"
unknown_auth = 123

[log]
level = "error"
unknown_log = "ignored"
"#;

    let cfg = parse_app_config(input).expect("unknown keys should not prevent deserialization");

    assert_eq!(cfg.server.port, 9999);
    assert_eq!(cfg.auth.external_header.as_deref(), Some("X-User"));
    assert!(matches!(cfg.log.level, LogLevel::Error));
}

#[test]
fn test_config_v21_warn_unknown_keys_can_be_called_with_raw_toml_value() {
    // REQ-ID: RUNTIME-CONFIG-003
    // IR contract: warn_unknown_keys(raw: &toml::Value) accepts parsed raw TOML and emits warnings for unknown keys.
    // Behavioral minimum here is that it can be called safely without panicking.
    // Log output verification is deferred to implementation-coupled tests.

    let raw: Value = toml::from_str(
        r#"
unknown_root = "ignored"

[server]
port = 8787
unknown = "x"
"#,
    )
    .expect("raw TOML should parse to toml::Value");

    librarr::config::warn_unknown_keys(&raw);
}

#[test]
fn test_config_v21_url_base_empty_is_valid() {
    // REQ-ID: RUNTIME-COMPOSE-004, RUNTIME-CONFIG-003
    // IR contract: validate_config allows empty url_base

    let input = r#"
[server]
url_base = ""
"#;

    let cfg = parse_app_config(input).expect("empty url_base should deserialize");
    let result = validate_config(&cfg);

    assert!(result.is_ok(), "empty url_base should be valid");
}

#[test]
fn test_config_v21_url_base_slash_is_valid_normalized_case() {
    // REQ-ID: RUNTIME-COMPOSE-004, RUNTIME-CONFIG-003
    // IR contract: "/" is a valid input because the config loading layer normalizes "/" -> ""
    // before validate_config is called. validate_config should therefore accept this boundary case.

    let input = r#"
[server]
url_base = "/"
"#;

    let cfg = parse_app_config(input).expect(r#""/" url_base should deserialize"#);
    let result = validate_config(&cfg);

    assert!(
        result.is_ok(),
        r#"url_base "/" should be accepted as the normalization boundary case"#
    );
}

#[test]
fn test_config_v21_url_base_prefixed_path_is_valid() {
    // REQ-ID: RUNTIME-COMPOSE-004, RUNTIME-CONFIG-003
    // IR contract: validate_config accepts url_base starting with "/" and without trailing "/"

    let input = r#"
[server]
url_base = "/librarr"
"#;

    let cfg = parse_app_config(input).expect("prefixed url_base should deserialize");
    let result = validate_config(&cfg);

    assert!(result.is_ok(), "url_base '/librarr' should be valid");
}

#[test]
fn test_config_v21_url_base_without_leading_slash_fails_validation() {
    // REQ-ID: RUNTIME-COMPOSE-004, RUNTIME-CONFIG-003
    // IR contract: validate_config rejects url_base that does not start with "/" unless empty

    let input = r#"
[server]
url_base = "librarr"
"#;

    let cfg = parse_app_config(input).expect("non-empty url_base string should deserialize");
    let err = validate_config(&cfg).expect_err("url_base without leading slash must fail");

    match err {
        ConfigError::InvalidValue { field, .. } => {
            assert_eq!(field, "server.url_base");
        }
        other => panic!("expected InvalidValue for server.url_base, got {other:?}"),
    }
}

#[test]
fn test_config_v21_url_base_with_trailing_slash_fails_validation() {
    // REQ-ID: RUNTIME-COMPOSE-004, RUNTIME-CONFIG-003
    // IR contract: validate_config rejects url_base with trailing "/" except normalization case "/"

    let input = r#"
[server]
url_base = "/librarr/"
"#;

    let cfg = parse_app_config(input).expect("url_base with trailing slash should deserialize");
    let err = validate_config(&cfg).expect_err("trailing slash url_base must fail validation");

    match err {
        ConfigError::InvalidValue { field, .. } => {
            assert_eq!(field, "server.url_base");
        }
        other => panic!("expected InvalidValue for server.url_base, got {other:?}"),
    }
}
