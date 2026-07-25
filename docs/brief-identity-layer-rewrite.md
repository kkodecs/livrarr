# Brief — identity-layer rewrite (scoping, not a design)

Status: **scoped, not started.** No PO go. Date opened: 2026-07-25.

This is a holding document so the bug fix (`docs/design-subtitle-matching.md` r3) can
say "not here" and point somewhere real. It records what the work is, why it is a
rewrite rather than a fix, and what must be re-verified before anyone sizes it.

**Source of record:** `build/findings-identity-sidebar-2026-07-25.md` — the
identity-sidebar session, which produced the decisions below with the PO in the room.
Full reasoning transcript at
`~/.claude/projects/-mnt-opt-livrarr/83f71bd9-5365-4550-b56c-e4b237a97773.jsonl`.

## The core move

**Our own work id becomes the anchor. Provider ids become routes** — plural, optional,
zero is valid.

This dissolves the defect underneath every symptom the PO hit in testing: the Goodreads
number stored as a work anchor is an **edition** id. Book ids 10884, 2059858 and 6602781
all resolve to Goodreads work 985244 (fetched and verified). Under routes you keep all
three, and any one arriving means "this is that book" — instead of one column that can
only hold one of them and can never answer "same work?".

Minimum to create a work: **main title + at least one author.** A book no provider has
catalogued is still a book.

## Why a rewrite and not a fix

- Provider ids go from one column each to many-per-provider — a new table.
- Editions become first-class.
- New work fields: subtitle, alternative titles, compilation, parent work, canonical
  pointer, default edition per format.
- The `Confirmed` badge changes meaning, and the "no metadata until identity settles"
  gate changes with it.
- Two new capabilities: extracting the cover embedded in the user's own file, and
  capturing the Goodreads *work* id (free — same page blob already fetched).
- The matching rules are rewritten, not tuned.
- **All six creation doors change behavior.**

## Contents

| Area | Substance |
|---|---|
| Identity model | Routes; our own work id; the hybrid shape borrowed from Hardcover (skeleton), OpenLibrary (authors-with-roles, subjects), Goodreads (work id + editions) |
| Matching | Two failure directions need different tools — *same book, different clothes* vs *different book, same clothes*. One threshold cannot serve both; that is why every past adjustment traded one failure for the other |
| Evidence order | your choice > the file > a provider id > nothing |
| Covers | Rank: your choice → your file's embedded cover → the work's default edition for that format → any other edition → placeholder. Never downgrade |
| Cover UI | Replace "Trust: validated/unvalidated" with **source** — "validated" only ever meant *the deleted gate passed it* |
| Placeholders | Two, work-level: *Searching for a cover* / *No cover found*. Must not be permanent; must be actionable |
| Doors | Readarr import holds an ISBN, an ASIN and the actual files, and resolves no identity at all |
| Modal | Sibling panel goes informational; the PO's parked wording call lands here |
| Cover sweep | r2's S4b, rebuilt once and never shipped. Only affects *when* the PO sees the change, not *whether* |

## Consistency tensions to carry in

- **The badge breaks.** "Confirmed" means a provider work id exists. Under routes, a work
  with zero routes is valid. The badge becomes about *connectedness*, not identity — and
  the enrichment gate hanging off it starves valid books unless decided explicitly.
- **ISBN is both essential and useless, and that is not a contradiction.** Excellent as a
  *lookup key*; worthless as a *comparison* (a work has dozens). **Write this down or
  someone will helpfully restore Rule A.**
- **The bug fix collects the wrong Goodreads number.** C5 gains book ids, not work ids.
  Not wasted — a book id is a valid route and reaches the work id free on the same page —
  but incomplete. The rewrite revisits those works.
- **The cover rank cannot be fully built yet.** Nothing extracts a file's embedded cover
  today; that rank-2 slot is a new capability.
- **"Searching for a cover" needs a third exit.** A work with zero routes has nowhere left
  to look — distinct from both a provider outage and a genuine miss.

## Verify before sizing

**The eight-site matching conformance list (V1–V8), 2026-07-14.** The rewrite converges
matching into one authority; eight sites with their own recipes means eight call sites to
migrate, not one function to edit. Memory: `project_matching_conformance_gaps`.

- **V1 is already fixed** — verified 2026-07-25. `flm_title` (colon-truncation + word-set
  containment) no longer exists; `flm_match` (`crates/livrarr-identity/src/async_resolver.rs:318`)
  routes through the shared authority and is reached twice from `settle_identity`
  (`:174`, `:198`). The audit's placement was right; its mechanism is gone.
- **V2–V8 are unverified and their line numbers are dated.** Re-verify before sizing.

## Open questions

- Audio-first books cannot get a work id from Audible or Audnexus — neither models a
  work. Exception, or unlinked until a text provider is reached?
- Is an abridgement the same work? Is an omnibus a work or a container?
- Are release-matching and file-matching in scope for the shared primitive?
- Manual import runs a machine dedup *before* honoring the user's pick; if it decides the
  book already exists, the pick is discarded and the file attaches elsewhere, unasked.
  Predicted, **not observed** — no duplicate-work damage found in the PO's library.
- The four foreign-language dead ends (memory `project_foreign_gr_deadends`) — deferred to
  the next foreign-language block, not to this rewrite, unless the routes model explains
  them.
