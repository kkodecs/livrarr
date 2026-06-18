# Livrarr — Reconciled Plan (two deltas triangulated)

Three independent reads converged: the **verified audit** (existing system), **my delta** (target vs. audit-summary), and **DELTA-CC** (target vs. live code). The cross-check tightened the plan — it caught a defect my delta missed and deflated where my delta over-architected. Net: **smaller, more tactical, lower-risk** than the three-architectural-decisions framing.

---

## Locked — both deltas agree (high confidence)
- **#1 divergence: the background convergence reconciler was deleted** (RC2a / §4.3). Both call it the headline. Manual-only convergence breaks the "improves without manual intervention" guarantee. DELTA-CC receipt: `JobRunner` registers 7 jobs, none for enrichment/identity convergence; `list_works_due_for_retry` has no live caller. **Top priority.**
- **Editions: stay collapsed.** Settled. DELTA-CC independently: over-built; flatten-onto-Work + `monitor_*` flags + dual cover slots covers the dual-format case; Readarr does the same. The projection/UX is already achieved.
- **Leave alone — confirmed over-built for a single-user SQLite app:** full append-only observation log (event-sourcing), per-field authority at full grain (per-category is right), must/cannot-link constraint-clustering, drift detection, corroboration scoring, a blocking index.

## Where the cross-check corrected my delta
- **Δ2 "evidence/projection keystone" — over-architected.** Existing already has current-winner provenance + an append-only `work_field_dissents` (loser) log + latest-payload-per-provider for replay. The full evidence layer is event-sourcing the scale doesn't need; convergence comes from the reconciler **re-fetching**, not a stored observation set. **Δ2 largely dissolves.**
- **F3 (Layer-1 freeze) and G2/E3 (Thin/Conflict stranded) are TACTICAL, not architectural.** Existing already *derives* status (`derived_identity_status` reads the anchor set — identity.rs:318-329). F3 = add a user-set check to Layer-1. G2/E3 = include Thin/Conflict in the convergence set.
- **Durable corrections = a merge-executor, not a constraint store.** Real gap confirmed: conflict `resolve()` writes an action label and does nothing (E1/D1). Fix = execute the merge (repoint `library_items.work_id`, tombstone, alias B→A) and don't undo it. Not constraint satisfaction.

## New — DELTA-CC caught what my delta missed
- **`cleared`-flag text/cover asymmetry (BUG).** Cover path honors `!fp.cleared` (lib.rs:949, releases the lock); text-field path checks `setter==User` only (lib.rs:840). A user who clears a text override stays locked out of provider updates. Latent, almost certainly unintended.
- **`NotFound`/`source_empty` terminal (TENSION — decision, not clear bug).** Target wants empty sources retried slow-cadence (long-tail appears over time); code makes `NotFound` terminal (`is_phase2_terminal`, domain/lib.rs:1356-1365). DELTA-CC's counter is fair: auto-retrying every provider that lacks a book risks hammering hostile sources. Slow-retry-empty vs. don't-hammer — **your call.** The unambiguous part is the missing `WillRetry` convergence loop.
- Minor: external IDs live in **three** overlapping stores (scalar columns + `work_identity_anchors` + `external_ids`) — consistency smell, pragmatic; the `0.90` auto-confirm threshold is hardcoded (matching/lib.rs:185) — should be config, per the target's own "thresholds are config."

## Mine that DELTA-CC missed (scope — still real)
- **D2 bridge-anchor-dedup (verified).** The `add()` dedup loop is ol/gr/hc only; same-ISBN + title-variant misses both anchor and normalized dedup → UNIQUE-constraint 500. DELTA-CC didn't drill the dedup loop; the verified test (T2) stands. Fix = include ISBN/ASIN as dedup keys.
- The tactical normalizer P0s (colon-collapse C1/C2/C3, RC1) are internal-impl bugs, orthogonal to the target diff — covered by the verified findings, still required.

---

## The plan

### One architectural fix
**A. Restore the convergence loop.** Re-wire `converge_identity_pending` / `retry_all_incomplete` as a **recurring job** (it's a `POST`-only manual sweep today; `list_works_due_for_retry` already exists with no live caller). Add priority (incompleteness × P(retry helps) × budget) + per-source backoff. Decide the `source_empty` retry policy here (the one tension). Drains the Pending sink; restores the core guarantee. Closes RC2a and the recovery half of E2/E4/G2/E3.

### Tactical punch-list
- **Stop active corruption (P0, tests exist):** canonical normalizer (C1/C2/C3 + the A1/D2/D3 title-variant fallout); RC1 author-monitor false `Confirmed`; **5b corroboration guard** (C-tier "fuzzy never auto-merges alone" — moves toward the target).
- **Merge executor (E1/D1):** make conflict resolution actually execute + not undo.
- **D2:** include ISBN/ASIN in the `add()` dedup loop.
- **F3:** Layer-1 locks on *user*-set only.
- **`cleared`-flag:** text-field path honors `!fp.cleared` (symmetric with cover).
- **Retry coverage (G2/E3):** include Thin + Conflict in the convergence set.
- **Minor:** `0.90` → config.

### Leave alone (deliberate scale-downs — both deltas)
Edition entity · full observation log · per-field authority · constraint-clustering · drift detection · corroboration scoring · blocking index.

---

*Triangulated from: livrarr-suggested-changes.md (verified audit), livrarr-delta.md (my delta), DELTA-CC (build-Claude delta). Where the three disagreed, live-code evidence won.*
