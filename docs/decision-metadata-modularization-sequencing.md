# Decision — Metadata Modularization: Sequencing & Migration (v3, FINAL)

**Status:** ✅ **DECISION LOCKED.** Adopt **C′ — structured extraction.** PO sign-off 2026-06-02, after two cross-family confer rounds converged.
**Author:** PO design session + read-only code trace (Serena + code-index) + two confer rounds (Gemini + Codex).
**Companions:** target architecture → `design-metadata-modularization.md` + `diagrams/metadata-modularization.html`; current pipeline → `diagrams/metadata-lifecycle.html`; in-flight feature → `spec-work-creation-consistency.md` (v5) + IRs.

**Decision history:**
- v1 — preliminary C (~78%).
- **Round-1 confer** — corrected the baseline (§2.1: `work_service` owns *inline* discovery + reaches the resolver's concrete cache) and **split A (Gemini) vs C-refined (Codex)**.
- v2 — reconciled the split into **C′**.
- **Round-2 confer** — both families, verifying against the call graph, **endorsed C′; the canary passed** (providers extracts cleanly with `lookup_filtered` as a consumer; no cycle). Codex added one hard refinement (§6.2). **Locked.**

---

## 1. The decision

**Adopt C′ — structured extraction:**

> Extract the genuinely-**stable** layers and define their **ports** first (the clean ~73%: provider clients, the enrichment queue, materialize modules, the `EstablishedIdentity` contract). Then cut each **feature-touched** coupling (discovery, cover, status) *as part of building the feature that refactors it* (discovery→GR ladder, cover→cover-decouple, status→two-state-machines) — landing the result in the **target crate, never back in the monolith.**

This is **reorganization, not a rebuild** — the existing code is good, just un-walled. Confidence ~82%, and the first move (§8) is a cheap, reversible experiment that *proves or kills* the plan.

Rejected: **A** (build new work in-place, extract later) — pours more into the monster files. **B** (abandon + rebuild) — destroys landed, tested code.

---

## 2. Baseline — what the code actually is (verified)

`livrarr-metadata` = ~20,600 LOC / 37 modules. Identity-federation foundation (`f78f3bc`) **already on `main`**; WCC branch +2 commits + ~30 files uncommitted; the new metadata-pipeline work is unwritten.

### 2.1 Two provider-access paths (the round-1 correction)
- **Enrichment path** — `provider_queue` → `provider_client`: trait-based, injected. **Clean.**
- **Discovery path** — `WorkService::lookup_filtered` (work_service.rs:1378-1564): **inline** parallel fetch via `self.lookup_google_books / lookup_openlibrary / lookup_hardcover / lookup_goodreads` — concrete methods on `WorkServiceImpl`, own timeouts/interleave/LLM-filter/15-min cache. **Not** trait-routed.
- `transport_cache()` (2640) = `self.resolver…cache` — reaches the **concrete** `LiveEnglishIdentityResolver` cache.

**So the thesis is precise:** "well-factored, un-walled" **holds for the add→enrich→materialize spine** (orchestration over injected `DB/E/Q/ME/V/L/T`) and **fails for the discovery front-half + the resolver-composition** (both inline in `work_service`).

### 2.2 Module → crate map
| Future crate | Modules | LOC ≈ |
|---|---|---|
| **livrarr-providers** | provider_client, provider_queue, 6 clients, llm_scraper/caller, transport_cache, language, parsers, title_cleanup, normalize | ~9,000 |
| **livrarr-identity** | english_identity_resolver, async_resolver, bulk_resolver, llm_validator | ~1,560 |
| **livrarr-materialize** | cover*, preadd_cover_service (+ tagwrite) | ~1,800 |
| **livrarr-enrichment** | merge engine + status (lib.rs), enrichment_workflow_service | part of lib.rs |
| (distribute) | list/author/series/rss workflows | ~3,500 |

### 2.3 The seams (> 4)
1. `provider_queue ← EnrichmentMode/EnrichmentContext`. 2. `enrich_work → EnrichmentStatus::Conflict` writeback (one-way violation). 3. cover-gate in `enrich_work`. 4. `merge_impl` status. 5. **inline discovery in `work_service`** (`lookup_filtered`). 6. **`work_service` reaches the resolver's concrete `transport_cache`**. 7. phase-1 cover + cover-upgrade calls inline. 8. tag-sync + enrichment's lock/source-data stores.

### 2.4 Confirmed
- Identity resolution is upstream of `add` (receives a resolved `IdentityState`) — but **not a clean crate boundary** (work_service composes the resolver + cache).
- `add` **enriches `Pending` works today** (work_service.rs:816) — contradicts the target gate. Behavior delta.
- The crate is genuinely generic over injected services (the enrich spine).

---

## 3. Options considered
- **A** finish-in-place then extract — Gemini's round-1 pick; rejected (pours more into the monsters; its only edge — ship sooner — is deprioritized).
- **C** relocate-everything then build — Codex's round-1 pick, but pure form walls interfaces the features then reshape.
- **B** abandon + rebuild — rejected by all (the foundation is landed + well-factored enough).
- **C′** structured extraction — **adopted** (the reconciliation; both families endorsed it in round 2).

---

## 4. Reviewer reconciliation (agree / disagree)

### Gemini
- ✅ Thesis overstated for `work_service` (inline discovery + concrete cover/cache). Verified; corrected.
- ✅ The new features are the forcing function (GR ladder *is* the discovery refactor; cover-decouple *is* the cover refactor).
- ✅ Walling coupled interfaces before the features reshape them risks throwaway abstractions.
- ❌ Disagree with "build all new features *in-place* first" — perpetuates the exact problem (more in the monsters). Build the feature *in the target crate*, not the monolith.
- ❌ Disagree with the "undifferentiated swamp" framing — ~73% is genuinely clean; the swamp is localized to `work_service`'s discovery + resolver-composition.

### Codex
- ✅ The real failure mode is untangling `work_service`, **not** `lib.rs` type-redistribution. (Corrected v1's named top risk.)
- ✅ "Extract ports/interfaces, don't just move files."
- ✅ Thesis "mostly holds"; identity-upstream-not-clean-boundary; > 4 seams.
- 🔶 Refine — "ports first, then build" is right for *stable* ports, wrong for *feature-touched* ports (which the features reshape). So: wall stable ports first; co-define feature-touched ports *with* their features.

### Round-2 convergence
Both endorsed **C′** after verifying the canary in code (Gemini: *"surgically precise, not mushy"*; Codex: *"I endorse C′"*). The disagreement was about the baseline — once corrected, the split resolved.

---

## 5. Why C′
1. The clean periphery (~73%) doesn't depend on the new features — extract first, cheap, low-risk.
2. The three messy couplings (discovery/cover/status) are *exactly* what three planned features refactor — so cut-the-seam and build-the-feature are one act, done in the target crate (avoids both "throwaway ports" and "more in the monolith").
3. The foundation is landed + well-factored — extraction + targeted untangle, not rebuild.
4. Shipping WCC is deprioritized — A's only edge is gone.
5. **The first move is falsifiable** (§8) — C′ proves or kills itself cheaply before any large commitment.

---

## 6. Design for C′

### 6.1 Three tracks (not strict phases)
- **Track 1 — Extract the stable foundation.** `livrarr-providers`, `livrarr-materialize`, and the `EstablishedIdentity` contract into `domain`. Behavior-preserving, tests green. Independent of the new features.
- **Track 2 — Cut feature-touched seams via their features, into the target crates.** Discovery (re-home `lookup_filtered` as discovery *policy* consuming `livrarr-providers`; the **GR ladder** lands in providers/identity). Cover (concrete `crate::cover::*` calls behind a materialize port as **cover-decouple** is built). Status (the **two-state-machine** split cuts the Conflict-writeback + status seams).
- **Track 0 — Stabilize first.** Commit the in-flight WCC green work to a clean baseline before Track-1 churn.

### 6.2 Codex's refinement — BAKED IN (the thing that makes C′ real, not vague)
**`livrarr-providers` must own the provider-facing *contracts* + a *search/discovery* surface — not just `ProviderClient::fetch`.** The enrichment shape (`fetch(&Work,&EnrichmentContext)->ProviderOutcome<NormalizedWorkDetail>`) is **not** the discovery shape, so `lookup_filtered` can't consume a fetch-only crate. Therefore the crate must expose **both** surfaces, and the **stable contract types (`NormalizedWorkDetail`, `ProviderOutcome`) — currently in `lib.rs` — move out first.** That relocation is *also* the falsification test (§8).

### 6.3 Folding in WCC + the IR
WCC's landed identity work → `livrarr-identity`; WCC's in-flight → committed in Track 0; today's metadata-pipeline IR (spec v5 / IR v7-v2) → the Track-2 feature specs (GR ladder / status / cover-decouple / Bug #2 / de-facto / Q4 gating / user-ID-edit).

---

## 7. Risks & open questions
- **Top risk:** untangling `work_service`'s inline discovery + concrete resolver-cache reach without changing behavior. *(Not `lib.rs` type-redistribution.)*
- `transport_cache`'s concrete-resolver reach (`Option<Arc<LiveEnglishIdentityResolver>>`) is a leaky boundary — replace with a small resolver/cache capability during extraction (both reviewers; not a cycle).
- Behavior changes ride along (Q4 Pending-holds, Conflict-writeback move, cover-decouple) — explicit tests required.
- Bank for Track 2: cache key = `(provider, provider-key)` not `work_id`; cache TTL/eviction; Provisional→Confirmed clean-overwrite; `tag_generation` vs `merge_generation`.
- kk-build framing: WCC closes / morphs into `metadata-modularization`.

---

## 8. Decision & the first move (falsifiable)

**Locked: C′.** The first move is the experiment that confirms it — cheap, reversible, no large commitment until it passes.

**First move:**
1. **Stabilize** — commit the in-flight WCC green work to a clean baseline.
2. **Extract `livrarr-providers`** — move out of `lib.rs`/`work_service`: the stable contract types (`NormalizedWorkDetail`, `ProviderOutcome`), a **search/discovery surface** + the detail/fetch surface, the parsing modules (goodreads/google_books/hardcover/openlibrary), `transport_cache`, and the queue/client. Leave `lookup_filtered` and `enrich_work` as **consumers**.

**Go / no-go (the canary):**
- ✅ **GO** — `livrarr-metadata → livrarr-providers` compiles with **no back-edge**. C′ confirmed; proceed with Tracks 1–2.
- ❌ **NO-GO** — extraction forces a **back-edge** (`providers → metadata` for `LookupResult`/`NormalizedWorkDetail`/`ProviderOutcome`/`EnrichmentContext`/queue traits). Stop; isolate those contract types into `domain`/`providers` first, or fall back to **A**. *(Both families named this exact condition.)*

**Next:** kick off the `metadata-modularization` feature (`/kk-session-start metadata-modularization`) with this brief as the plan and the `livrarr-providers` extraction as step 1. Final design → HTML later.
