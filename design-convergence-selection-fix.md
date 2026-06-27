# Design: convergence selection + pacing fix (Part 1, round-4 / Q-004)

**Status:** design for review — NO code yet. Resolves review findings R-007 (P0), R-008 (P1), R-006 (P2),
and supersedes the provider_retry_state-based selection/pacing that produced R-001/R-004.

## 1. Problem (grounded)

The background tick `converge_pending_due` selects works via `list_works_due_for_retry`
(`sqlite_retry_state.rs:280`), whose query is an **INNER JOIN on `provider_retry_state`**:

```sql
SELECT prs.work_id, prs.provider FROM provider_retry_state prs
JOIN works w ON prs.work_id = w.id
WHERE w.user_id = ? AND prs.next_attempt_at IS NOT NULL AND prs.next_attempt_at <= ?
```

But the works convergence exists to fix — `IdentityStatus::Pending` works seeded by **series monitor**
(`series_query_service.rs:795`), **Readarr import** (`readarr_import_workflow.rs:1224`), and **list import**
(`list_service.rs:45`) — are created `Pending` with **no `provider_retry_state` row** (none of those doors
call `record_will_retry`; verified). The only writers of retry rows are the enrichment provider queue
(`provider_queue.rs:807`, runs only AFTER identity resolves) and convergence itself. So a freshly-seeded
Pending work is **categorically invisible** to the selection query → convergence silently skips its entire
purpose (**R-007, P0**). Every behavioral test passed only because a `make_due` helper manually inserted the
retry row the real doors never write.

Compounding: `reset_for_manual_refresh` (`sqlite_work.rs:1081`) **DELETEs all `provider_retry_state` rows**
for a work before re-enriching. So a work that goes through `refresh` loses its retry rows and also drops
out of the due query until something re-inserts one (**R-008, P1**).

**Root cause:** convergence's *selection and pacing* are coupled to `provider_retry_state`, a table that is
(a) empty for its primary targets and (b) wiped by refresh. That table is the **provider rate-limit clock**,
a different concern (cf. V3 two-clock separation: transport-cache vs provider-retry; convergence is a third).

## 2. Decision

**Give convergence its own per-work pacing clock, and select by STATUS, not by retry-row existence.**

- **Migration:** add `next_convergence_at TEXT` (nullable, UTC RFC3339) to `works`. NULL = "due now / never
  attempted". Plus a partial/covering index to keep the scan cheap.
- **Selection query** (new `WorkDb` method `list_convergence_due(user_id, now, limit)`):
  ```sql
  SELECT id FROM works
  WHERE user_id = ?
    AND (
      identity_status = 'pending'
      OR (identity_status IN ('confirmed','provisional')
          AND enrichment_status NOT IN ('enriched','thin')))   -- "incomplete" by inversion (see §8 R-014)
    )
    AND (next_convergence_at IS NULL OR next_convergence_at <= ?)
  ORDER BY added_at ASC
  LIMIT ?
  ```
  Status-based → seeded-Pending works ARE selected (R-007). `LIMIT` in SQL (R-006). Refresh-immune — a work
  is selected by its status regardless of retry rows (R-008).
- **Pacing:** a setter `set_next_convergence_at(user_id, work_id, Option<ts>)`. After each attempt, a work
  that did NOT complete gets `next_convergence_at = now + 1h`; a completed work falls out of the status
  filter naturally (optionally clear it to NULL). Terminal identity (NeedsReview/NotFound/Conflict) also
  falls out of the status filter, so it is never re-selected.
- **`converge_pending_due` rewrite:** select via `list_convergence_due`; per work run the identity leg
  (Pending → `converge_identity_pending` + badge-settle incl. the ISBN/ASIN→Provisional bridge from R-002)
  then the enrichment leg via the **Background enrichment path, NOT `refresh`** (§7 R-009 / §8); then set
  `next_convergence_at`. **Convergence does not read or write `provider_retry_state` for selection/pacing** (`list_works_due_for_retry`, `reset_all_retry_states`, `list_retry_states`, `record_will_retry`)
  — that interplay is what produced R-001 (query leak), R-004 (clobbering enrichment's granular backoffs),
  and R-008. Convergence no longer reads or writes the provider rate-limit table at all.

## 3. Data flow / state contract

Per tick, per user: `list_convergence_due(now, BATCH=5)` → for each work:
- **Pending** → `converge_identity_pending` → settle badge: work-anchor→Confirmed, ISBN/ASIN-only→Provisional,
  dead-end→NeedsReview (terminal). 
- **Confirmed/Provisional + incomplete** → **Background enrichment** (`run_unified` /
  `enrich_work` with `EnrichmentMode::Background` — NOT `refresh`, which would wipe `provider_retry_state`).
- Re-read: **complete** (identified + enriched) → done, falls out of the filter; **still incomplete** →
  `next_convergence_at = now + 1h`; **terminal** → falls out of the filter.

States a work moves through: `Pending → Confirmed/Provisional → (enriched=complete)` | `Pending →
NeedsReview (terminal)` | stays incomplete, paced by `next_convergence_at`. No path loops; no path is
stranded (selection is status-based, so a still-incomplete work is always re-selected when its clock is due).

## 4. Alternatives considered (and why rejected)

- **Bootstrap a `provider_retry_state` row in each seeding door.** Touches 3 doors, and `refresh` would still
  wipe it (R-008 unfixed). Rejected.
- **Reuse `next_attempt_at` on `provider_retry_state` as the convergence clock.** It is the provider
  rate-limit clock (wiped by refresh, absent for seeded works). Overloading it re-creates R-007/R-008.
  Rejected — keep the clocks separate.
- **Full-scan `list_works` + in-memory filter (mirror `retry_all_incomplete`).** Works for correctness but
  loads every work each tick (R-006) and still needs a pacing clock. The dedicated query + clock is cleaner
  and bounded. (The manual button can stay full-scan; it is user-triggered and one-shot.)

## 5. Faithful test plan (no `make_due`)

The load-bearing new test: **create a Pending work the way Readarr does** (`work_service.add` with an
`IdentityState::Pending` candidate, NO retry row injected) and assert `converge_pending_due` finds and
resolves it. This is the test that would have caught R-007. Plus: seeded-Pending with no retry row is paced
(not re-swept next tick) via `next_convergence_at`; a refreshed-then-failed work is still re-selected next
cycle; the existing R-001/R-002/R-003/R-004 edge cases re-expressed against the new selector.

## 6. Open questions for review

- **Q1 (load-bearing):** the exact `enrichment_status` string values. `EnrichmentStatus` (snake_case) =
  `unenriched|enriched|thin|failed` — **no `pending`** — yet migration 001 defaults the column to `'pending'`
  and `reset_for_manual_refresh` *writes* `'pending'` (`sqlite_work.rs:1060`). Is `'pending'` aliased to
  `Unenriched` on read, or is this a latent inconsistency? The incomplete-set in the WHERE clause depends on
  the answer (likely `IN ('unenriched','failed','pending')`). **Must resolve before coding.**
- **Q2:** index — `(user_id, identity_status, next_convergence_at)` vs a partial index on the incomplete
  predicate. Which keeps the per-tick scan cheap without bloating writes?
- **Q3:** should `next_convergence_at` be cleared to NULL on completion, or left (harmless, since the status
  filter already excludes complete works)?
- **Q4:** interaction with the `EnrichmentStatus::Unenriched` "crash-recovery: retry job picks up Unenriched
  works older than 5 min" note (`lib.rs:78`) — does any *other* job already sweep Unenriched works, and would
  convergence double up with it?

## 7. Revisions after design-review r1 (resolves R-008..R-013)

- **R-009 (P0) — convergence MUST NOT call `refresh()`.** Verified: `refresh` (`work_service.rs:1284`) calls
  `reset_for_manual_refresh`, which `DELETE`s all `provider_retry_state` rows — so refreshing every tick
  would wipe the provider 429/rate-limit backoffs and re-create the quota-churn this feature exists to avoid.
  **Revised enrichment leg:** convergence invokes the **Background enrichment path** — the same one `add()`
  uses to async-enrich a newly-identified work (`EnrichmentMode::Background`; entry points at
  `work_service.rs:2780,2954` / `enrichment/lib.rs:729` — exact symbol pinned at impl-time signature
  grounding), which **preserves and respects** `provider_retry_state`. Clarified separation: convergence's
  **selection + pacing** use ONLY `works.next_convergence_at` (it never reads/writes `provider_retry_state`
  directly); the enrichment it triggers uses `provider_retry_state` normally and must not wipe it.
- **R-010 / R-008 (Q1 RESOLVED).** Verified `parse_enrichment_status` (`sqlite_work.rs:179`): `'pending'` and
  `'partial'` both read as `Unenriched`, and `reset_for_manual_refresh` WRITES `'pending'` (`:1060`). So the
  incomplete-set in the WHERE clause is **`enrichment_status IN ('unenriched','pending','partial','failed')`**.
  **Source-fix (no-bandaid):** also change `reset_for_manual_refresh` to write `'unenriched'` instead of
  `'pending'`, normalizing at the source (the IN-clause still tolerates legacy `'pending'`/`'partial'` rows).
- **R-011 (Q2 RESOLVED).** Use a **partial index** (not a full one — avoids indexing every completed work):
  `CREATE INDEX idx_works_convergence_due ON works(user_id, next_convergence_at) WHERE identity_status =
  'pending' OR (identity_status IN ('confirmed','provisional') AND enrichment_status IN
  ('unenriched','pending','partial','failed'))`.
- **R-012 / R-009 (Q3 RESOLVED).** Clear `next_convergence_at` to **NULL on completion AND inside
  `reset_for_manual_refresh`** — so a user-triggered refresh re-derives the work as due-now (NULL) rather
  than being gated by a stale far-future clock from an old failed convergence attempt.
- **R-013 / R-011 (Q4 RESOLVED).** No active competing sweep: `list_stale_unenriched_works`
  (`sqlite_work.rs:828`) is a dormant Phase-7 no-op (`maintenance.rs:47`). Convergence supersedes it — note
  for cleanup; do not wire it.
- **Codex R-010 — explicit badge-settle rule.** After `converge_identity_pending`: set **Confirmed** iff a
  work anchor (`ol_key`/`gr_key`/`hc_key`) is present; set **Provisional** iff `isbn_13`/`asin` is present and
  there is no work anchor; leave **Pending** only for a transient unresolved; a deterministic dead-end is set
  to terminal **NeedsReview** by `converge_identity_pending` itself. (NOT the `retry_all_incomplete` rule,
  which confirms any Resolved.)

## 8. Revisions after design-review r2 (resolves R-009/R-014/R-015)

- **R-014 (P1) — predicate by INVERSION, not enumeration.** Verified `increment_enrichment_retry_count`
  (`sqlite_work.rs:1352`) actively writes `enrichment_status = 'exhausted'` (parses to `Failed`), which the
  r1 `IN ('unenriched','pending','partial','failed')` list MISSED → would strand exhausted works (same class
  as R-007). **Fix: invert** — the identified-incomplete predicate is `enrichment_status NOT IN
  ('enriched','thin')`. This treats every non-complete enrichment string (unenriched/pending/partial/failed/
  exhausted/skipped/any legacy) as incomplete and is immune to the "missed a string" class. `'thin'` stays
  excluded (the G2 non-requirement); `'enriched'` is the only other complete state. The partial index uses
  the same inverted predicate.
- **R-009 (P1) — doc made internally consistent.** §2 and §3 previously still said `refresh`; both now
  specify the **Background enrichment path** as the only enrichment leg. Confirmed safe by the reviewer's
  trace: `run_unified_enrichment` → `EnrichmentServiceImpl::enrich_work` dispatches providers/merges and does
  **not** call `reset_for_manual_refresh` (so `provider_retry_state` is preserved). Exact entry symbol +
  signature pinned at impl-time grounding.
- **R-015 (P2) — writer normalization (hygiene, no longer correctness-critical given the inversion).** Three
  writers store non-canonical incomplete strings: `reset_for_manual_refresh` (`'pending'`),
  `reset_enrichment_for_refresh` (`'pending'`, `sqlite_work.rs:1321`), `increment_enrichment_retry_count`
  (`'exhausted'`). Normalize them to canonical enum values (`'unenriched'` / `'failed'`) as a follow-on
  cleanup. The inverted predicate (§8 R-014) means convergence is correct regardless, so this is optional
  hygiene — do it if cheap, but it is NOT a blocker.

**Design status:** core (own `next_convergence_at` clock + status-based bounded query + Background-enrichment
leg) validated across two review rounds; findings converged (5 → 3 → small/concrete, no P0). Sound to
implement.
