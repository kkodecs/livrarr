# Findings — identity sidebar session, 2026-07-25

Origin: a forked side-session ("identity sidebar", herdr tab 2) spun off the main
alpha-testing session to work item 1 and item 2 to the bottom without polluting
the main context. Read-only session — nothing was changed; this file is its only
write, PO-authorized.

**Full reasoning transcript:**
`~/.claude/projects/-mnt-opt-livrarr/83f71bd9-5365-4550-b56c-e4b237a97773.jsonl`
(3.0 MB, forked from session `15fa3f3a-b33d-41a8-9d79-f39e9e8ca8a6` via
`claude --resume … --fork-session`.)

This file records the **conclusions**. The transcript records the **arguments** —
why Rule A cannot work, the three-way confirmation that the subtitle is
edition-level, the provider ontology queries and their raw responses, the
door-by-door evidence table, and two claims of mine that were wrong and got
corrected mid-session. If any finding here is challenged and the citation given is
not enough, the derivation is in the transcript.

**Audience: the main session.** It has everything up to the fork point (items 1–9,
`docs/design-subtitle-matching.md` r2, both r1 reviews). It has none of this.

---

## The five deltas — read this first

1. **The bug fix shrinks to five contained changes.** Nothing in it gets undone later.
2. **Everything else is a rewrite of the identity layer.** A real project, now scoped.
3. **Three corrections to what the main session currently believes:**
   - Goodreads *does* have a work id (I said it didn't)
   - `user_confirmed` already exists and works — item 9 is far cheaper than quoted
   - The cover gate **deletes**; it isn't tuned
4. **The subtitle was never wrong.** OpenLibrary's *work* title for work 136 is
   literally `"Einstein"`. The library's title came from the work-level truth, and
   the rule has been rejecting the authoritative answer as a mismatch.
5. **Six consistency tensions** must be carried into the rewrite (Part 3).

**Action for the main session:** re-scope `docs/design-subtitle-matching.md` r2 → r3
(snapshot r2 to `docs/design-history/design-subtitle-matching-r2.md` first, per
`CLAUDE.md`), and open a separate brief for the deferred work. r2 is now much larger
than the bug fix should be.

---

## Part 1 — Verified facts (external sources, this session)

**F1. The subtitle is edition-level, not work-level. Three independent confirmations.**
- Hardcover's canonical short handle for the work is `einstein` (`slug`)
- Hardcover stores `subtitle` as its own field, separate from `title`
- **OpenLibrary's work record `/works/OL4288870W.json` has `title: "Einstein"`** — no
  subtitle field at work level at all

**F2. Goodreads has a work id, and it costs nothing extra.**
`Work:kca://work/amzn1.gr.work.v1...` with `legacyId: 985244`, plus work-level
`originalTitle` and a `/work/editions/985244` list. It sits in the same
`__NEXT_DATA__` blob already being fetched for the book page.
*Correction: I told the PO earlier that Goodreads had no work id. That was wrong.*

**F3. The Goodreads number used as a work anchor today is an EDITION id.**
Book ids 10884, 2059858 and 6602781 all resolve to **work 985244** — verified by
fetching all three. Hardcover's `book_mappings` independently list two *verified*
Goodreads ids for the same work. So a GR book id can never answer "same work?":
two copies of one book carry different numbers by design.

**F4. ISBN and ASIN are edition-shaped.** A shared one proves sameness; a different
one proves nothing (a work has dozens). They may confirm a match, never block one.

**F5. Provider ontologies differ in what they model at all.**

| Provider | Work concept | Strength | Weakness |
|---|---|---|---|
| Hardcover | ✅ `book` = work (Einstein: 33 editions, 4 languages, read + listened) | Best shape: `compilation`, `parent_book_id`, `alternative_titles`, `canonical_id`, `default_{physical,ebook,audio,cover}_edition` | Thin coverage (PO: "right shape, not enough data") |
| Goodreads | ✅ work id + editions list | Best coverage | Work object otherwise mostly stats |
| OpenLibrary | ✅ clean `/works/…W` vs `/books/…M` | Authors as an **array with roles**; rich `subject_people`/`_places`/`_times` | Thinnest work model; 10 unranked covers |
| Google Books | ❌ volumes only | — | No work concept |
| Audible / Audnexus | ❌ one production per ASIN | — | No work concept |

**F6. `user_confirmed` already exists and is honored.** A user-confirmed seed carrying
a work anchor is trusted directly — no fan-out, no title verification, comment reads
*"The user's pick is the identity vote."* Two doors set it (manual import,
add-from-search), and it is durably recorded as the anchor's `setter = user`.
**Item 9 is therefore much cheaper than the "needs cross-crate plumbing" estimate
given in the main session.** The fact is stored; the cover gate simply never asked.

**F7. Six creation doors, six different policies.** Only two use the human's judgment.
Evidence held does not predict behavior:

| Door | Evidence available | Uses it? |
|---|---|---|
| Direct Add | Picked candidate: all provider ids | ✅ trusted outright |
| Manual Import | Picked candidate **+ the file** (embedded ISBN/ASIN/language) | ✅ both; code calls the file *"the richest seed"* |
| List Import | CSV row: Goodreads book id, ISBN | ⚠️ used, but deliberately re-verified |
| Author Monitor | One OL work key | ⚠️ stamped Confirmed with **no** verification |
| Series Monitor | Title + series position | — nothing to use |
| **Readarr Import** | **ISBN + ASIN + the actual files** | ❌ **none.** No identity resolution, never opens the files |

Also found: manual import runs a silent machine dedup *before* using the user's pick —
if it decides the book already exists, the pick is discarded and the file attaches
elsewhere, unasked. That dedup does route through the shared authority (not ad-hoc),
and its "close but not identical → don't absorb, create a duplicate instead" arm is a
deliberate documented choice. **No duplicate-work damage observed in the PO's library**
(only Dune / Dune Messiah, a legitimate pair) — predicted, not observed.

**F8. Nothing extracts the file's embedded cover today.** Rank 2 of the cover order
(Part 2, D13) is a **new capability**, not a re-ranking.

**F9. Rule A has never fired.** 50 `OneSidedSubtitle` declines in the whole log, zero
rescues. Structurally incapable of helping the case it exists for.

**F10. Rule B can barely fire.** No provider reports another provider's work key —
verified across all of them (OL and GB explicitly null the other slots). Its only live
site is a fresh identity attaching to an existing work.

---

## Part 2 — Design decisions taken (PO-approved this session)

### Identity

- **D1.** Two primitives: *create a work*, and *is this the same work?*
- **D2.** **Our own work id is the anchor.** Provider ids are **routes** — plural,
  optional. Zero routes is valid. Three Goodreads numbers is valid. This dissolves F3:
  you keep all of them, and any one arriving means "this is that book."
- **D3.** Minimal to create a work: **main title + at least one author.** Everything
  else optional, with a placeholder. A book no provider has catalogued is still a book.
- **D4.** Hybrid shape: **Hardcover's skeleton**, **OpenLibrary's** authors-with-roles
  and subjects, **Goodreads'** work id and editions.
- **D5.** Store subtitle separately. *Caution: providers are inconsistent — Hardcover's
  Einstein carries the subtitle both inside `title` and in `subtitle`, spelled
  differently ("His Life and Universe" vs "His Life and His Universe"). Still needs
  normalizing; don't trust the provider's split blindly.*
- **D6.** **Delete Rule A** — "a near-title match needs a corroborating ISBN/ASIN."
- **D7.** **Rule B: keep the detection, drop the silent veto.** Route contradictions to
  review. Today a wrong id protects itself forever — it rejects every correct match
  that disagrees with it.
- **D8.** Two failure directions need **different tools**: *same book, different clothes*
  (too strict → lost matches) versus *different book, same clothes* (too loose → wrong
  data). One threshold cannot serve both; that is why every past adjustment traded one
  failure for the other.
- **D9.** Human watching → **flag, never filter.** Machine alone → decide or defer,
  never guess silently. (Bulk add and import review are *human watching* — the PO's
  call; the machine's job there is to draw the eye, not to hide candidates.)
- **D10.** Evidence ladder: **your choice > the file > a provider id > nothing.**
- **D11.** *"Whose text is this?"* separates study guides and adaptations (different
  work) from translations and abridgements (same work) — no fuzzy logic needed.

### Covers

- **D12.** **The cover gate deletes.** It was an identity mechanism in cover clothing,
  built to serve the since-abandoned "show a cover before identity settles" goal.
  Nothing replaces it — rank plus routes covers the whole job.
- **D13.** Rank (accuracy over resolution): **your choice → your file's embedded cover
  → the work's default edition for that format → any other edition → placeholder.**
  Never downgrade. Note the file's own cover outranks every provider: it is the only one
  guaranteed to be the book in your hand.
- **D14.** Two placeholders, **work-level**: *Searching for a cover* / *No cover found*.
  Must not be permanent (retry + a way to force a re-ask), and must be actionable
  (the user is rank zero).
- **D15.** Replace the UI's *"Trust: validated/unvalidated"* with **source** —
  "validated" only ever meant *the gate passed it*. Source is already displayed below
  it; add "yours" for uploads.
- **D16.** Per-format was already decided correctly and needs no change: both slots
  always show, and the audiobook slot **falls back to the ebook cover** with an
  explicit label.

---

## Part 3 — Six consistency tensions for the rewrite

- **T1. The badge's meaning breaks.** "Confirmed" currently means a provider work id
  exists. Under D2/D3 a work with zero routes is valid. The badge becomes about
  *connectedness*, not identity — and the "no metadata until identity settles" rule
  becomes wrong (an ISBN route is enough to enrich). **Needs an explicit decision, or
  the new model inherits the old gate and starves valid books.**
- **T2. Item 7's justification changes** from "these 17 books are broken" to "these 17
  books are under-connected." Still worth doing; no longer a correctness bug.
- **T3. ISBN looks both useless and essential — it isn't a contradiction.** As a
  **lookup key** it is excellent. As a **comparison** it is worthless.
  **Write this down explicitly or someone will helpfully restore Rule A.**
- **T4. Item 7 collects the wrong Goodreads number** — book ids, not work ids. Not
  wasted (a book id is a valid route and reaches the work id free on the same page)
  but incomplete. The rewrite revisits those works.
- **T5. D13's rank 2 does not exist yet** (F8). The cover rank cannot be fully
  implemented until embedded-cover extraction is built.
- **T6. "Searching for a cover" needs a third exit** — a work with zero routes has
  nowhere left to look. Distinct from both a provider outage and a genuine miss.

---

## Part 4 — The split

**Test applied:** does it stand alone without the new model, *and* would the rewrite
have to undo it?

### (a) Fix now — five changes

| # | Change | Scope | Why it is safe |
|---|---|---|---|
| 1 | **Item 4** — a refusal is not "not found". Goodreads conforms to the house standard; the breaker learns from 4xx across **all** providers | Provider layer | Model-independent, and the rewrite *needs* it: D14 depends on the same distinction |
| 2 | **Item 5** — log the preview path | Provider layer | Model-independent; makes the rewrite debuggable |
| 3 | **Item 7** — clear the 17 `gr_work` dead ends + reset `next_convergence_at` | Data repair | Every route gained now is inherited |
| 4 | **Delete Rule A** | ~4 lines | The rewrite removes the comparison entirely, so this can only be made moot, never undone. Difference between 17 stuck books and none |
| 5 | **Delete the cover gate** — **PO-approved this session** | ~10 lines | Goodreads is already first in the existing ebook cover order, so deleting the gate yields the right covers with **zero new machinery** |

**Accepted tradeoff on #5, explicitly:** with the gate gone and routes not yet built, a
wrong Goodreads id yields a wrong cover with nothing to catch it. Mitigated by #4
shipping alongside (ids more likely correct) and by the user's override always winning.
**The PO accepted this on the record.**

**Free rider:** item 3 (the modal stops threatening to clear the user's Open Library id)
comes with #4 — same rule. Needs only a test pinning it.

**Also from r2, unchanged and still in scope:** the cross-provider survey behind #1, and
the new S2c (4xx feeds the breaker in every client).

### (b) Deferred to the identity-layer rewrite

- **Item 1 in full** — routes model, our own work id, schema, borrowed shape
  (D1–D5, D8–D11)
- **Item 2 in full** — rank, placeholders, trust-label removal (D13–D15)
- **Item 6** — the sibling panel going informational (depends on routes: if nothing is
  ever cleared, the three cause words describe an outcome that no longer happens)
- **Item 8** — both halves. **Local half first** (extract covers from owned files — no
  provider traffic, no pacing problem, no breaker risk), then the paced provider half
- **Item 9** — confirmation-wins becomes structural rather than a patch (and see F6:
  the fact is already stored)
- **New capabilities** — embedded cover extraction, Goodreads work-id capture,
  multi-route storage
- **T1** — what the badge means and what it gates

### Why it is a rewrite, not a fix

Provider ids go from one column each to many-per-provider (new table). Editions become
first-class. New work fields (subtitle, alternative titles, compilation, parent work,
canonical pointer, default edition per format). Badge semantics change, and the
enrichment gate with them. Two new capabilities. The matching rules are rewritten, not
tuned. **All six creation doors change behavior.**

### Still open, non-blocking

- Item 6's two sentences of copy; whether to show the panel when nothing is at risk
- Item 8's restart-vs-self-retry — now only affects the provider half
- Audio-first books cannot get a work id from Audible or Audnexus (F5): exception, or
  unlinked until a text provider is reached?
- Definitional: is an abridgement the same work? Is an omnibus a work or a container?
- Are release-matching and file-matching in scope for the shared primitive?
- The four foreign-language dead ends — already recorded for the next foreign-language
  block (memory `project_foreign_gr_deadends`)

---

## Confidence and what would flip it

**High confidence (fetched and verified this session):** F1, F2, F3, F5 — all read
directly from Hardcover's GraphQL schema, Goodreads' page data across three editions,
and OpenLibrary's work JSON.

**High confidence (read at source):** F6, F7, F8, F9, F10.

**Not verified:** whether Hardcover's `book` is *formally* their work-level entity in
their own documentation — inferred from 33 editions spanning languages and formats,
which is strong but is inference. Whether any component resolves identity for Readarr
imports *after* creation (the background convergence job will, but by then the file and
the ISBN are out of reach).

### Unverified, and it matters for scope — the eight non-conforming matchers

There is a **dated audit** (2026-07-14, PO-ordered) enumerating **eight sites** that
violate the locked Phase 5 matching spec, named V1–V8 with locations — not a rumor.
Recorded in memory `project_matching_conformance_gaps`.

**I verified none of them this session.** The one matcher I did read —
`work_dedup::find_matching_work`, reached from manual import's `find_existing_work` —
routes cleanly through the shared authority, but **it is not on the V-list.** V4 is a
*different* function in the same file. So: eight named sites, zero re-verified here,
and the audit's line numbers are 11 days old and may have drifted.

**Two consequences.**

**(a) The rewrite's scope is larger than Part 4 implies.** D8–D11 say the matching rules
are rewritten rather than tuned. If eight sites each carry their own recipe, the rewrite
has eight call sites to converge, not one authority to edit. Re-verify the V-list before
sizing the rewrite.

**(b) It corrects the risk framing on change #4 — read this before shipping it.**
I told the PO that relaxing the subtitle rule *introduces* a sequel/omnibus
false-positive risk, using "Dune" versus "Dune Messiah" as the example. Per the audit,
**V1 (`flm_title`/`flm_match`, the settle-road auto-confirm gate) is runtime-proven to
already return `true` for exactly that pair.** If that still holds, the risk is not
introduced by change #4 — **it is already present today from a different cause**, on a
gate that fires on every add, refresh and convergence.

That does not make change #4 riskier. It makes the *existing* state worse than I
described, and it means the omnibus danger needs its own fix regardless of what happens
to Rule A. **Verify V1 against the current tree before quoting either claim.**

**What would flip the split:** if Rule A's deletion admits a bad match class we did not
predict, #4 becomes riskier than stated and the omnibus risk needs the year guard the PO
rejected as "edition shaped." The V1 finding above is the first place to look.
