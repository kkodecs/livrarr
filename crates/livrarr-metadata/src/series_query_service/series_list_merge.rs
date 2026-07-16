use livrarr_db::SeriesCacheEntry;
use livrarr_domain::services::*;
use livrarr_domain::*;

pub(super) fn build_merged_series_list(
    cache_entries: &[SeriesCacheEntry],
    db_series: &[Series],
    works: &[Work],
) -> Vec<AuthorSeriesItemView> {
    let mut matched_db_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut views: Vec<AuthorSeriesItemView> = cache_entries
        .iter()
        .map(|ce| {
            let db_match = if ce.gr_key.is_empty() {
                db_series.iter().find(|s| s.name == ce.name)
            } else {
                // An unresolved stub has no gr_key to match on yet, so it
                // never satisfies the exact-key check above — fall back to
                // the same normalized-name rule promote_stub already trusts
                // for silent resolution, so a placeholder with an in-library
                // book count doesn't show up as a separate, redundant row
                // next to its real Goodreads entry.
                db_series
                    .iter()
                    .find(|s| s.gr_key == ce.gr_key)
                    .or_else(|| {
                        let normalized_ce_name = identity_matching::identity_key(&ce.name, "").0;
                        db_series.iter().find(|s| {
                            crate::series_link::is_stub_key(&s.gr_key)
                                && identity_matching::identity_key(&s.name, "").0
                                    == normalized_ce_name
                        })
                    })
            };

            let (id, monitor_ebook, monitor_audiobook) = if let Some(s) = db_match {
                matched_db_ids.insert(s.id);
                (Some(s.id), s.monitor_ebook, s.monitor_audiobook)
            } else {
                (None, false, false)
            };

            let works_in_library = if let Some(s) = db_match {
                works.iter().filter(|w| w.series_id == Some(s.id)).count() as i64
            } else {
                works
                    .iter()
                    .filter(|w| w.series_name.as_deref() == Some(&ce.name))
                    .count() as i64
            };

            AuthorSeriesItemView {
                id,
                name: ce.name.clone(),
                gr_key: ce.gr_key.clone(),
                book_count: ce.book_count,
                monitor_ebook,
                monitor_audiobook,
                works_in_library,
                language: ce.language.clone(),
            }
        })
        .collect();

    // REQ-003: DB rows (stubs included) that matched no cache entry are
    // appended, never dropped — FK-counted. A stub's gr_key is exposed as
    // empty: "stub:" keys are internal, and the UI hides GR links for
    // keyless series.
    for s in db_series {
        if matched_db_ids.contains(&s.id) {
            continue;
        }
        let works_in_library = works.iter().filter(|w| w.series_id == Some(s.id)).count() as i64;
        let is_stub = crate::series_link::is_stub_key(&s.gr_key);
        views.push(AuthorSeriesItemView {
            id: Some(s.id),
            name: s.name.clone(),
            gr_key: if is_stub {
                String::new()
            } else {
                s.gr_key.clone()
            },
            book_count: if is_stub { 0 } else { s.work_count },
            monitor_ebook: s.monitor_ebook,
            monitor_audiobook: s.monitor_audiobook,
            works_in_library,
            // DB-only stub, no matching cache entry — no detected-language
            // signal exists for it.
            language: None,
        });
    }

    views
}

#[cfg(test)]
mod author_series_list_merge_tests {
    use super::*;
    use crate::series_link::stub_key_for;
    use chrono::Utc;

    fn cache_entry(name: &str, gr_key: &str, book_count: i32) -> SeriesCacheEntry {
        SeriesCacheEntry {
            name: name.to_string(),
            gr_key: gr_key.to_string(),
            book_count,
            language: None,
        }
    }

    fn stub_series(id: i64, name: &str) -> Series {
        Series {
            id,
            user_id: 1,
            author_id: 1,
            name: name.to_string(),
            gr_key: stub_key_for(name),
            monitor_ebook: false,
            monitor_audiobook: false,
            monitor_language: None,
            work_count: 0,
            added_at: Utc::now(),
        }
    }

    fn linked_work(id: i64, series_id: i64) -> Work {
        Work {
            id,
            series_id: Some(series_id),
            ..Default::default()
        }
    }

    #[test]
    fn stub_with_matching_name_merges_into_cache_entry_not_appended_separately() {
        let stub = stub_series(7, "Bloodsworn Saga");
        let db_series = vec![stub];
        let works = vec![linked_work(1, 7), linked_work(2, 7)];
        let cache_entries = vec![cache_entry("Bloodsworn Saga", "58486", 3)];

        let views = build_merged_series_list(&cache_entries, &db_series, &works);

        assert_eq!(
            views.len(),
            1,
            "expected one merged row, not a real + placeholder pair"
        );
        let v = &views[0];
        assert_eq!(v.id, Some(7));
        assert_eq!(v.gr_key, "58486");
        assert_eq!(v.book_count, 3);
        assert_eq!(v.works_in_library, 2);
    }

    #[test]
    fn stub_with_no_matching_cache_entry_still_appended() {
        let stub = stub_series(9, "Some Other Series");
        let db_series = vec![stub];
        let works = vec![linked_work(1, 9)];
        let cache_entries = vec![cache_entry("Bloodsworn Saga", "58486", 3)];

        let views = build_merged_series_list(&cache_entries, &db_series, &works);

        assert_eq!(
            views.len(),
            2,
            "an unmatched stub must still be appended (REQ-003)"
        );
        let stub_view = views
            .iter()
            .find(|v| v.id == Some(9))
            .expect("stub row present");
        assert_eq!(
            stub_view.gr_key, "",
            "stub gr_key stays masked when unmatched"
        );
        assert_eq!(stub_view.book_count, 0);
        assert_eq!(stub_view.works_in_library, 1);
    }
}
