# Design — metadata-refactor "one road" (S4 + S7)

Grounded live 2026-06-08. This is the implementation contract for the two
enrichment-road slices. Authored by the orchestrator (Opus); Sonnet implements.

## Pathway (3 layers, confirmed live)
```
door → WorkServiceImpl::run_unified_enrichment   (metadata, work_service.rs:2996)   ← materialize home
     → EnrichmentWorkflow::enrich_work            (domain trait, thin delegate)
     → EnrichmentService::enrich_work             (enrichment, lib.rs:1093)          ← merge core
```
- **run_unified_enrichment**: inject Readarr → enrich_work → reload → `maybe_upgrade_cover` (ebook+audio) → `retag_library_items` → return `(status, identity_not_found)`.
- **enrich_work core**: lock → get_work → `dispatch_enrichment` → reconstruct payloads from retry_state → **[LLM identity-validate (Step 8.5) + GR cover-gate LLM]** → CAS merge loop (`merge` → `apply_enrichment_merge`, 3×) → `EnrichmentResult`.

## DD-007: one road · covers always land · zero LLM

### S4 — enrichment crate, `EnrichmentService::enrich_work`
**Oracle:** `test_metadata_refactor_pipeline` (AC-009/017/018/020 now; AC-001 after the Codex harness fix). Full behavioral no-regress.

ADD
- **Candidate reuse** (AC-001, relocates the deleted fork's logic): new optional `transport_cache` port on `EnrichmentServiceImpl`. If `Some(candidate_id)` → `transport_cache.cache_take(user, cid)` → `cached_payloads_match_work` revalidate (mismatch → fall through to network) → on hit, `merge_from_cached(payloads)` and **skip `dispatch_enrichment`** (this is what makes AC-001's `dispatch_count==1` hold).
- **`EnrichmentResult.changed: bool`** — derive from merge mutations (`work_update.is_some() || !external_id_updates.is_empty() || cover_resolution.is_some() || audiobook_cover_resolution.is_some()`). Drives the wrapper's materialize gate.
- **`resolve_status` (REQ-011):** `Enriched` if ≥1 usable field saved; else `Thin` if ≥1 provider responded OK (reused OR fetched) with 0 usable fields; else `Failed`. **VERIFY first** whether the merge's existing `enrichment_status` already satisfies AC-018/020 (empty-success ⇒ Thin); add `resolve_status` only if it does not.

DELETE (REQ-005 — zero LLM in the pipeline)
- **Step 8.5** LLM identity validation (`self.validator.validate` / `all_success_rejected` / the `identity_not_found = true` branch). After deletion, `identity_not_found = merge_output.conflict_detected` only.
- **GR cover-gate LLM escalation** (`cover_gate::CoverGateOutcome::AskLlm` → `llm_ewl::ask_same_book`). KEEP the deterministic Jaccard gate (the `evaluate_gr_cover_gate` part with no LLM call).

PRESERVE (load-bearing, NOT in the pipeline test — do not drop)
- per-work lock; the **CAS retry loop** (3 attempts, Superseded → re-read + retry); **Readarr `source_data_store` injection**; the deterministic merge (S2).

### S7 — metadata crate, `run_unified_enrichment` + doors + fork
**Oracle:** `test_metadata_refactor_materialize` (the service, S3a-green) + integration. The wrapper→materialize wiring is NOT locked-tested → orchestrator reviews the diff.

- **Replace** `run_unified_enrichment` steps 4–5 (`maybe_upgrade_cover` ×2 + the inline retag) with ONE **change-gated** `MaterializeService::materialize(MaterializeRequest)` — only when `enrich_result.changed` (REQ-012). Build the request from `post_enrich_work` + `enrich_result`:
  - `changed` / `tag_fields_changed` ← from `enrich_result.changed`
  - `ebook_cover` / `audiobook_cover` (`CoverSlotState`): `chosen_new_url` ← `enrich_result.{cover_resolution,audiobook_cover_resolution}` url; `current_url`/`current_path` ← work record; `user_locked` ← `work.cover_trust == User`
  - `file_paths` ← `db.list_taggable_items_by_work(user,work).map(|i| i.path)`
  - `tags` ← `MaterializeTags` from `post_enrich_work`; `covers_dir` ← `data_dir/covers/user_id`
  - DROP `maybe_upgrade_cover` / dimension+upgrade machinery (REQ-006 priority-only).
- **Thread `candidate_id`**: `door → run_unified_enrichment(.., candidate_id) → enrich_work(.., candidate_id)`. Widen `run_unified_enrichment` + the `EnrichmentWorkflow::enrich_work` delegate + the **6 canonical doors** (WCC spec §1 L42: add, manual_import, list_service, readarr, series_monitor, author_monitor). A seed without a candidate passes `None`.
- **DELETE** `try_reuse_cached_payloads` (work_service.rs:2546) — logic moved into enrich_work (S4); doors reach reuse through `enrich_work` now.

## Wiring (composition root — orchestrator, at integration)
- `transport_cache` port → `EnrichmentServiceImpl` (the `TransportCache` already built at main.rs:489).
- `MaterializeService` (LiveMaterializeService + http + covers_dir) → `WorkServiceImpl`.
- **AC-001 harness:** Codex (OpenAI) revises `test_metadata_refactor_pipeline`'s `service()`/setup to inject a seeded `transport_cache` so `author_page`'s candidate resolves — assertions unchanged.

## Decisions (PO-approved)
- **LLM out of the pipeline** (REQ-005 / P-C / NR-identity). User-visible: GR covers governed by priority policy not LLM identity-check; held/unverified works enrich + get covers (REQ-015).
- **Materialize lives in the wrapper**, not enrich_work core — enrich_work stays the locked 6-arg `db+queue+merge` shape; one materialize home (P-D).
- **`identity_not_found` = merge `conflict_detected`** (no LLM).

## Blast radius / risks
- Prior-feature tests asserting the LLM identity rejection or the GR cover-gate LLM will break under REQ-005. The agent REPORTS which break; reconciliation (update/remove) is handled at integration (Codex re-authors, or orchestrator patches + Gemini reviews) — this is the intentional behavior change, not a regression to fight.
- The wrapper→materialize wiring has no locked test → orchestrator diff-review is the gate.

## Forbidden (post-impl grep must be 0)
`merge_impl_llm`, `LlmMergeResponse`, `LlmFieldSelection` (S2), `try_reuse_cached_payloads`, the pipeline calling `validator.validate` / `ask_same_book`, `INSERT OR REPLACE`.

---

## As-built reconciliation (2026-06-09, Opus) — the add-door cutover was incomplete; completed via "Option A"

A deep audit during the post-S7 unification overturned this doc's assumption that threading `candidate_id` into the doors (S7, above) put every door on the one road. **It did not for the primary interactive door.**

**Finding.** The `add` door (Add box + author page → `livrarr-handlers/work.rs::add`) was NOT on the one road after S7. It set `skip_sync_enrichment: true` → `finish_created_work`'s skip-gate returned `Unenriched` (pipeline never ran; the threaded `candidate_id` went unused on the sync path), then the handler spawned its OWN route: `enrich_work(.., None)` **called directly** (bypassing `run_unified` → bypassing materialize) + an ad-hoc `download_cover_from_url`. Net on the primary door: candidate reuse never fired (None), tags never written, cover saved by a separate route — a direct P-A / REQ-001 / AC-002 violation, structurally the §1 originating bug. Threading `candidate_id` into the `WorkCandidate` (S7) only ever fed a *second* off-road reuse path — an inline block in `finish_created_work` (NOT the `try_reuse_cached_payloads` this doc named; a distinct S7 artifact) that also early-returned before materialize. So three roads coexisted on this door (inline reuse, handler enrich-direct, handler ad-hoc cover). The 508-green suite missed it: behavioral tests cover the *road* (`run_unified`/materialize), never the *door→road wiring* (axum handler).

**Completion — "Option A" (PO-approved 2026-06-09):** make the `add` door match the other doors (sync through the one road).
- `livrarr-server/main.rs` — created the shared `Arc<TransportCache>` ABOVE `EnrichmentServiceImpl::new` and wired it into the service via `.with_transport_cache` AND into the resolver. (This doc's Wiring §, line ~45, *assumed* this; S7 never did it, so `enrich_work` reuse was dead in production regardless of the inline path.)
- `livrarr-metadata/work_service.rs` — deleted the inline `finish_created_work` candidate-reuse block (~145 lines) + the now-dead `anchors_match`/`anchor_compatible` helpers. Reuse lives only in `enrich_work` step 2.5.
- `livrarr-handlers/work.rs` — `add` now `skip_sync_enrichment: false` (→ `run_unified` → reuse + materialize cover **and** tags) with its Phase-2 ad-hoc enrich/cover spawn deleted; `refresh_all` restructured to drop a redundant ad-hoc `download_cover_from_url` + a dead retag (`refresh()` already materializes).
- `tests/behavioral/test_wcc_add.rs` (Codex, reviewed) — `reqs_014_015` rewritten to drive `add(skip=false, candidate_id)` through a REAL `EnrichmentServiceImpl` (seeded `TransportCache` wired) — asserts cache-only fields surface, `dispatch_count==0`, `http_spy==0`, `Enriched`. Covers the `add → run_unified → enrich_work` threading that AC-001 (calls `enrich_work` directly) never exercised.

**Verified as-built door map (every live add-door now funnels through `run_unified`):**

| Door | On the one road | Mechanism |
|---|---|---|
| Add box / author page | ✅ (fixed by Option A) | `add(skip=false, candidate_id)` → run_unified → reuse + materialize |
| Manual import | ✅ | `add(skip=false, candidate_id)` |
| List import | ✅ | `add(skip=false, Pending)` → background converge |
| Author monitor | ✅ | `add(skip=false, candidate_id=None)` → network enrich (no user pick = no cache) |
| Readarr import | ✅ | `add` + `source_provider_data` → run_unified |
| Single refresh | ✅ | `refresh` → run_unified (Manual) |
| Bulk refresh (`refresh_all`) | ✅ (cleaned) | loops `refresh()`; redundant ad-hoc cover+retag removed |
| Background retry job | ❌ off-road (ad-hoc cover) | being deleted in S6 |
| Series monitor | n/a | no live add-workflow found (only `SeriesMonitorWorkerParams`); series-entity creation is a deferred non-requirement (spec §4) |

**Delivered:** P-A / REQ-001 / AC-002 now satisfied at the add door (verified by `test_wcc_add_reqs_014_015` + the door map). Suite 508/0/300, clippy 0, fmt clean.

**Deferred / intentional debt (carry into Phase-B):**
- S5 pacing (daily budget + fg/bg lanes) + S6 status-surface (derive in-progress; `retry_all_failed`; DELETE the auto-retry job — which closes the last off-road door above). Both need real API design first.
- `finish_created_work` runs `run_unified` in `Background` mode for all doors; interactive add should be `Foreground` (PO #127) — cosmetic until S5's lanes exist.
- Dead trait bounds left in place (no warning): `HasEnrichmentWorkflow` on `add`, `HasTagService` on `refresh_all` — trivial cleanup.
- Stale line refs in this doc's body (pre-Option-A): `run_unified_enrichment` now `work_service.rs:2736`; `enrich_work` now `lib.rs:1215`.

---

## S6 as-built (2026-06-09, Opus) — background retry job deleted; REQ-001 fully closed; user-triggered retry shipped

S6 closed the last off-road door in the map above (the "Background retry job ❌" row) and replaced the recurring background enrichment retry with an explicit user action — realizing §7 ("No background retry job… the primary fix for the GB quota churn") and REQ-011.

**Deletions (compiler-guided; 0 callers confirmed before removal):**
- The `enrichment_retry` background job — the inline spawn block in `livrarr-server/jobs/mod.rs` + the whole `jobs/enrichment.rs` file (the 4-source retry tick).
- `WorkService::download_cover_from_url` — the off-road cover route (domain trait + `work_service.rs` impl + the `StubWorkService` stub). Its only caller was the deleted job → **REQ-001 ("no path writes enrichable metadata, covers, or tags by any other route") is now fully closed.**
- The `enrichment_notify` wakeup mechanism — AppState field + `main.rs` init + the `HasEnrichmentNotify` trait (context.rs) + impl. 0 users after the job died + the identity-unit's ping removal.

**Replacement — `WorkService::retry_all_incomplete` (REQ-011):** a single-pass, no-loop sweep of every incomplete work for the user (enrichment `Failed`/`Unenriched` OR identity `Pending`). A Pending work re-resolves identity first — straight to `self.resolver.resolve()` (NOT `resolve_identity`, which gates on a hard anchor so a title+author seed won't fan out) → on `Resolved`: `confirm_anchor` (when an OL key is present) + `set_identity_status(Confirmed)` (for ANY Resolved — the relocated Source-4 convergence, now WITH the badge-flip the old job omitted, see R-002 below) → then `refresh()` (the one road). Returns `RetrySummary{total, recovered, still_incomplete}`.
- **Handler + route:** `handlers/work.rs::retry_all_incomplete` mirrors `refresh_all` (shared `try_start_bulk_refresh` guard → spawn one-shot → `BulkEnrichmentComplete` notify → 202); `POST /api/v1/work/retry-incomplete` (router.rs, singular `/work` to match `/work/refresh`).
- **Frontend:** `retryAllIncomplete` (api/index.ts) + a "Retry Incomplete" button on the Works page next to "Refresh All".

**Identity-unit (S6 prerequisite):** bulk list-import (`list_service.rs::resolve_candidate_from_row`) now resolves identity synchronously at add-time via the shared `WorkService::resolve_identity` (mirroring the interactive Add door), replacing hard-coded `Pending`. With no door producing intentionally-Pending works, the deleted job's Source-4 (identity resolver) had nothing left to converge → the whole job was safe to delete. Dropped the vestigial `enrichment_notify` ping in `list_import.rs`.

**Cross-family code review (PO-override 2026-06-09):** Codex PASS ×2. Gemini r1 FAIL → **R-002** (the Pending→Confirmed badge-flip was gated on `ol_key`, inconsistent with `resolve_identity`) FIXED — `set_identity_status` now fires for any `Resolved`; **R-003** (stale wiki refs to the deleted symbols) FIXED; **R-001** (the sweep excludes provider-level `WillRetry` / the deleted job's Source-1) REJECTED on spec grounds — REQ-011/AC-019 scope recovery to "failed works"; §7 deliberately deletes the provider-retry mechanism, so re-querying `provider_retry_state` would resurrect the GB-quota churn the refactor exists to kill; Codex never raised it. Gemini r2 timed out (tooling null) → PO-override recorded. Gate green: 564/0/299, clippy clean.

**Updated door map:** the "Background retry job ❌ off-road" row is now **deleted** — every live cover/tag write path funnels through `run_unified` → materialize. REQ-001 fully closed.

**Deferred / intentional debt:**
- **S5 pacing** (daily budget + fg/bg lanes) — still deferred; needs real API design first.
- **`enrichment_retry_count`** (a `works` column the deleted job used) is now likely dead; removing it needs a migration → tracked as a separate follow-up (GH issue).
- Dead trait bounds (`HasEnrichmentWorkflow` on `add`, `HasTagService` on `refresh_all`) — still trivial cleanup.
