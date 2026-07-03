# Matching Inventory — every "same book?" method at `ab99693` (2026-07-02)

Phase-5 (one matching authority, audit M-002/M-008) ground truth. Every citation
below was opened and verified by the orchestrator at HEAD `ab99693` on 2026-07-02 —
do NOT re-derive; do NOT trust the 2026-06-28 audit's line numbers (stale).
Companion doc: `docs/matching-precedent-research-2026-07-02.md` (how Readarr, beets,
Lidarr, Picard, Calibre, Audiobookshelf, Kavita, Radarr/Sonarr do it).

## A. Fuzzy scorers

**1. Canonical (`livrarr-domain/src/text_norm.rs`)** — `title_tokens` :48-72,
`author_tokens` :74-96, `jaccard` :98-108, stopwords :7-8. Pipeline: `clean_title`
(title_cleanup.rs — series/edition/A-Novel marker strip) → CJK → char bigrams; else
NFKD accent-strip → lowercase → split on non-alphanumeric → drop 1-char tokens →
drop stopwords (a/an/the/of/and/in/on/for/to) → set-Jaccard. No colon special-case
(colon = punctuation split, subtitle tokens REMAIN). No threshold of its own.
Users: cover gate (0.6, `cover_gate.rs:3,67-91` — canonical, called once at the
merge chokepoint `apply_gr_cover_gate`), Google Books picker (0.75 + author-overlap
≥1, `google_books.rs:15-16,466-494`; only when no ISBN), shared provider picker
`audible::score_provider_candidates` (:349-380; 0.75 + overlap ≥1) used by Audible
(:150 ASIN-verify, :205 search), OpenLibrary (`provider_client.rs:991`), Goodreads
(`gr_best_match`, `provider_client.rs:1558-1572`).

**2. Identity engine's PRIVATE family (`livrarr-identity/src/english_identity_resolver.rs`)**
— `TITLE_MATCH_JACCARD = 0.75` :21; `normalize_match_title` :743-757 (lowercase,
drop bracketed content by depth, **TRUNCATE AT FIRST COLON** :751, collapse ws — NO
accent strip, NO stopwords, NO punctuation split); `token_set` :759-761 (whitespace
only); private `jaccard` :763-770; `title_matches` :772-782 (exact → else Jaccard ≥
0.75); `author_matches` :784-791 (**ANY single shared token**). Consumed by
`agree()` :479-517 — the quorum clustering predicate (add/refresh/convergence
identity): work-key equality → true; same-key-type different-values → false; else
authored pair = title_matches && author_matches, authorless = exact normalized
equality; ISBN/ASIN bridge only when titles don't contradict. Also `verify_gr_payload`
:293-304 (unsolicited-GR-key trust gate). THE wrong-book-merge surface (audit M-002,
C1 colon bug — Dresden test :931-963, step 2 rescued only by the anchor veto).

**3. Recognition scorer (`livrarr-matching/src/m4_scoring.rs`)** — `normalize`
:176-205 (NFKD accent-strip, lowercase, &→and, leading AND trailing article strip,
alnum+space only); metric = `string_similarity` :126-141 =
max(normalized-Levenshtein, word-sorted-Levenshtein) via `rapidfuzz` (:3, :217-234)
— NOTE both-empty → 1.0 (canonical jaccard's both-empty → 0.0); `canonicalize_author`
:207-215 (Last,First→First Last — 3rd author canonicalizer); composite
`score_candidate` title .45 / author .40 / year .10 / series .05; `fails_hard_gate`
:81-98 (title < .50 || author < .40 || no author → never auto-confirm). Consumers:
manual-import M1–M4 extraction clustering (`reconcile.rs:149-171`, 0.80), **RSS
release matching** (`rss_sync_workflow.rs:356-358`, threshold from
`rss_match_threshold` DEFAULT 0.80, migration `015_rss_sync.sql:22`, admin range
0.50-0.95), download-poller grab match (`download_poller.rs:171`, ≥ 0.6), silent GR
author auto-link (author_similarity ≥ 0.90; `handlers/{author,series,work}.rs`,
`series_query_service.rs:114`).

**4. Variant folder (`livrarr-matching/src/lib.rs:37-80` `normalize_title_variants`)**
— strips "(Unabridged)", **TRUNCATES AT FIRST COLON** :55-60, strips ", Book N",
falls through to m4 `normalize`. Used only in `title_similarity_with_variants`
(`m4_scoring.rs:111-122`): equal fold-keys → **forced 1.0**, guarded ONLY when BOTH
series positions known and differ (:113-116) — position missing on either side →
same-series siblings can score fake-perfect. Second, independent C1-shape colon bug
(no test covers it).

## B. Exact-equality gates

**5. DB identity key** — `normalize_for_matching` (`livrarr-domain/src/lib.rs:884-916`;
filename-derived: illegal chars/dot/underscore→space, collapse, lowercase; KEEPS
stopwords + accents) → stored `works.normalized_title/author` → `create_work`
`ON CONFLICT(user_id, normalized_title, normalized_author) DO NOTHING`
(`sqlite_work.rs:1330-1331`) — the final add backstop. Also: library scan
(root_folder handlers), series stub keys/matching (`series_link.rs`), Readarr
preview classification (`readarr_import_workflow.rs`, 9 sites).

**6. Library duplicate cascade (`livrarr-matching/src/work_dedup.rs`)** — `normalize`
:4-9 (alnum-only lowercase), `base_title`/`has_subtitle` :12-18, `canonical_author`
:21-35; `find_matching_work` :52-105: keys → exact norm title+author → base-title
ONLY when exactly one side has a subtitle; deliberately NO fuzzy fallback (:113-116).
Consumers: manual-import dedup (`manual_import.rs:464,701,818,1003`), series roster
(`series_query_service.rs:763,1205`), eager auto-match
(`best_candidate_index_lang` — HARD language gate — `work_service.rs:1901,1957`),
anchor-graft/cover-borrow same-work test (`authors_match`, work_service ~3161,3217).

**7. Strict "already in library" key** — `normalize_title_for_match`
(`work_dedup.rs:209-224`: lowercase, cut at `:` or " - ", strip leading article,
alnum+ws) → exact equality. Consumers: bibliography flag (`author_service.rs:619-622`),
anchor-graft + cover-upgrade (work_service ~3153-3214).

**8. Hardcover Tier 1 (`hardcover.rs:186-230`)** — trim+lowercase EXACT title
equality + exact author-in-list; ties by `users_read_count`. Strictest matcher in
the app.

## C. Overrides / tie-breaks

**9. Anchor arbitration** (`agree()` + `run_quorum` :311-417) — key equality wins
outright; key contradiction vetoes; anchored clusters always outrank ISBN/ASIN-only
clusters (:359-379); shared-ISBN + contradicting titles = collision (AC-020).

**10. Hardcover Tier 2 LLM (`hardcover.rs:232-249`, `llm_disambiguate` :443-557)** —
LIVE: when Tier 1 finds nothing, LLM picks an index from the numbered hit list.
The only real LLM-chooses-match left in the app.

**11. Goodreads — deterministic (wiki insight 13 was stale; corrected 2026-07-02).**
`is_gr_junk_edition` (`provider_client.rs:1540-1551`) + `gr_best_match` (:1558-1572,
shared 0.75 picker) + explicit abstain (:1441-1455 — "a fabricated key is worse than
no key"). Only LLM on the GR path = `llm_extract_payload` (:1172-1218), an
HTML-parse repair. RESIDUAL GATE: `select_providers`
(`english_identity_resolver.rs:240-242`) still excludes GR from the identity fan-out
unless `llm_configured` — leftover; PO call pending.

## Findings (all orchestrator-verified in source)

1. **Colon-truncation exists in TWO places**: #2 (:751) and #4 (:55-60). No
   precedent for it in any researched app.
2. **Add-time adopt-path mismatch (new bug)**: `works.normalized_*` written via
   `normalize_for_matching(cleaned_title)` (`work_service.rs:479-480`), but the
   Step-3 REQ-005 adopt lookup passes RAW `candidate.fields.title/author_name`
   (:569-580) into `find_normalized_match_no_anchor_for_user`
   (`sqlite_work.rs:796-823`, `normalize` = trim+lowercase only :263-265) — any
   title where cleaning differs (colon, dot, "(Unabridged)", double space) can NEVER
   match; the anchorless-duplicate absorb is silently skipped. The later dedup at
   :631 uses correctly-normalized inputs and partially covers; exact user-visible
   delta not yet traced (open item).
3. **Fake confidence score**: `ResolutionScore.title_jaccard` hardcoded `1.0` at its
   only construction site (`english_identity_resolver.rs:714-718`) — the score a
   user sees on NeedsConfirmation candidates is never computed.
4. **Dead scaffolding** (Phase-5 cleanup candidates): `MatchResult`/`DuplicateClass`
   (`matching/types.rs:106-128`, zero constructions), `HardcoverMatcher` trait
   (`livrarr-metadata/src/lib.rs:107-123`, only a cfg(test) stub impl),
   `cover_gate::CoverGateOutcome::AskLlm` branch (unreachable — sole caller
   hardcodes `llm_enabled=false`), `WorkField::normalization_class` (zero callers).
5. **Fifth/sixth accidental normalizers**: `sqlite_work.rs::normalize` (:263-265)
   and `Parser`-side variants — the app has ~6 title cleaners + 3 author
   canonicalizers where precedent apps carry 2-3 NAMED, purpose-built ones.
