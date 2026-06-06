//! Title cleanup — LLM-assisted polish layer.
//!
//! Pure-function primitives (`clean_title`, `clean_author`, regex consts, etc.)
//! live in `livrarr_domain::title_cleanup`. This module re-exports them and adds
//! the LLM-assisted polish path that depends on `HttpClient`.

pub use livrarr_domain::title_cleanup::{
    capitalize_first, clean_author, clean_title, collapse_whitespace, normalize_last_first,
    strip_colon_novel_marker, strip_colon_series_marker, strip_trailing_paren_if_match, title_case,
    title_case_word, SMALL_WORDS,
};

use livrarr_http::HttpClient;
use serde::Deserialize;
use std::time::Duration;

/// 2.5s ceiling on the LLM call per the design spec — keeps add-work fast.
const LLM_POLISH_TIMEOUT: Duration = Duration::from_millis(2_500);

const POLISH_SYSTEM_INSTRUCTION: &str = r#"You are a metadata normalization tool for a book library system. Given a raw book title and author from a search result, return cleaned canonical forms.

Return ONLY a JSON object with EXACTLY these fields (these names are required):
{
  "title": "<cleaned title string>",
  "author_name": "<cleaned author string>",
  "series_name": "<series name or null>",
  "series_position": <number or null>
}

Cleanup rules for "title":
- Apply proper Title Case. Capitalize the first letter of every significant word. Keep small words ("the", "of", "and", "a", "an", "to", "in", "on", "or", "for", "with", "but", "as", "by", "at", "from") lowercase EXCEPT when they are the first or last word of the title or subtitle.
- Examples: "the power broker: robert moses and the fall of new york" → "The Power Broker: Robert Moses and the Fall of New York". "DUNE" → "Dune". "the way of kings" → "The Way of Kings".
- Preserve intentionally stylized words that contain INTERNAL uppercase letters (e.g. "iCon", "iPhone", "MacBook", "eBay") — leave those words exactly as written.
- Strip trailing parentheticals matching: series info like "(Series Name, #N)", edition markers like "(Deluxe Edition)", "(Hardcover)", "(Paperback)", "(Audiobook)", "(Unabridged)", year markers like "(1963)".
- Strip series-marker suffixes after a colon like ": Book Two of the Expanse", ": Volume 1", ": A Novel". DO NOT strip substantive descriptive subtitles like "Robert Moses and the Fall of New York" or "Steve Jobs, The Greatest Second Act in the History of Business" — those ARE part of the work identity and must be preserved.
- Trim and collapse internal whitespace.

Cleanup rules for "author_name":
- Normalize "Last, First" → "First Last".
- Fix all-caps or all-lowercase author names to proper Name Case.
- Preserve initials and punctuation (e.g. "Robert A. Caro", "J.R.R. Tolkien").

Extract "series_name" and "series_position" ONLY when explicitly present in the raw title (e.g. inside parens like "(The Expanse, #2)"). Use null when not present.

Output ONLY the JSON object. No markdown code fences. No commentary."#;

/// Polished add-time output. `title` and `author_name` are the locked
/// identity anchor values. `series_name` and `series_position` are extracted
/// when present in the input — usable to populate the Work record at add-time.
#[derive(Debug, Clone)]
pub struct PolishedAddTime {
    pub title: String,
    pub author_name: String,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct LlmPolishResponse {
    title: String,
    author_name: String,
    #[serde(default)]
    series_name: Option<String>,
    #[serde(default)]
    series_position: Option<f64>,
}

/// Polish a raw title + author at add-time using the LLM if configured.
///
/// On any failure (LLM not configured, timeout, HTTP error, malformed
/// response), falls back to the deterministic `clean_title` / `clean_author`
/// pair. Per project Principle 11, LLM is value-add and never gatekeeps.
///
/// `llm_config` is `(endpoint, api_key, model)`. Pass None to skip the LLM
/// attempt entirely (purely deterministic path). The endpoint is the
/// OpenAI-compat base URL (e.g. `https://api.groq.com/openai/v1`,
/// `https://generativelanguage.googleapis.com/v1beta/openai`).
pub async fn polish_addtime(
    http: &HttpClient,
    llm_config: Option<(&str, &str, &str)>,
    raw_title: &str,
    raw_author: &str,
) -> PolishedAddTime {
    let deterministic = || PolishedAddTime {
        title: clean_title(raw_title),
        author_name: clean_author(raw_author),
        series_name: None,
        series_position: None,
    };

    let Some((endpoint, api_key, model)) = llm_config else {
        return deterministic();
    };

    match tokio::time::timeout(
        LLM_POLISH_TIMEOUT,
        call_polish_llm(http, endpoint, api_key, model, raw_title, raw_author),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            tracing::info!(
                raw_title,
                raw_author,
                "LLM add-time polish failed, falling back to deterministic: {e}"
            );
            deterministic()
        }
        Err(_) => {
            tracing::info!(
                raw_title,
                raw_author,
                "LLM add-time polish timed out (>2.5s), falling back to deterministic"
            );
            deterministic()
        }
    }
}

async fn call_polish_llm(
    http: &HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    raw_title: &str,
    raw_author: &str,
) -> Result<PolishedAddTime, String> {
    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let user_msg = format!("title_raw: {raw_title}\nauthor_raw: {raw_author}");
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": POLISH_SYSTEM_INSTRUCTION},
            {"role": "user",   "content": user_msg},
        ],
        "temperature": 0.0,
        "response_format": {"type": "json_object"},
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }
    let envelope: serde_json::Value = resp.json().await.map_err(|e| format!("envelope: {e}"))?;
    let content_raw = envelope
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or("missing choices[0].message.content")?;
    // Tolerate code-fence wrapping that some providers add.
    let trimmed = content_raw.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let unfenced = unfenced.strip_suffix("```").unwrap_or(unfenced).trim();
    let parsed: LlmPolishResponse =
        serde_json::from_str(unfenced).map_err(|e| format!("inner parse: {e}"))?;
    Ok(PolishedAddTime {
        title: parsed.title,
        author_name: parsed.author_name,
        series_name: parsed.series_name,
        series_position: parsed.series_position,
    })
}
