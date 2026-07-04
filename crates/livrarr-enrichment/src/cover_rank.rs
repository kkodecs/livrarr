//! The single authoritative cover-provider rank table (S1).
//!
//! Three call sites consume this table: the live picker's `PriorityModel`
//! (`cover`/`audio` lists), the comparator's same-tier tiebreak in
//! `cover_resolution`, and the import-time seed-cover host preference in
//! `livrarr-metadata::work_service`. A rank-order change here changes all
//! three at once — there is exactly one order per media type + language
//! model, never three.

use livrarr_domain::{CoverMediaType, MetadataProvider};

/// Which rank list applies. Ebook splits by language model; audiobook uses
/// one list for both models (PO decision — audio-native sources always lead
/// the audiobook slot regardless of the work's text language).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverRankModel {
    EbookEnglish,
    EbookForeign,
    Audiobook,
}

impl CoverRankModel {
    /// Select the ebook model from a language-foreign flag.
    pub fn for_ebook(foreign: bool) -> Self {
        if foreign {
            Self::EbookForeign
        } else {
            Self::EbookEnglish
        }
    }

    /// Select the model for a media slot; `foreign` is ignored for
    /// `CoverMediaType::Audiobook` (one audiobook order for both models).
    pub fn for_media(media_type: CoverMediaType, foreign: bool) -> Self {
        match media_type {
            CoverMediaType::Ebook => Self::for_ebook(foreign),
            CoverMediaType::Audiobook => Self::Audiobook,
        }
    }
}

use MetadataProvider as P;

/// Ebook, English: GR -> HC -> GB -> Readarr -> OL -> Audnexus -> Audible.
const EBOOK_ENGLISH: &[MetadataProvider] = &[
    P::Goodreads,
    P::Hardcover,
    P::GoogleBooks,
    P::Readarr,
    P::OpenLibrary,
    P::Audnexus,
    P::Audible,
];

/// Ebook, foreign: GB -> GR -> HC -> Readarr -> OL -> Audnexus -> Audible.
const EBOOK_FOREIGN: &[MetadataProvider] = &[
    P::GoogleBooks,
    P::Goodreads,
    P::Hardcover,
    P::Readarr,
    P::OpenLibrary,
    P::Audnexus,
    P::Audible,
];

/// Audiobook, both language models: Audible -> Audnexus -> HC -> GR -> OL -> GB.
const AUDIOBOOK: &[MetadataProvider] = &[
    P::Audible,
    P::Audnexus,
    P::Hardcover,
    P::Goodreads,
    P::OpenLibrary,
    P::GoogleBooks,
];

/// The canonical provider order for `model`. Lower index = higher priority.
pub fn rank_table(model: CoverRankModel) -> &'static [MetadataProvider] {
    match model {
        CoverRankModel::EbookEnglish => EBOOK_ENGLISH,
        CoverRankModel::EbookForeign => EBOOK_FOREIGN,
        CoverRankModel::Audiobook => AUDIOBOOK,
    }
}

/// `provider`'s position in `model`'s rank list (lower = better). A provider
/// absent from the list (shouldn't happen for the seven known providers, but
/// defensive) ranks last.
pub fn rank_index(provider: MetadataProvider, model: CoverRankModel) -> usize {
    rank_table(model)
        .iter()
        .position(|&p| p == provider)
        .unwrap_or(rank_table(model).len())
}

/// True for the shared amazon CDN family (`images-amazon`, `media-amazon`,
/// `ssl-images-amazon`) — hosts that carry MULTIPLE providers' art: Goodreads
/// ebook covers and Audible/Audnexus audiobook covers alike. Which provider a
/// URL on these hosts belongs to therefore depends on the SLOT it sits in;
/// `i.gr-assets.com` is excluded (a genuinely Goodreads-owned host).
fn is_shared_amazon_family(url_lower: &str) -> bool {
    url_lower.contains("images-amazon")
        || url_lower.contains("media-amazon")
        || url_lower.contains("ssl-images-amazon")
}

/// Classify a cover asset URL's host to the provider that owns it.
/// Ebook-slot semantics: the shared amazon family maps to Goodreads, the
/// dominant real-world case for the ebook slot. For the audiobook slot use
/// [`provider_for_cover_host_for_slot`], which maps that family to Audible
/// instead. Hosts with no known provider return `None` (never guessed).
pub fn provider_for_cover_host(url: &str) -> Option<MetadataProvider> {
    let u = url.to_ascii_lowercase();
    if u.contains("i.gr-assets.com") || is_shared_amazon_family(&u) {
        Some(MetadataProvider::Goodreads)
    } else if u.contains("hardcover.app") {
        Some(MetadataProvider::Hardcover)
    } else if u.contains("books.google") || u.contains("googleusercontent") {
        Some(MetadataProvider::GoogleBooks)
    } else if u.contains("covers.openlibrary.org") {
        Some(MetadataProvider::OpenLibrary)
    } else {
        None
    }
}

/// Slot-aware host classification: identical to [`provider_for_cover_host`]
/// for the ebook slot, but for the audiobook slot the shared amazon family
/// maps to Audible — amazon-hosted art in that slot is Audible/Audnexus
/// catalog imagery, and stamping it "goodreads" would mis-rank later
/// same-tier comparator decisions (Audible leads the audiobook order;
/// Goodreads sits fourth).
pub fn provider_for_cover_host_for_slot(
    url: &str,
    media_type: CoverMediaType,
) -> Option<MetadataProvider> {
    match media_type {
        CoverMediaType::Ebook => provider_for_cover_host(url),
        CoverMediaType::Audiobook => {
            let u = url.to_ascii_lowercase();
            if is_shared_amazon_family(&u) {
                Some(MetadataProvider::Audible)
            } else {
                provider_for_cover_host(url)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ebook_english_order_is_gr_hc_gb_readarr_ol_audnexus_audible() {
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
    fn ebook_foreign_order_is_gb_first_then_gr_hc_readarr_ol_audnexus_audible() {
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
    fn audiobook_order_is_audible_audnexus_hc_gr_ol_gb() {
        assert_eq!(
            rank_table(CoverRankModel::Audiobook),
            &[
                P::Audible,
                P::Audnexus,
                P::Hardcover,
                P::Goodreads,
                P::OpenLibrary,
                P::GoogleBooks,
            ]
        );
    }

    #[test]
    fn for_media_ignores_foreign_flag_for_audiobook() {
        assert_eq!(
            CoverRankModel::for_media(CoverMediaType::Audiobook, false),
            CoverRankModel::Audiobook
        );
        assert_eq!(
            CoverRankModel::for_media(CoverMediaType::Audiobook, true),
            CoverRankModel::Audiobook
        );
    }

    #[test]
    fn for_media_splits_ebook_by_foreign_flag() {
        assert_eq!(
            CoverRankModel::for_media(CoverMediaType::Ebook, false),
            CoverRankModel::EbookEnglish
        );
        assert_eq!(
            CoverRankModel::for_media(CoverMediaType::Ebook, true),
            CoverRankModel::EbookForeign
        );
    }

    #[test]
    fn rank_index_reflects_table_position() {
        assert_eq!(rank_index(P::Goodreads, CoverRankModel::EbookEnglish), 0);
        assert_eq!(rank_index(P::Hardcover, CoverRankModel::EbookEnglish), 1);
        assert_eq!(rank_index(P::Audible, CoverRankModel::EbookEnglish), 6);
        assert_eq!(rank_index(P::GoogleBooks, CoverRankModel::EbookForeign), 0);
        assert_eq!(rank_index(P::Audible, CoverRankModel::Audiobook), 0);
        assert_eq!(rank_index(P::GoogleBooks, CoverRankModel::Audiobook), 5);
    }

    #[test]
    fn provider_for_cover_host_maps_known_hosts() {
        assert_eq!(
            provider_for_cover_host("https://i.gr-assets.com/books/x.jpg"),
            Some(MetadataProvider::Goodreads)
        );
        assert_eq!(
            provider_for_cover_host("https://m.media-amazon.com/images/I/x.jpg"),
            Some(MetadataProvider::Goodreads)
        );
        assert_eq!(
            provider_for_cover_host("https://images-na.ssl-images-amazon.com/images/P/x.jpg"),
            Some(MetadataProvider::Goodreads)
        );
        assert_eq!(
            provider_for_cover_host("https://assets.hardcover.app/x.jpg"),
            Some(MetadataProvider::Hardcover)
        );
        assert_eq!(
            provider_for_cover_host("https://books.google.com/books/content?id=x"),
            Some(MetadataProvider::GoogleBooks)
        );
        assert_eq!(
            provider_for_cover_host("https://books.googleusercontent.com/x.jpg"),
            Some(MetadataProvider::GoogleBooks)
        );
        assert_eq!(
            provider_for_cover_host("https://covers.openlibrary.org/b/id/1-L.jpg"),
            Some(MetadataProvider::OpenLibrary)
        );
    }

    #[test]
    fn provider_for_cover_host_unknown_host_returns_none() {
        assert_eq!(
            provider_for_cover_host("https://random-cdn.example.com/x.jpg"),
            None
        );
        assert_eq!(provider_for_cover_host(""), None);
    }

    #[test]
    fn slot_aware_classifier_maps_amazon_family_per_slot() {
        let amazon = "https://m.media-amazon.com/images/I/x.jpg";
        assert_eq!(
            provider_for_cover_host_for_slot(amazon, CoverMediaType::Ebook),
            Some(MetadataProvider::Goodreads)
        );
        assert_eq!(
            provider_for_cover_host_for_slot(amazon, CoverMediaType::Audiobook),
            Some(MetadataProvider::Audible)
        );
    }

    #[test]
    fn slot_aware_classifier_gr_assets_stays_goodreads_in_both_slots() {
        // i.gr-assets.com is Goodreads-owned, not the shared amazon family.
        let gr = "https://i.gr-assets.com/books/x.jpg";
        assert_eq!(
            provider_for_cover_host_for_slot(gr, CoverMediaType::Ebook),
            Some(MetadataProvider::Goodreads)
        );
        assert_eq!(
            provider_for_cover_host_for_slot(gr, CoverMediaType::Audiobook),
            Some(MetadataProvider::Goodreads)
        );
    }

    #[test]
    fn slot_aware_classifier_non_amazon_hosts_are_slot_independent() {
        for url in [
            "https://assets.hardcover.app/x.jpg",
            "https://books.google.com/x",
            "https://covers.openlibrary.org/b/id/1-L.jpg",
        ] {
            assert_eq!(
                provider_for_cover_host_for_slot(url, CoverMediaType::Ebook),
                provider_for_cover_host_for_slot(url, CoverMediaType::Audiobook),
                "{url} must classify identically in both slots"
            );
        }
    }
}
