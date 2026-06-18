# Livrarr — Architecture Ideas (running list)

**Purpose.** Capture provider-adapter design ideas as we discuss them.
**Pipeline.** This list → CC investigation (how / whether already in code) → deltas → 3-LLM review → merge into `livrarr-execution-plan.md`.
**Status.** Research pass complete — items now carry canonical pattern names + prior-art validation (see end). Next: submit the **whole list** for cross-model review.
**Scale target.** ~90% single-user, ~99% ≤5 users; multi-user is an eventual goal (not the dominant case). Optimize for **1–5 users**. Don't build for a fleet / SLA / multi-tenant-at-scale — **but don't hard-block multi-user either.** Structural consequence: provider adapters, the per-source queue / rate-limit / breaker, the cache, and the convergence loop are **shared per instance** (one polite client per source for everyone); library membership and (probably) overrides are **per-user**. *This note governs every over-engineering judgment below.*
**Status of code references below.** Memory-based *leads*, not confirmed implementation. The whole point of the CC pass is to establish the *how* — do not treat leads as facts.

---

## Principles (govern all items below)

**P1 — One pipeline, bounded differences.** As soon as possible and as far as possible, the process is uniform (DRY, modular). Variation is confined to exactly two slots:
- **Data** — the precedence / config table (#6) and adapter membership (#1). Different situation = different rows, never different code.
- **Strategy traits** — a bounded, named set (normalizer per language; query-builder per source — already inside the adapter, #2). The pipeline invokes them uniformly; only the bound implementation differs.

**Convergence point** = the normalized-observation boundary (#2). Upstream (inside adapters) differences are allowed and expected; downstream is uniform and situation-blind (generalizes #5).

Two senses of "as soon as possible":
- *In the pipeline* — converge to normalized observations early; zero situational branching after.
- *In the build* — ship the uniform default path first; add bounded differences later (matches the execution-plan phase order).

**Allowed exception (bounded ≠ zero):** a real domain difference may legitimately require distinct behavior — e.g. the translation identity decision (D1) may need separate-Work logic in the identity layer. Make such differences *explicit and contained*; never let DRY steamroll a genuine divergence.

**Invariant (CC-checkable + lint going forward):** no situational branch (`language == …`, media-type switch) downstream of the observation boundary. Any such branch is a leak — push it up into a table or a strategy trait.

**P2 — Safe degradation.** A failing or unavailable source must **never** write identity. When a source is open/erroring (#8) or its data is missing, degrade by serving last-cached values (#4) or leaving the field unresolved — never by falling back to a guess (e.g. name-search against a different source) that can stamp a wrong identity. *Source: resilience literature + a real Jellyfin production bug (see Prior art).*
- **Invariant (CC-checkable):** no identity write originates from a fallback path triggered by a failed/unavailable primary source.

---

## Open decisions (need the review's position)
Both block design; the cross-model review must take a position on each.
- **D1 — Translation identity.** Is a translation the **same Work** (collapsed, language-tagged) or a **separate Work** linked as a translation? Blocks #6's config design and engages P1's exception clause. *No firm lean — genuinely unsure.*
- **D2 — Multi-user override/lock scope.** Are user overrides, field-locks, and "stays-merged" facts **global** (shared golden record — one user's edit changes everyone's view) or **per-user** (a personal override layer over the shared catalog)? This is the multi-user scope of the MDM *override rule* (#6). *Lean: global now (1–5 trusting users = simpler), but key the storage so a `user` dimension can be added later without a rewrite — i.e. don't hard-block per-user (Scale target).*

---

## Components

## 1. Provider adapter = the consolidation seam
One self-contained module per source, owning `{interface, queue, cache}`. Replaces today's scattered provider concerns (the audit reported provider logic spread across ~4 places).
- **Investigate:** Where do provider concerns currently live? Map every site that talks to / configures / caches / rate-limits a source.
- **Leads (verify):** external IDs reportedly in 3 stores; `transport_cache`; `provider_retry_state`; `provider_policy`.
- **Canonical name:** the *adapter* in **ports & adapters (hexagonal architecture)**.

## 2. Interface returns *normalized observations* (not raw payloads)
Common trait. Each adapter hides its source's quirks (Google: no work concept; Goodreads: LLM scrape; Audible: ASIN-keyed) and emits a uniform shape — `(field, value)` observations + external IDs / anchors. Everything above becomes provider-agnostic.
- **Investigate:** Is there already a common provider trait? Does it return raw payload or normalized data? Where does normalization happen today — per-source or central?
- **Leads (verify):** `provider_retry_state.normalized_payload_json` implies normalization exists *somewhere*; `MetadataProvider` enum; `derive_anchor_query`.
- **Implementation (Rust):** provider set is closed and self-controlled → **enum dispatch** (`enum_dispatch` crate to kill boilerplate), trait as the contract. The win is **compile-time exhaustiveness** when adding a provider — *not* speed (calls are network-bound; vtable cost is irrelevant). The existing `MetadataProvider` enum already leans this way.
- **Canonical name:** the normalized-observation boundary is an **anti-corruption layer** (Evans, DDD).

## 3. Per-source queue = the home of rate-limiting / backoff
One queue per provider (not per-request), **shared across all callers and all users** — exactly one polite client per source per instance. The convergence loop *enqueues*; the queue drains politely. This is where hostile-source policy (Goodreads / OL slow or manual-only, per the confer) lives. (Multi-user makes the sharing load-bearing: per-user rate-limiting → N× traffic → bans.) Keep transient — rebuilt from the DB's "works due" state on restart; DB is the durable truth.
- **Investigate:** Is there any per-source queue or rate-limiter today? How is backoff / politeness enforced, if at all? Is any queue state persisted?
- **Leads (verify):** `provider_policy`; `provider_retry_state`.
- **Also enforces (table-stakes resilience):** **bulkhead** = bounded per-source concurrency (cap in-flight requests, not just rate — matters more with multiple users); per-request **timeout** (never wait forever); **jitter** on backoff delays.
- **Distinct concern:** failure isolation (circuit breaker) lives alongside the queue but is its own thing — see #8.

## 4. Per-source cache; TTL doubles as the `source_empty` retry clock
Keyed by query / anchor. Cache hit = don't re-ask; expiry = eligible to re-ask. Unifies caching with retry-cadence — one mechanism, not two.
- **Ties to:** execution-plan **Phase 3** (`source_empty` / triggered retry). Delta-merge must reconcile here.
- **Investigate:** What caching exists, keyed how, with what TTL / eviction? Is cache-expiry currently wired to any re-query decision?
- **Leads (verify):** `transport_cache` (payloads cached under `CandidateId`); `provider_retry_state.normalized_payload_json` (latest payload per provider).
- **Also the fallback store (P2):** when a source is open/failing, serve last-cached here rather than guessing.
- **Shared across users:** book metadata is user-independent → one cache for the instance; one fetch serves everyone (reinforces #3's single client per source).

## 5. Invariant: the provider-agnostic line
Resolver + enrichment consume normalized observations only — never source identity or quirks. Recorded as an architectural rule to hold during refactor.
- **Investigate:** Do resolver / enrichment branch on provider identity anywhere today? List every provider-specific branch above the adapter layer.

## 6. Situation-conditioned provider configuration
A *situation* selects a provider configuration. Start with two discriminators: **language** and **media-type**.

**What varies:**
- **Membership** — which providers are active (Korean → Aladin / RIDI / Nat'l Library of Korea; drop Goodreads).
- **Priority** — already per-field (per-category authority); this adds a second axis → `(field × situation)`.
- **Per-field trust** — which fields a source may set, not just its rank.
- **Normalization** — language-aware (CJK: no space tokens, romanization variants, surname-first). Reaches into the P0 canonical normalizer.
- **Query construction** — script / romanization, per source.

**Model:** one precedence table keyed by `(situation, field) → ordered provider list`; membership falls out (absent = inactive).

**NOT config — decisions:**
- **Identity:** is a translation the same Work (collapsed, language-tagged) or a separate linked Work? Reopens the work-identity boundary *narrowly* for translations — the "collapse editions" lock covered same-text editions only. **Decide first — see D1.**
- **Bootstrap:** language is discovered from metadata, then reconfigures the set → default set → learn language → situation config. Re-pick happens in the convergence loop (ties to Phase 3).

**Guard rail:** static table, two discriminators only, explicit identity call — not a general rules engine.

- **Investigate (CC):** Where do field priorities live today (per-category authority structure)? Is language captured and used in provider selection or normalization at all? Any media-type-based provider routing? Any notion of per-language / per-situation config?
- **Leads (verify):** per-category authority; `derive_anchor_query`; canonical normalizer work (P0, C1/C2/C3).
- **Canonical names (MDM survivorship):** the per-field priority *is* the **consolidation rule**; the user-override *is* the **override rule**. Adopt this vocabulary — it names the F3 / cleared-flag bugs precisely (they are override-rule defects).
- **Optional tiebreakers (DEFER):** within a priority tier, recency / completeness can break ties. Defer until a real conflict appears — adding preemptively is over-build.
- **Eventual per-user dimension:** users may prefer different situations/providers → the precedence key may grow to `(user?, situation, field)`. Don't bake a single global config so deep that per-user becomes a rewrite (Scale target; see D2).

## 7. Convergence loop = level-triggered reconciliation
The loop's *decision* derives from **current persisted state** (which works have gaps), not from the event that woke it. Events (`source_empty`, new anchor learned, settings change, source recovered) are **wake-up nudges**, not the logic. A low-frequency **periodic resync** is the safety net so nothing is permanently stranded.
- **Why not edge-triggered:** edge-triggering loses state on missed/dropped events and may never recover it; level-triggering is self-correcting and forgiving of lost / duplicate / out-of-order events.
- **No-op cheaply:** diff before write; write only on change. Prevents a hot write-loop and the `tag_convergence` write-race the confer flagged (more likely under multi-user concurrency).
- **Refines:** execution-plan **Phase 3** — sharpens the confer's "triggered, not cadence" into its correct form (level-triggered, events-as-hints, + resync).
- **Investigate (CC):** Was the (deleted) reconciler edge- or level-shaped? Does it derive action from state or from the triggering event? Is there any periodic resync today?
- **Leads (verify):** `enrichment_retry_tick` (deleted); `list_works_due_for_retry`; `converge_identity_pending`.
- **Canonical name:** **reconciliation / controller loop** (idempotent, level-triggered).

## 8. Circuit breaker per source (failure isolation)
Distinct from backoff/politeness (#3): backoff expects eventual success; the breaker stops calling a source that is *persistently* failing (timeouts, 5xx, 403 / ban). After N hard failures → trip **open** for a cooldown → one **half-open** probe → reopen on success. The convergence loop **skips open sources**. When open, degrade per **P2**.
- **Scope:** light. Counter + cooldown + half-open probe; the full enterprise 3-state machine is still overkill. With providers **shared across up to 5 users** (Scale target), failure isolation is *more* justified than at single-user — a down/banned source hits everyone, and the breaker protects them all at once.
- **Investigate (CC):** Any failure-isolation today, or only retry/backoff? How are hard failures (ban / 5xx) currently distinguished from transient ones?
- **Leads (verify):** `provider_policy`; `provider_retry_state`.

---

## Prior art & canonical names (grounding for reviewers)

- **Provider adapter + normalized boundary** = ports & adapters (hexagonal architecture) + anti-corruption layer (Cockburn 2005; Evans, DDD).
- **Identity resolution** = the entity-resolution pipeline: *attribute alignment → blocking → matching → canonicalization*. We use the standard hybrid — deterministic anchors (ISBN/ASIN as blocking keys) + guarded fuzzy — and "standardize before matching" (normalizer-first, our P0).
- **Field merge** = golden-record survivorship (MDM): *consolidation rule* + *override rule*, configured per attribute.
- **Convergence loop** = reconciliation / controller loop: idempotent, level-triggered, events-as-hints, periodic resync.

**Production validation — Jellyfin (same problem space):**
- Its ProviderManager is a single registry, filtered + sorted **per item** by config and built-in priorities, with **separate ordering per field-class** (metadata / image / local). → confirms #1 + #6 (per-item config, per-category authority).
- Bug: a manual Identify does **not** stick — other enabled providers re-run, name-search, and pollute the item; the fix is to **lock after identifying**. → confirms execution-plan **Phase 1 lock + Phase 2 durable user-decision** sequencing (turning the loop on before durable decisions exist = corruption).
- Bug: a **down provider** triggers a fallback that **mis-identifies permanently**. → confirms **#8 circuit breaker + P2 safe degradation**.

---

*Open list — append new items below as the architecture discussion continues.*
