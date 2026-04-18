use std::collections::HashMap;

use livrarr_domain::services::{
    LlmCallRequest, LlmCallResponse, LlmCaller, LlmError as DomainLlmError, LlmField, LlmValue,
};
use livrarr_http::HttpClient;

/// Concrete implementation of the domain `LlmCaller` trait.
///
/// Calls an OpenAI-compatible chat/completions endpoint, rendering
/// caller-supplied templates with context values before sending.
pub struct LlmCallerImpl {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    client: HttpClient,
}

impl LlmCallerImpl {
    pub fn new(
        endpoint: Option<String>,
        api_key: Option<String>,
        model: Option<String>,
        client: HttpClient,
    ) -> Self {
        Self {
            endpoint,
            api_key,
            model,
            client,
        }
    }

    /// Convenience constructor for no-LLM mode.
    pub fn not_configured() -> Self {
        Self {
            endpoint: None,
            api_key: None,
            model: None,
            client: HttpClient::builder()
                .build()
                .expect("default HttpClient build"),
        }
    }
}

impl LlmCaller for LlmCallerImpl {
    async fn call(&self, req: LlmCallRequest) -> Result<LlmCallResponse, DomainLlmError> {
        // 1. Validate allowed_fields before any network call.
        for key in req.context.keys() {
            if !req.allowed_fields.contains(key) {
                return Err(DomainLlmError::DisallowedField { field: *key });
            }
        }

        // 2. Check configuration.
        let endpoint = self
            .endpoint
            .as_deref()
            .ok_or(DomainLlmError::NotConfigured)?;
        let api_key = self
            .api_key
            .as_deref()
            .ok_or(DomainLlmError::NotConfigured)?;
        let model = self.model.as_deref().unwrap_or("gpt-4o");

        // 3. Render templates.
        let system_prompt = render_template(req.system_template, &req.context);
        let user_prompt = render_template(req.user_template, &req.context);

        // 4. Build request body.
        let url = format!(
            "{}chat/completions",
            endpoint.trim_end_matches('/').to_owned() + "/"
        );

        let body = serde_json::json!({
            "model": model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt },
            ],
            "max_tokens": 4000,
            "temperature": 0.0,
        });

        tracing::debug!(purpose = ?req.purpose, "sending LLM request");

        // 5. Send with timeout.
        let start = std::time::Instant::now();

        let resp = tokio::time::timeout(
            req.timeout,
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| DomainLlmError::Timeout)?
        .map_err(|e| DomainLlmError::Provider(e.to_string()))?;

        let elapsed = start.elapsed();

        // 6. Check HTTP status.
        if !resp.status().is_success() {
            return Err(DomainLlmError::Provider(format!("HTTP {}", resp.status())));
        }

        // 7. Parse response.
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| DomainLlmError::InvalidResponse(e.to_string()))?;

        let content = data
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            return Err(DomainLlmError::InvalidResponse(
                "empty content in response".into(),
            ));
        }

        let model_used = data
            .pointer("/model")
            .and_then(|v| v.as_str())
            .unwrap_or(model)
            .to_string();

        tracing::debug!(
            purpose = ?req.purpose,
            model_used = %model_used,
            elapsed_ms = elapsed.as_millis(),
            "LLM call complete"
        );

        Ok(LlmCallResponse {
            content,
            model_used,
            elapsed,
        })
    }
}

/// Simple template renderer: replaces `{field_name}` with stringified value.
fn render_template(template: &str, context: &HashMap<LlmField, LlmValue>) -> String {
    let mut result = template.to_string();
    for (field, value) in context {
        let placeholder = format!("{{{}}}", field_name(field));
        let replacement = stringify_value(value);
        result = result.replace(&placeholder, &replacement);
    }
    result
}

fn field_name(field: &LlmField) -> &'static str {
    match field {
        LlmField::Title => "title",
        LlmField::AuthorName => "author_name",
        LlmField::Description => "description",
        LlmField::SeriesName => "series_name",
        LlmField::Genres => "genres",
        LlmField::Language => "language",
        LlmField::Publisher => "publisher",
        LlmField::Year => "year",
        LlmField::Isbn => "isbn",
        LlmField::SearchResults => "search_results",
        LlmField::BibliographyHtml => "bibliography_html",
        LlmField::ProviderName => "provider_name",
        LlmField::CandidateTitle => "candidate_title",
        LlmField::CandidateAuthor => "candidate_author",
    }
}

fn stringify_value(value: &LlmValue) -> String {
    match value {
        LlmValue::Text(s) => s.clone(),
        LlmValue::Number(n) => n.to_string(),
        LlmValue::TextList(items) => items.join(", "),
    }
}
