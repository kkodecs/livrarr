---
feature: sprint-e-refresh-gate
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002]
type: bugfix
---

# Spec: sprint-e-refresh-gate (confirmed-work identity re-chase gate)

Bugfix. A manual/bulk work refresh re-runs the identity anchor-completion (`complete_anchors`)
on **every** refresh, including for works whose identity is already `Confirmed`. On works that
are confirmed but missing a non-essential anchor (e.g. a Goodreads work key), this re-chases an
unresolvable anchor — including an LLM Goodreads-key chase — on every refresh, for no enrichment
benefit. This spec scopes a guard that skips the re-chase for `Confirmed` works.

## 0a. Design Principles

- **Skip only for `Confirmed`, and only at the refresh door (door 1).** `Pending` and `Provisional` works
  still need a work anchor, so they keep re-chasing. `Conflict`/`NeedsReview`/`NotFound` keep current
  behavior — out of scope. The gate applies to the `complete_anchors` invocation **directly in `refresh()`**
  (door 1); a second `complete_anchors` inside `run_unified_enrichment` (door 2, ST-006) is intentionally
  left running.
- **Surgical, no new structure.** No new entities, states, columns, or migrations. One guard around the
  refresh-door (`refresh()`) `complete_anchors` block plus a corrected comment. Door 2 is untouched.
- **Do not entangle the parked identity-convergence restructure** (insight 54 / memory
  `project_identity_pipeline_restructure_needed`). Smart background convergence belongs there, not here.
- **Persisted-suppression is rejected** (see ST-003): `provider_retry_state` can't distinguish an
  identity-completion park from an enrichment park, so keeping identity parks across a refresh isn't clean.
  The gate is on `identity_status`, not on retry-state persistence.
- **Accepted degradation (PO-approved 2026-06-14):** a `Confirmed` work that is missing a *resolvable*
  anchor (e.g. a `gr_key` that was never chased because the add-door only resolves anchorless works —
  insight 54, `work_service.rs:2666,2720`) will no longer auto-complete that anchor on a plain refresh.
  Because background convergence is currently dead (insight 54), such a work will not gain the missing
  anchor — and any enrichment that depends on it (e.g. Goodreads) — until a recovery path runs. This is a
  deliberate, accepted trade-off for the speed win; the durable fix is the parked identity-convergence
  restructure, not this gate. **Recovery paths that still complete anchors:** a user manually supplying the
  anchor via an identity edit, or the future convergence restructure. (`retry_all_incomplete` is NOT a
  recovery path for these works — `work_service.rs:1421` selects only `Failed`/`Unenriched` enrichment or
  `Pending` identity, so a `Confirmed`+`Enriched` work missing an anchor is never picked up; and its inner
  `refresh()` would skip door 1 anyway.) The measured cases are un-resolvable in practice (Dune #7 chases
  `gr_key` for ~6.4s and Goodreads still does not resolve — ST-005), so the realized loss today is near-zero.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `crates/livrarr-db/src/sqlite_work.rs:1080` (`reset_for_manual_refresh`): `DELETE FROM provider_retry_state WHERE work_id=? AND user_id=?` | A manual refresh deletes all `provider_retry_state` rows for the work **before** `complete_anchors` reads suppression — so provider suppression does NOT survive a plain refresh. | Relying on `provider_retry_state` suppression to bound the per-refresh re-chase across refreshes. | high |
| ST-002 | `crates/livrarr-metadata/src/work_service.rs:1312–1337` (`refresh`) | `complete_anchors` is invoked on every refresh, gated only on `resolver.is_some()` — there is no identity-status condition today. | — | high |
| ST-003 | `crates/livrarr-db/migrations/029_add_provider_retry_state.sql:12` (`PRIMARY KEY (work_id, provider)`; `user_id` is a NOT NULL FK column, not part of the key) | `provider_retry_state` has no park-origin column — an identity-completion park and an enrichment-scatter park collide on the same `(work_id, provider)` row. | Selectively deleting only identity-completion parks on refresh while preserving enrichment parks. | high |
| ST-004 | `crates/livrarr-metadata/src/work_service.rs:1284–1399` (`refresh` returns the refreshed work; no spawn) | A work refresh is synchronous — the HTTP POST blocks until completion. Time spent in `complete_anchors` is time the user's request blocks. | — | high |
| ST-005 | `build/reports/speed-baseline-2026-06-14-refresh-probe.json` (git `5a8e3b9`, alpha5, 10-work dev lib) | Measured refresh POST durations: #1 Summer Knight **463ms** (source `hardcover,audnexus`); #2 Jade City **2347ms**; #7 Dune **6359ms** (source `openlibrary,audnexus` — Goodreads absent despite the gr_key chase). | — | high — durations sampled; identity_status confirmed via Q-001 (all 10 works `confirmed`; #2/#7 have ol/hc/isbn anchors, so door 2 never fires for them — door 1 is their entire identity cost). |
| ST-006 | `crates/livrarr-metadata/src/work_service.rs:2970–3003` (`run_unified_enrichment`) | A SECOND `complete_anchors` (door 2) runs inside `run_unified_enrichment`, gated on the work having **none** of `ol_key`/`isbn_13`/`asin`/`hc_key` (anchor-poor — e.g. a GR-only work). It is the only enrichment path for anchor-poor works (no enrich provider consumes `gr_key` alone). | Gating door 2 on `Confirmed` — it would permanently starve anchor-poor `Confirmed` works of all enrichment. | high |

## 1. Problem Statement

**What's broken.** `WorkService::refresh` (`crates/livrarr-metadata/src/work_service.rs`) calls
`complete_anchors` on every refresh, gated only on whether a resolver is configured (ST-002). The
function `reset_for_manual_refresh`, called earlier in the same `refresh` (line ~1297), deletes the
work's `provider_retry_state` rows (ST-001) — so the terminal-`NotFound` "park" that `complete_anchors`
writes for an unresolvable anchor is erased before the next refresh can honor it. Net effect: a
`Confirmed` work that is missing a non-essential anchor re-runs a full identity fan-out — including an
LLM Goodreads-key chase — on **every** refresh.

**Why it matters.** The refresh is synchronous (ST-004), so this cost blocks the user's request. The
chase yields no benefit on the measured case: #7 Dune spends ~6.4s and Goodreads is still absent from
its enrichment source (ST-005). A confirmed work has the identity it needs; re-deriving the missing
anchor every refresh is wasted, repeated work.

**Steps to reproduce.** Refresh a `Confirmed` work that is missing its `gr_key` (e.g. #7 Dune on the dev
lib). Observe the `identity fan-out responder` / `identity quorum` debug log lines and a multi-second
POST. Refresh again — the same fan-out and cost recur, every time.

**The stale comment.** The comment at `work_service.rs:1305–1310` asserts "neither refresh reset touches
`provider_retry_state`, so suppression survives plain refresh." This is false (ST-001) and must be corrected.

**Affected existing requirement.** Sprint B **REQ-008** ("identity anchor-completion precedes the scatter
on every refresh door") — this bugfix amends its scope to *every refresh door for works not yet `Confirmed`*.

## 2. Requirements

- **REQ-001**: A work refresh MUST NOT run the **refresh-door** identity anchor-completion — the
  `complete_anchors` invocation directly in `refresh()` (door 1) — when the work's identity is `Confirmed`.
  For all other identity states, door 1 is unchanged. The enrichment scatter (`run_unified_enrichment`)
  MUST still run for `Confirmed` works. The **anchor-poor completion inside `run_unified_enrichment` (door 2,
  ST-006) is OUT OF SCOPE and intentionally unchanged** — it fires only when the work has none of
  `ol_key`/`isbn_13`/`asin`/`hc_key`, and is the only enrichment path for anchor-poor (e.g. GR-only) works.
- **REQ-002**: The code comment describing the refresh suppression behavior (`work_service.rs:1305–1310`)
  MUST be corrected to state that `reset_for_manual_refresh` deletes `provider_retry_state` and that
  provider suppression therefore does not survive a manual refresh.

## 4. Non-Requirements

- Wiring or consuming the dead 24h `metadata_cache` (migration 056) — separate, secondary lever.
- The dead-background-convergence / identity-pipeline restructure (insight 54) — explicitly kept separate.
- Changing refresh behavior for `Pending`, `Provisional`, `Conflict`, `NeedsReview`, or `NotFound` works.
- Persisting identity-completion suppression across refreshes (rejected per ST-003).
- Parallelizing the enrichment scatter — already parallel (insight 55).
- Re-measuring the full speed baseline — separate task.
- Auto-completing a resolvable-but-missing anchor on a `Confirmed` work — out of scope (accepted
  degradation, see 0a). Deferred to the parked identity-convergence restructure.
- The broader dead-boundedness cleanup in `refresh` (the always-empty `list_retry_states`/`suppressed`
  computation and the cross-refresh `not_found` park, both defeated by ST-001's wipe) — see Q-003.
- Gating or changing the anchor-poor `complete_anchors` (door 2) inside `run_unified_enrichment` (ST-006) —
  intentionally left running. A GR-only `Confirmed` work therefore still completes anchors on refresh; this
  is correct (its only enrichment path) and is not the measured cost (the measured slow works have other
  anchors, so door 2 never fires for them).

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Are the slow dev works (#2 Jade City, #7 Dune) `identity_status = confirmed`? Determines the *magnitude* of the win, not its correctness. | resolved | Verified 2026-06-14 via `SELECT id, identity_status, gr_key FROM works` on `testdata/livrarr.db`: **all 10 works are `confirmed`**; **9/10 are missing `gr_key`** (all but #1 Summer Knight). The gate applies to all 10 and removes the re-chase from the 9 — including #2 and #7. |
| Q-002 | Should the gate read `identity_status` from the pre-reset work snapshot (line ~1289) or re-fetch after `reset_for_manual_refresh` (line ~1297)? `reset` promotes `NotFound`→`Confirmed`/`Provisional`/`Pending`; a just-recovered `NotFound` work should still complete anchors. | resolved | **Decision: evaluate the gate on the pre-reset snapshot** (the work read at the start of `refresh`). `reset` only promotes *upward from* `NotFound`, never demotes a `Confirmed` work, so a work that is `Confirmed` before the reset is still `Confirmed` after — the gate is correct. A just-recovered `NotFound` work reads its stale `NotFound` and therefore does NOT skip, so it still completes anchors (desired). |
| Q-003 | Should the always-empty `list_retry_states`/`suppressed` computation and the cross-refresh `not_found` park in `refresh()` (dead code per ST-001's wipe, raised by spec review R-002) be removed as part of this bugfix? | resolved | **Deferred (PO decision 2026-06-14).** Keep this a minimal one-guard bugfix. The dead code is pre-existing and harmless; a full removal touches the shared `complete_anchors` signature + the add-door — territory of the parked identity-convergence restructure. Recorded as a follow-up, not done here. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Refreshing a `Confirmed` work that has at least one of
  `ol_key`/`isbn_13`/`asin`/`hc_key` does not invoke door 1's `complete_anchors` — no identity fan-out from
  the refresh door (no `identity fan-out responder` / `identity quorum` debug lines, no completion-origin
  `provider_retry_state` writes).
- [ ] **AC-002** (REQ-001): Refreshing a work with `identity_status` in {`pending`, `provisional`} STILL
  invokes door 1's `complete_anchors` — behavior unchanged from current.
- [ ] **AC-003** (REQ-001): For a `Confirmed` work, the enrichment scatter still runs — enrichment source
  and merged fields are unchanged from a pre-gate refresh; only the door-1 identity-completion cost is removed.
- [ ] **AC-004** (REQ-002): The comment at `work_service.rs:1305–1310` accurately states that
  `reset_for_manual_refresh` deletes `provider_retry_state` and that suppression does not survive a refresh.
- [ ] **AC-005** (REQ-001): A `Confirmed` work that is anchor-poor (none of `ol_key`/`isbn_13`/`asin`/`hc_key`
  — e.g. GR-only) STILL completes anchors via door 2 inside `run_unified_enrichment` — door 2 is unchanged.
