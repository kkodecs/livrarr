use livrarr_domain::identity_matching::{
    author_verdict, id_verdict, parse_title, title_verdict, AuthorVerdict, IdEvidence, IdVerdict,
    TitleVerdict,
};
use livrarr_domain::Work;

/// True when the identity authority agrees these two (title, author) pairs
/// name the same book at absorb grade: exact main-title agreement
/// ([`TitleVerdict::Same`]) plus an author verdict of
/// [`AuthorVerdict::Agree`] or [`AuthorVerdict::Abstain`] (REQ-005 keeps
/// authorless agreement — it just requires the exact title match `Same`
/// already guarantees). Anything else — [`TitleVerdict::Grey`] (including
/// the former one-sided-subtitle base-title tier, now subsumed by the
/// parse), [`TitleVerdict::VetoVolume`], [`TitleVerdict::Different`], or an
/// [`AuthorVerdict::Grey`]/[`AuthorVerdict::Disagree`] — is NOT an absorb
/// match (REQ-008: never act on grey at this seat; a visible duplicate is
/// the safe, cheap-to-fix outcome).
fn identity_absorb_match(a_title: &str, a_author: &str, b_title: &str, b_author: &str) -> bool {
    let pa = parse_title(a_title);
    let pb = parse_title(b_title);
    if title_verdict(&pa, &pb) == TitleVerdict::Same {
        let a_names = [a_author.to_string()];
        let b_names = [b_author.to_string()];
        matches!(
            author_verdict(&a_names, &b_names),
            AuthorVerdict::Agree | AuthorVerdict::Abstain
        )
    } else {
        false
    }
}

/// True when two author strings refer to the same author under the
/// identity authority's author verdict (REQ-005): an unambiguous full-name
/// match, or one/both sides carrying no usable author (abstain — non-
/// evidence, never a block). Used to constrain anchor-grafting so an
/// author-scoped provider result that returned a same-title book by a
/// *different* author can't lend its work anchor (#97).
pub fn authors_match(a: &str, b: &str) -> bool {
    let a_names = [a.to_string()];
    let b_names = [b.to_string()];
    matches!(
        author_verdict(&a_names, &b_names),
        AuthorVerdict::Agree | AuthorVerdict::Abstain
    )
}

/// Provider keys for matching — pass whatever is available.
#[derive(Default)]
pub struct ProviderKeys<'a> {
    pub ol_key: Option<&'a str>,
    pub gr_key: Option<&'a str>,
    pub isbn_13: Option<&'a str>,
    pub asin: Option<&'a str>,
}

/// Identifier evidence the incoming candidate's [`ProviderKeys`] carry
/// (`ProviderKeys` has no hc_key field, so that slot is always absent —
/// `id_verdict`'s presence rules make an absent slot non-evidence).
fn keys_id_evidence<'a>(keys: &ProviderKeys<'a>) -> IdEvidence<'a> {
    IdEvidence {
        ol_key: keys.ol_key,
        gr_key: keys.gr_key,
        hc_key: None,
        isbn_13: keys.isbn_13,
        asin: keys.asin,
    }
}

fn work_id_evidence(w: &Work) -> IdEvidence<'_> {
    IdEvidence {
        ol_key: w.ol_key.as_deref(),
        gr_key: w.gr_key.as_deref(),
        hc_key: w.hc_key.as_deref(),
        isbn_13: w.isbn_13.as_deref(),
        asin: w.asin.as_deref(),
    }
}

/// Find a matching work in the existing library.
///
/// Match cascade (stops at first hit):
/// 1. Authority arbitration per candidate work (REQ-006), [`id_verdict`]
///    FIRST: same-provider work-key equality ([`IdVerdict::WorkKeyEqual`])
///    absorbs outright; a work-key contradiction
///    ([`IdVerdict::WorkKeyContradiction`]) excludes the work from EVERY
///    later arm — including the ISBN/ASIN equality arms, because a shared
///    edition ID with contradicting work keys is the collision shape
///    (AC-021: ISBN equal + same-provider work keys different → two
///    different real books, never a merge). Equality-before-contradiction
///    precedence lives inside `id_verdict` itself — trusted here, never
///    re-derived.
/// 2. Edition-ID equality (ISBN, then ASIN) — the legitimate bridge, over
///    the non-contradicted works only.
/// 3. The identity authority's absorb verdict ([`identity_absorb_match`]):
///    exact main-title agreement plus author agreement/abstain. There is
///    deliberately **no fourth, looser tier** — the former one-sided-subtitle
///    base-title match is subsumed by the parse: a tail on exactly one side
///    lands [`TitleVerdict::Grey`], and grey never absorbs (REQ-008). A pair
///    the authority calls grey surfaces as a visible library duplicate,
///    one click from being merged (the merge-two-works action).
pub fn find_matching_work<'a>(
    existing: &'a [Work],
    title: &str,
    author: &str,
    keys: &ProviderKeys<'_>,
) -> Option<&'a Work> {
    // 1. id_verdict arbitration: work-key equality wins outright; a
    //    contradicted work is excluded from every later arm.
    let incoming_ids = keys_id_evidence(keys);
    let mut eligible: Vec<&'a Work> = Vec::with_capacity(existing.len());
    for w in existing {
        match id_verdict(&incoming_ids, &work_id_evidence(w)) {
            IdVerdict::WorkKeyEqual => return Some(w),
            IdVerdict::WorkKeyContradiction => {}
            IdVerdict::EditionBridge | IdVerdict::NoEvidence => eligible.push(w),
        }
    }

    // 2. Edition-ID equality over the non-contradicted works.
    if let Some(key) = keys.isbn_13.filter(|k| !k.is_empty()) {
        if let Some(w) = eligible
            .iter()
            .copied()
            .find(|w| w.isbn_13.as_deref() == Some(key))
        {
            return Some(w);
        }
    }
    if let Some(key) = keys.asin.filter(|k| !k.is_empty()) {
        if let Some(w) = eligible
            .iter()
            .copied()
            .find(|w| w.asin.as_deref() == Some(key))
        {
            return Some(w);
        }
    }

    // 3. Text tier (contradicted works already excluded).
    eligible
        .into_iter()
        .find(|w| identity_absorb_match(&w.title, &w.author_name, title, author))
}

/// Pick the index of the `(title, author)` candidate that best matches the
/// given `title`/`author`, using the same identity-authority absorb verdict
/// as [`find_matching_work`] (minus provider keys) — see
/// [`identity_absorb_match`]. Author is matched too, so a corpus mixing
/// several authors (e.g. one provider query that returned more than the
/// requested author) is safe. Returns `None` when nothing matches — there is
/// deliberately **no fuzzy fallback**, so an absent title never resolves to
/// a wrong work (e.g. a sequel's cover).
pub fn best_candidate_index(
    candidates: &[(&str, &str)],
    title: &str,
    author: &str,
) -> Option<usize> {
    candidates
        .iter()
        .position(|&(t, a)| identity_absorb_match(t, a, title, author))
}

/// Language-aware variant of [`best_candidate_index`] for the manual-import
/// eager auto-match (#8). Additive: the language-blind [`best_candidate_index`]
/// is unchanged for its other callers.
///
/// `langs[i]` is the (raw) language tag of `candidates[i]`, if known.
///
/// Title/author comparison uses the identity authority's absorb verdict
/// ([`identity_absorb_match`] — exact main-title agreement plus author
/// agreement/abstain), so the same title compares equal here, in the scan's
/// query, and in the anchor-graft. There is deliberately **no fuzzy
/// fallback**. The language gate below is unchanged by Phase 5: it stays a
/// hard filter, independent of the title/author verdict.
///
/// Language filter:
/// * `file_language = Some(known)` → candidates with a *known, different*
///   language are excluded; a candidate with an *unknown* (None) language is
///   allowed (Hardcover/Goodreads don't tag language — excluding them would
///   collapse the search to OpenLibrary). The title+author match still gates.
/// * `file_language = None` → no language filter; ranks on title + author alone.
pub fn best_candidate_index_lang(
    candidates: &[(&str, &str)],
    langs: &[Option<&str>],
    title: &str,
    author: &str,
    file_language: Option<&str>,
) -> Option<usize> {
    let want_lang = file_language.and_then(normalize_lang);

    candidates.iter().enumerate().find_map(|(i, &(t, a))| {
        // Language filter: exclude a candidate only when it has a KNOWN,
        // different language. A candidate with an unknown (None) language is
        // allowed — Hardcover/Goodreads don't tag language, and excluding them
        // would collapse the search to OpenLibrary. The title+author match below
        // still gates, so a different-language edition with a mismatched title
        // is never selected.
        if let Some(ref want) = want_lang {
            if let Some(cand_lang) = langs.get(i).and_then(|l| l.and_then(normalize_lang)) {
                if cand_lang != *want {
                    return None;
                }
            }
        }
        identity_absorb_match(t, a, title, author).then_some(i)
    })
}

/// Normalize a language tag to its ISO 639-1 code via the domain authority,
/// so comparisons are robust to "en"/"eng"/"English"/"en-US" variants.
fn normalize_lang(raw: &str) -> Option<String> {
    livrarr_domain::normalization::normalize_language(raw)
}

/// Normalize a title for bibliography "already in library" matching and the
/// anchor-graft/cover-borrow same-work checks: the identity authority's
/// cleaned MAIN title only (REQ-014, ST-01 — this replaces the third
/// colon-cut site; site 7 of Phase 5's matching inventory). Deliberately
/// coarser than the stored identity key
/// ([`livrarr_domain::identity_matching::identity_key`]'s title component
/// encodes the full main+subtitle+volume triple): these seats compare
/// titles ACROSS sources whose subtitle conventions differ (an OL
/// bibliography entry vs a GB record vs a stored work), where main-title
/// agreement is the useful signal — the same tolerance the old colon-cut
/// gave them, minus its lookalike traps. The author dimension is handled
/// separately by [`authors_match`] at the same call sites.
pub fn normalize_title_for_match(title: &str) -> String {
    parse_title(title).main
}

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_domain::Work;

    fn make_work(title: &str, author: &str) -> Work {
        Work {
            identity_status: Default::default(),
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
            enrichment_status: livrarr_domain::EnrichmentStatus::Unenriched,
            enriched_at: None,
            enrichment_source: None,
            cover_url: None,
            cover_manual: false,
            cover_source: None,
            cover_trust: Default::default(),
            cover_width: 0,
            cover_height: 0,
            audiobook_cover_url: None,
            audiobook_cover_source: None,
            audiobook_cover_trust: Default::default(),
            audiobook_cover_width: 0,
            audiobook_cover_height: 0,
            monitor_ebook: false,
            monitor_audiobook: false,
            import_id: None,
            added_at: chrono::Utc::now(),
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
        // FLIPPED under the identity authority (Phase 5, sanctioned): the
        // old base-title tier absorbed a one-sided-subtitle pair; the
        // authority lands it TitleVerdict::Grey and grey never absorbs at
        // this seat (REQ-008) — the pair stays a visible duplicate, one
        // click from being merged.
        let works = vec![make_work("The Obstacle Is the Way", "Ryan Holiday")];
        let result = find_matching_work(
            &works,
            "The Obstacle Is the Way: The Timeless Art of Turning Trials into Triumph",
            "Ryan Holiday",
            &ProviderKeys::default(),
        );
        assert!(result.is_none());
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
        let result = find_matching_work(&works, "Dune", "Herbert, Frank", &ProviderKeys::default());
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
        let result = find_matching_work(&works, "Dune", "Brian Herbert", &ProviderKeys::default());
        assert!(result.is_none());
    }

    #[test]
    fn normalize_title_strips_subtitle() {
        assert_eq!(
            normalize_title_for_match("Dune: The Battle of Corrin"),
            "dune"
        );
        assert_eq!(
            normalize_title_for_match("Dune - The Battle of Corrin"),
            "dune"
        );
    }

    #[test]
    fn normalize_title_strips_articles() {
        assert_eq!(
            normalize_title_for_match("The Great Gatsby"),
            "great gatsby"
        );
        assert_eq!(
            normalize_title_for_match("A Farewell to Arms"),
            "farewell to arms"
        );
        assert_eq!(
            normalize_title_for_match("An Inspector Calls"),
            "inspector calls"
        );
    }

    #[test]
    fn normalize_title_strips_punctuation_and_collapses_whitespace() {
        // FLIPPED under the identity authority (Phase 5, sanctioned): the
        // old alnum-filter deleted apostrophes in place ("philosophers");
        // the authority's canonical phrase folds EVERY non-alphanumeric to
        // a token boundary, so an apostrophe now yields a separate token
        // ("philosopher s"). One recipe everywhere — no bypass cleaning;
        // recognition's Levenshtein still catches elided-apostrophe forms
        // in file names.
        assert_eq!(
            normalize_title_for_match("Harry Potter & the Philosopher's Stone"),
            "harry potter the philosopher s stone"
        );
    }

    #[test]
    fn normalize_title_case_insensitive() {
        assert_eq!(
            normalize_title_for_match("DUNE"),
            normalize_title_for_match("dune")
        );
    }

    #[test]
    fn authors_match_canonicalizes_last_first_and_case() {
        assert!(authors_match("Frank Herbert", "Herbert, Frank"));
        assert!(authors_match("frank herbert", "Frank Herbert"));
        assert!(!authors_match("Frank Herbert", "Brian Herbert"));
    }

    // ---- best_candidate_index_lang (#8 HARD language filter) -----------------

    #[test]
    fn lang_filter_picks_same_language_over_english() {
        // A German file with both a German and an English candidate picks German.
        let cands = [
            ("Der Steppenwolf", "Hermann Hesse"),
            ("Der Steppenwolf", "Hermann Hesse"),
        ];
        let langs = [Some("en"), Some("de")];
        let idx = best_candidate_index_lang(
            &cands,
            &langs,
            "Der Steppenwolf",
            "Hermann Hesse",
            Some("de"),
        );
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn lang_filter_abstains_when_only_other_language() {
        // A German file with ONLY an English candidate abstains (no match).
        let cands = [("Steppenwolf", "Hermann Hesse")];
        let langs = [Some("en")];
        let idx =
            best_candidate_index_lang(&cands, &langs, "Steppenwolf", "Hermann Hesse", Some("de"));
        assert_eq!(idx, None);
    }

    #[test]
    fn lang_filter_unknown_file_matches_on_title_author() {
        // Unknown file language → no language filter; ranks on title+author.
        let cands = [
            ("Some Other Book", "Hermann Hesse"),
            ("Steppenwolf", "Hermann Hesse"),
        ];
        let langs = [Some("en"), Some("en")];
        let idx = best_candidate_index_lang(&cands, &langs, "Steppenwolf", "Hermann Hesse", None);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn lang_filter_unknown_does_not_force_english() {
        // Unknown file language still selects a non-English candidate on title+author.
        let cands = [("Der Steppenwolf", "Hermann Hesse")];
        let langs = [Some("de")];
        let idx =
            best_candidate_index_lang(&cands, &langs, "Der Steppenwolf", "Hermann Hesse", None);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn lang_filter_normalizes_variants() {
        // "eng"/"English"/"en-US" all normalize to "en" on both sides.
        let cands = [("Steppenwolf", "Hermann Hesse")];
        let langs = [Some("English")];
        let idx =
            best_candidate_index_lang(&cands, &langs, "Steppenwolf", "Hermann Hesse", Some("eng"));
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn lang_filter_canonical_title_strips_article_and_subtitle() {
        // Canonical normalizer makes "The X: Subtitle" compare equal to "X".
        let cands = [("The Great Gatsby: A Novel", "F. Scott Fitzgerald")];
        let langs = [Some("en")];
        let idx = best_candidate_index_lang(
            &cands,
            &langs,
            "Great Gatsby",
            "F. Scott Fitzgerald",
            Some("en"),
        );
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn lang_filter_allows_unknown_language_candidate_when_file_known() {
        // A candidate with unknown (None) language is ALLOWED under a known file
        // language — Hardcover/Goodreads don't tag language; only a KNOWN,
        // different language is excluded. Title+author still gates.
        let cands = [("Steppenwolf", "Hermann Hesse")];
        let langs = [None];
        let idx =
            best_candidate_index_lang(&cands, &langs, "Steppenwolf", "Hermann Hesse", Some("de"));
        assert_eq!(idx, Some(0));
    }

    // ---- Phase 5: dedup cascade routed through the identity authority -------

    #[test]
    fn one_sided_subtitle_is_grey_never_absorbed() {
        // REQ-008: a tail on exactly one side (a true subtitle, not junk or a
        // series marker) lands TitleVerdict::Grey, and grey never absorbs at
        // this seat — the former base-title tier that used to auto-match this
        // shape is gone. The pair stays two distinct, visible works (a
        // one-click merge candidate), not a silent absorb.
        let works = vec![make_work("The Obstacle Is the Way", "Ryan Holiday")];
        let result = find_matching_work(
            &works,
            "The Obstacle Is the Way: The Timeless Art of Turning Trials into Triumph",
            "Ryan Holiday",
            &ProviderKeys::default(),
        );
        assert!(
            result.is_none(),
            "one-sided subtitle must not absorb (REQ-008); got {result:?}"
        );
    }

    #[test]
    fn one_sided_subtitle_is_grey_never_absorbed_best_candidate_index() {
        // Same REQ-008 rule at the eager-auto-match seat (best_candidate_index).
        let cands = [("The Obstacle Is the Way", "Ryan Holiday")];
        let idx = best_candidate_index(
            &cands,
            "The Obstacle Is the Way: The Timeless Art of Turning Trials into Triumph",
            "Ryan Holiday",
        );
        assert_eq!(idx, None);
    }

    #[test]
    fn junk_only_tail_still_absorbs() {
        // A junk tail ("A Novel") is ignored for identity — unlike a true
        // subtitle, it does not demote to grey, so this still auto-absorbs.
        let works = vec![make_work("Dune", "Frank Herbert")];
        let result = find_matching_work(
            &works,
            "Dune: A Novel",
            "Frank Herbert",
            &ProviderKeys::default(),
        );
        assert!(
            result.is_some(),
            "junk-only tail difference must still absorb"
        );
    }

    #[test]
    fn series_marker_tail_does_not_veto_when_only_one_side_carries_it() {
        // A one-sided series-volume marker (not a disagreement — the OTHER
        // side simply doesn't mention a volume) is grey, same rule as a true
        // subtitle: never a hard veto, never a silent absorb.
        let works = vec![make_work("Storm Front", "Jim Butcher")];
        let result = find_matching_work(
            &works,
            "Storm Front: The Dresden Files, Book 1",
            "Jim Butcher",
            &ProviderKeys::default(),
        );
        assert!(result.is_none(), "one-sided series marker must not absorb");
    }

    #[test]
    fn conflicting_series_volumes_never_absorb() {
        // A hard veto (conflicting volume numbers) blocks absorb even with
        // identical authors and no other disagreement (AC-003 at this seat).
        let works = vec![make_work("History of Rome: Volume 1", "Author")];
        let result = find_matching_work(
            &works,
            "History of Rome: Volume 2",
            "Author",
            &ProviderKeys::default(),
        );
        assert!(result.is_none());
    }

    #[test]
    fn shared_surname_different_given_name_never_absorbs() {
        // ST-02 kill: author_verdict lands Grey (shared surname only), which
        // the absorb seat treats identically to Disagree — never a match,
        // even though the title is identical.
        let works = vec![make_work("Some Title", "John Smith")];
        let result =
            find_matching_work(&works, "Some Title", "Jane Smith", &ProviderKeys::default());
        assert!(result.is_none());
    }

    #[test]
    fn provider_key_still_wins_regardless_of_title_text() {
        // Tier 1 (provider keys) is unchanged: an OL key match short-circuits
        // straight to absorb, independent of the identity-authority tier.
        let mut work = make_work("Dune", "Frank Herbert");
        work.ol_key = Some("OL1W".to_string());
        let works = vec![work];
        let result = find_matching_work(
            &works,
            "Completely Different Title",
            "Completely Different Author",
            &ProviderKeys {
                ol_key: Some("OL1W"),
                ..Default::default()
            },
        );
        assert!(result.is_some());
    }

    #[test]
    fn normalize_title_for_match_routes_through_parse_title_main() {
        // Site 7 (ST-01): the third colon-cut site dies, replaced by the
        // authority's parse. A dash-separated tail is handled identically to
        // a colon-separated one (both were already handled by the old cut;
        // this pins the new recipe's equivalent coverage).
        assert_eq!(
            normalize_title_for_match("Dune: The Battle of Corrin"),
            "dune"
        );
        assert_eq!(
            normalize_title_for_match("Dune - The Battle of Corrin"),
            "dune"
        );
        assert_eq!(
            normalize_title_for_match("The Great Gatsby"),
            "great gatsby"
        );
    }

    #[test]
    fn normalize_title_for_match_apostrophe_becomes_token_boundary() {
        // Known, deliberate behavior change from the old alnum-filter (which
        // deleted apostrophes with no boundary): the authority's canonical
        // phrase splits on all non-alphanumeric characters, so an apostrophe
        // becomes a token boundary rather than vanishing in place.
        assert_eq!(
            normalize_title_for_match("Philosopher's Stone"),
            "philosopher s stone"
        );
    }

    #[test]
    fn authors_match_still_rejects_shared_surname_only() {
        // ST-02 kill, pinned directly at the authors_match seat used by the
        // anchor-graft/cover-borrow same-work checks (site 6).
        assert!(!authors_match("Frank Herbert", "Brian Herbert"));
        assert!(authors_match("Frank Herbert", "Herbert, Frank"));
    }

    #[test]
    fn both_sided_sibling_subtitles_never_absorb() {
        // Series siblings: same main title, disagreeing subtitles →
        // TitleVerdict::Grey → never an absorb through the cascade. The
        // sibling stays its own work (AC-006 at this seat).
        let works = vec![make_work("Mistborn: The Final Empire", "Brandon Sanderson")];
        let result = find_matching_work(
            &works,
            "Mistborn: The Well of Ascension",
            "Brandon Sanderson",
            &ProviderKeys::default(),
        );
        assert!(result.is_none(), "a series sibling must never be absorbed");
    }

    #[test]
    fn work_key_contradiction_blocks_text_absorb() {
        // REQ-006: identical title + author but DIFFERENT same-provider
        // work keys — two different real books (the "La Nuit Des Temps"
        // shape). The contradiction gate blocks the text tier outright,
        // regardless of the perfect title/author agreement.
        let mut work = make_work("La Nuit Des Temps", "René Barjavel");
        work.ol_key = Some("OL1W".to_string());
        let works = vec![work];

        let contradicted = find_matching_work(
            &works,
            "La Nuit Des Temps",
            "René Barjavel",
            &ProviderKeys {
                ol_key: Some("OL2W"),
                ..Default::default()
            },
        );
        assert!(
            contradicted.is_none(),
            "a same-provider work-key contradiction must block text-tier absorb"
        );

        // Control: the identical call WITHOUT contradicting keys absorbs —
        // proving the gate (not the text tier) produced the None above.
        let uncontradicted = find_matching_work(
            &works,
            "La Nuit Des Temps",
            "René Barjavel",
            &ProviderKeys::default(),
        );
        assert!(uncontradicted.is_some());
    }

    #[test]
    fn isbn_collision_with_work_key_contradiction_never_absorbs() {
        // AC-021: ISBN equal + same-provider work keys different = the
        // collision shape. The work-key contradiction outranks the
        // edition-ID agreement — the shared ISBN must NOT bridge the pair
        // into an absorb, even with identical title and author.
        let mut work = make_work("Same Title", "Same Author");
        work.ol_key = Some("OL1W".to_string());
        work.isbn_13 = Some("9780000000001".to_string());
        let works = vec![work];

        let collided = find_matching_work(
            &works,
            "Same Title",
            "Same Author",
            &ProviderKeys {
                ol_key: Some("OL2W"),
                isbn_13: Some("9780000000001"),
                ..Default::default()
            },
        );
        assert!(
            collided.is_none(),
            "a work-key contradiction must outrank shared-ISBN agreement (AC-021)"
        );

        // Control: the same shared ISBN WITHOUT the contradicting work key
        // absorbs via the edition-ID bridge — proving the contradiction
        // (not the ISBN arm itself) produced the None above.
        let bridged = find_matching_work(
            &works,
            "Same Title",
            "Same Author",
            &ProviderKeys {
                isbn_13: Some("9780000000001"),
                ..Default::default()
            },
        );
        assert!(bridged.is_some());
    }
}
