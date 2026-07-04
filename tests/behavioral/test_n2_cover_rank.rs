//! AC-1: one rank table, consumed by all three cover-ranking sites (S1).
//!
//! Two of the three sites are public and directly exercised here: the live
//! picker's `PriorityModel` and the comparator's same-tier tiebreak
//! (`should_upgrade_same_tier`). The third — the import-time seed-cover host
//! preference (`work_service::cover_source_rank`/`best_same_work_cover`) — is
//! a private fn inside `livrarr-metadata` and is proven by that crate's own
//! unit tests (`cover_rank_prefers_high_res_sources_over_openlibrary`,
//! `cover_rank_foreign_prefers_googlebooks_over_goodreads_hosted_amazon`,
//! `cover_rank_unknown_host_ranks_above_empty_below_known_providers` in
//! `crates/livrarr-metadata/src/work_service.rs`), which assert against the
//! exact same `cover_rank::rank_table`/`provider_for_cover_host` this file
//! exercises — a table reorder moves all three together, by construction.

use livrarr_domain::MetadataProvider as P;
use livrarr_enrichment::cover_rank::{provider_for_cover_host, rank_table, CoverRankModel};
use livrarr_enrichment::cover_resolution::should_upgrade_same_tier;
use livrarr_enrichment::PriorityModel;

#[test]
fn ac1_priority_model_cover_and_audio_lists_are_exactly_the_rank_table() {
    assert_eq!(
        PriorityModel::english().cover,
        rank_table(CoverRankModel::EbookEnglish).to_vec()
    );
    assert_eq!(
        PriorityModel::foreign().cover,
        rank_table(CoverRankModel::EbookForeign).to_vec()
    );
    assert_eq!(
        PriorityModel::english().audio,
        rank_table(CoverRankModel::Audiobook).to_vec()
    );
    assert_eq!(
        PriorityModel::foreign().audio,
        rank_table(CoverRankModel::Audiobook).to_vec()
    );
}

#[test]
fn ac1_ebook_english_order_is_gr_hc_gb_readarr_ol_audnexus_audible() {
    assert_eq!(
        rank_table(CoverRankModel::EbookEnglish),
        &[
            P::Goodreads,
            P::Hardcover,
            P::GoogleBooks,
            P::Readarr,
            P::OpenLibrary,
            P::Audnexus,
            P::Audible,
        ]
    );
}

#[test]
fn ac1_ebook_foreign_order_is_gb_first_gr_hc_readarr_ol_audnexus_audible() {
    assert_eq!(
        rank_table(CoverRankModel::EbookForeign),
        &[
            P::GoogleBooks,
            P::Goodreads,
            P::Hardcover,
            P::Readarr,
            P::OpenLibrary,
            P::Audnexus,
            P::Audible,
        ]
    );
}

#[test]
fn ac1_audiobook_order_is_audible_first_for_both_language_models() {
    let expected = [
        P::Audible,
        P::Audnexus,
        P::Hardcover,
        P::Goodreads,
        P::OpenLibrary,
        P::GoogleBooks,
    ];
    assert_eq!(rank_table(CoverRankModel::Audiobook), &expected);
}

#[test]
fn ac1_comparator_tiebreak_respects_english_order_gr_beats_hc() {
    assert!(should_upgrade_same_tier(
        400,
        600,
        500,
        700,
        Some("hardcover"),
        "goodreads",
        CoverRankModel::EbookEnglish,
    ));
    assert!(!should_upgrade_same_tier(
        400,
        600,
        500,
        700,
        Some("goodreads"),
        "hardcover",
        CoverRankModel::EbookEnglish,
    ));
}

#[test]
fn ac1_comparator_tiebreak_respects_foreign_order_gb_beats_gr() {
    assert!(should_upgrade_same_tier(
        400,
        600,
        500,
        700,
        Some("goodreads"),
        "googlebooks",
        CoverRankModel::EbookForeign,
    ));
    assert!(!should_upgrade_same_tier(
        400,
        600,
        500,
        700,
        Some("googlebooks"),
        "goodreads",
        CoverRankModel::EbookForeign,
    ));
}

#[test]
fn ac1_comparator_tiebreak_respects_audiobook_order_audible_beats_hardcover() {
    assert!(should_upgrade_same_tier(
        500,
        700,
        500,
        700,
        Some("hardcover"),
        "audible",
        CoverRankModel::Audiobook,
    ));
    assert!(!should_upgrade_same_tier(
        500,
        700,
        500,
        700,
        Some("audible"),
        "hardcover",
        CoverRankModel::Audiobook,
    ));
}

#[test]
fn ac1_provider_for_cover_host_backs_the_import_time_site() {
    // The exact classifier `work_service::cover_source_rank` calls (S3's
    // backfill migration also calls it — one classifier, three consumers).
    assert_eq!(
        provider_for_cover_host("https://i.gr-assets.com/x.jpg"),
        Some(P::Goodreads)
    );
    assert_eq!(
        provider_for_cover_host("https://assets.hardcover.app/x.jpg"),
        Some(P::Hardcover)
    );
    assert_eq!(
        provider_for_cover_host("https://books.google.com/x"),
        Some(P::GoogleBooks)
    );
    assert_eq!(
        provider_for_cover_host("https://covers.openlibrary.org/x.jpg"),
        Some(P::OpenLibrary)
    );
    assert_eq!(
        provider_for_cover_host("https://unknown.example.com/x.jpg"),
        None
    );
}
