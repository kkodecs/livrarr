# Work-Creation Pipeline — the five phases and the M9 convergence gap

> As-of 2026-06-14. Grounded by reading the code that session (door call sites,
> `identity.rs`, `work_service.rs`) and principle **M9** (`domain/metadata-principles.md`).
> Code + principles win over this page; fix on drift.
>
> **⚠ The line numbers below are from 2026-06-14 and have drifted.** Every one spot-checked
> against the current tree was wrong: `resolve_identity` is `work_service.rs:397` (not 843),
> `WorkService::add` is `:332` (not 421), `derived_identity_status` is `identity.rs:407` (not
> 318). Those three are corrected in place. The door table's citations and the phase-4/5
> pointers below are **unverified** — treat them as symbol names to search for, not as
> locations. `series_query_service.rs` is now a directory
> (`series_query_service/service.rs`).

## The five phases

A book entering the library moves through five phases, in order:

1. **Identify** — determine which real-world work this is. `WorkService::resolve_identity`
   (`work_service.rs:397`) runs a multi-provider quorum and returns a badge: `Confirmed`
   only when a work-level anchor (OL/GR/HC key) is corroborated, `Provisional` for an
   ISBN/ASIN bridge alone, else `Pending` (`identity.rs:407` `derived_identity_status`).
2. **Create the seed** — build the `WorkCandidate` through the single seed factory
   (`livrarr-domain/src/seed.rs`). Each door calls its `seed_*` constructor, which stamps
   the phase-1 identity plus title/author/language. seed.rs does **not** identify — it
   takes identity as an input.
3. **Create the work** — `WorkService::add` persists the work row (`work_service.rs:332`).
4. **Capture metadata (enrich)** — `run_unified_enrichment` fans out to providers and
   merges fields. **Gated on identity:** a Pending/Conflict/NeedsReview work returns
   *without* enriching (`work_service.rs:2737`). No identity → no metadata.
5. **Materialize** — persist the artifacts (download cover to disk, decode dimensions;
   `livrarr-materialize`, `work_service.rs:3109`). A "best-in-hand" cover is materialized
   even for a held identity (`work_service.rs:828`).

**Key chain:** phase 4 is gated behind phase 1. Skip identify and the work is created (3)
but stays an empty shell — no metadata (4), only a best-in-hand cover (5).

## Where each door identifies (phase 1)

All doors use the unified seed factory (phase 2). They differ only in **where** they
resolve identity:

| Door | Identify step | Badge | Source |
|---|---|---|---|
| Direct Add (search / GR-link) | resolve @ Interactive | real | `work.rs:198,213` |
| Manual Import (scan review) | resolve @ Interactive | real | `manual_import.rs:1078,1093` |
| List Import (CSV) | resolve @ Background (per row) | real; Pending only on resolver error | `list_service.rs:76,91` |
| Author Monitor | none — stamps Confirmed from its OL work key | Confirmed | `author_monitor_workflow.rs:511` |
| **Series Monitor** | **none — seeds Pending** | **Pending** | `series_query_service.rs:815` |
| **Readarr Import** | **none — seeds Pending** | **Pending** | `readarr_import_workflow.rs:1248` |

## What is — and isn't — a bug here (read M9 first)

**Principle M9** (`domain/metadata-principles.md`) governs this and is explicit:

- Interactive paths are synchronous / fully-formed.
- **Batch/background paths (list import, Readarr import, series/author monitors) MAY
  create an `identity-pending` work that converges to full identity via the shared async
  resolver (REQ-022, REQ-026).** Unresolvable items go to a **surfaced** terminal
  `needs-review` — *"never silent limbo, never an indefinite retry loop."*
- **Binding invariant (REQ-022):** every path converges on the same identity for the same
  work.

Therefore:

- **Seeding `Pending` is BY DESIGN, not an oversight.** M9 explicitly permits
  series-monitor / Readarr / list-on-error to do it. (An earlier pass of this analysis
  called it "oversight" — that was wrong, corrected against M9.)
- **RESOLVED (verified 2026-07-25 against `e864c485`). The defect this page diagnosed —
  "convergence no longer runs automatically" — has been fixed.** A recurring background
  sweep exists and is registered as a scheduled job:
  - `convergence_tick` (`crates/livrarr-server/src/jobs/convergence.rs:27-123`), spawned by
    the job scheduler as `"convergence"` (`crates/livrarr-server/src/jobs/mod.rs:129-135`).
  - **Enabled by default**; `[convergence] enabled = false` opts out
    (`crates/livrarr-server/src/config.rs:150-154`, `:185-187`). Defaults: 1-hour cadence,
    batch 25 per user per tick, dead-end `attempt_threshold` 3 (`:189-199`).
  - Each tick walks every user, selects works due (`list_convergence_due`), runs one
    `converge_work` pass each, and paces the next attempt via `next_convergence_at`
    (`convergence.rs:37-119`).
- **The "no surfaced `needs-review`" half is also fixed.** `converge_work` terminalizes a
  hopeless Pending work to `NeedsReview` on the first pass — when it holds no hard anchor to
  resolve from, or every still-missing anchor is dead-ended
  (`crates/livrarr-metadata/src/convergence_service.rs:78-97`). That is M9's surfaced
  terminal, not silent limbo.
- **`bulk_resolver::resolve_bulk` no longer exists.** `crates/livrarr-identity/src/` contains
  `async_resolver.rs`, `english_identity_resolver.rs`, `title_cleanup.rs` and `lib.rs` — no
  bulk-resolver module. `LatencyTier::Bulk` survives as a pacing tier
  (`crates/livrarr-domain/src/identity.rs:142`), explicitly "not a mode" (`:300-304`).

### Secondary gap (quality, not correctness)

Per the `LatencyTier::Bulk` design, Readarr was meant to resolve **at the door** via the
bulk resolver; it never was, so it dumps every work to Pending. Series-monitor likewise
never resolves at the door. List Import demonstrates the resolve-at-door pattern working.
Wiring these two to resolve at the door would shrink the Pending population — but the
**correctness** fix is restoring convergence, not changing where they resolve.

## Fix pointer — CLOSED

> The two compliant directions this section proposed have **both** shipped: (a) a bounded
> recurring convergence sweep, per-user and cancellation-aware, and (b) a surfaced
> `needs-review` terminal. Citations in the RESOLVED bullets above. Nothing here is
> outstanding; the original text is kept below only as the record of what was asked for.

This was the **Sprint E prerequisite (#144 remainder)** plus the owed
**dead-background-convergence** PO decision. Because M9 makes convergence binding,
"manual-only / do nothing" is **not** M9-compliant (it leaves silent limbo). Two compliant
directions: (a) restore a bounded recurring convergence sweep (honors M9's
deferred-convergence model; the smallest real fix — reuse `retry_all_incomplete`'s
single-pass logic on a schedule, per-user, cancellation-aware), and/or (b) surface a
`needs-review` state so nothing sits silent. Optionally also wire Readarr/series to resolve
at the door (the bulk resolver already exists).

## Confidence

The 2026-06-14 analysis rated itself ~90% and named what would flip it: *"a scheduled job
that re-resolves Pending works that this sweep missed."* That job now exists
(`jobs/convergence.rs`, registered in `jobs/mod.rs:129-135`, default-on), so the finding is
flipped — whether it was missed then or built since is not decidable from this tree. The
"seeding Pending is deliberate" point stands: M9's text is explicit.

## Stale-sibling reconciled

`architecture/metadata-pathway.md` § "Background Retry Job" described a job at
`jobs/enrichment.rs`; that file no longer exists and the job was removed. Updated
2026-06-14.
