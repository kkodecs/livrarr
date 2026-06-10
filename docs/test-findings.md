# Test Findings — metadata-modularization + WCC (pre-push)

**Session:** 2026-06-06 · **Branch:** `wcc-stage5-green` @ `2cf112d` (merged, **NOT pushed**)
**Tester:** PO (manual) · **Verdict: DO NOT PUSH until triage + manual-import fixes.**

## What's solid (the merge's own deliverables tested clean)
- Crate split is behavior-preserving: §1 smoke — existing works, **ebook reader**, **audio playback**, release search, authors, series all good.
- **Search add + trust-the-pick** → lands **Confirmed** (zero re-lookup). Multisource results, Goodreads ratings, Filtered/Raw all work.
- **Status-badge UI** (Identity + Details two-section split, tooltips, Confirmed/Enriched) good.

## Findings

| # | Area | Severity | Merge vs pre-existing? | Summary | To check / fix |
|---|------|----------|------------------------|---------|----------------|
| 1 | Covers | Cosmetic | pre-existing | ~38 cover 404s on grid (works 1630–1667); placeholders render | ✅ **FIXED 2026-06-07** — same root as #12 (relative cover URL); resolved at source + DB backstop + backfill now nulls bad rows |
| 2 | Audiobook player | Med | unknown (maybe pre-merge) | Audio plays, but **chapter list/markers missing** | Chapter fetch endpoint + `audiobook_chapters` |
| 3 | Search results | Cosmetic | merge-area | Some filtered results lack a **source chip** | Why `source` is null on some results |
| 4 | — | — | — | ~~Live-refresh~~ → **RESOLVED** (expected 3s poll) | — |
| 5 | Manual import | Cosmetic | merge-area | "Matching against OpenLibrary" label understates it (**GB+OL**) | Fix label |
| 6 | Manual import (#97) | **High** | mostly GB-429 confound + brittle matcher | Auto-match **abstains** → falls back to parsed-from-filename guesses | Exact-match-only by design + GB-down removed candidates/ISBN signal. Unify title normalization + language-gated fuzzy tier (see Cluster Plan) |
| 7 | Manual import (#97) | **High** | real bug (grouping/display), **NOT traversal** | ~~Scan not recursive~~ — scan **IS** recursive (`manual_import.rs:1155`). Real: multi-CD audiobooks split across sub-subdirs mis-group; bare-filename rows make a deep tree look undescended | Group by reconciled (author,title); show scan-root-relative path labels (see Cluster Plan) |
| 8 | Manual import (#97) | **High** | **2 real bugs (GB-independent)** | German *Das Parfum* (.m4b) → **English** edition | (1) **Audiobook language never extracted** — `extract_m4b`/`extract_mp3` hardcode `language:None` (`m1_embedded.rs:122`); only EPUB reads `dc:language` → .m4b/.mp3 default "en". (2) Selector is **language-blind** (`work_service.rs:1620`). Fix both (see Cluster Plan) |
| 9 | Manual import (#97) | Med | real gap (no cover UI) + GB-429 worsens default | **No cover UI at all**; uses the match's cover (GB down → OL, weakest) | Best-source cover ranking auto-applied (S) + optional per-row picker (M) (see Cluster Plan) |
| 11 | Enrichment quality | **High** | merge-area | **LLM Goodreads resolver returns WRONG book** for foreign titles (Kongres Futurologiczny → German Böll book `gr_key=136426`) | LLM GR-key resolver validation for foreign |
| 12 | Covers | **High** | pre-existing (logs since May 24) | Cover download **SSRF: "relative URL without a base"** (work 1775) | ✅ **FIXED 2026-06-07** — root: Goodreads LLM-fallback (`extract_with_llm`) + search/autocomplete passed empty base, so relative covers never resolved. Now resolved against page/GR base. Backstop: `absolute_http_cover_url` guard at all DB cover-write binds (`sqlite_common.rs`/`sqlite_work.rs`) → non-absolute ⇒ NULL. Backfill job validates + nulls bad rows (kills daily spam). Gates green (build/clippy/fmt/test 770). NOT committed/pushed. Applies to foreign too; foreign covers repopulate once GB quota resets |
| obs | Enrichment | Triage | unknown | ~130 works skipped as **"identity conflict detected"** | Legit bulk-import conflicts vs over-detection? |

## Research shelf
- **R1 — Priority metadata queue + budget-aware throttle.** Google Books is **429 across the board** (key IS set; ~1 req/sec backlog blows the ~1,000/day free-tier cap in ~17 min). Need a single funnel for all provider calls, priority insertion (foreground > background), per-provider qps + **daily budget**, reason-aware 429 backoff (we currently log only `status=429`, not the reason), and volume reduction (cache reuse / dedupe / skip already-enriched). Subsumes the old Finding #10. **Not now** per PO.
  - **Testing caveat:** while GB quota is exhausted, **foreign-language enrichment cannot succeed** — §5 / #8 / #11 data is confounded until quota resets.
  - **Root cause (verified by sub-agent, 2026-06-06):** the `enrichment_retry` job (5-min tick, `livrarr-server/src/jobs/`) re-dispatches **non-English** works to GB (English works skip GB). Each GB 429 sets `next_attempt_at = now+60s` (`livrarr-external-data/src/google_books.rs:500-503`) → works re-due every minute → perpetual ~1 req/s. A GCRA limiter (1 rps) + in-memory circuit breaker only pace it; `max_attempts:5` makes it self-decay to terminal over *hours*. No config flag disables the job.
  - **Immediate levers (no code change):** (a) clear the Google Books API key via the app's Settings save → GB short-circuits to `NotConfigured` (terminal, zero network), other providers keep running, no restart, reversible; or (b) stop the server (don't restart with key set until quota resets). Do NOT clear `provider_retry_state` (resets the attempts counter → prolongs churn).
  - **Durable quick-fix (small, pre-R1):** GB 429 backoff in `google_books.rs:500-503` from `now+60s` → ~12h / next-midnight-Pacific; raise breaker `open_duration_secs` (`main.rs:218-223`); add a global "GB quota-exhausted until T" flag so one 429 pauses all GB works. Collapses steady-state from ~1/s to a few/day.

## Recommended next step
1. **Triage** each finding: merge-regression vs pre-existing/environmental (decides what actually blocks *this* push).
2. **Fix the manual-import cluster (#6/#7/#8/#9)** — the #97 headline this merge exists to deliver.
3. ~~Fix the cover bug (#12 → #1)~~ ✅ **DONE 2026-06-07** (uncommitted). Still: **#11** (wrong-book foreign data).
4. Cosmetics (#3/#5) and #2 (chapters) as cleanup.
5. R1 as a separate design effort.

## Manual-import cluster — diagnosis & plan (2026-06-07)
*From sub-agent research (Serena + code-index) + the audiobook-language trace. Confidence ~85% (read from code); GB-quota re-test confirms the rest.*

**Confound:** Google Books was quota-exhausted during testing, so the matcher (which queries GB+OL together) only saw OpenLibrary. This inflated #6, part of #8, and #9. **P0 = re-test once GB resets** before sizing effort.

**#6 — abstains (mostly confound).** `best_candidate_index` (`work_dedup.rs:113-141`) is exact-match-only *by design* (no fuzzy). GB-down removed half the candidates + killed the ISBN fast-path (`work_service.rs:1628`, GB-only ISBNs) → abstain. Residual real bug: title normalization is inconsistent across 3 call sites. **Fix (cross-cutting w/ #8):** unify title normalization + add a graded fuzzy tier, gated by the language guard.

**#7 — NOT recursion.** Scan is fully recursive (`manual_import.rs:1155-1206`). Real bugs: (a) multi-CD audiobooks split across sub-subfolders (`/Book/CD1/`, `/CD2/`) group by *immediate* parent → 2 groups instead of 1; (b) rows show bare filenames (no folder context) → deep trees look undescended. **Fix:** group by reconciled (author,title) instead of immediate parent; show scan-root-relative path labels.

**#8 — language wrong (TWO stacked, both real, GB-independent).** (1) Audiobook language never extracted — `extract_m4b`/`extract_mp3` hardcode `language:None` (`m1_embedded.rs:122`); only EPUB reads `dc:language` → `.m4b`/`.mp3` default to "en" → foreign audiobooks match English **even with GB up**. (2) Selector is language-blind — `best_candidate_index` ranks title+author only (`work_service.rs:1620`), and the anchor-graft too. **Fix:** (1) read audiobook language (MP4 language atom / id3 `TLAN` frame) in `m1_embedded`; (2) thread language into selection + anchor-graft with same-language preference.

**#9 — no cover UI.** Manual import renders no cover picker/thumbnail; forwards the match's cover (GB down → OL, weakest). **Fix:** best-source cover ranking auto-applied (S; benefits all import paths) + optional per-row "change cover" override (M).

**OPEN DECISION — language filter: HARD vs SOFT.** Hard = if no same-language edition, abstain (manual search); never wrong-language. Soft = prefer same-language but fall back to English (re-risks the *Perfume* bug for thin foreign corpora). **Recommendation: HARD** (a recoverable "no match" beats a silent wrong-edition import).

### Prioritized plan
1. **P0 — re-test with GB quota restored** (~0 code; gates effort on #6/#8/#9).
2. **P1 — cross-cutting matcher fix:** unify normalization + language-aware selection + language-gated fuzzy tier (fixes residual #8-layer-2 + most of #6).
3. **Audiobook language extraction** (`m1_embedded.rs`) — required so foreign audiobooks even have a language signal (#8-layer-1).
4. **#7 grouping** by reconciled (author,title) + relative-path row labels.
5. **#9 best-source cover ranking** (+ optional per-row picker later).
