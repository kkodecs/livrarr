# Livrarr Architecture — Cross-Model Review Prompt

**Orchestration (Claude Code).** CC runs this review itself, spawns the adversarial runs (Gemini CLI, Codex CLI) with this same prompt + `livrarr-architecture-running-list.md`, and compiles all reports **verbatim** (no summarizing, no synthesis, no editing) into one output for reconciliation. Three independent reviews in, three reports out, untouched.

## Your task (each reviewer)
You are one of three **independent** reviewers. Red-team the attached architecture list. **Do not rubber-stamp.** For every principle and component, argue the strongest case for cutting or simplifying it *before* deciding whether it survives. Disagreement between reviewers is useful signal — it gets reconciled afterward.

## Context (read before reviewing)
- **Subject:** the book identity + metadata subsystem of Livrarr — an existing, working, self-hosted Rust app (SQLite, single process, async). Providers: OpenLibrary, Hardcover, Google Books, Goodreads, Audible, Audnexus.
- **This is an evolution of working code, not greenfield.** The existing system's bones already match a sound clean-room design (internal-ID key, signal hierarchy, two-threshold review band, per-category authority, attempt ledger). **Bias toward minimal change.**
- **Scale target:** ~90% single-user, ~99% **≤5 users**; multi-user is an eventual goal (not the dominant case). **Optimize for 1–5 users.** Reject fleet / SLA / multi-tenant-at-scale gold-plating — *and* reject anything that **hard-blocks multi-user**. Both directions are failures. Structural split to assume: **shared per instance** (provider adapters, per-source queue / rate-limit / breaker, cache, convergence loop — one polite client per source for everyone) vs **per-user** (library membership, and probably overrides — but that scope is open, see D2). Several items were mined from enterprise / MDM / Kubernetes literature; separate the useful idea from the at-scale gold-plating.
- **An execution plan already exists** (`livrarr-execution-plan.md`), three phases: **Phase 1** freeze/correctness (canonical normalizer, ISBN/ASIN dedup, field-lock fixes); **Phase 2** durable corrections (merge executor + "stays-merged" facts); **Phase 3** automation (restore the convergence loop **last**, capped, observable). The architecture must be consistent with — and sequence into — that plan.
- **House rules:** extreme bias against over-engineering; say "I don't know"; flag confidence with real numbers; no hedged synthesis.

## What the list contains
- **2 principles:** **P1** — one pipeline, bounded differences (DRY; situational variation confined to data tables + a bounded set of strategy traits; uniform downstream of the normalized-observation boundary). **P2** — safe degradation (a failing source never writes identity).
- **2 open decisions:** **D1** translation identity (same Work vs separate Work); **D2** multi-user override/lock scope (global vs per-user).
- **8 components:** (1) provider adapter as the consolidation seam; (2) interface returns normalized observations [enum_dispatch]; (3) per-source queue [rate-limit / backoff / bulkhead / timeout / jitter, shared per source]; (4) per-source cache [TTL = `source_empty` retry clock, shared]; (5) provider-agnostic-line invariant; (6) situation-conditioned provider config [language + media-type discriminators; precedence table keyed by `(situation, field)`]; (7) convergence loop = level-triggered reconciliation; (8) circuit breaker per source.
- **Prior-art / validation block** (hexagonal/ACL, entity-resolution pipeline, golden-record survivorship, reconciliation loop; Jellyfin production-bug validation).

## Review along these axes
1. **Right-sizing (priority, two-sided).** Per item: (a) is this gold-plating for 1–5 users that should be cut/simplified — state the simplest version that still works; **and** (b) does it bake in a single-user assumption that would **hard-block** multi-user? Call out both directions.
2. **Multi-user (1–5).** What does eventual multi-user actually change? What's currently single-user-baked that shouldn't be? What would a wrong call *now* make a rewrite *later*? (Focus on the shared-vs-per-user split: providers/queue/cache/loop shared; library/overrides per-user?)
3. **Gaps.** What's missing that a 1–5-user metadata system actually needs? Only real gaps — don't pad.
4. **Internal coherence.** Do any principles/components/decisions contradict each other? Specific suspects: does **P2** (serve cache / never guess identity) conflict with **#4** (cache as the retry mechanism)? does **P1**'s uniformity mandate fight **#6**'s situational variation or **#8**'s per-source breaker? is **#7** (level-triggered loop) consistent with **#4** (cache-TTL-as-retry-clock), or do they encode two different retry triggers that must be reconciled? does **D2** (override scope) interact with the shared-cache / shared-Works assumption?
5. **Rust feasibility.** Single process, SQLite, async. Is **enum_dispatch (#2)** right vs trait objects, given a closed provider set? Is a **level-triggered loop with periodic resync (#7)** straightforward here? Concurrency / write-race concerns (the list flags `tag_convergence`, made worse under multi-user)?
6. **Sequencing.** Does the architecture fit Phase 1→2→3, and specifically the rule "turn the convergence loop on **LAST**, after durable user-decision facts exist"? Flag anything that forces out-of-order work.
7. **The open decisions.** Take a position on **both**: **D1** — is a translation the **same Work** (collapsed, language-tagged) or a **separate Work** linked as a translation? **D2** — are overrides / locks / "stays-merged" facts **global** or **per-user**? Reasoning + confidence for each.

## Output format
- **Per item (P1, P2, 1–8):** verdict = **KEEP / SIMPLIFY / CUT**, one-line rationale, confidence %.
- **Gaps:** bulleted; each with why-it-matters-for-1–5-users.
- **Coherence conflicts:** contradictions found + the fix.
- **Decisions (D1, D2):** position + reasoning + confidence for each.
- **Top 3 changes** if you could change only three.
