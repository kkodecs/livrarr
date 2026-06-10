# Metadata Lifecycle Audit — Launch Brief

**Status:** specced & approved (PO, 2026-06-07). Run this in a **fresh** session.
**Nature:** read-only diagnosis. **No code changes, no commits, no push.** Output is a report + fix plan only.

---

## 0. How to use this brief (fresh session, cold start)

1. You are in `/mnt/opt/livrarr`, branch `wcc-stage5-green` with an **uncommitted alpha-6 fix batch** in the working tree. Do not commit/push anything. Do not disturb the working tree.
2. **Tooling:** agents use **code-index / Zoekt** (`mcp__code-index__search_code`) to locate code — **NOT Serena** (OOM on subagents). Always `Read` the actual files to confirm (the index may be stale vs. the uncommitted tree). The main loop may use Serena.
3. **DB:** read-only queries against `testdata/livrarr.db` (the live dev DB; has the GB key + real foreign test works). `sqlite3` is fine.
4. Launch the two-phase **workflow** in §8. No adversarial-verify phase (PO's call).
5. The audit measures **actual vs. two targets**: the 8 behavioral promises (§3) and the 9 structural dimensions (§4). It must independently **re-derive every bug in the falsification set (§5)** — if it doesn't, it isn't thorough enough.

---

## 1. Objective

An exhaustive, two-part audit of the metadata subsystem:
- **(A) Behavioral correctness** vs. the 8 promises (the ideal state).
- **(B) Structural / architectural quality** vs. the 9 dimensions.

Then **correlate** them (which structural weakness causes which behavioral gap) and produce a **root-cause fix plan aimed at the ideal** — diagnosis only, not implementation.

## 2. Scope

Full lifecycle, **entry → file-tag**, **exhaustive** (read *every* function in the metadata path, not just changed files). Crates in scope:
`livrarr-external-data` (provider clients) · `livrarr-identity` · `livrarr-enrichment` (merge engine, queue) · `livrarr-metadata` (orchestration: work_service, list_service, author_monitor, rss_sync) · `livrarr-db` (apply + queries) · `livrarr-domain` (entities/traits/enums) · `livrarr-handlers` (metadata + work routes) · `livrarr-server` (composition root, provider/queue wiring, jobs) · `livrarr-tagwrite` (tag stage) · `livrarr-matching`. Frontend only where it drives metadata behavior (refresh button, cover picker).

Series metadata is **in scope** (series fields live on `works`; series assignment flows through merge).

---

## 3. Ideal state — 8 behavioral promises (target A)

- **P1 — Same book, same result, every door.** All 8 entry paths converge on identical metadata; they differ only in starting richness, never in destination.
- **P2 — A book's language is sacred.** A French book shows French title/description/cover, never borrows a Chinese/English edition. Language set once at identity; only a user changes it.
- **P3 — One book, one entry, the right book.** No duplicate rows for the same edition; no silent swap to a different book; only ever enrich the book you actually have.
- **P4 — Nothing the user set is ever clobbered.** Manual edits, locked covers, user choices survive every automatic refresh; a matched source's key is persisted.
- **P5 — Partial beats wrong beats empty.** Provider down → fewer fields, never a wrong value, never a blocked add.
- **P6 — What's in the DB is in the file.** After enrichment, cover-on-disk and file tags match the DB (Principle 5).
- **P7 — Refresh only ever helps.** Refresh improves or preserves, never regresses; honors the active filter.
- **P8 — No phantom fields.** Every stored column is populated by some path and read by some consumer. No always-empty/always-0 fields, no write-never-read.

### Per-stage assertions (how each promise is upheld)

| Stage | Ideal-state assertion | Serves |
|---|---|---|
| 1 Entry/seed | Every path forwards *all* signal it holds (language, ISBN, keys, cover); sparse OK, *dropped* not | P1 |
| 2 Identity | Match key **includes language**; identity locked + LLM-validated; no dup rows | P2,P3 |
| 3 Dispatch | Only applicable providers; each **queried scoped to the work's language** (GB `langRestrict`) | P2 |
| 4 Normalize | Each payload carries its own detected language; cover URLs absolute; IDs shape-checked | P2 |
| 5 Validate | Reject different-book OR different-language payloads, with a recorded reason | P2,P3 |
| 6 Merge | Wrong-language can't win; null never overrides real; user fields preserved; matched keys persisted | P2,P4 |
| 7 Persist | Atomic CAS; **every resolved field actually written**; nullable clears only when intended | P6,P8 |
| 8 Cover | Best **same-language** source, dims captured, refresh upgrades, user-lock respected, URL live | P2,P7,P8 |
| 9 Tags | File tags converge to DB metadata; sync tracked | P6 |
| 10 Refresh | Each mode a precise, idempotent, monotonic contract; no inescapable terminal state; honors filter | P5,P7 |

---

## 4. Structural dimensions (target B)

Two tiers: **(A) conformance to livrarr's own declared architecture** (compile wall; deps→domain; no SQL outside db; trait+impl+stub; enum-dispatch not dyn; pure merge engine), **(B) general structural health.**

| # | Dimension | What to examine | Suspect already in view |
|---|-----------|-----------------|--------------------------|
| **S1** | Boundary / layering conformance | deps→domain only; no cycles; no SQL outside `livrarr-db`; compile wall; no logic in DB-apply/handlers | The 4-way metadata split's edges — `livrarr-metadata` depends on external-data+identity+enrichment+db+matching: coherent orchestrator or catch-all? |
| **S2** | Crate/module cohesion (one job) | each module single responsibility; no god modules | `work_service.rs` ≈ **3,383 lines** (add+refresh+bulk-refresh+unified-enrichment+eager-match) |
| **S3** | Convergence by sharing, not duplication | all 8 entry paths share ONE funnel vs. reimplement seed/identity/merge | manual-import / list / Readarr / author-monitor each build their own seed — confirm true sharing of `add`/`run_unified_enrichment` |
| **S4** | Provider abstraction uniformity | one interface; adding a provider is localized | legacy `MetadataProvider` trait (2 impls) coexisting with `ProviderClient::fetch` enum dispatch = half-migrated; Readarr special-casing |
| **S5** | Cross-cutting concerns have a home | language/provenance/cover/pacing centralized vs. smeared | **language** — P2 likely needs edits in dispatch+normalize+validate+merge+cover (no single home). *Bridge metric.* |
| **S6** | Data-model integrity | entities/enums coherent+minimal; no vestigial types/dead columns/concept↔type divergence/missing fields | `Conflict*` cluster (~8 types) vestigial post-rename; overlapping `thin`/`EnrichmentStatus`/`IdentityStatus`; `WorkFilter` has **no language field**; always-empty `metadata_source`, always-0 `cover_dims` |
| **S7** | Outcome/error modeling coherence | provider-outcome→enrichment-status→API-error one taxonomy, handled consistently | `list_works` skips bad rows; `list_works_paginated` fails whole query → 500 |
| **S8** | State & concurrency structure | locks/CAS/guards/pacing a coherent subsystem | `bulk_refresh_users` Mutex<HashSet> held for the entire bulk loop (caused #135); per-work lock + CAS + breaker + rate-limiter + retry-state coherent? |
| **S9** | Testability seams | trait+impl+stub per stage; merge engine genuinely pure/unit-testable; providers mockable | verify the merge engine's claimed purity; IO buried in "pure" code |

(S10 conceptual integrity / naming — names match domain vocabulary, e.g. the "`import_pipeline.rs` is actually utilities" trap — folded into S6.)

**The bridge:** for each behavioral gap, assess **how localized the fix is.** A gap whose fix isn't localized *is* a structural finding (→ fix the structural root, which closes a cluster of bugs, not each symptom).

---

## 5. Known-bug falsification set (the audit MUST re-derive each)

| Item | Symptom | Promise / dim | Stage |
|---|---|---|---|
| #133 | foreign enrichment merges wrong-language edition (GB `zh` for `fr`) | P2 / S5 | 3,5,6 |
| #134 | foreign covers: refresh doesn't re-resolve; `cover_width/height=0` | P7,P8 / S6 | 7,8 |
| #135 | "Refresh All" ignores language filter + 409 | P7 / S8 | 10 |
| #132 | audiobook manual import: nested works not listed + common titles fail match | P3 / S3 | 1,2 |
| #11 | GR resolver returns wrong book for foreign titles | P3 | 2,5 |
| #8 (internal) | audiobook language never extracted; selector language-blind | P2 / S5 | 1,6 |
| #96 | BCP-47 locale tags (`en-us`) misroute English | P2 | 2,3 |
| #110 | refresh flips whole work to Conflict on single-provider dissent | S6 (enum semantics) | 5,6 |
| #112 | foreign-edition series leaks into English bibliography | P2,P3 | 2,6 |
| #111 | standalone 'Anathem' shown as 3-book series | P3 | identity/series |
| #109 | series line should be blank, not placeholder | P8 | display/data |
| #59 | Conflict-status works have no `cover_url` | P8 | 8 |
| #58 | series workflow: find existing works and map | P3 | 2 |
| #53 | adding from author biblio creates junk entries | P3 | 1,2 |
| #52 | series monitoring fails | adjacent | 10 |
| obs | `gr_key` dropped despite GR success payload | P4 | 6 |
| obs | duplicate rows (La Nuit Des Temps ×2; Pan Tadeusz/Master Thaddeus) | P3 | 2 |
| obs | `三体` tagged `language=es` | P2 | 2,4 |
| obs | `cover_dims=0`, `metadata_source=''` across rows | P8 | 7 |
| obs | dual provider dispatch (legacy `MetadataProvider` trait) | S4 | 3 |

For each: the audit explains the mechanism with `file:line`, ties it to the violated promise/dimension, and surfaces the **unknown siblings** in the same code.

---

## 6. Lifecycle spine — stages & where they live

(Authoritative current behavior: `wiki/architecture/metadata-pathway.md` — note it + `enrichment-pipeline.md` are **partly stale** (pre-3-way-split crate names; `EnrichmentStatus::Conflict` was dropped/renamed to `IdentityStatus::NotFound` in `6164915`). Reconcile them as part of output.)

Key entry points to trace from:
- `WorkService::add` / `refresh` / bulk-refresh (`livrarr-metadata/src/work_service.rs`)
- `WorkServiceImpl::run_unified_enrichment` (post-add/refresh materialization)
- `EnrichmentService(Impl)::enrich_work` (dispatch→validate→merge→CAS apply)
- `DefaultProviderQueue::dispatch_enrichment` (scatter-gather, applicability rule, pacing)
- provider clients (`livrarr-external-data/src/{provider_client,goodreads,google_books,...}.rs`)
- `MergeEngine` (the pure field-winner logic; priority model)
- `SqliteWorkRepository::apply_enrichment_merge` + `list_works(_paginated)` (`livrarr-db/src/sqlite_work.rs`)
- cover resolution / download-to-disk; `TagService::retag_library_items` (`livrarr-tagwrite`)
- entry paths: `manual_import` (handlers), `list_service`, `readarr_import_workflow` (server), `author_monitor_workflow`, `rss_sync_workflow`

`works` table columns (for the empirical pass): id, user_id, title, sort_title, subtitle, original_title, author_name, author_id, description, year, series_name, series_position, genres, language, page_count, duration_seconds, publisher, publish_date, ol_key, hc_key, isbn_13, asin, narrator, narration_type, abridged, rating, rating_count, enrichment_status, enriched_at, enrichment_source, cover_url, cover_manual, monitor_ebook, added_at, enrichment_retry_count, metadata_source, detail_url, monitor_audiobook, gr_key, import_id, series_id, merge_generation, normalized_title, normalized_author, cover_source, cover_trust, cover_width, cover_height, audiobook_cover_url, audiobook_cover_source, audiobook_cover_trust, audiobook_cover_width, audiobook_cover_height, identity_status.
Aux tables: `provider_retry_state` (user_id, work_id, provider, attempts, suppressed_passes, last_outcome, last_attempt_at, next_attempt_at, normalized_payload_json, first_suppressed_at); `work_metadata_provenance` (user_id, work_id, field, source, set_at, setter, cleared).

---

## 7. Method

- **Static (primary):** exhaustive top-down read of every function per stage. Trust file contents over the index.
- **Empirical:** cross-check the code's *claimed* writes against the DB's *actual* values — columns that are always-null/always-0 with a code write path = broken write path (P8/S6). Already-known: `cover_width/height=0`, `metadata_source=''`. Also compare across entry paths (P1/S3): pick a few works added via different paths, diff their final field/provenance state.
- **Bridge metric:** for each behavioral gap, rate fix-localization (1 site vs. N sites) → feeds the structural findings + the root-cause fix plan.

---

## 8. Workflow design (run via the Workflow tool)

**No verify phase.** Concurrency auto-caps; total agents ≈ 17.

**Phase 1 — fan-out (parallel):**
- **1a · Stage agents (10)** — one per lifecycle stage (§6). Each: exhaustive read of that stage's code across whatever crates it touches; assess against the stage's promises; re-derive the assigned known bugs; apply the structural lens locally; do the empirical check where relevant. Structured output: `{stage, behavioral_gaps[], structural_notes[], known_bugs_explained[], new_findings[], fix_localization}`.
- **1b · Cross-cutting structural agents (6)** — one each for S1 (seams/layering), S2 (cohesion/god-objects), S3 (entry-path duplication), S5 (concern-homes), S6 (data-model/dead-types/dead-columns), S8 (concurrency). Whole-subsystem view. Output: `{dimension, findings[], severity, affected_files[]}`.
- **1c · Empirical DB agent (1)** — runs the actual-vs-claimed DB cross-checks; produces dead-field/missing-write-path + entry-path-divergence evidence.

**Phase 2 — synthesis (barrier):**
- Merge + dedupe all findings; build the unified **lifecycle map** annotated with defects; **correlate** behavioral gaps ↔ structural roots; cluster root causes; produce the **fix plan aimed at the ideal** (sequenced, root-first); reconcile the two stale wiki pages; optionally emit the metadata-domain `data_flow` (sources/sinks/invariants) for the canonical model.

Agent prompts must include the tooling constraints from §0 (code-index not Serena; read actual files; read-only).

---

## 9. Output

1. **Unified lifecycle map** — the 10 stages with each defect located.
2. **Behavioral gap inventory** — keyed by promise (P1–P8), stage, severity, `file:line`.
3. **Structural findings** — keyed by dimension (S1–S9), severity, affected files.
4. **Correlation table** — structural root → behavioral bugs it causes.
5. **Root-cause fix plan** — sequenced, root-first, aimed at the 8 promises + structural roots. Diagnosis only; no code.
6. **Dead-field / missing-write-path inventory** (empirical).
7. **Wiki reconciliation** — corrected `metadata-pathway.md` + `enrichment-pipeline.md`.
8. (Optional) metadata-domain `data_flow` for the canonical model.

Write the report to `docs/metadata-lifecycle-audit-findings.md`.

---

## 10. Guardrails

- Read-only. No edits, no commits, no push. Don't disturb the uncommitted alpha-6 batch.
- Pseudonymity: any `gh`/git op uses the `kkodecs` account (already active).
- This is diagnosis; implementation is a separate build cycle the PO will authorize after reviewing the findings.
