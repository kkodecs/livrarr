use std::collections::HashMap;
use std::path::Path;

use livrarr_domain::{CoverMediaType, CoverResolution, MetadataProvider};

use crate::cover_rank::{rank_index, CoverRankModel};
use crate::NormalizedWorkDetail;

/// Emergency v9 containment: Goodreads' drifted book-page parser currently
/// emits unrelated cover URLs while its identity fields remain usable. Keep
/// Goodreads metadata/search/identity enabled, but exclude it at this one
/// cover-candidate seam. Re-enabling this flag belongs to the next round and
/// requires a parser fix pinned against a captured drifted-page fixture.
pub const GOODREADS_COVER_CANDIDATES_ENABLED: bool = false;

fn cover_candidate_enabled(provider: MetadataProvider) -> bool {
    provider != MetadataProvider::Goodreads || GOODREADS_COVER_CANDIDATES_ENABLED
}

pub fn is_good_enough(width: u32, height: u32) -> bool {
    width >= 400 && height >= 600
}

pub fn measure_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

/// Pick the first usable provider cover from the caller's format-specific
/// rank order. Provider outcomes and title-keyword heuristics do not create a
/// second selection tier: eligibility and the single rank table are the whole
/// decision.
pub fn resolve_cover(
    media_type: CoverMediaType,
    priority_list: &[MetadataProvider],
    eligible: &HashMap<MetadataProvider, Option<&NormalizedWorkDetail>>,
) -> Option<CoverResolution> {
    priority_list.iter().find_map(|provider| {
        if !cover_candidate_enabled(*provider) {
            return None;
        }
        let detail = eligible.get(provider)?.as_ref()?;
        let url = detail.cover_url.as_deref().filter(|url| !url.is_empty())?;
        Some(CoverResolution {
            url: url.to_string(),
            source: format!("{provider:?}").to_lowercase(),
            media_type,
        })
    })
}

/// Compare an offered cover with the incumbent using the quality floor,
/// dimensions, and the one format-aware provider rank table.
pub fn should_upgrade_same_tier(
    current_w: u32,
    current_h: u32,
    new_w: u32,
    new_h: u32,
    current_source: Option<&str>,
    new_source: &str,
    rank_model: CoverRankModel,
) -> bool {
    let current_good = is_good_enough(current_w, current_h);
    let new_good = is_good_enough(new_w, new_h);

    let priority_of = |source: &str| -> usize {
        match provider_from_source_str(source) {
            Some(provider) => rank_index(provider, rank_model),
            None => usize::MAX,
        }
    };

    match (new_good, current_good) {
        (true, false) => true,
        (false, true) => false,
        (true, true) => priority_of(new_source) < priority_of(current_source.unwrap_or("")),
        (false, false) => {
            let new_area = (new_w as u64) * (new_h as u64);
            let current_area = (current_w as u64) * (current_h as u64);
            if new_area != current_area {
                new_area > current_area
            } else {
                priority_of(new_source) < priority_of(current_source.unwrap_or(""))
            }
        }
    }
}

fn provider_from_source_str(source: &str) -> Option<MetadataProvider> {
    match source {
        "goodreads" => Some(MetadataProvider::Goodreads),
        "hardcover" => Some(MetadataProvider::Hardcover),
        "openlibrary" => Some(MetadataProvider::OpenLibrary),
        "audnexus" => Some(MetadataProvider::Audnexus),
        "readarr" => Some(MetadataProvider::Readarr),
        "googlebooks" => Some(MetadataProvider::GoogleBooks),
        "audible" => Some(MetadataProvider::Audible),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_good_enough_boundary() {
        assert!(is_good_enough(400, 600));
        assert!(!is_good_enough(399, 600));
        assert!(!is_good_enough(400, 599));
        assert!(!is_good_enough(0, 0));
    }

    #[test]
    fn selection_uses_the_supplied_rank_order() {
        let goodreads = NormalizedWorkDetail {
            cover_url: Some("https://gr.test/cover.jpg".to_string()),
            ..Default::default()
        };
        let hardcover = NormalizedWorkDetail {
            cover_url: Some("https://hc.test/cover.jpg".to_string()),
            ..Default::default()
        };
        let eligible = HashMap::from([
            (MetadataProvider::Goodreads, Some(&goodreads)),
            (MetadataProvider::Hardcover, Some(&hardcover)),
        ]);

        let selected = resolve_cover(
            CoverMediaType::Ebook,
            &[MetadataProvider::Hardcover, MetadataProvider::Goodreads],
            &eligible,
        )
        .expect("a ranked candidate should be selected");

        assert_eq!(selected.url, "https://hc.test/cover.jpg");
        assert_eq!(selected.source, "hardcover");
    }

    #[test]
    fn goodreads_is_absent_from_cover_candidate_assembly() {
        let goodreads = NormalizedWorkDetail {
            cover_url: Some("https://gr.test/wrong-book.jpg".to_string()),
            ..Default::default()
        };
        let eligible = HashMap::from([(MetadataProvider::Goodreads, Some(&goodreads))]);

        assert!(resolve_cover(
            CoverMediaType::Ebook,
            &[MetadataProvider::Goodreads],
            &eligible,
        )
        .is_none());
    }

    #[test]
    fn goodreads_exclusion_falls_through_to_the_next_ranked_source() {
        let goodreads = NormalizedWorkDetail {
            cover_url: Some("https://gr.test/wrong-book.jpg".to_string()),
            ..Default::default()
        };
        let hardcover = NormalizedWorkDetail {
            cover_url: Some("https://hc.test/right-book.jpg".to_string()),
            ..Default::default()
        };
        let eligible = HashMap::from([
            (MetadataProvider::Goodreads, Some(&goodreads)),
            (MetadataProvider::Hardcover, Some(&hardcover)),
        ]);

        let selected = resolve_cover(
            CoverMediaType::Ebook,
            &[MetadataProvider::Goodreads, MetadataProvider::Hardcover],
            &eligible,
        )
        .expect("Hardcover should remain after Goodreads is contained");

        assert_eq!(selected.source, "hardcover");
        assert_eq!(selected.url, "https://hc.test/right-book.jpg");
    }

    #[test]
    fn no_usable_candidate_returns_none() {
        let empty = NormalizedWorkDetail::default();
        let eligible = HashMap::from([(MetadataProvider::Goodreads, Some(&empty))]);
        assert!(resolve_cover(
            CoverMediaType::Ebook,
            &[MetadataProvider::Goodreads],
            &eligible,
        )
        .is_none());
    }

    #[test]
    fn comparator_prefers_quality_then_rank() {
        assert!(should_upgrade_same_tier(
            200,
            300,
            400,
            600,
            Some("goodreads"),
            "hardcover",
            CoverRankModel::EbookEnglish,
        ));
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
    fn measure_dimensions_nonexistent_file() {
        assert_eq!(measure_dimensions(Path::new("/nonexistent/path.jpg")), None);
    }
}
