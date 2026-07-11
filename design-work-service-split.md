# Design — Splitting the `work_service.rs` God Object (M-005)

**Date:** 2026-07-10
**Author:** Claude (main session). Grounded against current `main`. `✓` = orchestrator read the cited lines this session; sizing/edge counts marked *(map)* come from a Serena-driven coupling pass and carry `path:line`.
**Status:** Draft for cross-model review.

---

## 1. Goal & non-goals

**Goal.** Split `crates/livrarr-metadata/src/work_service.rs` (**3,742 lines** ✓) into cohesive per-concern modules, so (a) an AI writing a new feature can see one concern at a time instead of re-implementing logic it can't see (the documented duplication risk), and (b) new metadata concerns get an obvious home instead of accreting here — the file *grew* 3,684 → 3,742 since the 2026-06-28 audit.

**Non-goals (explicit — reviewers hold me to these):**
- **No behavior change.** Every code path stays byte-for-byte equivalent.
- **No decoupling / redesign.** This is a *move*, not a re-plumb. (Justified in §3.)
- **No public-API change.** The `WorkService` trait and `WorkServiceImpl` constructors keep identical signatures; no caller (handlers, server wiring) changes.
- **No dead-field removal in the move itself.** Removing the 3 dead fields changes a constructor signature — carved out as an optional, separate follow-up (§8).

## 2. The invariant that must hold (the contract)

Preserve, unchanged:
- The **`WorkService` trait** — 20 methods ✓ (`livrarr-domain/src/services/work.rs:349-…`): `add, resolve_identity, get, get_detail, list, list_paginated, update, delete, refresh, retry_all_incomplete, upload_cover, download_cover, lookup, lookup_filtered, eager_match_by_author, search_works, try_start_bulk_refresh, converge_work, preview_merge_works, merge_works`.
- The **`WorkServiceImpl<D,E,H,L,M,T>` constructors** ✓ (`work_service.rs:66-176`): `new`, `new_with_llm`, `new_with_all`, `with_resolver`, `without_enrichment`.
- All associated request/response types (`AddWorkRequest`, `LookupResult`, `MergePreview`, …) — they live in the domain trait file already, not here.

If a caller outside `livrarr-metadata` would need to change, the design is wrong.

## 3. Verdict: move, not decouple — and the evidence

Three independent facts settle this:

1. **No mutable state is contended across concerns.** ✓ The struct's shared fields are all either `Arc<…>` (immutable share) or internally-synchronized locks: `resolver: Option<Arc<…>>`, `refresh_locks: KeyedMutex<…>`, `bulk_refresh_users: Arc<Mutex<…>>`, `lookup_cache: Arc<Mutex<…>>` (`work_service.rs:39-63` ✓). Moving a method to another file changes nothing about state access or concurrency — it still reaches the same `Arc`/`Mutex` through `self`/`svc`. The one field I *guessed* was cross-concern (`lookup_cache`) is **Discovery-only** *(map: only `lookup_filtered:1748,1804`)* — disconfirmed.
2. **The concerns are already in separate `impl` blocks.** *(map)* Discovery, Add-helpers, Enrichment-orchestration, and preflight are each their own inherent `impl` block today; only the 20 trait methods share one coherence-locked block.
3. **The codebase already did one of these cleanly.** ✓ `convergence_service.rs` holds `converge_work`/`retry_all_incomplete` as free functions taking `svc: &WorkServiceImpl<…>`; the trait methods are 1-line shims (`work_service.rs:2050` ✓). This split just applies that proven pattern to the rest.

The only method that spans concerns is `run_unified_enrichment` — and it is a legitimate **coordinator** (called by Add ×2, Refresh ×1, and external convergence ×1 *(map)*), not tangled logic. It stays a coordinator; callers keep calling it.

## 4. Current structure (grounded)

| Concern | Public methods | Also owns (helpers/types/tests) | ~lines *(map)* | Stateful field |
|---|---|---|---|---|
| **CRUD** | get, get_detail, list, list_paginated, update, delete | `delete_cover_files` | ~337 | — (db only) |
| **Discovery** | lookup, lookup_filtered, eager_match_by_author, search_works, llm_filter_search, lookup_{goodreads,openlibrary,google_books,hardcover} | `CachedLookup`, take_lookup, interleave_by, lookup_result_from_captured, lookup_results_from_resolution, dedupe_lookup_results, cover_source_rank, finalize_eager_pick, best_same_work_cover, **the 14-test `discovery_tests` mod** | ~1,295 | `lookup_cache`, `llm` |
| **Creation** | add (461 lines!), resolve_identity | try_dedup_by_normalized, find_or_create_author, ensure_identity_and_enrichment, handle_race_loser, finish_created_work, preflight_and_merge_anchors, conflict_source_for, write_addtime_provenance | ~990 | — |
| **Enrichment (coordinator)** | — (`run_unified_enrichment`, `pub(crate)`) | — | ~287 | — |
| **Refresh/convergence** | refresh, retry_all_incomplete*, converge_work*, try_start_bulk_refresh | chaseable_anchor_types, `DEAD_END_THRESHOLD` | ~205 in-file (+~278 already in `convergence_service.rs`) | `refresh_locks`, `bulk_refresh_users` |
| **Covers (narrow)** | upload_cover, download_cover | is_supported_image | ~108 | — |
| **MergeWorks** | preview_merge_works, merge_works | merge_field_conflicts | ~164 | — |

`*` already a thin shim into `convergence_service.rs`. Dead fields (no readers ✓): `http_client`, `merge_engine`, `tag_service`. Orphan: `unproxy_cover_url` (no in-file caller; likely a duplicate of `livrarr_domain::unproxy_cover_url` — see §8).

**Corrections the coupling pass forced on my first cut** (surfacing so reviewers see the reasoning): Covers is *narrow* — the auto-cover-write path already lives in `cover_write_gate.rs` and the phase-1 fetch in `cover.rs`/`cover_resolution.rs`; do **not** re-merge them here. There is **no "shared helpers" module** — every free fn has exactly one owning concern and travels with it. `delete_cover_files` belongs to CRUD (its only caller is `delete`), not Covers.

## 5. Mechanism (Rust specifics — the part review should scrutinize)

The constraint: Rust coherence forbids a second `impl WorkService for WorkServiceImpl<…>` block (E0119), so the 20 trait methods cannot be physically relocated as a block. Two patterns solve this:

- **Pattern A — module directory + split inherent impls (RECOMMENDED).** Turn `work_service.rs` into a directory `work_service/` (`mod.rs` + one file per concern). `mod.rs` keeps the struct, the constructors, and the *single* `impl WorkService` block — each of its 20 methods reduced to a thin shim (`{ self.lookup_impl(req).await }`), leaving trivial ones (get/delete) inline if smaller than their shim. Each concern file holds an inherent `impl WorkServiceImpl<…>` block with the real bodies as `pub(crate)`/`pub(super)` methods. Because concern files are **descendant modules** of the module defining the struct, they access private fields via `self.field` with **no struct-field visibility change** — only the moved methods take a `pub(super)` marker so the shims (and sibling concerns like `add → run_unified_enrichment`) can call them.
- **Pattern B — sibling free-fn modules (the existing precedent).** Exactly what `convergence_service.rs` does: `discovery_service.rs`, etc., with `pub(crate) async fn foo(svc: &WorkServiceImpl<…>, …)`, trait methods as shims. Matches the one prior extraction verbatim, but requires widening ~7 private fields (`http, enrichment, llm, data_dir, refresh_locks, bulk_refresh_users, lookup_cache`) to `pub(crate)` so the free fns can reach them.

**Recommendation: Pattern A.** Same shim churn as B, but it widens *method* visibility (`pub(super)`, behavior) instead of *field* visibility (`pub(crate)`, raw state), and it's the idiomatic "split one big impl across files." `convergence_service.rs` can stay a sibling as-is (its import `use crate::work_service::{…}` still resolves against the directory module) or later fold into `work_service/convergence.rs`. **This A-vs-B call is the main thing I want the review to confirm or overturn.**

The already-inherent method groups (Discovery providers, Add helpers, `run_unified_enrichment`, `preflight_and_merge_anchors`) move as **whole `impl` blocks** with zero shim — only the 20 trait-block methods need the shim treatment.

## 6. Target layout (Pattern A)

```
work_service/
  mod.rs         struct + 12 fields + constructors + the one `impl WorkService` (20 thin shims)
  crud.rs        get/get_detail/list/list_paginated/update/delete + delete_cover_files
  discovery.rs   lookup*/eager_match/search_works/llm_filter_search/lookup_<provider> + CachedLookup + Discovery free fns + discovery_tests
  creation.rs    add/resolve_identity/*_helpers + conflict_source_for/write_addtime_provenance
  enrichment.rs  run_unified_enrichment  (pub(crate); the coordinator)
  refresh.rs     refresh/try_start_bulk_refresh/converge_work+retry shims + chaseable_anchor_types + DEAD_END_THRESHOLD
  covers.rs      upload_cover/download_cover/is_supported_image
  merge_works.rs preview_merge_works/merge_works + merge_field_conflicts
```

## 7. Slice sequence (leaf-first; each slice independently green + committed)

Ordering is by **risk size**, not dependency (all methods stay on one struct in one crate, so `self.x()` resolves regardless of file). Smallest/most-isolated first to prove the mechanics before touching `add`.

| # | Slice | ~lines | Why here |
|---|---|---|---|
| S0 | `work_service.rs` → `work_service/mod.rs`; add empty `mod` decls | ~0 moved | pure structural; establishes the skeleton |
| S1 | **merge_works.rs** | ~164 | smallest real concern, zero cross-concern calls — the mechanics dry-run |
| S2 | covers.rs | ~108 | narrow, isolated |
| S3 | crud.rs | ~337 | `get()` stays reachable via `self`; 4 of 6 methods have zero internal callers |
| S4 | discovery.rs | ~1,295 | large but fully self-contained (incl. its own tests) |
| S5 | enrichment.rs | ~287 | the coordinator; already proven cross-file-callable |
| S6 | creation.rs | ~990 | `add` (461 lines) is the riskiest single move — do it once the pattern is boring |
| S7 | refresh.rs | ~205 | converge/retry already shimmed; finishes the file |

**Per-slice gate (every slice):** `cargo fmt --all -- --check` clean · `cargo clippy --workspace --all-targets` zero warnings · `cargo test` green · then commit. A slice that can't go green is reverted, not patched forward.

## 8. Follow-ups carved OUT of the move (do not bundle)

- **Dead-field removal** (`http_client`, `merge_engine`, `tag_service`) — changes `new_with_all`'s signature → touches `livrarr-server` wiring. Separate opt-in slice; note the struct comments call `merge_engine`/`tag_service` "reserved for future slices," so this is a real decision, not obvious cleanup.
- **`unproxy_cover_url`** — likely a duplicate of `livrarr_domain::unproxy_cover_url` (`livrarr-domain/src/lib.rs:1041` *(map)*), which handlers already call directly. Verify and **delete** rather than relocate.
- **Lookup → provider gateways (audit M-004/B2)** — the audit wanted `lookup_*` moved into the provider crates, not just into a `discovery.rs`. That's a bigger, cross-crate step; this split makes it *possible* later but does not attempt it.

## 9. The convention that stops regrowth (the real lever)

Splitting once is a temporary tidy — the file already regrew after the convergence extraction. Pair the split with a **CLAUDE.md rule**: *new metadata concerns get a new `work_service/<concern>.rs` submodule; nothing new is added to `mod.rs` beyond a thin trait shim.* Without this, it re-accretes.

## 10. Process (right-sized — NOT full kk-build)

kk-build's heavy stages (Spec/Architecture/Design + gate reviews) exist to de-risk *new behavior*; this has none, so they'd be ceremony over an empty spec. What actually de-risks a refactor: **this move-map + sliced commits + tests-green-before-and-after + one cross-family diff read per slice** asking only "did any behavior or public signature change, was anything dropped?" That's the whole process.

## 11. Risks & open questions (for the review)

1. **Pattern A vs B** (§5) — the one genuine design choice. A avoids field-visibility widening; B matches the single existing precedent exactly. Reviewer call.
2. **`add` (461 lines)** is the highest-risk single move (S6). It calls many `self.*` helpers and free fns; all stay in-crate, so it's mechanical — but it's the one to review a diff on most carefully.
3. **Test coverage is the entire safety net — and it's not in CI.** "Tests green before/after" only proves behavior-preservation to the extent the behavioral suite exercises `work_service.rs`. That suite is **local-only, not run in CI** (Docker build only). The in-file `discovery_tests` (14 tests) travel with `discovery.rs` and cover the Discovery helpers; coverage of `add`/`refresh`/`merge_works` behavior should be **confirmed before** trusting a green run as proof. **Recommend: verify coverage of this file first; if thin, add characterization tests before S6.**
4. **`converge_work`/`retry_all_incomplete`** already delegate to `convergence_service.rs`; decide whether that sibling stays or folds into `work_service/convergence.rs` (cosmetic).
5. **Visibility surface.** Pattern A adds `pub(super)` to moved methods; confirm none accidentally becomes `pub(crate)` wider than needed.

## 12. What this does and doesn't buy (honest framing)

Buys: less duplication risk, a home for new concerns, safer edits, and (with §9) no regrowth. Does **not** buy: faster compiles (still one crate), decoupling (there was none to do), or any user-visible change. It only speeds up features that actually touch this area.
