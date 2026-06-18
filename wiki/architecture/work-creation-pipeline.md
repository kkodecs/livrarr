# Work-Creation Pipeline — the five phases and the M9 convergence gap

> As-of 2026-06-14. Grounded by reading the code this session (door call sites,
> `identity.rs`, `work_service.rs`) and principle **M9** (`domain/metadata-principles.md`).
> Code + principles win over this page; fix on drift.

## The five phases

A book entering the library moves through five phases, in order:

1. **Identify** — determine which real-world work this is. `WorkService::resolve_identity`
   (`work_service.rs:843`) runs a multi-provider quorum and returns a badge: `Confirmed`
   only when a work-level anchor (OL/GR/HC key) is corroborated, `Provisional` for an
   ISBN/ASIN bridge alone, else `Pending` (`identity.rs:318` `derived_identity_status`).
2. **Create the seed** — build the `WorkCandidate` through the single seed factory
   (`livrarr-domain/src/seed.rs`). Each door calls its `seed_*` constructor, which stamps
   the phase-1 identity plus title/author/language. seed.rs does **not** identify — it
   takes identity as an input.
3. **Create the work** — `WorkService::add` persists the work row (`work_service.rs:421`).
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
- **The actual defect is a regression: the convergence M9 mandates no longer runs
  automatically.** The recurring background sweep (`enrichment_retry_tick`) was
  **removed**; its replacement `retry_all_incomplete` is *user-triggered, single-pass, no
  recurring loop* (`services/work.rs:310`), reachable only from a POST route. The add-time
  leg in `add()` only resolves **anchorless** works and won't adopt on a fuzzy match
  (`work_service.rs:2666,2720`), so a Pending work carrying a seeded gr_key is not rescued
  at create. The batch resolver built for exactly this (`bulk_resolver::resolve_bulk`,
  `LatencyTier::Bulk`) has **zero production callers** (tests only). No `jobs/` task
  re-resolves Pending works.
- **Result:** series-monitor / Readarr Pending works sit in exactly the **"silent limbo"
  M9 forbids** — empty shells, no surfaced `needs-review` — until a user manually hits
  "retry incomplete." Their end state diverges from the interactive paths, violating the
  REQ-022 binding invariant.

### Secondary gap (quality, not correctness)

Per the `LatencyTier::Bulk` design, Readarr was meant to resolve **at the door** via the
bulk resolver; it never was, so it dumps every work to Pending. Series-monitor likewise
never resolves at the door. List Import demonstrates the resolve-at-door pattern working.
Wiring these two to resolve at the door would shrink the Pending population — but the
**correctness** fix is restoring convergence, not changing where they resolve.

## Fix pointer

This is the **Sprint E prerequisite (#144 remainder)** plus the owed
**dead-background-convergence** PO decision. Because M9 makes convergence binding,
"manual-only / do nothing" is **not** M9-compliant (it leaves silent limbo). Two compliant
directions: (a) restore a bounded recurring convergence sweep (honors M9's
deferred-convergence model; the smallest real fix — reuse `retry_all_incomplete`'s
single-pass logic on a schedule, per-user, cancellation-aware), and/or (b) surface a
`needs-review` state so nothing sits silent. Optionally also wire Readarr/series to resolve
at the door (the bulk resolver already exists).

## Confidence

~90% on the mechanism and the "convergence does not run automatically" finding (read the
code + M9). What would flip it: a scheduled job that re-resolves Pending works that this
sweep missed. The "seeding Pending is deliberate" point is high confidence — M9's text is
explicit.

## Stale-sibling reconciled

`architecture/metadata-pathway.md` § "Background Retry Job" described a job at
`jobs/enrichment.rs`; that file no longer exists and the job was removed. Updated
2026-06-14.
