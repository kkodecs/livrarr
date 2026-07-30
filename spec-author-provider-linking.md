---
feature: "author-provider-linking"
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009]
---

# Spec: author-provider-linking (F1 of the two-feature author/identity commitment)

GitHub issue: #176. Sequencing of record (PO 2026-07-28, do not re-raise): ONE routes-model
design covering works and authors; TWO features back-to-back. **F1 (this spec): the
author-record half — additive only.** F2 (committed, starts behind F1): the identity-layer
rewrite with the works-side author changes (work→author pointer, uniqueness on author id,
settle-road inheritance) riding it. Fuse tripwire: two blocked-on-F2-decision events during
F1 design → the features merge. Full record: kk-build session log 2026-07-28 03:20 UTC;
discussion record: `build/spec-discussion-notes-176.md` (+ §8 resolution).

## 0a. Design Principles

- **Routes, not a single key.** The author record is the anchor; provider identifiers
  (OL/GR/HC) are routes to that person — plural, optional, zero is valid. Same model F2
  applies to works; designed once.
- **One standard of proof for every route write.** The disease being cured is
  per-consumer standards (first-hit here, 0.90 there, nothing elsewhere). No writer
  bypasses the guard.
- **The author road consumes work identity; it never manufactures it.** No second road
  to work identity, no re-running work matches the works pipeline already decided
  (PO-caught during spec; the works fan-out already runs OL-by-ISBN as its Tier 1).
- **Never guess silently; surface the unsure** (D8). A bare name match never auto-links.
- **Additive only.** No `works` table changes, no settle-road changes, no migration of
  existing identity data. Those are F2.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | PM-read 2026-07-28: `crates/livrarr-db/src/sqlite_author.rs:585-594` (`list_monitored_authors`: `monitored = 1 AND ol_key IS NOT NULL`); gate `crates/livrarr-metadata/src/author_service.rs:222-228`; UI toast `frontend/src/pages/authors/AuthorsPage.tsx:59-73` | Author monitoring consumes ONLY the OL route: the monitor's feed is OL's per-author works list; enabling monitor without ol_key is rejected server-side | Any F1 design where a GR/HC route unlocks monitoring; frontend-only gating claims | high |
| ST-002 | PM-read: `author_service.rs:286-343` (`lookup` hits `openlibrary.org/search/authors.json`, shipped, serves interactive add-author) | Name→OL-author-candidate resolution exists in production (returns key+name docs through the paced OL bucket) | Treating author name search as new/untested plumbing | high |
| ST-003 | PO observation 2026-07-28 ("OL is really really slow") + outbound-queue pacing (OL bucket 1s, in-flight cap 2, wiki insight 30) + `wiki/integrations/openlibrary.md:23-26` (no API key; UA+contact is the identification mechanism; 1 rps unidentified / 3 rps identified) + standing PO pause on NEW OL UA identifiers | Full-library sweeps over OL are hours-scale slow-drip, not minutes | Sweep designs assuming fast OL; any user flow synchronously waiting on an OL call; introducing new OL UA strings | high |
| ST-004 | SAMPLED 2026-07-28. OL: live fetch of work `OL24217656W` → `authors: [{author:{key:"/authors/OL832299A"}}, …]` (3 author keys on this work). GR: real captured page `crates/livrarr-external-data/fixtures/gr-book-23692271.html` → the legacy `authorName` anchor is ABSENT on the current layout (0 matches; **corrects discussion notes §1.6**); the author ID is in the `__NEXT_DATA__` Apollo state — Contributor entity with `legacyId: 395812` + `webUrl …/author/show/395812.Yuval_Noah_Harari` — the same blob `parse_detail_next_data` already parses for book fields. NB `NormalizedWorkDetail` carries NO author-ID field today (`crates/livrarr-external-data/src/types.rs:6-36`) — capture is new work wherever the road consumes it | OL work records carry author keys; current GR pages carry the GR author id in the Apollo blob (legacy-layout pages carry it in the authorName href, fallback path only). Both providers present MULTIPLE author/contributor refs per work — the name-agreement guard is what selects ours | Designs requiring author IDs from GB/Audible/Audnexus; assuming any provider author ID is already captured; GR extraction that does not discriminate Contributor entities from other `legacyId` carriers (reviewer/user blobs carry legacyIds too — observed in the same sample) | high on availability (both sampled); GR layout-drift risk stands per insight 62 — parse defensively |
| ST-004b | `wiki/integrations/hardcover.md:152` documents HC GraphQL author search; discussion notes §6 debt #1: HC author IDs on work responses are BELIEVED available (`contributions { author { id } }`), explicitly NOT verified | HC author-ID availability is a hypothesis pending a design-stage probe of the live GraphQL shape | F1 designs that REQUIRE HC author routes before the probe lands; asserting HC capture as established | low — deferred to design-stage prototype |
| ST-005 | `docs/design-author-dedup.md:11` — OL assigned different ol_keys to the same person in 3 of 3 live-checked pairs | One person ≠ one OL key; key inequality is not person-inequality | Uniqueness assumptions person→one-ol_key; dedup logic keyed on ol_key equality alone | high |
| ST-006 | `wiki/integrations/goodreads.md:21-26` (endpoint table: `/author/show/<id>` by id, volatile `/search?q=`, book autocomplete by title — no author-name→id search listed) + prior-art sweep found no such endpoint anywhere | GR cannot be name-searched for an author id | Any GR name-search tier in the road | high |
| ST-007 | `docs/design-author-dedup.md:89` (works UNIQUE index `(user_id, normalized_title, normalized_author)`, verified vs live schema) + merge behavior (wiki insight 67): `merge_authors` rewrites `works.author_name` WITHOUT touching `normalized_author`, bumps `merge_generation` → tag re-sync | A display-name cascade over an author's works is a proven, shipped mechanism that does not re-key work uniqueness | F1 rename paths that rewrite `normalized_author`; treating rename-cascade as novel/risky plumbing | high |
| ST-008 | `spec-bugfix-175-duplicate-authors.md:69` (confirm runs up to 5 add-pipelines concurrently) + #175 fix (author identity writes serialized, `8ad256d5`) | Import-time author linking runs under concurrency; creation races converge on one row | Linking designs assuming serial import; re-introducing per-door author creation | high |
| ST-009 | PM-verified 2026-07-28: the file's ONLY production author-create site stores `ol_key: None, gr_key: None` (`crates/livrarr-server/src/readarr_import_workflow.rs:1671-1677`, inside `process_authors_batch`; the other `create_author` occurrences in that file sit under `#[cfg(test)]` modules) while the book seed beside it captures `gr_key` (`:1990-2006`) | The Readarr door discards author-identifying data present in its input | — (opportunity, not constraint); exact usable field = design-stage verification (Q-002) | high on the discard; low on exact field shape |

## 0c. Prior Art

Searched: `docs/` and `wiki/` recursively (python walk + targeted full reads, prior-art
subagent sweep 2026-07-28), auto-memory, plus PM source verification of every load-bearing
claim cited below.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `docs/design-author-dedup.md` | The strict author-name authority (`unambiguous_author_match`) and its adoption rules; `merge_authors` as the ONE merge transaction (reuse, never reimplement); monotonic key fill; documented refusal to touch the 0.90 GR comparator (`:144`) — that follow-up is this feature. |
| PA-002 | `docs/brief-identity-layer-rewrite.md` | The routes model this spec adopts for authors ("provider ids become routes — plural, optional, zero valid"); scoped for works, zero Author content — no collision with F1; now committed as F2 behind this feature. |
| PA-003 | `spec-bugfix-175-duplicate-authors.md` | `(user_id, canonical_author_key)` uniqueness + rename recompute; the 5-wide concurrent import window any import-time linking runs inside; mandate to reuse `merge_authors` semantics. |
| PA-004 | `docs/matching-inventory-2026-07-02.md:49`, `docs/metadata-remediation-phase5-matching-spec.md:109` (ST-11) | Catalog of the live GR ≥0.90 author auto-link — one of the two unguarded writers this feature deletes. |
| PA-005 | `wiki/domain/series.md:28-40` | The same gate shape (no monitor without a resolved key) already solved for Series, including its promotion-time auto-link — precedent and contrast for the author gate. |
| PA-006 | `wiki/domain/author.md`, `wiki/architecture/roads.md` (R11), wiki insights 42-43/67 | Author lifecycle, the author-monitoring road, monitor mechanics (OL works feed, 3 controls), dedup wiring. |
| PA-007 | `build/spec-discussion-notes-176.md` (+ §8) | The source-verified defect map (§1), decisions D1–D10, rejected options R1–R7, and the O2 resolution this spec implements. |
| PA-008 | `build/grounding-176-monitor-gate.md` | PM verification record separating PM-verified from sweep-reported claims feeding this spec. |

## 1. Problem Statement

Authors have no identity road of their own. Works get a full identity pipeline; an
author's provider identity exists only as a side effect of the bibliography feature, with
a different (or absent) standard of proof per consumer — and the list-import door's
author-linking trigger is wired to a test no-op in production (`NoOpBibliographyTrigger`,
PM-verified at 4 sites). Consequence (#176): a CSV-imported library's authors are
name-only shells; author monitoring requires the OL route (ST-001), so essentially the
whole library is unmonitorable — the reporter shows 939 of 944 authors unlinked (~7.2k
works; mechanism proven, count not independently reproduced). Adjacent defects ride the
same cause: `resolve_ol_key` adopts the FIRST OL search hit with no matching bar, and a
family of GR ≥0.90 name-similarity links (four production sites) fills `gr_key` for the
series machinery — unguarded writers with different standards per consumer.

F1 makes authors real, identified entities: provider routes with one standard of proof,
name variants with one display pick, every creation door feeding one linking road, a
recovery sweep for existing libraries, a review surface for the unsure — and real author
editing. The works-side half (work→author pointer, uniqueness, settle-road inheritance)
is F2.

## 2. Requirements

- **REQ-001**: Author routes. An author carries zero or more provider routes (OL, GR,
  HC), user-scoped like the author row itself. Routes accrue monotonically from evidence;
  no automatic process ever overwrites or removes an existing route. Only explicit user
  action (REQ-008) removes a route — and removal leaves a tombstone (author, provider,
  route value, removed-when): no automatic process ever re-adds a tombstoned route,
  regardless of new evidence; the review surface may show a tombstoned candidate,
  labeled as previously removed; only an explicit user pick clears its tombstone. The
  dead `authors.hc_key` column's role is subsumed by routes (fixes B3).
- **REQ-002**: Name variants and display pick. Every distinct author name observed
  (per source: provider payloads, import rows, user entry) is retained with its source.
  One display name is chosen by a ranked priority table in the cover-rank pattern —
  two orders: English `GR → HC → GB → OL`, foreign `GB → HC → GR → OL` (PO-ratified
  2026-07-28); the order applies per-author by the dominant language of the author's
  works — computed by the EXISTING `seed::dominant_language` rule: unique maximum over
  the author's works' `language`; a tie or no language data means no dominant, which
  selects the English order; `monitor_language` is not consulted. A user-entered name
  is its own source and outranks every provider unconditionally.
- **REQ-003**: The linking road (one road, three tiers). All automatic author-route
  acquisition follows. "Identity-settled" means `IdentityStatus` Confirmed or
  Provisional — Pending, NeedsReview, and Conflict works count for nothing on this
  road. **Tier 1 (inherit):** for an author with ≥1 identity-settled work, walk EVERY
  provider key present on those works (OL, GR, HC — works routinely hold multiple
  anchors), fetch each provider's record of that work BY KEY (no search), and attach
  each author identifier that passes the name-agreement guard. The guard is the
  existing author authority: `author_verdict` must be Agree between the provider's
  name for that author id and our author's name — anything other than Agree (Grey,
  Abstain, Disagree) means no write.
  Work matching is never re-run. **Tier 2 (OL completion):** for an author with
  settled works but no OL route, OL name search — and in F1 every Tier-2 outcome
  PARKS with candidates; there is NO Tier-2 auto-link (the second-family spec review
  rejected the proposed catalog-corroboration auto-link as unsound: no
  exactly-one-candidate rule, homonym catalogs, monitoring blast radius). Corroboration
  evidence IS still computed per candidate — catalog overlap with the author's settled
  works counts only `title_verdict` Same (Grey and vetoes count for nothing) — and is
  displayed as evidence in review, ordered by strength. Tier 2 writes ONLY author
  routes (via user pick) — never any work identity. **Tier 3:** an author with no
  settled works parks visibly; when any of their works settles, the author
  automatically re-enters Tier 1. **Invariant: a bare name match NEVER auto-links —
  at any tier, from any caller; in F1, no Tier-2 candidate auto-links at all.**
- **REQ-004**: One standard — the unguarded writers die. `resolve_ol_key`'s
  first-hit adoption is deleted. ALL GR ≥0.90 auto-link writers are deleted — four
  known production sites (`livrarr-handlers` work.rs, author.rs, series.rs +
  `series_query_service::silently_resolve_author_key`; PM-verified 2026-07-28), and
  the deletion criterion is the predicate, not the count: no `author_similarity`-based
  author-route write may survive outside tests. Their consumers get the author's GR
  route via the road. Design-stage deliverable: a
  mechanical inventory of EVERY author-route write site (known so far: the two deleted
  guessers, interactive add-author user pick, the adoption gate's key fill, merge
  coalescing, the Authors/Series UI grKey write, Readarr import), each classified
  user-sovereign / keyed-evidence / road-guarded — no unclassified writer survives.
- **REQ-005**: Every door feeds the road. All author-creating doors (add-box, manual
  import, list import, series-monitor roster, Readarr import) leave new authors either
  route-linked (evidence in hand) or enqueued for the road — none orphaned. List
  import's dead trigger is replaced by real wiring (B1) — landing in the same change as
  the REQ-003/REQ-004 bar (B2): the wiring must never go live with the first-hit
  adopter still alive. The Readarr door: IF the design-stage probe (Q-002) finds a usable
  author identifier in the Readarr input, it is attached as a guarded route at import;
  if it finds none, Readarr-created authors simply enter the road like every other
  door's — no orphaning on either branch (ST-009 records only that data is currently
  discarded, not that a usable field is guaranteed). Manual-import- and
  series-roster-created authors are explicitly in scope for the road and sweep.
- **REQ-006**: Recovery sweep. A background job links existing unlinked authors via
  the road: low priority on the shared queue, per-author persisted progress (resumable
  across restarts, never one big transaction), visible progress state, recurring
  (parked/unlinked authors re-evaluate as their works settle — authors converge like
  works). Honest pacing per ST-003: hours-scale on a large library, and that is
  acceptable and visible. The sweep counts, per author, whether the deleted 0.90
  guesser WOULD have auto-linked where the road parked — surfaced as a design-review
  data point (the measurement safeguard on the REQ-004 kill).
- **REQ-007**: Review surface. Parked authors appear in a new Authors section of the
  existing Review page with their candidates and the evidence for each (name agreement,
  catalog corroboration found/missing); the user picks a candidate (writes the route) or
  dismisses (stays unlinked, re-enterable). The Authors page shows a link-state badge
  per author (linked / needs review / unlinked) that links to the fix flow. Vocabulary
  is load-bearing: **route-linked** means ≥1 provider route of any kind; **monitorable**
  means specifically an OL route is present (ST-001) — a GR/HC-only author is linked
  but not monitorable, and the monitor enable gate and its rejection message are
  UNCHANGED in F1 for exactly that case as well as for unlinked authors.
- **REQ-008**: Author editing, for real (see Q-001). (a) Rename: editing an author's
  name updates the author record and cascades the display string to all their works via
  the proven merge mechanism (`works.author_name` rewrite, `normalized_author`
  untouched, generation bump → tag re-sync; ST-007). (b) Display pick: the user can
  choose among stored name variants (their choice = user-source variant, REQ-002).
  (c) Route fixing: the user can view an author's routes with provenance, remove a
  wrong route, and trigger re-resolution (re-enters the road; candidates land in
  review). (d) The existing author-merge endpoint gets a UI door. Moving a BOOK to a
  different author stays F2 (needs the work→author pointer).
- **REQ-009**: Legacy keys are routes with honest provenance. Existing pre-F1
  `authors.ol_key`/`gr_key` values were written by the unguarded linkers (or user
  picks — indistinguishable today) and never passed the road's bar. They are ingested
  as routes with provenance `legacy-unguarded`, and they KEEP WORKING — monitoring
  that works today keeps working (no functional regression). Road re-entry re-validates
  them opportunistically — Tier-1 evidence arriving as their works settle or gain
  keys, or a user re-resolve; a dedicated legacy pass is design's option, and if
  design adds one, REQ-006's unlinked-only sweep scope expands explicitly. When road
  evidence (Tier-1 inherit or user pick) confirms a
  legacy route, its provenance upgrades; when road evidence CONTRADICTS one (a
  different id passes the guard for the same provider), the disagreement surfaces in
  review — never auto-removed, never auto-replaced; no evidence either way leaves the
  flag standing. Legacy provenance is visible wherever routes are shown (REQ-008c).

## 3. UI/Interface Design

No mockups at spec stage (PO folded the UI mode into the draft). Surfaces named for
design: Review page Authors section (list + candidate pick/dismiss, mirroring the works
review pattern); Authors page link-state badge + filter; author-detail: rename, name-
variant pick, routes panel (view/remove/re-resolve), merge action. HTML mockups, if any,
land at design stage per house rule.

## 4. Non-Requirements

- Work→author pointer, per-book author-name copy removal, works uniqueness on author id,
  and the O3 collision policy — **F2**.
- Settle-road inline inheritance (writing author routes during enrichment) — **F2**; F1's
  road does its own keyed fetches. Parser-level capture of provider author IDs is in
  scope ONLY where the road consumes it.
- Moving a book between authors from the work page (D9) — **F2**.
- Monitor gate semantics/multi-provider monitoring — unchanged; the monitor still runs
  on the OL route only.
- Tag writing continues to read the work's author-name copy; REQ-008's cascade updates
  that copy, but the tag pipeline itself is untouched (O7 activates at F2).
- #182 (list-import undo leaves author rows) — separate issue, untouched.
- Re-validating or removing existing `authors.ol_key`/`gr_key` values as a blocking
  pass — they remain functional, ingested as `legacy-unguarded` routes per REQ-009;
  only opportunistic re-validation, no monitorability regression, no destructive
  migration in F1.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Editing package (REQ-008) included in F1 — PO asked "enable author editing for real"; PM recommended yes; included in this draft | resolved | PO confirmed 2026-07-28 at the spec gate — REQ-008 ships in F1 |
| Q-002 | Exact Readarr author-identifier field usable at the import door (ST-009) | open — design-stage verification | |
| Q-003 | Tier-2 catalog-corroboration auto-link: REJECTED for F1 by the second-family spec review (xai F-001: no exactly-one-candidate rule, homonym catalogs, monitoring blast radius) — F1 Tier 2 parks always (REQ-003 as now written). Design review MAY reopen auto-link only under the tightened bar: exactly ONE candidate passing, ≥2 distinct settled-work corroborations at `title_verdict` Same, and an explicit rule for prior work-level OL failure/abstention. The PO's adversarial-test condition stands for any reopening | resolved for F1; reopenable at design | Park-always shipped in F1; tightened-bar reopening is design review's call |
| Q-004 | OL author-search response: does it carry enough catalog evidence (e.g. top-work data) to corroborate without a second fetch? Optimization only; REQ-003 stands either way | open — design-stage live probe | |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-005): A production-configuration list import creates authors that
  are linked or enqueued — the no-op trigger wiring is gone; no code path constructs
  `NoOpBibliographyTrigger` outside tests.
- [ ] **AC-002** (REQ-003): An author whose only evidence is a unique bare-name OL
  match (no catalog corroboration) is PARKED, not linked — pinned at the road, not a
  caller.
- [ ] **AC-003** (REQ-003): An author with an identity-settled (Confirmed or
  Provisional) GR-keyed work gains a GR route via Tier 1 with no name-search call
  issued; a Pending/NeedsReview/Conflict work triggers nothing; the route write fails
  closed when `author_verdict` is not Agree. A settled work holding BOTH gr_key and
  ol_key yields BOTH routes from Tier 1 (all present keys walked).
- [ ] **AC-004** (REQ-003): Every Tier-2 outcome parks — no Tier-2 code path writes a
  route without a user pick. Each parked candidate carries computed corroboration
  evidence: the count of the author's settled works whose titles match the candidate's
  catalog at `title_verdict` Same; a Grey title match contributes zero evidence.
- [ ] **AC-005** (REQ-004): `resolve_ol_key` first-hit adoption is gone, and ZERO
  `author_similarity`-based author-route writes remain in production code (predicate,
  not a site count — the four known sites and any others fall together); the
  design-stage writer inventory exists and every listed writer is classified; no
  author-route write site bypasses its classification.
- [ ] **AC-006** (REQ-006): A sweep interrupted by process restart resumes from
  persisted per-author state without re-processing completed authors; sweep progress is
  queryable; the would-have-linked-at-0.90 counter is emitted.
- [ ] **AC-007** (REQ-007): A parked author appears on the Review page with candidates
  and evidence; picking writes the picked route. When the picked route is OL, the
  author becomes monitorable (ST-001 gate passes); an author with only GR/HC routes
  remains non-monitorable and receives the unchanged monitor-gate rejection.
  Dismissing leaves the author unlinked and re-enterable; the Authors page badge
  reflects linked / needs-review / unlinked.
- [ ] **AC-008** (REQ-002): Name variants from two providers plus a user entry yield
  the user's choice as display; with no user choice, the ratified order picks; the
  foreign-dominant author uses the foreign order; a tied or language-less set of works
  selects the English order.
- [ ] **AC-009** (REQ-008): Renaming an author updates the author record and every
  work's displayed author string, does NOT change any work's `normalized_author`, and
  triggers tag re-sync; work uniqueness collisions are impossible by construction
  (ST-007 mechanism).
- [ ] **AC-010** (REQ-008): Removing a wrong route and re-resolving lands the author in
  review with fresh candidates; the SAME automatic evidence that produced the removed
  route cannot re-add it (road and sweep honor the tombstone); a tombstoned candidate
  shown in review is labeled as previously removed; an explicit user re-pick clears the
  tombstone and re-adds the route.
- [ ] **AC-011** (REQ-005): Per Q-002's resolution — if a usable Readarr author
  identifier exists, import attaches it as a guarded route when present; if none
  exists, Readarr-created authors are verified to enter the road (enqueued, not
  orphaned). Exactly one branch is implemented and tested.
- [ ] **AC-012** (REQ-003/REQ-006): An author parked at Tier 3 whose work later reaches
  Confirmed or Provisional identity is automatically re-evaluated — it re-enters
  Tier 1 with zero user action; route attach remains conditional on Tier-1 evidence
  (a present work-level provider key whose author id passes `author_verdict` Agree).
  Happy path pinned: a Confirmed work holding an agreeing provider key yields the
  route with no user action. No universal gains-a-route claim — a Provisional
  bridge-work may hold no walkable keys, and that outcome is a compliant park.
- [ ] **AC-013** (REQ-003): On a multi-contributor provider record (both sampled
  providers present several author refs per work), every contributor identifier whose
  name reaches `author_verdict` Agree against our author attaches — one route per
  agreeing identifier (attach-each, matching REQ-003's attach-every-Agree rule; in the
  common case exactly one agrees), and no route for a Grey/Abstain/Disagree
  contributor; when zero contributors agree, nothing attaches and a routeless author
  parks.
- [ ] **AC-014** (REQ-009): Pre-F1 keys display `legacy-unguarded` provenance; a
  Tier-1-confirmed legacy route upgrades its provenance; a road-found CONTRADICTING id
  for the same provider surfaces both in review with neither auto-removed; author
  monitorability never regresses during any of this.
