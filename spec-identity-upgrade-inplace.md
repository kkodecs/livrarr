---
feature: identity-upgrade-inplace
stage: spec
status: draft
version: 1
type: bugfix
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008]
---

# Bug Spec: identity-upgrade-inplace

## Executive summary

Starting the current build on an existing alpha6 library (migration 073, with books) must just work.
Today it refuses to start until four manual commands are run, and the manual conversion loses the
user's "you picked this match" marks and every book's status. This spec transcribes the accepted
plan r2 ([PA-006](#0c-prior-art)) and the PO decision ([PA-009](#0c-prior-art)); it adds nothing.

- **What changes.** One new migration, 091, converts the library in place on the first start after
  the upgrade. The existing startup check, unchanged, then switches the library over. No commands,
  no rehearsal, no approval ([REQ-003](#2-requirements)).
- **Nothing the user did is lost.** Identifiers, confirmations, status, titles, authors, monitoring,
  covers and edits carry over; a book's badge shows green "Confirmed" ([REQ-001](#2-requirements)).
  Alpha6's audiobook cover choice, which two published migrations drop, is saved before them and
  restored by 091 ([REQ-002](#2-requirements)).
- **Odd cases never stop startup.** Look-alike books stay separate, a shared identifier stays with
  the first book, old unanswered match questions are closed keeping the current match; each is
  logged ([REQ-004](#2-requirements)).
- **One protected backup per upgrade**, not one per restart. Three backup-bookkeeping findings from
  the last review are carried in as required behaviour ([REQ-005](#2-requirements), AC-014 to AC-016).
- **Not yet approved to build.** The PO accepted the plan as the build basis; building needs a
  separate PO go, and the PO must say who writes the tests (Q-008). Estimate: about 27 hours,
  provisional (the reviewer did not accept it as complete).

Sections: [system truths](#0b-system-truths), [problem](#1-problem-statement),
[requirements](#2-requirements), [questions](#5-open-questions), [acceptance](#6-acceptance-criteria).

## Revision

v1, 2026-09-28: transcribed from plan r2 (sha256 `b5c387ab…d5a16`), `DECISION.md` and
`REVIEW-astra-r2.md` findings R2-01 to R2-03. Where those are silent, this spec says so.

## 0a. Design Principles

- Never merge. Automatic conversion on first start. The user is asked nothing (PO, `DECISION.md:11-13`).
- Fix the data where it is created: the conversion itself carries every user choice over.
- Reuse the existing readiness check and test harness; no new startup orchestration.
- Published migrations 001–090 are never edited.

## 0b. System Truths

Facts about main `8c60a5f4`. "T" = tested on `/tmp` copies in plan r2 or r1; "read" = code read only.

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-001 | sqlx applies and commits each pending migration separately; a migration's SQL and its ledger row share one transaction. A failure in 091 leaves 074–090 applied and undoes only 091. | `sqlx-core-0.8.6/src/migrate/migrator.rs:168-183`; `sqlx-sqlite-0.8.6/src/migrate.rs:136-162` | Treating a 073→091 upgrade as one transaction. | T: fault at 088 left max 87 (plan §2a); injected failure inside 091 left `.dump` byte-identical (plan §4); Astra r2 independent probe. |
| ST-002 | `run_migrations` is the one migration entry for server and CLI. | `crates/livrarr-db/src/pool.rs:51-53`; `crates/livrarr-server/src/main.rs:780`; `crates/livrarr-server/src/identity_layer.rs:762` | A cover-saving step placed on only one door. | read (plan §2a); `pool.rs:51-53` re-read 2026-09-28. |
| ST-003 | Migration 084 drops `audiobook_cover_trust`; 085 adds `audiobook_cover_manual` defaulting to 0. Alpha6 recorded an audiobook cover choice only as `audiobook_cover_trust='user'`; the ebook slot also set `cover_manual`. | `crates/livrarr-db/migrations/084_drop_cover_trust_columns.sql:2`; `085_round15_audiobook_cover_manual.sql:5-7`; alpha6 `crates/livrarr-db/src/sqlite_work.rs` lines 639-650, 663-686 | Any rescue that runs in or after 084; a new migration before 084 (published versions and checksums are fixed). | T: work 90 in the 178-book library has trust `user` at 073 and manual 0 after 090. |
| ST-004 | The cover gate honours a user audiobook choice only through `audiobook_cover_manual`. The Goodreads cover repair clears slots with manual 0 as a background pass after startup, commits queue and slot changes, and can return without its completion marker while serving continues. | `crates/livrarr-metadata/src/cover_write_gate.rs:213-254`; `crates/livrarr-db/src/identity_layer.rs:4996-5006`, `:5062-5064`, `:5092-5105`, `:5117-5119`, `:5174-5178`; `crates/livrarr-server/src/main.rs:1113-1127`; `crates/livrarr-server/src/jobs/cover_startup.rs:25-43` | Restoring the flag after the cover pass; treating the title repair as the only writer with unmarked progress (plan line 218, refuted by Astra R2-03). | read (plan §2a; Astra r2 source ordering, not an end-to-end run). |
| ST-005 | The readiness check returns active when `identity_authority_v2=active`; otherwise it activates only with no open legacy conflicts, no pending cards, and a latest apply run `ready` with `index_ready=1` and `blocker_count=0`, in one transaction that creates `idx_works_identity_v2`. | `crates/livrarr-db/src/identity_layer.rs:2956-3047` (`:2959-2966`, `:2971-3001`, `:3003-3046`) | Changing the check; planting the ready report or marker in tests. | T: three converted copies activated with the real binary (r1); activation fault then retry activated the same run (r2). |
| ST-006 | The activation unique index covers six columns (user, normalized main, subtitle, volume, primary author, distinction). Alpha6 created a runtime `idx_works_identity` that migrations do not create. | `crates/livrarr-db/src/identity_layer.rs:3032-3034`; alpha6 `crates/livrarr-db/src/pool.rs` lines 375-382 | A test library without alpha6's runtime index; look-alike groups left colliding. | read; `:3032-3034` re-read 2026-09-28. |
| ST-007 | The legacy identifier staging trims with Rust `str::trim` (25 Unicode whitespace code points); the identity key columns use SQL one-argument `trim`, which removes spaces only. | `crates/livrarr-db/src/identity_layer.rs:6264-6272` (called `:6155`, `:6189`); `:6111-6127`; Rust `core/src/char/methods.rs:893-897`, `core/src/unicode/unicode_data.rs:734-756` | SQL `trim(x)` on identifiers; changing the key-column expressions. | T: whitespace case (plan §2); four libraries byte-identical under the r2 normalization. |
| ST-008 | Alpha6 accepted OpenLibrary and Hardcover anchors with only a non-empty check, so padded anchor values are a legal alpha6 state. | alpha6 `crates/livrarr-db/src/sqlite_work_identity.rs` lines 27-38 | Exact-match anchor comparison without normalization. | read (plan §2 step 7). |
| ST-009 | Route readers parse `observed_at` as RFC 3339. | `crates/livrarr-db/src/identity_layer.rs:3226-3231` | SQLite's default timestamp text in routes. | read. |
| ST-010 | `schema_version` stays 83; the version gate and the report's version check depend on it; 089 set the precedent of not bumping it. | `crates/livrarr-db/src/pool.rs:57-63`; `identity_layer.rs:5937`; `migrations/089_authors_identity_index.sql:10-11` | Bumping `schema_version` in 091. | read. |
| ST-011 | `report_json` has no production reader; the SHA-256 fingerprints are compared only by the manual `apply`. | `migrations/082_identity_layer_foundation.sql:251`; `identity_layer.rs:6060`, `:2942-2948`, `:6417-6434` | Requiring Rust-only fingerprints in 091. | read; `RTK_DISABLED=1 rg` with a known-hit control (r2). |
| ST-012 | Status `connected` and `user_confirmed` both show the green "Confirmed" badge; `not_connected` shows "Pending". | `crates/livrarr-handlers/src/types/work.rs:496-499`; `frontend/src/pages/work-detail/components/BookInformationTab.tsx:21` | Leaving converted books `not_connected`. | T: four libraries (plan §1 table). |
| ST-013 | Alpha6 libraries end at migration 073 and have `identity_key_generation=1`; the title-key recompute is Rust-only with no production caller. | `git ls-tree v0.1.0-alpha6`; `crates/livrarr-db/src/pool.rs:424`, `:1607` | Recomputing keys in 091. | T: all four libraries; read for the recompute. |
| ST-014 | Today a backup is taken on every start; the keep-3 prune sorts by name and runs only after every check passes. The pool creates the file before the existence test. | `crates/livrarr-server/src/main.rs:765-777`, `:929-937`, `:756`; `crates/livrarr-db/src/pool.rs:220-252`, `:1584-1604`, `:19` | Version-named reuse (four copies prune the original). | T: a backup written with no migration pending; the empty fresh-install backup is inferred. |
| ST-015 | Nine marker-gated startup repairs run after migrations: review-dismissal adoption, title policy (off: `WORK_MERGING_AVAILABLE=false`), seam sweep, dedup residue, round-10, round-11, round-15 search ledger, round-15 Goodreads cover, round-21. | `crates/livrarr-server/src/main.rs:812-923`; `pool.rs:518-527`, `:496`, `:1276`; `identity_layer.rs:4499-4509`; `crates/livrarr-domain/src/identity_layer/services.rs:54` | A backup rule that ignores repairs. | read (plan §5; Astra r2). |
| ST-016 | Alpha6 cannot open a library once a newer migration has committed: its default migrator refuses unknown applied versions. | `sqlx-core-0.8.6/src/migrate/migrator.rs:28-45`; alpha6 `pool.rs` lines 38-39, `main.rs` lines 628-631 | Promising rollback without the pre-upgrade copy. | inferred, not run. |
| ST-017 | In sqlite3 3.45.1 a row-value `IN (SELECT … FROM json_each(…))` matched 0 real rows; the `EXISTS` form matched. | plan §2a, `/tmp/livrarr-inplace-091/r2-stash/` | The row-value `IN` form in step 0. | T in sqlite3; bundled `libsqlite3-sys 0.30.1` (`Cargo.lock:1542-1543`) inferred same, pinned by test 3. |
| ST-018 | Test-harness facts: the existing failpoint is thread-local and cannot reach the child binary; triggers on `_sqlx_migrations` and `_livrarr_meta` can; the test helper migrates an empty database to head (skipping 091); new files under `tests/` are gitignored. | `identity_layer.rs:6451-6463`; `crates/livrarr-db/src/lib.rs:178-181`; `.gitignore:3`; `crates/livrarr-server/tests/test_fresh_author_index.rs:23`, `:39-81`, `:94-115` | Thread-local faults, the empty-to-head helper, or an unregistered test file for upgrade cases. | T: both triggers reached the child binary (r2). |
| ST-019 | A failed migration exits with "Migration failed" and Docker restarts the container. | `crates/livrarr-server/src/main.rs:780-784`; `docker-compose.yml:15` | A retry that adds a copy per restart. | read; loop observed in the research report. |
| ST-020 | The look-alike query must use `GROUP BY`: the pairwise form took 20.6 s on 10,008 books; the grouped form 0.61 s. | plan §1 "Speed" | The pairwise form. | T. |

## 0c. Prior Art

Searched `build/reports/`, `build/reviews/identity-cutover-inplace/`, `docs/`, the identity-layer-rewrite artifacts.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `spec-identity-layer-rewrite.md`, `ir-v1-/ir-v2-identity-layer-rewrite.yaml`, `contract-identity-layer-rewrite.yaml:332` | Defines the manual cutover and forbids activating a non-empty database without two approved rehearsals. This feature replaces that rule for existing libraries; the manual CLI stays. A decision record is required (REQ-007). |
| PA-002 | `contract-identity-layer-rewrite.yaml:342`, `:347` | Legacy columns are not identity authority after the marker. 091 reads them only while the marker is inactive. |
| PA-003 | `docs/identity-lifecycle-audit-2026-08.md:107-111`, `:126` (AUD-P0-4, AUD-P0-5, fix-wave 3) | Non-empty cutover discards user confirmations; an invisible skipped route collision. REQ-001 and REQ-004 close both for this path. |
| PA-004 | `build/reports/identity-cutover-upgrade-research-2026-09-27.md` | Why the cutover was manual (ratified 2026-08-06); recommends automatic first start (Option B); Docker backup growth. |
| PA-005 | `build/reports/identity-cutover-plan-2026-09-27.md` | Sections A–C: field mapping and defects that plan r2 builds on; the 52-hour plan this replaces; the bad-author-key trap (its decision 6), out of scope here. |
| PA-006 | `build/reports/identity-cutover-inplace-plan-2026-09-27.md` (r2) | The accepted basis of this spec; r1 kept at `build/reviews/identity-cutover-inplace/review-plan-r1/plan-r1.md`. |
| PA-007 | `build/reviews/identity-cutover-inplace/review-plan-r1/REVIEW-astra-r1.md` | FAIL, R1-01 to R1-05; plan r2 folded all five. |
| PA-008 | `build/reviews/identity-cutover-inplace/review-plan-r2/REVIEW-astra-r2.md` | FAIL on R2-01 to R2-03 (backup bookkeeping); conversion, cover rescue, normalization and activation tests judged correct. Findings carried into REQ-005. |
| PA-009 | `build/reviews/identity-cutover-inplace/review-plan-r2/DECISION.md` | PO accepted plan r2 with R2-01..03 required; no third plan round; build needs a separate go. |
| PA-010 | `migrations/088_provider_policy_enrichment_defaults.sql:21-24`; `026_cascade_user_fk.sql:8-11` | Precedent for temporary decision tables inside a migration. |

## 1. Problem Statement

**Action and starting condition.** The user replaces an alpha6 image with the current build and starts
it on the existing library (migration 073, has books). No book is added or imported.

**Expected:** the server starts; the library looks the same; books show "Confirmed"; one backup exists.
**Observed today** (tested on `/tmp` copies, PA-004/PA-005):
- Startup stops with "cutover required" until four manual commands are run; in Docker the container
  restarts in a loop and each restart writes another full backup.
- The manual conversion drops about 300 "you picked this match" marks per library and leaves every
  book "Pending"; one open legacy conflict blocks startup forever; a shared identifier is dropped
  from the second book without naming it.
- Migrations 084/085 drop alpha6's audiobook cover choice (work 90, 178-book library).

## 2. Requirements

- **REQ-001**: Migration 091 converts an alpha6 library losslessly (plan §2 steps 1–13).
  - *Identifiers.* Each legacy identifier and anchor value is normalized with the 25-code-point
    whitespace set (ST-007) before the empty test, grouping, owner lookup and audit text; an empty
    result makes no route and no edition. Column→provider/kind/owner mapping and JSON encodings
    match the staging (`identity_layer.rs:6143-6207`). Owner is an existing active route, else the
    lowest book id. Goodreads, ISBN-13 and ASIN get one edition per owned value, format "Unknown";
    provenance `Migrated{legacy_field}`.
  - *Identity keys.* The staging's expressions verbatim, including its SQL `trim`; `primary_author_id`
    only from an existing author row.
  - *Confirmations.* `user_confirmed=1` on an active `Migrated` route when the same book has a
    `setter='user'`, `confidence='confirmed'` anchor of the matching type whose normalized value
    equals the route value (alpha6 "picked this search result" counts as confirmed, Q-005).
  - *Contributors and status.* One ordinal-0 contributor equal to the primary author when none;
    status `user_confirmed` / `connected` / `not_connected` from active routes.
  - *Preserved unchanged:* titles, author names, series and positions, monitor flags, metadata
    provenance, `cover_manual` and both cover slots (URL, source, size, file bytes), and another
    user's identical identifier. Import anchors become unconfirmed routes.
  - *Not carried* (as plan r2 §1 and PA-005 section A record): pending machine guesses, empty
    "still pending" markers; PA-005 also lists the dead-end list as not carried. Plan r2 adds no change here.
  - Result on the four real libraries must match the manual staging row for row, plus the marks.
- **REQ-002**: Alpha6 audiobook cover choices survive 084/085 (plan §2a). Inside `run_migrations`,
  before the migrator, if `works.audiobook_cover_trust` exists, one transaction writes
  `_livrarr_meta('upgrade_audiobook_cover_user_choices', [{u,w}…])` for every `'user'` row, replacing
  any earlier value; if the column is gone, a saved list is left alone. 091 step 0, ungated, sets
  `audiobook_cover_manual=1` for saved pairs with an `EXISTS` join to `json_each` (ST-017) and deletes
  the key. Absent key: no change. Limit: databases that passed 084 without a saved list are not recovered.
- **REQ-003**: Conversion is automatic on the first start, through migration 091 and the unchanged
  readiness check (ST-005). Every identity statement is gated on one decision row: books exist and
  `identity_authority_v2` is not `active`. 091 cancels pending review cards, resolves pending route
  clashes ("kept on first book"), records one `identity_cutover_runs` row (mode `apply`, branch
  `automatic`, status `ready`, JSON summary, zero-filled fingerprints) and a report with
  `index_ready=1`, `blocker_count=0`; `schema_version` stays 83 (ST-010); temporary tables are dropped.
  Audit actor `identity-upgrade`. Fresh and already-active libraries are no-ops. A crash inside 091
  rolls back only 091 (ST-001); a crash between 091 and activation activates the same ready run next start.
  The CLI command code is untouched.
- **REQ-004**: Odd cases never stop startup, and each is audited, never asked.
  - *Look-alikes:* group by the six index columns, skip books without an author; every member but the
    lowest id gets `text_distinction='upgrade:separate:<id>'` and one audit event. No merging.
  - *Shared identifier:* the first book keeps it; others get no route or edition and one audit event
    naming the value and the keeping book.
  - *Old unanswered match questions:* an audit event with `incoming_payload_json`, then `dismissed`,
    `resolved_at`, note "closed by identity upgrade; existing match kept", `resolution_action` NULL.
    Closed rows untouched.
  - *Needs-review books* with an identifier show "Confirmed" (plan §8; Q-004 parked).
- **REQ-005**: One pre-upgrade copy per upgrade, protected (plan §5 as amended by R2-01..03).
  - *Records.* `startup_generation` (highest embedded migration plus each marker-gated repair's key
    and target generation, ST-015) and `upgrade_snapshot` (file, version taken at, repair marker values,
    `in_progress`/`complete`, previous completed copy).
  - *Decision before migrations.* Fresh install: no copy. In-progress and database moved on: reuse.
    Generation differs: new upgrade via `VACUUM INTO …pre-migrate-v<NNN>-<UTC>.partial`, rename, then
    record. Otherwise no copy. Completion releases the previous rollback copy. The keep-3 prune never
    counts `.partial` files and never deletes the in-progress or rollback copy. Retention: kept until
    the next upgrade finishes.
  - *R2-01 (required).* Interruption before publication, after publication, or before the
    `upgrade_snapshot` commit, and rejection of that commit on several consecutive starts, leave a
    bounded, explicitly associated snapshot, keep the previous protected copy, and run no migration
    or repair until the snapshot is recoverably associated. A published orphan is recovered or
    reclaimed on a later start, not only in an error handler. No reuse based only on a `vNNN` prefix.
    Restore-and-edit still yields a snapshot containing the edit.
  - *R2-02 (required).* Progress is determined independently of whether the copy exists. Progress
    with the original missing: report that the original rollback copy is unavailable and stop before
    further upgrade writes; never write the progressed database under the original's name. Retake only
    when no upgrade mutation has occurred, via a partial file and atomic replacement.
  - *R2-03 (required).* Completion is defined against the repairs that actually apply, distinguishing
    synchronous init completion from the asynchronous cover repair (ST-004). A prior rollback copy is
    not released while that repair's marker or worklist says incomplete; its durable progress counts
    if the attempt stays `in_progress`. The disabled title repair does not block completion. Reuse the
    existing marker/worklist; no new user step or network-dependent serving gate.
  - How R2-01..03 are implemented, and the missing-original message text, are not specified by the
    plan or the review; the design stage settles them.
- **REQ-006**: When the check has just activated an automatic run, the server logs one summary line
  from the stored summary, e.g. "Identity upgrade complete: 139 books, 604 identifiers (299 you had
  confirmed); 0 look-alike books kept separate; 0 shared identifiers; 0 old questions closed." The
  wording is illustrative (plan: inferred).
- **REQ-007**: Docs: CHANGELOG; README upgrade note with the rollback-copy rule; a wiki upgrade page;
  one decision record replacing the "refuse a non-empty inactive library" rule, snapshotting any design
  doc first (`docs/design-history/`).
- **REQ-008**: Acceptance (plan §7 packet 5): the four real libraries on the release-candidate image
  with `--network none`; a 10-minute Docker restart loop; rollback to alpha6 from the copy, an edit
  and a second upgrade; the existing suites re-run unmodified; full fmt, clippy and test.

## 3. UI/Interface Design

No frontend change. Visible effects (plan §8): log lines "pre-upgrade backup: livrarr.db.pre-migrate-v073-…",
"Database migrations complete", the REQ-006 line; green "Confirmed" badges. If 091 fails: "Migration
failed: …", earlier migrations stay applied, 091's changes are undone, no new copy per restart, and
going back to alpha6 means restoring the pre-upgrade copy (ST-016).

## 4. Non-Requirements

- No merging, no recovery UI, no manual approval step, no wider startup refactor, no change to the readiness check.
- No edit of migrations 001–090; no key recompute for pre-alpha6 libraries (Q-001).
- No CLI backup (Q-002); no review cards for needs-review books (Q-004).
- The bad-legacy-author-key startup refusal stays out of scope (PA-005 decision 6).

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Pre-alpha6 libraries keep an older title-key recipe (ST-013). Support, or require alpha6 first? | parked — plan open question 1, not ruled in `DECISION.md` | Outside this scope (Astra r2 §4). |
| Q-002 | The CLI migrates, and now converts, with no backup. Leave it? | parked — plan open question 2, not ruled | Documented limit (plan §5; Astra r2 §4). |
| Q-003 | How common are the odd cases outside the four dev libraries? | parked — plan open question 3 | Measured: one audiobook cover choice; all others 0. |
| Q-004 | Should needs-review books get a review card instead of "Confirmed"? | parked — plan open question 4, not ruled | Not built here. |
| Q-005 | Does alpha6 "picked this search result" count as confirmed? | closed — PO, `DECISION.md:13` | Yes (REQ-001). |
| Q-006 | Automatic conversion; no merging; shared id stays with the first book and is logged; old questions closed keeping the current match. | closed — PO, `DECISION.md:11-13` | REQ-003, REQ-004. |
| Q-007 | Backups: when, and how long kept? | closed — PO, `DECISION.md:13`; retention accepted with plan r2 | Only when an upgrade or tracked repair is pending; kept until the next upgrade finishes (REQ-005). |
| Q-008 | Who writes the tests (Opus or Astra)? | parked — PO call at the build go (plan §7) | Not decided in `DECISION.md`. |
| Q-009 | A third plan round for R2-01..03? | closed — PO, `DECISION.md:7-9` | No; carried as required assertions (packet 1) and behaviour (packet 3). |

## 6. Acceptance Criteria

Tests live in `crates/livrarr-server/tests/test_identity_upgrade.rs` (new; register `[[test]]` in
`crates/livrarr-server/Cargo.toml` and `git add -f` with the manifest `fixtures/alpha6_migrations_073.sha384`).
The harness runs historical migrations 001–073 through `Migrator`, seeds synthetic alpha6 rows
(including alpha6's runtime `idx_works_identity`, ST-006) and starts the real binary. Faults are
database triggers (ST-018). Never plant the ready report or active marker; never use the empty-to-head helper.

- [ ] **AC-001** (REQ-001, REQ-003, REQ-006): test 1 `alpha6_library_upgrades_on_first_start`. `/health` 200; marker `active`; exactly one run, `automatic:activated`; exactly one pre-upgrade copy that opens as the seeded library; an authenticated work read shows the projected identifier and the Confirmed badge; the summary line's counts equal the stored summary; a second start logs no conversion and writes no copy.
- [ ] **AC-002** (REQ-001): test 2 `upgrade_keeps_user_picks_and_status`. Per kind, the user-confirmed anchor's route has `user_confirmed=1`, import and pending do not; statuses; one ordinal-0 contributor per book; title, author, monitor flags, metadata provenance and both cover slots unchanged; the second user's identical identifier stays with that user.
- [ ] **AC-003** (REQ-002): test 3 `upgrade_keeps_audiobook_cover_choice`. A `trust='user'` Goodreads audiobook cover ends with `audiobook_cover_manual=1`, URL, source, size and file bytes unchanged after the cover startup pass (await its observable completion, not only `/health`); machine-trust control manual 0. With a fault at 088: start refused, old column gone, saved list present; fault removed, next start restores manual 1.
- [ ] **AC-004** (REQ-001): test 4 `upgrade_trims_identifiers_like_the_app`. Tab/newline and NBSP copies of one value give one route with the trimmed value on the lower id and one shared-identifier audit on the other; the owner's padded user anchor still confirms it; whitespace-only values (including U+3000) give no route and no edition.
- [ ] **AC-005** (REQ-004): test 5 `upgrade_keeps_lookalikes_separate_and_first_book_keeps_shared_id`. Three books remain; two non-`common` distinctions; unique index present; shared value routed to the lower id only; one audit row for the other book.
- [ ] **AC-006** (REQ-004): test 6 `upgrade_closes_old_match_questions`. Open row `dismissed` with the note and `resolution_action` NULL; audit payload equals `incoming_payload_json`; closed rows unchanged; the book keeps its key; server serves.
- [ ] **AC-007** (REQ-003, REQ-004): test 7 `upgrade_finishes_an_abandoned_manual_attempt`. Historical migrator to 090, `run_identity_cutover(Apply, None)`, then binary: blocked run plus pending card before; server serves after; card `cancelled`.
- [ ] **AC-008** (REQ-003, REQ-005): test 8 `fresh_install_writes_no_copy_and_no_run`. First start: no automatic run, no copy; second start: no copy.
- [ ] **AC-009** (REQ-003): test 9 `active_library_skips_091`. Manual apply and activation at 090, then binary: 091 absent before and present after; identity tables and run rows unchanged; no new automatic run.
- [ ] **AC-010** (REQ-003, REQ-005): test 10 `failed_091_rolls_back_and_retries`. Fault at 091: start refused; 091 absent; no automatic run; identity tables as at 090; one copy. Fault removed: converts; still one copy, same checksum.
- [ ] **AC-011** (REQ-003, REQ-005): test 11 `retry_between_upgrade_and_activation`. Fault on the `identity_authority_v2` insert: 091 recorded; run `ready`; marker and unique index absent; not serving. Fault removed: `/health` 200; the same run `activated`; one automatic run; route, edition, contributor and audit counts unchanged; original copy kept, same checksum.
- [ ] **AC-012** (REQ-005): test 12 `original_copy_survives_partial_progress_prune_and_restore`. Faults at 078, 083, 091 in turn: one copy, same checksum; then success; with four older timestamped copies the prune keeps the original; a stale `.partial` is removed and never counted; starts refused at a planted author key add no copy; restore, edit a book, upgrade again: a new copy contains the edit and the earlier copy stays until that upgrade finishes.
- [ ] **AC-013** (REQ-005): test 13 `startup_repair_gets_one_copy`. Database at head with no startup generation: first start takes a copy predating the first repair (markers absent in the copy, present live); a refused start after some repairs committed reuses it, same checksum; a fully current library writes no copy.
- [ ] **AC-014** (REQ-005, R2-01): extension of test 12 (Astra r2 §3). Interrupt immediately before publication, immediately after, and before the `upgrade_snapshot` commit; reject that commit on several consecutive starts. Snapshot count stays bounded and associated with the attempt; the previous protected copy's checksum is unchanged; no migration or repair runs before association; an orphan left by an interrupted start is recovered or reclaimed on the next start; restore-and-edit still yields a snapshot with the edit.
- [ ] **AC-015** (REQ-005, R2-02): binary case extending test 12. Commit historical migrations, remove or relocate the original copy, restart: no fabricated pre-upgrade snapshot, no further migration or repair progress, a specific missing-original error. The no-progress retake-with-edit assertion is kept separately.
- [ ] **AC-016** (REQ-005, R2-03): extension of test 13. A partial cover pass commits queue/slot changes without its marker; restart; a later upgrade and prune: required originals stay protected with constant checksums; retries do not retake them from the mutated database; the disabled title repair does not block completion.
- [ ] **AC-017** (REQ-007): no automated test; checked at review. CHANGELOG, README (rollback-copy rule), wiki upgrade page and the decision record exist; any revised design doc has its prior snapshot in `docs/design-history/`.
- [ ] **AC-018** (REQ-008, REQ-002): packet 5, manual on the release-candidate image. Four real libraries with `--network none` start and convert; work 90 in the 178-book library ends with `audiobook_cover_manual=1`; a 10-minute restart loop keeps the copy count constant; rollback to alpha6 from the copy, an edit and a second upgrade give a new copy containing the edit.
- [ ] **AC-019** (REQ-008): re-run unmodified and green: `tests/behavioral/test_ilr_contracts.rs`, `tests/behavioral/test_irf_u5_durable_dismissal.rs`, `crates/livrarr-server/tests/test_fresh_author_index.rs`, `tests/behavioral/test_author_link_db.rs`; `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (zero warnings) and `cargo test` pass.
