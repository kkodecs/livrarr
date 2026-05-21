use livrarr_domain::text_norm;

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

pub fn evaluate_gr_cover_gate(
    anchor: &OlAnchor<'_>,
    candidate: &GrCandidate<'_>,
    llm_enabled: bool,
) -> CoverGateOutcome {
    let anchor_tokens = text_norm::title_tokens(anchor.title);
    let candidate_tokens = text_norm::title_tokens(candidate.title);
    let jaccard = text_norm::jaccard(&anchor_tokens, &candidate_tokens);

    if jaccard >= COVER_GATE_JACCARD_THRESHOLD {
        return CoverGateOutcome::Apply {
            jaccard,
            via: GateReason::DeterministicAccept,
        };
    }

    if !llm_enabled {
        return CoverGateOutcome::Skip {
            jaccard,
            via: GateReason::DeterministicSkipNoLlm,
        };
    }

    CoverGateOutcome::AskLlm {
        jaccard,
        prompt_inputs: LlmPromptInputs {
            ol_title: anchor.title.to_string(),
            ol_author: anchor.author_name.to_string(),
            ol_year: anchor.year,
            ol_isbn: anchor.isbn.map(String::from),
            gr_title: candidate.title.to_string(),
            gr_author: candidate.author_name.to_string(),
            gr_year: candidate.year,
            gr_isbn: candidate.isbn.map(String::from),
        },
    }
}

pub fn apply_llm_decision(decision: LlmDecision, jaccard: f64) -> CoverGateOutcome {
    match decision {
        LlmDecision::SameBook => CoverGateOutcome::Apply {
            jaccard,
            via: GateReason::LlmAccepted,
        },
        LlmDecision::NotSameBook => CoverGateOutcome::Skip {
            jaccard,
            via: GateReason::LlmRejected,
        },
        LlmDecision::Failed => CoverGateOutcome::Skip {
            jaccard,
            via: GateReason::LlmCallFailed,
        },
    }
}
