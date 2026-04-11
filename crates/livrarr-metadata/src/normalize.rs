use unicode_normalization::UnicodeNormalization;

/// NFC-normalize a string. Applied at every provider boundary
/// to prevent byte-level mismatches when the same title is encoded
/// differently by different systems (critical for CJK).
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}
