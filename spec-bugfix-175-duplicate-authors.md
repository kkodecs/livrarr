---
feature: "bugfix-175-duplicate-authors"
stage: spec
status: draft
version: 3
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004]
---

# Bug Spec: bugfix-175-duplicate-authors

GitHub issue: kkodecs/livrarr#175 — CSV list import creates duplicate author rows for
identical names; works scattered across the duplicates.

Revision 4 (snapshots: `build/reviews/bugfix-175-duplicate-authors/spec-r1.md`,
`spec-r2.md`, `spec-r3.md`). r2 folded all 12 r1 findings; r3 folded the 4 r2
findings (ST-010 NULL-key exemption, REQ-003 tx factoring, AC-006 error detail,
ST-007 citation). r4 folds round 3's two: REQ-001/REQ-002 and their echoes are now
explicitly scoped to canonicalizable names (non-NULL keys) — closing the
contradiction openai r3 filed via BLOCKED-spec.md — and AC-004/AC-006 gain live
create/rename coverage of the NULL-key path.

## Prior-design context (governs this spec)

`docs/design-author-dedup.md` (2026-07-11, SHIPPED) already built author dedup for
*spelling variants*: the `unambiguous_author_match` adopt gate at the create doors, and
a full merge primitive — `AuthorDb::merge_authors` (sqlite_author.rs:80-363) +
`AuthorService::merge` + `POST /author/{id}/merge` — with settled semantics (series
fold/move, cache drops, monotonic author fields, works repoint + display-name rewrite +
`merge_generation` bump; `works.normalized_author` untouched, D-3). Its **D-4** decided
*against* a DB unique backstop because fuzzy adoption is non-transitive — and recorded
the "byte-identical concurrent-insert race" as a theoretical residual. **Issue #175 is
that residual, real.** This fix supersedes only D-4's "stays theoretical" premise: a
unique key over an exact stored form IS expressible and transitive, and coexists with
the fuzzy gate (which keeps handling variants the key cannot). D-6 (no merge UI) stands.

## Routing flag (disqualifier vocabulary)

The core unit of this fix is **concurrency + identity-integrity**: it changes a
uniqueness invariant on a BIG7 domain entity (Author) under concurrent writers, and it
carries a **schema/migration** change. Disqualifiers tripped: domain-entity touched,
invariant change, schema/migration. Route the implementation unit to the
**`dev-critical` seat (fable max)**; the PM re-runs that unit's gates independently
after handback (seating-table output gate for that row).

## 0a. Design Principles

- **One normalization authority.** The stored author key is computed by exactly one
  named existing recipe (REQ-004 names it); no second ad-hoc variant per call site.
- **The invariant lives at the write layer.** One author row per (user, stored key)
  holds no matter which door writes — never re-implemented per door. Scope: rows
  whose names canonicalize (non-NULL keys); ST-010's NULL-key rows are exempt by
  design.
- **Reuse the shipped merge contract.** Duplicate-group repair goes through the
  `merge_authors` semantics (fold/move/drop/monotonic-fields, D-3/D-7) — never a
  parallel re-implementation of "re-parent the references."
- **Mirror the works-repair precedent** for arming uniqueness over a table that
  already holds duplicates: one marker-guarded transaction — repair, then index —
  never a bare CREATE UNIQUE INDEX in a migration file.
- **Repair is not destructive.** The shipped monotonic policy governs (booleans OR,
  `monitor_since` MIN, keys/sort_name fill-missing-never-overwrite via survivor-first
  coalesce). No new logging obligations are attributed to it; conflict logging, if the
  implementation adds any, is NEW behavior and optional.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Live schema sampled: `PRAGMA index_list(authors)` on testdata/livrarr.db → only `idx_authors_user_id`, non-unique; matches `crates/livrarr-db/migrations/001_initial_schema.sql:38-49`; full sweep of migrations shows no author-name unique ever added | The `authors` table has NO database-level uniqueness on name today | Assuming any DB-level protection exists | high |
| ST-002 | `crates/livrarr-metadata/src/list_service.rs` `confirm` (body 279–507): rows stream through `.buffer_unordered(5)`, each row calling `work_service.add` | List-import confirm runs up to 5 rows' add-pipelines concurrently inside one confirm call | Treating the import as serial; "serialize the CSV loop" as a sufficient fix | high |
| ST-003 | `find_or_create_author` (work_service.rs:2836-2905): `find_author_by_name` (SQL `LOWER(TRIM(name))=LOWER(TRIM(?))`, sqlite_author.rs:495-513) → `unambiguous_author_match` fuzzy adopt → `create_author` bare INSERT, no ON CONFLICT (sqlite_author.rs:393-413) | Author resolution is non-atomic check-then-act; nothing between the check and the INSERT closes the window | Treating the fuzzy adopt gate as race protection; fixing only the lookup while leaving the bare INSERT | high |
| ST-004 | Sampled live: `sqlite3 :memory: "SELECT LOWER('É'), 'é'='É'"` → `É`, `0` | SQLite `LOWER()`/default collation folds ASCII only; Unicode case variants compare unequal | Computing the uniqueness key in SQL; relying on DB collation for case-insensitive Unicode equality — the key must be Rust-computed and stored | high |
| ST-005 | Works precedent, read at source: `insert_work_row` `ON CONFLICT(user_id, normalized_title, normalized_author) DO NOTHING` + winner re-select (sqlite_work.rs:177-222, 1589-1614); one-time startup repair backfill→merge→`CREATE UNIQUE INDEX idx_works_identity` in ONE marker-guarded tx (`backfill_normalized_identity`, pool.rs:400-429, 780-783); migration 038 adds columns only; repair uses sanctioned `INSERT OR IGNORE` where THREE overlapping unique constraints make one named conflict target impossible (pool.rs:688-706) | The repo's proven pattern for exactly this class exists and handles the pre-existing-duplicates problem | A plain `CREATE UNIQUE INDEX` inside a migration file (fails on installs holding duplicates); editing applied migrations (insight 19); `INSERT OR REPLACE` anywhere (insight 20). For the author-row create/upsert itself: use a named `ON CONFLICT` target (a single unique key exists — the overlapping-constraints exemption that sanctions `OR IGNORE` does not apply there) | high |
| ST-006 | Issue #175 (alpha6 install: Anne Rice ×4, Diana Gabaldon ×4, Lois Lenski ×4, Heinlein ×3, works scattered); local dev DB sampled clean (39 authors, 0 dup groups, read-only query) | Real installs already contain duplicate author rows with works split across them; dev data does not reproduce this by itself | Assuming a fresh-schema-only fix is complete; repro/repair fixtures that rely on pre-existing local dupes | high |
| ST-007 | `AuthorDb::merge_authors` (api/author.rs:29-42; impl sqlite_author.rs:80-363) + `AuthorService::merge` + `POST /author/{id}/merge` — shipped by `docs/design-author-dedup.md` U-3. Contract read at source: works repoint + display `author_name` rewrite + `merge_generation` bump (D-3: `normalized_author` untouched); series FOLD on same `gr_key` (flags OR'd, roster repointed, work-flags propagated) else MOVE — `series` carries `UNIQUE(user_id, author_id, gr_key)` (migrations/023:13) so naive re-parenting is impossible; `author_series_cache` (migrations/023:18-22) and `author_bibliography` (migrations/003:1-4) loser rows DELETED (refetchable, `author_id` PKs); author fields monotonic (OR / MIN / survivor-first COALESCE, sqlite_author.rs:289-342 — silent, no conflict log). The method OWNS its transaction: `begin()` at sqlite_author.rs:93, `commit()` at :357; no in-tx variant exists today | A complete, reviewed, shipped merge primitive with settled collision semantics exists for author duplicate resolution — but as a self-committing method, not a composable in-tx body | Re-deriving re-parenting semantics; "re-parent every referencing row" phrasing (two tables cannot be re-parented); attributing conflict-logging to the existing policy; calling the self-committing method per group inside a repair that promises single-transaction atomicity | high |
| ST-008 | `docs/design-author-dedup.md` D-4: no DB unique backstop — fuzzy adoption is pairwise/non-transitive, no canonical key expresses it; "the residual byte-identical concurrent-insert race stays theoretical (single-admin usage)" | A prior decision rejected a unique index for the FUZZY relation, with the #175 race explicitly accepted as theoretical residual | Re-litigating D-4 as if it forbade THIS fix (it does not: an exact stored-key unique is transitive and compatible); leaving the supersession implicit | high |
| ST-009 | `AuthorService::update` passes `req.name` through to `UpdateAuthorDbRequest` (author_service.rs:202-246, name at :234); `SqliteDb::update_author` writes it | A live RENAME door mutates `authors.name` after creation | Maintaining the stored key only at create time; an inventory of "creation doors" as the full invariant surface | high |
| ST-010 | `canonical_author_key` returns the EMPTY STRING for non-canonicalizable input (`canonical_author_name(...).map(...).unwrap_or_default()`, identity_matching.rs:642-650) — e.g. suffix-only ("Jr.") or credit-only ("(Editor)") names; production doors accept such names today (manual add rejects only trim-empty, author_service.rs:82-87; Readarr checks only trim-empty, readarr_import_workflow.rs:1766-1769); SQLite plain unique indexes treat NULLs as DISTINCT (in-repo documented: migrations/032_notification_dedup_index.sql:13-16) | Distinct junk-named authors would all share the "" key; a NULL stored key is exempt from a plain unique index by SQLite semantics | Storing "" as a key value (would merge distinct junk-named authors at repair and converge their creates); rejecting such names at any door (a behavior change this bugfix does not make) | high |

## 1. Problem Statement

**What's broken (verified at source, not just reported):** during CSV list import,
`confirm` processes up to 5 rows concurrently (ST-002). Each row resolves its author
through a non-atomic find-or-create (ST-003) with no DB backstop (ST-001). Two or more
in-flight rows naming the same author can all miss the lookup and all insert — one
author row per racing row. Works then attach to whichever duplicate their row created
or found, scattering one author's works across 2–4 rows. The observed ×2–×4
multiplicity matches the concurrency window of 5. This is exactly the race D-4 accepted
as theoretical (ST-008), now observed in the field.

**Reporter corrections (run-#2 expectation confirmed):** the reporter's "concurrent or
repeated rows" is half right — *serial* repeated rows do NOT duplicate (the exact
lookup, then the adopt gate, catch them); the trigger is specifically the concurrent
window. The suggested `INSERT OR IGNORE` is not the shape for the author create: the
works-table precedent (named `ON CONFLICT` target + winner re-select) is the sanctioned
form (ST-005).

**Steps to reproduce:** import a Goodreads-format CSV with many rows sharing authors
(thousands of rows makes it reliable); open Authors; prolific authors appear 2–4×.

**What correct behavior looks like:** for every canonicalizable author name (non-NULL
stored key, ST-010): one author row per distinct stored key per user, regardless of
import size or concurrency; all of that author's works attached to that single row;
installs that already hold duplicates are repaired once at upgrade through the shipped
merge semantics; after the fix the database itself rejects a duplicate.
Non-canonicalizable (junk) names keep today's behavior exactly (ST-010 exemption).

**Affected surfaces — the full invariant surface (verified):** three production
creation doors — `find_or_create_author` (every work-add path incl. list import — the
racing one), `AuthorServiceImpl::add` (manual author add — same check-then-act shape),
Readarr `process_authors` (serial batch-local snapshot; safe intra-import, unprotected
against a concurrent other-door write) — plus the RENAME door (ST-009).
`SecondaryApiImpl::add` is a test harness by its own header (api_secondary_impl.rs:1),
zero non-test constructors; NOT a production door (already so ruled in
design-author-dedup §0 [REV codex R-1]) — it is out of the invariant surface, but its
tests must stay green.

## 2. Requirements

- **REQ-001** (converge under concurrency): When multiple list-import rows whose
  author names canonicalize to the same NON-NULL stored key (ST-010) are processed
  concurrently in one confirm call, exactly one author row exists afterwards for that
  (user, stored key), and every work from those rows is attached to it.
- **REQ-002** (invariant holds at every door, honestly reported): The
  one-row-per-(user, stored key) invariant holds for NON-NULL stored keys across ALL
  production author-writing surfaces (the three creation doors and the rename door);
  NULL-key rows are exempt per REQ-004's empty-recipe policy. A writer that loses a
  creation race converges on the winning row; its caller-visible outcome is the same
  as a lookup hit: no error surfaced, the winner's author id returned, and **no
  "created" signal** — specifically, Readarr's `authors_created` progress counter does
  not count a converged row (readarr_import_workflow.rs:1806 increments today on every
  successful create call).
- **REQ-003** (one-time repair of existing installs, via the shipped merge): On
  upgrade, installs whose `authors` table holds two or more rows with the same
  (user, stored key), the key being NON-NULL, are repaired exactly once, BEFORE the
  uniqueness constraint arms: every such duplicate group resolves with the
  `merge_authors` SEMANTICS (ST-007 — series fold/move, cache drops, works repoint
  incl. display-name rewrite and `merge_generation` bump, monotonic author fields).
  Rows whose stored key is NULL (ST-010) are never merged by the repair. Keeper per
  group follows the shipped D-5 policy: most works → most external keys → oldest id.
  The whole repair — key backfill, merges, index creation, completion marker — is one
  atomic, marker-guarded, idempotent unit: a partial run can never pass as complete,
  and a re-run after completion is a no-op. Because the shipped `merge_authors`
  method commits its own transaction (ST-007), the repair must run the merge
  semantics INSIDE its single transaction via a shared in-transaction body used by
  both the live merge endpoint and the repair — the same factoring the works repair
  uses (`merge_user_identity_state`, "shared with the live work-merge action so both
  paths apply the identical policy", pool.rs:680-686) — never as N independent
  `merge_authors` commits.
- **REQ-004** (DB-enforced going forward, key maintained on every name write): After
  repair, uniqueness of (user, stored key) is enforced by the database for every row
  with a NON-NULL key — a duplicate insert fails at the DB even from a code path that
  bypasses the service layer. The stored key is computed in Rust — **recipe:
  `canonical_author_key`, the same canonicalization `identity_key` already stores
  into `works.normalized_author` (identity_matching.rs:638-650: order-normalized,
  accent-stripped, suffix-dropped; Q-002 resolved, PO 2026-07-27)** — and never in
  SQL (ST-004). **Empty-recipe policy (ST-010):** when the recipe yields the empty
  string (non-canonicalizable name), the stored key is NULL — the row is exempt from
  uniqueness (SQLite NULL-distinct semantics), every door keeps today's acceptance
  behavior (no new rejections), and the shipped "unusable candidates create
  separately" rule is preserved. Accepted residual, unchanged from today: concurrent
  creates of such junk names can still duplicate; the merge endpoint remains the
  recovery. Every write of `authors.name`, including the rename door (ST-009),
  recomputes the stored key in the same transaction. A rename whose new NON-NULL key
  collides with a different existing row is REJECTED with a validation error that
  names the colliding author (no silent merge, no partial write); the recovery for
  an intended merge is the existing merge endpoint.

## 3. UI/Interface Design

No UI change. The Authors page simply stops showing duplicates; repaired installs show
merged rows.

## 4. Non-Requirements

- **Fuzzy near-duplicate unification beyond current behavior.** The
  `unambiguous_author_match` adopt gate and its semantics stay exactly as shipped;
  this fix adds the DB backstop for stored-key-identical rows only. (Under the
  Q-002-recommended recipe, "identical" means identical canonical form — e.g.
  "J.K. Rowling"/"J K Rowling" — per the same authority the adopt gate and
  `works.normalized_author` already use. Glued-initials variants ("JK Rowling")
  canonicalize differently and remain the fuzzy gate's job — design-author-dedup §0.)
- **No changes to `find_author_by_name`'s SQL comparison beyond what the stored-key
  lookup supersedes** on the paths this fix touches (ST-004's ASCII-only miss noted;
  no further matcher work).
- **A user-facing "merge authors" UI.** D-6 stands (endpoint exists, no frontend);
  Q-001 resolved OUT — and the backend action already shipped, so nothing to build.
- **Cross-user dedup.** Authors are per-user rows; nothing merges across users.
- **Work-identity rekeying.** `works.normalized_author` divergence handling stays with
  the identity-key generation machinery (D-3); repair rewrites display names via the
  merge contract only.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Manual "merge duplicate authors" maintenance action? | resolved | PO 2026-07-27: out of scope; no follow-up issue. Premise correction (r2): the backend merge action already exists (ST-007); only a UI affordance would be new, and D-6 already parks that. Decision unchanged. |
| Q-002 | Stored-key recipe: `canonical_author_key` (the one existing authority; also converges dotted/spaced/accent/order variants) vs bare trim+Unicode-lowercase (narrower: byte-case-identical only)? | resolved | PO 2026-07-27: `canonical_author_key` (option A). Baked into REQ-004. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): A red repro test drives the REAL list-import confirm door
  (real `ListService::confirm` seam or real route, with the real `SqliteDb` writer —
  production entry path, no injected state) with N concurrent rows sharing one author
  name; it asserts exactly 1 matching author row and all works parented to it. It
  FAILS on current code (≥2 rows) and passes after the fix.
- [ ] **AC-002** (REQ-002): Race-loser convergence is exercised per production door:
  (a) concurrent `work_service.add` candidates with the same author → one row, both
  results carry the winner's id, at most one path reports `author_created`;
  (b) the manual author-add door racing the same name → converges, result is the
  Updated/adopted shape, never a second Created row; (c) Readarr `process_authors`
  whose `create_author` call converges on an existing row → `authors_created` counter
  unchanged and the Readarr id maps to the winner row.
- [ ] **AC-003** (REQ-003): Repair fixture seeds the REAL legacy precondition by
  direct SQL — pre-constraint rows, exactly as the works-repair tests seed
  pre-migration-038 state (pool.rs:1293-1297: the real writers' conflict target
  requires the very index the repair creates; this constructed state is the
  production-faithful representation of a pre-upgrade DB and carries this
  justification). Groups include: works scattered across dupes; a same-`gr_key`
  series on keeper AND loser (fold arm); a loser-only series (move arm);
  bibliography/cache rows; distinct monitor flags/keys per row; plus TWO distinct
  authors whose names canonicalize to empty (ST-010). After one repair pass: one
  keeper per non-NULL-key group chosen by the D-5 policy (asserted on a fixture
  where most-works ≠ oldest-id); the two empty-canonical authors remain SEPARATE
  rows with NULL keys; zero references to deleted rows in ANY referencing table;
  series folded/moved per ST-007 with no work unlinked; monotonic fields per policy;
  re-run is a no-op.
- [ ] **AC-004** (REQ-004): After repair, the live schema shows the unique constraint;
  a direct DB-level duplicate insert of the same (user, non-NULL stored key) cannot
  produce a second row; two inserts with NULL keys (non-canonicalizable names) both
  succeed (ST-010 exemption); and a LIVE create through a real production door of a
  non-canonicalizable name (e.g. "(Editor)") succeeds and stores a NULL key —
  asserted from the resulting row, proving the door computes NULL rather than ""
  and rejects nothing.
- [ ] **AC-005** (REQ-003 all-or-nothing): A mid-transaction failure injected during
  the author repair (mirroring `mid_transaction_failure_rolls_back_all_data_and_marker_together`,
  pool.rs:1454+) rolls back key backfill, merges, index, and completion marker
  TOGETHER; a subsequent clean run completes the repair fully.
- [ ] **AC-006** (REQ-004 rename door): Renaming an author recomputes its stored key
  in the same transaction (verified by a subsequent create of the OLD name producing
  a new row, and a create of the NEW name converging); renaming onto a different
  existing author's key is rejected with a validation error whose caller-visible
  message identifies the colliding author (name or id asserted, not just the error
  class), and changes no rows; renaming TO a non-canonicalizable name succeeds with
  the key recomputed to NULL — including when another NULL-key row already exists
  (no collision, ST-010 exemption).

## System Truths check (bugfix lane item 5)

The bug revealed truths missing from any spec: ST-001 (no DB uniqueness on authors),
ST-004 (SQLite LOWER is ASCII-only), ST-007 (a shipped merge primitive with settled
collision semantics), ST-008 (D-4's deliberate no-unique decision and its now-false
"theoretical" premise), ST-009 (the rename door). All recorded in 0b with sampled
sources.
