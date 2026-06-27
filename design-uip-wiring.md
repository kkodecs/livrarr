# Design: unified-identity-path WIRING — the full-door cutover

> **⚠ AMENDED 2026-06-23 by `design-uip-id-completeness.md`.** The wiring mechanics here (§3 insertion
> points, §5 legacy deletion) still hold. **§4's "preserve the Sprint-E Confirmed-gate" recommendation is
> SUPERSEDED** — the ID-completeness direction reverses it (chase missing IDs on Confirmed works, with
> dead-end suppression re-added). Read `design-uip-id-completeness.md` first.

**Status:** design for review — NO code yet.
**Scope (PO decision 2026-06-23):** wire **every** identity-resolving door — plus a new background
convergence loop — through the engine `settle_identity`, and delete the duplicated resolve→badge logic it
supersedes. This is the **wide** cutover: it **supersedes the narrow Part-1 framing** of
`spec-convergence-unified-path.md` §4, which deliberately excluded the synchronous doors.

This design folds together two prior artifacts:
- `spec-unified-identity-path.md` (v4) — the engine contract (built, done).
- `design-convergence-selection-fix.md` (r4) — the background-loop **selection + pacing** (reused as-is;
  only its per-work identity *leg* changes, see §6).

---

## 1. The engine, as built (grounded this session)

`settle_identity` — `crates/livrarr-identity/src/async_resolver.rs:293-421` (body read this session):

```rust
pub async fn settle_identity<R: EnglishIdentityResolver, D: WorkIdentityRepository>(
    resolver: &R, repo: &D, user_id: UserId, work: &Work,
    mode: IdentityMode, source: ConflictSource,
) -> Result<IdentityReport, WorkIdentityError>
```

- Operates on an **already-persisted `&Work`** and writes the badge + anchors itself (REQ-008). → callers
  must **create the work first, then settle** (§3 ordering).
- `IdentityMode` — `crates/livrarr-domain/src/identity.rs:254-263`: `Interactive | Background`. Maps to
  `LatencyTier` (Interactive→Interactive, Background→Background).
- `ConflictSource` — `crates/livrarr-domain/src/identity.rs:450-460`:
  `ManualAdd | ManualImport | ListImport | ReadarrImport | SeriesMonitor | AuthorMonitor | Refresh`.
  **One variant per door already** — the engine was built *for* this wiring. The two **re-processing**
  callers (manual-retry, background convergence) have **no variant yet** → §5 adds them.
- **Terminal short-circuit (body lines ~300-310):** only `Conflict | NotFound | NeedsReview` return early as
  a no-op. **`Confirmed` and `Provisional` do NOT short-circuit** — they fall through to a full
  `resolver.resolve(...)` fan-out, then no-op the badge write (the monotonic-raise guard blocks a downgrade).
  **This is the load-bearing cost fact (§4).**

## 2. Current state — the door trace (grounded; subagent matrix + spot-verified)

| # | Door | Entry (path:line) | Identity TODAY | Sync/spawn | Reaches engine? |
|---|------|-------------------|----------------|-----------|-----------------|
| 1 | Add-from-search | `livrarr-handlers/src/work.rs:164` | `resolve_identity` (`work_service.rs:842`) pre-create → `ensure_identity_and_enrichment` (`work_service.rs:2648`, anchorless leg resolves at ~2706) | Sync | NO |
| 2 | Manual add (Add Work) | same handler `work.rs:164` | same chain | Sync | NO |
| 3 | Per-file manual import | `manual_import.rs:836` → `find_or_create_work:1004` | `resolve_identity` (`manual_import.rs:1065`) → add → chain | Sync | NO |
| 4 | Readarr import | `readarr_import_workflow.rs:120` → `process_works:1083` | seeds `IdentityState::Pending{seed_anchors}` (`:1220`), **no** `resolve_identity` → add → chain | Spawned | NO |
| 5 | List import | `list_service.rs:278` → `resolve_candidate_from_row:45` | `resolve_identity` (`list_service.rs:77`, `LatencyTier::Background`) → add | Sync (batch) | NO |
| 6 | Series monitor | `series_query_service.rs:642` | seeds `Pending{seed gr_key}` (`:793`), **no** `resolve_identity` → add → chain | Spawned | NO |
| 7 | Author monitor | `author_monitor_workflow.rs:262` | seeds `IdentityState::Confirmed{ol_key}` directly (`:499`) → add → chain (anchorless leg **skipped**, it has an anchor) | Spawned (daily) | NO |
| 8 | Refresh (single) | `work.rs:562` → `work_service.rs:1285` | `complete_anchors` (`async_resolver.rs:89`) **gated `!= Confirmed`** at `work_service.rs:1340` | Sync | NO |
| 9 | Refresh (bulk) | `work.rs:587`, loops `refresh()` | same as #8 | Spawned (202) | NO |

**Shared chokepoint (verified callers):** `ensure_identity_and_enrichment` (`work_service.rs:2648`) is called
by `add` (3 sites: 485/538/586), `handle_race_loser` (2785), `finish_created_work` (2883). **Every add door
funnels here** — one insertion point covers doors 1-6.

**Legacy resolve sites (the duplication ST-003 names):** `resolve_identity` (`work_service.rs:842`, pre-create
"single place identity is decided" — but writes no badge); `ensure_identity_and_enrichment`'s anchorless
`resolver.resolve` (~2706); `complete_anchors` (`async_resolver.rs:89`, refresh); `retry_all_incomplete`'s
direct `resolver.resolve` (`work_service.rs:1472`).

**Confirmed dead (zero callers, `find_referencing_symbols` empty):** `settle_identity` (293),
`converge_identity_pending` (`async_resolver.rs:22`), `list_works_due_for_retry`
(`db/sqlite_retry_state.rs:280`).

**Background jobs (`server/src/jobs/mod.rs:78`):** download_poller, session_cleanup, author_monitor,
state_map_cleanup, rss_sync, tag_convergence, call_record_retention. **None does identity convergence** —
confirming the silent-limbo gap (insight 54).

## 3. The wiring — 4 insertion points

Ordering rule (from §1): **create the work first (Pending + seed anchors), then `settle_identity`.** The
engine needs a persisted `&Work`.

| Insertion point | Where | Mode | Source | Covers |
|-----------------|-------|------|--------|--------|
| **A. Add chokepoint** | `ensure_identity_and_enrichment` anchorless leg (`work_service.rs:~2706`) → `settle_identity` | per-door (below) | per-door | doors 1-6 |
| **B. Refresh** | `complete_anchors` call (`work_service.rs:1340`) → `settle_identity`, **keep the `!= Confirmed` gate** | Interactive | Refresh | doors 8-9 |
| **C. Manual retry** | `retry_all_incomplete` direct `resolver.resolve` (`work_service.rs:1472`) → `settle_identity` | Interactive | ManualRetry (new) | Retry-Incomplete button |
| **D. Background convergence loop** | NEW caller; per-work identity leg of the convergence sweep | Background | Convergence (new) | the M9 silent-limbo fix |

**Per-door mode/source for insertion A:**

| Door | Mode | Source |
|------|------|--------|
| Add-from-search / Manual add | Interactive | ManualAdd |
| Per-file manual import | Interactive | ManualImport |
| List import | Background | ListImport |
| Readarr import | Background | ReadarrImport |
| Series monitor | Background | SeriesMonitor |
| Author monitor | — | — (see §7: **deliberate exception**, not routed) |

**Mode rule:** a person is waiting → Interactive; a spawned/batch worker → Background. (List import is
already `LatencyTier::Background` today; Readarr/series are spawned.) The engine guarantees the **same
identity** under both modes (REQ-005); mode only governs how an *ambiguous* verdict is parked
(Interactive → stays Pending for a user pick; Background → terminal NeedsReview).

## 4. The Confirmed-fan-out constraint (preserve Sprint-E — do NOT regress)

`settle_identity` does not short-circuit `Confirmed` (§1) — it fans out the resolver, then no-ops the badge.
Sprint-E (`fb77b40`, insight 55) **deliberately gated the per-refresh identity re-chase OFF for Confirmed
works** to remove ~2s/work of exactly this cost. Today's gate: `complete_anchors` is called from refresh
**only when `identity_status != Confirmed`** (`work_service.rs:1340`).

**Constraint:** insertion **B keeps that gate** — `if work.identity_status != Confirmed { settle_identity(...) }`.
Non-Confirmed works (Pending, Provisional) still re-settle on refresh (and a Provisional can upgrade to
Confirmed — the desirable case). Confirmed works skip the engine on refresh, as Sprint-E intends.

Consequence (flag for PO, §8): a Confirmed work that is **missing a secondary anchor** (e.g. Confirmed by
gr_key, no isbn) is **not** topped up on refresh, and the convergence loop (§6) targets *incomplete* works,
not Confirmed-complete ones — so it isn't topped up there either. This matches the Sprint-E decision (the
re-chase rarely helped). REQ-007's "MAY improve a Confirmed work" is satisfied only for the non-refresh
paths. If the PO wants Confirmed anchor top-up restored, that is a separate, explicit re-opening of Sprint-E.

## 5. Legacy collapse / deletions

- **Delete `converge_identity_pending`** (`async_resolver.rs:22`) — superseded by `settle_identity`
  (Background mode); zero callers.
- **Delete `complete_anchors`** (`async_resolver.rs:89`) after insertion B — superseded; its only callers
  are refresh (1340) and `run_unified_enrichment` (~3004), both re-pointed at `settle_identity`.
- **Delete `is_terminal_pending`** (the buggy legacy helper, `async_resolver.rs:~293-298` per ST-002) once
  the two functions above are gone (it was kept only because they used it).
- **Collapse the pre-create `resolve_identity` badge role.** Target end state: doors create Pending+seed and
  let `settle_identity` be the single resolve (avoids the double fan-out when `resolve_identity` returns
  Pending and the anchorless leg then resolves again). See §8 Q-2 — the exact reshape of `resolve_identity`
  is the one internal open question for review.
- **`list_works_due_for_retry`** stays dead — the convergence loop uses its own `list_convergence_due`
  selector (design-convergence-selection-fix.md §2), not this one. Note for cleanup only.
- **Add `ConflictSource::ManualRetry` and `ConflictSource::Convergence`** — the enum covers the 7
  creation/refresh doors but not the 2 re-processing callers (insertions C, D).

## 6. The background convergence loop (insertion D)

**Reuse `design-convergence-selection-fix.md` (r4) wholesale for selection + pacing** — it is sound across
two review rounds and is independent of which identity function runs per work:
- own `works.next_convergence_at` clock (migration) + status-based `list_convergence_due` selector
  (incomplete = identity Pending OR (Confirmed/Provisional AND enrichment NOT IN enriched/thin));
- bounded batch (default 5) + conservative cadence (TOML, no env); advance `next_convergence_at` on every
  non-completing attempt (no re-sweep, the locked guardrail);
- pre-activation worst-case GB-quota volume check + DB snapshot before first enable.

**The one change vs that design:** its per-work identity leg was written around `converge_identity_pending`
+ a caller-side badge-settle hack (§3 / Codex R-010 of that doc) — i.e. the very duplication the engine
kills. **Replace it with `settle_identity(work, Background, Convergence)`.** The engine *is* the badge-settle
(work-anchor→Confirmed, ISBN/ASIN→Provisional, dead-end→NeedsReview, transient→stay Pending), so the
caller-side rule is deleted, not re-implemented. The enrichment leg (Background enrichment path, never
`refresh()` — that design's R-009) is unchanged.

## 7. Author monitor — the one deliberate exception

Author monitor seeds `IdentityState::Confirmed{ol_key}` directly from the OpenLibrary author-works API
(`author_monitor_workflow.rs:499`). The OL key from OL is **authoritative** — the door **asserts** identity,
it does not **resolve** it. Routing it through `settle_identity` would (a) fan out the resolver for **every**
auto-added work (the Confirmed-no-short-circuit cost, §4 — and author monitor can add many works per daily
scan), to (b) almost always no-op the badge. **Recommendation: leave it as-is** — it is a create-time
derivation, the same category the engine spec explicitly excludes (`finish_created_work`, ST-003). So "wire
every door" means *every door that resolves identity*; the one door that asserts a hard ID is the principled
exception. **Tradeoff:** no conflict-check on author-monitor adds — acceptable, because they are new works
(the monitor screens out works already in the library) carrying an authoritative key. **PO flag (§8).**

## 8. Open questions

**For the PO:**
- **Q-1 (Confirmed top-up, §4):** confirm we **preserve** Sprint-E — refresh does not re-chase Confirmed
  works, so a Confirmed-but-missing-secondary-anchor work is not topped up. *(Recommend: preserve.)*
- **Q-2 (author monitor, §7):** confirm we **exclude** author monitor from the engine (it asserts, doesn't
  resolve). *(Recommend: exclude.)*

**For cross-family review (internal):**
- **Q-3:** the exact reshape of the pre-create `resolve_identity` — drop it entirely (create Pending+seed,
  settle once) vs keep it as a seed-builder. Drop is cleaner (one fan-out) but touches every add door; keep
  is lower-blast-radius but allows an occasional double fan-out. Reviewers weigh in.
- **Q-4:** does insertion A run **inside** `ensure_identity_and_enrichment` (one chokepoint, all doors) or at
  each door post-create? Chokepoint is fewer edits and matches today's funnel; per-door gives explicit
  mode/source at the call site. (Leaning chokepoint, passing mode/source as params threaded from the door.)
- **Q-5:** `settle_identity` Background mode on a freshly-seeded Readarr/series Pending work — confirm the
  resolver's provider fan-out at import time is acceptable burst load (it is the **current** behavior via the
  anchorless leg, so no regression — but worth a reviewer's eyes given no Part-2 throttle exists).

## 9. Test plan (faithful door→engine — the lesson that bit this project twice)

The metadata-refactor bug: a door compiled + passed the whole suite while routing **off** the road (insight
46). So the load-bearing tests assert each door **reaches** `settle_identity`, not just that the pipeline
works:
- Per door (1-6,8,9): drive the **real entry path** (no injected identity state) and assert the work's badge
  is settled **by the engine** (observe via `IdentityReport` / a spy resolver call with the expected
  `(mode, source)`), not by a legacy writer.
- Door 7 (author monitor): assert it is **NOT** routed (seeds Confirmed, no resolver fan-out) — the negative
  test that locks the §7 exception.
- Refresh Confirmed (B/§4): assert a Confirmed work on refresh triggers **no resolver fan-out** (Sprint-E
  no-regression).
- Convergence (D): the design-convergence-selection-fix.md §5 faithful tests (seed Pending the way Readarr
  does, no retry-row injection), plus assert the identity leg calls `settle_identity(Background, Convergence)`.
- Legacy: a compile-level check that `converge_identity_pending` / `complete_anchors` are gone (no caller).

## 10. Alternatives considered

- **Narrow first (background loop only), wire sync doors later.** Rejected by PO 2026-06-23 (wide cutover
  chosen) — and the engine's per-door `ConflictSource` shows it was built for the wide wiring.
- **Make `settle_identity` short-circuit Confirmed internally.** Tempting (removes the §4 caller gate), but
  it changes the engine's committed contract (REQ-007 allows a Confirmed anchor top-up) and re-opens a
  reviewed artifact. Keep the gate at the caller (refresh), where Sprint-E already put it. Revisit only if
  the PO re-opens Q-1.
- **Keep `converge_identity_pending` for the loop, leave `settle_identity` for the sync doors.** Rejected —
  that preserves the duplication the engine exists to kill (two badge-settle implementations drifting).
