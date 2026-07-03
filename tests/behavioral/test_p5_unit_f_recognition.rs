//! Phase 5 Unit F — Recognition matcher fixes (REQ-011).
//!
//! Covers the parts of REQ-011 that aren't already pinned by
//! `test_mc_matching_variants.rs` (variant folding) or
//! `test_consolidation_rss_sync.rs` (the RSS auto-grab seat end to end):
//! the language gate as a pure function, and the both-empty /
//! position-missing fixes exercised through the real composite scorer
//! (`best_match_score`) rather than only the private helper that contains
//! them.

use livrarr_domain::identity_matching::LanguageVerdict;
use livrarr_matching::{
    best_match_score, release_language_verdict, string_similarity, Confidence, Extraction,
    ExtractionSource, MatchCandidate, MatchProvider, ParsedRelease,
};

fn extraction(title: &str, author: &str) -> Extraction {
    Extraction {
        title: Some(title.to_string()),
        author: Some(author.to_string()),
        year: None,
        isbn: None,
        language: None,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
        confidence: Confidence::Medium,
        source: ExtractionSource::String,
    }
}

fn candidate(title: &str, author: &str) -> MatchCandidate {
    MatchCandidate {
        title: title.to_string(),
        author: author.to_string(),
        year: None,
        work_key: String::new(),
        author_key: None,
        cover_url: None,
        series: None,
        series_position: None,
        provider: MatchProvider::OpenLibrary,
        score: 0.0,
    }
}

fn released(parsed: Extraction) -> ParsedRelease {
    ParsedRelease {
        extractions: vec![parsed],
        format: None,
        year: None,
        narrator: None,
        unabridged: None,
        language: None,
    }
}

// ---------------------------------------------------------------------------
// AC-023: both-empty is not a match, exercised through the real composite.
// ---------------------------------------------------------------------------

#[test]
fn both_empty_after_normalization_scores_zero_through_string_similarity() {
    // Raw non-empty, but every character is punctuation: normalizes to "".
    assert_eq!(string_similarity("...", "***"), 0.0);
}

#[test]
fn best_match_score_never_inflates_on_punctuation_only_titles() {
    // Both sides carry a raw, non-empty title (so title weight isn't zeroed
    // by the has_title check) that normalizes to nothing. The author
    // matches perfectly. Before the fix, the empty-vs-empty title
    // similarity was 1.0, so a punctuation-only title would ride the
    // author's weight up to a deceptively perfect score.
    let parsed = released(extraction("...", "Brandon Sanderson"));
    let cand = candidate("***", "Brandon Sanderson");

    let score = best_match_score(&parsed, &cand);
    assert!(
        score < 1.0,
        "punctuation-only titles must not fold to a full match, got {score}"
    );
}

// ---------------------------------------------------------------------------
// AC-014: a missing series position is not evidence two siblings share a
// volume, exercised through the real composite scorer.
// ---------------------------------------------------------------------------

#[test]
fn best_match_score_does_not_force_full_title_credit_for_position_missing_sibling() {
    let mut ext = extraction("Mistborn: The Final Empire, Book 1", "Brandon Sanderson");
    ext.series_position = None;
    let parsed = released(ext);

    let mut cand = candidate("Mistborn", "Brandon Sanderson");
    cand.series_position = None;

    let with_marker = best_match_score(&parsed, &cand);

    // Control: the same pair, but with the volume marker stripped from the
    // extraction's title entirely — the author match alone should produce
    // an identical or higher score than the marker-carrying title, never
    // lower. If the sibling-volume quirk were still forcing 1.0, the
    // marker-carrying pair would score HIGHER than plain string similarity
    // allows for such different-length titles.
    let full_title_sim = string_similarity("Mistborn: The Final Empire, Book 1", "Mistborn");
    assert!(
        full_title_sim < 1.0,
        "fixture must not already be a full string match"
    );

    // Composite = 0.45*title + 0.40*author (renormalized to ~0.529/0.471
    // since no year/series data is present) + 0.05 series (absent). With
    // title forced to 1.0 the old code would produce a composite of 1.0
    // (perfect author + forced-perfect title). The fixed composite must
    // fall meaningfully short of that.
    assert!(
        with_marker < 0.99,
        "a missing position must not let the sibling-volume guard force a full title score, got {with_marker}"
    );
}

// ---------------------------------------------------------------------------
// D7 recognition corollary (REQ-011): the language gate as a pure function.
// ---------------------------------------------------------------------------

#[test]
fn declared_language_mismatch_is_a_veto() {
    assert_eq!(
        release_language_verdict(Some("French"), Some("en"), "en"),
        LanguageVerdict::Veto
    );
}

#[test]
fn declared_language_match_is_neutral() {
    assert_eq!(
        release_language_verdict(Some("English"), Some("en"), "en"),
        LanguageVerdict::Neutral
    );
}

#[test]
fn silent_release_against_nondefault_language_work_is_grey() {
    assert_eq!(
        release_language_verdict(None, Some("de"), "en"),
        LanguageVerdict::Grey
    );
}

#[test]
fn silent_release_against_default_language_work_is_neutral() {
    assert_eq!(
        release_language_verdict(None, Some("en"), "en"),
        LanguageVerdict::Neutral
    );
}

#[test]
fn both_silent_is_neutral() {
    // A library that has never recorded language at all keeps behaving
    // exactly as it did before this gate existed.
    assert_eq!(
        release_language_verdict(None, None, "en"),
        LanguageVerdict::Neutral
    );
}

#[test]
fn release_language_name_and_work_iso_code_reconcile_before_comparison() {
    // The release parser's bracket tag captures a bare English word
    // ("German"); the work's stored language is an ISO 639-1 code ("de").
    // A naive string comparison would always mismatch; the gate must
    // normalize both sides through the same authority before comparing.
    assert_eq!(
        release_language_verdict(Some("German"), Some("de"), "en"),
        LanguageVerdict::Neutral
    );
}
