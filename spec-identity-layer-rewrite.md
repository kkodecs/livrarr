---
feature: "identity-layer-rewrite"
stage: spec
status: draft
version: 10
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018, REQ-019, REQ-020, REQ-021, REQ-022, REQ-023, REQ-024, REQ-025, REQ-026, REQ-027]
---

# Spec: identity-layer-rewrite

The rewrite committed by the PO on 2026-07-28 (F2, behind author-provider-linking):
the app's own work id becomes the identity anchor; provider ids become routes.
Decision record: `build/findings-identity-sidebar-2026-07-25.md` (works side, PO in
the room), `build/spec-discussion-notes-176.md` §3+§8 (authors side + the F1/F2
split), `docs/brief-identity-layer-rewrite.md` (scoping brief). Those decisions are
settled; this spec turns them into testable requirements and isolates what is still
genuinely open. v2 folded the r1 cross-family review (openai 26 + xai 11;
`build/reviews/identity-layer-rewrite/FOLD-SPEC-R1.md`); v3 folds r2 (openai 14 +
xai 6 fresh; `FOLD-SPEC-R2.md`). v4 folds r3 (openai 7 + xai 2 fresh), authored
by the OpenAI family under the architect swap (`FOLD-SPEC-R3.md`). v5 folds r4
(anthropic 8 + xai 3 fresh), authored by the OpenAI family (`FOLD-SPEC-R4.md`).
v6 folds r5 (anthropic 4 residuals; xai verified with no findings), authored by
the OpenAI family (`FOLD-SPEC-R5.md`). v7 folds the final r6 micro-review
(anthropic 3 residuals; xai clean), authored by the OpenAI family
(`FOLD-SPEC-R6.md`). v8 (2026-08-18) adds REQ-027 — the machine title+author
search fallback at the shared chase's anchor seam — plus its REQ-013 carve-out
and AC-024: the PO-initiated round-13 scope widening, folded from the PM prep
ratified in the 2026-08-18 handoff. The amendment shipped without its own spec
review round (PO may order one); the round-13 code review covers its semantics.
v10 (2026-08-18 evening, PO decisions from the clean re-import): (1) REQ-027
search-leg precondition widens — a provider whose derivable anchors have ALL
dead-ended terminally (`not_found`) at the current generation counts as
anchor-less for the search leg (the junk-ISBN starvation found live on Ender in
Exile: a print-on-demand ISBN occupied the anchor slot, OL/HC fetched it
not_found, and the search that would have auto-linked never fired). The
no-work-route precondition and ledger bounding are unchanged. (2) REQ-027 card
surfacing: minting a PendingRoute card raises a user notification (toast + the
existing notification surface); the review card presents the book (title +
author), the proposal in plain language (provider name + proposed catalog
entry), and a clickable link to the proposed provider page so the user can
verify before affirming — never a bare internal id string. Exact copy at
design/implementation, plain-English rule binding.
v9 (2026-08-18, PO live decision after first production day): REQ-027 revision —
text-decisive unambiguous picks now AUTO-LINK (the card bar produced correct-but-
frictional proposals at real-library scale; "the cure was worse than the problem" —
PO); adds the PendingRoute card lifecycle rules (decision-time generation, satisfied-
card cancellation — the sibling-card staleness defect found live), the REQ-014
Goodreads cover-source containment (GR layout drift broke cover-image extraction,
verified live), and AC-025. AC-024(b)/(c) fixtures move to near-miss shapes.
v11 (2026-08-20, PO live order after the work-74 starvation): REQ-027
eligibility becomes per-provider — a search-capable provider lacking its own
work-level route fires the leg; one provider's id no longer disables
completion for the others. One per-work ledger; AC-026 added. Shipped without
its own spec review round (flagged, as v8–v10).

## 0a. Design Principles

Choices committed by the PO. If a requirement conflicts, the principle wins.

- **P1 — Own anchor, provider routes.** Livrarr's own work id is the identity
  anchor. Provider ids are routes to it: plural, optional, zero is valid, several
  per provider is valid. (D2)
- **P2 — A book no provider catalogues is still a book.** Minimum to create a
  work: main title + at least one author. Everything else is optional with a
  placeholder. (D3)
- **P3 — Two failure directions, two tools.** *Same book, different clothes*
  (too strict → lost matches) and *different book, same clothes* (too loose →
  wrong data) are different failures; one threshold cannot serve both. (D8)
- **P4 — Human watching → flag, never filter. Machine alone → decide or defer,
  never guess silently.** Human-watching surfaces, exhaustively: manual import,
  import review, and list-import's review/add surface (D9's "bulk add"). Direct
  add needs no flag machinery — the user's pick IS the decision. Machine-alone
  surfaces: author/series monitors, Readarr import, background convergence.
  (D9, D8-authors)
- **P5 — Evidence ladder: the user's choice > the user's file > a provider id >
  nothing.** (D10)
- **P6 — Edition-scoped ids are lookup keys, never comparisons.** ISBN, ASIN,
  GR book id — any edition-scoped id, current or future. A shared one may
  confirm sameness; a different one proves nothing (a work has dozens of
  editions). Nothing may ever demand a corroborating edition id or veto on a
  differing one. Written down so nobody helpfully restores Rule A. (F4/T3)
- **P7 — Covers: accuracy over resolution; never downgrade; the user's override
  always wins.** (D13, standing cover decisions)
- **P8 — "Whose text is this?" separates works.** Study guides and adaptations
  (someone else's text) are different works; translations (the author's text,
  same content) are the same work. Abridgements are their OWN work (PO
  2026-08-02, superseding D11's abridgement arm — an abridged and unabridged
  text cannot even share a reading position). An omnibus is its own work that
  never merges with its parts; contains/part-of pointers are stored when
  providers supply them. (D11 as amended)
- **P9 — No LLM chooses a match anywhere.** Standing project rule (insight 13);
  the rewrite keeps matching fully deterministic.

## 0b. System Truths

Facts about the environment we don't control. Sampling status is stated honestly;
"sidebar transcript" = live fetches recorded 2026-07-25 in
`~/.claude/projects/-mnt-opt-livrarr/83f71bd9-5365-4550-b56c-e4b237a97773.jsonl`.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | OL work JSON `/works/OL4288870W.json` sampled 2026-07-25: `title: "Einstein"`, no subtitle field at work level; HC stores `subtitle` as its own field | The subtitle is edition-level, not work-level; a work record legitimately carries the bare main title | Treating a one-sided subtitle as evidence of a different work; any work-level subtitle "match" requirement; treating the work's stored subtitle as edition truth | High |
| ST-002 | GR page `__NEXT_DATA__` sampled 2026-07-25: book ids 10884, 2059858, 6602781 all resolve to `Work` `legacyId: 985244`, with work-level `originalTitle` and a `/work/editions/985244` list, in the blob already fetched | A GR `/book/show/<id>` number is an EDITION id; the work id exists in the same fetch at zero extra cost | Using a GR book id to answer "same work?"; paying an extra request to learn the work id; migrating a stored `gr_key` as a work id | High on availability; Medium on layout stability (insight 62: three GR parse paths drifted silent-empty in one day) |
| ST-003 | Provider ontology survey, sidebar transcript (HC GraphQL schema; GR page data; OL work JSON; GB volumes API; Audible/Audnexus responses) | HC `book` is work-shaped (Einstein: 33 editions, 4 languages) with `compilation`, `parent_book_id`, `alternative_titles`, `canonical_id`, `default_{physical,ebook,audio,cover}_edition`; GR has work id + editions list; OL separates `/works/` from `/books/` and models authors as an array with roles; **GB models volumes only; Audible/Audnexus model one production per ASIN — neither has a work concept** | Requiring a work-id route from GB, Audible, or Audnexus; assuming any provider's author field is bare strings (OL's is role-tagged); modeling a work with a singular flat author string | High |
| ST-004 | Same survey; HC `book_mappings` lists two *verified* GR ids for one work | A work legitimately carries many ISBNs/ASINs (dozens across editions) and may carry several ids per provider | One-column-per-provider storage; uniqueness assumptions on provider id alone; treating multiple edition-id routes as a conflict | High |
| ST-005 | HC Einstein record, sidebar transcript: `title` = "Einstein: His Life and Universe" (subtitle embedded) while `subtitle` = "His Life and His Universe" (different spelling) | Providers are inconsistent about the title/subtitle split even within one record; the structured `subtitle` field and the embedded tail can DISAGREE textually | Trusting any provider's split blindly; storing subtitle without normalization; any split rule that must compare the embedded tail against the structured field to decide | High |
| ST-006 | Not yet sampled — deferred to code-stage prototype, each via capture of a data-rich entity BEFORE any design branches on it: (a) OL author `role` label value domain; (b) HC `compilation` / `parent_book_id` / `canonical_id` / `alternative_titles` / `default_{physical,ebook,audio,cover}_edition` full observed shapes incl. null and dangling-reference semantics; (c) GR editions-list pagination shape; (d) HC author-id availability on the existing GraphQL query (`contributions { author { id } }` — notes-176 §6 debt 1) and GR author-id href reliability across layouts (§6 debt 2); (e) observable provider signals distinguishing translation / abridgement / adaptation / study guide / omnibus (per provider); (f) provider payload shapes that present two work ids as ONE work (aliases / canonical pointers) — the only admissible "maps together" evidence for REQ-007 | (to be established by capture per the value-domain rule) | Branching on any of these fields or signals from plausibility-sized assumptions; shipping a whose-text classifier before (e) is sampled; role-based primary-author selection before (a) is sampled | Low (unsampled — deliberately) |
| ST-007 | EPUB OPF spec + observed practice; not freshly sampled this session | EPUB files commonly embed their own cover image, addressable without provider traffic | Assuming every file has one (absence is a normal case); conflating "could not inspect the file" with "the file has no cover" | Medium; sampling deferred to code-stage prototype on the PO's real library |

## 0c. Prior Art

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `build/findings-identity-sidebar-2026-07-25.md` | The decision record: D1–D16 (works), six tensions T1–T6, the fix-now/rewrite split. This spec implements its Part 4(b). |
| PA-002 | `docs/brief-identity-layer-rewrite.md` | The scoping brief; names the core move, the six-door blast radius, and the verify-before-sizing debt (now paid). |
| PA-003 | `build/verification-vlist-2026-08-02.md` | V-list re-verified at HEAD `350d6722`: V1–V4 already conform through `identity_matching`; only V5–V8 survive (peripheral). The rewrite converges ONE authority, not eight sites. Rule A and the cover gate are already deleted. |
| PA-004 | `build/spec-discussion-notes-176.md` §3, §8 | Authors-side decisions D1–D10 and the settled F1/F2 split: F2 carries D5 (work → author record pointer), D6 (uniqueness on author id), D7-forward (settle-road inheritance), O3/O7/O8. §6 debts 1–2 (HC/GR author-id availability) now sit in ST-006(d). |
| PA-005 | Author-provider-linking as-built (`build/design-176/CONFORMANCE-author-provider-linking.md`) | F1 already shipped the authors half of the routes model: `author_links` routes table, name variants, display pick, review cards. F2's works-side routes mirror shapes F1 proved; the settle-road author-inheritance seam was left for F2. `PARKED-U8b-route-repair.md` (tier1_inherited repair) is a SEPARATE parked design round — out of scope here. |
| PA-006 | `docs/design-subtitle-matching.md` (r3, as built) | The five fix-now changes shipped 2026-07-26/27, incl. Rule-A deletion (`title_id_trust`) and cover-gate deletion. Its C5 repair collected GR *book* ids; this rewrite upgrades them to work-id routes (T4). |
| PA-007 | `identity_matching` authority (`crates/livrarr-domain/src/identity_matching.rs`): `title_verdict`/`author_verdict`/`id_verdict`/`title_id_trust`/`pick_best_candidate` + deterministic storage key feeding `works.normalized_title`/`normalized_author` | The one matching authority the rewrite extends. The storage key + UNIQUE index are the dedup substrate D6 re-keys. Named verdicts are the vocabulary REQ-024's merge predicate binds to. The title PARSE (main/subtitle/volume triple) is the deterministic splitter REQ-003 binds to. |
| PA-008 | `crates/livrarr-enrichment/src/cover_rank.rs` | The multi-table rank pattern (EBOOK_ENGLISH / EBOOK_FOREIGN / AUDIOBOOK behind one accessor) that D13's cover rank and D4-authors' name-priority copy. |
| PA-009 | Roads map + door gate (`docs/roads.md`, `tests/behavioral/test_door_gate.rs`, insight 46) | Door→road wiring is untested by default; every changed door needs its door-gate row traced at design. Six doors change here — this is the feature's biggest process risk. |
| PA-010 | Standing cover decisions (memory `project_cover_design_decisions`, D16) | Per-format slots + audiobook-falls-back-to-ebook stay unchanged; accuracy > resolution carries over. |

## 1. Problem Statement

The library's answer to "which book is this?" is built on the wrong keys. Each work
holds at most one id per provider, and the Goodreads slot actually holds an *edition*
number — so two copies of the same book carry different numbers by design, and the
system can never conclude they are the same work (ST-002). Books that no provider
catalogues cannot exist cleanly. A wrong stored id protects itself forever, because
matching lets it veto every correct match that disagrees with it. Six creation doors
apply six different identity policies — one (Readarr import) resolves no identity at
all despite holding the strongest evidence (ISBN + ASIN + the files). Covers inherit
these defects: "validated" merely meant a since-deleted gate once passed, the file's
own cover (the only image guaranteed to match the book in hand) is never used, and a
work whose lookups are exhausted shows "searching" forever. Authors got their half of
the fix in F1; the works side, and the seams that join the two, are this feature.

## 2. Requirements

### Identity model

- **REQ-001**: Routes storage. A work's provider identity is a set of routes:
  (provider, provider-scoped id, id kind), many per provider, zero legal. All
  current per-provider anchor consumers read routes instead. No behavior may
  distinguish "the" provider id — any route arriving means "this is that book."
  Ownership rule: work-id routes live on the work; edition-scoped ids (ISBN,
  ASIN, GR book id) live on their edition record (REQ-005) and appear in the
  work's route set as derived projections — single-sourced, never dual-written.
  Scope and uniqueness invariant: routes are user-scoped exactly as works are;
  within one user's library a given (provider, kind, id) attaches to at most ONE
  work. An arriving route that is already attached to a different work of the
  same user is REQ-007's conflict class (c), never a silent reassignment.
- **REQ-002**: Minimal creation. A work is creatable and fully usable with only
  a main title and at least one author. "Fully usable", exhaustively: create,
  list/detail display, edit, per-media-type monitoring, file import/attach, tag
  writing, user-set and file-extracted covers, and the manual provider search —
  all function; absent fields render as placeholders, not errors. The one
  deliberate limitation: automatic provider enrichment requires a usable route
  (REQ-013) — that exclusion is intentional, not degradation.
- **REQ-003**: Subtitle is its own field, split deterministically. Work records
  store main title and subtitle separately; editions carry their own subtitle
  when a provider supplies one (the edition-level truth, ST-001). The split
  rule involves NO comparison between the embedded tail and the structured
  field (ST-005 forbids one): stored main = the authority's title parse of the
  provider title, always (PA-007 — any parsed-out tail never remains in main);
  stored subtitle = the provider's structured `subtitle` field when present,
  else the parse's extracted subtitle, else absent. Truth table: structured
  only → (parse.main, structured); embedded only → (parse.main,
  parse.subtitle); both → (parse.main, structured); neither → (parse.main,
  absent). The ST-005 payload lands exactly as: main "Einstein", subtitle "His
  Life and His Universe". At creation, the P5-winning evidence is captured as
  a stable identity-title tuple (parsed main, the subtitle selected by this
  truth table or absent, and parsed volume or absent); a REQ-002 minimum-only
  door uses that door's title parse and records absent subtitle/volume when the
  input supplies neither. Provider/default-edition changes never mutate that
  tuple. An explicit user edit to any title part updates it and re-keys through
  REQ-024. The displayed main title is the tuple's main.

  The edition-derived work subtitle is a separate DISPLAY-only machine
  projection, not edition truth and never itself an input to matching, dedup,
  the REQ-024 uniqueness key, or P8 classification. Its edition is selected
  totally: default ebook, default audiobook, default physical, defaults for
  other named formats lexically, then non-default known-format editions ordered
  ebook → audiobook → physical → other named formats lexically, then
  unknown-format editions. Within a tier, source order is user's file →
  Hardcover → OpenLibrary → Goodreads → Google Books → Audible →
  Audnexus → any other provider by provider name → source-less or
  migration-only; an exact tie breaks by provider-scoped edition id, then the
  edition's own stable id. The selected edition's subtitle is the projection,
  INCLUDING absence — no lower-ranked edition lends one. With no edition the
  projection is absent. Monitoring flags never affect selection.

  Rendered subtitle is total over the independent user-override state: an
  explicit user value renders that value; an explicit user-set absence renders
  absent; in automatic state, a non-absent machine projection renders first,
  else the identity tuple's subtitle renders, else absent. Thus the ST-005
  creation payload still displays its tuple subtitle even before an edition
  exists. Setting a user value or absence updates the identity tuple and takes
  REQ-024's re-key lifecycle; reset-to-automatic removes only the display
  override and leaves the tuple unchanged. Each override state can exit through
  another explicit edit or reset.

  The machine projection is recomputed atomically after EVERY committed change
  to its candidate editions, default designation, format, source-order fields,
  or the selected edition's subtitle value/provenance. It stays current even
  behind a user override. A parked conflict changes neither projection nor
  rendered subtitle. Every machine recomputation records provenance and may
  change display only: the identity tuple, uniqueness key, and merge state
  remain unchanged. No matching or display logic re-derives the split from a
  joined string.
- **REQ-004**: Work shape extensions. Works gain: alternative titles,
  compilation flag, containment pointers, canonical pointer, a default edition
  per format (ST-003's Hardcover skeleton), an ordered contributor list with
  roles (ST-003's OL shape), and subjects (people/places/times/topics) — each
  populated when a provider supplies it and absent otherwise. Containment is
  modeled in BOTH directions (a container lists contained works; a contained
  work lists its containers), many-to-many, and a pointer to a work not in the
  library is stored as an unresolved reference, not dropped. Design may not
  branch on the unsampled shapes until ST-006(a)/(b) are captured.
- **REQ-005**: Editions are first-class. Editions exist as records attached to
  exactly one work (format, language, edition-level ids, edition subtitle,
  cover), so edition-shaped facts (ISBN, ASIN, GR book id, per-format cover)
  live at edition level instead of being flattened onto the work. An
  edition-id route resolves through its edition to that one work (REQ-001's
  projection rule). The format domain includes ebook, audiobook, physical,
  other declared formats, and `unknown`; language may be absent. A provisional
  edition uses a known format/language ONLY when direct payload or file evidence
  supplies it — an ISBN or ASIN kind alone proves neither format nor language.
  An unknown-format edition remains a valid lookup/route home, sorts last for
  REQ-003's display projection, and cannot be a per-format default or supply a
  REQ-014 format cover slot. Its cover is retained and, whenever a target slot
  is uncovered, surfaces through REQ-016's single work-scoped format-needed
  panel rather than being discarded or placed in a guessed slot; with all slots
  covered it remains retained and the panel reappears if a slot becomes
  uncovered. Later direct evidence resolves unknown format or
  absent language idempotently; contradictory direct evidence parks that
  format/language candidate for review while the existing committed value (or
  `unknown`/absent) remains active until the user chooses. The edition stays
  usable as a route home throughout, and permanent absence of such evidence
  validly leaves format `unknown` and language absent.
- **REQ-006**: Goodreads work-id capture. Wherever a GR book page is already
  fetched, the work id is captured and stored as a route at no extra request
  (ST-002). Upgrade triggers for works holding only a GR book id (incl. every
  C5-repaired work), exhaustively: ANY GR book-page fetch for that work — an
  enrichment pass, a manual refresh, or a convergence visit — records the work
  id idempotently. Page unavailable or parse drift changes nothing and retries
  on the normal convergence cadence; a later successful fetch completes the
  upgrade. No standalone sweep is required or performed.
- **REQ-007**: Conflicts are flagged, never silently enforced. The conflict
  classes, exactly: **(a)** a same-provider WORK-id disagreement — a candidate
  work-id from a provider differing from that provider's existing work-id
  route on the work; **(b)** a cross-provider work-key disagreement detected
  during matching (the Rule-B detection, veto dropped); **(c)** an arriving
  route — any kind — already attached to a DIFFERENT work of the same user
  (REQ-001's uniqueness invariant). Explicitly never a conflict: any number of
  DISTINCT edition-id routes on one work (ST-004), and two work ids the
  provider's own payload presents as one work (aliases/canonical pointer —
  ST-006(f); until (f) is sampled, every same-provider work-id disagreement is
  class (a)). Pending state, all classes: existing routes stay untouched, the
  contested candidate is parked on the review surface, and matching proceeds
  on the uncontested evidence — nothing blocks. Review resolutions, each
  audit-trailed and each naming the SURVIVING route set explicitly: accept
  (candidate becomes a route; any route it displaces is retired to an audited
  archive — excluded from matching and projection, never silently deleted, so
  a wrong id cannot re-trigger the same conflict), reject (candidate
  discarded), different-work (candidate belongs to another work — for class
  (c), the user picks which work keeps the route). A wrong id must not protect
  itself. Class-(c) transfer is total over route kind. A work-id route moves
  between the chosen works atomically and its former attachment is archived.
  An edition-id resolution moves ONLY the contested id, never its source
  edition, sibling ids, cover, subtitle, language, or format by implication:
  the id is removed from the losing edition, that attachment and provenance
  are archived, and the id is attached to exactly one edition of the winning
  work. A winning-work edition qualifies as a target ONLY when the parked
  provider/file payload presents the contested id together with a
  non-contested edition id already homed on that edition — REQ-026's direct
  co-edition proof. Title, subtitle, format, language, or cover similarity never
  qualifies. An explicit user target-edition choice overrides this candidate
  set (P5). Without that choice: exactly one qualifying edition becomes the
  home; several leave the card pending until the user selects one; zero creates
  one provisional edition under REQ-005/026. These cardinality branches are
  disjoint and exhaust every winning-work edition graph. The losing edition
  keeps every non-contested fact and id; if none remain, its empty shell is
  archived rather than orphaned. The
  transfer and projected route update are one atomic outcome: failure leaves
  the original graph and pending card unchanged and actionable for the same
  resolution to be retried, and no intermediate or final state dual-writes the
  id. This procedure applies whenever accept or
  different-work chooses a winner other than the current owner; reject leaves
  the current edition graph intact.

### Matching

- **REQ-008**: One authority, two directions. All same-work decisions route
  through the single matching authority. Each failure direction has its OWN
  named guard set — the lost-match direction (too strict) and the wrong-merge
  direction (too loose) — named at design, tuned independently: adjusting one
  direction's guards may change only that direction's outcomes (P3).
- **REQ-009**: Edition-scoped ids never compare. No code path may require, or
  veto on, (dis)agreement of ANY edition-scoped id — ISBN, ASIN, GR book id, or
  a future edition-scoped kind — between candidate work identities. Shared
  edition ids may only *confirm* (P6). Mechanical scope: edition-id equality
  may be consumed as same-work evidence only inside the authority
  (`id_verdict`'s confirm arm) and the migration mapping; everywhere else it is
  a lookup key. [Already true on the settle road since the Rule-A deletion;
  this REQ pins it tree-wide, permanently.]
- **REQ-010**: Peripheral matchers conform. Of the four surviving off-road
  recipes (PA-003): the two dedup keys (V5 list-import, V8 discovery display)
  and the fast HC cover search's title comparison (V7) are replaced by, or
  derived from, the authority's primitives — they decide title-sameness. The
  cover junk list (V6) is NOT a sameness decision: it dies with its context
  when REQ-014's rank replaces trust-grade selection, and its deletion is
  asserted there. After this feature, zero sites carry a private
  title-sameness recipe.
- **REQ-011**: Text-identity classification. Where the system must decide
  whether two catalog entries are the same work in different clothes or
  different works (study guide, adaptation, abridgement, omnibus), the decision
  follows P8's whose-text rule using ONLY sampled provider signals (ST-006(e));
  an entry carrying no recognized sampled signal goes to the review surface
  (P4) — never a silent guess, never a title-keyword heuristic. Until
  ST-006(e) is captured, review is the only classification path.
- **REQ-027**: Machine search fallback (v8 amendment; PO-initiated,
  2026-08-18). Scope precondition (v11, PER-PROVIDER — supersedes v8's
  work-level rule): the leg is evaluated per search-capable provider. Each
  enrichment pass adds ONE title+author search-fallback leg for every
  applicable provider that (i) holds NO active work-level route of its own on
  the work AND (ii) has empty anchor derivation or (v10) only anchors whose
  durable standing is terminal `not_found` at the current generation —
  OpenLibrary
  (`search.json`), Goodreads (`/book/auto_complete`), Hardcover (search) only;
  never Google Books, Audible, or Audnexus (FP-ST-003 providers keep
  anchor-only dispatch). The leg is a route-finding action (REQ-013's
  vocabulary), the machine twin of the manual search: query = identity-tuple
  main title + primary author record name, through the shared outbound queue at
  the pass's priority, with honest chase accounting (a spawned search or probe
  fetch sets the pass's chase flag; skips never do). Candidate selection rides
  the one authority — `pick_best_candidate`, Same-tier only (`accept_grey` =
  false); P9 stands (no LLM). Outcomes, exhaustive:
  (a) *Corroborated settle* — the selected candidate's own edition evidence
  contains an edition-scoped id already active on the work (ASIN / ISBN-13 /
  GR book id): P6's confirm arm. The candidate's work-level id — plus, for
  Goodreads, the corroborating book id as edition-homed evidence — enters the
  identity road through the captured-route handoff and settles machine-alone
  with an audit naming the search-fallback origin and the corroborating id
  kind. A differing edition id remains no evidence and never vetoes (REQ-009).
  (b) *Text-decisive auto-link (v9)* — with no corroborating id, an
  UNAMBIGUOUSLY decisive pick still settles machine-alone: the winner's title
  verdict is Same under the authority's parse AND its author verdict is Agree
  (Abstain never qualifies) AND no other candidate proposing a DIFFERENT
  provider work id clears that same Same+Agree bar. The decision function
  lives in the matching authority (REQ-008), never inline in the queue. The
  settlement audit and route provenance name the text-decisive origin,
  distinguishable from (a). [PO 2026-08-18: at real-library scale the card
  friction outweighed the wrong-book risk; a wrong link remains displaceable
  and audited via REQ-007's conflict machinery.]
  (c) *Proposal card* — a pick above the picker bar that is NOT text-decisive
  (author Abstain, Grey title, or two distinct work-id candidates at the bar)
  never writes a route: the road mints ONE `PendingRoute` review card carrying
  the proposed route (semantic idempotency — an equivalent pending card is
  reused, never duplicated); affirm connects through the existing resolve
  continuation.
  (d) *Miss* — no candidate clears the bar: an honest miss.
  Card lifecycle (v9, the insight-91 rule applied to route cards): a
  settlement that activates a route proposed by a pending `PendingRoute` card
  cancels that card in the same transaction (satisfied); listing/loading a
  pending card serves the CURRENT actionable generation (mint generation is
  history); resolve proceeds on mere generation drift and rejects with a
  specific 409 only on real proposal invalidation (work gone, or the proposed
  route now actively owned by a different work); affirming a card whose route
  is already active on this work is a success no-op.
  Bounding: the generation-scoped machine-chase attempt ledger covers BOTH
  search-leg classes — the edition-only bridge (already selectable), the
  not-connected minimum work, and (v11) the connected work with at least one
  eligible search-capable provider — the latter two become cadence-selectable
  for the search leg only. ONE per-work generation-scoped ledger, not
  per-provider: a pass that fired at least one leg and ended every fired leg
  in (c) or (d) burns one attempt; a pass reaching any (a) or (b) burns none.
  At threshold the work leaves automatic selection until its identity
  generation changes. A provider already holding an active work-level route
  on the work never fires its own leg; on connected works, outcome (a)'s
  corroboration draws on the work's full active edition-id set, and another
  provider's work-level route is corroboration-neutral (neither confirms nor
  vetoes — P6 unchanged). Applicability filtering is unchanged (a foreign
  work fires only Goodreads' leg). [v8's "works holding ANY work-level route
  never fire a search leg" is SUPERSEDED — PO live decision 2026-08-20 after
  the work-74 starvation: one provider's id disabled completion for every
  other provider.] A provider may spend at most one corroboration probe per leg, riding
  its existing fetch machinery and rate bucket; an id already present in a
  search/autocomplete response is never re-requested (FP-ST-002). Search legs
  do not read or write the provider-response cache. REQ-013's manual search is
  otherwise unchanged and remains the human tool at the same route-finding
  trust bar.

### Badge and enrichment gate

- **REQ-012**: The badge reports connectedness. One vocabulary, everywhere:
  **user-confirmed / connected / not-connected**, replacing the old "a provider
  work id exists" meaning. Not-connected (zero routes) is a first-class,
  honestly-labeled state — not an error state. No route-kind distinction (an
  ASIN-only book is simply connected; Q-001/Q-002). User confirmation is
  provenance ON TOP of connectedness, and it implies a surviving route:
  a legacy setter=user anchor migrates as a route, so user-confirmed with zero
  routes is structurally impossible; should the invariant ever be violated,
  the badge falls back to not-connected and a review card repairs it. Legacy
  mapping rule, total over every legacy identity-state value: anchor
  setter=user → user-confirmed (with its route); else ≥1 route after
  migration → connected; else → not-connected. Exact UI wording finalized at
  design.
- **REQ-013**: Enrichment eligibility keys on usable routes, not the badge.
  The usable-kind list is CLOSED and declared: work ids (OL work, GR work, HC
  work) and edition ids (ISBN-13, ASIN, GR book id) — every route of a
  declared kind makes the work enrichable; a kind not on the declared list
  does not count until deliberately added. Works with ZERO USABLE routes —
  whether they hold no routes at all or only undeclared-kind routes — are
  excluded from automatic anchor passes (no retry storms) and offer the manual
  title+author provider search instead (a route-finding action; Q-001);
  REQ-027's bounded machine search-fallback selection is the sole automatic
  carve-out. Their
  badge still reads per REQ-012 (a route of any kind shows connected). The
  old "no metadata until identity settles" gate must not starve zero-route or
  edition-route-only works (T1).

### Covers

> **Containment (v9, PO 2026-08-18):** Goodreads is EXCLUDED as a cover
> candidate source. The GR page layout drift broke cover-image extraction
> while id extraction stayed correct — verified live: unrelated-book covers
> at scale (51 ebook + 44 audiobook slots), identities independently
> confirmed right. Machine-selected GR-sourced covers are re-selected by a
> marker-gated one-shot heal; manual covers are never touched. Re-enabling
> GR covers requires the parser fix pinned against a captured drifted page
> (its own round). This note supersedes nothing in REQ-014's rank — GR is
> simply absent from the candidate set until then.

- **REQ-014**: Cover rank. For each target-format slot, cover selection follows,
  strictly: user's choice for that slot → the user's file cover whose edition
  has that format → the work's default edition for that format → any other
  edition of that SAME known format. If the audiobook slot has no eligible
  audiobook cover, it then uses the selected ebook cover with the existing
  explicit fallback label (PA-010); otherwise selection enters REQ-016's
  placeholder evaluation. An unknown-format edition cover is retained but eligible
  for NO format slot until REQ-005 resolves or the user assigns its format —
  accuracy beats guessing (P7). Resolution reruns rank immediately. Selection
  never downgrades an existing cover to a lower rank, and the user's choice
  always wins. Both format slots remain shown. The rank cutover retires the trust-grade
  selection machinery INCLUDING the V6 junk list (`should_reject_cover`'s
  private substring list) — asserted deleted here, per REQ-010.
- **REQ-015**: Embedded-cover extraction, local half. Supported format for
  this feature: EPUB (the load-bearing ebook format; others are
  Non-Requirements). The system can extract the embedded cover from a user's
  own EPUB — no provider traffic — and offer it at its rank. Three distinct
  outcomes, never conflated: cover extracted; file inspected and carries no
  cover (a normal, silent absence); file could NOT be inspected
  (malformed/encrypted/unreadable — logged against that observed file revision,
  retriable, and never recorded as "no cover") (ST-007). Retry is event-driven,
  not an endless timer: once per newly observed file revision after repair or
  replacement, once on a library rescan that reports the file changed, and on
  every explicit user retry. An unchanged unreadable revision is not retried by
  background cover passes. A retry exits to extracted, verified-no-cover, or a
  refreshed could-not-inspect record; file removal exits to file-gone and cover
  selection falls through REQ-014. Every local retry makes zero provider
  requests. [The paced provider half of the historical cover sweep is sequenced
  separately: Q-006.]
- **REQ-016**: Honest cover placeholders. Placeholder evaluation has one
  work-scoped state and three slot-scoped states. Whenever at least one
  target-format slot lacks an eligible selected cover AND any retained
  unknown-format edition has a cover, the UI renders exactly one work-scoped
  *Cover found — format needed* panel — never one copy per slot. It lists all
  such covers in REQ-003's stable edition order. The user may assign the chosen
  cover's format, wait for REQ-005 direct evidence, reject that chosen cover, or
  set a cover for a chosen uncovered slot. Waiting leaves the panel pending;
  every mutating action updates the shared work state once and reruns REQ-014
  for every slot. Rejecting archives only the chosen cover and its provenance:
  another retained unknown cover keeps the single panel while an uncovered slot
  remains, and rejecting the last removes the panel.

  Slot evaluation is exhaustive alongside that work state. Any uncovered slot
  with zero USABLE routes (REQ-013) renders **(1)** *nowhere to look* and offers
  the manual provider search — a route-finding action, NOT a cover retry — even
  while the format-needed panel is present. With at least one usable route, the
  panel, while present, supplies the placeholder; when it is absent, the slot
  renders exactly one of **(2)** *Searching for a cover* while eligible provider
  lookups are pending/retriable, allowing wait or set-cover, or **(3)** *No cover
  found* once those lookups are exhausted, offering an on-demand provider re-ask.
  A successful manual search creates a route, exits *nowhere to look*, and
  reevaluates both the work panel and every slot; it does not discard the retained
  unknown-format cover. Every mutation performs that same atomic reevaluation.
  None of the states is permanent, and the user can always set a cover directly
  at rank zero.
- **REQ-017**: Source, not trust. Cover UI shows *source* (which provider /
  your file / yours); the "Trust: validated/unvalidated" label and its stored
  trust grades are retired (D15) — "validated" only ever meant the deleted gate
  passed it.

### Doors

- **REQ-018**: One identity road, six doors. All six creation doors (direct
  add, manual import, list import, author monitor, series monitor, Readarr
  import) settle identity through the same road with the same evidence ladder
  (P5); per-door divergence is limited to what evidence each door *has*, never
  to a different standard of proof. "Identity outcome" means the settled
  identity, its routes, and the badge — NOT the interaction path: a
  human-watching surface flagging where a machine door decides (P4) is
  permitted process divergence on top of identical identity outcomes. The
  authoritative door×evidence matrix is below. `Always` means every resolving
  invocation carries the bundle, `conditional` means both present and absent
  are legal paths, and `never` means the door may not synthesize or consume the
  bundle as identity evidence. "User choice" means an explicit identity-candidate
  pick OR an explicit manual title+author creation, not merely clicking
  Add/Review; file-carried ids remain file evidence; "minimum only" means
  REQ-002 title+author (plus non-identity context such as series position) with
  no route-bearing evidence. A minimum-only cell is present exactly when all
  route-bearing provider/file evidence cells for that invocation are absent.

| Door | User choice | Owned-file evidence | Provider identity evidence | Minimum only |
|------|-------------|---------------------|----------------------------|--------------|
| Direct add | Always | Never | Conditional (picked candidate) | Conditional |
| Manual import | Always | Always | Conditional (picked candidate) | Conditional |
| List import | Conditional (explicit candidate pick on review/add) | Never | Conditional (row id or picked candidate) | Conditional |
| Author monitor | Never | Never | Always (monitored provider work) | Never |
| Series monitor | Never | Never | Conditional (when the monitored entry carries one) | Conditional |
| Readarr import | Never | Always | Conditional (provider result reached from file ids) | Conditional |

  A conditional user choice outranks all other present bundles; file evidence
  outranks a provider result even when its ids later project as routes. When a
  door holds mixed evidence that disagrees, P5's order settles it: the user's
  choice beats the file, the file beats a provider id. A `never`-cell injection
  cannot affect identity and fails that door's gate; a minimum-only machine door
  may create the valid zero-route work or defer, but never invents evidence.
  The series row deliberately makes no environment claim about whether monitored
  entries carry provider ids: present ids are consumed, absence takes the
  minimum-only path.
  Every changed door's wiring gets its door-gate row (PA-009).
- **REQ-019**: Readarr import resolves identity. The Readarr door uses the
  evidence it holds through the shared road at import time, instead of creating
  identity-less works. File-evidence scope remains embedded identifiers
  (ISBN/ASIN) and the embedded cover — no filename/release-name matching (that
  primitive stays excluded per Q-005). Structured embedded title+author may
  supply REQ-002's minimum for a new work or a minimum-only review proposal, but
  cannot select an existing work or justify a machine attach to one.

  Branch precedence is binding. Valid embedded identifiers resolve first. If
  their provider results agree on one identity, the shared road settles that
  identity and persists every valid edition-id route even when embedded title or
  author is absent, invalid, or disagrees across the files. Conflicting identifier
  or provider identities instead flag review under REQ-007. Only when no valid
  identifier resolves does the embedded title+author minimum participate. With
  identifiers but no provider hit, readable, mutually agreeing files with a valid
  REQ-002 title+author create that minimum-founded work and persist all valid
  embedded edition-id routes, yielding a connected work; a provider outage has
  that identical result plus one idempotent convergence retry. In either arm,
  missing/invalid title or author, unreadable/malformed metadata, or disagreement
  on the proposed minimum flags review instead of creating a work. If an outage
  coincides with that flag, the convergence retry may settle a later identifier
  hit; otherwise the flag remains pending. With zero valid identifiers, readable,
  mutually agreeing files with a valid proposed title+author take REQ-018's
  conditional minimum-only arm and defer to import review without creating a work
  or attaching a file; missing/invalid minimum fields, unreadable/malformed
  metadata, or disagreement likewise flags review. A conflicting-identity flag
  remains unattached and follows REQ-007's accept/reject/different-work lifecycle.
  Every other flagged import remains unattached and exits through corrected
  metadata + retry, a later resolving identifier hit where applicable, an
  explicit identity choice on the import-review surface (including confirmation
  of title+author to create the zero-route P2 work and then attach any embedded
  cover), or cancellation.
- **REQ-020**: User confirmation is structural. A user-picked identity
  (`user_confirmed`, already stored durably) outranks every machine conclusion
  on every road — including cover selection and later refreshes — as the top of
  the evidence ladder, not as a per-feature patch.
- **REQ-021**: Import dedup defers to the watching human. On every
  human-watching surface enumerated in P4 — manual import, import review, and
  list-import's review/add surface — a "this book may already exist" detection
  flags the candidate to the user instead of silently discarding the user's
  pick or attaching the file elsewhere. Machine-alone doors (P4) decide via the
  authority or defer to review; they never guess silently.
- **REQ-022**: Sibling panel goes informational. The identity modal's sibling
  panel describes state without offering or threatening clearing actions that
  the routes model made obsolete (nothing is cleared by confirming). The
  panel's copy is not yet written — it is frozen with the PO at design; until
  then acceptance binds to the behavioral properties only.

### Authors riding (settled in §8 of the F1 notes)

- **REQ-023**: The work points to its author record. Works reference authors by
  record id; the per-book author-name copy is retired from storage, and every
  display path reads through the record. A work may carry multiple contributors
  (REQ-004); the PRIMARY author — used for referencing, uniqueness, tags,
  matching, and route inheritance, identically at every one of those seats —
  is the FIRST contributor in the stored order. That order is reconciled
  independently of provider array order: an existing primary stays first while
  its author record remains a contributor. Contributors whose author records
  are co-created in one settle operation are canonicalized BEFORE record ids
  can influence ordering, by this total tuple: normalized full author identity
  name, then sorted provider-route set, then sorted exact source-name set;
  incoming array position and unsampled role never participate. Evidence-identical
  occurrences that F1 resolves as one author collapse. If unresolved occurrences
  still tie on the full tuple, no arrival-order tiebreak is allowed. Their
  resolution must partition the tied occurrences into author identities and, if
  more than one distinct author remains, give their full order and primary. An
  interactive road may obtain that audited choice inline. A machine door instead
  parks a named contributor-order review card holding the source evidence and
  proposed contributor set: for a new work, no work, contributor, or co-created
  author record commits while the card is pending; for an existing work, its
  current contributors and primary remain unchanged and the proposal stays
  unattached. The card exits through the same audited partition/order choice,
  after which creation or update resumes, or through cancellation, which
  discards a new-work candidate and leaves an existing work unchanged. The
  co-created author records are then established in that resolved canonical
  order, so their stable ids cannot encode incoming array position. For a new work
  after this canonicalization, or when the prior primary no
  longer remains, the contributor with the lowest stable Livrarr author-record
  id is first; all others follow by that same order. Repeated
  occurrences of one author record collapse to one contributor while their
  sourced roles are retained. A provider reorder or a newly arriving lower id
  never displaces a surviving primary. If the primary is removed, the canonical
  successor is proposed and any resulting re-key collision completes REQ-024's
  review/merge lifecycle before the change commits; rejection retains the old
  contributor, while acceptance commits the successor. An operation that would
  leave zero contributors is rejected by REQ-002 and retains the existing
  primary. Role-based selection is
  forbidden until ST-006(a) is sampled AND a new spec amendment defines the
  role value domain, no-writer/equal-rank tiebreaks, migration, key stability,
  and acceptance changes; capture alone changes nothing. A work created with
  only REQ-002's minimum has that one author as primary. Written file tags carry
  the primary author record's chosen display name at write time; a display-pick
  change re-tags on the next write, never retroactively (Q-007). (D5-authors)
- **REQ-024**: Book uniqueness keys on the author record. The work-uniqueness
  key comprises the stable normalized REQ-003 identity-title tuple, the PRIMARY
  author's record id, and a stable text distinction. The author component is an
  id, not the author's mutable display name, so a rename can never change a
  book's identity key (D6-authors). The title component is never the
  edition-derived display subtitle: display provenance and every
  default-edition/monitoring transition are excluded. Tuple subtitle/volume may
  keep exact same-main siblings in distinct storage keys, but equality of those
  parts is never required and disagreement never vetoes matching: works sharing
  normalized main + primary author still reach the authority. An explicit user
  edit of any identity-title part re-keys through the same collision lifecycle
  below; a machine subtitle-projection change never re-keys.
  The text distinction has one common value until an audited different-from-all
  resolution, or a deterministic P8 classification after ST-006(e), establishes
  that two works with the same title tuple and primary author contain different
  texts. That outcome assigns each additional own-work anchor a durable
  non-common distinction unique across the user's library. The distinction never
  comes from subtitle wording, provider keywords, arrival order, or display
  state; its audited assignment is bound to that stable own-work anchor, it
  survives refreshes and identity-title edits, and it is retired only by an
  audited same-work merge. Thus an abridgement or other legitimately
  different text can coexist without weakening P1's own-work anchor.
  Additional contributors do not participate in the key — deliberate, matching
  the authority's primary-author comparison semantics. Re-key collision policy
  (Q-004, resolved), with "authority-certain" bound to named verdicts (PA-007):
  auto-merge iff `title_verdict = Same` AND `author_verdict = Agree` AND the
  pair's id evidence is not `WorkKeyContradiction` AND the pair carries no
  surviving audited different-from-all resolution; every other collision becomes
  a review card.

  Before EVERY create or re-key commits, reconciliation enumerates the complete
  title/author group: every other active work for that user with the same
  identity-title tuple and primary author, regardless of text distinction;
  audited archived losers remain retrievable but excluded as elsewhere. This
  group check runs even when the proposed key would not collide with a member's
  non-common distinction. Each active member is compared independently: REQ-011's
  whose-text classification runs first whenever applicable, using only
  ST-006(e), then the authority-certain predicate above runs for a same-text
  possibility. Before an automatic same-text merge, reconciliation also checks
  every pair in its proposed merge cohort — the candidate plus all such anchors —
  for a surviving audited different-from-all resolution. Human evidence outranks
  the later machine conclusion under P5: any such pair is excluded from the
  automatic branch, and the complete group proceeds to the group identity review
  card instead. The history stops surviving only when an audited same-work merge
  retires it.

  The resulting branches are total. With an empty group, a new work uses the
  common distinction and an existing work retains its bound distinction. With
  one or more authority-certain same-text anchors and no surviving
  different-from-all audit within that proposed merge cohort, the listed anchor
  with the lowest stable Livrarr work id is the survivor; every other such anchor
  and the candidate attach/merge into it under Q-004 and the merge policy below,
  and that survivor's distinction is retained. If that cohort contains a
  surviving audited different-from-all pair, commit pauses on the complete-group
  card. With no same-text anchor and every member established as different text by audited choice or
  deterministic post-ST-006(e) classification, the candidate commits as
  different-from-all, retaining its existing non-common distinction or receiving
  one new durable distinction. With no same-text anchor and ANY unresolved or
  non-authority-certain comparison, commit pauses on one group identity review
  card that lists every member, including each existing distinction and the
  evidence for its pairwise outcome.

  That card has exactly three outcome classes: attach/merge into a selected
  listed anchor; affirm different-from-all (the audited choice establishes the
  candidate differs from every listed anchor, then retains/assigns its durable
  distinction); or cancel. Attaching retains the selected anchor's distinction
  and never mints another, so repeated ingest of the same abridgement converges
  on its existing anchor. A machine door parks the card; an interactive road may
  resolve it inline. While pending, an edited work retains its prior key and a
  new candidate claims no key. Cancelling an edit retains that key; cancelling a
  create discards the pending candidate. Cutover safety: the rehearsal performs
  this same group-wide reconciliation, materializes unresolved group cards, and
  assigns resolved different-from-all distinctions FIRST; the new unique index
  installs only once zero collisions remain unresolved. Merge policy (binding; the
  exhaustive field table is a design deliverable bound to it): plural state
  unions losslessly (routes, editions, files; covers keep the rank winner,
  both retained); for singular fields a user-set value (including an explicit
  user-set absence) always beats a
  machine-set value; two conflicting USER-set singular values (two user
  covers, two user-edited titles) pause commit on a field-resolution review card
  — no auto-pick; the authority-certain same-work predicate remains binding. For
  machine-set singular values, equal values coalesce and a sole present value
  survives; two different present values also pause on a field-resolution card
  unless another requirement already defines a total derivation from preserved
  source state. Thus covers follow REQ-014 and the display subtitle is recomputed
  by REQ-003, while conflicting canonical pointers or per-format defaults never
  get an arbitrary provider winner. On a predicate-eligible pair, this card does
  not reopen identity: it exits when the user chooses the left value, right value,
  or absence where the field is optional, then the merge resumes. A group card
  never narrows to only the common-distinction pair: its attach/merge,
  different-from-all, and cancel outcomes remain bound to the complete listed
  group. Every outcome is audited and any loser is archived. Nothing is
  destroyed anywhere; per-media-type monitoring flags OR-combine.
- **REQ-025**: Author identity inherits from work identity. When a work's
  identity settles with a provider record that names its author by id, that id
  becomes an author route through F1's name-agreement guard — no independent
  author matcher on this path (D7-authors). Inheritance attaches to the
  PRIMARY author (REQ-023's rule). When the settled record carries NO author
  id (provider lacks them, or parse drift — ST-006(d)), no link is created and
  no fallback matcher runs on this path; the author keeps its existing routes,
  and F1's name-search road remains the only alternative. This is the
  settle-road seam F1 deliberately left to F2.

### Data migration

- **REQ-026**: Lossless, kind-correct, edition-homed cutover. Existing
  per-column provider ids become routes under this exact mapping: `ol_key` →
  OL work-id route; `hc_key` → HC work-id route; `gr_key` → GR **edition-id**
  route (ST-002 — the stored number is a book id; REQ-006 upgrades it on next
  touch, never the migration itself); `isbn_13` → ISBN edition-id; `asin` →
  ASIN edition-id. Every edition-scoped value gets an edition HOME (REQ-005 —
  routes are projections, never free-floating): values are grouped onto one
  edition only with proven co-edition evidence (one provider record or one owned
  file presented them together); otherwise each becomes its own provisional
  edition of the work. This is REQ-007's direct co-edition proof.
  A provisional edition starts with format `unknown` and language absent unless
  direct migrated payload/file evidence proves a value; ISBN and ASIN kind alone
  NEVER infer format. Unknown-format editions remain route homes but cannot be
  per-format defaults or fill per-format cover slots and sort last under
  REQ-003; a retained cover instead enters REQ-016's single work-scoped
  format-needed panel whenever a target slot is uncovered and remains retained
  without a panel while all slots are covered. Later direct evidence resolves
  them under REQ-005; conflicting evidence goes to review.
  Every legacy work also receives a total REQ-003 identity-title tuple. Its
  stored legacy title is parsed by REQ-003: tuple main is the parsed main;
  tuple subtitle is the stored nonblank legacy subtitle when present, else the
  parsed subtitle, else absent; tuple volume is the stored nonblank legacy
  volume when present, else the parsed volume, else absent. Blank or
  whitespace-only structured values are absent. Existing user provenance is
  preserved; a value
  without preserved provenance is marked as migrated, never invented as a user
  edit. If parsing leaves no valid main title, migration blocks that work on a
  repair review while retaining its original record and key; a user supplies a
  valid main title and retry exits the block. The new unique index cannot install
  while any such repair remains unresolved.
  Edition-derived subtitle projection is computed separately only after the
  edition homes exist, and its rendered fallback follows REQ-003; it never
  supplies or changes the migrated identity tuple. Repeating migration over the
  same legacy inputs produces the identical tuple and uniqueness key. A later
  projection change likewise cannot alter either. The cutover rehearsal applies
  REQ-024's same-text/different-work lifecycle and assigns every required stable
  text distinction before it tests the unique index, including legacy
  abridgements and other legitimate same-title/author different texts.
  No value is lost; badge states map per REQ-012's total rule; convergence
  scheduling survives; per-media-type monitoring flags
  (`monitor_ebook`/`monitor_audiobook`) are preserved independently across
  migration and any merge. The migration is rehearsed on a copy of the PO's
  real library before it runs live (snapshot-first, standing rule).

## 3. UI/Interface Design

Surfaces touched: identity badge states (REQ-012), cover source label + four
placeholder states (REQ-016/017: one work-scoped format-needed panel plus three
per-slot states), sibling panel copy (REQ-022, frozen at design),
import-review flag state (REQ-021). HTML mockups (house rule) to be produced at
design; none exist yet.

## 4. Non-Requirements

- No LLM anywhere in matching or selection (P9; standing).
- The four foreign-language GR dead ends (works 8/9/33/71) stay deferred to the
  foreign-language block (memory `project_foreign_gr_deadends`) unless the routes
  model trivially explains them — explaining is in scope, fixing is not.
- U8b `tier1_inherited` route repair: separate parked design round
  (`PARKED-U8b-route-repair.md`); not folded in.
- The paced provider half of the cover sweep ships on its own schedule (Q-006);
  this feature does not block on it.
- mp3-specific audiobook work: out (standing PO deprioritization).
- No new provider integrations; the routes model consumes providers we already
  speak to.
- Release-matching and file-matching primitives: out (Q-005). REQ-019's file
  evidence is embedded identifiers + embedded cover only — never filenames.
- Full contributor-role participation in uniqueness/matching: out. Contributors
  are stored and displayed (REQ-004/023); the key and the author bar stay
  primary-author (REQ-024). Revisit only with a PO decision.
- Embedded-cover extraction beyond EPUB (mobi/azw3/pdf/m4b): out for the local
  half; revisit with the provider-half unit (Q-006).

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Badge states + enrichment threshold (T1) | resolved | PO 2026-08-02: any declared route kind counts — work-id OR edition-id (ISBN/ASIN/GR book id) — enriches; zero-usable-route works get the honest not-connected state (or connected, if they hold an undeclared-kind route) + a manual title+author provider search; no automatic passes for them. (v8: REQ-027 adds the bounded machine search-fallback carve-out.) |
| Q-002 | Audio-first books with no work-concept provider | resolved | PO 2026-08-02: no special treatment. An ASIN route is a full connection; convergence quietly links text catalogs (and pulls their work metadata) when it finds the book there. |
| Q-003 | Abridgement / omnibus definitions | resolved | PO 2026-08-02: BOTH are their own works. Abridgement never merges with the unabridged text (supersedes D11's abridgement arm). Omnibus never merges with its parts; contains-pointers stored when supplied. |
| Q-004 | Re-key collision policy (O3) | resolved | PO 2026-08-02: authority-certain pairs auto-merge (predicate bound in REQ-024); everything else becomes a review card. Migration rehearsed on a snapshot first. |
| Q-005 | Release/file matching on the shared primitive? | resolved | PO 2026-08-02: later — own pass once the model is proven live. Stays a Non-Requirement here. |
| Q-006 | Provider-side cover sweep sequencing | resolved | PO 2026-08-02: standalone follow-up unit; this feature does not block on it. |
| Q-007 | Tag names after the per-book copy retires (O7) | resolved | PO 2026-08-02: tags carry the author record's chosen display name at write time; re-tag on next write, never retroactively. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): A work carrying three GR routes (the ST-002 trio) resolves any of the three arriving ids to the same work; a work with zero routes exists, displays, and supports every REQ-002 operation. Attaching a route already held by a different work of the same user produces a REQ-007 class-(c) flag and changes neither work — no silent reassignment.
- [ ] **AC-002** (REQ-002): Creating a work with only "title + author" succeeds through the real creation road, and each operation on REQ-002's usable list (display, edit, per-media-type monitoring, file attach, tag write, user/file cover, manual search) is exercised against it with no errors logged; automatic enrichment is asserted NOT to fire for it.
- [ ] **AC-003** (REQ-003): All four truth-table rows land exactly as specified — in particular the ST-005 payload lands as main "Einstein" + subtitle "His Life and His Universe" with no tail text remaining in main, and an embedded-only payload splits per the authority's parse. An edition's own subtitle survives distinct from the display projection; bare-vs-subtitled identity tuples with the same main/primary author still reach the authority and never mismatch merely on subtitle (ST-001). Projection fixtures prove: ebook+audiobook defaults select ebook; no ebook default selects audiobook; no defaults plus disagreeing providers follows the fixed source order; known format beats unknown; selected-edition projection absence stays absent rather than borrowing from another edition; and monitoring changes do nothing. With an absent projection, automatic display renders the identity-tuple subtitle and the ST-005 work therefore shows "His Life and His Universe" even with no edition; if both projection and tuple subtitle are absent, it renders absent. Each committed candidate-edition, default, format, source-order, and selected-edition subtitle value/provenance change recomputes projection and provenance; parked changes do neither. An explicit user subtitle value/absence renders its choice and updates the tuple through REQ-024, while reset restores automatic projection-or-tuple fallback without changing the tuple. Every machine-only recomputation leaves the identity tuple, uniqueness key, and merge state byte-for-byte unchanged.
- [ ] **AC-004** (REQ-004/005): An edition's ISBN/ASIN/GR-book-id live on the edition record and resolve through it to its one work; HC-shaped fields, contributors-with-roles, and subjects round-trip from a provider payload carrying them and stay absent otherwise; provider-supplied containment round-trips in both directions, including an unresolved pointer to a work not in the library. A provisional id with no direct format/language evidence remains format-unknown/language-absent, supplies routes but no format default/cover slot, and later direct evidence resolves it; an attached cover remains retained and, while any target slot is uncovered, appears in REQ-016's single work-scoped format-needed panel before entering only the resolved format's rank after direct evidence or user assignment. Contradictory evidence parks review without losing lookup behavior.
- [ ] **AC-005** (REQ-006): Each REQ-006 trigger (enrichment pass, manual refresh, convergence visit) on a book-id-only work records the GR work-id route with no extra request, idempotently (a second trigger changes nothing); page-unavailable and parse-drift runs change nothing and leave the normal retry cadence intact; a later successful fetch completes the upgrade.
- [ ] **AC-006** (REQ-007): Injecting each conflict class — (a) same-provider work-id disagreement, (b) cross-provider work-key disagreement, (c) cross-work route collision — produces a review flag; existing routes are untouched and matching proceeds on uncontested evidence while pending; each resolution (accept / reject / different-work) leaves an audit trail with the expected surviving route set — including accept DISPLACING a wrong existing route into the audited archive, after which it is excluded from matching and the same conflict cannot re-trigger. Class-(c) fixtures cover both route kinds. Moving a work id changes only the chosen work-route ownership. Moving an ISBN off Work A's edition onto Work B moves only that id into the sole edition carrying a non-contested co-edition id from the parked payload, or into a provisional edition when none qualifies: A's sibling ASIN/subtitle/cover remain on A, the former id attachment is archived, exactly B projects the ISBN, and no intermediate state dual-writes it. Title/subtitle/format/language/cover similarity alone qualifies no target; several co-edition-qualified targets stay pending; an explicit user target wins; a separate no-sibling fixture archives the emptied source shell. An injected atomic failure leaves the original graph intact. Injecting additional distinct edition-id routes on one work produces NO flag.
- [ ] **AC-007** (REQ-008/009): Named fixtures per direction: bare-vs-subtitled same-main (ST-001 shape) MUST match — the lost-match set; "Dune" vs "Dune Messiah" MUST NOT — the wrong-merge set. Adjusting the lost-match direction's guards moves only lost-match fixtures (the wrong-merge set stays pinned) and vice versa; neither direction's guards can alter edition-id behavior (REQ-009 isolation). An automated check enumerates edition-id comparison sites tree-wide and fails on any outside the authority's confirm arm + the migration mapping.
- [ ] **AC-008** (REQ-010): V5, V7, and V8 (PA-003 citations) no longer contain private title-sameness recipes — their behavior is pinned by tests that route through the authority's primitives; V6's `should_reject_cover` substring list is asserted DELETED along with the trust-grade selection it served (REQ-014).
- [ ] **AC-009** (REQ-011): Pre-ST-006(e): every classification-requiring entry lands in review — no exceptions, including one whose normalized main-title/primary-author key collides. Post-capture, the classification arms bind: a study-guide entry for a held work lands distinct (or review), never merged; a translation lands as the same work; an abridgement lands as its own work, never merged with the unabridged text; an omnibus never merges with any contained part; an entry with no recognized sampled signal still lands in review — and every non-review classification in the suite cites the ST-006(e) sampled signal it keyed on. Changing display subtitles never changes any of those outcomes or the stable distinction that keeps different-text works representable.
- [ ] **AC-010** (REQ-012/013): A zero-route work shows not-connected and is excluded from automatic enrichment without accumulating retry outcomes; adding one route of any declared kind makes it enrichable on the next pass. A work holding ONLY an undeclared-kind route shows connected, is not auto-enriched, and offers the manual search. ASIN-only lifecycle: the work connects, enriches from audio providers, convergence later discovers the text-catalog work → work-id route + text-provider metadata appear; a convergence miss leaves it connected with no retry storm.
- [ ] **AC-011** (REQ-014): With a user cover set, no refresh ever replaces it; with an embedded file cover present and no user cover, the file cover beats any provider cover; removal of a higher-rank source falls back down the rank, never below the best available. A slot's non-default fallback considers only editions of that same known format: an unknown- or different-format cover never fills it. Both format slots select independently; an audiobook slot without its own cover shows the ebook cover WITH the fallback label, and a real audiobook cover arriving replaces the fallback.
- [ ] **AC-012** (REQ-015): Against sanitized fixtures — an EPUB with an embedded cover, an EPUB without, and a malformed/encrypted file — extraction yields: the cover; a clean no-cover absence; and a distinct could-not-inspect outcome tied to the observed file revision (logged, retriable, NOT recorded as no-cover) — with zero provider requests on the path. An unchanged revision is skipped by background passes; repairing/replacing it triggers one inspection that reaches extracted or verified-no-cover, and an explicit retry repeats inspection even without a revision change. File removal exits to file-gone/fallback. Every transition makes zero provider requests. (The run against the PO's real library is a code-stage prototype gate for ST-007, recorded separately — it is not this AC.)
- [ ] **AC-013** (REQ-016): The four placeholder states are reachable and distinct. With both slots uncovered and one unknown-format cover, exactly one work-scoped cover-found-format-needed panel renders—not one per slot—and both slots observe the result of each shared action. It lists all unknown-format covers in stable order and offers choose-one assign-format/wait-for-direct-evidence/reject plus set-cover-for-a-chosen-uncovered-slot. Assigning/resolving format reruns every slot; rejecting one of several candidates leaves the one panel with the remainder, rejecting the last removes it, and filling all slots removes it without discarding retained candidates. With usable routes and that panel absent, each uncovered slot independently reaches searching (wait/set-cover) or no-cover-found (provider re-ask). A zero-usable-route work with an unknown-format cover renders both the one work panel and nowhere-to-look/manual-provider-search for each uncovered slot; a successful search creates a route, exits nowhere-to-look, and starts enrichment/cover lookup without discarding the unknown cover, so the panel remains until assign, direct resolution, reject, set-cover, or all-slots-covered removes it. The same manual action is available without the panel. A zero-usable-route work never shows eternal "searching."
- [ ] **AC-014** (REQ-017): No UI surface or API response carries the validated/unvalidated trust label; covers display their source; existing stored trust grades are migrated/retired.
- [ ] **AC-015** (REQ-018): The door-gate table has a passing row per changed door; a spot-injected off-road spawn in any door fails its gate test. Every authoritative matrix cell is exercised: `Always` must be present, each `conditional` cell has present+absent fixtures, and a `never` injection cannot affect identity and fails the door gate. Identical evidence bundles produce identical identity outcomes (settled identity + routes + badge — P4 flag-vs-decide process divergence permitted). Mixed-evidence fixtures cover every matrix-permitted seat: user+file (manual import), user+provider (direct add/manual import/list review), and file+provider (manual/Readarr), and resolve per P5. Direct-add and manual-import fixtures without route-bearing provider/file evidence take their conditional minimum-only road and create the valid P2 work; readable identifier-free Readarr takes its conditional minimum-only road and defers to import review without creating a work. Series-monitor fixtures with provider ids consume them; without ids they take the minimum-only road and create a valid zero-route work without invented evidence.
- [ ] **AC-016** (REQ-019): Readarr fixtures exhaust the precedence-ordered file-identity branches. Valid ISBN+ASIN with one agreeing provider identity settles through the shared road with every edition-id route even when embedded title/author is absent, invalid, or disagrees; conflicting identifier/provider identities flag REQ-007 review and exercise each of its audited exits. The same ids plus no hit and a valid agreeing REQ-002 minimum create the connected minimum-founded routed work; outage produces that identical persisted state plus one idempotent convergence retry. With no hit, missing/invalid minimum fields, unreadable/malformed metadata, or a disputed proposed minimum instead creates one unattached review flag and no work; the outage version also retains one retry whose later agreeing hit settles and clears the flag. Readable agreeing metadata with valid proposed title+author, no ids, and an embedded cover creates one unattached import-review flag and no work, while changing only those title/author strings never selects or rejects a match candidate. Confirming title+author on that review creates one not-connected P2 work, attaches the cover, performs no automatic enrichment, and offers manual search. Every non-conflict flagged import attaches no file before resolution and exits through corrected retry, later resolving identifier hit where applicable, explicit import-review identity/create choice, or cancellation. The identity-less-work path is gone.
- [ ] **AC-017** (REQ-020): A user-confirmed identity survives refresh, convergence, and cover re-selection unchanged on every road (not only the two doors that honored it before).
- [ ] **AC-018** (REQ-021): On each human-watching surface (manual import, import review, list-import review/add), a dedup hit surfaces a flagged choice; a file or row is never attached to a different work than the user picked without the user seeing it.
- [ ] **AC-019** (REQ-022): The sibling panel offers no clearing/threatening actions and renders informational state; the frozen-at-design copy, once provided, is pinned by a content assertion added at that point.
- [ ] **AC-020** (REQ-023/024): Renaming an author's display name changes zero works' identity keys and breaks zero uniqueness constraints; the per-book name copy is gone from storage and all display paths read through the record; after a display-pick change, no file is retagged until its next write, and that next write carries the new name. Reversing provider arrays for two co-created contributors produces the same pre-id canonical order, author-record ids, stored order, and primary on a new work; normalized identity name, provider routes, and exact source names break the cases in sequence. For an unresolved full-tuple tie, the interactive road's audited partition+total-order choice creates stable records; a machine road parks a contributor-order card and commits no new work/author/contributor record until the same choice resolves it, while cancellation discards the candidate. On an existing work the card leaves its contributors/primary unchanged until resolution or cancellation. Later provider reorders and lower-id arrivals preserve an existing primary. Removing that primary proposes the canonical successor and completes the REQ-024 collision lifecycle before commit; removing the sole contributor is rejected with the old primary intact. Merely capturing ST-006(a) changes no primary. The same first stored contributor is asserted at every primary-consuming seat: reference, key, tag write, matching, and inheritance.
- [ ] **AC-021** (REQ-024/Q-004): The snapshot rehearsal groups every active work by full identity tuple+primary author regardless of text distinction, reports every unresolved group, assigns resolved different-from-all distinctions before the unique-index test, and installs no index while an identity or field card remains unresolved; zero silent merges. Runtime group fixtures begin with an unabridged common anchor and an abridged non-common anchor: before ST-006(e), a second ingest of that same abridgement produces one card enumerating both, an audited attach-to-abridged outcome retains its distinction and creates no third work, and repeating that ingest+outcome leaves the active work/key/distinction graph byte-stable apart from its audit entry. A post-capture candidate that reads authority-certain same-text against both of two anchors whose audited different-from-all relation still survives never auto-merges either anchor: one card enumerates the complete group, and attach-to-one, affirm-different-from-all, and cancel retain the unselected anchor and its distinction. A separate post-capture fixture whose entire proposed merge cohort has sampled same-text/authority evidence and no surviving different-from-all pair converges automatically. A genuinely third text established different from every listed anchor receives exactly one new durable distinction. With multiple same-text anchors and no surviving different-from-all pair in their proposed merge cohort, the listed anchor with the lowest stable Livrarr work id survives and all plural state merges losslessly. Archived losers are retrievable but never re-enter the group. With no authority-certain same anchor and an unresolved comparison, one card lists the complete group and offers only attach/merge to a selected anchor, audited different-from-all, or cancel; edit-cancel retains the prior key and create-cancel discards the candidate. Refresh, display-projection change, and later identity-title edit preserve a surviving distinction until audited same-work merge. Merge fixtures also prove: a user-set singular beats a machine-set one; a user-vs-user singular conflict pauses field resolution without reopening identity; equal machine values and present-vs-absent merge deterministically; conflicting machine canonical pointers and per-format defaults pause likewise; covers and display subtitle recompute under REQ-014/003. Choosing left/right/allowed-absence resumes the authority-certain merge. All outcomes are audited and archived losers remain retrievable.
- [ ] **AC-022** (REQ-025): When a work settles against a provider record naming its author by id, the PRIMARY author gains that route iff F1's name-agreement guard passes; a guard failure produces the F1 review surface entry, not a link; a settled record with NO author id produces no link, no fallback matching, and no error.
- [ ] **AC-023** (REQ-026): Post-migration, every pre-existing provider id is present as a route OF THE MAPPED KIND (`gr_key` asserted an edition-id route, `ol_key`/`hc_key` work-id, `isbn_13`/`asin` edition-id), and every edition-scoped value is OWNED by an edition row linked to its work — grouped only where co-edition evidence existed, else separate provisional editions. An ISBN-only and an ASIN-only row with no direct format evidence both migrate as format-unknown/language-absent (no inference by id kind), project their routes, supply no per-format default/cover, and sort last for display; an attached cover is retained and appears in the single work-scoped format-needed panel while a target slot is uncovered, later direct evidence resolves format and reruns every slot, while contradictory evidence creates the REQ-005 review state. Legacy-title fixtures with structured subtitle present/absent produce REQ-003's exact identity tuple and preserve provenance; display projection is computed separately. A blank/invalid main blocks index installation with the original record/key intact until user repair and successful retry. Two rehearsals over identical inputs produce byte-identical tuples, text distinctions, and uniqueness keys; changing only an edition subtitle projection changes none of them. Badge states match REQ-012's rule for every legacy state × route combination (incl. the setter=user → user-confirmed-with-route invariant); convergence schedules are preserved; each of the four per-media-type monitoring combinations survives unchanged; the rehearsal ran on a snapshot first and its report is on record.
- [ ] **AC-024** (REQ-027): Against the activated schema through the real convergence tick: (a) an ASIN-only bridge work whose scripted OpenLibrary search response carries its ASIN in `id_amazon` ends one tick with an active `OpenLibraryWork` route, a settlement audit naming search-fallback and the corroborating kind, generation advanced, and ZERO ledger burn; (b) the same shape whose response omits the work's ids (and includes a DIFFERENT ASIN, pinning no-veto) with a NEAR-MISS pick (author verdict Abstain — v9) writes no route, mints exactly one `PendingRoute` card carrying the proposed route, and burns one attempt — a second tick reuses that card (still one) and burns again, and at threshold the work stops being selected; (c) a not-connected title+author-only work is cadence-selected, a near-miss Goodreads autocomplete pick (`workId` taken from the response, no extra request for it; author Abstain — v9) mints the card, and affirming through the real resolve continuation settles the `GoodreadsWork` route and flips the badge to connected; (d) a work holding any work-level route fires zero search requests, and a foreign-language work fires only the Goodreads leg; (e) a Goodreads probe corroboration (book page carrying the work's ASIN) settles both the work route and the edition-homed book-id evidence, with no request issued solely to re-learn an id already present in a prior response.
- [ ] **AC-025** (REQ-027 v9 + REQ-014 containment): (a) auto-link — an ASIN-only work whose single scripted search candidate is Same-title + author-Agree with NO corroborating id settles the work route machine-alone with a text-decisive audit/provenance distinct from corroborated, generation advanced, no card, no burn; (b) the boundary holds — author-Abstain cards, and two candidates proposing DIFFERENT work ids at Same+Agree card, and the decision function lives in the matching authority; (c) sibling-card staleness dead — a work with TWO pending route cards affirms one through the real door (route settles, generation moves), then affirms the second successfully; listing after the first shows the current actionable generation; (d) satisfied cancellation — a settlement activating a route proposed by a pending card cancels that card in the same transaction, and affirming an already-active proposal is a success no-op while a route now owned by a DIFFERENT work returns the specific 409; (e) the parked-ledger heal — marker-gated, deletes machine-chase attempts ONLY for works with no active work-level route, exact counts, second run zero; (f) cover containment — Goodreads absent from cover candidate assembly; the one-shot heal re-selects every machine-selected GR-sourced ebook/audiobook cover slot (manual covers byte-untouched), falling to the next rank or the honest placeholder, and forces re-materialization of changed slots.
- [ ] **AC-026** (REQ-027 v11 per-provider eligibility): against the activated
  schema through the real convergence tick: (a) a connected work holding ONLY
  an active `GoodreadsWork` route (the work-74 shape) is cadence-selected and
  fires OpenLibrary and Hardcover search legs; a scripted OL Same+Agree
  candidate auto-links `OpenLibraryWork` ALONGSIDE the existing Goodreads
  route, generation advanced, no burn; (b) a provider holding an active
  work-level route on the work fires ZERO search HTTP of its own, pinned for
  all three search-capable providers; (c) a pass whose fired legs all miss
  burns exactly one attempt on the ONE per-work ledger, and at threshold the
  connected work leaves automatic selection until its identity generation
  changes; (d) applicability holds — a foreign connected work fires only the
  Goodreads leg.
