//! LLM-based extraction fallback for Goodreads detail pages when direct
//! JSON-LD/regex parsing yields nothing useful — used primarily for
//! foreign-language pages whose HTML structure differs from the English
//! layout.

use livrarr_http::HttpClient;

use super::client::GoodreadsFetchError;

// =============================================================================
// LLM extraction fallback (foreign-language path)
// =============================================================================

/// System prompt for LLM-driven extraction from a foreign-language Goodreads
/// detail page. Used when direct JSON-LD + regex parsing fails (often on
/// foreign locales where GR's HTML structure differs).
///
/// The prompt is language-aware: it instructs the model to filter out
/// descriptions in unexpected languages so the validator's language guard
/// has clean inputs to work with.
const FOREIGN_LLM_EXTRACTION_PROMPT: &str = r#"You are a metadata extraction tool. Extract book details from the provided book detail page HTML.

Return ONLY a JSON object with exactly these fields:
- "title": string or null (book title in the work's primary language)
- "author": string or null (author name)
- "description": string or null (book description/synopsis, plain text, no HTML)
- "series_name": string or null (series name if this book is part of a series)
- "series_position": number or null (position in the series, e.g. 1, 2, 3)
- "genres": array of strings or null (genre/shelf tags, max 10)
- "page_count": integer or null
- "publisher": string or null
- "publish_date": string or null (in YYYY-MM-DD or YYYY format)
- "cover_url": string or null (full URL of the largest/highest resolution cover image)
- "rating": number or null (average rating, typically 1-5 scale)
- "rating_count": integer or null (number of ratings)
- "isbn": string or null (ISBN-13 if visible)
- "language": string (ISO 639-1 code) or null

Rules:
- Return ONLY the JSON object, no markdown fences, no explanation
- If a field is not visible on the page, use null
- Do NOT invent or guess missing data
- For cover_url, prefer the largest image version available
- For description, use ONLY text in the work's expected language (the language hint provided in the user message) or English. If the description is in another language, return null.
- For genres, use the most specific applicable tags"#;

/// LLM extraction response shape from Gemini.
#[derive(Debug, serde::Deserialize)]
struct LlmExtractionResult {
    title: Option<String>,
    author: Option<String>,
    description: Option<String>,
    series_name: Option<String>,
    series_position: Option<f64>,
    genres: Option<Vec<String>>,
    page_count: Option<i32>,
    publisher: Option<String>,
    publish_date: Option<String>,
    cover_url: Option<String>,
    rating: Option<f64>,
    rating_count: Option<i32>,
    isbn: Option<String>,
    language: Option<String>,
}

/// Extract foreign-language metadata from raw HTML using the configured LLM
/// (provider-agnostic OpenAI-compat).
///
/// Used as a fallback inside `GoodreadsClient::fetch` when direct JSON-LD +
/// regex parsing returns nothing useful.
///
/// Privacy: the prompt body contains only the cleaned page HTML and a
/// language hint. NO user-private fields (filenames, work IDs, etc.).
///
/// `endpoint` is the OpenAI-compat base URL the user configured (e.g.
/// `https://api.groq.com/openai/v1`,
/// `https://generativelanguage.googleapis.com/v1beta/openai`,
/// `https://api.openai.com/v1`). The function appends `/chat/completions`.
///
/// `language_hint` should be the work's expected language English-name (e.g.
/// "French", "Japanese") or "the original" if unknown — used to tell the LLM
/// which-language description to keep / drop.
///
/// `page_url` is the absolute detail-page URL the HTML was fetched from; it is
/// the base against which a relative `cover_url` from the LLM is resolved to an
/// absolute URL. A relative cover that cannot be resolved is dropped.
pub async fn extract_with_llm(
    http: &HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    raw_html: &str,
    language_hint: &str,
    page_url: &str,
) -> Result<crate::NormalizedWorkDetail, GoodreadsFetchError> {
    let cleaned = crate::provider_util::clean_html_for_llm(raw_html);
    if cleaned.is_empty() {
        return Err(GoodreadsFetchError::Parse);
    }

    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let user_msg = format!(
        "This book is in {language_hint}. Extract book details from this page. \
         For the description, use ONLY text in {language_hint} or English. \
         If the description is in a different language, return null for description.\n\n{cleaned}"
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": FOREIGN_LLM_EXTRACTION_PROMPT},
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
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| GoodreadsFetchError::Network(format!("LLM extract: {e}")))?;
    if !resp.status().is_success() {
        return Err(GoodreadsFetchError::HttpStatus(resp.status().as_u16()));
    }
    let envelope: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GoodreadsFetchError::Network(format!("LLM envelope: {e}")))?;
    let content_raw = envelope
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or(GoodreadsFetchError::Parse)?;
    // Tolerate code-fence wrapping that some providers add.
    let trimmed = content_raw.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let unfenced = unfenced.strip_suffix("```").unwrap_or(unfenced).trim();
    let result: LlmExtractionResult =
        serde_json::from_str(unfenced).map_err(|_| GoodreadsFetchError::Parse)?;

    let nfc = crate::normalize::nfc;
    let year = result
        .publish_date
        .as_deref()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse::<i32>().ok());
    let cover_url = result
        .cover_url
        .as_deref()
        .and_then(|u| crate::provider_util::validate_cover_url(u, page_url));

    Ok(crate::NormalizedWorkDetail {
        title: result.title.map(|s| nfc(&s)),
        subtitle: None,
        original_title: None,
        author_name: result.author.map(|s| nfc(&s)),
        description: result.description.map(|s| nfc(&s)),
        year,
        series_name: result.series_name.map(|s| nfc(&s)),
        series_position: result.series_position,
        genres: result
            .genres
            .map(|g| g.into_iter().map(|s| nfc(&s)).collect()),
        language: result
            .language
            .as_deref()
            .map(livrarr_domain::normalize_language),
        page_count: result.page_count.filter(|&p| p > 0),
        duration_seconds: None,
        publisher: result.publisher.map(|s| nfc(&s)),
        publish_date: result.publish_date,
        hc_key: None,
        gr_key: None,
        ol_key: None,
        isbn_13: result.isbn.filter(|s| s.len() >= 10),
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: result.rating,
        rating_count: result.rating_count,
        cover_url,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    })
}
