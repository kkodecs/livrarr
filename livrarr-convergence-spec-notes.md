# Convergence via Unified Path — SPEC-PREP NOTES

> **These are notes, not a spec.** PO call 2026-06-18. The actual spec is generated through the
> kk-build process flow when this feature is started — do NOT treat this file as the spec. It exists so
> the spec author (likely a fresh session) inherits the grounded findings and the decided constraints
> instead of re-deriving them.

## The decision (PO, 2026-06-18)

Restore **automatic** convergence of identity-pending works by wiring everything through the **unified
pipeline** — in two sequenced parts:

- **Part 1 (this feature, next):** wire convergence through the unified path *now*, reusing the existing
  backoff-aware primitives. Turns automatic finishing back on.
- **Part 2 (fast-follow, separate feature):** make the throttling / rate-limit / quota layer fit for
  purpose (the foundational piece — see the 2026-06-14 consolidation call,
  memory `project_identity_pipeline_restructure_needed`).

**Acknowledged & accepted risk:** Part 1 is a half-solution until Part 2 lands. Acceptable —
*provided Part 1 ships with the interim guardrails below* (the line between "acceptable half-solution"
and reintroducing the bug that got the old converger deleted).

## The problem (plain)

M9 (`wiki/domain/metadata-principles.md`) promises that batch/monitor-created `identity-pending` works
converge in the background — *"never silent limbo, never an indefinite retry loop."* The background
converger was deliberately deleted (`ff60e7f`, metadata-refactor S6, 2026-06-09) for three then-valid
reasons: it was off-road (wrote covers/tags outside the one pipeline → closed REQ-001), it churned
Google Books daily quota via a retry loop, and at that moment every door resolved identity
synchronously so it had nothing to converge. That third premise broke afterward: Sprint-C/D
re-introduced pending-work creation (series/author monitors, Readarr + list import) without restoring
convergence → silent-limbo regression (insight 54).

## Scope — where this applies

**Applies to** books the app adds *for you* that it couldn't fully identify at add-time:
author monitor, series monitor, list import, Readarr import. **Plus** the existing backlog of works
already stuck `identity-pending`.

**Does NOT touch:** hand-added works (Add Work / manual per-file import — already complete), or works
whose identity was already pinned cleanly at add-time.

## Part 1 — convergence via unified path (interim guardrails = acceptance criteria)

The mechanism is "make a list of due pending works → run them through the unified pipeline" (the same
thing the existing manual **Retry Incomplete** button already does — build the automatic version on
that path). To keep the interim genuinely safe **without** Part 2's throttle fix, Part 1 MUST:

1. **Select via the backoff clock, not "all pending every tick."** Use the due-for-retry query that
   respects each work's `next_attempt_at`, so a work isn't re-hit rapidly.
2. **Identity-only convergence** (the light step), not the heavy full enrichment re-fetch.
3. **Dead-ends → Needs-Review, not loop.** Already built into `converge_identity_pending` (REQ-026).
4. **Route through the unified pipeline** (one road) — no side-doors (the REQ-001 sin that killed v1).
5. **Bounded batch per tick** + conservative cadence.

With 1–5 the interim makes a bounded, backoff-paced number of calls even before the proper throttle.

**Lock as the non-negotiable acceptance criterion:** *no naive "re-sweep all pending every tick" loop.*
That single line is the difference between the accepted half-solution and re-creating the deleted bug on
a live library.

## Part 2 — throttle / quota governance (fast-follow, separate feature)

A single governance layer owning: per-provider **rate** + **daily/quota budget** + **global vs.
per-instance** + **defer/resume (WillRetry)** semantics. Consolidate the ≥3 fragmented rate-limit
locations; resolve the field documented "not enforced by the queue runtime in this phase"; add the
daily-budget awareness that does not exist today. This is the real foundation; once it lands, automatic
convergence is safe at scale and the per-instance GR over-pacing is fixed.

## Grounded code anchors (this session, 2026-06-16/18 — verify before asserting as still-current)

**Convergence (Part 1):**
- `converge_identity_pending` — `crates/livrarr-identity/src/async_resolver.rs:21–86`. Exists, **zero
  referencing symbols** (orphaned). Re-runs `resolve(Background)` + anchor-merge; dead-end → NeedsReview
  not loop (REQ-026); user-resolved Conflict never re-litigated (REQ-025). *Verified live this session.*
- `list_works_due_for_retry` — trait `livrarr-db/src/lib.rs:2176`, impl `sqlite_retry_state.rs:281`.
  Exists, zero callers; respects `next_attempt_at`. *(From stage-0 audit transcript; spot-verify.)*
- `enrichment_retry_tick` — **DELETED** in `ff60e7f`; no live equivalent. *Verified (symbol absent).*
- `retry_all_incomplete` (the "Retry Incomplete" button) — user-triggered single-pass sweep through the
  unified pipeline; `services/work.rs` ~310, `POST /work/retry-incomplete`. The path to build on.

**Throttle / rate-limit (Part 2):**
- `TokenBucket` GCRA limiter — `crates/livrarr-enrichment/src/provider_queue.rs:220`. Enforces
  `requests_per_second` only (no daily counter). *Verified.*
- `RateBucket` enum (per-provider) — `crates/livrarr-domain/src/services/http.rs:10`: OpenLibrary,
  Hardcover, Audnexus, Goodreads, GoogleBooks. *Verified.*
- `RateLimitContract` / `DefaultRateLimiter` — `crates/livrarr-http/src/rate_limit.rs:14,23`.
- `ProviderQueueConfig.requests_per_second` — `crates/livrarr-enrichment/src/lib.rs:195`, documented
  *"Not enforced by the queue runtime in this phase."* *Verified (the doc string).*
- **No daily-quota mechanism anywhere** — zero `quota` symbols in the codebase. *Verified.*
- **GR throttle is per-fetcher-instance, no global cap** — `HttpFetcherImpl::new()` called 12+ times in
  `main.rs`, each a fresh `RateLimiterMap`; flagged ~5–7× over GR's polite rate; intended global pacer
  `LivePacingQueue` flagged unbuilt. *From the 2026-06-14 grounding (memory) — NOT re-verified this
  session; re-trace at Part-2 spec.*

## Open questions to resolve at spec time (do NOT answer now)

- What actually limits live traffic today across the ≥3 rate-limit locations? (Trace before Part-2 spec.)
- Part-1 sweep cadence + batch size; worst-case total daily call volume without Part 2's quota layer.
- Does Part 1 also surface the pending state in the UI (M9's "surfaced, never silent" half)?
- Does the identity-resolution path tag GR calls with `RateBucket::Goodreads`? (Unconfirmed in memory.)
- **G2 (in-scope for Part 1):** `retry_all_incomplete` excludes `EnrichmentStatus::Thin` — decide whether the convergence sweep re-attempts Thin ("known book, no metadata") works.

## Gated tests — known bugs parked behind `#[ignore]` (2026-06-18)

Two local behavioral tests assert correct-but-unimplemented behavior; `#[ignore]`'d so the suite is
green-when-clean. Run with `cargo test -p livrarr-behavioral -- --ignored`.
- **`test_verify_g2`** — Thin works skipped by `retry_all_incomplete` (see open question above).
  **In-scope** for the convergence feature.
- **`test_verify_d2`** (case `same_isbn_different_title_must_dedup_not_error`) — a second add of the same
  ISBN with a subtitle-variant title errors with a UNIQUE-constraint Validation (500) instead of graceful
  dedup. **Standalone dedup bug**, not convergence-scope; small, fix when convenient. The fix must still
  respect the AC-020 wrong-book guard (same ISBN + flatly-disagreeing titles must Conflict, not merge —
  insight 52).

## Pointers

- Decision lineage: memory `project_identity_pipeline_restructure_needed` (2026-06-14 consolidation call).
- Regression detail + door matrix: insight 54; `wiki/architecture/work-creation-pipeline.md`.
- The principle: M9 in `wiki/domain/metadata-principles.md`.
- Why the old converger died: commit `ff60e7f` (metadata-refactor S6).
- Stage-0 audit verdicts: `livrarr-stage0-verification-results.md` (V4 provider map, V6 primitives).
