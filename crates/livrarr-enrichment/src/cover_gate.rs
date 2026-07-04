use livrarr_domain::text_norm;

pub const COVER_GATE_JACCARD_THRESHOLD: f64 = 0.6;

#[derive(Debug, Clone, PartialEq)]
pub enum CoverGateOutcome {
    Apply { jaccard: f64, via: GateReason },
    Skip { jaccard: f64, via: GateReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateReason {
    DeterministicAccept,
    DeterministicSkipNoLlm,
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

/// Deterministic cover-acceptance gate (REQ-017/D5). A Goodreads cover
/// survives only if its title clears the Jaccard bar against the anchor
/// title; a borderline title is a strip, full stop — there is no LLM tier
/// here (REQ-016/D10: no live LLM-chooses-match remains anywhere).
pub fn evaluate_gr_cover_gate(
    anchor: &OlAnchor<'_>,
    candidate: &GrCandidate<'_>,
) -> CoverGateOutcome {
    let anchor_tokens = text_norm::title_tokens(anchor.title);
    let candidate_tokens = text_norm::title_tokens(candidate.title);
    let jaccard = text_norm::jaccard(&anchor_tokens, &candidate_tokens);

    if jaccard >= COVER_GATE_JACCARD_THRESHOLD {
        CoverGateOutcome::Apply {
            jaccard,
            via: GateReason::DeterministicAccept,
        }
    } else {
        CoverGateOutcome::Skip {
            jaccard,
            via: GateReason::DeterministicSkipNoLlm,
        }
    }
}
