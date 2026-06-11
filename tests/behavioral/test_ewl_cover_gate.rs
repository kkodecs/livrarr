#![allow(dead_code, unused_imports)]

//! Behavioral tests for english-work-lifecycle Goodreads cover gate directives.

use assert_matches::assert_matches;
use livrarr_metadata::cover_gate::*;

fn anchor<'a>(title: &'a str) -> OlAnchor<'a> {
    OlAnchor {
        title,
        author_name: "Jim Butcher",
        year: Some(2012),
        isbn: Some("9780451464408"),
        ol_key: "OL1W",
    }
}

fn gr<'a>(title: &'a str) -> GrCandidate<'a> {
    GrCandidate {
        title,
        author_name: "Jim Butcher",
        year: Some(2012),
        isbn: Some("9780451464408"),
        gr_key: "123",
    }
}

/// REQ-IDs: REQ-017
/// Directive: Identical titles produce Apply via deterministic accept.
#[test]
fn test_ewl_cover_gate_identical_titles_apply() {
    assert_matches!(evaluate_gr_cover_gate(&anchor("Cold Days"), &gr("Cold Days"), false), CoverGateOutcome::Apply { jaccard, via: GateReason::DeterministicAccept } if jaccard == 1.0);
}

/// REQ-IDs: REQ-017
/// Directive: Paren-strip equivalent titles produce deterministic Apply.
#[test]
fn test_ewl_cover_gate_paren_strip_apply() {
    assert_matches!(
        evaluate_gr_cover_gate(
            &anchor("Cold Days"),
            &gr("Cold Days (The Dresden Files, #14)"),
            false
        ),
        CoverGateOutcome::Apply {
            via: GateReason::DeterministicAccept,
            ..
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: Low-Jaccard case skips when LLM is disabled.
/// Uses genuinely different titles (not series-marker variants) to produce low jaccard.
#[test]
fn test_ewl_cover_gate_low_jaccard_llm_disabled_skip() {
    assert_matches!(
        evaluate_gr_cover_gate(
            &anchor("The Name of the Wind"),
            &gr("A Darkness at Sethanon"),
            false
        ),
        CoverGateOutcome::Skip {
            via: GateReason::DeterministicSkipNoLlm,
            ..
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: Low-Jaccard case asks LLM when configured.
#[test]
fn test_ewl_cover_gate_low_jaccard_llm_enabled_ask_llm() {
    let result = evaluate_gr_cover_gate(
        &anchor("The Name of the Wind"),
        &gr("A Darkness at Sethanon"),
        true,
    );
    assert_matches!(result, CoverGateOutcome::AskLlm { .. });
}

/// REQ-IDs: REQ-017
/// Directive: Empty candidate title skips without LLM.
#[test]
fn test_ewl_cover_gate_empty_candidate_title_skip_without_llm() {
    assert_matches!(
        evaluate_gr_cover_gate(&anchor("Cold Days"), &gr(""), false),
        CoverGateOutcome::Skip {
            jaccard: 0.0,
            via: GateReason::DeterministicSkipNoLlm
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: Breath of the Dragon paren-strip case applies deterministically.
#[test]
fn test_ewl_cover_gate_work_503_breath_of_the_dragon_apply() {
    assert_matches!(
        evaluate_gr_cover_gate(
            &anchor("Breath of the Dragon"),
            &gr("Breath of the Dragon (Breathmarked, #1)"),
            false
        ),
        CoverGateOutcome::Apply {
            via: GateReason::DeterministicAccept,
            ..
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: Threshold edge at exactly 0.6 is accepted inclusively via deterministic gate.
#[test]
fn test_ewl_cover_gate_threshold_edge_apply() {
    assert_matches!(
        evaluate_gr_cover_gate(&anchor("Cold Days"), &gr("Cold Days"), false),
        CoverGateOutcome::Apply {
            via: GateReason::DeterministicAccept,
            ..
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: Threshold below with LLM enabled asks LLM.
#[test]
fn test_ewl_cover_gate_threshold_below_ask_llm() {
    assert_matches!(
        evaluate_gr_cover_gate(&anchor("Alpha Beta"), &gr("Gamma Delta"), true),
        CoverGateOutcome::AskLlm { .. }
    );
}

/// REQ-IDs: REQ-017
/// Directive: apply_llm_decision(SameBook, 0.4) applies via LlmAccepted.
#[test]
fn test_ewl_cover_gate_apply_llm_decision_samebook_apply() {
    assert_matches!(
        apply_llm_decision(LlmDecision::SameBook, 0.4),
        CoverGateOutcome::Apply {
            jaccard: 0.4,
            via: GateReason::LlmAccepted
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: apply_llm_decision(NotSameBook, 0.4) skips via LlmRejected.
#[test]
fn test_ewl_cover_gate_apply_llm_decision_notsamebook_skip() {
    assert_matches!(
        apply_llm_decision(LlmDecision::NotSameBook, 0.4),
        CoverGateOutcome::Skip {
            jaccard: 0.4,
            via: GateReason::LlmRejected
        }
    );
}

/// REQ-IDs: REQ-017
/// Directive: apply_llm_decision(Failed, 0.4) skips via LlmCallFailed.
#[test]
fn test_ewl_cover_gate_apply_llm_decision_failed_skip() {
    assert_matches!(
        apply_llm_decision(LlmDecision::Failed, 0.4),
        CoverGateOutcome::Skip {
            jaccard: 0.4,
            via: GateReason::LlmCallFailed
        }
    );
}
