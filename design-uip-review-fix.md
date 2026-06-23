# Fix proposal — unified-identity-path code review (round 1)

Both reviewers converged on 2 findings: Gemini R-001 **P0** / Codex R-001 **P1** (tie-branch projection), and Gemini R-002 **P1** / Codex R-002 **P2** (audit list). Fixes below are grounded against source (`run_quorum`, `agree`, `anchor_count`, `merge_missing_anchors`, `AnchorType`).

## Finding 1 (P0) — tie-branch projection picks the wrong cluster member

**Reachable (verified).** `agree()` (`english_identity_resolver.rs:462`) clusters two providers on a shared ISBN/ASIN when titles don't contradict (`:499`) — **no work-key required**. So a tied cluster can be `[bridge-only member, work-anchored member]`. The tie branch projects each cluster via `items[c[0]]` (provider-sorted first member); if `c[0]` is the bridge-only member, the projection drops the OL/GR/HC work anchor, so `settle_identity`'s AC-018 contradiction check (which compares OL/GR/HC) misses it.

**Root cause:** the tie branch did not reuse the projection the *winning* (`Resolved`) path already uses correctly — `rep_idx = top.iter().max_by_key(anchor_count)` then `merge_missing` of every member (`english_identity_resolver.rs:~383-394`).

**Fix — extract the winning-path projection into one helper, use it everywhere:**

```rust
fn project_cluster(cluster: &[usize], items: &[&NormalizedWorkDetail], seed: &WorkSeed) -> CapturedIdentity {
    let rep_idx = *cluster.iter().max_by_key(|&&i| anchor_count(items[i])).expect("non-empty cluster");
    let mut cap = captured_from_detail(items[rep_idx], seed);
    for &i in cluster {
        cap.merge_missing(&captured_from_detail(items[i], seed));
    }
    cap
}
```

- **Resolved path:** `identity: project_cluster(top, &items, seed)` (refactor of the existing inline logic — DRY, behavior identical).
- **Tie branch:** `captured: project_cluster(top, &items, seed)`; `tied: competing.iter().take_while(|c| c.len()==tie_len).map(|c| project_cluster(c, &items, seed)).collect()`; conflict representative = `items[*top.iter().max_by_key(|&&i| anchor_count(items[i])).unwrap()]`.

Reuses existing `anchor_count` + `CapturedIdentity::merge_missing`. **No new selection logic; clustering/winner DECISION unchanged** (only the reported payload). Fixes AC-018 for multi-member clusters and removes the captured-vs-tied inconsistency Gemini flagged.

**Red test (Codex, cross-family):** a 2-cluster tie where one cluster's provider-sorted-first member is ISBN-only and a later member carries an OL work key → assert the tied projection for that cluster carries the OL key. Red against current `c[0]`, green after.

## Finding 2 (P1) — `anchors_merged` diverges from the actual writes

**Confirmed.** `newly_merged_anchor_types` reads the denormalized `work` fields and reports struct-field names (`"ol_key"`); `merge_missing_anchors` reads the **confirmed ledger** (`list_anchors`) and writes `AnchorType`s (`"ol_work"`). Divergence on drifted data, wrong names, and an empty-string mismatch.

**Fix — single source of truth: `merge_missing_anchors` returns what it wrote.**

- `WorkIdentityRepository::merge_missing_anchors`: `Result<(), WorkIdentityError>` → `Result<Vec<AnchorType>, WorkIdentityError>` (the types it actually `confirm_anchor`ed).
- `SqliteDb` impl: collect each confirmed `AnchorType`, return them.
- `settle_identity`: `anchors_merged = repo.merge_missing_anchors(work.id, &captured).await?.iter().map(|t| t.as_str().to_string()).collect()`. **Drop** `newly_merged_anchor_types`.
- Legacy callers (`converge_identity_pending`, `complete_anchors`) call `…merge_missing_anchors(…).await?;` — they discard the returned `Vec`, so they compile unchanged.
- Test assertion updates (Codex): `"ol_key"→"ol_work"`, `"gr_key"→"gr_work"`, `"hc_key"→"hc_work"` (`isbn_13`/`asin` already match).

Exact (same source as the writes), correct names, ledger-based, empty-safe.

## Out of scope (flag for followup, not this fix)
- The empty-string `InvalidAnchorValue` crash path in `confirm_anchor`/`merge_missing_anchors` (Gemini R-002 issue 2) is **pre-existing** input-validation behavior shared by the legacy callers — not introduced by this feature. A separate hardening item.

## Questions for the finder models
1. Does each fix fully resolve the finding you raised, with nothing left?
2. Does `project_cluster` (most-anchored base + `merge_missing` all members) correctly handle every cluster shape, or is there a shape it still mishandles?
3. Does changing the representative `captured`/`conflict` in the tie branch to the most-anchored member (vs the old `items[top[0]]`) risk any behavior regression you can see?
4. Does returning `Vec<AnchorType>` from `merge_missing_anchors` introduce any issue (callers, ordering, semantics)?

---

# Round 2 — refined after conferral (Codex caveats folded in)

Round-1 confer: Gemini PASS (run degraded by a rate-limit; could not read the test file). Codex grounded + sound, with two caveats. Refinements below.

## Caveat A — empty-string anchors (FOLDED INTO THIS FIX)

`anchor_count` (`english_identity_resolver.rs:442`) counts `Some("")` as present, but `has_work_anchor` (`:434`) treats it as absent, `CapturedIdentity::merge_missing` won't overwrite an empty slot, and `confirm_anchor` REJECTS a blank value (`sqlite_work_identity.rs:10`, `InvalidAnchorValue`). So a `Some("")` anchor would (a) miscount the most-anchored rep and (b) make `merge_missing_anchors → confirm_anchor("")` return `Err`, aborting the settle.

**Guard — strip empty anchors to `None` at the single projection point, `captured_from_detail`:**
```rust
ol_key: d.ol_key.clone().filter(|s| !s.is_empty()),
gr_key: d.gr_key.clone().filter(|s| !s.is_empty()),
hc_key: d.hc_key.clone().filter(|s| !s.is_empty()),
isbn_13: d.isbn_13.clone().filter(|s| !s.is_empty()).or_else(|| seed.isbn_13.clone()),
asin:   d.asin.clone().filter(|s| !s.is_empty()).or_else(|| seed.asin.clone()),
```
No projected `CapturedIdentity` then carries a blank anchor → no DB reject, no empty slot for `merge_missing` to hold. Also align `anchor_count` to count only non-empty anchors (so the most-anchored rep is chosen by REAL anchors, consistent with `has_work_anchor`).

## Caveat B — transitive ISBN-collision bridging (OUT OF SCOPE — flagged, not fixed)

Clustering is transitive closure (`english_identity_resolver.rs:317`); `agree` (`:462`) vetoes only a DIRECT same-type disagreement. So `A(OL=X) — B(ISBN bridge, no work key) — C(OL=Y)` can transitively land in ONE cluster (A–B agree, B–C agree, A–C never directly compared), merging two genuinely different works; `project_cluster` keeps the base's `OL=X` and silently drops `C`'s `OL=Y`. This is the resolver's **clustering DECISION**, which §4 freezes for this feature — it is pre-existing and is neither introduced nor worsened by the payload projection fix. **Flagged for the deferred resolver-contract followup; NOT fixed here.**

## Round-2 questions for the finder models
1. Does the empty-string guard (strip empties in `captured_from_detail` + align `anchor_count`) FULLY and correctly close caveat A, or is there a better/safer point (e.g. at normalization, or also guarding the `seed` fallback)?
2. Is caveat B correctly scoped OUT (pre-existing clustering DECISION, §4), or does the payload fix change its blast radius in a way that obliges handling it now?
3. Anything ELSE the refined design (project_cluster + empty guard + Vec<AnchorType>) still mishandles?

---

# Round 3 — as-built reconciliation (post-implementation)

Implemented, cross-family reviewed (Gemini PASS, Codex PASS on round 3), all gates green (fmt clean, clippy zero warnings, `cargo test` 996 passed / 0 failed; `verify.py review` PASS). This section records the as-built result and the one new finding the code review surfaced.

## R-003 (Codex round-2, P1) — empty-anchor guard was incomplete → FIXED

Round-2 caveat A aligned `anchor_count` to `has_work_anchor`'s `!v.is_empty()` predicate. Codex round-2 correctly flagged that this left a gap: `non_blank` strips blanks with `trim()`, but `has_work_anchor`/`anchor_count` only stripped exact-empty (`is_empty`). So a whitespace-only work anchor (e.g. `"   "`) could make `any_anchored` true, enter the anchored-only competition, and be selected as the representative — then be dropped by `project_cluster`/`non_blank`. The blank definition was inconsistent between gating/ranking and projection.

**Fix (the only change after round 2):** `has_work_anchor` and `anchor_count` (`english_identity_resolver.rs`) now use `!v.trim().is_empty()`. Blank is now defined identically across projection (`non_blank`), classification (`has_work_anchor`), ranking (`anchor_count`), and persistence (`confirm_anchor`, which rejects `value.trim().is_empty()`). Only changes behavior for whitespace-only anchor values (unreachable for normalized provider data); no existing test behavior changes. Codex round-3: PASS.

The `CapturedIdentity`-based presence checks (`identity_has_anchor`; the separate `has_work_anchor(&CapturedIdentity)` in `async_resolver.rs`) operate on already-projected (post-`non_blank`) data and can never see a blank — correctly left unchanged.

## As-built helper names

- `project_cluster(cluster, items, seed)` — most-anchored member (`max_by_key(anchor_count)`) base + `merge_missing` of every member. Used by the Resolved path (refactor, behavior identical) and the tie branch (`captured` + each `tied` cluster); the tie-branch conflict representative is the most-anchored member of `top`.
- `non_blank(Option<String>) -> Option<String>` — strips `trim().is_empty()` values to `None`; applied to all 5 anchor fields in `captured_from_detail`/`captured_from_seed`, including the isbn/asin seed fallback.
- `WorkIdentityRepository::merge_missing_anchors -> Result<Vec<AnchorType>, _>` — returns the types it confirmed; `settle_identity` maps them via `.as_str()` into `anchors_merged` (`"ol_work"`/`"gr_work"`/`"hc_work"`/`"isbn_13"`/`"asin"`). `newly_merged_anchor_types` deleted. The 5 non-`settle_identity` callers discard the returned Vec and compile unchanged.

## Delivered vs Deferred

**Delivered:** R-001 (`project_cluster` reused by the tie branch), R-002 (`merge_missing_anchors -> Vec<AnchorType>` as the single audit source), the empty-string guard (`non_blank` + uniform `trim().is_empty()`), R-003 (consistent blank predicate).

**Deferred — frozen by spec §4, tracked as a resolver-contract follow-up (see `build/bugs.md`):**
- Empty-string handling inside `agree`/`opt_eq`/`opt_differs` (the clustering agreement predicates) — the empty-guard was deliberately scoped to projection, not the clustering DECISION.
- Transitive ISBN-collision bridging: `A(OL=X) — B(ISBN bridge) — C(OL=Y)` can transitively cluster two different works; `project_cluster` keeps the base's `OL=X` and drops `C`'s `OL=Y`. Pre-existing, neither introduced nor worsened by the payload fix.

## Doc-reconciliation notes

- Spec / IR-v1 / IR-v2 / contract need no edits: none declares `merge_missing_anchors`'s return type (prose-only references), and `anchors_merged` is typed `Vec<String>` with no hard-coded name format — the `ol_key`→`ol_work` format lives only in this design doc.
- `verify.py code` reports 4 warn-only structural-conformance advisories on IR-v1 (`set_identity_confirmed for SqliteDb`, `set_identity_provisional for SqliteDb`, `Resolution::Conflict payload (EXTENDED)`, `run_quorum (MODIFIED, …:368-373)`). These are checker mis-parses of IR-v1 `name:` prose labels that embed parenthetical suffixes; the underlying symbols all exist in code. Pre-existing, non-blocking; left as-is.
