//! Secret redaction for log-safe rendering of URLs and error strings.
//!
//! `redact_secrets` masks the credential-bearing parts of a string — sensitive
//! query-parameter values and `user:pass@` URL userinfo — to `[REDACTED]`,
//! leaving every other byte untouched. It works on a bare URL or on an error
//! message that embeds one (the shared HTTP fetcher's transport errors carry the
//! request URL). Display-only: the real value is never mutated at the source, so
//! it stays available to send to the external service.

use std::sync::LazyLock;

use regex::Regex;

const PLACEHOLDER: &str = "[REDACTED]";

/// The value of a sensitive query parameter, up to the next `&`, `#`,
/// whitespace, or quote. Case-insensitive on the key. Group 1 captures the
/// `?`/`&` separator plus `key=` so they survive while the value is replaced.
static QUERY_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([?&](?:apikey|api_key|token|passkey|password)=)[^&#\s"'<>]*"#).unwrap()
});

/// `user:pass@` userinfo in a URL. The colon is required, so a bare `user@`
/// (no password) is left alone. Group 1 captures the `://` scheme separator.
static USERINFO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(://)[^/?#\s@:]+:[^/?#\s@]+@"#).unwrap());

/// Mask credential-bearing parts of a string for safe logging. Leaves all
/// non-secret bytes unchanged (so an ordinary URL round-trips identically).
pub fn redact_secrets(input: &str) -> String {
    let step1 = QUERY_SECRET.replace_all(input, |c: &regex::Captures| {
        format!("{}{}", &c[1], PLACEHOLDER)
    });
    let step2 = USERINFO.replace_all(&step1, |c: &regex::Captures| {
        format!("{}{}@", &c[1], PLACEHOLDER)
    });
    step2.into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact_secrets;

    #[test]
    fn masks_apikey_query_param_keeps_the_rest() {
        let out = redact_secrets("http://host:9696/api?apikey=SECRET123&t=search");
        assert!(!out.contains("SECRET123"), "leaked: {out}");
        assert!(out.contains("apikey=[REDACTED]"), "{out}");
        assert!(out.contains("host:9696"));
        assert!(out.contains("t=search"));
    }

    #[test]
    fn masks_every_sensitive_key_case_insensitive() {
        for key in [
            "apikey", "api_key", "token", "passkey", "password", "ApiKey", "TOKEN", "Password",
        ] {
            let url = format!("http://h/x?{key}=ZZZSECRET");
            let out = redact_secrets(&url);
            assert!(!out.contains("ZZZSECRET"), "leaked for key {key}: {out}");
        }
    }

    #[test]
    fn masks_userinfo_password() {
        let out = redact_secrets("http://user:hunter2@host/path");
        assert!(!out.contains("hunter2"), "leaked: {out}");
        assert!(out.contains("[REDACTED]@host"), "{out}");
    }

    #[test]
    fn placeholder_is_literal_not_percent_encoded() {
        let out = redact_secrets("http://h/x?token=abc");
        assert!(out.contains("[REDACTED]"), "{out}");
        assert!(!out.contains("%5B"), "placeholder got url-encoded: {out}");
    }

    #[test]
    fn masks_secret_embedded_in_error_text() {
        let msg = "connection error: error sending request for url \
                   (http://10.0.0.1:9696/2/api?apikey=DEADBEEF)";
        let out = redact_secrets(msg);
        assert!(!out.contains("DEADBEEF"), "leaked: {out}");
        assert!(out.contains("connection error"));
    }

    #[test]
    fn leaves_ordinary_url_byte_identical() {
        let url = "http://host:9696/api?t=search&cat=7000";
        assert_eq!(redact_secrets(url), url);
    }

    #[test]
    fn host_port_without_userinfo_untouched() {
        let url = "http://host:9696/path";
        assert_eq!(redact_secrets(url), url);
    }

    #[test]
    fn masks_multiple_params_including_last_position() {
        let out = redact_secrets("http://h/x?a=1&apikey=SEC1&b=2&token=SEC2");
        assert!(!out.contains("SEC1"), "{out}");
        assert!(!out.contains("SEC2"), "{out}");
        assert!(out.contains("a=1"));
        assert!(out.contains("b=2"));
    }
}
