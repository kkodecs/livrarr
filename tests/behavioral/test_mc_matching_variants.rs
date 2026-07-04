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
fn one_sided_series_tail_does_not_fold_to_base() {
    // A title carrying a subtitle and volume marker never folds to the bare
    // base title: the colon-cut fold that once bridged this pair also
    // bridged every sibling volume ("Mistborn: The Well of Ascension,
    // Book 2") to the same "Mistborn" key. One-sided tail evidence keeps
    // distinct keys; near-matches are the composite scorer's job.
    assert_distinct_variant("Mistborn: The Final Empire, Book 1", "Mistborn");
}

#[test]
fn differing_subtitles_do_not_fold() {
    // Substantively different subtitles on both sides keep distinct keys —
    // the old fold bridged translated editions only via the same colon cut
    // that also bridged different volumes. Cross-language edition matching
    // is governed by the language rules and composite scoring, not the
    // variant fold.
    assert_distinct_variant(
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
