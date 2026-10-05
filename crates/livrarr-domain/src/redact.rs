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

/// Characters a masked value never runs past. Quotes and backslashes are
/// excluded so a masked JSON-format log line keeps every quote and every
/// escape it had; ANSI escapes are excluded so a colour code is never taken
/// for a value.
const VALUE: &str = r#"[^&#\s"'\\\x1b]+"#;

/// A value that follows an unescaped opening quote. Its first character is
/// not JSON structure, so the end of a JSON string is never taken for the
/// opening quote of a value.
const QUOTED_VALUE: &str = r#"[^&#\s"'\\\x1b,:}\]][^&#\s"'\\\x1b]*"#;

/// An ANSI colour code, as the console writes around a structured field.
const ANSI: &str = r"\x1b\[[0-9;]*m";

/// Header names whose values are credentials.
const SECRET_HEADERS: &str = "authorization|x-api-key|x-goog-api-key|cookie";

/// Query keys and structured-field names whose values are credentials. The
/// same names apply to a structured field in text form (`name=value`) and in
/// JSON form (`"name":value`).
const SECRET_KEYS: &str = "[a-z0-9_]*apikey|[a-z0-9_]*api_key|[a-z0-9_]*_token|token|passkey\
                           |password|passwd|authkey|auth|nzb_key";

/// Words that mark a JSON member name as holding a credential.
const SECRET_MEMBER_WORDS: &str = "apikey|api_key|token|password|passkey|secret|nzb_key";

/// One step inside URL userinfo: an escape sequence, kept whole so a JSON
/// string stays valid; an apostrophe; or a double quote that is not followed
/// by JSON structure, so the span never runs past the end of a JSON string.
const USERINFO_DELIMITER: &str = r#"\\u[0-9a-fA-F]{4}|\\[^\s]|'|"[^/?#\s@"\\,:}\]]"#;

/// The parts of matched userinfo that stay: escape sequences and quotes.
/// Every other run of characters is masked.
static USERINFO_PART: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\\u[0-9a-fA-F]{4}|\\[^\s]|["']|[^"'\\]+"#).unwrap());

/// How a rule rewrites a match.
type Mask = fn(&regex::Captures) -> String;

/// Keeps group 1 and, where present, group 2, with the placeholder between.
fn mask_between(c: &regex::Captures) -> String {
    let suffix = c.get(2).map_or("", |m| m.as_str());
    format!("{}{PLACEHOLDER}{suffix}", &c[1])
}

/// Keeps group 1, a JSON member name and colon, and replaces the number after
/// it with the placeholder as a JSON string, quoted the way the name is.
fn mask_number(c: &regex::Captures) -> String {
    let quote = if c[1].starts_with('\\') { "\\\"" } else { "\"" };
    format!("{}{quote}{PLACEHOLDER}{quote}", &c[1])
}

/// Whether an escape sequence represents `/`, `?` or `#`, the characters that
/// end URL userinfo: `\/`, `\?`, `\#`, or a `\u` escape of one of them.
fn escapes_url_boundary(escape: &str) -> bool {
    let represented = match escape.strip_prefix("\\u") {
        Some(hex) => u32::from_str_radix(hex, 16).ok().and_then(char::from_u32),
        None => escape
            .strip_prefix('\\')
            .and_then(|rest| rest.chars().next()),
    };
    matches!(represented, Some('/' | '?' | '#'))
}

/// Keeps group 1 (`://`) and group 3 (`@`) and masks the userinfo in group 2,
/// keeping its quotes and escape sequences. A span holding an escape that
/// represents `/`, `?` or `#` is a host, port and path rather than userinfo,
/// and is returned unchanged, as the same text with that character unescaped
/// would be.
fn mask_userinfo(c: &regex::Captures) -> String {
    if USERINFO_PART
        .find_iter(&c[2])
        .any(|part| escapes_url_boundary(part.as_str()))
    {
        return c[0].to_owned();
    }
    let userinfo = USERINFO_PART.replace_all(&c[2], |part: &regex::Captures| {
        let text = &part[0];
        if text.starts_with(['\\', '"', '\'']) {
            text.to_owned()
        } else {
            PLACEHOLDER.to_owned()
        }
    });
    format!("{}{userinfo}{}", &c[1], &c[3])
}

/// The rules of the log cleanser, applied in order, each with how it masks.
static LOG_RULES: LazyLock<Vec<(Regex, Mask)>> = LazyLock::new(|| {
    let field = format!(r#"(?:^|[?&\s"]|{ANSI})(?:{SECRET_KEYS})(?:{ANSI})*=(?:{ANSI})*"#);
    let member = format!(r#"(?:[^"\\\n]*(?:{SECRET_MEMBER_WORDS})[^"\\\n]*|(?:{SECRET_KEYS}))"#);
    let userinfo = format!(
        r#"(?:[^/?#\s@:"'\\]|{USERINFO_DELIMITER})+:(?:[^/?#\s@"'\\]|{USERINFO_DELIMITER})+"#
    );
    [
        // A Debug tuple `("Authorization", "Bearer …")`, with plain or
        // backslash-escaped quotes; the whole value up to its closing quote.
        (
            format!(r#"(?i)(\(\s*\\?"(?:{SECRET_HEADERS})\\?"\s*,\s*\\?")[^"\\]+"#),
            mask_between as Mask,
        ),
        // A header line `Authorization: Bearer …`; an auth scheme word stays.
        (
            format!(r"(?i)(\b(?:{SECRET_HEADERS}):[ \t]*(?:(?:bearer|basic)[ \t]+)?){VALUE}"),
            mask_between,
        ),
        // A bearer or basic credential anywhere.
        (
            format!(r"(?i)(\b(?:bearer|basic)[ \t]+){VALUE}"),
            mask_between,
        ),
        // A query value or a structured field, ANSI codes allowed around `=`,
        // bare or after a backslash-escaped quote (a quoted field inside a
        // JSON string).
        (format!(r#"(?im)({field}(?:\\")?){VALUE}"#), mask_between),
        // A structured field whose value the formatter quotes; the quotes stay.
        (format!(r#"(?im)({field}"){QUOTED_VALUE}"#), mask_between),
        // A JSON member with plain quotes and a string value, which may open
        // with an escaped quote (a Debug string field in a JSON-format event).
        (
            format!(r#"(?i)("{member}"\s*:\s*"(?:\\")?)[^"\\]+"#),
            mask_between,
        ),
        // A JSON member with backslash-escaped quotes, as a JSON body carried
        // inside a JSON-format log line.
        (
            format!(r#"(?i)(\\"{member}\\"\s*:\s*\\")[^"\\]+"#),
            mask_between,
        ),
        // A structured-field-named JSON member with a number value, with plain
        // or backslash-escaped quotes.
        (
            format!(r#"(?i)(\\?"(?:{SECRET_KEYS})\\?"\s*:\s*)-?[0-9][0-9.eE+\-]*"#),
            mask_number,
        ),
        // `user:pass@` userinfo in a URL.
        (format!(r"(://)({userinfo})(@)"), mask_userinfo),
    ]
    .into_iter()
    .map(|(pattern, mask)| (Regex::new(&pattern).unwrap(), mask))
    .collect()
});

/// Mask secret-shaped text in one log line before it reaches a log sink.
///
/// Covers everything `redact_secrets` masks plus header values, bearer and
/// basic credentials, more query keys, structured fields (bare or quoted, with
/// or without ANSI colour codes around `=`, and as JSON members with string or
/// number values), and secret-named JSON members with plain or
/// backslash-escaped quotes. A masked value never loses a quote or a
/// backslash, so a JSON-format line still parses afterwards; text with no
/// secret shape is returned byte-identical. For log sinks only: text returned
/// to clients goes through `redact_secrets`.
pub fn cleanse_log_line(input: &str) -> String {
    let mut out = input.to_owned();
    for (rule, mask) in LOG_RULES.iter() {
        if !rule.is_match(&out) {
            continue;
        }
        out = rule.replace_all(&out, *mask).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{cleanse_log_line, redact_secrets};

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

    #[test]
    fn log_cleanser_masks_every_secret_shape() {
        let cases = [
            "http://h/x?apikey=S3CRETv41",
            "http://h/x?a=1&API_KEY=S3CRETv41",
            "http://h/x?sab_apikey=S3CRETv41",
            "http://h/x?Access_Token=S3CRETv41&b=2",
            "refresh_token=S3CRETv41",
            "password=S3CRETv41",
            "http://h/x?passwd=S3CRETv41",
            "http://h/x?AuthKey=S3CRETv41",
            "http://h/x?auth=S3CRETv41",
            "NZB_KEY=S3CRETv41",
            "http://h/x?api_key=S3CRETv41%2BS3CRETv41",
            "http://admin:S3CRETv41@h/x",
            "X-Api-Key: S3CRETv41",
            "X-Goog-Api-Key: S3CRETv41",
            "Cookie: SID=S3CRETv41",
            "Authorization: Bearer S3CRETv41",
            "Authorization: Basic S3CRETv41",
            "upstream said Bearer S3CRETv41 was refused",
            r#"headers: [("X-Api-Key", "S3CRETv41")]"#,
            r#"headers: [("authorization", "Bearer S3CRETv41")]"#,
            r#"headers: [(\"Cookie\", \"SID=S3CRETv41\")]"#,
            r#"{"apikey":"S3CRETv41"}"#,
            r#"{"user_api_key":"S3CRETv41","id":7}"#,
            r#"{"Password": "S3CRETv41"}"#,
            r#"{"client_secret":"S3CRETv41"}"#,
            r#"{\"nzb_key\":\"S3CRETv41\"}"#,
            "first line\nsecond http://h/x?apikey=S3CRETv41",
            "body\u{1b}[0m\u{1b}[2m=\u{1b}[0mx \u{1b}[3mpassword\u{1b}[0m\u{1b}[2m=\u{1b}[0mS3CRETv41",
        ];
        for case in cases {
            let out = cleanse_log_line(case);
            assert!(!out.contains("S3CRETv41"), "{case:?} -> {out:?}");
            assert!(out.contains("[REDACTED]"), "{case:?} -> {out:?}");
        }
    }

    #[test]
    fn log_cleanser_keeps_ordinary_text_byte_identical() {
        for text in [
            "http://host:9696/api?t=search&cat=7000",
            "http://host:9696/path",
            "http://h/x?key=OL123W&q=dune",
            "http://h/x?author=Herbert&q=dune",
            "Ordinary text: Dune (1965), 412 pages.",
            "login successful username=fresh-author user_id=1",
            "user@example.com",
        ] {
            assert_eq!(cleanse_log_line(text), text);
        }
    }

    #[test]
    fn log_cleanser_keeps_a_json_event_parseable() {
        let event = serde_json::json!({
            "fields": {
                "message": "Unknown config key: x?apikey=ab\\cd\"ef",
                "body": r#"{"api_key":"S3CRETv41","nested":{"token":"T0KENv41"}}"#,
                "username": "u Authorization: Bearer B3ARERv41",
            }
        })
        .to_string();
        let out = cleanse_log_line(&event);
        let parsed: serde_json::Value = serde_json::from_str(&out).expect(&out);
        let text = parsed.to_string();
        for secret in ["S3CRETv41", "T0KENv41", "B3ARERv41", "ab"] {
            assert!(!text.contains(secret), "{secret} in {out}");
        }
    }

    #[test]
    fn redact_secrets_output_is_unchanged_for_shapes_only_the_log_cleanser_masks() {
        let warning = "error 100: access_token=abc";
        assert_eq!(redact_secrets(warning), warning);
    }

    /// Parses a cleansed JSON event, failing with the line when it does not parse.
    fn parse_event(out: &str) -> serde_json::Value {
        serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"))
    }

    #[test]
    fn log_cleanser_masks_a_quoted_structured_field_and_keeps_its_quotes() {
        let plain = r#"WARN quoted field password="QUOTEDSECRET""#;
        assert_eq!(
            cleanse_log_line(plain),
            r#"WARN quoted field password="[REDACTED]""#
        );

        let ansi = "\u{1b}[33m WARN\u{1b}[0m quoted field \u{1b}[3mpassword\u{1b}[0m\
                    \u{1b}[2m=\u{1b}[0m\"QUOTEDSECRET\"";
        assert_eq!(
            cleanse_log_line(ansi),
            "\u{1b}[33m WARN\u{1b}[0m quoted field \u{1b}[3mpassword\u{1b}[0m\
             \u{1b}[2m=\u{1b}[0m\"[REDACTED]\""
        );

        let debug = format!(
            "WARN debug field password={:?}",
            String::from("DEBUGSECRET")
        );
        assert_eq!(
            cleanse_log_line(&debug),
            r#"WARN debug field password="[REDACTED]""#
        );
    }

    #[test]
    fn log_cleanser_masks_quoted_fields_inside_a_json_event() {
        let debug_field = serde_json::json!({
            "fields": {
                "message": "debug field",
                "password": format!("{:?}", String::from("DEBUGSECRET")),
            }
        })
        .to_string();
        let out = cleanse_log_line(&debug_field);
        let parsed = parse_event(&out);
        assert!(!out.contains("DEBUGSECRET"), "{out}");
        assert_eq!(parsed["fields"]["password"], "\"[REDACTED]\"", "{out}");

        let fragment = serde_json::json!({
            "fields": { "message": "Unknown config key: password=\"QUOTEDSECRET\"" }
        })
        .to_string();
        let out = cleanse_log_line(&fragment);
        let parsed = parse_event(&out);
        assert!(!out.contains("QUOTEDSECRET"), "{out}");
        assert_eq!(
            parsed["fields"]["message"], "Unknown config key: password=\"[REDACTED]\"",
            "{out}"
        );
    }

    #[test]
    fn log_cleanser_keeps_a_json_string_ending_in_a_field_name_intact() {
        let event = r#"{"fields":{"message":"set token=","level":"x"}}"#;
        assert_eq!(cleanse_log_line(event), event);
    }

    #[test]
    fn log_cleanser_masks_every_structured_field_name_in_a_json_event() {
        let event = serde_json::json!({
            "fields": {
                "message": "field names",
                "auth": "AUTHSECRET",
                "Passwd": "PASSWDSECRET",
                "AUTHKEY": "AUTHKEYSECRET",
                "access_token": "ACCESSSECRET",
                "id": "ordinary-7",
            }
        })
        .to_string();
        let out = cleanse_log_line(&event);
        let parsed = parse_event(&out);
        for secret in [
            "AUTHSECRET",
            "PASSWDSECRET",
            "AUTHKEYSECRET",
            "ACCESSSECRET",
        ] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(out.contains("[REDACTED]"), "{out}");
        assert_eq!(parsed["fields"]["id"], "ordinary-7", "{out}");
        assert_eq!(parsed["fields"]["message"], "field names", "{out}");
    }

    #[test]
    fn log_cleanser_masks_numeric_structured_fields_in_a_json_event() {
        let event = serde_json::json!({
            "fields": { "message": "numeric field", "password": 123456789, "token": 987654321, "count": 42 }
        })
        .to_string();
        let out = cleanse_log_line(&event);
        let parsed = parse_event(&out);
        for secret in ["123456789", "987654321"] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert_eq!(parsed["fields"]["password"], "[REDACTED]", "{out}");
        assert_eq!(parsed["fields"]["count"], 42, "{out}");

        let carried = serde_json::json!({
            "fields": { "body": r#"{"auth":"AUTHSECRET","token":24681357,"code":401}"# }
        })
        .to_string();
        let out = cleanse_log_line(&carried);
        let parsed = parse_event(&out);
        for secret in ["AUTHSECRET", "24681357"] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(
            parsed["fields"]["body"]
                .as_str()
                .unwrap()
                .contains("\"code\":401"),
            "{out}"
        );
    }

    #[test]
    fn log_cleanser_masks_userinfo_before_a_quote_or_backslash() {
        for url in [
            r"https://user:prefix'suffix@host/path",
            r#"https://user:prefix"suffix@host/path"#,
            r"https://user:prefix\suffix@host/path",
        ] {
            let out = cleanse_log_line(&format!("Unknown config key: probe {url}"));
            assert!(!out.contains("prefix"), "{url} -> {out}");
            assert!(out.contains("[REDACTED]"), "{url} -> {out}");
            assert!(out.contains("@host/path"), "{url} -> {out}");

            let event = serde_json::json!({
                "fields": { "message": format!("Unknown config key: probe {url}") }
            })
            .to_string();
            let out = cleanse_log_line(&event);
            let parsed = parse_event(&out);
            let message = parsed["fields"]["message"].as_str().unwrap();
            assert!(!message.contains("prefix"), "{url} -> {out}");
            assert!(message.contains("[REDACTED]"), "{url} -> {out}");
            assert!(message.contains("@host/path"), "{url} -> {out}");
        }
    }

    #[test]
    fn log_cleanser_keeps_host_and_port_urls_beside_quotes_byte_identical() {
        for text in [
            r#"{"fields":{"url":"http://host:9696","email":"me@example.com"}}"#,
            r#"url="http://host:9696" user="me@example.com""#,
            r#"{"fields":{"url":"http://host:9696"},"span":{"who":"me@example.com"}}"#,
        ] {
            assert_eq!(cleanse_log_line(text), text);
        }
    }

    #[test]
    fn log_cleanser_keeps_a_url_with_an_escaped_path_boundary_byte_identical() {
        let mut changed = Vec::new();
        for text in [
            r#"{"url":"http://host:9696\/dir@host/path","id":7}"#,
            r#"{"url":"http://host:9696\u002fdir@host/path","id":7}"#,
            r#"{"url":"http://host:9696\u002Fdir@host/path","id":7}"#,
            r#"{"url":"http://host:9696\u003fq=dir@host/path","id":7}"#,
            r#"{"url":"http://host:9696\u0023dir@host/path","id":7}"#,
            r"probe http://host:9696\?q=dir@host/path",
            r"probe http://host:9696\#dir@host/path",
        ] {
            let out = cleanse_log_line(text);
            if out != text {
                changed.push(format!("{text} -> {out}"));
            }
        }
        assert!(changed.is_empty(), "{changed:#?}");
    }
}
