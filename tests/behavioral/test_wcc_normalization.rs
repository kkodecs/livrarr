#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency domain normalization.

use livrarr_domain::normalization::{
    normalize_asin, normalize_gr_key, normalize_isbn13, normalize_language, AsinNorm,
};

/// REQ-IDs: REQ-029, AC-037
/// Directive: invalid ISBN checksums and wrong lengths are treated as absent.
#[test]
fn test_wcc_normalization_req_029_ac_037_invalid_isbn_checksum_and_11_char_return_none() {
    assert_eq!(normalize_isbn13("9780439139602"), None);
    assert_eq!(normalize_isbn13("04391396001"), None);
}

/// REQ-IDs: REQ-004
/// Directive: checksum-valid ISBN-10 input is converted to canonical ISBN-13.
#[test]

fn test_wcc_normalization_req_004_valid_isbn10_converts_to_isbn13() {
    assert_eq!(
        normalize_isbn13("0439139600"),
        Some("9780439139601".to_string())
    );
}

/// REQ-IDs: REQ-004
/// Directive: an ISBN-10-shaped ASIN that passes checksum folds into isbn_13.
#[test]

fn test_wcc_normalization_req_004_asin_isbn10_shape_valid_checksum_returns_isbn13() {
    assert_eq!(
        normalize_asin("0439139600"),
        AsinNorm::Isbn13("9780439139601".to_string())
    );
}

/// REQ-IDs: REQ-004, REQ-029, AC-029
/// Directive: an ISBN-10-shaped ASIN that fails checksum is retained as an ASIN.
#[test]

fn test_wcc_normalization_req_004_req_029_ac_029_asin_isbn10_shape_bad_checksum_retained() {
    assert_eq!(
        normalize_asin("0439139601"),
        AsinNorm::Asin("0439139601".to_string())
    );
}

/// REQ-IDs: REQ-002, REQ-029, AC-003
/// Directive: Goodreads keys persist as bare leading numeric segments only.
#[test]

fn test_wcc_normalization_req_002_req_029_ac_003_gr_key_slug_forms_strip_to_digits() {
    assert_eq!(
        normalize_gr_key("12345.Some_Slug"),
        Some("12345".to_string())
    );
    assert_eq!(
        normalize_gr_key("12345-Some_Slug"),
        Some("12345".to_string())
    );
    assert_eq!(normalize_gr_key("not-a-goodreads-key"), None);
}

/// REQ-IDs: REQ-005, AC-005
/// Directive: language region subtags are stripped to bare ISO-639-1 codes.
#[test]

fn test_wcc_normalization_req_005_ac_005_language_region_subtags_strip_to_primary_code() {
    assert_eq!(normalize_language("en-US"), Some("en".to_string()));
    assert_eq!(normalize_language("pt-BR"), Some("pt".to_string()));
}
