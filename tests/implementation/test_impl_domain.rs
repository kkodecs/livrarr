#![allow(dead_code)]

use std::path::Path;

use librarr_domain::{classify_file, derive_sort_name, sanitize_path_component, MediaType};

// =============================================================================
// sanitize_path_component — edge cases beyond behavioral tests
// =============================================================================

#[test]
fn sanitize_preserves_internal_dots_and_spaces_while_trimming_only_trailing() {
    // Targets the boundary between allowed internal punctuation and trailing trim behavior.
    let out = sanitize_path_component("chapter. 1 . ", "fallback");
    assert_eq!(out, "chapter. 1");
}

#[test]
fn sanitize_replaces_illegal_chars_before_trailing_trim() {
    // Targets interaction between illegal-char replacement and trailing dot/space trimming.
    let input = "name<>:\"/\\|?* . ";
    let out = sanitize_path_component(input, "fallback");
    // All illegal chars become _, then trailing dots/spaces trimmed
    assert_eq!(out, "name_________");
}

#[test]
fn sanitize_all_dots_input_returns_fallback() {
    // Targets "..." which trims to empty, triggering fallback.
    assert_eq!(sanitize_path_component("...", "fallback"), "fallback");
}

#[test]
fn sanitize_dotdot_a_is_not_fallback() {
    // Targets near-miss traversal-like input that should remain.
    assert_eq!(sanitize_path_component("..a", "fallback"), "..a");
}

#[test]
fn sanitize_dot_a_dot_trims_trailing() {
    // Targets trailing dot trim on mixed input.
    assert_eq!(sanitize_path_component(".a.", "fallback"), ".a");
}

#[test]
fn sanitize_falls_back_when_trim_collapses_to_empty() {
    // Input is spaces + dots — sanitization/trim collapses to nothing.
    assert_eq!(sanitize_path_component(" . . ", "fallback"), "fallback");
}

#[test]
fn sanitize_dot_with_surrounding_whitespace_returns_fallback() {
    assert_eq!(sanitize_path_component(" . ", "fallback"), "fallback");
    assert_eq!(sanitize_path_component(" .. ", "fallback"), "fallback");
}

#[test]
fn sanitize_keeps_leading_spaces_and_internal_whitespace() {
    // Only trailing spaces are trimmed, not leading or internal.
    let out = sanitize_path_component("  leading  and   internal  ", "fallback");
    assert_eq!(out, "  leading  and   internal");
}

#[test]
fn sanitize_removes_all_control_characters_including_tabs_newlines_nulls() {
    let out = sanitize_path_component("ab\tcd\n\r\x0b\x0c\0ef", "fallback");
    assert_eq!(out, "abcdef");
}

#[test]
fn sanitize_preserves_unicode_combining_sequences() {
    // Multi-codepoint grapheme sequences should not be altered.
    let input = "e\u{0301}cole";
    assert_eq!(sanitize_path_component(input, "fallback"), "e\u{0301}cole");
}

#[test]
fn sanitize_replaces_slash_in_rtl_text() {
    let out = sanitize_path_component("كتاب/2024", "fallback");
    assert_eq!(out, "كتاب_2024");
}

#[test]
fn sanitize_fallback_is_returned_verbatim_even_if_it_contains_illegal_chars() {
    // Implementation returns fallback without sanitizing it.
    let out = sanitize_path_component("", "bad/..\\.name");
    assert_eq!(out, "bad/..\\.name");
}

#[test]
fn sanitize_empty_fallback_is_returned_when_input_is_whitespace_only() {
    let out = sanitize_path_component("   ", "");
    assert_eq!(out, "");
}

#[test]
fn sanitize_truncation_after_illegal_char_replacement_in_long_input() {
    // Targets interaction between replacement and truncation.
    let input = format!("{}{}", "a".repeat(250), "/\\:*?\"<>|");
    let out = sanitize_path_component(&input, "fallback");
    assert_eq!(out.len(), 255);
    assert!(out.ends_with("..."));
    assert!(out.starts_with(&"a".repeat(250)));
}

#[test]
fn sanitize_truncation_with_combining_characters_does_not_panic() {
    // Long string of combining sequences near the truncation boundary.
    let input = "e\u{0301}".repeat(130); // ~390 bytes
    let out = sanitize_path_component(&input, "fallback");
    assert!(out.ends_with("..."));
    assert!(out.len() <= 255);
    assert!(out.is_char_boundary(out.len()));
}

#[test]
fn sanitize_long_input_with_trailing_illegal_char() {
    // 254 x's + "/" → replace to "_", result is 255 chars, no truncation needed.
    let input = format!("{}/", "x".repeat(254));
    let out = sanitize_path_component(&input, "fallback");
    assert_eq!(out, format!("{}_", "x".repeat(254)));
    assert_eq!(out.len(), 255);
}

#[test]
fn sanitize_4byte_emoji_truncation() {
    // 4-byte emoji characters — truncation must land on char boundary.
    let input = "😀".repeat(100); // 400 bytes
    let out = sanitize_path_component(&input, "fallback");
    assert!(out.ends_with("..."));
    assert!(out.len() <= 255);
    assert!(out.is_char_boundary(out.len()));
}

// =============================================================================
// derive_sort_name — edge cases beyond behavioral tests
// =============================================================================

#[test]
fn derive_sort_name_collapses_multiple_internal_spaces_in_given_names() {
    // split_whitespace normalizes multiple spaces.
    assert_eq!(derive_sort_name("Mary   Ann   Smith"), "Smith, Mary Ann");
}

#[test]
fn derive_sort_name_handles_tabs_and_newlines_as_whitespace() {
    assert_eq!(derive_sort_name("  Ada\tLovelace\n"), "Lovelace, Ada");
}

#[test]
fn derive_sort_name_single_token_with_unicode() {
    assert_eq!(derive_sort_name("Łukasz"), "Łukasz");
}

#[test]
fn derive_sort_name_rtl_multi_word_names() {
    assert_eq!(derive_sort_name("محمد علي"), "علي, محمد");
}

#[test]
fn derive_sort_name_whitespace_only_returns_empty() {
    assert_eq!(derive_sort_name(" \t\r\n "), "");
}

#[test]
fn derive_sort_name_hyphenated_given_name() {
    assert_eq!(derive_sort_name("Jean-Luc Picard"), "Picard, Jean-Luc");
}

#[test]
fn derive_sort_name_already_has_comma_is_not_special_cased() {
    // Implementation does not detect existing "Last, First" format.
    assert_eq!(derive_sort_name("Doe, John"), "John, Doe,");
}

// =============================================================================
// classify_file — edge cases beyond behavioral tests
// =============================================================================

#[test]
fn classify_file_hidden_file_without_real_extension_returns_none() {
    // ".epub" is a hidden file named "epub" — Path::extension() returns None.
    assert_eq!(classify_file(Path::new(".epub")), None);
    assert_eq!(classify_file(Path::new(".hidden")), None);
}

#[test]
fn classify_file_trailing_dot_returns_none() {
    assert_eq!(classify_file(Path::new("book.")), None);
}

#[test]
fn classify_file_double_dot_returns_none() {
    assert_eq!(classify_file(Path::new("book..")), None);
}

#[test]
fn classify_file_multi_extension_uses_last_only() {
    // Only the last extension matters.
    assert_eq!(classify_file(Path::new("comic.cbz.epubx")), None);
    assert_eq!(classify_file(Path::new("archive.tar.gz")), None);
    // But epub after another ext works:
    assert_eq!(
        classify_file(Path::new("file.txt.epub")),
        Some(MediaType::Ebook)
    );
}

#[test]
fn classify_file_mixed_case_extensions() {
    assert!(matches!(
        classify_file(Path::new("BOOK.EpUb")),
        Some(MediaType::Ebook)
    ));
    assert!(matches!(
        classify_file(Path::new("audio.M4B")),
        Some(MediaType::Audiobook)
    ));
}

#[cfg(unix)]
#[test]
fn classify_file_non_utf8_extension_returns_none() {
    // Targets the to_str()? failure path.
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let bytes = b"bad.\xFF".to_vec();
    let os = OsString::from_vec(bytes);
    let path = Path::new(&os);
    assert_eq!(classify_file(path), None);
}

#[test]
fn classify_file_extension_with_whitespace_returns_none() {
    // Extensions are matched exactly after lowercasing — no trim.
    assert_eq!(classify_file(Path::new("book.epub ")), None);
    assert_eq!(classify_file(Path::new("book. mp3")), None);
}

#[test]
fn classify_file_with_directory_components_still_classifies() {
    assert!(matches!(
        classify_file(Path::new("dir/subdir/book.pdf")),
        Some(MediaType::Ebook)
    ));
    assert!(matches!(
        classify_file(Path::new("/absolute/path/audio.flac")),
        Some(MediaType::Audiobook)
    ));
}
