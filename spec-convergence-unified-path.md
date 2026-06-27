---
feature: convergence-unified-path
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007]
---

# Spec: convergence-unified-path (Part 1)

> **⚠ SUPERSEDED 2026-06-23 by `spec-id-completeness.md`** — that feature absorbs + WIDENS this Part-1 scope
> (it wires every door + adds the safe ID-harvest, and widens this loop's selector). The loop's
> selection/pacing mechanics in `design-convergence-selection-fix.md` are REUSED, not superseded.
> **Build from `spec-id-completeness.md`, not this doc.**

> **Scope marker.** This is **Part 1** of the 2026-06-18 PO convergence decision: wire **automatic**
> convergence of identity-pending works through the **unified pipeline**, reusing existing primitives.
> **Part 2** (throttle / quota governance) is a separate, later feature and is an explicit non-requirement
> here (§4). Accepted interim risk: Part 1 is a deliberate half-solution that is safe **only because** the
> guardrails in §2 hold (the unbounded sweep is exactly what got the prior converger deleted).

## 0a. Design Principles

Choices committed to. If a requirement conflicts, the principle wins.

- **One road, no side-doors.** Convergence routes only through the unified pipeline / shared async
  resolver. It never writes covers, tags, or other artifacts via an ad-hoc path. (Off-road writes were
  one of the three reasons the prior converger was deleted — ST-003.)
- **Backoff-paced, never naive.** Works are selected by the due-for-retry backoff clock
  (`next_attempt_at`), never "re-sweep all identity-pending works every tick." This is the single
  non-negotiable line separating the accepted half-solution from re-creating the deleted bug.
- **Reuse over rebuild.** Wire the existing, tested primitives (`converge_identity_pending`,
  `list_works_due_for_retry`). Do not author a new converger. (V6: ~50–80 lines of wiring, not new logic.)
- **Complete-per-work, pending-only (Q-001 = B).** Each due pending work runs the **full unified pipeline**
  (identity → enrich → materialize) so it ends complete. The sweep still targets `identity-pending` works
  only — it never re-enriches already-complete (Confirmed) works and never sweeps Thin. Because B fans a
  full provider scatter per work and there is **no quota net** (ST-004), the bounded batch + conservative
  cadence (REQ-006) and the worst-case-volume bound (Q-002, now a pre-activation safeguard) carry the safety.
- **Terminal honesty.** A dead-end becomes a terminal, surfaced needs-review — never an indefinite loop
  (M9; reuses the resolver's existing REQ-026 behavior).
- **The guardrails ARE the safety net.** No daily-quota or global-rate layer exists yet (ST-004), so the
  bounded batch + conservative cadence + backoff selection are the *only* volume protection until Part 2.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `wiki/domain/metadata-principles.md:37–48` (M9), read this session | Batch / background paths (list import, Readarr, series/author monitors) MAY seed `identity-pending` works that **must converge to full identity in the background** via the shared async resolver (REQ-022/026); an unresolvable item transitions to a **terminal, surfaced** needs-review — "never silent limbo, never an indefinite retry loop." | Leaving identity-pending works with **no automatic convergence path** (the current silent-limbo regression, insight 54). | High |
| ST-002 | `converge_identity_pending` @ `crates/livrarr-identity/src/async_resolver.rs:21–86` (signature + docstring read this session; `find_referencing_symbols` → **empty**); `list_works_due_for_retry` trait @ `livrarr-db/src/lib.rs:2175`, impl @ `sqlite_retry_state.rs:280`, returns `Vec<(WorkId, MetadataProvider)>` ordered by `next_attempt_at` | Both primitives are **built, tested, and orphaned** (zero production callers). `converge_identity_pending` already re-runs `resolve(Background)` + anchor-merge toward the full federated anchor set (REQ-022), transitions a Tier-B dead-end to **NeedsReview not loop** (REQ-026), and **never re-litigates a user-resolved Conflict** (REQ-025). Convergence is *wiring*, not new logic. | Authoring a new converger or new dead-end / conflict logic. | High (read myself, 2026-06-19) |
| ST-003 | Insight 54 (quoted this session) + spec-notes; commit `ff60e7f` (metadata-refactor S6) | The prior background converger (`enrichment_retry_tick`) was **deliberately deleted** for three reasons: (a) it wrote covers/tags **off-road**, (b) it churned **Google Books daily quota** via an unbounded retry loop, (c) at the time every door resolved identity synchronously so it had nothing to converge. Premise (c) broke when Sprint-C/D re-introduced pending-work creation. | Reintroducing an **unbounded re-sweep** or **side-door artifact writes** — the exact two live hazards. | High on the three reasons (insight 54); commit `ff60e7f` not personally re-read |
| ST-004 | Spec-notes grounding 2026-06-16/18 — **NOT re-verified this session** | There is **no daily-quota and no global rate-cap mechanism** in the codebase today; the existing GCRA `TokenBucket` enforces per-second rate only, per-fetcher-instance. | Designing Part 1 as if a quota/throttle safety net exists. It does not — until Part 2. | Medium (prior grounding, not re-verified) |

## 1. Problem Statement

M9 (ST-001) promises that batch/monitor-created `identity-pending` works converge to full identity in the
background. That convergence **no longer runs**: the recurring background sweep (`enrichment_retry_tick`)
was deleted (ST-003), and its only replacement — the **Retry Incomplete** button (`retry_all_incomplete`,
`services/work.rs:310`) — is user-triggered, single-pass, and reachable only from a POST route. Meanwhile
Sprint-C/D re-introduced doors that **seed Pending by design** — Series Monitor
(`series_query_service.rs:815`), Readarr Import (`readarr_import_workflow.rs:1248`), and List Import on
resolver error (`list_service.rs`). The result is the **silent limbo M9 forbids** (insight 54): a
series-monitored or Readarr-imported book sits `identity-pending` indefinitely unless the user manually
clicks Retry — a binding-principle (REQ-022) regression that produces divergent end states.

The fix is small and already-built: re-wire the orphaned primitives (ST-002) into an automatic,
backoff-paced sweep through the one pipeline. The danger is equally specific: doing it as a naive
re-sweep re-creates the quota-churning bug that justified the deletion (ST-003). Part 1 delivers the
automatic convergence **with** the interim guardrails; Part 2 (separate feature) adds the throttle/quota
governance that makes it safe at scale.

## 2. Requirements

- **REQ-001 — Automatic convergence exists.** The system MUST, **without user action**, periodically
  attempt to advance `identity-pending` works toward resolved identity by running them through the unified
  pipeline's shared async resolver. (Restores M9's auto-convergence leg.)

- **REQ-002 — Backoff-clock selection (the locked guardrail).** Convergence MUST select works via the
  due-for-retry backoff clock (`next_attempt_at`), attempting only works currently **due**. It MUST NOT
  re-attempt all identity-pending works on every tick. It MUST also **advance** `next_attempt_at` for any
  work it attempts that does not complete, so an unresolvable/transiently-failing work is not re-swept
  every tick (no loop, no front-of-queue starvation). *This is the non-negotiable acceptance criterion.*
  *(r2: the advance-on-attempt clause was added after review finding R-001.)*

- **REQ-003 — Incomplete-only, complete-per-work (Q-001 = B; amended r2).** The sweep targets works that
  are still **incomplete** — identity `Pending`, OR identified (`Confirmed`/`Provisional`) but enrichment
  unfinished (`Failed`/`Unenriched`). It MUST NOT sweep already-complete works, `Thin` works, or
  terminal-identity works (`NeedsReview`/`NotFound`/`Conflict`). Each targeted work runs the **full unified
  pipeline** (identity → enrich → materialize) so it converges to a **complete** state. *(r2: broadened
  from pending-only so a work whose identity resolves but whose enrichment transiently fails is retried
  automatically rather than stranded — review finding Codex-R-001. Thin re-attempt — the G2 question —
  remains an explicit non-requirement, §4.)*

- **REQ-004 — One road, no side-doors.** Convergence MUST route through the unified pipeline / shared
  resolver. It MUST NOT write covers, tags, or other artifacts through any path outside that pipeline.

- **REQ-005 — Terminal honesty, no loop.** A work the resolver cannot resolve MUST transition to a
  terminal **needs-review** state and MUST NOT be re-attempted indefinitely (reuses the resolver's REQ-026
  dead-end behavior). A user-resolved Conflict MUST NOT be re-litigated (REQ-025).

- **REQ-006 — Bounded & conservative.** A single tick MUST process at most a bounded batch of works, and
  the cadence MUST be conservative. Both bounds are configurable (TOML; no env overrides) with conservative
  defaults. These guardrails are the only interim volume protection (ST-004) until Part 2.

- **REQ-007 — Pre-activation volume safeguard.** Before the automatic sweep is enabled on a live library:
  (a) the worst-case daily provider-call volume for the configured batch + cadence — **including the
  first-activation drain of the existing identity-pending backlog** — MUST be computed and shown to stay
  within provider limits, with **Google Books' hard daily quota** (ST-003/ST-004) as the binding
  constraint; and (b) the live database MUST be snapshotted before first activation.

## 3. UI/Interface Design

**No UI.** Part 1 is backend convergence only (PO decision, 2026-06-18). Surfacing the pending /
needs-review state in the UI — M9's "surfaced, never silent" half — is **out of scope** (§4). Whether that
surfacing already exists is a separate verification item, not this feature's deliverable.

## 4. Non-Requirements

Explicit scope exclusions.

- **Part 2 — throttle / quota governance.** No daily-quota counter, no global rate cap, no consolidation of
  the ≥3 fragmented rate-limit locations, no per-instance→global GR pacing fix, no defer/resume (WillRetry)
  semantics. Separate later feature.
- **Re-enrichment / refresh of already-complete works.** Convergence first-time-enriches *pending* works
  (Q-001 = B); it never refreshes or re-enriches works that are already Confirmed / complete.
- **Thin-work re-attempt (G2).** `retry_all_incomplete` excludes `EnrichmentStatus::Thin`; the convergence
  sweep does **not** re-attempt Thin ("known book, no metadata") works. `test_verify_g2` stays `#[ignore]`'d.
- **UI surfacing** of pending / needs-review state (backend only, §3).
- **Interactive / hand-added works.** Add Work and manual per-file import already resolve synchronously
  (M9 interactive tier); convergence does not touch them.
- **A new converger.** Reuse `converge_identity_pending` + `list_works_due_for_retry`; do not rebuild.
- **The `d2` dedup bug** (same-ISBN subtitle-variant → UNIQUE-constraint 500). Standalone, not convergence
  scope; `test_verify_d2` stays `#[ignore]`'d.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Stop at identity, or run the full unified pipeline per work? | **resolved → B** (PO, 2026-06-19) | Full unified pipeline per due pending work (identity → enrich → materialize); each converges to complete — the **automatic version of *Retry Incomplete*** (`retry_all_incomplete`'s full-pipeline path) gated by due-for-retry selection. **Build on the unified pipeline, not `converge_identity_pending` alone** (that primitive is identity-only = option A). **Consequence:** B fans a full provider scatter per work with **no quota net** (ST-004), so the worst-case daily-volume bound (Q-002) is a **blocking pre-activation safeguard**, not architecture-deferred. |
| Q-002 | Tick **cadence + batch-size** defaults, and the computed **worst-case daily provider-call volume** without Part 2's quota layer. | in progress (REQ-007) | Now a **blocking pre-activation safeguard** (REQ-007), not design-deferred. Backlog measured this session (2026-06-19); architecture sets conservative batch/cadence so worst-case daily GB calls (incl. first-activation backlog drain) ≤ GB quota. |
| Q-003 | Does the identity-resolution path tag its provider calls with the correct `RateBucket` (e.g. `Goodreads`) so the existing per-provider GCRA limiter actually paces them? | open | Verify at **architecture** (trace the resolver's call path). |
| Q-004 | `list_works_due_for_retry` returns `(WorkId, MetadataProvider)` keyed on **provider-retry-state**. Confirm this set equals "identity-pending works due," or whether an identity-pending-specific due filter is needed. | open | Resolve at **architecture** (the selection-source seam). |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Given an `identity-pending` work whose `next_attempt_at` is due, when a
  convergence tick runs, `converge_identity_pending` is invoked for it and — on a resolvable identity — its
  `identity_status` advances out of `pending`.
- [ ] **AC-002** (REQ-002): Given two identity-pending works, one **due** and one **not yet due** per
  `next_attempt_at`, a tick attempts **only the due one**; the not-yet-due work is untouched. *(The locked
  guardrail, expressed as a test.)*
- [ ] **AC-003** (REQ-002): No code path selects identity-pending works for convergence while **ignoring**
  `next_attempt_at` (no "all pending, every tick" query). *(Contract forbidden-pattern + review.)*
- [ ] **AC-004** (REQ-003): A `Confirmed` work and a `Thin` work are **not** selected or attempted by the
  convergence sweep.
- [ ] **AC-005** (REQ-003): A due pending work that resolves runs the full pipeline and ends **complete**
  (identity resolved **and** enrichment + materialize applied), not merely identified.
- [ ] **AC-006** (REQ-004): During a convergence tick, no cover or tag write occurs through any path
  outside the unified pipeline (no side-door writer invoked).
- [ ] **AC-007** (REQ-005): An identity-pending work whose resolver dead-ends transitions to terminal
  `needs-review` and is **not** re-attempted on the following tick (no loop).
- [ ] **AC-008** (REQ-005): A work with a user-resolved Conflict is not re-litigated by a tick.
- [ ] **AC-009** (REQ-006): Given more due identity-pending works than the batch bound N, a single tick
  attempts at most N; the remainder are picked up on subsequent ticks (still backoff-paced).
- [ ] **AC-010** (REQ-006): Cadence and batch bound are read from TOML config (no env override) with
  conservative defaults; a sub-minute or unbounded default is a failure.
- [ ] **AC-011** (REQ-001/004 — M9 invariant): A work that converges reaches the **same identity** a
  synchronous door would have produced for the same inputs (REQ-022 "same destination, not same clock").
- [ ] **AC-012** (REQ-007): The chosen batch + cadence are documented with the computed worst-case daily
  Google Books call count (incl. first-activation backlog drain) shown ≤ the GB daily quota; and a DB
  snapshot is taken before the sweep is first enabled on the live library.
