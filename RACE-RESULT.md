# RACE RESULT — identity-edit implementation (side: FABLE)

- **Start:** 2026-07-24 20:40:16 UTC
- **End:** 2026-07-24 22:11:29 UTC (wall clock ≈ 1h 31m, single agent, solo, no subagents)
- **Token usage:** the harness exposes no /status readout to the agent; honest estimate from
  session scale (~95 tool calls, ~3.7k added lines, full contract + core-file reads):
  **≈ 450–550k tokens total** (input+output). Treat as ±30%.

## Gate results (actual tails)

**1. Durable suite** — `cargo test -p livrarr-behavioral --test test_identity_edit_durable`

```
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s
```

**2. Gated suite** — `cargo test -p livrarr-behavioral --test test_identity_edit --features identity_edit_red`

```
failures:
    collision_preview_unions_ledger_and_columns_without_cross_tenant_leakage

test result: FAILED. 32 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 36.71s
```

**32/33.** The one failure is a latent defect in the merged test file itself, escalated per the
packet's protocol in **`BLOCKED-collision-leak-assert.md`**: line 637 asserts the user-B response
contains the owner work id's string nowhere — the owner id is `1` in a fresh fixture DB, and the
same suite + design pin `"canonicalValue":"12345"` (contains `"1"`) into that very response, so
the assertion is unsatisfiable by ANY design-conformant implementation (it was only ever verified
compile-red, never green-run). The leak property it intends **is** enforced and is covered by the
adjacent asserts (`collision` absent :634, owner title absent :636). Proposed one-token fix in the
BLOCKED file; not applied unilaterally (assertions change only via escalation).

**3. Workspace** — `cargo test --workspace --no-fail-fast`

```
164 test binaries: 1835 passed; 0 failed; 297 ignored — all suites ok; exit code 0
```
(The known `goodreads_through_queue…` flake did not fire.)

**4. Format** — `cargo fmt --all -- --check`

```
exit 0, zero diffs
```

**5. Clippy** — `cargo clippy --workspace --all-targets`

```
Finished `dev` profile [unoptimized + debuginfo] target(s) — zero warnings ("No issues found")
```

**6. TypeScript** — `cd frontend && npx tsc --noEmit`

```
exit 0, no errors
```

**7. Frontend build** — `cd frontend && npx vite build`

```
✓ built in 4.96s   (pre-existing >500kB chunk-size advisory only; exit 0)
```

## What was built (per the r4 contract)

- **Migration 076** (`076_anchor_uniqueness_identity_generation.sql`) — the three verbatim ops
  only. 041/042/044 untouched (zero diffs under `migrations/`).
- **Durable `identity_generation`** — chokepoint bump inside `confirm_anchor_in_tx`; same-tx bumps
  in `raise_identity_conflict` (via new shared `raise_identity_conflict_in_tx`),
  `set_identity_pending`, `record_pending_anchor`; same-STATEMENT bumps on every raw
  `identity_status` arm (`set_identity_status`, `set_needs_review`, `set_identity_confirmed`,
  `set_identity_provisional`, review-guard statements); CASE-conditional bump in
  `reset_for_manual_refresh`'s NotFound recovery; `merge_works` first-statement double-bump
  requiring 2 rows.
- **Claimed doors** — pending affirm (`affirm_anchor_claimed`), review apply/dismiss
  (`*_claimed`), conflict resolve/dismiss (`apply_conflict_*_claimed` + coherent
  `get_identity_conflict_with_generation` read), each with a first-statement conditional
  generation claim mapping to the exact 409 envelopes
  (`pending_anchor_stale` / `identity_review_stale` / `identity_conflict_stale`).
- **Claimed delayed completion** — `WorkIdentityRepository::complete_anchors(work_id,
  expected_generation, IdentityCompletion)` applies merge-anchors / pending guesses /
  needs-review park / conflict raises / badge under ONE first-statement claim;
  `IdentityCompletionOutcome::Superseded` on a lost claim (zero writes). `settle_identity`
  builds one completion per verdict and submits it; convergence skips its `before_missing`
  dead-end accounting on Superseded; `complete_add`'s delayed NotFound conclusion rides the
  same claimed primitive.
- **Preview/commit/clear** — `classify_identifier_input` (the one paste authority, full
  precedence table incl. the 10-digit-checksum-invalid→GR rule and slot-hinted WrongSlot 422);
  `ProviderQueue::preview_fetch` → `EnrichmentService::preview_fetch` →
  `EnrichmentWorkflow::fetch_anchor_preview` → domain `IdentityPreviewRecord` (AC-25 boundary,
  livrarr-domain never names `NormalizedWorkDetail`); ordered per-slot legs (GB ISBN-echo
  verified); proven-agreement sibling bar via `title_id_trust` + author Agree, HC
  NotConfigured=keep (HC-only), bridges warn-only; ledger∪column same-user collision authority
  (`find_anchor_owner`, exclusion of the edited work, deterministic lowest-id);
  bounded snapshot store (4/user, 64 global, 10-min TTL, own-oldest eviction, cross-tenant 503
  `preview_capacity` + Retry-After); commit = atomic consume → generation-anchored true-no-op
  matrix → `apply_identity_edit` tx (CAS first statement, in-tx collision recheck,
  unique-violation backstop via `is_unique_violation` → re-lookup → typed Collision, supersede,
  chokepoint confirm, conflict closures incl. QuorumTie, exact drop-set with pending+dead-end
  residue deletion, edited-slot residue deletion, `merge_generation` bump, union badge);
  clear = user-scoped claim → empty→404 rollback → full residue clear + badge + parked flag.
- **Union badge** — `derive_badge_in_tx` now derives from open conflicts →
  confirmed-ledger ∪ **validated** columns (quarantined-invalid columns earn nothing);
  `read_identity_edit_basis` exposes the same projection + generation coherently.
- **Startup ledger completion** — `livrarr_db::backfill_work_identity_ledger(pool)` wired at
  main.rs step 9d (pre-service, no generation bumps): normalize-or-quarantine, canonical column
  rewrite, owner-preserving / lowest-id-deterministic work-key grouping, per-work bridges,
  atomic marker-last.
- **add_fast multi-bridge abstention** — collect ALL verdict-eligible bridge hits; exactly one
  adopts, two+ abstain to normalized dedup/create.
- **API error contract** — envelope gains optional `details` object
  (`code` / `owningWorkId` / `owningWorkTitle`); `ConflictDetailed`, `Unprocessable`,
  `ServiceUnavailableRetry` (+`Retry-After` header) variants; BUSY/LOCKED/FULL/IOERR/NOMEM
  classified 503 at the edit boundary; `WorkDetailResponse.parkedByConflicts` computed in the
  shared mapper.
- **Frontend** — preview-confirm modal (input → previewing → certifiable with keep/drop chips +
  causes + bridge warnings | collision → Merge-works handoff | unresolvable; 409
  `preview_required` recovery re-previews; 503 capacity messaging); pencil on GR/OL/ASIN rows,
  HC clear-only, ISBN row moved to Details (read-only + clear), Fix match button; client
  `ApiError` retains `details`; exact mixed-type invalidation keys
  (`["work", String(id)]`, `["works"]`, `["work", String(id), "pending-anchors"]`, numeric
  `["history", id]`); bounded post-save poll (1.5s × ≤6, early-stop on `enriching` handoff or
  blocking identity status); HistoryTab renders `"{action}: {identity}"` with skew fallbacks.
- **History door inventory** — edit + clear rows added to `contract-work-history.yaml` (26-row
  invariant, door-(d) empty-set enumeration amended per its standing obligation) and
  `ir-v2-work-history.yaml` doors (traced at the exact call lines; retrofitted doors' traced
  lines refreshed).
- **`supersede_anchor` deleted** (trait + impl + the one test double carrying it), per ground
  truth 3.

## Notable decisions / deviations to flag for review

1. **`complete_anchors` is ONE claimed primitive, not two.** The design's
   `merge_missing_anchors(user_id, work_id, expected_generation, incoming, target_badge)`
   re-signature is impossible without breaking the durable suite, which pins the CURRENT
   `merge_missing_anchors(work_id, &CapturedIdentity)` call shape
   (`test_identity_edit_durable.rs:322`). Kept the pinned method (bumps via the chokepoint) and
   implemented the claimed completion as a single `IdentityCompletion` struct covering all four
   write shapes — same coverage, one primitive.
2. **`settle_identity` performs the coherent `(Work, generation)` pair-read itself** (new
   `get_work_with_identity_generation`, one SELECT) instead of every caller passing
   `expected_generation`. Same invariant ("never pair stale anchors with a fresh generation"),
   enforced inside the one identity authority rather than at 10 call sites; the enumerated stale
   `Work` a caller holds contributes only the id.
3. **`record_pending_anchor` now advances the generation.** The gated CAS test
   (`repository_edit_cas…`, `test_identity_edit.rs:929`) uses it as the competing writer whose
   landing must stale a preview; the merge notes' rationale for dropping the codex durable
   pending-bump test reads the other way — the kept gated test wins (tests are the gate).
   Semantics are defensible: a landed guess is affirmable state a certifying user must see.
4. **Outbound-queue dispatcher resurrection** (`livrarr-http/src/outbound_queue.rs`): the
   process-global lane's dispatcher task dies with whichever test runtime spawned it, stranding
   already-queued waiters from other runtimes forever (the existing `DispatcherGuard` clears the
   flag but nothing respawns for queued items). Waiters now probe every 500ms and resurrect the
   dispatcher on their own live runtime, and a torn hand-off (dispatcher died between pop and
   send) re-enqueues. Pre-existing hazard, first exposed by this suite's ~20 concurrent
   real-HTTP tests; production (one runtime) is behaviorally unchanged — the probes are no-ops.
5. **Gated-file mechanical compile fix** (assertions untouched): the CC-merged backfill test's
   `owner_rows` closure moved `db` and was called four times (E0382 — the file had only ever
   been verified compile-red). The closure now clones the handle; every assertion is
   byte-identical. Flagged rather than silent.
6. **One escalated assertion** — see `BLOCKED-collision-leak-assert.md` (unpassable
   contains-check, evidence + one-token proposed fix).
7. Preview runs the collision check **before** sibling fetches (design lists sibling assessment
   first): a blocked preview never spends sibling/bridge provider calls. Response content for
   the tested cases is unchanged.
8. Bridge-slot edits (isbn/asin) skip sibling assessment and the collision block entirely —
   ratified doctrine (bridges never drop work keys; same-user sharing legal post-076). The
   informational same-user-sharing list for bridges is not built (nothing pins it); noted as a
   possible polish item.
9. Storage-taxonomy 503s (BUSY/FULL/IOERR) return the plain 503 envelope without a `details`
   code — the design's stable-code list has no storage code; `preview_capacity` keeps its code +
   Retry-After.
10. Deferred exactly the packet's OUT list (gated-file header): AC-24 global-saturation arm,
    AC-4 OL/HC sibling fixture arms, AC-12 BUSY/FULL injection, FE vitest/Playwright. FE
    production code is fully in scope and shipped.

## Post-race follow-ups I'd suggest

- Apply the one-token BLOCKED fix and re-run the gated suite to 33/33 (everything else is green).
- The deferred test clusters above, plus a vitest pass over the new modal.
- `hc_work` DELETE returns the standard DTO like every slot; the design's residual list about
  `settled_anchor_types`' conservative column view is unchanged (accepted residual, not "fixed").
