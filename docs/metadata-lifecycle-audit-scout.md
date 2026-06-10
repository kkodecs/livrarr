# Metadata Lifecycle Audit — Scout Pass (2-agent)

**Status:** scout (not the exhaustive audit). Read-only diagnosis, no code changes.
**Date:** 2026-06-07 · branch `wcc-stage5-green` (uncommitted alpha-6 tree)
**Coverage:** 2 agents — (1) entry paths / seed, (2) core funnel (Identity→Refresh) + light empirical DB check on `testdata/livrarr.db` (125 works).
**Purpose:** orient on the big rocks, confirm the known bugs are real, decide whether the full ~6-agent audit is warranted. Items marked **[deep pass]** need the exhaustive audit to fully nail.

---

## 0. Executive summary

The subsystem has **one real creation funnel** (`WorkServiceImpl::add`) and an **isolated pure merge core** — the architecture's spine is sound. Almost every known bug traces to one of **8 structural roots**, and most are root-localized (fix the root, close a cluster). The headline: *convergence is structural but not behavioral* — paths funnel through shared code, but the **seed they hand in** and the **policies that code applies** diverge by entry path and by cached-vs-network, so the same book lands differently depending on the door.

Two brief assumptions were **corrected by evidence**:
- "8 entry doors" → **6** seed doors. `rss_sync` and library `import_workflow` grab files onto existing `work_id`s; they build no seed.
- "dual provider dispatch (legacy `MetadataProvider` trait)" → **not live dual-dispatch**. The legacy trait has **zero implementors and zero `dyn` usage** — it's vestigial dead surface, not a running second path.
- Bonus: `wiki/domain/metadata-sources.md:69` claims foreign refresh is skipped via `metadata_source` — **false**; that column has no writer (0/125), so the gate reads a column nothing fills.

**Top 8 root causes (ranked):**

| # | Root cause | Closes | Sev |
|---|-----------|--------|-----|
| R1 | No shared **SeedBuilder**; 6 hand-built seeds, 4 hardcode `language="en"` | P1 seed divergence, #8, part of #53 | High |
| R2 | **Language has no home** — stamped from *query intent* at GR lookup, routed on *raw string*, re-normalized at create & merge | #11, 三体=es, #96 | High |
| R3 | **Two foreign-merge policies** — cached path drops OL/HC for foreign, network path keeps them | #133, P1 | High |
| R4 | **Inconsistent anchor overwrite** — `gr_key/hc_key/ol_key/isbn` hard-set, `asin` COALESCE, same UPDATE | gr_key drop (82/120 NULL) | High |
| R5 | **Whole-work conflict** — one provider/LLM dissent discards ALL merged fields | #110, #59 (part), P5 | High |
| R6 | **Phantom columns / missing write paths** — `metadata_source` 0/125, `cover_w/h` writer uncalled, `series_id` 0/125 | #134, P8, #58/#52/#111/#112 | High |
| R7 | **`enrichment_status` string↔enum drift** — `'exhausted'`/`'pending'` written but unrepresented; stale selector filters `'unenriched'` | refresh-never-completes risk | High |
| R8 | **God object + dead surface** — `work_service.rs` 3,383 lines / 6 type params; dead `MetadataProvider` trait | maintainability; masks the above | Med |

---

## 1. Unified lifecycle map (defects located)

```
ENTRY (6 doors) ──► [R1 6 hand-built seeds, 4× lang="en"] ──┐
  manual_import (rich: re-reads file)                       │
  Add Work box (drops desc/series)                          │
  list_service (lang=en, drops cover/series/desc)           ├──► WorkService::add  ── ONE funnel ✓
  author_monitor (lang=en, Confirmed-from-raw-OL → junk #53)│         │
  series_query (lang=en, seed_anchors=None → drops gr_key)  │         ▼  dedup 3-tier (anchor + normalized title/author)
  readarr_import (rich)                                     │   [R6 series_id always None; no cross-lang/cross-key merge → dup rows]
                                                            │         ▼
IDENTITY/LOOKUP  [R2 lookup_goodreads stamps query-lang on every hit → #11/三体]
DISPATCH         [provider_priority routes on RAW string → #96 en-us misroute]  · [deep pass: provider_queue not read]
NORMALIZE        (per-provider; language re-derived)
VALIDATE         [R5 LLM identity_valid=false → whole-work Unenriched #110]
MERGE  pure core ✓  but  [R3 cached drops OL/HC foreign ≠ network keeps them → #133/P1] · [R5 any Conflict blocks whole merge]
PERSIST apply_enrichment_merge  [R4 gr_key unconditional set ≠ asin COALESCE → key drop] · [R6 metadata_source never bound]
COVER  [R6 update_cover_dimensions uncalled; finish_created_work writes 0,0; backfill skips if {id}.jpg exists → #134]
TAGS   (not deeply traced this pass)
REFRESH [#135 WorkFilter has no language → ignores filter] · [HashSet flag held whole loop → panic = permanent 409]
        [R7 reset writes 'pending', selector wants 'unenriched' → may never re-enrich]
```

---

## 2. Known-bug re-derivation ledger

| Bug | Re-derived | Mechanism (abridged) | Location |
|-----|-----------|----------------------|----------|
| **#8** audiobook language never extracted | ✅ yes | Seed contract is ebook-shaped (`WorkSeedFields` has no narrator/duration/audiobook fields); 4 doors hardcode `en`; only manual re-reads file | identity.rs:431-442; list_service.rs:66; author_monitor:325; series_query:657 |
| **#132** nested works + common-title match | 🟡 partial | (a) `group_audio_files` keys on immediate parent, stray non-divider sibling → fragments; (b) common title, no anchor → fuzzy match mis-binds | manual_import.rs:1292-1378, 616-637 · **[deep pass]** matching trace |
| **#53** author-biblio junk entries | 🟡 partial | author-monitor adds every "eligible" OL biblio entry as **Confirmed**, no quality screen → row 1949 wrong-author junk | author_monitor_workflow.rs:300-360 · **[deep pass]** what "eligible" filters |
| **dup rows** (La Nuit ×2; Pan Tadeusz/Master Thaddeus) | ✅ yes | No cross-language / cross-OL-key merge; dedup keys on language-specific `normalized_title`. `GROUP BY normalized_title,author HAVING count>1` = **0 rows** → every dup is a key MISS | work_service.rs:507-682 |
| **#133** foreign merges wrong-lang edition | 🟡 partial | `merge_from_cached` drops OL/HC for foreign (lib.rs:463-471) but network `merge_impl` keeps them via `PriorityModel::foreign()` (lib.rs:264-291); no per-field language guard | lib.rs:264-291 vs 463-471 · **[deep pass]** end-to-end |
| **#134** foreign covers / `cover_w/h=0` | ✅ yes | `update_cover_dimensions` exists but **no caller**; cover writers hardcode `0,0`; backfill skips if `{id}.jpg` exists → no re-resolve. Empirical: `cover_width`>0 only 8/125 | sqlite_work.rs:661; cover_backfill.rs:60-74; work_service.rs:2601 |
| **#135** Refresh All ignores filter + 409 | ✅ yes | `WorkFilter` has **no language field**; spawned loop only clears HashSet flag at the very end → panic = permanent 409 | work.rs:661-681,742; work_service.rs:61,1774; services/work.rs:112-120 |
| **#11 / 三体=es** GR wrong book / mis-lang | ✅ yes | `lookup_goodreads` stamps the **query lang** onto every autocomplete hit (GR returns none); the hard lang guard then trusts the fabricated label | work_service.rs:1939 |
| **#96** BCP-47 `en-us` misroute | 🟡 partial | `provider_priority` matches the **raw** string (only `en/eng/english/''`→English); `normalize_language` strips region but isn't called first | language.rs:153-156 vs normalization.rs:167 · **[deep pass]** find bypass write |
| **#110** refresh flips whole work to Conflict | ✅ yes | LLM `identity_valid=false` → `conflict_detected, work_update:None, Unenriched` for the **whole work**; deterministic path: any `Conflict` outcome blocks the merge | lib.rs:1109-1126, 690-707 |
| **#59** conflict works have no `cover_url` | 🟡 partial | Both conflict returns set `cover_resolution:None`; status-only DB branch doesn't touch `cover_url`. Actual NULL-write not found; **0 conflict rows** in test DB | lib.rs:704-705,1124; sqlite_work.rs:962-974 · **[deep pass]** |
| **gr_key dropped** | ✅ yes | `gr_key = ?` unconditional (not COALESCE) → a merge with no gr-eligible provider NULLs the create-time key. 82/120 enriched works NULL gr_key | sqlite_work.rs:919,943 |
| **metadata_source='' / cover_dims=0** | ✅ yes | `metadata_source` bound by **no** write path (0/125); cover dims per #134 | sqlite_work.rs:912-924,422-450,1176 |
| **dual provider dispatch** | ✅ yes (corrected) | Legacy `MetadataProvider` trait has **zero impls / zero dyn** — vestigial, not live | metadata/lib.rs:55-65 |
| **#58/#52/#111/#112** series | 🟡 partial | `series_id` 0/125; 106 works carry orphan `series_name` strings, no FK; foreign series_name merges into English biblio with no entity dedup | sqlite_work.rs:1197 · **[deep pass]** materialization |

**Re-derived:** 9 yes, 6 partial, 0 missed. The partials all need the matching-service / provider-queue / series-materialization reads the scout deliberately skipped.

---

## 3. Behavioral gap inventory (by promise)

- **P1** — 6 hand-built seeds + cached-vs-network foreign policy split → same book diverges by door and by refresh-vs-add. *High.*
- **P2** — 4/6 doors stamp `en`; language assigned by query intent at GR; routing on raw string. *High.*
- **P3** — no cross-language/cross-key work merge (dup rows); `list_works` skips corrupt rows while paginated 500s (same library, two counts). *High/Med.*
- **P4** — `gr_key` (and latent `isbn/hc/ol`) nulled by unconditional set on single-provider refresh. *High.*
- **P5** — one Conflict/LLM dissent empties the whole work (wrong beats partial, inverted). *High.*
- **P6** — covers/tags: foreign cover never re-resolved on refresh; dims never captured. *High.*
- **P7** — Refresh All ignores filter, can permanently 409-lock, can regress cover & key. *High.*
- **P8** — phantom columns: `metadata_source` (0/125), `cover_w/h` (8/125), `series_id` (0/125). *High.*

---

## 4. New findings (not in the brief's known set)

| Finding | Sev | Location |
|---------|-----|----------|
| `enrichment_status='exhausted'` written but **no enum variant** — parsed back as `Failed`; a hidden state retry/refresh can't see | Med | sqlite_work.rs:1308-1311 vs 180-207 |
| **Refresh-never-completes risk:** `reset_enrichment_for_refresh` writes `'pending'`, but `list_stale_unenriched_works` filters `'unenriched'` → reset rows may never be picked up | **High** | sqlite_work.rs:1278 vs 843 · **[deep pass]** |
| `list_works_for_enrichment` still queries `('pending','partial','failed')` — migration 035 collapsed those → returns ~empty (vestigial selector) | Med | sqlite_work.rs:703-712 |
| `asin` COALESCE vs `gr/hc/ol/isbn` unconditional in the **same** UPDATE — the root of gr_key drop + latent multi-key drop | High | sqlite_work.rs:919-920,943-946 |
| `merge_from_cached`'s computed `cover_resolution` appears **dropped** on the add-reuse path (only create-time `cover_url` written) | Med | work_service.rs:2549-2605 · **[deep pass]** |
| `series_query_service.rs:668` live `TODO(REQ-006)` — series-monitored works created with `seed_anchors:None`, dropping `book.gr_key` | Med | series_query_service.rs:666-671 |
| Add Work box discards picked candidate's `description/series` at seed; manual import extracts file metadata **twice** (scan + import) and they can disagree | Low | work.rs:265-277; manual_import.rs:404 vs 1037 |

---

## 5. Structural findings (by dimension)

- **S2 cohesion** — `work_service.rs` 3,383 lines / `WorkServiceImpl<D,E,H,L,M,T>`: identity, all add variants, refresh, bulk-refresh, 4-provider inline lookup+parse, LLM filtering, cover orchestration, eager-match. **The metadata god object.** *High.*
- **S3 convergence** — ✅ one funnel (`add` → single `CreateWorkDbRequest`; refresh/bg re-enter via `run_unified_enrichment`). Divergence is *within* (seeds + foreign policy), not the funnel. *Low (positive).*
- **S4 provider abstraction** — `ProviderClient` enum-dispatch is the live path; legacy `MetadataProvider` trait is **dead surface to delete**. *Med.*
- **S5 language (cross-cutting, no home)** — handled at lookup (query stamp), routing (raw match), normalization (create + merge); routing fn and normalizer **disagree on input contract**. *Med.*
- **S6 data-model integrity** — phantom `metadata_source`/`cover_w/h`/`series_id`; `WorkFilter` lacks a language field; `enrichment_status` string↔enum drift. *High.*
- **S7 error modeling** — `list_works` skip-and-drop vs `list_works_paginated`/`search_works` propagate→500. Same corruption, two behaviors. *Med.*
- **S8 concurrency** — bulk-refresh membership flag (`Arc<Mutex<HashSet>>`) held for the whole loop, cleared only at the end → panic strands the user. *High (= #135).*
- **S9 testability** — pure deterministic core (`merge_impl`) is well isolated ✅, but the public `merge` entrypoint couples LLM I/O + whole-work arbitration into the merge stage. *Med.*

---

## 6. Correlation: structural root → behavioral bugs

| Root | Bugs it causes |
|------|----------------|
| R1 no SeedBuilder / `en` defaults | P1 divergence, #8, #53 (part) |
| R2 language no home | #11, 三体=es, #96 |
| R3 cached≠network foreign policy | #133, P1 |
| R4 anchor overwrite asymmetry | gr_key drop, latent isbn/hc/ol drop |
| R5 whole-work conflict | #110, #59 (part), P5 |
| R6 phantom columns / dead write paths | #134, metadata_source, series_id → #58/#52/#111/#112 |
| R7 status string↔enum drift | refresh-never-completes, hidden `exhausted` |
| R8 god object + dead trait | not a bug; raises change-cost and hides R1–R7 |

---

## 7. Root-cause fix plan (sequenced, root-first — diagnosis only)

1. **Single `SeedBuilder` + real per-door language** (R1/R2) — one builder fills every `WorkSeedFields` from whatever the door holds; each non-manual door derives language from its source, never `.into("en")`. *Closes P1 seed divergence, #8; localized to 6 sites + 1 builder.*
2. **One language home** (R2) — normalize at the single identity boundary; stop deriving language from query intent in `lookup_goodreads`; make `provider_priority` normalize its input. *Closes #11/三体/#96.*
3. **Unify foreign-merge policy** (R3) — hoist the foreign OL/HC drop into `PriorityModel::foreign()`/`merge_impl` so cached and network share one filter. *Closes #133, P1-merge.*
4. **Consistent anchor overwrite** (R4) — COALESCE all anchor keys (or carry them through merge). *Closes gr_key drop.*
5. **Per-field conflict** (R5) — drop the dissenting provider from `eligible_providers` instead of returning a blocking `MergeOutput`; reserve whole-work conflict for true identity contradiction. *Closes #110, #59 (part), P5.*
6. **Wire/retire phantom columns** (R6) — call `update_cover_dimensions` (read jpg dims post-download; replace the `0,0` literals); add `metadata_source` to the UPDATE/INSERT *or* delete it + the false foreign-skip gate; decide `series_id` real-vs-phantom. *Closes #134, P8.*
7. **Fix `enrichment_status` machine** (R7) — reconcile written strings (`pending`/`exhausted`) with the enum and the stale selector. *Closes refresh-never-completes.*
8. **Refresh All** (S8) — add language to `WorkFilter` + thread into `refresh_all`; replace the HashSet flag with an RAII/time-boxed lease. *Closes #135.*
9. **Cleanup** (R8) — delete the dead `MetadataProvider` trait; split `work_service.rs` along the seams above. *Lowers change-cost; do last.*

---

## 8. Recommended next step

The scout confirms the brief's bug set is **real and mostly root-localized** — and the architecture is salvageable, not rotten. Open question: whether to commission the **full ~6-agent audit** to nail the 6 partials, or go straight to a fix cycle on R1–R8.

The deep-pass items that would change the fix plan if confirmed:
- **[deep pass]** `provider_queue.rs` dispatch/applicability/pacing/circuit-breaker — **not read this pass at all**.
- **[deep pass]** `#133` network path end-to-end; per-field language guard existence.
- **[deep pass]** `#59` the actual cover-NULL-on-conflict write (0 conflict rows in test DB — needs a seeded conflict).
- **[deep pass]** series materialization (`#58/#52/#111/#112`) — 106 orphan `series_name` strings.
- **[deep pass]** `'pending'`-reset vs `'unenriched'`-selector (refresh-never-completes) — high-stakes, confirm before R7.
- **[deep pass]** `#132` common-title matching trace; `#96` the bypass write.

---

*Generated by the 2-agent scout (entry-paths + core-funnel). For the exhaustive version, run the §8 workflow from `docs/metadata-lifecycle-audit-brief.md`.*
