#![allow(dead_code, unused_imports)]

//! Behavioral tests for english-work-lifecycle text normalization directives.

use livrarr_domain::text_norm::{author_tokens, jaccard, title_tokens};
use std::collections::HashSet;

fn set(words: &[&str]) -> HashSet<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='The Stand': Then result == {'stand'}
#[test]
fn test_ewl_text_norm_title_tokens_equals_stand() {
    assert_eq!(title_tokens("The Stand"), set(&["stand"]));
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='Cold Days (The Dresden Files, #14)': Then result == {'cold', 'days'}.
#[test]
fn test_ewl_text_norm_title_tokens_equals_cold_days_paren_strip_applied_clean_title_tokenized() {
    assert_eq!(
        title_tokens("Cold Days (The Dresden Files, #14)"),
        set(&["cold", "days"])
    );
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='11/22/63': Then result == {'11', '22', '63'}.
#[test]
fn test_ewl_text_norm_title_tokens_equals_11_22_63_digit_tokens_preserved() {
    assert_eq!(title_tokens("11/22/63"), set(&["11", "22", "63"]));
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='': Then result is empty.
#[test]
fn test_ewl_text_norm_title_tokens_empty() {
    assert!(title_tokens("").is_empty());
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='A': Then result is empty (stopword).
#[test]
fn test_ewl_text_norm_title_tokens_empty_stopword() {
    assert!(title_tokens("A").is_empty());
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='Wind and Truth: Book Five of the Stormlight Archive':
/// clean_title strips the ": Book Five of the Stormlight Archive" series marker,
/// leaving "Wind and Truth". Stopword "and" is dropped. Result: {"wind", "truth"}.
#[test]
fn test_ewl_text_norm_title_tokens_includes_wind_truth_book_five_stormlight_archive_excludes() {
    let tokens = title_tokens("Wind and Truth: Book Five of the Stormlight Archive");
    assert_eq!(tokens, set(&["wind", "truth"]));
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given raw='Cafe with acute accent': Then result == {'cafe'} after NFKD accent stripping.
#[test]
fn test_ewl_text_norm_title_tokens_equals_cafe_nfkd_strips_combining_acute() {
    assert_eq!(title_tokens("Café"), set(&["cafe"]));
}

/// REQ-IDs: REQ-003
/// Directive: Given raw='C. S. Lewis': Then result == {'c', 's', 'lewis'}.
#[test]
fn test_ewl_text_norm_author_tokens_equals_c_s_lewis() {
    assert_eq!(author_tokens("C. S. Lewis"), set(&["c", "s", "lewis"]));
}

/// REQ-IDs: REQ-003
/// Directive: Given raw='Stephen King': Then result == {'stephen', 'king'}.
#[test]
fn test_ewl_text_norm_author_tokens_equals_stephen_king() {
    assert_eq!(author_tokens("Stephen King"), set(&["stephen", "king"]));
}

/// REQ-IDs: REQ-003
/// Directive: Given raw='Brandon Sanderson, Jr.': Then result == {'brandon', 'sanderson'}.
#[test]
fn test_ewl_text_norm_author_tokens_equals_brandon_sanderson_jr_suffix_dropped() {
    assert_eq!(
        author_tokens("Brandon Sanderson, Jr."),
        set(&["brandon", "sanderson"])
    );
}

/// REQ-IDs: REQ-003
/// Directive: Given raw='J.R.R. Tolkien': Then result == {'j', 'r', 'tolkien'}.
#[test]
fn test_ewl_text_norm_author_tokens_equals_j_r_tolkien_initials_preserved() {
    assert_eq!(author_tokens("J.R.R. Tolkien"), set(&["j", "r", "tolkien"]));
}

/// REQ-IDs: REQ-003
/// Directive: Given raw='': Then result is empty.
#[test]
fn test_ewl_text_norm_author_tokens_empty() {
    assert!(author_tokens("").is_empty());
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given a={'cold','days'}, b={'cold','days'}: Then jaccard == 1.0.
#[test]
fn test_ewl_text_norm_jaccard_jaccard_equals_1_0() {
    let a = set(&["cold", "days"]);
    assert_eq!(jaccard(&a, &a), 1.0);
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given a={'cold','days'}, b={'cold','days','dresden','files'}: Then jaccard == 0.5.
#[test]
fn test_ewl_text_norm_jaccard_jaccard_equals_2_4_0_5() {
    assert_eq!(
        jaccard(
            &set(&["cold", "days"]),
            &set(&["cold", "days", "dresden", "files"])
        ),
        0.5
    );
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given a={}, b={}: Then jaccard == 0.0.
#[test]
fn test_ewl_text_norm_jaccard_jaccard_equals_0_0() {
    assert_eq!(jaccard(&HashSet::new(), &HashSet::new()), 0.0);
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given a={'foo'}, b={}: Then jaccard == 0.0.
#[test]
fn test_ewl_text_norm_jaccard_jaccard_equals_0_0_2() {
    assert_eq!(jaccard(&set(&["foo"]), &HashSet::new()), 0.0);
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Given a={'a','b'}, b={'c','d'}: Then jaccard == 0.0.
#[test]
fn test_ewl_text_norm_jaccard_jaccard_equals_0_0_3() {
    assert_eq!(jaccard(&set(&["a", "b"]), &set(&["c", "d"])), 0.0);
}

/// REQ-IDs: REQ-003, REQ-017
/// Directive: Symmetry check: jaccard(a,b) == jaccard(b,a) for any non-empty pair.
#[test]
fn test_ewl_text_norm_jaccard_symmetry_check_jaccard_b_equals_jaccard_b_empty_pair() {
    let a = set(&["cold", "days"]);
    let b = set(&["cold", "winter"]);
    assert_eq!(jaccard(&a, &b), jaccard(&b, &a));
}
