# Design: provider-picker conformance — one authority-based picker, kill the loose 0.75 bar

**Unit:** matching-conformance (quality-waves), follow-up to the identity-fix / settle-road
unit (`25f1c811`). Charter = the settle-road design's explicit out-of-scope list
(`design-settle-road-matching.md` §"Out of scope") + the 2026-07-14 conformance audit
(session-log-quality-waves.md 03:05 entry, memory `project_matching_conformance_gaps`).
**Status:** IMPLEMENTED 2026-07-14. Design gate closed (both-family PASS r10). Red pins →
sonnet implementation → workspace 1566/0/299 → code review r1 (gemini PASS; codex R-1, a
pre-existing OL compacted-index wrong-key bug, FOLDED via the HC/GR `kept` pattern) → r2
both-family PASS → committed. Delivered as specified: `pick_best_candidate` (Same-only for
Audible/OL/HC/GB, `accept_grey=true` for GR), V4 colon-cut removed, stale SFC canary
removed. Deferred on record: Audible/GB fetch-level pins (clients not injectable — covered
by the 14 domain-unit pins + review); the R-6 GR-grey-cache-merge residual (follow-up unit).
**Governing rules:** `docs/metadata-remediation-phase5-matching-spec.md` (PO-locked
2026-07-02): D1 (two brains; all other matchers collapse into them), D4 (auto-same = EXACT
cleaned-main equality; ≈0.75 = grey; IDs outrank text), D9 (author = full-name match;
any-shared-token dies), REQ-001 (ONE identity authority answers every sameness question
incl. **provider hit-picking**; AC-001 exactly one implementation), REQ-004/AC-007
(0.75-auto behavior at the identity seat is gone), REQ-002 (one closed spec-carried junk
vocabulary). Precedent: `gr_best_match` (`provider_client.rs:1824-1880`) — the GR hit-picker
already routed through the authority in N4 (insight 59); this unit finishes the same job for
the other providers.

---

## 0. The central question, adjudicated: ratified or violation?

The two audit agents split on whether the shared 0.75 whole-string pickers are **blessed**
(ST-10 names a "canonical fuzzy scorer" with these exact consumers/bars) or **violations**
(D1 says all matchers collapse into two; the pickers are a third). Adjudicated against the
verbatim spec:

**ST-10 is a System Truth, not a Decision.** §3 is titled "System Truths (verified
environment/code facts)" and every citation is "verified at `ab99693`" — the PRE-Phase-5
baseline. ST-10 sits beside ST-01 (the three colon-truncators) and ST-02 (the any-shared-token
author bug): those are inventoried as **defects to fix**, not blessings. ST-10 inventories the
"shared provider picker 0.75 (Audible/OL/GR)" and "Google Books picker 0.75 + author-overlap"
the same way — a description of what exists, feeding the requirements, not a ratification.

**The Decisions and Requirements mandate conformance:**
- **REQ-001** names "**provider hit-picking**" explicitly among the sites that "No other code
  path may decide sameness for" — they must route through the one authority. AC-001: "after
  cutover, the repo contains exactly one implementation of identity-grade title cleaning and
  one of author canonicalization."
- **§5 site-routing table, row 1** ("canonical text_norm + consumers") routes to "Identity
  recipe" with the note: "**provider pickers per REQ-004 abstain-on-grey**."
- **REQ-004 / AC-007**: "no code path auto-merges or auto-absorbs on a fuzzy title score
  alone; **the 0.75-auto behavior at the identity seat is gone**." The pickers accept at
  0.75 whole-string jaccard — that is the behavior AC-007 says must be gone.
- **D9 / AC-008**: "any-shared-token matching dies." `score_provider_candidates` accepts on
  `author_overlap >= 1` (any shared token) — ST-02's named defect, verbatim alive.

**Disconfirming branch traced and absent:** no Decision (D1–D10), no §6 walkthrough
resolution, and no §7 out-of-scope item carves out the non-GR provider pickers as keeping the
0.75 bar. §7 keeps "Recognition threshold recalibration (RSS/poller/reconcile bars stay)" —
that is the m4 **Recognition** side (ST-11), a different scorer from the identity-side pickers
(ST-10). Nothing preserves the ST-10 pickers.

**Verdict: VIOLATIONS.** Conforming them EXECUTES the locked spec (§5 row 1 + REQ-001 +
REQ-004); it does not amend a locked reading, so by the handoff's own conditional it is not a
PO amendment call — it is the deferred Phase-5 delivery for these sites. **Confidence ~85%.**
What would flip it: a governing-text carve-out I missed, or a reviewer showing "provider
hit-picking" in REQ-001 refers only to the GR picker already done. The unprimed both-family
design review is the external check on this reading.

**Honest residual tension (surfaced, not hidden):** insight 13 (2026-07-03, Phase-5 close)
describes "the same shared 0.75 picker as every provider [that] abstains below the bar" as if
it were the intended mechanism. Read against the spec, that reflects Phase 5 conforming
`gr_best_match` (N4) but never finishing the shared `score_provider_candidates` — incomplete
delivery of AC-001/AC-007, not a deliberate exception. The wiki insight will be corrected on
cutover.

---

## 1. Problems (verified at source, HEAD `25f1c811`)

**V2 — the shared fuzzy picker `score_provider_candidates`** (`audible.rs:347-378`): whole-string
set-Jaccard `>= min_title_jaccard` used as an ACCEPT bar, plus author matched by
`author_overlap >= min_author_overlap` (token intersection count). Every call site passes
`(0.75, 1)`. Consumers (Serena references, unbounded):
- `AudibleCatalogClient::fetch` — TWO sites: the ASIN-lookup title-verify arm (`audible.rs:148`)
  and the title+author search arm (`audible.rs:203`).
- `OpenLibraryClient::title_author_search` (`provider_client.rs:1038`) — OL tier-3, fires when
  ISBN + ol_key lookups both return a genuine no-match.
- `query_hardcover` tier-2 (`hardcover.rs:254`) — fires when the tier-1 exact match misses. Its
  own doc comment claims it "rides the standard grey-candidate flow"; the code returns the pick
  directly.

**V3 — the Google Books copy `score_candidates`** (`google_books.rs:464-492`): same recipe, own
consts `MIN_TITLE_JACCARD = 0.75` / `MIN_AUTHOR_OVERLAP = 1` (`google_books.rs:14-15`). Consumed
by `GoogleBooksClient::fetch`'s no-ISBN text-search arm (`google_books.rs:437`). GB is the
designated foreign-language provider — a wrong pick lands on the population the June F1 incident
damaged.

The any-shared-token author rule (`author_overlap >= 1`) inside BOTH V2 and V3 is ST-02's killed
defect. This part is unambiguous regardless of the §0 adjudication and must die: `author_verdict`
is the authority (D9).

## 2. Reachability — which roads fire these pickers (recon-verified, unbounded)

The four picker call sites are reached ONLY via `ProviderClient::fetch` (the enum-dispatch
method), whose production callers are exactly four:

| Caller | Road | Picker fires? |
|---|---|---|
| `LiveEnglishIdentityResolver::resolve` (`english_identity_resolver.rs:108`) | **identity fan-out** | YES — the responder legs |
| `LiveCoverService::resolve_provider_url` (`cover_service.rs:297`) | covers | YES |
| `fetch_internal_alternatives` (`cover_alternatives.rs:83`) | covers | YES |
| `LivePreaddCoverService::fetch_cover_alternatives` (`preadd_cover_service.rs:48`) | covers | YES |

**They do NOT fire on the enrichment scatter.** `DefaultProviderQueue::dispatch_enrichment`
(`provider_queue.rs:337`) is the only caller of `fetch_by_anchor`; every arm was read — Audible
`fetch_by_asin`, GB `fetch_by_isbn`, OL `fetch_by_anchor_query`, HC `fetch_by_anchor_query` —
none falls through to `fetch(&Work)` or a text search (anchor-grounded since Sprint B, insight
51). **They do NOT fire on discovery/lookup** — `discovery_service` calls the raw search fns
directly and returns every hit for the user to pick (recon Q1).

**Blast radius (why it still matters despite the narrow roads):** a payload the picker chooses on
the identity fan-out is (a) re-judged by the quorum's `agree`/`run_quorum` (which uses the
authority) before it can corroborate an anchor, AND (b) cached (D-005: `resolve` `cache_put`
`english_identity_resolver.rs:179`) and later merged field-by-field via
`EnrichmentServiceImpl::enrich_work`'s candidate-reuse (`lib.rs:490` cache_take → `:521`
`merge_from_cached` → `:530` `apply_enrichment_merge`) with NO fresh fetch, gated by
identity-status + `cached_payloads_match_work` anchor overlap + the language drop. A wrong pick
that shares an anchor and clears those gates writes its fields onto the work. Defense-in-depth is
the Phase-5 doctrine — the picker is the first line and today it is loose.

**Upstream doors [REV — gemini R-9].** The four picker sites sit behind ONE chokepoint per road,
so the Same-only bar lands at a single point. Fan-out road: every door funnels through
`LiveEnglishIdentityResolver::resolve` — `settle_identity`'s six settle doors (all add doors via
`ensure_identity_and_enrichment`, async `complete_add`, single+bulk `refresh`, pre-scatter
`run_unified_enrichment`, background `converge_work`, `retry_all_incomplete` — the settle-road
design's verified door table), plus `WorkService::resolve_identity` (manual-import + list import)
and discovery `lookup_filtered`. The **background poller / manual-refresh metadata paths reach
providers via the enrichment scatter (`fetch_by_anchor`), NOT `resolve`/`fetch`** — so they do not
touch the pickers at all (anchor-grounded, verified below). No skip-gate or ad-hoc spawn reaches a
picker off-chokepoint.

## 3. Change 1 — ONE shared authority-based picker (finishes REQ-001/AC-001)

Introduce a single picker in the authority, `identity_matching::pick_best_candidate` (name TBD
at pins), and route all four V2/V3 sites AND `gr_best_match` through it — one shared consumer of
the authority's verdicts (REQ-001/AC-001; "one implementation" = one title-cleaning/verdict
recipe = `parse_title`/`title_verdict`/`author_verdict`, which BOTH GR and the others call). The
only per-seat policy knob is `accept_grey`:

```
pick_best_candidate(seed_title, seed_author, candidates: &[(title, author)], accept_grey: bool)
  -> Option<usize>
  for each candidate:
     t = title_verdict(parse_title(seed_title), parse_title(cand_title))
     a = author_verdict([seed_author], [cand_author])
     accept + tier per the rule table below; skip otherwise
  return best by (tier, grey score, then earliest hit)   // gr_best_match ranking, unchanged
```

**Accept rule table [REV — both families r-round: Same-only for the merge-eligible path]:**

| title_verdict | author_verdict | Outcome |
|---|---|---|
| `Same` | `Agree` \| `Abstain` | accept, tier 2 |
| `Grey` (any cause) | `Agree` | accept, tier 1 — **only when `accept_grey`** |
| `Same` | `Disagree` \| `Grey` | reject |
| `Grey` | `Abstain` \| `Grey` \| `Disagree` | reject |
| `Grey` | `Agree`, `accept_grey == false` | reject |
| `Different` \| `VetoVolume` | any | reject |

**Per-seat `accept_grey` (the corrected load-bearing decision):**
- **The four newly-conformed pickers — Audible, OL, HC, GB — pass `accept_grey = false`
  (Same-only).** [REV — codex R-6 + gemini R-6, both P1, both verified] §5 row 1 ("provider
  pickers per REQ-004 abstain-on-grey") + REQ-008/AC-012 (no background path writes provider data
  onto a grey-matched work) bind here because a picked payload is not just a display candidate —
  it is cached by the identity fan-out (`resolve` cache_put `english_identity_resolver.rs:179`)
  and later field-merged with no fresh fetch via `enrich_work` (`cache_take lib.rs:490` →
  `merge_from_cached :521` → `apply_enrichment_merge :530`), gated ONLY by anchor overlap
  (`cached_payloads_match_work lib.rs:249-278` has no title/grey check). These four providers have
  NO per-provider grey-corroboration gate, so Same-only is their correct and complete bar. My
  original draft's "Same-or-Grey, seats abstain" was wrong: the cache-merge seat does not abstain
  on grey.
- **`gr_best_match` passes `accept_grey = true`** — it MUST keep returning grey subtitled-from-bare
  records, because that is the input the ratified `verify_gr_payload` / AC-004 grey-corroboration
  hatch consumes (the shipped settle-road unit, insight 72). Verified: a blanket Same-only picker
  would break the pinned `picker_matches_subtitled_record_from_bare_seed` (`provider_client.rs:1920`,
  asserts `Some(0)`) and regress WWZ (`GoodreadsClient::resolve_detail_url :1675` is the caller).
  GR's grey handling is gated downstream by `verify_gr_payload` (grey + agreeing ID → adopt; else
  park), which the other four providers do not have.
- **[REV — codex R-7 + gemini R-7]** The GR call site pre-filters `is_gr_junk_edition`
  (`provider_client.rs:1809`) BEFORE building the candidate pairs (as `gr_best_match` does today),
  so the study-guide/summary trap-corpus rejection (REQ-017) survives the fold. A guard test pins
  that a GR study-guide hit cannot be picked post-fold.

`score_provider_candidates`, `score_candidates`, and `gr_best_match`'s inline scorer are DELETED;
their five call sites call `pick_best_candidate` (four with `accept_grey=false`, GR with `true`).
GB's `MIN_TITLE_JACCARD` / `MIN_AUTHOR_OVERLAP` consts are removed; the convergent `0.75` literals
collapse into the authority's `TITLE_GREY_FLOOR` (`identity_matching.rs:25`), the single source.
**Crate boundary [REV — gemini R-8]:** `pick_best_candidate` is `pub` in `livrarr-domain`;
`livrarr-external-data` already depends on `livrarr-domain` and `gr_best_match` already imports
`livrarr_domain::identity_matching::{parse_title, title_verdict}` — the call direction is
established and legal per `canonical-model.yaml`. No new edge.

### R-6 residual, explicitly scoped OUT (PO thread)

A grey GR pick that `verify_gr_payload` legitimately consumes is still cached, so its FIELDS can
later field-merge on anchor overlap alone — the same REQ-008 shape, for GR only, PRE-EXISTING
(gr_best_match already returns grey and is already cached today; this unit does not introduce it).
Fully closing it means gating the cache-merge on the identity RELATIONSHIP, not the anchor — a
seat-level fix on `cached_payloads_match_work` / `merge_from_cached`, settle-road-adjacent, and it
hinges on REQ-008's scope (does "grey match" mean the title verdict at pick time, or the final
anchor-corroborated relationship?). **Recommend: follow-up unit** (smallest real step; this unit
closes R-6 for its four actual targets). Surfaced for the PO to confirm defer-vs-close-now.

## 4. Change 2 — the strays (V4-V8) + junk-vocabulary duplication (per-item disposition)

| ID | Site | Disposition |
|---|---|---|
| V4 | `manual_import.rs:628-638` conditional colon-cut on the eager-match title comparand | **FIX** — the cut mangles the comparand before `best_candidate_index_lang` → `identity_absorb_match` (the authority's absorb verdict). Feed the raw parsed title; the authority already handles subtitles. |
| V6 | `cover_resolution.rs:44-59` `should_reject_cover` REJECT_SUBSTRINGS (study-guide list) | **FOLD to REQ-002** — one of ≥3 independent copies of the study-guide junk family (also `is_gr_junk_edition` `provider_client.rs:1809`, author-monitor `SUMMARY_KEYWORDS`). Consolidate into one spec-carried vocabulary. |
| V7 | `cover.rs:410-474` `fast_hc_cover_search` exact-lowercase title+author equality | **LEAVE, documented** — exact-equality is stricter than the authority's `Same` (no fuzzy), so it under-matches (misses), never mis-matches. Low risk; note it as an intentional fast-path exact check, not a violation of "no loose matching." |
| V5 | `list_service.rs:616-624` `normalize_for_dedup` alnum-lowercase concat | **LEAVE, documented** — a list-import dedup KEY, not a sameness verdict; structural, no fuzzy accept. Same class as V8. |
| V8 | `discovery_service.rs:1036-1060` `dedupe_lookup_results` | **LEAVE, documented** — cosmetic display dedup of discovery results; exact-string, no fuzzy. |

**Junk-vocabulary duplication (REQ-002, new recon finding).** `title_cleanup.rs`
`RE_FORMAT_PAREN` / `RE_EDITION_PAREN` / `RE_COLON_NOVEL_MARKER` (`:36-68`) encode ~26 of
`JUNK_VOCAB`'s 29 phrases as a SECOND regex-based vocabulary, feeding `text_norm::title_tokens`
(every picker's tokenizer). `identity_matching::classify_paren` already composes the two
deliberately, so this is two-vocabularies-for-one-concept, not a silent bug. REQ-002 wants ONE
closed spec-carried vocabulary. **Recommend: assess-and-consolidate as a scoped sub-item within
this unit if cheap; otherwise split to its own unit** — flagged for the review/PO to size, not
force-fit here.

## 5. Behavior changes to expect (honest ledger)

- **Author matching tightens everywhere the pickers run.** "John Smith" vs "Jane Smith" (shared
  surname → `author_verdict::Grey`) and any single-shared-token pair stop matching. Any provider
  test pinning the old overlap behavior is pinning ST-02's defect — each flip enumerated and
  dispositioned at implementation review, never silently rewritten.
- **Title matching tightens from whole-string to main-title parse.** "Dune Messiah" no longer
  matches a "Dune" seed via whole-string jaccard (it was already borderline; the authority makes
  it `Different`/`Grey` deterministically). Sibling volumes get the veto (`VetoVolume`).
- **GB foreign picks** now demand `author_verdict::Agree` (or Same+Abstain) — should REDUCE
  wrong-language / wrong-book picks on the F1 population.
- **No road that fired before stops firing** — same four call sites, same providers eligible;
  only the accept criterion changes from loose to authority-grade.

## 6. Tests

- Unit (`identity_matching`): `pick_best_candidate` truth table for BOTH `accept_grey` values —
  `accept_grey=false`: Same+Agree, Same+Abstain accept; Grey+Agree REJECT (the R-6 fix); Different/
  Veto reject. `accept_grey=true`: Grey+Agree accept, tier-1 ranked. Both: any-shared-token author
  ("John/Jane Smith") rejects; "Dune"/"Dune Messiah" rejects (Different). Ranking (Same beats Grey,
  higher grey score, earliest ties).
- In-crate (livrarr-external-data, recording-fetcher precedent): each of the five call sites picks
  the right hit / abstains. The four non-GR sites REJECT a subtitled-from-bare (grey) hit; the GR
  site ACCEPTS it (`accept_grey=true`). **`gr_best_match`'s existing pins stay green** —
  `picker_matches_subtitled_record_from_bare_seed` (grey `Some(0)`), `picker_junk_filter_still_applies`,
  and the sequel/veto guards all hold after the refactor onto the shared fn.
- R-7 guard: a GR study-guide/summary hit is pre-filtered and can never be picked post-fold.
- V4: manual-import eager match no longer mangles a subtitled comparand.
- Guard: the trap-corpus cases (REQ-017) and the Phase-5 frozen decision-diff harness
  (`test_p5_matching_diff.rs`, insight 61) stay stable.
- Live: snapshot first; a foreign-work refresh through the GB picker.

## 7. Out of scope (this unit)

- Recognition-side bars (ST-11 / m4 / RSS / poller) — §7 keeps them.
- Cover acceptance bar (0.6, D5) — ratified; V6 touches only the junk-list vocabulary, not the bar.
- The `id_verdict` mixed-evidence question at the quorum/dedup seats (flagged from settle-road
  r6; reopening the equality-first collapse there needs its own cross-family review, insight 59).
- Review-surface plumbing for uncorroborated grey GR keys (deferred from settle-road pending live
  WWZ evidence).
- **The R-6 GR-grey-field-merge residual** (§3): grey GR payloads legitimately returned for
  `verify_gr_payload` still field-merge on anchor overlap via the reuse cache — a pre-existing
  REQ-008 gap closed by a cache-seat gate, settle-road-adjacent. Follow-up (PO to confirm
  defer-vs-close-now).
