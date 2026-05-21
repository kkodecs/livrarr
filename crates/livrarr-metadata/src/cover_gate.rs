pub const COVER_GATE_JACCARD_THRESHOLD: f64 = 0.6;

#[derive(Debug, Clone, PartialEq)]
pub enum CoverGateOutcome {
    Apply {
        jaccard: f64,
        via: GateReason,
    },
    Skip {
        jaccard: f64,
        via: GateReason,
    },
    AskLlm {
        jaccard: f64,
        prompt_inputs: LlmPromptInputs,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateReason {
    DeterministicAccept,
    DeterministicSkipNoLlm,
    LlmAccepted,
    LlmRejected,
    LlmCallFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmDecision {
    SameBook,
    NotSameBook,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmPromptInputs {
    pub ol_title: String,
    pub ol_author: String,
    pub ol_year: Option<i32>,
    pub ol_isbn: Option<String>,
    pub gr_title: String,
    pub gr_author: String,
    pub gr_year: Option<i32>,
    pub gr_isbn: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OlAnchor<'a> {
    pub title: &'a str,
    pub author_name: &'a str,
    pub year: Option<i32>,
    pub isbn: Option<&'a str>,
    pub ol_key: &'a str,
}

#[derive(Debug, Clone)]
pub struct GrCandidate<'a> {
    pub title: &'a str,
    pub author_name: &'a str,
    pub year: Option<i32>,
    pub isbn: Option<&'a str>,
    pub gr_key: &'a str,
}

pub fn evaluate_gr_cover_gate<'a>(
    _anchor: &OlAnchor<'a>,
    _candidate: &GrCandidate<'a>,
    _llm_configured: bool,
) -> CoverGateOutcome {
    todo!()
}

pub fn apply_llm_decision(_decision: LlmDecision, _jaccard: f64) -> CoverGateOutcome {
    todo!()
}
