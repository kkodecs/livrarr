# Metadata Lifecycle Audit — Deep-Pass Addendum (5 targeted agents)

**Status:** read-only diagnosis. Companion to `metadata-lifecycle-audit-scout.md`.
**Date:** 2026-06-07 · branch `wcc-stage5-green`
**Purpose:** close the 5 scout gaps that could change the fix plan. Each agent: confirm / refute / refine one question. Verdicts below **supersede** the scout where they conflict.

---

## Verdicts at a glance

| # | Question | Verdict | Net effect on fix plan |
|---|----------|---------|------------------------|
| DP1 | provider_queue: applicability, pacing, scatter-gather (P5) | **Refined** | P5 is fine — scatter-gather never blocks merge except on Conflict. New: foreign works lose **HC + OL** entirely (hardcoded). |
| DP2 | refresh-never-completes (`'pending'` vs `'unenriched'`) | **REFUTED** | **Drop R7 as a bug.** Refresh runs enrichment inline (Manual), overwrites `'pending'` instantly. |
| DP3 | #59 cover-NULL on conflict | **REFUTED** | **Re-frame #59.** Nothing nulls cover; held works just never reach cover resolution. Product gap, not a write bug. |
| DP4 | series materialization (#58/#52/#111/#112) | **Refined** | Series entity exists + works, but **0 rows** — nothing back-fills 106 string-only works. 3 precise roots. |
| DP5 | #133 network path + #96 BCP-47 write | **Refined / split** | **#133 ESCALATED** (real on refresh path, cached fix incomplete). **#96 REFUTED** (writes already normalized). |

**Bottom line: 3 items leave the fix plan (R7, #59-as-written, #96-write-path), 1 escalates to top priority (#133), and the series cluster gets 3 sharp high-severity roots.**

---

## DP1 — provider queue (Refined)

**P5 graceful degradation: CONFIRMED sound.** Scatter-gather (`provider_queue.rs:427-481`) isolates panics (JoinSet → missing provider backfilled as `ProviderPanic`); one provider's failure never blocks others. Merge is hard-blocked **only** by a `Conflict` outcome (`:534-537`); otherwise `deferred` (Background) or forced (Manual/HardRefresh). Pacing (GCRA token bucket + semaphore + in-memory breaker) is coherent and decoupled from `provider_retry_state` persistence; the concurrency-before-ratelimit ordering is bounded by `max_queue_time=30s` (no hang risk). **→ The fix plan should NOT add any "provider failure aborts merge" concern.** The only merge-blocker is R5 (whole-work conflict).

**New finding (medium):** Foreign-language applicability gates out **Hardcover AND OpenLibrary** — for any non-`en` work, `main.rs:303-314` dispatches only `Goodreads, Audnexus, GoogleBooks, Audible`. OL exclusion is intentional product policy (per "No OL for foreign enrichment"). **HC exclusion is undocumented — confirm intent.** This rule is a hardcoded closure in the composition root, not data-driven, and is *not* exercised by the queue's behavioral tests (they use the default always-true rule).

**New finding (low):** GB `langRestrict` is applied on the title+author search path only, never the ISBN path (`google_books.rs:177-201`) — fine by design, noted for cross-language ISBN false-positives.

---

## DP2 — refresh-never-completes (REFUTED)

The scout's premise (a reset work must be picked up by the background `'unenriched'` selector) is **false for the refresh path.** `WorkServiceImpl::refresh` (`work_service.rs:1208-1248`) resets to `'pending'` then **synchronously** runs `run_unified_enrichment(…, Manual)` inline (`:1233`), which overwrites `enrichment_status` via `apply_enrichment_merge` the instant the merge completes. The `'pending'` string never waits on any selector. Bulk refresh is identical (loops `refresh` per work). **The two resets have no other production callers.**

**→ R7 is not a real bug.** Downgrade to two cosmetic hygiene notes (low):
- `'pending'` is a dead/legacy DB string — no live read keys on it (parse folds → `Unenriched`). Resets could write `'unenriched'` to match the only live enum. Cosmetic.
- `'exhausted'` (`increment_retry_count:1309`) also has no enum variant (parses → `Failed`) — but this is an **intentional terminal sink** (retries spent; no selector re-picks it). By design, not a bug. Confirms a string↔enum drift *pattern* in this column worth a cleanup, not a fix.

**Residual (low likelihood):** if inline enrichment's merge writes no status for *all* retries (contended/failing-merge edge) AND identity ∉ (confirmed, provisional), a work could sit at `'pending'` uncaught. Edge, not the claimed mechanism.

---

## DP3 — #59 cover-NULL on conflict (REFUTED)

**No code path nulls `cover_url` on a conflict / `NotFound` transition.** Full write-trace:
- Merge conflict branch returns `work_update:None, cover_resolution:None` (`lib.rs:695-707`) → no work-row write.
- Status-only DB branch (`sqlite_work.rs:962-974`) writes only `enrichment_status` — never references `cover_url`.
- `set_identity_status` (`:564-583`) writes only `identity_status`.
- Held identities (Pending/Conflict/NeedsReview) **return early** (`work_service.rs:2678-2710`) *before* `run_unified_enrichment`, so cover resolution never runs for them.

**→ #59 is mechanism (b): held/unverified works lack a cover because resolution never ran, not because anything cleared it.** Re-frame from "stop nulling cover on conflict" (nonexistent write) to a **product question: should held/unverified works still get a cover?** If yes, the lever is the pre-gate cover step / allowing resolution for held identities — not a write-site bugfix. (Empirical caveat: test DB has 0 conflict/not_found rows, so #59 can't be reproduced from data; conclusion rests on code-path analysis.)

**New finding (low, latent footgun):** `apply_enrichment_merge` work-update branch writes `cover_url = ?` **without COALESCE** (`sqlite_work.rs:921/952`), safe today only because `lib.rs:923` feeds the work's own existing `cover_url` back in. If any future change makes `work_update.cover_url` a merge-resolved `Option` that can be `None`, this **will** null an existing cover. Recommend defensive `cover_url = COALESCE(?, cover_url)`.

---

## DP4 — series materialization (Refined)

**A normalized `series` entity DOES exist and works** (domain `Series`, `sqlite_series.rs`, FK `works.series_id`) — but it is written **only** by the explicit, user-triggered Goodreads series-monitor flow (`series_query_service.rs:472/628/677`). Ordinary enrichment and author-monitor set the descriptive `series_name` string and leave `series_id: None`. **Empirical: 125 works, `series_id` 0/125, `series_name` 106/125, `series` table 0 rows.** Three precise roots:

| Bug | Root mechanism | Location | Sev |
|-----|----------------|----------|-----|
| **#58** find/map existing | **Nothing back-fills `series_id` from `series_name`.** Library>Series (`list_enriched`) groups strictly by FK → renders **empty** despite 106 `series_name` works. | `series_query_service.rs:68,135` (FK-only group); writers only at `:472/:628/:677` | High |
| **#112** foreign-edition leak | `build_merged_series_list` falls back to **exact string equality** `w.series_name == ce.name` (`:953,969`) — a localized/variant `series_name` silently fails to group; foreign editions drop out or form garbled phantom groups. | `series_query_service.rs:953,969` | High |
| **#111** standalone shown as N-book | `fetch_series_from_book_search` **synthesizes** a series and sets `book_count` = number of author search-result titles whose parenthetical matches the tag (≤3 pages), with empty `gr_key`. Series "length" is an artifact of search-result repetition, not an authoritative count. Empty `gr_key` also makes these permanently unmonitorable. | `series_query_service.rs:890-942` | High |

**#52 (monitoring):** the FK traversal is sound, but monitoring only ever has an FK for user-monitored series — it never covers the pre-existing string-only corpus until a dedup-match happens. **Back-fill (#58) is the prerequisite** that makes monitoring meaningful for existing works.

**New finding (medium):** `link_work_to_series` only relinks a work if the new series has a *smaller* `work_count` (`sqlite_series.rs:163-172`) — a larger/cleaner canonical series can never override a smaller stale one. The #58/#112 reconcile design must account for this guard.

---

## DP5 — #133 foreign merge (CONFIRMED, escalated) + #96 BCP-47 (REFUTED)

**Trace A — #133 CONFIRMED and bigger than the scout thought.** The foreign-language OL/HC drop exists **only** in the cached fast-path `merge_from_cached` (`lib.rs:463-471`); the **network/refresh path does not replicate it.** `run_unified_enrichment` feeds the unfiltered `reconstructed` payloads straight into `MergeInput` (`lib.rs:1789-1797`). `PriorityModel::foreign()` (`:264-291`) still lists Hardcover/OpenLibrary/Audible as content/description fallbacks, and `merge_impl`'s per-field winner loop (`:768-776`) has **no per-field language guard at all.** → For a French work where GoogleBooks+Goodreads are empty on a field (description, subtitle, series, publisher, genres), an **English HC/OL value wins and is written.** The cached add path is protected; refresh is not — a path asymmetry that's easy to miss because the protective code visibly exists nearby.

**→ The cached-path fix is INCOMPLETE.** This is now the **top correctness bug.** Two fix options:
1. *(lowest-surprise)* Replicate the input filter before `MergeInput` at `lib.rs:1789` — drop OL/HC (and reconsider Audible for ebook fields) from `reconstructed` when `provider_priority(language)==Foreign`. Mirrors the existing cached fix; one mental model for both paths.
2. *(principled, riskier)* Add a per-field language-compatibility guard in `merge_impl`, or remove HC/OL/Audible from `foreign()`'s content/description lists (cover/audio lists may still legitimately want them — needs care).
The in-code comment at `:461-462` already notes reordering `PriorityModel` alone is insufficient.

**Trace B — #96 REFUTED on the live tree.** Every `works.language` write **is normalized upstream** before the DB. The three SQL bind sites (`sqlite_work.rs:461/937/1195`) do raw binds, but all production callers normalize: merge sets `normalize_language(...)` (`lib.rs:906`); both `create_work` builders normalize (`work_service.rs:717,846`). `normalize_language` strips region (`en-US→en`, `fr-FR→fr`). Empirical: test DB holds only clean `en/fr/pl/es`, zero region tags. **→ Drop the "#96 region-tag write" line item.** Residual (low): a provider could supply an unknown primary subtag that the None-fallback stores lowercased as-is (benign for routing → Foreign); data-hygiene, not the routing bug.

---

## Revised fix backlog (supersedes scout §7)

Ranked by correctness impact, root-first:

| ID | Root / fix | Closes | Sev | Δ from scout |
|----|-----------|--------|-----|--------------|
| **F1** | Apply the foreign OL/HC drop on the **network/refresh** merge path too (or guard `merge_impl` per-field) | **#133** | **Crit** | ⬆ escalated — cached fix is incomplete |
| **F2** | Single `SeedBuilder` + real per-door language (kill 4× `"en"` hardcodes) | P1 divergence, #8, #53 (part) | High | unchanged |
| **F3** | Stop stamping language from **query intent** in `lookup_goodreads`; `provider_priority` normalize defensively | #11, 三体=es | High | ⬇ narrowed (#96 write-path dropped) |
| **F4** | COALESCE all anchor keys (`gr_key/hc/ol/isbn`) + defensive `cover_url` COALESCE | gr_key drop (82/120), latent cover null | High | unchanged + footgun added |
| **F5** | Per-field / per-provider conflict instead of whole-work block | #110, P5 | High | unchanged (#59 split out) |
| **F6** | Series reconcile: back-fill `series_id` from `series_name`; canonical (language-aware) series key; authoritative `book_count` | #58, #112, #111, #52 | High | ⬆ sharpened into 3 roots |
| **F7** | Wire `update_cover_dimensions` (read jpg dims); decide `metadata_source` (wire or delete + drop false foreign-skip gate) | #134, P8 | Med | unchanged |
| **F8** | god object split + delete dead `MetadataProvider` trait | S2/S4 | Med | unchanged, do last |
| — | **Product calls (not bugs):** (a) should held/unverified works get a cover? (#59 re-framed) · (b) is HC-for-foreign exclusion intended? | — | — | new — your decisions |
| — | **Cosmetic hygiene:** `'pending'`/`'exhausted'` string↔enum drift; vestigial `list_works_for_enrichment` query | — | Low | ⬇ R7 downgraded from High |

---

## What no longer needs work
- **R7 refresh-never-completes** — refuted; refresh enriches inline. (cosmetic string cleanup only)
- **#59 as a write-bug** — refuted; re-framed as a product decision about held works.
- **#96 region-tag write path** — refuted; writes are already normalized.

*Generated by 5 surgical deep-pass agents. The metadata path is now mapped well enough to spec the fix cycle.*
