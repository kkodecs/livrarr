---
feature: small-bugs-pass
stage: spec
status: delivered
version: 3
type: bugfix
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005]
---

# Bug Spec: small-bugs-pass

## Executive summary

Five small bugs were fixed together, one requirement each, with no database migration. The fixes
sit uncommitted on main `43b369c2` and passed code review on the second round. This spec now describes what
was built.

- **Pop-ups no longer show twice** (GitHub #183). Only one notification box is left, at the top of
  the app, with the dark bottom-right look. The setup screen and the reader and player still show
  notifications ([REQ-001](#2-requirements)).
- **Search, Queue and History see the whole library** (#180). Before, they read only the newest
  1,000 books, so Queue and History showed "Work #123" for older books. They now share one
  whole-library read, and the Missing page and the Works page's series view use the same
  page-by-page loader ([REQ-002](#2-requirements)).
- **Small usenet ebooks import on their own** (#178). The "files not fully synced" size check now
  runs only for torrent downloads. For usenet the advertised size includes repair files that
  unpacking removes ([REQ-003](#2-requirements)).
- **Undoing a list import removes the authors it left empty** (#182). This also covers list imports
  made before the fix. Authors that existed before the import, still have a book, or that the user
  monitors or has edited are kept ([REQ-004](#2-requirements)).
- **Two unfinished branches in provider error handling are filled** with the answers the rest of that
  code uses. No lookup could reach them ([REQ-005](#2-requirements)).

**Accepted limit.** If removing the authors fails after the books are already gone, undo still
finishes and only logs the error, so some empty authors may stay. They can be deleted by hand. The
PO accepted this on 2026-09-29 ([REQ-004](#2-requirements), `packet-2-code/DECISION.md`).

**Never checked in a browser or against a real download client.** Evidence is the automated tests
and code review. Nobody looked at the notifications in the real app, ran a real SABnzbd download or
opened a library of more than 1,000 books.

Sections: [system truths](#0b-system-truths), [problems](#1-problem-statement),
[requirements](#2-requirements), [decisions](#5-open-questions), [acceptance](#6-acceptance-criteria).

## Revision

v1, 2026-09-29: written from source at main `43b369c2` and GitHub issues #178, #180, #182, #183
(read-only). Code facts were read, not run; nothing was checked in a browser.

v2, 2026-09-29: PO decisions folded (`FOLD-PACKET.md`). Q-001 and Q-002 closed as recommended.
Q-003 replaced by the PO's rule for bug 4, as stated by the PM in the fold packet. REQ-004,
ST-013/014/016/017, ST-021/022, PA-003/004/006, the bug-4 non-requirement and AC-004 were
rewritten. v1 is kept at `build/reviews/small-bugs-pass/packet-0-spec/spec-small-bugs-pass-v1.md`.

v3, 2026-09-30: brought in line with the built change (`packet-2-code/code-r3.patch`, reviews
`packet-1-red-tests/REVIEW-astra-tests-r1.md`, `packet-2-code/REVIEW-astra-code-r1.md` and `-r2.md`,
`packet-2-code/DECISION.md`), per `FOLD-PACKET-v3.md`. Status `delivered`; every requirement is
marked Delivered. Changed: the executive summary; REQ-002, REQ-003 and REQ-004 as built;
AC-001 to AC-005 as built; ST-021's SQLite version; new ST-023 to ST-025. v2 is kept at
`build/reviews/small-bugs-pass/packet-0-spec/spec-small-bugs-pass-v2.md`.

## 0a. Design Principles

- Fix where the wrong data or behaviour is created; one definition per rule, shared by every door.
- Smallest change that fixes each bug; no new tables, settings or user-facing guardrails.
- Tests drive the real entry path and pin the named observable only.

## 0b. System Truths

ST-001 to ST-022 describe main `43b369c2` before the fix ("read" = code read; nothing executed).
ST-023 to ST-025 describe the built working tree on that commit.

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-001 | sonner 2.0.7: every `<Toaster>` without an `id` subscribes to the one global toast store and shows every toast that has no `toasterId`. | `frontend/node_modules/sonner/package.json:3`; `sonner/dist/index.mjs:916-927`, `:956-957` | Fixing duplicates by styling; two id-less containers. | read |
| ST-002 | Two containers are mounted: one at the app root outside all routes, one inside the main layout. | `frontend/src/App.tsx:444`; `frontend/src/components/Page/AppLayout.tsx:53-63` | — (the defect) | read |
| ST-003 | `/setup`, `/login`, `/read/:id` and `/listen/:id` render outside the main layout. Setup and the reader/player raise toasts; login raises none. | `App.tsx:107-145`, `:147-153`; `pages/setup/SetupPage.tsx:189-247`, `:431-440`, `:518-520`; `pages/reader/AudioPlayer.tsx:353`, `:795`; `pages/reader/EpubReader.tsx:464`; `pages/reader/useStreamUrl.ts:50` | Keeping only the layout container. | read; `rg` for `sonner` imports |
| ST-004 | The root container arrived in `cce973d1` (2026-06-09, "audiobook sync test"); the layout one dates from the first release. | `git log -S'<Toaster'` | — | read |
| ST-005 | `GET /work` clamps `page_size` to 1–1000. `listWorks` defaults to page 1, 1000 per page, newest first. | `crates/livrarr-handlers/src/types/pagination.rs:19-21`; `frontend/src/api/index.ts:156-172` | Treating one response as the library. | read |
| ST-006 | Search, Queue and History call bare `listWorks()` under the shared key `["works"]`. Queue and History name a book through `workName`, which falls back to `Work #<id>`; History shows a spinner until the list arrives. Search uses the list for its "in your library" matches and markers. | `pages/search/SearchPage.tsx:78-82`, `:85`, `:117-128`; `pages/activity/queue/QueuePage.tsx:49-53`, `:151`; `pages/activity/history/HistoryPage.tsx:103-111`, `:192`; `utils/works.ts:35-42` | — (the defect) | read |
| ST-007 | Two whole-library walks exist: Missing (refreshes the page count from each response) and the Works page's collapsed-series view (uses page 1's count). Both use keys under `["works", …]`. `MergeDialog` has no importer. | `pages/wanted/MissingPage.tsx:41-67`; `pages/works/WorksPage.tsx:198-235`; `rg MergeDialog` | A third copy of the walk. | read |
| ST-008 | An existing test pins the defect: it asserts the three pages make one bare `listWorks()` call and `["works"]` holds a single 1,000-item page. | `pages/wanted/MissingPage.test.tsx:548`, `:646-665` | Leaving that assertion as is. | read |
| ST-009 | `grab.size` is the indexer's advertised size (`<size>` or enclosure `length`) for torrent and usenet alike; the one production grab writer stores `Some(req.size)`. The grab has no protocol field; its download client's implementation gives the protocol. | `crates/livrarr-domain/src/torznab.rs:74-77`, `:171-174`; `crates/livrarr-handlers/src/release.rs:132`; `crates/livrarr-metadata/src/rss_sync_workflow.rs:640`; `crates/livrarr-download/src/release_service.rs:452-466`; `crates/livrarr-domain/src/entities.rs:600-619`; `crates/livrarr-domain/src/infra_config.rs:33-39` | Reading the protocol from anything but the grab's client. | read |
| ST-010 | The import check fails the grab when all files under the source total under 90% of `grab.size`. Its recorded purpose is "Remote seedbox rsync may not be complete". | `crates/livrarr-library/src/import_workflow.rs:1645-1665`, `:2500-2537`; `build/design/ir-consolidation.yaml:2959-2961` | Removing the check for torrents. | read |
| ST-011 | Both import doors run the same check: the poller (auto) and the manual retry. A failed import is retried 5 times, backing off 2, 4, 8, 16, 32 minutes. | `crates/livrarr-server/src/jobs/download_poller.rs:61`, `:807-841`; `crates/livrarr-server/src/import_service.rs:251`; `crates/livrarr-handlers/src/queue.rs:117`; `crates/livrarr-db/src/sqlite_grab.rs:408-424` | — | read |
| ST-012 | Livrarr starts a SABnzbd import only when SABnzbd history says `Completed` and the mapped local path exists. That SABnzbd reports `Completed` only after repair and unpack is SABnzbd behaviour, not verified here. | `download_poller.rs:426-455` | — | read; external fact inferred |
| ST-013 | A list import's `imports` row, with `started_at = Utc::now().to_rfc3339()`, is written on the first confirm batch before any row is processed; later batches reuse the row and are checked to be `running`. Preview creates no author (the only author creation in the service is in row processing). | `crates/livrarr-metadata/src/list_service.rs:502-535`, `:561`, `:227`; `crates/livrarr-db/src/sqlite_list_import.rs:88-102`; `migrations/017_readarr_import.sql:4-9` | Using the preview time or `completed_at` as the start. | read |
| ST-014 | Both production author writers set `added_at = Utc::now().to_rfc3339()` on create; the adoption branch returns the existing row unchanged. No production code rewrites `authors.added_at`. List-import authors carry no import mark (`import_id` is `None` from the list seed), and works are marked afterwards. | `crates/livrarr-db/src/sqlite_author_link.rs:2879`, `:2884-2913`; `crates/livrarr-db/src/sqlite_author.rs:437`, `:448-460`; `crates/livrarr-domain/src/seed.rs:152-172`; `list_service.rs:612-625`; `rg "added_at\s*="` over `crates/livrarr-db/src` (only a works insert, `identity_layer.rs:786`) | Selecting list-import authors by `import_id`. | read |
| ST-021 | chrono's RFC 3339 text has a variable-length fraction (none, 3, 6 or 9 digits), and other rows may use `Z`; SQLite `julianday()` reads all of these and keeps sub-second precision. `datetime()` truncates to whole seconds. Precedent: `datetime(a.added_at)` comparison at `identity_layer.rs:4568`. | sqlite3 3.45.1 probe on `:memory:`: `julianday('…58.123456789+00:00') >= julianday('…58.123+00:00')` = 1, `julianday('…58Z') = julianday('…58+00:00')` = 1; `crates/livrarr-db/src/identity_layer.rs:4568` | Comparing the two columns as plain text; `datetime()` (a same-second pre-existing author would count as new). | T in sqlite3 CLI 3.45.1. The app bundles SQLite 3.46.0 (`libsqlite3-sys-0.30.1/sqlite3/sqlite3.c:462`), not probed |
| ST-022 | A work names its authors in three places: `works.author_id`, `works.primary_author_id`, and `work_contributors` (several authors per work). Deleting a work removes the row and cascades its contributor rows. | `migrations/001_initial_schema.sql:59`; `migrations/082_identity_layer_foundation.sql:11`, `:55-63`; `crates/livrarr-db/src/identity_layer.rs:3598`, `:3846`, `:3905` | Reading only `author_id`. | read |
| ST-015 | Undo deletes only works; `WorkService::delete` removes the work, its files, cover and an empty series stub, never an author. Deleting a work cascades its contributor rows. | `list_service.rs:770-829`; `crates/livrarr-metadata/src/work_service.rs:1518-1576`; `migrations/082_identity_layer_foundation.sql:60` | — | read |
| ST-016 | `delete_orphan_authors_by_import` selects authors by `import_id` and keeps any with a work (`author_id` or `primary_author_id`), a contributor or series row, any monitoring field set, a user-picked or user-removed route, a user name variant, or a picked link candidate. Its selector has no `user_id` term; the guards are user-scoped through `a.user_id`. Readarr undo and rollback call it. New authors default to unmonitored. | `crates/livrarr-db/src/sqlite_import.rs:179-207`; `crates/livrarr-db/src/api/import.rs:48-49`; `crates/livrarr-server/src/readarr_import_workflow.rs:1286-1289`, `:2343-2349`; `migrations/001_initial_schema.sql:44-46` | A second copy of the guards. | read |
| ST-017 | The list undo route is `DELETE /api/v1/listimport/{import_id}`. Undo checks ownership by `(import_id, user_id)` and lists works by both. The list service's database bound lacks `ImportDb`. | `crates/livrarr-server/src/router.rs:542-545`, `:699`; `list_service.rs:342-349`, `:776-792`; `sqlite_list_import.rs:204-213`, `:344-356` | An author delete without a `user_id` term. | read |
| ST-018 | `NotConfigured`, `Retryable`, `Permanent` and `LayoutDrift` are produced only in `author_link.rs`, which has its own mapper. OpenLibrary and Audnexus produce only `RateLimited`, `Transient`, `NotFound`, `CircuitOpen`, `QueueFull` and `Other`; so the two `todo!()` arms are unreachable today. | `crates/livrarr-external-data/src/author_link.rs:1216-1228`; `openlibrary.rs:23-29`; `audnexus.rs:23-29`; `provider_client.rs:1518-1560`; `rg -c` per file with a known-hit control | Claiming a live crash. | read |
| ST-019 | In this file an unparseable response (`Other("parse: …")`) and an unexpected 4xx both map to `PermanentFailure { Unsupported }`; `Transient` maps to `WillRetry { ServerError }` after `retry_backoff_secs`. `PermanentFailureReason::InvalidResponse` has no producer anywhere. | `provider_client.rs:880-900`, `:1264-1284`; `openlibrary.rs:108`; `audnexus.rs:236`; `rg InvalidResponse` | A new reason with unknown downstream handling. | read |
| ST-020 | The Audnexus client is fixed to the real HTTP fetcher; the existing tests call the two mappers directly for every other variant. | `provider_client.rs:2604-2697` | A stub-HTTP test for the four arms (no response reaches them). | read |
| ST-023 | The bundled SQLite allows at most 32,766 bound variables per statement (the historical default was 999). The author delete binds three values plus its author ids, so one statement for every collected id fails on a very large import. | `~/.cargo/registry/src/*/libsqlite3-sys-0.30.1/sqlite3/sqlite3.c:13892-13894`; `packet-2-code/REVIEW-astra-code-r1.md:36` (finding C2) | One unbatched `IN (…)` list over all collected authors. | read; reviewer probe (`REVIEW-astra-code-r2.md:84`) |
| ST-024 | The app's query client marks data fresh for 30 seconds by default. | `frontend/src/App.tsx:69-77` | — | read |
| ST-025 | Four doors reach the import size check through `spawn_import` → import service → `ImportWorkflowImpl::import_grab`: completed qBittorrent, SABnzbd and Transmission downloads, and the automatic failed-import retry. The manual queue retry reaches the same method directly. | `crates/livrarr-server/src/jobs/download_poller.rs:297`, `:524`, `:750`, `:87`, `:807-841`; `crates/livrarr-handlers/src/queue.rs:117`; `crates/livrarr-server/src/import_service.rs:251`; `REVIEW-astra-code-r1.md:89-95` | Deciding by anything other than the grab's client. | read |

## 0c. Prior Art

Searched `wiki/insights/`, `wiki/architecture/import-pipeline.md`, `build/design/`, `~/Projects/kk-build/build/state/`, `docs/`.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | Insight 81, `wiki/insights/data-and-state.md:56-64`; issue #177 fix in `MissingPage.tsx:41-67` | Walk every page in one query, refresh the page count from each response, check abort, key under `["works", …]`. REQ-002 reuses it. |
| PA-002 | `ir-consolidation.yaml:2959-2961` (`file_size_precheck`) | The only recorded reason for the size check: a remote seedbox copy may be incomplete. REQ-003 keeps it for torrents. |
| PA-003 | `delete_orphan_authors_by_import` (ST-016) and the Readarr undo | The existing "empty and untouched author" guards. REQ-004 shares them; only the selector differs. |
| PA-004 | `build/state/retro-bugfix-readarr-import-undo-orphans.md` | About orphaned files after Readarr undo, not authors. No bearing on the list-undo selector. |
| PA-005 | Insight 33, `wiki/insights/providers-and-transport.md:51-56` | Map provider errors at the adapter and keep real distinctions. REQ-005 maps each variant explicitly. |
| PA-006 | Insight 67, `wiki/insights/identity.md:77-83` | One adoption gate over the user's authors. An adopted author keeps its original `added_at`, so REQ-004's time test keeps it. |

## 1. Problem Statement

1. **Pop-ups (#183).** Starting condition: any main page. Action: anything that shows a
   notification, e.g. adding a root folder. Expected: one notification. Observed (reporter's
   screenshot): two, one list at bottom-right and one stack at bottom-centre.
2. **Library truncation (#180).** Starting condition: more than 1,000 books. Action: open Queue or
   History for a download of an older book, or search for an older book's title. Expected: its title
   in Queue/History, and Search shows it as already in the library. Observed (from code, ST-006):
   "Work #123", and Search lists it only as a new result.
3. **Usenet size check (#178).** Starting condition: SABnzbd client; a grab whose ebook is much
   smaller than the advertised release. Action: SABnzbd finishes. Expected: the ebook imports.
   Observed (reporter): `files not fully synced: local 0.2MB vs expected 8.3MB`, five retries, then
   import failed for good.
4. **List undo (#182).** Starting condition: a CSV list import that created new authors. Action:
   undo the import. Expected: its books and the authors it created are gone. Observed (reporter,
   59-row import): books gone, authors left with no books.
5. **Unfinished branches.** `audnexus_error_outcome` and `ol_error_outcome` panic on four error
   kinds (`provider_client.rs:881-884`, `:1265-1268`). Not reachable today (ST-018).

## 2. Requirements

- **REQ-001** (Delivered): Exactly one notification container is mounted: the root one in `App.tsx`, taking the
  layout container's settings (dark theme, bottom-right, 5 visible, gap 8, expanded, close button,
  the same toast class). The container in `AppLayout.tsx` is removed. Every screen, inside or outside
  the layout (ST-003), shows each notification once. Test door: mount `App` for real with only the
  `@/api` network boundary stubbed. Built: `frontend/src/App.tsx:444-454`; the layout container is
  removed from `AppLayout.tsx`.
- **REQ-002** (Delivered): Search, Queue and History read the whole library. The page walk exists
  once: `listAllWorks(params, signal)` in `frontend/src/hooks/useLibraryWorks.ts:20-34` (pages of
  1,000, page count refreshed from each response, abort checked between pages; PA-001). Search,
  Queue and History read it through the hook `useLibraryWorks()` (`useLibraryWorks.ts:40-45`) under
  the key `["works", "library"]` (`SearchPage.tsx:79`, `QueuePage.tsx:50`, `HistoryPage.tsx:104`).
  That read takes the app's 30-second freshness default (ST-024). Missing (`MissingPage.tsx:44-48`)
  and the Works page's collapsed view (`WorksPage.tsx:205-219`) call `listAllWorks` under their own
  keys and keep their own 60-second freshness. Nothing reads or writes the bare `["works"]` entry any
  more. Cost: 5 sequential requests on a 5,000-book library, the same walk Missing already did; not
  measured. Test door: real component mount with `listWorks` stubbed to return 1,500 books over two
  pages.
- **REQ-003** (Delivered): The 90% size comparison runs only when the grab's download client is a
  torrent client (ST-009). Usenet grabs skip it and import what is on disk. If the client row cannot
  be read, the check runs as before and a warning is logged. Built at
  `crates/livrarr-library/src/import_workflow.rs:2505-2518`. Torrent behaviour, the error text and
  the retry schedule are unchanged. All five doors reach this one check (ST-025): completed
  qBittorrent, SABnzbd and Transmission downloads, the automatic retry, and the manual queue retry.
  Test door: `ImportWorkflowImpl::import_grab` over the real `SqliteDb`, as the existing size test
  does.
- **REQ-004** (Delivered): Rule from the PO, as stated by the PM in `FOLD-PACKET.md`. No author is
  marked at creation.
  1. Before deleting works, undo collects the authors of the works it will remove: `author_id`,
     `primary_author_id` and every `work_contributors` author (ST-022), for this user only
     (`list_authors_of_import_works`, `crates/livrarr-db/src/sqlite_import.rs:192-214`). If this
     lookup fails, undo returns the error before anything is removed
     (`crates/livrarr-metadata/src/list_service.rs:794-801`).
  2. After the works are deleted, it deletes each collected author that belongs to the undoing user
     and was added at or after the import began:
     `julianday(a.added_at) >= julianday(imports.started_at)` for this import and user (ST-013,
     ST-014, ST-021). Built as `delete_empty_authors_added_since_import`
     (`sqlite_import.rs:216-250`), called at `list_service.rs:821-833`.
  3. The same statement applies the "empty and untouched" keep-checks (ST-016). They are written
     once, as `EMPTY_UNTOUCHED_AUTHOR` (`sqlite_import.rs:261`), and used by both the Readarr delete
     (`sqlite_import.rs:181-190`, still selecting by `import_id`) and the list-undo delete.
  4. The delete runs in batches of 500 author ids (`AUTHOR_ID_BATCH`, `sqlite_import.rs:255`), each
     batch using the same statement and keep-checks. Each statement binds 503 values, under both
     limits in ST-023. Removed counts are summed over batches.
  Consequences: undo also cleans up after list imports made before this fix, since nothing depends
  on a mark. An author added after the import began but not by it (e.g. the user added it, the
  import then reused it) is deleted too if it ends empty and untouched; the PO asked for a full
  clean-up. Undo's response shape is unchanged. Test door: the real confirm and undo routes through
  the real router and `SqliteDb`.

  **Accepted limit** (PO, 2026-09-29, `build/reviews/small-bugs-pass/packet-2-code/DECISION.md`;
  review finding C1): if the author delete fails after the works are gone, undo logs a warning and
  still completes and marks the import undone (`list_service.rs:821-839`). The batches are not
  inside one transaction, so a failure part-way can leave some empty authors from this import. They
  can be deleted by hand; the user is not told.
- **REQ-005** (Delivered): The four arms return explicit outcomes, the same in both functions:

  | Variant | Outcome | Basis |
  |---|---|---|
  | `NotConfigured` | `PermanentFailure { Unsupported }` | Keyless provider; doc comment forbids `NotConfigured` (`:869-875`, `:1252-1259`). |
  | `Retryable { retry_not_before, .. }` | `WillRetry { ServerError }` at `retry_not_before`, else now + `retry_backoff_secs` | Same as `Transient`; keeps the provider's wait (insight 33). |
  | `Permanent(_)` | `PermanentFailure { Unsupported }` | Same as an unexpected 4xx (`Other`). |
  | `LayoutDrift(_)` | `PermanentFailure { Unsupported }` | Same as a parse failure (ST-019). |

  Test door: the existing mapper tests (ST-020); no HTTP response reaches these arms. Built at
  `crates/livrarr-external-data/src/provider_client.rs:876-908` and `:1268-1300`; no `todo!()`
  remains in the file.

## 3. UI/Interface Design

REQ-001: reader, player and setup notifications move from bottom-centre to bottom-right and take
the dark style. REQ-002: Queue and History may wait longer on a big library before showing rows.
No other visible change.

## 4. Non-Requirements

- No change to what `GET /work` returns, no by-id lookup endpoint, no change to `MergeDialog`.
- No new size heuristic for usenet, no "wait until sizes stop changing" mechanism.
- No author count in the list-undo reply; no import mark written on list-import authors.
- No change to how `author_link.rs` maps the same four variants.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | D1, size-check rule (REQ-003). Options: (a) skip for usenet, keep for torrents; (b) lower the 90% bar; (c) compare against a size SABnzbd reports; (d) replace the check with a "size stopped changing" wait for both protocols. | closed — PO 2026-09-29: "fine keep it / fix it" | Option (a). (b) cannot work: the reporter's book is 2% of the release. (c) relies on an unverified SABnzbd field. (d) is a new mechanism, over the small-fix bar. (a) is about an hour. What changes for the user: small usenet books import at once. A usenet folder still being copied by an outside sync tool could be imported half-copied; no such sync feature exists in Livrarr (`rg -i "sftp\|seedbox"` over `crates`: no hits). |
| Q-002 | D2, whole library or lookup by id (REQ-002). Options: (a) fetch all pages via one shared helper; (b) look books up by id. | closed — PO 2026-09-29: "good" | Option (a); Missing and the Works collapsed view share the helper. (b) fixes only Queue and History: Search needs every title and author, and there is no batch endpoint, so (b) means up to 25 single lookups per Queue page or a new server route. (a) is 5 requests on 5,000 books, the walk Missing already runs. About 2–3 hours, including rewriting ST-008's test. |
| Q-003 | D3, how undo knows which authors to remove (REQ-004). v1 recommended marking authors at creation. | closed — PO 2026-09-29: "we should clean up fully if we undo an import"; monitored or edited authors: "keep them" | Rule restated by the PM in `FOLD-PACKET.md`: authors of the removed works, now empty, added at or after the import began, with the shared guards. No marking and no migration; covers imports made before the fix. About 2–3 hours. |
| Q-004 | Which notification look to keep (REQ-001). | closed — spec | The layout look: it is on every main page and older (ST-004). |

## 6. Acceptance Criteria

As built. "Red" means the case failed on `43b369c2` before the fix (logs under
`build/reviews/small-bugs-pass/packet-1-red-tests/`); "guard" means it passed before the fix and
pins behaviour the fix must keep.

- [x] **AC-001** (REQ-001): `frontend/src/App.toaster.test.tsx`. "shows a notification once on a
  main page" (`:113`, red): mount `App` authenticated at `/`, call `toast("probe")`, and expect
  exactly one element with text "probe" and exactly one `[data-sonner-toaster]`. "shows a
  notification once on the setup screen" (`:130`, guard): at `/setup` with setup required, one
  "probe".
- [x] **AC-002** (REQ-002): in `frontend/src/pages/wanted/MissingPage.test.tsx`, a 1,500-book
  library over two pages where the target book is only on page 2. One cold-start case per page,
  each red: "History opened first names a book that is only on page 2" (`:635`), "Queue opened
  first …" (`:649`), "Search opened first lists a book that is only on page 2 in the library"
  (`:663`). One shared case, "History, Queue and Search share a whole-library read after Missing"
  (`:680`, red), which also pins that the bare `["works"]` entry holds no data (`:725`); it replaces the
  old assertions that pinned the defect (ST-008). The older History test in
  `frontend/src/pages/activity/history/HistoryPage.test.tsx:17` now loads through the `listWorks`
  stub instead of pre-filling the `["works"]` cache.
- [x] **AC-003** (REQ-003): `test_import_grab_usenet_skips_size_precheck`
  (`tests/behavioral/test_consolidation_import_workflow.rs:542`, red): a SABnzbd client,
  `create_source_dir(&["book.epub"])` (23 bytes) and `grab.size = 1000`; `final_status` is
  `Imported` with one imported file. The torrent case `test_import_grab_partial_sync_rejected`
  (`:498`) is unchanged and green.
- [x] **AC-004** (REQ-004): in `tests/behavioral/test_ilr_contracts.rs`, through the real router
  harness (`build_route_harness`), preview through the list service, then
  `POST /api/v1/listimport/confirm` and `DELETE /api/v1/listimport/{id}`. Each case pins whether the
  author row exists after undo.
  1. `list_undo_keeps_author_that_existed_before_the_import` (`:7302`, guard): an author added
     through `POST /api/v1/author` and reused by an import row remains.
  2. `list_undo_keeps_empty_author_added_before_the_import_began` (`:7368`, guard): isolates the
     date rule. An earlier list import creates the author and is completed. Its book is deleted
     through `DELETE /api/v1/work/{id}`, leaving the author empty and unprotected. The target import
     reuses the author, and the author survives that import's undo.
  3. `list_undo_removes_author_the_import_created` (`:7463`, red): the author has no import mark
     after confirm and is gone after undo. This is also the "older imports" case: the fixed confirm
     writes no mark, so its imports look like pre-fix ones.
  4. `list_undo_keeps_import_author_with_a_work_added_outside_the_import` (`:7496`, guard): remains.
  5. `list_undo_keeps_import_author_the_user_monitors` (`:7545`, guard): the user turns on "monitor
     new items" through `PUT /api/v1/author/{id}`, and the author remains. Plain `monitored: true`
     is refused for an author with no OpenLibrary link (`crates/livrarr-metadata/src/author_service.rs:593-610`),
     which a list-import author without a provider match does not have.
  Batch coverage is a unit test in `crates/livrarr-db/src/sqlite_import.rs` (`:282-384`, `empty_author_cleanup_accepts_more_candidates_than_sqlite_binds`) with more
  than 32,766 candidate ids.
- [x] **AC-005** (REQ-005): `audnexus_error_outcome_classifies_every_variant` and
  `ol_error_outcome_classifies_every_variant` (`provider_client.rs:2711`, `:2753`, red) cover the
  four variants, including `retry_not_before` being honoured (helper at `:2635-2690`).
- [x] **AC-006**: checks as recorded by the PM in `build/reviews/small-bugs-pass/packet-2-code/`:
  `pm-fold-r3.log` (fmt exit 0, clippy exit 0, `livrarr-db` tests 73 passed), `pm-frontend-full-r3.log`
  (26 files, 192 tests passed) and `pm-rust-full-r2.log` (full Rust suite on the r2 patch). The
  typecheck result is reported in `REVIEW-astra-code-r1.md:127`, not in these logs.
