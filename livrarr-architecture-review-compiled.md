# Livrarr Architecture — Three Independent Reviews (compiled verbatim)

Red-team review of `livrarr-architecture-running-list.md`. Three independent reviewers, three reports, reproduced **verbatim** below (boot/tool-trace noise stripped; review text untouched). Reconciliation is a separate step — this file is the raw input to it.

- **Gemini** (gemini-3.1-pro-preview) — code-grounded (read DELTA-CC, reconciled plan, wiki).
- **Codex** (gpt-5.5) — **text-only**; it self-reported that shell + Serena + code-index all failed in its sandbox, so its leads are NOT code-verified. Preserved as-is.
- **Claude** (opus-4-8) — code-grounded where cited; author of DELTA-CC.

---
---

# REVIEWER 1 — GEMINI (verbatim)

## 1. Per-Item Architectural Verdict (Red-Team Assessment)

### P1 — One pipeline, bounded differences
* **Verdict:** **SIMPLIFY**
* **Rationale:** A rigid uniformity check that bans *all* situational branches downstream is academic gold-plating. Keep the pipeline structured, but allow clean, explicit Rust branches for specific, high-signal language/media variations over an over-engineered table-driven runtime config.
* **Confidence:** 92%

### P2 — Safe degradation
* **Verdict:** **KEEP**
* **Rationale:** Essential defense against silent data corruption. Stamping a guessed identity because a primary provider failed is how databases get polluted and local file tags permanently corrupted (M6).
* **Confidence:** 96%

### 1. Provider adapter = the consolidation seam
* **Verdict:** **SIMPLIFY**
* **Rationale:** Consolidate external adapter calls, but keep the scheduling, queue, and retry logic centralized in a single DB-backed subsystem (`provider_retry_state`) rather than repeating queue mechanics in every adapter module.
* **Confidence:** 90%

### 2. Interface returns *normalized observations* (not raw payloads)
* **Verdict:** **SIMPLIFY**
* **Rationale:** Return a uniform, typed Rust struct (e.g., `NormalizedBookMetadata`) rather than dynamic streams of generic `(field, value)` tuples. Static typing is cleaner and safer in Rust; use `enum_dispatch` for compile-time exhaustiveness.
* **Confidence:** 91%

### 3. Per-source queue = the home of rate-limiting / backoff
* **Verdict:** **SIMPLIFY**
* **Rationale:** Bounded concurrent actors or complex bulkhead queues are overkill. Use a passive, crash-resilient database-backed schedule (using `provider_retry_state.next_attempt_at`) combined with a simple semaphore in the HTTP client.
* **Confidence:** 93%

### 4. Per-source cache; TTL doubles as the `source_empty` retry clock
* **Verdict:** **CUT**
* **Rationale:** Conflating transport/HTTP cache TTL with the domain-level retry schedule is a structural anti-pattern. Cache external JSON responses purely for transport efficiency, and let the domain retry ledger separately govern empty-source schedules.
* **Confidence:** 95%

### 5. Invariant: the provider-agnostic line
* **Verdict:** **KEEP**
* **Rationale:** Keeping downstream resolver and merge logic isolated from provider-specific JSON quirks is the core of a clean, testable design.
* **Confidence:** 94%

### 6. Situation-conditioned provider configuration
* **Verdict:** **SIMPLIFY**
* **Rationale:** A dynamic database precedence table mapping 24 fields to 7 providers is unmaintainable for self-hosters. Keep static priority models in Rust code (such as the existing 4-category split: Content, Description, Cover, Audio) and select the priority model at merge start.
* **Confidence:** 94%

### 7. Convergence loop = level-triggered reconciliation
* **Verdict:** **KEEP / SIMPLIFY**
* **Rationale:** Level-triggered reconciliation is the only self-healing pattern. However, avoid complex reactive event-nudge machinery that risks write-races with the 60s `tag_convergence` loop; use a simple, budgeted database-polling interval (e.g., every 10–15 minutes).
* **Confidence:** 95%

### 8. Circuit breaker per source (failure isolation)
* **Verdict:** **SIMPLIFY**
* **Rationale:** A 3-state (Closed/Open/Half-Open) enterprise machine is over-engineered. Implement a stateless DB penalty: if a provider returns consecutive 5xx or rate-limit failures, apply a multiplier that moves its `next_attempt_at` significantly into the future.
* **Confidence:** 96%

---

## 2. Gaps (Real 1–5 User Needs)

* **Durable Merge Executor (E1/D1):** `LiveIdentityConflictService::resolve` only records action labels; it lacks an executor to repoint `library_items.work_id`, tombstone the merged work, and write a durable alias. Without this, background loops will eventually undo manual user splits/merges.
* **ISBN/ASIN Dedup in `add()` Loop (D2):** The current `add()` flow only deduplicates based on `ol_key/gr_key/hc_key`. Adding works with identical ISBNs but title variations misses dedup and crashes on a database `UNIQUE` constraint (HTTP 500). ISBN/ASIN must be included as primary dedup keys.
* **Symmetric override-lock `cleared` behavior:** The text-merge path checks `setter == User` without verifying `!fp.cleared`, permanently locking out text updates after a user edit is cleared. Only the cover path currently respects `!fp.cleared`.
* **The `NotConfigured` Reset Chokepoint:** Providers that are unconfigured are permanently set to `OutcomeClass::NotConfigured`. The system must automatically trigger `reset_not_configured_outcomes` when a user updates their provider configuration settings.
* **Configurable Auto-Confirm Threshold:** The `0.90` auto-confirm score is currently hardcoded in `matching/lib.rs:185`. It must be moved to configuration (`ResolverConfig`) to respect the rule that thresholds are user-tunable.

---

## 3. Coherence Conflicts & Solutions

### Conflict 1: Re-resolution loop undoes manual merges
* **Description:** The reconciler (`converge_identity_pending`) re-runs resolution from the seed with no knowledge of prior manual merges.
* **Fix:** The Merge Executor must write a durable, "stays-merged" alias fact (e.g., `tombstone_work_id` -> `target_work_id`), which the reconciler reads to instantly bypass re-resolution.

### Conflict 2: Concurrency write-race with file tagging
* **Description:** `tag_convergence` runs every 60s to write metadata to files. Concurrent reconciler writes to `Work` rows will cause write-races or DB locks in SQLite.
* **Fix:** Reconciler database transactions must synchronize or serialize with the file tag-writing queue (using a per-work write lock or serializing background runners).

### Conflict 3: Hostile rate-limits vs. `NotFound` slow-cadence retry
* **Description:** Sweeping Goodreads or OpenLibrary for `NotFound` items on a flat-time cadence will trigger IP blocks or scrapers crashing.
* **Fix:** Adopt a **Trigger-over-Timer** retry policy. Never auto-retry `NotFound` on a flat blind clock. Instead, gate retries on system triggers (e.g., acquiring a new hard anchor ID elsewhere, manual user refresh, or configuration changes).

---

## 4. Decisions (D1, D2)

### D1 — Translation Identity
* **Position:** **Separate Work, linked.**
* **Reasoning:** Collapsing translations into a single Work creates major data-modeling friction (colliding titles, descriptions, and covers across different languages). Separate Works keep file-tagging (M6) completely coherent for bilingual users while a self-referential `translation_of` key preserves semantic grouping.
* **Confidence:** 95%

### D2 — Multi-User Override/Lock Scope
* **Position:** **Global catalog, key-prepared storage.**
* **Reasoning:** 1–5 trusted household users share a single physical library; they want changes made by one user (such as a corrected cover or title) to be visible to all. Keep overrides global for simplicity, but key all schema rows with a nullable `user_id` so per-user overrides can be added later without a schema migration.
* **Confidence:** 92%

---

## 5. Top 3 Strategic Changes

1. **Implement the Merge Executor before the Reconciler Loop:** Stop the loop from undoing user decisions. Ensure `resolve()` executes a real DB write (tombstoning, repointing `library_items`, writing durable aliases) so the reconciler respects user merges.
2. **Reverse the Execution Order (Stabilize first):** Turn on the background convergence loop **last**. Fix the normalizer bugs, `cleared` text asymmetry, and ISBN-dedup crashes *before* automating background runs to prevent bulk-amplifying existing corruption.
3. **Trigger-over-Timer `NotFound` Retry Policy:** Avoid flat periodic checks on empty sources. Keep Goodreads and OpenLibrary `NotFound` states terminal until a high-signal event (e.g., learning a new external ID, or a user manual refresh) triggers an explicit retry.

---
---

# REVIEWER 2 — CODEX (verbatim)

> I could not verify the repo files: shell commands failed before execution with `bwrap: loopback: Failed RTM_NEWADDR`, Serena timed out, and code-index calls returned cancelled. So this is a red-team review of the supplied architecture text, not a code-grounded confirmation of the memory leads.

**Per-Item Verdicts**

| Item | Verdict | Rationale | Confidence |
|---|---:|---|---:|
| **P1** | **SIMPLIFY** | Keep the normalized-observation boundary, but the "no downstream branch" invariant is too absolute; identity rules may legitimately vary by media/language/translation. | 75% |
| **P2** | **KEEP** | "Never write identity from fallback" is the strongest safety rule here and directly supports Phase 1 correctness. | 90% |
| **#1 Provider adapter** | **SIMPLIFY** | Adapters should own provider translation/query quirks; queue/cache/breaker can be shared provider-scoped infrastructure instead of fully embedded per adapter. | 70% |
| **#2 Normalized observations** | **KEEP** | This is the right anti-corruption boundary. I would not mandate `enum_dispatch`; use it only if it fits the existing async/provider shape cleanly. | 80% |
| **#3 Per-source queue** | **KEEP** | One shared polite client per source is right for 1-5 users and avoids provider abuse. Simplest form: DB-derived due work plus bounded async worker, not a heavyweight durable queue. | 85% |
| **#4 Per-source cache / TTL retry clock** | **SIMPLIFY** | Cache TTL can prevent re-asking too often, but it should not be the whole retry model; empty/failure retry state and cached successful payloads have different semantics. | 80% |
| **#5 Provider-agnostic line** | **KEEP** | Resolver/enrichment consuming normalized observations only is the core simplification. Allow explicit identity-layer exceptions. | 85% |
| **#6 Situation config** | **SIMPLIFY** | Per-field authority by language/media is useful, but keep it static and tiny. Do not add per-user config until a real product need appears. | 75% |
| **#7 Level-triggered loop** | **KEEP** | This fits SQLite/single-process Rust well and is safer than edge-triggered retries. Must be Phase 3 only, after durable merge/lock facts exist. | 85% |
| **#8 Circuit breaker** | **SIMPLIFY** | Keep a light per-source breaker, but avoid enterprise state-machine ceremony. Counter, cooldown, half-open probe, and observable reason are enough. | 80% |

**Gaps**

- **Explicit identity-write gate.** For 1-5 users, the most important missing piece is a narrow API/path that is allowed to mutate identity. Without that, P2 becomes an aspiration rather than an enforceable invariant.

- **Durable user-decision facts before automation.** The architecture mentions locks and "stays-merged," but these need to be first-class facts with clear precedence over provider observations before the convergence loop is restored.

- **Observed provenance per field.** A user needs to know why title/author/series changed: source, timestamp, field authority rule, and whether it was automated or manual.

- **Conflict/review band UX contract.** The two-threshold review band is mentioned in context but not represented strongly in the component list. For this domain, ambiguous merges must pause, not silently converge.

- **Schema path for multi-user ownership.** Library membership must be per-user from the start. Overrides can start global, but tables should not make "one global truth only" impossible to unwind.

**Coherence Conflicts**

- **P2 vs #4:** Serving cached values during provider failure is fine for display/enrichment, but cached fallback must not create new identity links. Fix: classify cache use as `display/enrichment reuse` vs `identity assertion`.

- **#4 vs #7:** TTL-as-retry-clock and level-triggered reconciliation are two different triggers. Fix: make the loop authoritative: it asks "what is currently due or incomplete?" TTL/cache expiry is one input to due-ness, not the scheduler itself.

- **P1 vs #6/#8:** "Uniform downstream" conflicts with situation-specific authority and source failures if interpreted literally. Fix: downstream code is uniform, but data-driven policy can vary by situation/source.

- **D2 vs shared cache/shared Works:** Shared cache is fine; shared Works plus per-user overrides need layering. Fix: shared catalog/work identity underneath, user-visible field layer above.

**Open Decisions**

**D1: translations should be separate Works linked as translations. Confidence: 70%.**
Collapsing translations into one Work sounds simpler, but it leaks language-specific titles, contributors, publishers, editions, narrator/audiobook metadata, and provider authority into a single identity. For a metadata system, translation is not just a display language tag. Use a relationship: `work A is translation_of work B` or `translation_group_id`.

**D2: overrides/locks/stays-merged should be global initially, but schema-keyed for per-user later. Confidence: 65%.**
For 1-5 trusted users, global corrections are much simpler and probably what users expect from a shared self-hosted catalog. But user library membership must be per-user immediately, and override tables should have a nullable or future-compatible `user_id` path so personal preference layers do not require a rewrite.

**Top 3 Changes**

1. Split **#4**: cache is not the retry mechanism; it is an input to the level-triggered loop plus a safe display fallback.
2. Make **durable decision facts** central: locks, overrides, merge decisions, and "stays-merged" must exist before Phase 3 automation.
3. Change **D1** to separate linked translation Works, and relax **P1** so identity-layer domain differences are explicit rather than treated as pipeline leaks.

---
---

# REVIEWER 3 — CLAUDE (verbatim)

Code-grounded where I cite file:line; I flag what I did not personally trace. Authored DELTA-CC, so I reuse its receipts.

## Per-item verdicts

### P1 — One pipeline, bounded differences — **SIMPLIFY** (88%)
The boundary idea is right and the lint is useful, but the absolute "zero situational branch downstream" is too strong, and the code already violates it benignly: provider-priority selection branches on language (`PriorityModel::english()` vs `foreign()`, enrichment/lib.rs:239-315) at merge start. That's a legitimate situation-select, not a leak. Keep the invariant as a *guideline* that pushes branches up into the model-selection step + strategy traits; don't promote it to a hard lint that would flag the D1 translation exception as a violation.

### P2 — Safe degradation — **KEEP** (95%)
Strongest rule in the list and the one with a real production-bug pedigree (Jellyfin down-provider mis-identify). It's also already the project's lived lesson — the SSRF/trusted-infra and Goodreads-needs-LLM history are the same shape: a degraded source must not stamp identity. Keep, and make it the one CC-checkable invariant worth a real test.

### #1 Provider adapter (consolidation seam) — **SIMPLIFY** (85%)
The consolidation is worth doing — provider logic *is* scattered (goodreads.rs ~60KB, google_books.rs ~43KB, plus provider_queue, provider_client, provider_policy, transport_cache). But "each adapter owns its own queue + cache" is the wrong cut: the queue and cache are *shared per source* (one polite client), so they can't live inside a per-call adapter instance. Adapter owns translation/normalization/query-building; the queue, cache, and breaker are shared infrastructure the adapter *uses*. Same conclusion Codex reached.

### #2 Normalized observations + enum_dispatch — **KEEP** (88%)
The boundary is the core simplification. On enum_dispatch specifically: it's not a free choice here — the project already hit `trait_variant::make(Send)` producing non-dyn-compatible traits and adopted enum dispatch for heterogeneous provider collections for exactly that reason (CLAUDE.md "foreign language retro" lesson; existing `MetadataProvider` enum). So enum_dispatch is the *already-forced* pattern, not new machinery — KEEP, and Gemini's "return a typed struct, not (field,value) tuples" is the right refinement (the code already passes typed provider payloads, not loose tuples).

### #3 Per-source queue — **KEEP, simplest form** (85%)
One shared polite client per source is correct and multi-user makes it load-bearing (per-user rate-limiting → N× traffic → bans). Simplest real version: DB-derived "due" set (`provider_retry_state.next_attempt_at` already exists) + a bounded async worker + a per-source semaphore. Don't build a durable in-memory queue; rebuild from DB on restart as the list says. Bulkhead/timeout/jitter are table-stakes, not gold-plating — keep them, they're a few lines each.

### #4 Cache TTL = the source_empty retry clock — **CUT the unification** (90%)
This is my sharpest code-grounded disagreement with the *list*. The proposal merges two clocks the codebase **already correctly separates**: `transport_cache` (cached payloads, transport efficiency) vs `provider_retry_state` (outcome + `next_attempt_at`, the domain retry ledger — DELTA-CC §4.2). Unifying them is a *regression*, not a simplification: a transport cache-hit (don't re-fetch) and "this source is eligible to re-ask for a work it lacked" are different questions with different lifetimes. Keep both; the retry clock stays in `provider_retry_state`. (Gemini CUT, Codex SIMPLIFY — I land with Gemini, and the receipt is that the split already exists.)

### #5 Provider-agnostic line — **KEEP** (90%)
Same as #2's downstream half. Worth recording as a rule; cheap to hold.

### #6 Situation-conditioned config — **SIMPLIFY hard** (88%)
Do **not** build a DB precedence table keyed by `(situation, field)` over 24 fields × 7 providers — unmaintainable and nobody will tune it (DELTA-CC §3.2 already called the full per-field map over-built). The language axis *already exists* as two hardcoded models (english/foreign); media-type is a small third discriminator. Right-sized version: keep static Rust priority models, add a `korean()`/`cjk()` model when Korean providers actually land, select the model at merge start. The valuable, non-config part is the **CJK-aware normalizer** (real work) and the MDM vocabulary (consolidation rule / override rule) — adopt the names, skip the rules engine.

### #7 Level-triggered convergence loop — **KEEP** (90%)
Level-triggered is the right shape and fits SQLite/single-process cleanly; it's strictly safer than the edge-triggered tick that was deleted. Two constraints: (a) it must be Phase 3, after durable merge/lock facts exist (all three of us agree); (b) diff-before-write is mandatory to avoid the `tag_convergence` 60s write-race — and note that's still unverified as a real race (it may already be per-work serialized; flag to check, don't assume). Simplest form is a budgeted poll of `list_works_due_for_retry` (exists, no live caller) — drop the `incompleteness × P(retry) × budget` formula until starvation is observed.

### #8 Circuit breaker — **SIMPLIFY** (88%)
Don't build the 3-state enterprise machine. Cheapest real version that the others also converged on: a consecutive-hard-failure counter that pushes `next_attempt_at` far out (Gemini's "stateless DB penalty") + one half-open probe + an observable reason. Multi-user makes it *more* justified (a banned source hits everyone), but that argues for keeping it light and shared, not elaborate.

## Gaps (1–5 users, real)
- **Identity-write chokepoint (echo Codex, strongest gap).** P2 is unenforceable without exactly one path allowed to write anchors. The code has anchor writers (`sqlite_work_identity.rs`: supersede/confirm), but I have **not** verified they're funneled through a single gate — that verification is the gap to close before P2 becomes a real invariant.
- **Merge executor before automation.** Already in the plan (Phase 2), but worth restating as the gating dependency for #7: `IdentityConflictService::resolve` writes a label and does nothing (DELTA-CC §2.4). Until it repoints/tombstones/aliases durably, the loop has no merge to respect.
- **Bootstrap re-resolution interacts with #7 and D1 (under-named).** #6's "learn language → reconfigure provider set" means a work can flip english→foreign *after* first resolution, changing its providers and (if D1 = separate-Work) possibly its identity. The level-triggered loop must treat "language discovered" as a due-ness input. Neither the list nor the other reviewers connected this; it's a real coherence thread.
- **No drift surface for user-vs-source disagreement** — deliberately deferred (DELTA-CC §3.4), correctly, for 1–5 users. Listing it only to confirm it stays cut.

## Coherence conflicts
- **#4 vs #7 (real, primary):** two retry triggers. Fix = the loop is authoritative ("what's due/incomplete?"); cache/TTL is at most one *input* to due-ness, not the scheduler. Both other reviewers independently flagged this — it's the highest-signal coherence finding.
- **P2 vs #4 (real):** serving last-cached on failure is fine for *display/enrichment* but must never create an *identity* link. Fix = classify cache reads as display-reuse vs identity-assertion; only the former may use stale cache.
- **P1 vs #6/#8 (apparent, resolvable):** "uniform downstream" vs situational authority/breaker. Not a true conflict — *code* is uniform, *data/policy* varies by situation/source. Resolve by stating P1 governs control flow, not data.
- **Self-inflicted conflict in #4:** as written, #4 contradicts the existing schema's correct two-clock separation. The cleanest fix is to not adopt #4's unification at all.

## Decisions
**D1 — Separate Work, linked by `translation_group_id`. (80%)**
Consistent with the existing collapse-editions model: editions = same text/same language (collapsed onto one Work, verified — title/lang/isbn all on `Work`); a translation = different language = different title/description/ISBN/ASIN/narrator, which can't ride one Work without multi-valuing every language-dependent field. So: separate Work. **But** add the nuance the others didn't: monitoring and series membership should operate on the *group*, so a bilingual user monitoring the work gets notified of either language. Separate identity, grouped intent. What would change my mind: if in practice users only ever want one language per work, the link adds cost for no benefit (~20% case).

**D2 — Global now, `user_id`-nullable-keyed for later. (85%)**
All three converge. My one grounded addition: library **membership is already user-scoped** (`library_items` UNIQUE includes `user_id`; provenance keyed by `user_id` — verified in DELTA-CC). What's *not* nailed down is whether the `works`/anchor catalog itself is shared or per-user — and that's precisely D2's storage-keying question. Recommendation: shared catalog + work identity underneath, a user-visible override/lock layer above, every override row carrying a nullable `user_id` (NULL = global today). Do not split the catalog per-user.

## Top 3 changes
1. **Reject #4's unification; keep the two clocks separate** (transport_cache vs provider_retry_state). It's the one item in the list that's a regression against working code, and it's the root of the #4-vs-#7 and P2-vs-#4 conflicts.
2. **Merge executor + durable "stays-merged" fact before the loop** (Phase 2 before Phase 3) — the gating dependency; without it #7 undoes user decisions.
3. **Right-size #6 to static Rust models + a CJK normalizer; drop the DB precedence table.** Adopt the MDM vocabulary, not the MDM machinery.

## Where I differ from the other two
- **#4:** I go further than Codex (SIMPLIFY) to Gemini's CUT, *and* I ground it — the separation already exists in the schema, so #4 is a regression, not just a smell.
- **enum_dispatch (#2):** Codex says "only if it fits"; I say it's already the forced choice (trait_variant dyn-incompatibility), so KEEP is firmer than Codex's hedge.
- **D1 confidence:** lower than Gemini's 95% — the separate-Work call is right, but the monitoring-on-the-group nuance is load-bearing and unaddressed, so I won't claim 95%.
