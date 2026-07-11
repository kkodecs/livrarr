# Cross-Model Reviews — design-work-service-split.md

**Date:** 2026-07-10
**Reviewers:** Gemini (via `gemini` CLI) and Codex (`codex exec`, model gpt-5.5), dispatched in parallel with an identical prompt.
**Provenance notes (read before weighing):**
- A **first review round was invalidated and discarded**: its prompt embedded the design's own conclusions and an A-vs-B framing as leading questions (PO called it — confirmation-bias risk). The reviews below are the clean second round.
- The round-2 prompt (verbatim, below) asked for a general adversarial review with NO framing of the design's claims and explicit license to reject the premise.
- **Codex could not read the design markdown** (its sandbox blocked shell/file reads of the .md; Serena only parses code), so its review grounds in `work_service.rs` + `convergence_service.rs` and critiques the *premise*, not the design text. It says so itself. Gemini read all three files.
- **Known factual dispute to settle by compile probe:** Gemini asserts child modules cannot access a parent module's private struct fields (its central argument against Pattern A). The design author holds the opposite (Rust privacy: items are visible to descendant modules; it's sibling *methods* that need `pub(super)`) — Codex's round-1 review and round-2 point 2 agree with the author. ~90% the author is right; a 5-line compile check settles it. Do not adopt Gemini's "Pattern A forces field widening" claim without running that probe.

## The prompt both reviewers received (verbatim)

> You are one of two independent reviewers giving an adversarial second opinion on a proposed Rust refactor. Be a skeptic, not a validator — your job is to push back, not to confirm. Disagree freely with the approach, the assumptions, the risk assessment, the sequencing, or the premise itself; if you think the refactor is wrong, unnecessary, premature, or should be done a different way, say so plainly. Stay reasonable and ground every claim in the actual files with path:line — reasonable pushback, not contrarianism for its own sake.
>
> Read and react to:
> - The design: /mnt/opt/livrarr/design-work-service-split.md
> - The file it proposes to split: /mnt/opt/livrarr/crates/livrarr-metadata/src/work_service.rs
> - For context (a prior extraction from the same file): /mnt/opt/livrarr/crates/livrarr-metadata/src/convergence_service.rs
>
> Give a general review in your own structure: what's wrong, what's risky, what's missing, what you would do differently, and whether the plan holds up. End with your single strongest objection.

---

## GEMINI review (verbatim)

### **The Verdict: Reject Pattern A. Choose Pattern B with Explicit Encapsulation.**

The design's recommendation of **Pattern A** (splitting the service into a module directory using split inherent `impl`s) is a trap in Rust. While it looks clean on paper, it hides structural coupling, degrades compiler safety, and introduces an idiomatic mess.

Instead, we should double down on **Pattern B** (sibling free-function modules), but with a strict encapsulation layer to avoid nakedly exposing the service's private fields.

### **1. What's Wrong: The Illusion of Pattern A**

Pattern A argues that keeping the methods inside inherent `impl WorkServiceImpl` blocks across separate files is cleaner because:
1. It avoids widening the visibility of the struct's private fields.
2. It represents the idiomatic "split one big impl across files" approach.

**Both claims are misleading in Rust:**

* **The Rust Module Visibility Paradox:** In Rust, descendant modules are *not* automatically granted access to the private fields of parent modules unless those fields are `pub(crate)` or `pub(super)`.
  Specifically, if `work_service/mod.rs` defines the struct, a sibling module defined in `work_service/creation.rs` (which is a submodule declared via `mod creation;` in `mod.rs`) **cannot** access private fields of `WorkServiceImpl` directly via `self.db` or `self.enrichment` *unless* those fields are already visible to it. Under Rust's privacy rules, parent modules can see submodule private parts, but **submodules cannot see parent private fields** unless they are marked `pub(in crate::work_service)` or `pub(super)`.
  Therefore, Pattern A **does not save us from widening field visibility**. We would still have to mark almost every field on the main struct as `pub(super)` or `pub(crate)`.
  *(⚠ disputed — see provenance notes; settle by compile probe before acting on this.)*

* **The Inherent `impl` Monolith:** Spreading `impl<D, E, H, L, M, T> WorkServiceImpl<D, E, H, L, M, T>` across 7 different files creates an immense amount of boilerplate. Every single file must duplicate the complex generic parameter declarations, trait bounds, and lifetimes. This is highly fragile and makes refactoring the type signature of `WorkServiceImpl` in the future a multi-file nightmare.

### **2. What's Risky: The "Shim" Churn & Compiler Blind Spots**

The proposal suggests generating 20 thin shims inside `mod.rs` for the `WorkService` trait implementation.

* **Why this is risky:** It introduces a useless boilerplate layer. It separates the trait contract from the actual execution logic, making navigation in editors harder (requiring two jumps instead of one).
* **Compiler Blind Spots (The Real Risk):** When you write free functions (Pattern B), the compiler strictly enforces what state you are allowed to touch because you must pass references or extract dependencies explicitly. In Pattern A, because every file has full access to `self`, developers will continue to casually reference unrelated fields (like `refresh_locks` inside `discovery.rs` or `lookup_cache` inside `creation.rs`). This guarantees **the god-object behavior will regrow within the submodules** because there are no physical compile-time boundaries preventing cross-concern state access.

### **3. What's Missing: A Structured Pattern B (The "Sub-Context" Approach)**

The existing precedent `convergence_service.rs` uses free functions, but the design flags a concern with Pattern B: *it requires widening private fields to `pub(crate)`.*

We can completely solve this concern by **extracting scoped Context interfaces** or **dependency tuples** instead of passing the raw `WorkServiceImpl` to free functions.

We define explicit, narrow, trait-based or struct-based interfaces for what each concern actually needs. For example, Creation (`creation.rs`) doesn't need the entire `WorkServiceImpl`. It only needs the `db`, `enrichment`, and `http` client:

```rust
// In work_service/creation.rs
pub(super) struct CreationContext<'a, D, E, H> {
    pub db: &'a D,
    pub enrichment: &'a E,
    pub http: &'a H,
    pub data_dir: &'a Path,
}

pub(super) async fn add_work<D, E, H, L, M, T>(
    ctx: CreationContext<'_, D, E, H>,
    req: AddWorkRequest,
) -> Result<WorkId, WorkServiceError> { ... }
```

**Why this is vastly superior:**
1. **Zero Field Visibility Leaks:** The fields on `WorkServiceImpl` can remain completely private within `mod.rs`. Only `mod.rs` instantiates the small, local `CreationContext` and passes it down.
2. **True Decoupling:** The compiler now structurally guarantees that the Creation code *cannot* touch `refresh_locks`, `llm`, `lookup_cache`, or any other unrelated service state. This completely prevents future regression and god-object regrowth.
3. **Unit Testability:** You can now unit-test the behavior of `add_work` by passing a mocked or simple `CreationContext` without having to construct a full `WorkServiceImpl` with all 12 of its heavy dependencies.

### **4. Single Strongest Objection**

> **The proposed Pattern A split is purely cosmetic.** By retaining `self` across 7 files, you are not actually breaking up the god object; you are merely sweeping it under 7 different rugs. Because every submodule still has implicit access to all of `WorkServiceImpl`'s fields, there is no compile-time enforcement of boundaries. Developers will continue to cross-contaminate concerns, and the implementation will silently re-tangle.

**The Fix:** Implement **Pattern B using encapsulated sub-contexts** (like `CreationContext`, `DiscoveryContext`). This enforces absolute isolation at the compiler level, keeps the core struct fields private, avoids generic-boilerplate duplication across files, and matches the established architectural pattern already started in `convergence_service.rs`.

---

## CODEX review (verbatim)

I need to be explicit: the sandbox blocked shell reads and code-index was unavailable, so I could not honestly inspect `design-work-service-split.md`. This review is grounded in the two Rust files and should be treated as a critique of the proposed split's premise, not a full markdown-design review.

**Findings**

1. The prior extraction is not a clean service split. `convergence_service.rs` imports `WorkServiceImpl` directly and its public functions take `&WorkServiceImpl<D, E, H, L, M, T>` with the full generic stack and DB bounds (convergence_service.rs:17, :25, :237). That is file extraction, not architectural decoupling. If the proposed refactor repeats this pattern, it mostly redistributes complexity across files while preserving the same dependency knot.

2. `WorkServiceImpl` is already the dependency container for DB, enrichment, HTTP fetcher/client, LLM, merge engine, tag service, locks, cache, and resolver (work_service.rs:32). Splitting behavior into "services" that still borrow this whole struct risks hiding coupling instead of reducing it. The constructor also wires all of this centrally, including mutable operational state like `refresh_locks`, `bulk_refresh_users`, and `lookup_cache` (work_service.rs:112).

3. The dangerous code is not merely long; it is stateful orchestration. `add` spans identity resolution, dedup/adoption, anchor conflict preflight, author creation, DB creation, provenance, and enrichment routing (work_service.rs:466). `ensure_identity_and_enrichment` is explicitly the single decision path for every add outcome (work_service.rs:2712). Splitting this without first naming the invariants in tests is high-risk because the correctness lives in call ordering and re-reads, not in isolated helper bodies.

4. `run_unified_enrichment` is a poor candidate for a naive "enrichment service" extraction. It does identity anchor completion, source injection, enrichment scatter, DB reload, series reconcile, cover write gate, materialization, tag file discovery, and dimension backfill (work_service.rs:3013). A moved file would still need DB, HTTP, data dir, enrichment workflow, resolver behavior, and tag/materialize semantics. That is almost certainly a module move unless the refactor carves out narrower contracts.

5. The convergence extraction actually demonstrates a regression risk: `retry_all_incomplete` calls back through `svc.refresh(...)`, which then runs identity and enrichment again through the main service path (convergence_service.rs:293, work_service.rs:1388). That dependency cycle is easy to miss once logic is spread across files.

6. Lookup/discovery is more separable than add/enrichment. `lookup`, `lookup_filtered`, and `eager_match_by_author` are large but have a clearer provider-search/cache/LLM-filter shape (work_service.rs:1631, :1692, :1832). If I were splitting first, I would start there, because the boundary is more naturally query-oriented and less tied to DB mutation ordering.

**What's Missing**

The plan needs compile-boundary criteria, not just file-boundary criteria. For each proposed extracted module, it should state whether the new code may depend on `WorkServiceImpl`, which fields it may touch, and which invariants must remain single-path. Without that, "service split" is an overclaim.

It also needs regression tests around the paths most likely to break: add matched/adopted/deduped work still calling `ensure_identity_and_enrichment`; refresh still re-chasing only chaseable anchors; convergence terminalizing anchorless pending works; cover writes still only going through the cover gate; retry-all still behaving as a one-pass sweep.

**What I'd Do Differently**

I would not start by carving `WorkServiceImpl` into broad subservices. I would first extract low-risk pure or mostly-pure clusters: lookup/provider search, cover utility functions, and merge preview/conflict helpers. Then I would introduce small context structs only where they reduce bounds, for example a `LookupContext` or `CoverContext`, rather than passing the full `WorkServiceImpl`.

For the identity/enrichment/add path, I would leave it in place until the tests describe the ordering contract. The code comments repeatedly call out "one road" behavior for identity and enrichment; that is a signal that duplication or reordered calls have already caused bugs.

**Does The Plan Hold Up?**

If the design is "split this file into modules while preserving behavior," that can be useful housekeeping. If it is presented as an architectural refactor or service decomposition, I do not think the evidence supports it. The current prior extraction still depends on the whole service, so it proves the opposite: the system has not yet found real ownership boundaries.

**Strongest Objection**

The plan likely mistakes file size for the problem. The real problem is that identity, enrichment, cover materialization, series reconcile, and DB mutation ordering are entangled behind `WorkServiceImpl`; moving those methods into new files without shrinking that dependency surface creates the appearance of design progress while preserving the riskiest coupling.

---

## Where the two independently converge (strong signal)

1. **A pure file-move is cosmetic** — it redistributes the tangle without a boundary, and it will re-tangle (the file already regrew once).
2. **Narrow context structs** (`CreationContext` / `LookupContext` / `CoverContext`) that hand each concern only what it needs — both proposed this unprompted, as the way to get compiler-enforced boundaries without exposing the whole struct.
3. **Leaves first, core last-or-never-yet:** extract discovery/covers/merge first; do NOT touch the add/enrich/identity orchestration until tests pin its call-ordering invariants.

## Where they diverge
- Gemini frames its proposal as "Pattern B done right" (free functions + contexts); Codex is agnostic on pattern and rejects the premise more fundamentally ("file size isn't the problem").
- Gemini's field-visibility claim (its stated reason to reject Pattern A) is disputed and unverified — the convergence divergence point that must be settled by compile probe.
