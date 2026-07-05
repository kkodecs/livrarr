# Phase 5 Spec — One Matching Authority (metadata-remediation)

Date: 2026-07-02 · Branch `metadata-remediation` @ `ab99693` · Status: DRAFT for PO walkthrough
Findings closed: M-002, M-008 (audit 2026-06-28) + inventory findings 1–5 (2026-07-02).
Decision record: D1–D10, locked with the PO 2026-07-02 (two sessions); cross-family
gut check (Gemini + Codex) returned SOUND on the full set.

Companion ground truth (do not re-derive):
- `docs/matching-inventory-2026-07-02.md` — all 11 current matching sites, verified at `ab99693`
- `docs/matching-precedent-research-2026-07-02.md` — Readarr/beets/Calibre/Picard/ABS/Kavita/Radarr/Sonarr
- `docs/metadata-remediation-plan-2026-06-29.md` — Phase 5 row (group C, highest blast radius)

---

## 1. Goal

Collapse ~11 accidental "same book?" matching sites (6 title cleaners, 3 author
canonicalizers) into **two named, deliberate matchers** — a strict **Identity
authority** (what a book IS) and a typo-tolerant **Recognition matcher** (reading
outside-world text) — with explicit uncertainty handling: when the Identity
authority is not sure, it never merges, never absorbs; it asks (interactive) or
parks (background). Kill the colon-truncation bug class at every site.

## 2. Decision record (authoritative; PO-locked)

| ID | Decision |
|----|----------|
| D1 | Two brains: strict Identity authority + typo-tolerant Recognition matcher. All other matchers collapse into these two. |
| D2 | Grey-zone policy = f(stakes × accuracy) per decision seat. Provider-data merges and absorb-into-existing NEVER act on grey: interactive → show candidates (one click); background → park as needs-review and move on. Covers keep guessing (see D5). Bars recalibrated later from rollout measurement only. |
| D3 | Title recipe: parse, don't truncate. Main title is the ONLY thing that can make two titles match. The tail (subtitle/series/junk) can only veto (conflicting volume/series numbers) or demote to grey (substantive disagreement, or tail on one side only). Junk tails ignored. Colon-truncation removed everywhere. |
| D4 | Identity auto bar = EXACT equality of cleaned main titles. Near (≈0.75+) = grey candidate. IDs outrank text entirely. |
| D5 | Cover acceptance bar stays 0.6 title similarity; unverified flag; user override locks (M-008 CLOSED: intentional, keep). |
| D6 | Goodreads unlocked without LLM: delete the leftover `llm_configured` gate in identity provider selection. |
| D7 | Per-user default-language setting replaces the hardcoded "en" assumption. Declared-language mismatch = hard veto (identity merge/absorb; recognition download-match). Language-silent records: auto-apply only when the work's language equals the user's default; otherwise grey. Recognition side made explicit per r1 review: language-silent releases auto-match only default-language works (REQ-011 — derived from D2, PO may override). |
| D8 | Rollout: trap tests green FIRST → DB snapshot → old-vs-new zero-write decision diff over the real library → PO reviews EVERY changed decision → cutover. The diff doubles as the grey-zone accuracy measurement. |
| D9 | Author rule = D4 analog: full cleaned author-name match (order-normalized); multi-author lists match on ≥1 shared full name; any-shared-token matching dies; partial/initials-ambiguity → grey. |
| D10 | Hardcover Tier-2 LLM pick DELETED (PO, post-walkthrough consistency call). After unification every provider has the same deterministic middle tier (grey candidates → human); an HC-only LLM chooser is an unprincipled exception, and its background auto-settling contradicts D2. No live LLM-chooses-match remains anywhere. LLM keeps its non-matching jobs (cleanup, HTML repair). |

Confer fold-ins (Codex, accepted): conservative dash-splitting; LLM-repaired payload
fields get zero extra trust; anchor-arbitration rules stated explicitly (REQ-007);
"(Unabridged)" stripped for identity only — remains edition/variant info for release
selection.

## 3. System Truths (verified environment/code facts)

All citations verified at `ab99693` via `docs/matching-inventory-2026-07-02.md`
unless noted. The 2026-06-28 audit's line numbers are STALE — do not use them.

- **ST-01** Colon-truncation exists at **three** sites: `english_identity_resolver.rs:751`
  (`normalize_match_title`, the merge seat), `livrarr-matching/src/lib.rs:55-60`
  (`normalize_title_variants`, no test coverage), and `work_dedup.rs:209-224`
  (`normalize_title_for_match` cuts at `:` and ` - `; feeds bibliography flag +
  anchor-graft/cover-borrow same-work tests). No researched app truncates at colons.
- **ST-02** Author agreement in the identity engine is ANY single shared token
  (`author_matches`, `english_identity_resolver.rs:784-791`) — "John Smith" matches
  "Jane Smith".
- **ST-03** The June F1 incident was wrong-BOOK adoption via provider title/author
  fuzzy fallback on anchor-less works (8/13 foreign works damaged; reverted). The
  systematic lookalikes are same-author sibling volumes and study guides.
- **ST-04** Add-time adopt lookup passes RAW title/author (`work_service.rs:569-580`)
  against columns written via `normalize_for_matching(cleaned_title)` (`:479-480`),
  through a DB helper that only trims+lowercases (`sqlite_work.rs:263-265,796-823`)
  — subtitled titles can never match; the `:631` dedup partially covers.
- **ST-05** `ResolutionScore.title_jaccard` is hardcoded `1.0` at its only
  construction site (`english_identity_resolver.rs:714-718`) — user-visible
  confidence on confirmation candidates is never computed.
- **ST-06** Dead scaffolding: `MatchResult`/`DuplicateClass` (`matching/types.rs:106-128`),
  `HardcoverMatcher` trait (`livrarr-metadata/src/lib.rs:107-123`),
  `CoverGateOutcome::AskLlm` branch (sole caller hardcodes `llm_enabled=false`),
  `WorkField::normalization_class` (zero callers).
- **ST-07** Goodreads matching is fully deterministic (`is_gr_junk_edition` +
  shared 0.75 picker + explicit abstain, `provider_client.rs:1540-1572,1441-1455`).
  The only LLM on the GR path is `llm_extract_payload` (HTML-parse repair,
  `:1172-1218`). The identity fan-out still excludes GR unless `llm_configured`
  (`english_identity_resolver.rs:240-242`) — a leftover gate.
- **ST-08** Language today: `works.language` is identity-sovereign; the merge
  chokepoint drops OL/HC payloads for foreign works (#133/REQ-027 — UNCHANGED by
  this phase); eager auto-match has a hard language gate; the `"en"` default lives
  in exactly one place (`SeedLanguage::resolve`, `livrarr-domain/src/seed.rs`,
  insight 53); identity clustering `agree()` ignores language entirely; the m4
  Recognition composite has no language term. Provider-language basis — SAMPLED 2026-07-02 (Unit B; closes the r1 R-6 /
  r2 openai R-6 evidence item): Google Books populated `volumeInfo.language` on
  5/5 live ISBN lookups spanning en/de/fr/ja/ko (genuine non-Latin editions,
  ISBN-pinned, via the app's own `fetch_by_isbn` request shape) — HIGH.
  Goodreads declared language via JSON-LD `Book.inLanguage` on 5/5 fetched pages
  (paced ≥1.6s, app UA, zero anti-bot responses) — HIGH for pages that parse,
  with one caveat: the ja/ko samples resolved to English-translation editions
  (autocomplete ranking artifact), so native-edition pages are unsampled. That
  artifact is itself evidence FOR D-lang: GR's top hit for a Korean work was the
  ENGLISH edition — exactly the wrong-language payload the veto must catch.
  Format note (binding on Unit D): GB returns ISO 639-1 codes, GR returns
  English names; `normalize_language` (`livrarr-domain/src/normalization.rs:167`)
  already reconciles both and GR's parser already calls it (`goodreads.rs:334-336`,
  orchestrator-verified) — the authority's language comparison reuses it, never
  reimplements. Net: language-SILENT payloads should be rarer than assumed; the
  declared-mismatch veto carries most of the load.
- **ST-09** Anchor arbitration exists and works (`agree()`/`run_quorum`
  `english_identity_resolver.rs:479-517,311-417`): key equality wins outright; key
  contradiction vetoes; anchored clusters outrank ISBN/ASIN-only clusters; shared
  ISBN + flatly disagreeing titles = collision, must Conflict (AC-020).
- **ST-10** Canonical fuzzy scorer (`livrarr-domain/src/text_norm.rs`): clean_title
  → CJK bigrams / NFKD accent-strip → stopword drop → set-Jaccard. Consumers and
  bars: cover gate 0.6 (at the merge chokepoint), Google Books picker 0.75 +
  author-overlap, shared provider picker 0.75 (Audible/OL/GR).
- **ST-11** Recognition scorer (`livrarr-matching/src/m4_scoring.rs`): rapidfuzz
  Levenshtein max(plain, word-sorted), composite title .45/author .40/year .10/
  series .05, hard gate title<.50 ∥ author<.40 ∥ no-author. Consumers and bars:
  manual-import clustering 0.80, RSS matching (admin 0.50–0.95, default 0.80),
  download-poller grab match 0.6, silent GR author auto-link 0.90. Quirk: both
  strings empty → similarity 1.0 (canonical Jaccard gives 0.0) — and the variant
  folder can force 1.0 for same-series siblings when either position is missing
  (`m4_scoring.rs:111-122`).
- **ST-12** Exact-equality gates: DB identity key `normalize_for_matching`
  (`livrarr-domain/src/lib.rs:884-916`, keeps stopwords+accents) → stored
  `works.normalized_title/author` + `ON CONFLICT DO NOTHING` backstop
  (`sqlite_work.rs:1330-1331`); library dedup cascade `work_dedup.rs:52-105`
  (keys → exact → base-title only when exactly one side has a subtitle;
  deliberately no fuzzy); Hardcover Tier 1 exact title + author-in-list
  (`hardcover.rs:186-230`); Hardcover Tier 2 `llm_disambiguate` (`:232-249`) is
  the one live LLM-chooses-match (sanctioned by P11: LLM advises).
- **ST-13** No user-facing merge-two-works action exists (verified 2026-07-02:
  zero handler fns or routes matching merge/combine/absorb in `livrarr-handlers`;
  two independent probes). The only combine paths are add-time automatic
  (adopt/dedup). D2's "a duplicate is cheap to fix" therefore requires REQ-015.
- **ST-14** All provider HTTP rides the Phase-3 process-global outbound queue
  (pacing, priority, breaker) — a library-wide dry-run identity sweep at Low
  priority is paced and safe by construction (insight 30).
- **ST-15** M10 ("no special cases by language") governs LIFECYCLE/STATES.
  D7 introduces language as a matching SIGNAL and a config default — routing/
  signal, not lifecycle. Compatible; this spec states it to close the tension
  flagged at handoff.

## 4. Requirements

### The two authorities

- **REQ-001 — One Identity authority.** A single named component answers every
  "is X the same book as Y?" question at identity grade. No other code path may
  decide sameness for: provider-payload clustering/merging, add-time
  adopt/absorb, library dedup, already-in-library flags, anchor-graft/cover-borrow
  same-work tests, provider hit-picking, and the DB identity key. The six title
  cleaners and three author canonicalizers collapse to the authority's one recipe
  (plus the Recognition normalizer of REQ-011).
  - AC-001: after cutover, the repo contains exactly one implementation of
    identity-grade title cleaning and one of author canonicalization; the sites in
    the inventory's §A/§B route through them (site-by-site table in §5).
  - AC-002: `normalize_match_title`, `normalize_title_variants`'s colon cut, and
    `normalize_title_for_match`'s `:`/` - ` cut no longer exist (ST-01 all three).

- **REQ-002 — Identity recipe: parse, then compare (D3).** Cleaning baseline:
  lowercase, NFKD accent strip, punctuation ignored, leading articles dropped,
  edition junk stripped, CJK char-bigram comparison. Title parsing: split at the
  first colon; split at a dash ONLY in unambiguous separator form (spaced " - "
  or em-dash) (confer fold-in a). Classify the tail deterministically (r1 R-1):
  - **series-volume marker** — the tail (or a parenthesized suffix) carries a
    volume token with a number: `Book|Vol(ume)|Part|No.|#` + digits or spelled
    ordinals (one–twenty), a bare trailing integer, or the `(Series Name, #N)`
    form. Extracted as a signal (REQ-003), never compared as text.
  - **junk** — a closed, spec-carried vocabulary matched whole: "a novel",
    "unabridged", "abridged", "large print", "annotated", "illustrated",
    "complete", "tie-in", "special|deluxe|collector's|anniversary|expanded
    edition" + the existing edition-junk list. Ignored for identity;
    "(Unabridged)"-class tokens remain available as edition info to release
    selection (confer fold-in d). Extending the list requires a matching
    trap-corpus test (REQ-017).
  - **true subtitle** — any non-empty remainder after junk and series-marker
    extraction.
  - A tail that cannot be confidently classified is treated as a TRUE SUBTITLE —
    the safest class: it can only demote to grey, never veto, never silently pass.
  - AC-003: "History of Rome: Volume 1" vs "History of Rome: Volume 2" → NOT the
    same book (hard veto), even with identical authors and no series metadata.
  - AC-004: "Storm Front" vs "Storm Front: The Dresden Files, Book 1" → grey
    ("likely, not certain"), never silent auto-merge on text alone; an agreeing
    ID (ISBN/anchor) may still confirm it.
  - AC-005: A hyphenated real title ("Catch-22") is never split at its hyphen.
  - AC-017: "Foo: Book Three" and "Foo, Vol. 3" classify as series markers;
    "Foo: A Novel" as junk; "Foo: And Other Stories" as a true subtitle.
  - AC-018: an unclassifiable tail never vetoes and never auto-passes — it can
    only demote the pair to grey.

- **REQ-003 — Tail semantics (D3).** The tail never makes a match. Conflicting
  volume/series numbers (from parsed tails OR series metadata) = hard stop.
  Substantive tail disagreement, or a tail on exactly one side = grey. Junk-only
  differences = ignored.
  - AC-006: both-sides-tails flatly disagreeing ("Mistborn: The Final Empire" vs
    "Mistborn: The Well of Ascension") → never auto-same.

- **REQ-004 — Identity bars (D4).** Auto-same requires EXACT equality of cleaned
  main titles (plus REQ-005 author agreement, minus any REQ-003/REQ-008 veto).
  Similarity ≈0.75+ = grey candidate, surfaced ranked by real score. Below = no.
  - AC-007: no code path auto-merges or auto-absorbs on a fuzzy title score alone;
    the 0.75-auto behavior at the identity seat is gone.

- **REQ-005 — Author rule (D9).** Cleaned full author names must match
  (order-normalized: "Smith, John" = "John Smith"; initials normalized). Decidable
  agreement rules (r1 R-002): (a) ≥1 full-name match (initials compatible with
  the expanded name on a matching surname) → authors AGREE; unmatched EXTRA
  credited names on either side (translators, illustrators, one-sided co-authors)
  are non-evidence and never subtract. (b) Zero full-name matches but a shared
  surname token → NOT agreement, grey at best (the John/Jane Smith zone — IDs or
  the user decide). (c) Zero overlap of any kind → disagreement (blocks
  auto-same; grey only when IDs carry the pair). Ambiguous initials (compatible
  with two different candidate names) → grey. A payload with no author ABSTAINS
  (agreement then requires exact full-title equality — current semantics kept).
  Any-shared-token matching is removed.
  - AC-008: "John Smith" vs "Jane Smith" → NOT an author match (kills ST-02).
  - AC-019: ["Jim Butcher"] vs ["Jim Butcher", "James Marsters (narrator)"] →
    AGREE; ["Rowling, J. K."] vs ["J.K. Rowling"] → AGREE; ["John Smith"] vs
    ["Jane Smith"] → grey at best, never auto.

- **REQ-006 — IDs outrank text; arbitration carried forward (D1, confer c).**
  The ST-09 rules move into the authority explicitly, with the two ID levels
  stated (r1 R-2): **WORK-level keys** (OL/GR/HC work keys) identify the work —
  equality wins; contradiction (same provider, different keys) vetoes; and a
  work-key contradiction OUTRANKS any edition-ID agreement (shared ISBN + work-key
  contradiction = the collision shape → Conflict, never auto-same).
  **EDITION-level IDs** (ISBN/ASIN) identify editions — equality is a positive
  bridge (subject to title non-contradiction, ST-09); INEQUALITY IS NO EVIDENCE
  (two editions of one work legitimately carry different ISBNs/ASINs) and never
  vetoes. Anchored clusters outrank ISBN/ASIN-only clusters; shared ISBN with
  contradicting main titles = collision → Conflict. An unverifiable key never
  rides in (existing verify-gate semantics kept).
  - AC-009: the Sprint-B collision case and the Dresden trap case still pass.
  - AC-021: ISBN equal + same-provider work keys different → Conflict surfaced,
    no auto-merge. ASINs differing while all else agrees → zero penalty.

- **REQ-007 — Language in identity (D7).** A payload whose declared language
  differs from `works.language` can never merge or absorb (hard veto), regardless
  of title similarity. Language-silent payloads: eligible for auto-apply only
  when the work's language equals the user's default language (REQ-013);
  otherwise grey. The #133 OL/HC foreign drop at the merge chokepoint is
  unchanged and remains the outer guard. (M10 reconciliation: ST-15.)
  - AC-010: a French-declared payload never merges onto an English work even with
    identical main titles.
  - AC-011: a language-silent payload on a work outside the user's default
    language lands grey, never auto-applies.

### Grey-zone behavior (D2)

- **REQ-008 — Never act on grey at the high-stakes seats.** Provider-data merges
  and absorb-into-existing: grey = no action. Interactive paths surface the ranked
  candidates for a one-click choice. Background paths park the work in a visible
  needs-review state and move on (M9 semantics: surfaced, never silent limbo).
  - AC-012: no background path can write provider data onto a work whose identity
    match was grey.
  - AC-013: a parked work is visible in a review list with its candidates and
    real scores; resolving applies the choice and un-parks.

- **REQ-009 — Covers keep guessing (D5).** Cover bar stays 0.6; accepted-by-guess
  covers carry the unverified flag; user override locks (CoverTrust semantics
  unchanged).

- **REQ-010 — Real scores (rider, ST-05).** Every user-facing candidate carries a
  genuinely computed similarity score; the hardcoded 1.0 dies.

### Recognition matcher

- **REQ-011 — Recognition keeps its seat, loses the traps (D1).** The m4 scorer
  remains the matcher for torrent/file/RSS text with its existing consumers and
  bars (ST-11 — no threshold changes this phase). Fixes: the variant folder's
  colon cut is replaced by the REQ-002 parse with the volume guard applying
  whenever positions are UNKNOWN (missing position ≠ safe); a release declaring a
  different language never matches a work for download (D7 recognition
  corollary). **Language-silent releases (r1 R-001/R-5):** a release declaring no
  language auto-matches only works whose language equals the user's default; for
  any other work it is never grabbed silently — background paths (RSS) skip it
  and surface it as needing confirmation; interactive search is unaffected (the
  user sees and picks). Rationale, derived from D2: release-naming convention
  leaves English releases unmarked, so a silent release against a
  non-default-language work is wrong more often than right, and a wrong grab
  imports by grab-hash with no further language check. (The symmetric leak for
  non-English-default users mirrors D7's accepted caveat.) **No-evidence
  comparisons (r1 R-7):** an empty title or author on either side contributes no
  positive evidence and can never satisfy any bar or gate — both-empty is not a
  match; representation (None/sentinel) is a Stage-2 choice.
  - AC-014: two same-series releases with one missing position no longer force
    title similarity 1.0.
  - AC-022: a language-silent release never auto-grabs for a work outside the
    user's default language; it surfaces for confirmation instead.
  - AC-023: two records with empty titles never match on title; the pair can
    proceed only on IDs.

### Language plumbing

- **REQ-012 — GR unlock (D6).** The `llm_configured` exclusion of Goodreads from
  identity provider selection is removed. LLM-repaired payload fields
  (`llm_extract_payload`) receive zero extra trust: same deterministic matching,
  vetoes, and bars as any payload; repair can never raise confidence (confer b).

- **REQ-013 — Default-language setting (D7; wording amended at Unit-H commit,
  PO-accepted 2026-07-03).** A default language, surfaced in settings, replaces
  the hardcoded `"en"` at the single seed chokepoint (`SeedLanguage::resolve`).
  Delivered PER-INSTALL (a `metadata_config` singleton column), not per-user:
  every existing Livrarr preference is a per-install singleton and no per-user
  prefs mechanism exists; if a per-user preferences layer ever lands, this
  setting migrates with it. Existing installs default to `"en"`
  (behavior-preserving). "User's default language" elsewhere in this spec means
  this setting.
  - AC-015: with default `de`, a language-unknown file import seeds `de`, not `en`.

### Repairs & cleanup (riders)

- **REQ-014 — Adopt-path fix (ST-04).** The add-time adopt lookup and the stored
  identity key derive from the SAME recipe on both sides. Stored
  `normalized_title/author` values are recomputed if the recipe output differs.
- **REQ-015 — Merge-duplicates action (ST-13, confer e; expanded per r1 R-3/R-4/
  R-003).** A user can combine two works from the UI (the duplicate produced by
  conservative non-absorption must be one click to resolve — this makes D2's
  premise true). Guarantees:
  - (a) **User-scoped:** both works must belong to the acting user (P4); no
    cross-user merges, ever.
  - (b) **Preview first:** the merge shows what moves, what would be overwritten,
    and what would be discarded, before anything happens.
  - (c) **Files:** library items and grabs reassign to the survivor; files
    re-organize under the survivor per the standard naming/layout rules; ZERO
    file deletions. (Amended at Unit-I commit, PO-accepted 2026-07-03: no
    filename auto-disambiguation machinery exists — the original wording
    wrongly assumed one. A name collision leaves the item at its existing
    path with a warning; the database stays consistent and nothing is
    deleted. Automatic rename is a possible follow-up.)
  - (d) **User-sovereign fields:** additive where safe (monitoring flags OR);
    genuinely conflicting single-valued user fields require an explicit choice in
    the preview (default: the survivor keeps its value; the loser's value is
    shown, never silently discarded). Per-user consumption data (progress,
    bookmarks) rides its library items and is preserved. (Amended at Unit-I
    commit, PO-accepted 2026-07-03: the survivor's title/author persist by
    design — the loser's user-set identity fields are disclosed in the
    preview but are not choosable.)
  - (e) **User-initiated only;** the loser is removed only after the survivor
    owns its items.
  - AC-024: merging two works never deletes a file and never crosses users.
  - AC-025: a merge where both works carry a user-set value for the same field
    requires an explicit user choice for that field (no silent overwrite).
- **REQ-016 — Dead scaffolding deleted (ST-06) + Hardcover Tier-2 (D10).** All
  four ST-06 items, plus `llm_disambiguate` and the Tier-2 branch
  (`hardcover.rs:232-249,443-557`) — Hardcover near-misses ride the standard
  grey-candidate flow like every other provider.
  - AC-016: after cutover, no code path submits provider candidates to an LLM for
    selection (repo-wide).

### Rollout (D8)

- **REQ-017 — Trap corpus first.** Before any rewiring ships, these cases exist
  as tests and pass on the new authority: all three colon sites' traps (ST-01);
  sibling volumes with and without series metadata; position-missing siblings
  (ST-11 quirk); study-guide lookalikes; one-sided subtitle; same-title-cross-
  language ("Dune" en/fr); author share-a-word ("John/Jane Smith"); ISBN-collision
  (Sprint-B heritage rule "AC-020"); Dresden case; both-empty
  strings; language-silent release against a non-default-language work (REQ-011).
  (AC numbering note: this spec's own ACs skip AC-020 to avoid colliding with the
  Sprint-B heritage label above.)
- **REQ-018 — Measured cutover.** DB snapshot → zero-write old-vs-new decision
  diff over the real library (library-internal decisions for all works; provider-
  candidate decisions for every work with non-Confirmed identity or anchor gaps,
  rides the queue at Low, ST-14) → the PO reviews EVERY changed decision → cutover
  on PO approval only. The diff's grey-zone accuracy numbers are recorded as the
  baseline for any future bar-loosening (D2).

## 5. Site routing table (all 11 inventory sites → new home)

| # | Site (inventory) | Routes to | Policy seat |
|---|---|---|---|
| 1 | canonical text_norm + consumers | Identity recipe (cover gate keeps 0.6 bar; provider pickers per REQ-004 abstain-on-grey) | covers D5; picking D2 |
| 2 | identity engine private family | REPLACED by Identity authority | merges D2/D4/D7 |
| 3 | m4 recognition scorer | Recognition matcher (kept; REQ-011 fixes) | existing bars |
| 4 | variant folder | absorbed into REQ-002 parse (colon cut dies) | — |
| 5 | DB identity key + ON CONFLICT backstop | Identity recipe output (REQ-014) | backstop |
| 6 | work_dedup cascade + eager auto-match | Identity authority absorb seat | absorb: never on grey |
| 7 | strict already-in-library key (colon cut) | Identity recipe (cut dies) | flags + graft/borrow |
| 8 | Hardcover Tier 1 exact | Identity recipe exact mode | — |
| 9 | anchor arbitration | carried into authority (REQ-006) | — |
| 10 | Hardcover Tier 2 LLM pick | DELETED (D10, REQ-016) — near-misses become standard grey candidates | grey → human (D2) |
| 11 | GR deterministic + llm gate | gate removed (REQ-012) | — |

## 6. Resolved during walkthrough

- **Hardcover Tier-2 LLM pick → DELETED (D10).** The PO's consistency challenge
  ("why here and nowhere else?") settled it: post-unification, HC gets the same
  deterministic grey-candidate middle tier as every provider, so the LLM chooser
  is a redundant single-provider exception whose background auto-settling would
  contradict D2. **Pending follow-up (PO-owned, at deploy):** principle P11's
  parenthetical example names Hardcover disambiguation — the principle's spirit
  (deterministic first; LLM advises; fully functional without one) is untouched,
  but that example sentence needs the PO's one-line amendment.

## 7. Out of scope (this phase)

Recognition threshold recalibration (RSS/poller/reconcile bars stay);
release-quality ranking; suppression machinery (separate pending PO call);
GR-breaker test-isolation flake (insight 58); display-title policy (what title to
SHOW when sources disagree — flagged during the interview as a later product
question); #133 foreign OL/HC drop (stays exactly as is); any LLM-based matching
(PO-excluded).

## 8. Non-goals / guardrails restated

No new lifecycle states by language (M10, ST-15). No env-var config. The authority
must be consumable without violating the seams in `docs/canonical-model.yaml`
(placement is a Stage-2/design call, not decided here). Behavior outside the 11
sites is unchanged. NO push of the branch (PO standing order).
