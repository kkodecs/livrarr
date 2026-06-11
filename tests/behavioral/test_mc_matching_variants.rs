//! Behavioral RED tests for metadata-correctness variant title normalization.

use livrarr_matching::normalize_title_variants;

fn assert_same_variant(left: &str, right: &str) {
    assert_eq!(
        normalize_title_variants(left),
        normalize_title_variants(right),
        "variant titles should normalize to the same Tier-A comparison key"
    );
}

fn assert_distinct_variant(left: &str, right: &str) {
    assert_ne!(
        normalize_title_variants(left),
        normalize_title_variants(right),
        "genuinely different books must not collapse to the same comparison key"
    );
}

#[test]
fn trailing_unabridged_marker_folds_away() {
    // REQ-020/AC-022: form-class representative pair (unit slice); full AC-022
    // validation runs manual import over the #132 staging dataset at the Test
    // stage (Q-001).
    assert_same_variant("Dune (Unabridged)", "Dune");
}

#[test]
fn subtitle_book_number_tail_folds_to_base_title() {
    // REQ-020/AC-022: form-class representative pair (unit slice); full AC-022
    // validation runs manual import over the #132 staging dataset at the Test
    // stage (Q-001).
    assert_same_variant("Mistborn: The Final Empire, Book 1", "Mistborn");
}

#[test]
fn translated_subtitle_variant_compares_on_base_segment() {
    // REQ-020/AC-022: form-class representative pair (unit slice); full AC-022
    // validation runs manual import over the #132 staging dataset at the Test
    // stage (Q-001).
    assert_same_variant(
        "The Witcher: Ostatnie zyczenie",
        "The Witcher: The Last Wish",
    );
}

#[test]
fn diacritic_only_difference_folds() {
    // REQ-020/AC-022: form-class representative pair (unit slice); full AC-022
    // validation runs manual import over the #132 staging dataset at the Test
    // stage (Q-001).
    assert_same_variant("Wiedźmin", "Wiedzmin");
}

#[test]
fn different_books_do_not_collapse() {
    // REQ-020/AC-022: variant acceptance must not create a false positive.
    assert_distinct_variant("Dune", "Dune Messiah");
}
