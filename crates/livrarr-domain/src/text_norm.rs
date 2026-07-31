use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};

use crate::title_cleanup::clean_title;

const TITLE_STOPWORDS: &[&str] = &["a", "an", "the", "of", "and", "in", "on", "for", "to"];
pub(crate) const AUTHOR_SUFFIX_STOPWORDS: &[&str] = &["jr", "sr", "iii", "iv"];

/// Returns true if the character falls in a CJK Unicode block (Unified Ideographs,
/// Hiragana, Katakana, or Hangul Syllables).
fn is_cjk_char(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

/// Returns true if the string contains at least one CJK character.
pub(crate) fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk_char)
}

/// Generates character bigrams from CJK characters in the input. Whitespace and
/// punctuation are stripped before bigram generation. For a single-character input,
/// the character itself is used as the token.
fn cjk_bigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s
        .chars()
        .filter(|c| !c.is_whitespace() && (is_cjk_char(*c) || c.is_alphanumeric()))
        .collect();
    if chars.is_empty() {
        return HashSet::new();
    }
    if chars.len() == 1 {
        let mut set = HashSet::new();
        set.insert(chars[0].to_string());
        return set;
    }
    chars
        .windows(2)
        .map(|w| format!("{}{}", w[0], w[1]))
        .collect()
}

pub fn title_tokens(raw: &str) -> HashSet<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return HashSet::new();
    }

    let cleaned = clean_title(trimmed);

    // Check for CJK before NFKD normalization — NFKD decomposes Hangul syllables
    // into jamo, which breaks bigram generation.
    if has_cjk(&cleaned) {
        let lowered = cleaned.to_lowercase();
        return cjk_bigrams(&lowered);
    }

    let normalized = strip_combining_marks(&cleaned);
    let lowered = normalized.to_lowercase();

    lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty() && s.chars().count() >= 2)
        .filter(|s| !TITLE_STOPWORDS.contains(s))
        .map(String::from)
        .collect()
}

pub fn author_tokens(raw: &str) -> HashSet<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return HashSet::new();
    }

    // Check for CJK before NFKD normalization — NFKD decomposes Hangul syllables
    // into jamo, which breaks bigram generation.
    if has_cjk(trimmed) {
        let lowered = trimmed.to_lowercase();
        return cjk_bigrams(&lowered);
    }

    let normalized = strip_combining_marks(trimmed);
    let lowered = normalized.to_lowercase();

    lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .filter(|s| !AUTHOR_SUFFIX_STOPWORDS.contains(s))
        .map(String::from)
        .collect()
}

pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    intersection as f64 / union as f64
}

pub(crate) fn strip_combining_marks(s: &str) -> String {
    s.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

/// Whether a name carries a letter belonging to a writing system other than
/// Latin.
///
/// The decision is the Unicode **Script** property, never a Unicode block: the
/// blocks disagree with the scripts (`U+AB65 GREEK LETTER SMALL CAPITAL OMEGA`
/// sits inside the Latin Extended-E block yet has Script=Greek), and a block
/// table would call that letter Latin.
///
/// Only script-bearing letters count. Digits, spaces, punctuation and combining
/// marks are not `is_alphabetic` and are ignored; `Common` and `Inherited`
/// alphabetics carry no script evidence and are ignored too. A name with no
/// script-bearing letter at all is therefore Latin — nothing in it says another
/// writing system's record exists.
pub(crate) fn contains_non_latin_letters(name: &str) -> bool {
    name.chars().filter(|c| c.is_alphabetic()).any(|c| {
        !matches!(
            c.script(),
            Script::Latin | Script::Common | Script::Inherited
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CJK detection ---

    #[test]
    fn is_cjk_char_detects_cjk_unified() {
        assert!(is_cjk_char('\u{4E00}')); // first CJK Unified Ideograph
        assert!(is_cjk_char('\u{9FFF}')); // last
    }

    #[test]
    fn is_cjk_char_detects_hiragana() {
        assert!(is_cjk_char('\u{3042}')); // あ
    }

    #[test]
    fn is_cjk_char_detects_katakana() {
        assert!(is_cjk_char('\u{30AD}')); // キ
    }

    #[test]
    fn is_cjk_char_detects_hangul() {
        assert!(is_cjk_char('\u{AC00}')); // 가 (first Hangul syllable)
        assert!(is_cjk_char('\u{CC44}')); // 채
    }

    #[test]
    fn is_cjk_char_rejects_latin() {
        assert!(!is_cjk_char('A'));
        assert!(!is_cjk_char('z'));
        assert!(!is_cjk_char('5'));
    }

    // --- CJK title_tokens ---

    #[test]
    fn title_tokens_japanese_katakana() {
        // キッチン (Kitchen by Banana Yoshimoto)
        let tokens = title_tokens("キッチン");
        assert!(
            !tokens.is_empty(),
            "Japanese katakana should produce tokens"
        );
        // 4 chars -> 3 bigrams: キッ, ッチ, チン
        assert_eq!(tokens.len(), 3);
        assert!(tokens.contains("キッ"));
        assert!(tokens.contains("ッチ"));
        assert!(tokens.contains("チン"));
    }

    #[test]
    fn title_tokens_korean() {
        // 채식주의자 (The Vegetarian by Han Kang)
        let tokens = title_tokens("채식주의자");
        assert!(!tokens.is_empty(), "Korean should produce tokens");
        // 5 chars -> 4 bigrams: 채식, 식주, 주의, 의자
        assert_eq!(tokens.len(), 4);
        assert!(tokens.contains("채식"));
        assert!(tokens.contains("의자"));
    }

    #[test]
    fn title_tokens_chinese() {
        // 三体 (The Three-Body Problem)
        let tokens = title_tokens("三体");
        assert!(!tokens.is_empty(), "Chinese should produce tokens");
        // 2 chars -> 1 bigram: 三体
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains("三体"));
    }

    #[test]
    fn title_tokens_single_cjk_char() {
        let tokens = title_tokens("道");
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains("道"));
    }

    // --- CJK author_tokens ---

    #[test]
    fn author_tokens_japanese() {
        // 村上春樹 (Murakami Haruki)
        let tokens = author_tokens("村上春樹");
        assert!(!tokens.is_empty());
        assert_eq!(tokens.len(), 3);
        assert!(tokens.contains("村上"));
        assert!(tokens.contains("上春"));
        assert!(tokens.contains("春樹"));
    }

    #[test]
    fn author_tokens_korean() {
        // 한강 (Han Kang)
        let tokens = author_tokens("한강");
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains("한강"));
    }

    // --- Jaccard with CJK ---

    #[test]
    fn jaccard_identical_cjk_is_one() {
        let a = title_tokens("キッチン");
        let b = title_tokens("キッチン");
        assert!((jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_different_cjk_less_than_one() {
        let a = title_tokens("キッチン"); // Kitchen
        let b = title_tokens("ノルウェイの森"); // Norwegian Wood
        let score = jaccard(&a, &b);
        assert!(
            score < 1.0,
            "Different CJK titles should score < 1.0, got {score}"
        );
    }

    #[test]
    fn jaccard_korean_identical() {
        let a = title_tokens("채식주의자");
        let b = title_tokens("채식주의자");
        assert!((jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_korean_different() {
        let a = title_tokens("채식주의자"); // The Vegetarian
        let b = title_tokens("소년이온다"); // Human Acts
        assert!(jaccard(&a, &b) < 1.0);
    }

    // --- Latin regression ---

    #[test]
    fn title_tokens_latin_unchanged() {
        let tokens = title_tokens("The Lord of the Rings");
        assert!(tokens.contains("lord"));
        assert!(tokens.contains("rings"));
        // Stopwords filtered
        assert!(!tokens.contains("the"));
        assert!(!tokens.contains("of"));
    }

    #[test]
    fn author_tokens_latin_unchanged() {
        let tokens = author_tokens("J.R.R. Tolkien");
        assert!(tokens.contains("tolkien"));
    }

    #[test]
    fn jaccard_latin_identical_is_one() {
        let a = title_tokens("The Lord of the Rings");
        let b = title_tokens("The Lord of the Rings");
        assert!((jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_latin_different_is_less_than_one() {
        let a = title_tokens("The Lord of the Rings");
        let b = title_tokens("The Hitchhiker's Guide to the Galaxy");
        assert!(jaccard(&a, &b) < 1.0);
    }

    // --- Edge cases ---

    #[test]
    fn title_tokens_empty() {
        assert!(title_tokens("").is_empty());
        assert!(title_tokens("  ").is_empty());
    }

    #[test]
    fn title_tokens_cjk_with_spaces_stripped() {
        // CJK with spaces between characters — spaces should be stripped
        let tokens = title_tokens("三 体");
        assert!(!tokens.is_empty());
        assert!(tokens.contains("三体"));
    }

    #[test]
    fn title_tokens_mixed_cjk_latin() {
        // Mixed input — CJK detected, bigram path used
        let tokens = title_tokens("1Q84 村上春樹");
        assert!(!tokens.is_empty());
        // Should contain CJK bigrams and Latin-CJK boundary bigrams
        assert!(tokens.contains("村上"));
    }

    // --- U9 D9-2 / INV-U9-9: the script carve-out's classifier ---
    //
    // The classifier decides whether an unlabelled credit whose name disagrees
    // is dropped silently or kept as a review card. Every row below is a live
    // shape from the review queue's own script split, plus the one regression
    // that killed the block-table design.

    /// The pin the block allowlist failed. `U+AB65 GREEK LETTER SMALL CAPITAL
    /// OMEGA` sits inside the Latin Extended-E block `U+AB30–U+AB6F` and is
    /// alphabetic, so a block table calls it Latin and silently drops the card.
    /// Its Script property is Greek. Blocks are never the test (INV-U9-9).
    #[test]
    fn a_greek_letter_inside_a_latin_block_is_non_latin() {
        assert!(contains_non_latin_letters("\u{AB65}"));
        assert!(contains_non_latin_letters("Name \u{AB65}"));
    }

    /// The 25 cards the carve-out exists to keep: the author's own name written
    /// in another writing system.
    #[test]
    fn every_non_latin_writing_system_is_non_latin() {
        for name in [
            "Уолтер Айзексон",    // Cyrillic
            "월터 아이작슨",      // Hangul
            "沃尔特·艾萨克森",    // Han
            "ピアース・ブラウン", // Katakana
            "ج.ك. رولينج",        // Arabic
            "Τιτίνα Σπερελάκη",   // Greek
        ] {
            assert!(
                contains_non_latin_letters(name),
                "{name:?} must be kept as a card"
            );
        }
    }

    /// The named accepted loss: extended-Latin romanizations are Latin, so they
    /// are dropped along with the translators they sit beside.
    #[test]
    fn extended_latin_names_are_latin() {
        for name in [
            "Walter Isaacson",
            "Džo Aberkrombijs",
            "Jean-François Ménard",
            "Cristina Macía Orio",
            "Dana Krejčová",
        ] {
            assert!(
                !contains_non_latin_letters(name),
                "{name:?} must be classified Latin"
            );
        }
    }

    /// Normalization form is not script evidence. A decomposed `é` is a Latin
    /// `e` plus a combining mark that is not alphabetic at all, so both spellings
    /// classify the same way — the classifier and the matcher never disagree
    /// about what they were handed.
    #[test]
    fn precomposed_and_decomposed_accents_agree() {
        assert!(!contains_non_latin_letters("Ménard"));
        assert!(!contains_non_latin_letters("Me\u{0301}nard"));
    }

    /// A Common-script alphabetic carries no evidence that another writing
    /// system's record exists, so it is ignored rather than counted.
    #[test]
    fn common_script_letters_are_ignored() {
        assert!(!contains_non_latin_letters("Name \u{A788}"));
        assert!(!contains_non_latin_letters("\u{A788}"));
    }

    /// One non-Latin letter anywhere is script evidence; the rest of the name
    /// being Latin does not dilute it.
    #[test]
    fn a_mixed_script_name_is_non_latin() {
        assert!(contains_non_latin_letters("Джеймс S. A. Корі"));
        assert!(contains_non_latin_letters("James S. A. Корі"));
    }

    /// Digits, spaces and punctuation are not alphabetic, so a name made only of
    /// them carries zero script-bearing characters. Nothing in it says another
    /// writing system's record exists, so it is Latin and drops.
    #[test]
    fn a_name_with_no_script_bearing_letters_is_latin() {
        for name in ["12345", "---", "", "   ", "·・"] {
            assert!(
                !contains_non_latin_letters(name),
                "{name:?} carries no script evidence"
            );
        }
    }
}
