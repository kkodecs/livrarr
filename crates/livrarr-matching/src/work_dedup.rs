use livrarr_domain::Work;

/// Normalize a string for dedup comparison: strip non-alphanumeric, lowercase.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Extract the base title (before the first colon), trimmed.
fn base_title(s: &str) -> &str {
    s.split_once(':').map(|(base, _)| base.trim()).unwrap_or(s)
}

fn has_subtitle(s: &str) -> bool {
    s.contains(':')
}

/// Canonicalize author: "Last, First" → "First Last", then normalize.
fn canonical_author(name: &str) -> String {
    let trimmed = name.trim();
    let reordered = if let Some((last, first)) = trimmed.split_once(',') {
        let first = first.trim();
        let last = last.trim();
        if !first.is_empty() && !last.is_empty() {
            format!("{first} {last}")
        } else {
            trimmed.to_string()
        }
    } else {
        trimmed.to_string()
    };
    normalize(&reordered)
}

/// Provider keys for matching — pass whatever is available.
#[derive(Default)]
pub struct ProviderKeys<'a> {
    pub ol_key: Option<&'a str>,
    pub gr_key: Option<&'a str>,
    pub isbn_13: Option<&'a str>,
    pub asin: Option<&'a str>,
}

/// Find a matching work in the existing library.
///
/// Match cascade (stops at first hit):
/// 1. Provider key match (OL, GR, ISBN, ASIN)
/// 2. Exact normalized title + canonical author
/// 3. Base-title + canonical author (only when one side has a subtitle and the other doesn't)
pub fn find_matching_work<'a>(
    existing: &'a [Work],
    title: &str,
    author: &str,
    keys: &ProviderKeys<'_>,
) -> Option<&'a Work> {
    // 1. Provider key match
    if let Some(key) = keys.ol_key.filter(|k| !k.is_empty()) {
        if let Some(w) = existing.iter().find(|w| w.ol_key.as_deref() == Some(key)) {
            return Some(w);
        }
    }
    if let Some(key) = keys.gr_key.filter(|k| !k.is_empty()) {
        if let Some(w) = existing.iter().find(|w| w.gr_key.as_deref() == Some(key)) {
            return Some(w);
        }
    }
    if let Some(key) = keys.isbn_13.filter(|k| !k.is_empty()) {
        if let Some(w) = existing.iter().find(|w| w.isbn_13.as_deref() == Some(key)) {
            return Some(w);
        }
    }
    if let Some(key) = keys.asin.filter(|k| !k.is_empty()) {
        if let Some(w) = existing.iter().find(|w| w.asin.as_deref() == Some(key)) {
            return Some(w);
        }
    }

    let norm_title = normalize(title);
    let norm_author = canonical_author(author);

    // 2. Exact normalized title + author
    if let Some(w) = existing.iter().find(|w| {
        normalize(&w.title) == norm_title && canonical_author(&w.author_name) == norm_author
    }) {
        return Some(w);
    }

    // 3. Base-title match (only when exactly one side has a subtitle)
    let incoming_has_sub = has_subtitle(title);
    let norm_base = normalize(base_title(title));

    existing.iter().find(|w| {
        let existing_has_sub = has_subtitle(&w.title);

        // Only match when one has subtitle and other doesn't
        if incoming_has_sub == existing_has_sub {
            return false;
        }

        let w_norm_base = normalize(base_title(&w.title));
        w_norm_base == norm_base && canonical_author(&w.author_name) == norm_author
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_domain::Work;

    fn make_work(title: &str, author: &str) -> Work {
        Work {
            id: 1,
            user_id: 1,
            title: title.to_string(),
            sort_title: None,
            subtitle: None,
            original_title: None,
            author_name: author.to_string(),
            author_id: None,
            description: None,
            year: None,
            series_id: None,
            series_name: None,
            series_position: None,
            genres: None,
            language: None,
            page_count: None,
            duration_seconds: None,
            publisher: None,
            publish_date: None,
            ol_key: None,
            hc_key: None,
            gr_key: None,
            isbn_13: None,
            asin: None,
            narrator: None,
            narration_type: None,
            abridged: false,
            rating: None,
            rating_count: None,
            enrichment_status: livrarr_domain::EnrichmentStatus::Pending,
            enrichment_retry_count: 0,
            enriched_at: None,
            enrichment_source: None,
            cover_url: None,
            cover_manual: false,
            monitor_ebook: false,
            monitor_audiobook: false,
            import_id: None,
            added_at: chrono::Utc::now(),
            metadata_source: None,
            detail_url: None,
        }
    }

    #[test]
    fn exact_title_match() {
        let works = vec![make_work("Dune", "Frank Herbert")];
        let result = find_matching_work(&works, "Dune", "Frank Herbert", &ProviderKeys::default());
        assert!(result.is_some());
    }

    #[test]
    fn case_insensitive_match() {
        let works = vec![make_work("The Obstacle Is the Way", "Ryan Holiday")];
        let result = find_matching_work(
            &works,
            "the obstacle is the way",
            "ryan holiday",
            &ProviderKeys::default(),
        );
        assert!(result.is_some());
    }

    #[test]
    fn subtitle_match_one_side() {
        let works = vec![make_work("The Obstacle Is the Way", "Ryan Holiday")];
        let result = find_matching_work(
            &works,
            "The Obstacle Is the Way: The Timeless Art of Turning Trials into Triumph",
            "Ryan Holiday",
            &ProviderKeys::default(),
        );
        assert!(result.is_some());
    }

    #[test]
    fn different_subtitles_no_match() {
        let works = vec![make_work(
            "A Brief History of Time: From the Big Bang to Black Holes",
            "Stephen Hawking",
        )];
        let result = find_matching_work(
            &works,
            "A Brief History of Time: A Reader's Companion",
            "Stephen Hawking",
            &ProviderKeys::default(),
        );
        assert!(result.is_none());
    }

    #[test]
    fn author_last_first_normalization() {
        let works = vec![make_work("Dune", "Frank Herbert")];
        let result = find_matching_work(
            &works,
            "Dune",
            "Herbert, Frank",
            &ProviderKeys::default(),
        );
        assert!(result.is_some());
    }

    #[test]
    fn provider_key_match() {
        let mut work = make_work("Dune", "Frank Herbert");
        work.isbn_13 = Some("9780441013593".to_string());
        let works = vec![work];
        let result = find_matching_work(
            &works,
            "Different Title",
            "Different Author",
            &ProviderKeys {
                isbn_13: Some("9780441013593"),
                ..Default::default()
            },
        );
        assert!(result.is_some());
    }

    #[test]
    fn different_author_no_match() {
        let works = vec![make_work("Dune", "Frank Herbert")];
        let result = find_matching_work(
            &works,
            "Dune",
            "Brian Herbert",
            &ProviderKeys::default(),
        );
        assert!(result.is_none());
    }
}
