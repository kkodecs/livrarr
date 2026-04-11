use std::time::Duration;

use crate::{LlmClient, LlmError, LlmMessage};
use livrarr_http::HttpClient;

/// Concrete LLM client that calls an OpenAI-compatible chat/completions endpoint.
pub struct HttpLlmClient {
    http: HttpClient,
    endpoint: String,
    api_key: String,
    model: String,
}

impl HttpLlmClient {
    pub fn new(http: HttpClient, endpoint: String, api_key: String, model: String) -> Self {
        Self {
            http,
            endpoint,
            api_key,
            model,
        }
    }
}

impl LlmClient for HttpLlmClient {
    async fn chat_completion(&self, messages: Vec<LlmMessage>) -> Result<String, LlmError> {
        let url = format!(
            "{}chat/completions",
            self.endpoint.trim_end_matches('/').to_owned() + "/"
        );

        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": match m.role {
                        crate::LlmRole::System => "system",
                        crate::LlmRole::User => "user",
                        crate::LlmRole::Assistant => "assistant",
                    },
                    "content": m.content,
                })
            })
            .collect();

        let body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "max_tokens": 4000,
            "temperature": 0.0,
        });

        let resp = tokio::time::timeout(
            Duration::from_secs(45),
            self.http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| LlmError::Timeout(Duration::from_secs(45)))?
        .map_err(|e| LlmError::RequestFailed(e.to_string()))?;

        if resp.status().as_u16() == 429 {
            return Err(LlmError::RateLimited);
        }

        if !resp.status().is_success() {
            return Err(LlmError::RequestFailed(format!("HTTP {}", resp.status())));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let content = data
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LlmError::InvalidResponse("missing choices[0].message.content".into()))?
            .to_string();

        Ok(content)
    }
}
