use livrarr_db::SeriesRosterEntry;
use livrarr_domain::services::*;
use livrarr_domain::*;

/// REQ-010 merge: every roster entry becomes a row — in-library when a linked
/// work matches (normalized GR key first, then the shared work-matching
/// authority), missing otherwise. Linked works no roster entry claimed are
/// appended, never dropped. Pure — unit-tested below.
pub(super) fn merge_roster_with_works(
    roster: &[SeriesRosterEntry],
    works: Vec<SeriesWorkView>,
    author_name: &str,
) -> Vec<SeriesBookRow> {
    use livrarr_domain::normalization::normalize_gr_key;

    let work_refs: Vec<Work> = works.iter().map(|sw| sw.work.clone()).collect();
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, sw) in works.iter().enumerate() {
        if let Some(k) = sw.work.gr_key.as_deref().and_then(normalize_gr_key) {
            by_key.entry(k).or_insert(i);
        }
    }

    let mut sorted: Vec<&SeriesRosterEntry> = roster.iter().collect();
    sorted.sort_by(|a, b| {
        let pa = a.position.unwrap_or(f64::MAX);
        let pb = b.position.unwrap_or(f64::MAX);
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut slots: Vec<Option<SeriesWorkView>> = works.into_iter().map(Some).collect();
    let mut rows = Vec::with_capacity(sorted.len());

    for entry in sorted {
        let idx = normalize_gr_key(&entry.gr_key)
            .and_then(|k| by_key.get(&k).copied())
            .or_else(|| {
                livrarr_matching::work_dedup::find_matching_work(
                    &work_refs,
                    &entry.title,
                    author_name,
                    &livrarr_matching::work_dedup::ProviderKeys {
                        gr_key: Some(&entry.gr_key),
                        ..Default::default()
                    },
                )
                .and_then(|m| work_refs.iter().position(|w| w.id == m.id))
            });

        match idx.and_then(|i| slots[i].take()) {
            Some(sw) => rows.push(SeriesBookRow::InLibrary {
                position: entry.position.or(sw.work.series_position),
                entry: Box::new(sw),
            }),
            None => rows.push(SeriesBookRow::Missing {
                position: entry.position,
                title: entry.title.clone(),
                year: entry.year,
            }),
        }
    }

    // Linked works the roster didn't claim — appended, never dropped.
    let mut leftovers: Vec<SeriesWorkView> = slots.into_iter().flatten().collect();
    leftovers.sort_by(|a, b| {
        let pa = a.work.series_position.unwrap_or(f64::MAX);
        let pb = b.work.series_position.unwrap_or(f64::MAX);
        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
    });
    for sw in leftovers {
        rows.push(SeriesBookRow::InLibrary {
            position: sw.work.series_position,
            entry: Box::new(sw),
        });
    }

    rows
}

#[cfg(test)]
mod roster_merge_tests {
    use super::*;

    fn entry(title: &str, gr_key: &str, position: Option<f64>) -> SeriesRosterEntry {
        SeriesRosterEntry {
            title: title.to_string(),
            gr_key: gr_key.to_string(),
            position,
            year: None,
        }
    }

    fn linked_work(
        id: i64,
        title: &str,
        gr_key: Option<&str>,
        position: Option<f64>,
    ) -> SeriesWorkView {
        SeriesWorkView {
            work: Work {
                id,
                title: title.to_string(),
                author_name: "Jim Butcher".to_string(),
                gr_key: gr_key.map(str::to_string),
                series_position: position,
                ..Default::default()
            },
            library_items: vec![],
        }
    }

    fn titles(rows: &[SeriesBookRow]) -> Vec<(String, bool)> {
        rows.iter()
            .map(|r| match r {
                SeriesBookRow::InLibrary { entry, .. } => (entry.work.title.clone(), true),
                SeriesBookRow::Missing { title, .. } => (title.clone(), false),
            })
            .collect()
    }

    #[test]
    fn matches_by_normalized_gr_key() {
        let roster = vec![entry("Storm Front", "12345.Storm_Front", Some(1.0))];
        let works = vec![linked_work(
            1,
            "Storm Front (different title casing)",
            Some("12345"),
            Some(1.0),
        )];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert!(matches!(rows[0], SeriesBookRow::InLibrary { .. }));
    }

    #[test]
    fn falls_back_to_title_match_when_no_key() {
        let roster = vec![entry("Storm Front", "99999", Some(1.0))];
        let works = vec![linked_work(1, "Storm Front", None, Some(1.0))];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert!(matches!(rows[0], SeriesBookRow::InLibrary { .. }));
    }

    #[test]
    fn unmatched_entry_is_missing() {
        let roster = vec![
            entry("Storm Front", "1", Some(1.0)),
            entry("Fool Moon", "2", Some(2.0)),
        ];
        let works = vec![linked_work(1, "Storm Front", Some("1"), Some(1.0))];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), true),
                ("Fool Moon".to_string(), false)
            ]
        );
    }

    #[test]
    fn linked_work_absent_from_roster_is_appended_never_dropped() {
        let roster = vec![entry("Storm Front", "1", Some(1.0))];
        let works = vec![
            linked_work(1, "Storm Front", Some("1"), Some(1.0)),
            linked_work(2, "Side Jobs", Some("777"), Some(12.5)),
        ];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), true),
                ("Side Jobs".to_string(), true)
            ]
        );
    }

    #[test]
    fn rows_follow_roster_position_order() {
        let roster = vec![
            entry("Fool Moon", "2", Some(2.0)),
            entry("Storm Front", "1", Some(1.0)),
        ];
        let rows = merge_roster_with_works(&roster, vec![], "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), false),
                ("Fool Moon".to_string(), false)
            ]
        );
    }

    #[test]
    fn one_work_claimed_once_second_entry_reads_missing() {
        let roster = vec![
            entry("Storm Front", "1", Some(1.0)),
            entry("Storm Front (Reissue)", "1", Some(2.0)),
        ];
        let works = vec![linked_work(1, "Storm Front", Some("1"), Some(1.0))];
        let rows = merge_roster_with_works(&roster, works, "Jim Butcher");
        assert_eq!(
            titles(&rows),
            vec![
                ("Storm Front".to_string(), true),
                ("Storm Front (Reissue)".to_string(), false)
            ]
        );
    }
}
