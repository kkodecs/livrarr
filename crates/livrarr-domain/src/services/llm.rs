use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum LlmValue {
    Text(String),
    Number(i64),
    TextList(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmField {
    Title,
    AuthorName,
    Description,
    SeriesName,
    Genres,
    Language,
    Publisher,
    Year,
    Isbn,
    SearchResults,
    BibliographyHtml,
    ProviderName,
    CandidateTitle,
    CandidateAuthor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmPurpose {
    IdentityValidation,
    SearchResultCleanup,
    BibliographyCleanup,
}

#[derive(Debug)]
pub struct LlmCallRequest {
    pub system_template: &'static str,
    pub user_template: &'static str,
    pub context: HashMap<LlmField, LlmValue>,
    pub allowed_fields: &'static [LlmField],
    pub timeout: Duration,
    pub purpose: LlmPurpose,
}

#[derive(Debug)]
pub struct LlmCallResponse {
    pub content: String,
    pub model_used: String,
    pub elapsed: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("LLM not configured")]
    NotConfigured,
    #[error("disallowed field in context: {field:?}")]
    DisallowedField { field: LlmField },
    #[error("provider error: {0}")]
    Provider(String),
    #[error("timeout")]
    Timeout,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[trait_variant::make(Send)]
pub trait LlmCaller: Send + Sync {
    async fn call(&self, req: LlmCallRequest) -> Result<LlmCallResponse, LlmError>;
}
