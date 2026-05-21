use crate::cover_gate::{LlmDecision, LlmPromptInputs};
use livrarr_domain::services::{LlmCallRequest, LlmCaller, LlmPurpose};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

const SYSTEM_TEMPLATE: &str = "You are a metadata disambiguation assistant. Given identity information for a known book and a candidate book from a third-party source, decide whether they refer to the same book. Be conservative: a derivative work (study guide, workbook, summary, analysis, audio adaptation by a different reader, foreign-language translation under a different author name) is NOT the same book as the original. You will receive a JSON object with two records: `known` (the authoritative identity, sourced from OpenLibrary) and `candidate` (the record we are evaluating). Reply with a single JSON object matching the output schema. No prose outside the JSON.";

const FIELD_CAP: usize = 1024;

#[derive(Debug, Deserialize)]
struct AskSameBookOutput {
    same_book: bool,
    #[allow(dead_code)]
    reason: String,
}

pub async fn ask_same_book<L: LlmCaller>(
    llm: &L,
    prompt_inputs: &LlmPromptInputs,
    _llm_configured: bool,
) -> LlmDecision {
    let user_prompt = build_user_prompt(prompt_inputs);

    let req = LlmCallRequest {
        system_template: SYSTEM_TEMPLATE.to_string(),
        user_template: user_prompt,
        context: HashMap::new(),
        allowed_fields: &[],
        timeout: Duration::from_secs(10),
        purpose: LlmPurpose::CoverDisambiguation,
    };

    let response = match llm.call(req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("LLM ask_same_book failed: {e}");
            return LlmDecision::Failed;
        }
    };

    let content = response.content.trim();
    let unfenced = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```"))
        .unwrap_or(content);
    let unfenced = unfenced.strip_suffix("```").unwrap_or(unfenced).trim();

    match serde_json::from_str::<AskSameBookOutput>(unfenced) {
        Ok(output) => {
            if output.same_book {
                LlmDecision::SameBook
            } else {
                LlmDecision::NotSameBook
            }
        }
        Err(_) => {
            let preview = &response.content[..response.content.len().min(500)];
            tracing::warn!(
                content_preview = preview,
                "LLM returned unparseable response"
            );
            LlmDecision::Failed
        }
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

fn build_user_prompt(inputs: &LlmPromptInputs) -> String {
    let known = serde_json::json!({
        "title": truncate(&inputs.ol_title, FIELD_CAP),
        "author": truncate(&inputs.ol_author, FIELD_CAP),
        "year": inputs.ol_year,
        "isbn_13": inputs.ol_isbn.as_deref().map(|s| truncate(s, FIELD_CAP)),
    });

    let candidate = serde_json::json!({
        "title": truncate(&inputs.gr_title, FIELD_CAP),
        "author": truncate(&inputs.gr_author, FIELD_CAP),
        "year": inputs.gr_year,
        "isbn_13": inputs.gr_isbn.as_deref().map(|s| truncate(s, FIELD_CAP)),
    });

    serde_json::json!({
        "known": known,
        "candidate": candidate,
    })
    .to_string()
}
