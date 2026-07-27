---
feature: "bugfix-177-missing-page"
stage: spec
status: draft
version: 2
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005]
---

# Spec: bugfix-177-missing-page

GitHub issue: kkodecs/livrarr#177 — "Missing page always shows 'No missing items' on libraries larger than the default page size"

Revision note (v2, 2026-07-27): folds review round 1 — codex R-001/R-002, grok P1×2 + P2×2 + P3×3. AC-001/002 rewritten for per-media-type correctness; REQ-004 (badges) and REQ-005 (sibling cache non-regression) added; #180 recorded; citations corrected.

## 0a. Design Principles

Choices you're committing to. If a requirement conflicts, the principle wins.

- **Completeness over speed.** The Missing page's entire value is a complete answer on large libraries — the libraries that need it most (issue reporter: 7,166 works). A fast partial answer is a silent lie; a slower complete answer is the product.
- **"Missing" (no file) ≠ "wanted" (monitored)** — wiki insight 17. Missing is a **per-media-type** predicate: a work is missing a type iff it is monitored for that type AND lacks a library item of that type. Per-media-type monitoring stays independent (insight 21); one monitored type having its file never clears the other monitored type's missing status.
- **Surgical bugfix.** Fix the Missing page's data acquisition and its per-type presentation correctness. Sibling pages with the same latent fetch pattern are tracked in issue #180, not ridden along.

## 0b. System Truths

Facts about the environment you don't control. Each truth needs all four fields.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `crates/livrarr-handlers/src/types/pagination.rs:20` (`unwrap_or(100).clamp(1, 1000)`) bound to the route by `crates/livrarr-handlers/src/work.rs:356-371` (`list` → `list_paginated(… pq.page_size() …)`), both read 2026-07-27 | `GET /work` is paginated: server default page_size 100, hard cap 1000. One request never returns more than 1000 works. | Treating a single `listWorks()` response as "all works" on any library that can exceed 1000 | high |
| ST-002 | `frontend/src/api/index.ts:167-170`, read 2026-07-27 | `listWorks()` with no args requests page 1, page_size 1000, sorted `date_added desc` — i.e. the 1000 most-recently-added works | Assuming default ordering ever surfaces older works; assuming the issue's "~100 items" figure (client overrides the server default to 1000 — symptom identical, threshold different) | high |
| ST-003 | Issue #177 report (7,166 works, ~944 authors, CSV list import) | Real deployments exceed the 1000-work page cap several times over | Sizing tests or fix reasoning to "just over one page" only — the walk must scale to ~8 pages | medium (external report, not sampled locally) |

**System-Truths check (Bug Spec item 5):** yes — this bug existed because ST-001/ST-002 were nowhere recorded; consumers treated one page as the library. Recorded here; candidate for promotion to wiki insights at close.

## 1. Problem Statement

**Observed (verified at source 2026-07-27, all lines 1-based):**

1. `frontend/src/pages/wanted/MissingPage.tsx:40-44` fetches the works list once: `queryKey: ["works"]`, `queryFn: () => listWorks()` — no pagination params.
2. `frontend/src/api/index.ts:168` defaults that call to `page=1&page_size=1000`, sorted `date_added desc` (`:169-170`).
3. The server road (`crates/livrarr-handlers/src/work.rs:356` `list` → `WorkService::list_paginated`) honors page_size up to the 1000 cap (`pagination.rs:20`) and offers only `media_type`/`language` filters — there is no monitored/has-file filter server-side.
4. `MissingPage.tsx:51-79` then filters that single page client-side: the monitor gate and per-tab combination live in the `switch` at `:53-66` (`monitorEbook && isMissingEbook`, `monitorAudiobook && isMissingAudiobook`, OR of both for All); the per-type file checks are the helpers at `:26-32`, which look only at `libraryItems` of that media type. The tab badge counts are computed from the same single page at `:91-101`.

**Consequence:** any monitored, missing work outside the 1000 most-recently-added works can never appear. On the reporter's 7,166-work library, ~6,166 works are invisible to the page; with the missing works all falling outside the window, the page renders the `"No missing items"` empty state (`MissingPage.tsx:169`) — a false "all clear" exactly on the libraries the feature exists for.

**Issue hypothesis vs source:** the reporter's mechanism (client-side filter over one fetched page) is confirmed. The reported "~100 items" page size is wrong — the client explicitly sends 1000 (the server's default of 100 applies only when no param is sent). Bug threshold: >1000 works, not >100.

**Adjacent defect on the same page (review r1, codex R-001, verified at source):** the per-row badge renderer `MissingBadges` (`MissingPage.tsx:274-300`) labels a media type missing on `isMissingEbook`/`isMissingAudiobook` alone (`:275-276`, `:280`, `:289`) — it never checks `monitorEbook`/`monitorAudiobook`. A work monitored only for audiobook with no files at all shows a red "Ebook" missing badge for a format the user never asked for. Same product promise ("what this page claims is missing"), so fixed here (REQ-004).

**Shared-cache seam (review r1, grok, verified at source):** MissingPage's query key `["works"]` is shared verbatim by `SearchPage.tsx:79-82`, `QueuePage.tsx:49-53`, and `HistoryPage.tsx:103-107`, all expecting the paginated single-page response shape. The WorksPage all-pages walk deliberately avoids that collision: distinct key `["works", "all", …]` returning a raw items array, `"works"`-prefixed so existing invalidations still cover it (`WorksPage.tsx:202-204`). A fix that changed the `["works"]` entry's shape or semantics would break the three siblings — hence REQ-005.

**In-repo precedent:** `frontend/src/pages/works/WorksPage.tsx:204-231` already solves the completeness problem for series-collapse mode — a sequential all-pages walk (`page: 1..computeTotalPages(first.total, 1000)`) with abort handling and `staleTime: 60_000`. Production-proven at this library scale.

**Same latent class elsewhere (out of scope — tracked in kkodecs/livrarr#180):** bare `listWorks()` at `SearchPage.tsx:80`, `QueuePage.tsx:51`, `HistoryPage.tsx:105`; capped `listWorks({ pageSize: 1000 })` at `MergeDialog.tsx:25`.

**Affected REQ-IDs (Bug Spec item 3):** none traceable — the Wanted/Missing page predates the REQ-ID regime (no spec exists for it; only `spec-work-history.md` at root, unrelated).

## 2. Requirements

- **REQ-001**: The Missing page lists **every** work that is monitored for at least one media type and lacks a library item of **any** media type it is monitored for — regardless of library size and regardless of the work's position in any list ordering. Presence of a file for one monitored type does not clear the missing status of the other monitored type.
- **REQ-002**: The Missing page's tab badge counts (all / ebook / audiobook) are computed over the **complete** library under the same per-type rule.
- **REQ-003**: The page never presents a partial answer as final: the `"No missing items"` empty state (and any rendered missing-list treated as complete) may appear only after the complete works data has been fetched. Until then the page shows its loading state. Fetch failure keeps showing the existing error state (with retry), never a false empty state.
- **REQ-004**: A listed row's missing badges name **exactly** the media types that are both monitored and missing for that work — never an unmonitored type, never a type whose file is present.
- **REQ-005**: The fix does not change what sibling consumers of the shared works data receive: Search, Queue, and History (today's `["works"]`-key consumers) keep their current behavior and response shape.

## 3. UI/Interface Design

No UI changes — the existing page layout, tabs, loading, error, and empty states are reused.

## 4. Non-Requirements

Explicit scope exclusions.

- **No new or changed API surface.** No server-side monitored/has-file filter, no dedicated `/api/wanted/missing` endpoint (the issue's suggested server-side fix is declined at bugfix tier — it is a public-surface change). If Missing-page latency at large scale becomes a real complaint, a server-side wanted/missing query is a future feature.
- **Sibling pages are not fixed here** — tracked in **kkodecs/livrarr#180** (SearchPage, QueuePage, HistoryPage, MergeDialog).
- **No performance work on `GET /work` itself.**
- **Empty-state copy stays as-is** — distinguishing "library has no works" from "all monitored media present" (review r1 gap note) is a UX nicety outside this bug, declined.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Fix approach: client walks all pages (WorksPage precedent) vs server-side filter | resolved | Copy the proven WorksPage all-pages walk (P10 lightest; performance envelope already accepted in production at this scale; server-side = public-surface change, wrong tier for a bugfix). PM call under delegated internals, PO-approved with the spec 2026-07-27. |
| Q-002 | Where to track the sibling pages sharing the bare-`listWorks()` pattern? | resolved | Filed as **kkodecs/livrarr#180** (2026-07-27, PO-directed). Scoped out of this fix per §4. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): With total works spanning multiple pages (e.g. 2,500 works / 3 pages of 1,000), a work that is monitored for ebook only, **owns an audiobook library item but no ebook item**, and sits **only on the last page**, is listed — under both the All and Ebook tabs.
- [ ] **AC-002** (REQ-001): Per-type exclusion is exact: (a) a dual-monitored work with an ebook item present and no audiobook item **is listed** (under All and Audiobook, not under Ebook); (b) a work whose every monitored type has its item is not listed; (c) a work monitored for neither type is never listed, whatever items it has.
- [ ] **AC-003** (REQ-002): Badge counts equal the true library-wide totals with qualifying works spread across non-adjacent pages (e.g. pages 1 and 3), and follow the per-type rule: the AC-002(a) work increments the All and Audiobook counts but not Ebook.
- [ ] **AC-004** (REQ-003): While any page of the walk is still unfetched, the loading state shows; the `"No missing items"` empty state never renders before the walk completes. If a page fetch fails, the error state (with retry) shows instead of an empty or partial-complete answer.
- [ ] **AC-005** (REQ-001): A single-page library (≤1,000 works) behaves exactly as today — same list, same counts (no regression at small scale).
- [ ] **AC-006** (REQ-004): Badges name only monitored-and-missing types: a work monitored for audiobook only, with no files at all, shows the Audiobook badge and **no Ebook badge**; the AC-002(a) dual-monitored work shows only the Audiobook badge.
- [ ] **AC-007** (REQ-005): With the fix applied, Search, Queue, and History receive the same works data they receive today on a ≤1,000-work library (no errors, unchanged rendering), and the Missing page's complete fetch does not replace or reshape the cache entry those pages read.
