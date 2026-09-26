# Tests & Fixtures Insights

Test-suite mechanics, shared test-only state, and fixture gotchas.

### 34. **RSS sync tests were masked**

34. **RSS sync tests were masked.** `cargo test` stops at the first failing binary. The author_monitor 429 test was hiding 3 pre-existing RSS sync test failures. Always run `cargo test` with `--no-fail-fast` to see all failures.

### 61. **The Phase-5 decision-diff harness carries FROZEN…**

61. **The Phase-5 decision-diff harness carries FROZEN old-side copies by design** (`tests/behavioral/test_p5_matching_diff.rs`): 15 line-for-line pre-Phase-5 duplicates (old string_similarity with both-empty→1.0, old normalize_title_variants colon cut, old find_matching_work cascade, old normalize_for_matching, …) so rewiring units cannot silently change what the instrument's OLD side measures — the sanctioned exception to the no-duplication rule. NEW sides call live code (identity_matching verdicts, identity_key). A fresh run must reproduce `build/reports/phase5-matching-diff-v2.{md,json}` byte-identically (volatile fields stripped); any deviation means the freeze drifted or the library changed — investigate before trusting either side.

### 65. **The behavioral suite compiles only what…**

65. **The behavioral suite compiles only what `crates/livrarr-behavioral/Cargo.toml` registers — 30 of 129 files in `tests/behavioral/` are NOT `[[test]]` targets and have never run** (found by Gemini in the work-service-split review, 2026-07-11; scope verified by manifest scan). The orphans are mostly superseded generations (`*_v21`, `test_metadata_redesign_phase*`, `test_verify_*`, `test_cup_convergence`, `test_db_*`, auth suites). Consequences: "the behavioral suite is green" means the 99 registered targets only; editing an unregistered file is never compile-checked (S1/S2b edited five of them blind before this was known). RESOLVED 2026-07-11 (`773afe8f`): 7 registered (incl. first-ever auth coverage), 3 parked on an explicit allowlist, 20 deleted (git-recoverable), and `test_manifest_guard.rs` now FAILS the suite on any present-but-unregistered file — the error class is closed. FULLY CLOSED later the same day (suite-consolidation session): the PARKED list is now EMPTY — test_verify_e2 ported (pins settle_identity persisting all anchors when ol_key is absent), test_cup_convergence and test_metadata_redesign_phase3a deleted (covered by test_id_completeness / drove the dead `AddWorkRequest` overload), and the registered test_verify_g2 deleted (PO decided Thin=settled is correct behavior, so its red gate was wrong-by-design). PO decisions same session: CI stays Docker-build-only (no cargo-test job — decided, don't re-raise); note the suite was ALWAYS tracked+public (the `tests/` gitignore line only blocks new files — `git add -f` for those). Workspace baseline: 1471 passed / 0 failed / 299 ignored / 143 suites. Full disposition record: `docs/orphan-test-triage-2026-07-11.md` §Resolution. **The guard does NOT cover the inverse class** — a file registered in Cargo.toml but never `git add -f`'d is invisible to it and to every local run, yet breaks `cargo test` on a fresh clone: `test_unified_identity_path.rs` + `test_uip_resolver_tied.rs` were registered by `8d66c7a3` but absent from origin until the responsiveness commit (2026-07-12) force-added them. Registering a new `tests/` file and force-adding it are ONE change. **The class includes FIXTURE files:** `tests/behavioral/fixtures/test_cover_100x150.png` (compile-time-read by test_n2_cover_write_gate.rs:712) sat untracked until 2026-07-13 — fresh clones could not compile the suite; any file a test reads at compile time needs the same `git add -f`. **And a whole DIRECTORY instance (found 2026-07-13, 2a session):** `tests/implementation/` (6 `test_impl_*.rs` files) is registered by NOTHING — no `[[test]]` entry in any Cargo.toml — and has never compiled or run; `test_manifest_guard` sweeps `tests/behavioral/` only. Do NOT author pins there; triage pass pending (mirror docs/orphan-test-triage-2026-07-11.md). Ripple: Wave 1's `create_test_notification` KEEP decision cited `test_impl_secondary.rs:377` as a live caller — that caller never compiles, so the helper is back on the dead-code candidate list.

### 83. **Never hand-roll tracing capture in a test**

83. **Never hand-roll tracing capture in a test.** A thread-scoped `set_default` subscriber races the tracing callsite interest cache against parallel sibling tests hitting the same callsite — the event is emitted and silently not captured (measured ~2/25 full-binary runs under CPU load; the U9 F06 helper had exactly this latent race, invisible while the test was red). Use `tracing-test`'s `#[traced_test]` (after `#[tokio::test]`): it installs ONE process-global subscriber and scopes assertions by a per-test span (`logs_assert` for exact counts). Integration tests in `tests/` MUST enable the `no-env-filter` feature (the default filter is `<calling crate>=trace` and drops events from the crate under test), and the proc-macro needs `tracing` itself as a sibling dev-dependency. Acceptance bar for any flake fix: repeated full-binary runs UNDER CPU LOAD, not one green run — and run newly-GREEN tests this way once at acceptance, because a red test's latent races only surface when it flips.

### 84. **`StubHttpFetcher` has a sticky tail**

84. **`StubHttpFetcher` has a sticky tail.** It pops the queue head only while more than one response remains, then serves the LAST response verbatim to every later caller (`stubs.rs` `next_response`; empty queue = default 200/empty). Queue every response up front at fixture setup; a response pushed mid-test lands BEHIND the leftover sticky tail and the next caller gets the stale body (that was U9 design-test 5's failure shape).

## 102. B0 red-test facts

Verified at source while writing the identity-conflict-authority B0 suites
(`tests/behavioral/test_ica_b0_{doors,contracts}.rs`, `ica_harness.rs`, 2026-09-01);
amended the same day after the IR errata (IR v1 r9 / IR v2 fold r4):

- Migration `087` is already the legacy-index drop (deletion pass) → the feature's schema begins at **088** (the errata fixed the IR and contract; `PRE_B0_LAST_MIGRATION = 87` in the upgrade fixture).
- `work_identity_conflicts.existing_work_id` is `ON DELETE CASCADE` (052). The design (errata E7) has migration 088 rebuild the table WITHOUT the works FK so the pre-B4 machine close (`status='resolved'`, `resolution_action='work_deleted'`, `resolved_at`, `resolution_notes='work deleted by identity authority'`) survives its Work; the pointer joins the effect manifest's bare-pointer registry (11 entries; closure stays 37 tables; 26 Work FKs).
- The live identity-edit close (`crates/livrarr-db/src/sqlite_work_identity.rs:1512`) writes `status='resolved'` with NO `resolution_action`; the deleted conflict page's dismiss wrote `dismissed` with none. A readability rule that requires an action on a resolved row misclassifies real history (caught on the errata; fixed in the IR).
- Only PendingRoute mints write a notification (`ref_key = identity-review-card:<id>`, `identityReviewNeeded`); GroupIdentity mints do not.
- `evaluate_captured_group` turns a text-certain cohort into `Review` when any existing member's `works.text_distinction != 'common'` — the cheapest way to drive the settlement engine's review park in a test (role `Incoming`, add source `search` for the DirectAdd door — `identity_road.rs` door→`WorkAddSource` mapping).
- The boot heals are generation-gated by `_livrarr_meta` keys (`identity_title_policy_generation`, `identity_dedup_residue_heal_generation`); a fixture must delete the key before driving a heal, and a refused pass must not write it (now a design-named observable: IR v2 `b0_refusal_no_write.heal_wrapper_rule`).
- The dedup heal's precondition is a pending GroupIdentity card whose proposal carries routes with the anchor first (`heal_identity_dedup_residue`); the deployed shape is a pre-B0 row in `SettlementReviewCard`'s serde bytes, adopted as role Unknown.
- The Readarr undo derives its orphan set by `import_id` WITHOUT tenant scope (`crates/livrarr-db/src/sqlite_import.rs:155-164`) — the one B0 door that can hand the delete authority another tenant's Work.
- `commit_pending_route_review` reports a standing dismissal as `Err(IdentityRepositoryError::StandingDismissal)`; assert that, never "zero pending cards" (any error leaves zero cards).
- The test `:memory:` pool is single-connection: any read issued after a `begin`-write on the same pool deadlocks. Read generations before opening the transaction.
- Patching rustfmt-formatted test files by text anchor fails after the first format (vertical args + trailing commas); patch by function scope with whitespace-insensitive matching (`scratchpad/patchlib.py`) or rewrite the file.
- "Zero writes" pins: the spec forbids ad hoc whole-state snapshots, but byte equality over a DESIGN-NAMED table set is the named observable for refused doors (IR v2 `b0_refusal_no_write`: authority 15 incl. `_livrarr_meta`; absorption = 15 ∪ closure 37). The harness snapshots every watched table and refuses to compare an unsnapshotted one (two absences compared equal — a silent pass).

Amended 2026-09-01 (first COMPILED run of the contracts suite, after the stub surface landed — five fixtures had never reached their door):

- **The legacy `create_work` writer fills NO identity columns** (`normalized_identity_main`, `primary_author_id`): a Work seeded with `CreateWorkDbRequest` is invisible to `list_captured_identities_in_group`, so nothing parks or merges against it. Seed existing members for identity-road fixtures through the production settlement writer (`WorkIdentityRepository::commit_settlement`; harness `seed_settled_work[_with]`).
- **The add-from-search request needs the user's `ExplicitCreate` choice** (`validate_road_request`: DirectAdd = HumanWatching ∧ choice ∧ no file); the production handler (`crates/livrarr-handlers/src/work.rs` `add`) sends choice + provider evidence + minimum. Without the choice the road answers `InvalidDoorEvidence`.
- **The live engine never returns `Review` for a machine request** (`livrarr-identity/src/identity_layer.rs` `decide`: not-certain ∧ HumanWatching → Review; not-certain ∧ MachineAlone → Defer). The road's "engine Review arm" (`decision.settlement == Review`, no route conflict → `GroupIdentity { work_ids: [existing_work_id] }`) is reachable only by a human request bound to an existing Work, and no production door sends such a shape today (list_service settles rows unbound; manual import binds without a minimum, so its title stays the Work's own). Its cohort is `[subject]`.
- **`idx_works_identity_v2`** (created by `ensure_identity_authority_ready` at F2 activation; columns user, normalized main/subtitle/volume, primary author, `text_distinction`) makes a second text-certain same-tuple member impossible (`commit_settlement` → `KeyCollision`); subtitle/volume variants are grey/different (Review), an audited distinction forces Review, a one-member group attaches. Consequences: the settlement AutoMerge absorption (a real loser) has no production fixture on the activated schema; the dedup-residue precondition can only be met by an orphan carrying a `text_distinction` (the CommitDifferent arm's write) — and the heal's candidate query does not filter on it.
- The pre-B0 production Dismiss decodes every pending card of the user: insert a corrupt-payload fixture row AFTER the pre-B0 writers have run.

Amended 2026-09-02 (F5 25-cell enumeration, U5 extensions, F4 fixture):

- **Clippy only lints the contracts suite under its feature flag.** `cargo clippy --workspace --all-targets` skips `test_ica_b0_contracts` (`required-features`); run it with `--features identity_conflict_authority_red` or "clippy 0" is a false green (7 warnings hid that way for a day).
- **The pre-B0 production Dismiss cancels every EQUIVALENT pending card** (same work_ids, no proposal) of the user — a Dismiss fixture on a shared member set silently cancels neighbouring census rows. Give such a fixture its own Work.
- **The live revoker (`identity_layer.rs:4316-4346`) filters by kind only** (`GroupIdentity`, `PendingRoute`) — it revokes proposal-bearing V1 group rows that fold r5 says to preserve. Tests pinning that rule are RED-UNTIL-B0 by design; IdentityConflict/IdentityPark/EditionEvidence rows already pass the kind filter.
- **Revocation doors and identity edits both write OL routes** — drive each door on its own Work in one fixture, or the affirm's active OL route collides with the edit's `sync_identity_edit_route`.
- **`services::WorkIdentityRepository::apply_identity_edit` is the provider-free way to reach `apply_identity_edit_in_tx`** (the certified-edit door needs an OL provider stub; the ICA harness registers none). Beware the two same-named traits: `identity_layer::WorkIdentityRepository` vs `services::WorkIdentityRepository` (alias the latter).
- **V1 ledger rows can be written by hand with the production V1 codec** (`ilr::ReviewDismissalKeyV1::canonical_json` + `kind().storage_code()`, `key_version 1`, membership rows from `work_ids()`) — the same calls the deployed writer makes, so the bytes are the deployed row shape.
- **Reconstruct a pending duplicate by copying every column** except id/created_at (status forced pending) via `pragma_table_info` — the B0 rebuild adds columns (origin, payload_version, card_version, resolution_*) — and copy the `identity_review_card_members` rows too.
- **A Serena restart replays queued edits** (memory `reference-serena-ops` signature E): after any restart, diff the stub tree's mtimes against the restart time before trusting "unchanged".

Amended 2026-09-03 (B0 M1 activation):

- **F2 readiness does not install the B0 repository.** `ensure_identity_authority_ready` only owns `identity_authority_v2='active'` and `idx_works_identity_v2`. Boot installs once on the lasting `SqliteDb` via `install_identity_authority_repository` (`tokio::sync::OnceCell`); the temporary post-migration handle must not install. Test DBs call F2 then install as two steps.
- **The B4 completion marker is not the F2 key.** F2 is `_livrarr_meta.identity_authority_v2='active'`. Completed-cutover state is `_livrarr_meta.identity_conflict_authority_b4` (JSON of counts/digests). Activation reads that key inside its `ImmediateTransaction` and constructs `MarkerAbsent` only when it is absent.
- **Do not install after `begin` on the single-connection test pool.** List undo must install (or find the installed Arc) before `BEGIN IMMEDIATE`; a schema inventory under an open writer times out and surfaces as `EffectManifestIncomplete`.

Amended 2026-09-04 (B0 M1 r2 / errata 5):

- **One installation owner per lasting root.** `SqliteDb::new` in production claims a process owner; clones share it; a second independent production root and the post-migration temporary handle cannot install (`ActivationError::SecondProductionRoot`). Test DBs use isolated owners. Activation-sensitive tests must start from `create_f2_ready_test_db_without_identity_authority` (or the namespaced sibling); `create_test_db` / `create_activated_test_db` still preinstall.
- **The B4 marker envelope is `marker_version` plus the full cutover report.** Key remains `identity_conflict_authority_b4`. Present-marker activation requires canonical compact JSON, exact version 1, 36 ordered adoption cells, recomputed report digest, and current table/inert counts. Do not put activation `seal_validation` in `report_digest`. Standard fixtures that preinstall hide activation; do not drive activation through them.

## 104. Cancellation test diagnostics

Accepted diagnostic lessons from the 2026-09-06 checkpoint; it was accepted as
INCOMPLETE evidence, not F4 implementation acceptance or complete cancellation coverage.

- **Capture the real task result before calling a missed barrier cancellation RED.**
  The MP3 retag task returned per-item failure before `AfterInvalidation`: its path
  had no file component below the Work root. The missing barrier was an early
  service refusal, not evidence of a cancellation hang.
- **MP3 fixtures need the audiobook Work directory.** Use
  `{user}/Writer Author/Cancel Work/{file}`; the ebook-shaped
  `{user}/author/file.mp3` omits the Work directory (`work_root_depth` is 3).
- **Blocking waiters need cleanup even when the barrier never fires.** Bound the
  arrival wait, make cleanup release blocked waiters, and join the waiter before
  asserting the result. A timeout alone does not clean up a blocking worker.
- **Drop the owned future before releasing the held lock.** After
  `tokio::pin!(invocation)`, `drop(invocation)` drops a `Pin<&mut Future>`, leaving
  its backing future alive. In the saved test, releasing the lock then let the
  waiter run. Put the owned future and its pin in a lexical scope that ends before
  lock release; establish cancellation before allowing the queued work to proceed.

Sources: [accepted checkpoint notes, I2 and queued-cancel paragraphs](../../build/reviews/identity-conflict-authority/IMPL-NOTES-B0-J-M3F4-CONT003.md),
[exact pre-removal test](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-003/evidence/CONT-jm3f4-003-i3-dropped-waiter-preremoval.rs.txt),
and [checkpoint receipt](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-003/RECEIPT.json)
(`i2-cancel-mp3.log`, `i2-cancel-mp3b.log`, `i3-domain-queued-cancel-3.log` under its evidence list).
The two passing logs establish only their named tests at those saved revisions.

## 105. Finish SQLite fixture setup before pausing Tokio time

Finish real asynchronous SQLite fixture setup before calling `tokio::time::pause()`.
In the moved constructor/sweeper test, `#[tokio::test(start_paused = true)]`
caused `PoolTimedOut` while `create_test_db` awaited pool connection, before the
target assertion. The preserved `i3-sweeper.log` records two passes and that
fixture failure; it is diagnostic evidence, not a sweeper assertion failure.

The corrected test awaits `create_test_db`, constructs the service, checks the
shared composition, then pauses time and advances 301 seconds. It retains the
real `SqliteDb`, `FileServiceImpl::new(db.clone())` and the database's own
`user_operation_scopes()` Arc. Keep that fixture when adjusting clock timing.
Both saved reruns (`i3-sweeper2.log`, `i3-sweeper3.log`) pass all three original
assertions: the constructor holds the live refresh-lock Arc, the sweeper survives
a full interval, and construction outside a Tokio runtime does not panic
(0.85s / 0.84s; both exit 0).

This is a local fixture lesson, not evidence that every paused-clock database
test fails. Checkpoint004 remains incomplete evidence: startup pre-scan, metadata
writer protocol, full ownership/type proofs and all seven pins' mutation evidence
remain open; these runs establish no implementation gate.

Sources: [accepted notes, I2/I3 and remaining work](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-004/evidence/IMPL-NOTES-B0-J-M3F4-CONT004.md),
[initial diagnostic](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-004/evidence/CONT-jm3f4-004-i3-sweeper.log),
[first rerun](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-004/evidence/CONT-jm3f4-004-i3-sweeper2.log),
[second rerun](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-004/evidence/CONT-jm3f4-004-i3-sweeper3.log),
and [checkpoint receipt](../../build/reviews/identity-conflict-authority/checkpoint-j-m3f4-session-004/RECEIPT.json)
(exact test and fixture source revisions are members of its `tree.tar`).
