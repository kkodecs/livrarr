# Confer — Reconciled Plan Reviews (Gemini · Codex · Claude)

Cross-family consultation on `livrarr-reconciled-plan.md`. Each model was given the full plan + access to the live repo and `DELTA-CC.md`, and asked to pressure-test priority, sequencing/interaction risk, the `source_empty` decision, the "leave alone" list, and anything missed. Verbatim outputs below; Claude's own review last.

---

## Gemini (gemini-3.1-pro-preview)

Here is my pressure-test of the reconciled plan for the **livrarr** project, grounded in the live codebase and operational insights.

---

### 1. Is the priority right? (The Architectural #1)
The plan’s choice of **A. Restore the convergence loop** as the single architectural fix is conceptually correct.
* **Why it's structurally correct:** Seeding `identity-pending` works on batch paths (series-monitor, Readarr import, list import) is *by design* (M9 principle). But because the background convergence loop (`enrichment_retry_tick`) was deleted (leaving `JobRunner` with exactly 7 jobs and no reconciler, `jobs/mod.rs:80`), these works sit in silent, permanent limbo.
* **The priority trap:** While restoring the loop is the correct **product priority**, launching it must be the **last execution step**. If you unleash a background runner over a library with active normalizer, dedup, and merge bugs, you will trigger silent data corruption, duplicate conflicts, and massive SQL unique-constraint failures (`D2` same-ISBN dedup 500 error) automatically and silently.

**Verdict:** The Priority is correct, but the **Sequence** must be reversed. Freeze and stabilize first; drive background execution last.

---

### 2. Sequencing & Interaction Risk
There are severe ordering hazards that, if unaddressed, will cause immediate loops or amnesia:

* **The Merge Executor (E1/D1) is a hard blocker for the Convergence Loop (A):**
  * Currently, `LiveIdentityConflictService::resolve` (`crates/livrarr-server/src/services/identity_conflict_service.rs:70`) only updates the database label in the `work_identity_conflicts` table (`sqlite_identity_conflict.rs:188`). It has no merge executor—no library item repointing, no work tombstoning, no B→A aliasing.
  * If the background loop is activated *before* the Merge Executor is implemented, the reconciler will re-examine the un-merged conflicts, have no "memory" of user splits or resolutions, and either automatically re-detect the conflict or silently overwrite manual changes.
* **The "Permanent Lock" Text-Override Bug (`cleared` asymmetry):**
  * The text-field merge path (`crates/livrarr-enrichment/src/lib.rs:840`) locks updates if `setter == User` without checking `!fp.cleared`. Only the cover path (`crates/livrarr-enrichment/src/lib.rs:949`) checks `!fp.cleared`.
  * If the background loop runs first, any text override a user has cleared will remain permanently locked, stranding the work's textual metadata indefinitely.
* **Recommended Sequence:**
  1. **Phase 1 (Freeze):** Fix the normalizer (C1/C2/C3), dedup crash (D2), and text `cleared` override bug.
  2. **Phase 2 (Durable State):** Implement the Merge Executor (E1/D1) so user decisions are written durably (repoint, tombstone, alias).
  3. **Phase 3 (Drive):** Spin up the background convergence loop (A).

---

### 3. The One Open Decision: `NotFound` Retry Policy
Treating `NotFound` as terminal (`is_phase2_terminal` in `crates/livrarr-domain/src/lib.rs:1356`) is the only thing preventing the app from self-destructing under rate-limiting blocks. For example, **Goodreads** is highly hostile, scraped without an API, and currently running at 5-7x over the polite rate floor (`wiki/integrations/goodreads.md`).

**Recommendation: A Differentiated, Triggered, and Decaying Backoff Retry Policy.**
* **Never use a flat-rate retry for `NotFound`.** Instead, use an exponentially decaying cadence that doubles with each failure and caps at a long period (e.g., retry at 7, 14, 30, 90, up to 180 days).
* **Source-Specific Limits:** Highly hostile sources (Goodreads, OpenLibrary) must decay rapidly to 180-day caps (or manual-only). Cohesive, cooperative sources (Google Books, Hardcover) can cap at 30 days.
* **Event-Based Triggers (Trigger-over-Timer):** Bypass the slow clock and trigger an immediate retry only on high-signal system events:
  1. **New media format added:** Adding an audiobook file to an existing ebook `Work` signals new metadata fields (narrator, duration) are active.
  2. **New anchor acquired:** If another provider finds an `ol_key` or `gr_key`, trigger immediate retries on previously empty providers using the precise key instead of a fuzzy title search.

---

### 4. Is anything in "leave alone" actually load-bearing?
Most of the deferred items are indeed academic overkill for a single-user SQLite app. However, **the Blocking Index cannot be deferred.**
* **The Risk:** The plan claims linear scan in-memory is fine (`work_dedup::find_matching_work`, `crates/livrarr-matching/src/work_dedup.rs:50`). But this requires querying the *entire* `works` table from SQLite into a flat Rust `Vec` on every add, search, and background loop tick—an O(N) database serialization and memory bottleneck.
* **The Fix:** We do not need a vector search engine, but we *must* use standard database indexes. SQLite already indexes `idx_works_user_normalized` (`works(user_id, normalized_title, normalized_author)`). Matching should query the database directly using this index rather than dumping the database into memory.

---

### 5. What the plan misses or gets wrong

* **Retry Heuristic Over-Scoping:** The plan's proposed retry utility formula `(incompleteness × P(retry helps) × budget)` is unnecessary math. A simple queue sorted by `next_attempt_at` and `added_at` (already implemented in `list_works_due_for_retry` at `sqlite_retry_state.rs:281`) is completely sufficient.
* **Concurrency Race with `tag_convergence`:** The server spawns `tag_convergence` every 60 seconds (`jobs/mod.rs:114`). If the restored identity loop writes to `Work` rows concurrently, a file-writing race condition is likely. Reconciler writes and tag writing must be synchronized or use a per-work write lock.
* **The `NotConfigured` Reset Chokepoint:** `OutcomeClass::NotConfigured` is treated as terminal. If a user subsequently enters their API credentials, the background loop will continue to ignore that provider. The plan must mandate that saving metadata settings explicitly invokes `reset_not_configured_outcomes` (`crates/livrarr-db/src/sqlite_retry_state.rs:2197`) to un-strand those records.

---

## Codex (gpt-5.5)

> Note: Codex's MCP/shell tooling timed out in this run, so its code receipts are taken from the plan/DELTA-CC citations rather than fresh reads; its reasoning is unaffected.

I’d adjust the priority: **restore the convergence loop is the right architectural #1, but not the first thing to turn on globally.** The first release order should be:

1. Fix corruption/amplification bugs: normalizer, D2 ISBN/ASIN dedup, RC1 false `Confirmed`, C-tier fuzzy auto-merge guard.
2. Add the merge executor plus durable alias/tombstone memory.
3. Fix field-lock semantics: Layer-1 user-set check and `cleared` text symmetry. Current plan cites status derivation from anchors in `identity.rs:318-329`, cover honoring `!fp.cleared` at `lib.rs:949`, and text only checking `setter == User` at `lib.rs:840`.
4. Then enable convergence, initially budgeted and observable.

The loop is the architectural fix because manual-only retry leaves `list_works_due_for_retry` effectively dead and breaks unattended improvement. But running it before the normalizer/dedup/merge fixes risks replaying bad decisions across the whole library faster.

**Sequencing hazards**

- **Convergence before normalizer/dedup fixes:** bad. It can amplify title-variant and same-ISBN duplicate behavior into repeated writes or conflicts.
- **Convergence before merge executor:** bad for `Conflict` items. If `resolve()` only records an action label, the loop has no durable “B merged into A” fact to respect, so it may rediscover or recreate the same conflict.
- **Including `Thin`/`Conflict` before merge memory:** risky. Include `Thin` once normalizer/dedup is fixed; include `Conflict` only after merge execution and tombstone/alias handling exist.
- **Field locking before convergence:** should be fixed first. Otherwise the loop may still fail to update text fields cleared by the user because text fields remain locked while covers do not.

**`source_empty` / `NotFound` policy**

Do not make `NotFound` terminal forever, but do not blindly sweep all empty sources either. Treat it as **per-source dormant with slow, triggerable retry**, not work-terminal. Current terminal behavior is cited at `domain/lib.rs:1356-1365`.

Recommended policy:

- Store `source_empty` per provider/source with `last_checked_at`, `next_retry_at`, `failure_kind`, and adapter/version context.
- Retry on triggers:
  - new external ID or stronger anchor added,
  - user manually refreshes,
  - provider adapter/version changes,
  - work becomes monitored/incomplete,
  - long slow cadence expires.
- Cadence:
  - cooperative sources: 30d, then 90d, then 180d backoff;
  - hostile/rate-limited sources: manual-only or very low global token budget;
  - network/WAF/rate-limit failures are not content `NotFound` and should back off separately.
- The loop’s priority score should heavily discount `source_empty` unless there is new evidence.

This preserves long-tail discovery without turning Goodreads/OpenLibrary-style failures into background traffic.

**Leave-alone list**

Mostly right, with one caveat: the plan should not defer the **minimal constraint implied by user merges**. Full must/cannot-link clustering is overbuilt, but “B was merged into A; do not recreate B” is load-bearing. That can be an alias/tombstone table, not a constraint solver.

Other deferrals look reasonable for single-user SQLite:

- Full append-only observation/event sourcing: defer.
- Per-field authority at full grain: defer; category-level plus user locks is enough.
- Drift detection: defer.
- Corroboration scoring: defer, but keep the tactical “fuzzy alone cannot auto-merge” guard.
- Blocking index: defer, but enforce concrete uniqueness/lookup constraints for anchors and external IDs.

**Misses / refinements**

- The convergence job needs a **dry-run or capped first pass**. First enable it for a small batch with logs/metrics before sweeping the whole library.
- Add explicit idempotency requirements: repeated convergence must not create new works, duplicate anchors, or undo merges.
- Make `0.90` configurable as planned; the cited hardcode at `matching/lib.rs:185` is exactly the kind of hidden policy that makes background automation dangerous.
- Clarify that “restore loop” includes per-provider budgets, not just registering a recurring job.

Net: the plan’s architecture call is right, but the implementation sequence should be “stop bad writes, make user corrections durable, then automate replay.”

---

## Claude (opus-4-8) — author of DELTA-CC

### 1. Priority / sequencing
The plan's *product* priority (convergence loop = #1) is right, but it states no execution order, and that omission is the main risk. **A must be the last step turned on, not the first.** A background loop re-running the pipeline across the whole library while the normalizer still mangles titles (C1/C2/C3) and `add()` still 500s on same-ISBN variants (D2) converts point-corruption into bulk-corruption automatically. Correct order: stop active corruption → make user corrections durable (merge executor) → fix field-lock semantics → then enable the loop, capped and observable. (Both other families reached the same conclusion independently — strong signal.)

### 2. The coupling the plan treats as separable
**A and the merge-executor (E1) cannot ship independently.** `converge_identity_pending` (async_resolver.rs:26) re-runs resolution from the seed with **no memory of prior user merges/splits** — safe only *because convergence is manual today*. Restoring the loop removes that mitigation, so a background pass can re-undo a user's merge. E1's "don't undo it" therefore needs a durable "stays-merged" assertion shipped **with or before** A, or A re-introduces the exact bug E1 fixes. All three reviews land here.

### 3. Internal inconsistency: fix A re-imports over-engineering the plan rejects
Fix A says "Add priority (incompleteness × P(retry helps) × budget) + per-source backoff." DELTA-CC explicitly called that formula over-built for a single-user app; a plain timed walk of `list_works_due_for_retry` (already exists, ordered by `added_at`) suffices. Gemini flagged the same formula as "unnecessary math." **Ship the dumb timed walk first; add a priority score only if Pending genuinely starves.**

### 4. `source_empty` recommendation (my POV on the one open call)
Don't auto-retry `NotFound` on a blind cadence. **Gate retry on a reason to believe the answer changed** — primarily a *new alias/anchor learned* (then re-query by exact key, not fuzzy title), plus user trigger and a new media-format add. Hostile sources (Goodreads WAF, OL UA burns — both live constraints) stay manual-only or on a very long cap. This is exactly where Gemini and Codex converged (trigger-over-timer + source-specific decaying backoff); I'd weight the *new-anchor* trigger as the primary one and treat blind time-based retry as the fallback, not the mechanism. ~75% confidence; what would change it: evidence a specific source reliably gains long-tail books over weeks.

### 5. Safety the plan omits
"Drains the Pending sink" = a bulk write across real library data on first run. **Snapshot the DB + dry-run/count log before the first production sweep** — not a cold flip. Codex independently called for a "dry-run or capped first pass" and idempotency guarantees (no new works, no duplicate anchors, no undone merges). Add those as explicit acceptance criteria on fix A.

### 6. Provenance of the claims (honesty flag)
I personally re-read and confirmed: convergence-deleted (jobs/mod.rs 7 jobs; work.rs:315), `cleared` asymmetry (lib.rs:840 vs :949), `NotFound` terminal (lib.rs:1356-1365). I did **not** independently verify the items inherited from the other two deltas — **D2 bridge-anchor-dedup, RC1 author-monitor false-Confirmed, the C1/C2/C3 normalizer bugs, the 5b corroboration guard**. They cite lines and are plausible, but rest on the verified-audit/other-delta, not my reads. Verify D2, RC1, and the normalizer before they anchor a P0 list.

### 7. Where I disagree with the other reviewers
- **Gemini says the blocking index "cannot be deferred"** (O(N) load of all works into a `Vec` per add). That's a real perf smell, but it's a *query-shape* fix (use the existing `idx_works_user_normalized` index instead of in-memory scan), **not** the blocking/candidate-generation *architecture* the plan deferred. The plan is right to defer the architecture; Gemini is right that the current `Vec` scan should become an indexed query. These are different items — keep both straight, and treat the indexed-query fix as a small tactical add, not a reversal of the "leave alone" call.
- **Gemini's `tag_convergence` write-race** is worth checking but may be a non-issue if writes are already per-work serialized; flag to verify, don't assume.

### Net
The plan is sound and genuinely tighter than the three-decisions framing. The three reviews agree on the substance: **(1) reverse the execution order — stabilize, make corrections durable, then automate; (2) A and the merge-executor are coupled; (3) drop the priority formula from A's first cut; (4) source_empty = triggered (new-anchor) retry, not blind cadence; (5) snapshot + capped first run.** My only standalone caution is provenance: three of the P0s are inherited, not independently verified.
