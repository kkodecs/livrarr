use std::path::Path;

use librarr_domain::{classify_file, derive_sort_name, sanitize_path_component, MediaType};

// =============================================================================
// sanitize_path_component — IMPORT-011
// =============================================================================

#[test]
fn test_domain_sanitize_replaces_illegal_filesystem_characters() {
    // Satisfies: IMPORT-011 — illegal chars (\ / : * ? " < > |) replaced with underscore
    let input = r#"a\b/c:d*e?f"g<h>i|j"#;
    let result = sanitize_path_component(input, "fallback");
    assert_eq!(result, "a_b_c_d_e_f_g_h_i_j");
}

#[test]
fn test_domain_sanitize_strips_control_characters() {
    // Satisfies: IMPORT-011 — control characters stripped
    let input = "ab\u{0000}cd\u{001F}ef\n";
    let result = sanitize_path_component(input, "fallback");
    assert_eq!(result, "abcdef");
}

#[test]
fn test_domain_sanitize_standalone_dot_returns_fallback() {
    // Satisfies: IMPORT-011 — "." as standalone name becomes fallback
    let result = sanitize_path_component(".", "fallback");
    assert_eq!(result, "fallback");
}

#[test]
fn test_domain_sanitize_standalone_dotdot_returns_fallback() {
    // Satisfies: IMPORT-011 — ".." as standalone name becomes fallback
    let result = sanitize_path_component("..", "fallback");
    assert_eq!(result, "fallback");
}

#[test]
fn test_domain_sanitize_trims_trailing_dots() {
    // Satisfies: IMPORT-011 — trailing dots trimmed
    let result = sanitize_path_component("filename...", "fallback");
    assert_eq!(result, "filename");
}

#[test]
fn test_domain_sanitize_trims_trailing_spaces() {
    // Satisfies: IMPORT-011 — trailing spaces trimmed
    let result = sanitize_path_component("filename   ", "fallback");
    assert_eq!(result, "filename");
}

#[test]
fn test_domain_sanitize_trims_trailing_spaces_and_dots_together() {
    // Satisfies: IMPORT-011 — trailing dots/spaces trimmed
    let result = sanitize_path_component("name . .  ", "fallback");
    assert_eq!(result, "name");
}

#[test]
fn test_domain_sanitize_empty_after_sanitization_returns_fallback() {
    // Satisfies: IMPORT-011 — empty result after sanitization becomes fallback
    let result = sanitize_path_component("\u{0000}\u{0008}   ...", "fallback");
    assert_eq!(result, "fallback");
}

#[test]
fn test_domain_sanitize_empty_string_returns_fallback() {
    // Satisfies: IMPORT-011 — empty input returns fallback
    let result = sanitize_path_component("", "fallback");
    assert_eq!(result, "fallback");
}

#[test]
fn test_domain_sanitize_truncation_appends_ellipsis() {
    // Satisfies: IMPORT-011 — overlong result truncated with ellipsis, total <= 255 bytes
    let input = "a".repeat(300);
    let result = sanitize_path_component(&input, "fallback");
    assert_eq!(result, format!("{}...", "a".repeat(252)));
}

#[test]
fn test_domain_sanitize_truncates_at_utf8_char_boundary() {
    // Satisfies: IMPORT-011 — truncation at UTF-8 char boundary
    // "é" is 2 bytes, 200 * 2 = 400 bytes. After truncation: 252 content bytes = 126 chars + "..."
    let input = "é".repeat(200);
    let result = sanitize_path_component(&input, "fallback");
    assert_eq!(result, format!("{}...", "é".repeat(126)));
}

#[test]
fn test_domain_sanitize_exactly_255_bytes_is_not_truncated() {
    // Satisfies: IMPORT-011 — boundary: exactly 255 bytes is NOT truncated
    let input = "a".repeat(255);
    let result = sanitize_path_component(&input, "fallback");
    assert_eq!(result, input);
}

#[test]
fn test_domain_sanitize_exactly_256_bytes_is_truncated_with_ellipsis() {
    // Satisfies: IMPORT-011 — boundary: 256 bytes IS truncated to 255 with ellipsis
    let input = "a".repeat(256);
    let result = sanitize_path_component(&input, "fallback");
    assert_eq!(result, format!("{}...", "a".repeat(252)));
    assert_eq!(result.len(), 255);
}

// =============================================================================
// derive_sort_name — SEARCH-005
// =============================================================================

#[test]
fn test_domain_derive_sort_name_two_word_name() {
    // Satisfies: SEARCH-005 — "Frank Herbert" → "Herbert, Frank"
    let result = derive_sort_name("Frank Herbert");
    assert_eq!(result, "Herbert, Frank");
}

#[test]
fn test_domain_derive_sort_name_initials_and_surname() {
    // Satisfies: SEARCH-005 — "J.R.R. Tolkien" → "Tolkien, J.R.R."
    let result = derive_sort_name("J.R.R. Tolkien");
    assert_eq!(result, "Tolkien, J.R.R.");
}

#[test]
fn test_domain_derive_sort_name_single_word_returns_as_is() {
    // Satisfies: SEARCH-005 — single-word name returned unchanged
    let result = derive_sort_name("Plato");
    assert_eq!(result, "Plato");
}

#[test]
fn test_domain_derive_sort_name_three_plus_words_uses_last_word_as_surname() {
    // Satisfies: SEARCH-005 — multi-word: last word is surname, rest are given names
    let result = derive_sort_name("George R. R. Martin");
    assert_eq!(result, "Martin, George R. R.");
}

#[test]
fn test_domain_derive_sort_name_empty_string_returns_as_is() {
    // Satisfies: SEARCH-005 — empty string edge case
    let result = derive_sort_name("");
    assert_eq!(result, "");
}

#[test]
fn test_domain_derive_sort_name_trims_leading_and_trailing_whitespace() {
    // Satisfies: SEARCH-005 — whitespace handling
    let result = derive_sort_name("  Isaac Asimov  ");
    assert_eq!(result, "Asimov, Isaac");
}

// =============================================================================
// classify_file — IMPORT-007
// =============================================================================

#[test]
fn test_domain_classify_file_epub_as_ebook() {
    // Satisfies: IMPORT-007 — .epub classified as Ebook
    assert_eq!(
        classify_file(Path::new("book.epub")),
        Some(MediaType::Ebook)
    );
}

#[test]
fn test_domain_classify_file_mobi_as_ebook() {
    // Satisfies: IMPORT-007 — .mobi classified as Ebook
    assert_eq!(
        classify_file(Path::new("book.mobi")),
        Some(MediaType::Ebook)
    );
}

#[test]
fn test_domain_classify_file_azw3_as_ebook() {
    // Satisfies: IMPORT-007 — .azw3 classified as Ebook
    assert_eq!(
        classify_file(Path::new("book.azw3")),
        Some(MediaType::Ebook)
    );
}

#[test]
fn test_domain_classify_file_pdf_as_ebook() {
    // Satisfies: IMPORT-007 — .pdf classified as Ebook
    assert_eq!(classify_file(Path::new("book.pdf")), Some(MediaType::Ebook));
}

#[test]
fn test_domain_classify_file_mp3_as_audiobook() {
    // Satisfies: IMPORT-007 — .mp3 classified as Audiobook
    assert_eq!(
        classify_file(Path::new("audio.mp3")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_m4a_as_audiobook() {
    // Satisfies: IMPORT-007 — .m4a classified as Audiobook
    assert_eq!(
        classify_file(Path::new("audio.m4a")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_m4b_as_audiobook() {
    // Satisfies: IMPORT-007 — .m4b classified as Audiobook
    assert_eq!(
        classify_file(Path::new("audio.m4b")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_flac_as_audiobook() {
    // Satisfies: IMPORT-007 — .flac classified as Audiobook
    assert_eq!(
        classify_file(Path::new("audio.flac")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_ogg_as_audiobook() {
    // Satisfies: IMPORT-007 — .ogg classified as Audiobook
    assert_eq!(
        classify_file(Path::new("audio.ogg")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_wma_as_audiobook() {
    // Satisfies: IMPORT-007 — .wma classified as Audiobook
    assert_eq!(
        classify_file(Path::new("audio.wma")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_extension_matching_is_case_insensitive() {
    // Satisfies: IMPORT-007 — case-insensitive extension matching
    assert_eq!(
        classify_file(Path::new("book.EPUB")),
        Some(MediaType::Ebook)
    );
    assert_eq!(
        classify_file(Path::new("audio.MP3")),
        Some(MediaType::Audiobook)
    );
}

#[test]
fn test_domain_classify_file_cbz_is_not_classified() {
    // Satisfies: IMPORT-007 — .cbz excluded per spec Section 5 (no comics)
    assert_eq!(classify_file(Path::new("comic.cbz")), None);
}

#[test]
fn test_domain_classify_file_cbr_is_not_classified() {
    // Satisfies: IMPORT-007 — .cbr excluded per spec Section 5 (no comics)
    assert_eq!(classify_file(Path::new("comic.cbr")), None);
}

#[test]
fn test_domain_classify_file_unrecognized_extension_returns_none() {
    // Satisfies: IMPORT-007 — unrecognized extensions return None
    assert_eq!(classify_file(Path::new("book.txt")), None);
}

#[test]
fn test_domain_classify_file_no_extension_returns_none() {
    // Satisfies: IMPORT-007 — no extension returns None
    assert_eq!(classify_file(Path::new("book")), None);
}
