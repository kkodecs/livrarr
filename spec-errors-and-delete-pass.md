---
feature: errors-and-delete-pass
stage: spec
status: delivered
version: 6
type: bugfix
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-101, REQ-102, REQ-103, REQ-105, REQ-106, REQ-107, REQ-108]
---

# Bug Spec: errors-and-delete-pass

## Executive summary

**Delivered.** Every requirement below was built, passed code review on the second round, and was
checked live by the PO on 2026-09-30. Nothing was deferred. This spec now describes what was built
([as built](#7-as-built-corrections-and-limits)).

**Recorded limits.** Opening a PDF after Retry, and saving your place in a PDF, are not proven end
to end; the PO put them out of scope, and the PDF save test is still marked `fixme`. The
check-then-remove race stays accepted ([§4](#4-non-requirements)). How the save warning behaves in
a hidden browser tab is untested ([limits](#limits)).

Two parts, approved by the PO on 2026-09-30, plus two additions the PO approved the same day ("yes
to both, add them to this batch"). Two coders built in parallel in one checkout: **Stream A** is
frontend only; **Stream B** holds every Rust change plus the delete screens. They touched no common
file ([build streams](#build-streams)).

**Part A: eight places where something fails and the user is told nothing.** Each gets exactly one
message, in the form the app already uses. When loading something fails, an error with a Retry
button appears where the content would have been. When an action fails, a red pop-up appears.
Saving your place in a book warns once when a save fails, adds nothing for further failures, and
clears when a later save succeeds; it does not promise to retry. If a new run of failures starts
while the previous warning is still leaving the screen, its warning appears once the old one has
gone. Readarr import progress, which polls by itself, shows one warning line that clears when a
poll succeeds. The eight: book and
author search, opening an EPUB or PDF, saving your place in the readers and the audio player, EPUB
bookmarks, the notification bell, the History page, Readarr import progress and the sidebar health
summary ([REQ-001 to REQ-008](#part-a--silent-failures)). No app-wide error handler is added
([DA3](#5-open-questions)).

**Book search when every source fails (added in v4).** Before this change, when every book source the search
tried fails, the server answers "no results", and the page says "No results". Now the server
answers with an error, "Every book source failed. Try again in a moment.", and the page shows it
with Retry. A search where any source answers, even with nothing, is unchanged
([REQ-009](#part-a--silent-failures)).

**Part B: deleting a book gets an "Also delete files from disk" box.** Deleting a book from its book
page, or deleting selected books on the Books page, shows the box unticked every time. Unticked, the
files stay on disk and the dialog says so. Ticked, the dialog warns that deletion is permanent and
the files really are removed. Before this change they never were, because the server looked for
them in the wrong folder. Livrarr removes only a plain file that sits inside the library folder. It never follows a
link, never removes a folder, and never touches another book's file. Anything it cannot remove is
named in a warning. The book is deleted either way ([REQ-101 to REQ-107](#part-b--deleting-a-book-with-opt-in-file-removal)).

**"Delete File" on a book's files tab removes the file (added in v4).** Before, it removed only
Livrarr's record and left the file on disk. Now confirming deletes the file from disk, under the
same safety rules, then the record. There is no box, because deleting the file is the button's
whole purpose; Readarr works the same way. If the file cannot be removed, the record stays and the
user sees one error saying why. A file already gone just has its record removed ([REQ-108](#part-b--deleting-a-book-with-opt-in-file-removal)).

**Decided during the spec:** undoing a list import keeps files (Q-001), and folders emptied by a
delete are left in place (Q-002).

**Evidence.** The System Truths are code reads at `4786c21f`, a read-only query of the live
database, the live log, and the reviewer's reads of installed libraries where marked. The built
change is checked by the red-first tests, the full Rust and frontend suites, two code-review rounds
and the PO's six live checks ([as built](#7-as-built-corrections-and-limits)).

**What the spec missed.** The builders found five more test callers of the book-delete method, one
extra trait bound in `convergence_service.rs`, and more code paths that share the changed code than
the spec listed. None changed behaviour ([corrections](#corrections-found-during-the-build)).

## Revision

v1, 2026-09-30: written from source at main `4786c21f` per `packet-0-spec/PACKET.md`.

v2, 2026-09-30: folds `packet-0-spec/REVIEW-astra-spec-r1.md` per `packet-0-spec/FOLD-PACKET.md`.
v1 is kept at `build/reviews/errors-and-delete-pass/packet-0-spec/spec-errors-and-delete-pass-v1.md`.

- Q-001 decided (a), keep files, by the PM under the PO's standing direction. Q-002 decided (b),
  never remove folders; REQ-104 withdrawn; PA-006 now cites Readarr source.
- R1: ST-025 corrected. REQ-102 and REQ-103 rewritten: removal never follows a link at the item's
  entry, removes only a regular file, and checks the resolved parent folder is inside the resolved
  root. New AC-103 to AC-105. The check-then-remove race recorded in §4.
- R2: REQ-102 keeps the original error and which side failed. The read callers' results are
  unchanged. ST-026 adds the stream and stream-token callers. New AC-106.
- R3: new mixed-failure, two-roots, explicit-false and root-path cases (AC-102, AC-107, AC-108).
  The Books-page bulk case is in AC-111.
- R4: dialog tests run the real `deleteWork` and `apiFetch` behind `apiStub.tsx` (AC-110).
- R5: ST-008 and problem 1 corrected. REQ-001 covers request failures only. "All providers
  failed shows No results" moved to §4, pending a PO decision.
- R6: REQ-003 text is now "Could not save your place.", with the rule for a failure run.
  DA2 and the summary were changed to match.
- R7: AC-001 to AC-008 extended.
- R8: §4 rewritten: Readarr undo is unchanged, and list undo is tied to REQ-107.
- R9: the full description is given for each dialog state (REQ-106, AC-110).
- R10: every AC is labelled red or guard; AC-008 split.
- Review note: ST-024 no longer claims one layout for every stored path.

v3, 2026-09-30: folds `packet-0-spec/REVIEW-astra-spec-r2.md` per `packet-0-spec/FOLD-PACKET-v3.md`.
Only §6 and this entry changed. v2 is kept at
`build/reviews/errors-and-delete-pass/packet-0-spec/spec-errors-and-delete-pass-v2.md`.

- F1: new AC-102b: a regular file whose removal fails (its folder made read-only) among
  removable and absent items. AC-108 now requires GET 404 and zero removals. §6 defines "the work
  is gone" as GET 404, so every removal case checks it.
- F2: new AC-106b (parent folder is a link loop: warned) and AC-106c (parent folder missing:
  silent). Both keep the read and email results as a guard.
- F3: AC-003 restores two failures in a row before the success.
- F4: AC-007 adds an outage after progress has loaded; AC-008a adds a cold first failure.
- F5: AC-110 adds cancel-while-ticked then reopen, and a new Books-page selection after a ticked
  delete.

v4, 2026-09-30: two PO-approved additions per `packet-0-spec/FOLD-PACKET-v4.md`. v3 is kept by
the PM.

- New REQ-009: book search answers 502 with a plain message when every attempted source failed.
  It adds ST-032 to ST-036, DA4 and AC-009a to AC-009f. The matching §4 item and the v2/v3
  "outside this feature" wording in the summary and problem 1 are removed.
- New REQ-108: "Delete File" removes the file through REQ-102's remove rule, then the record. The
  record stays with one 409 error when the file cannot be removed. It adds ST-037 to ST-039,
  PA-007, DB7 and AC-115 to AC-121. The "single-file delete is outside this change" item is removed
  from §4.
- The file lists are re-cut into Stream A (frontend only) and Stream B (all Rust plus the delete
  screens).
- The design principle on keeping files now applies to book deletes. The single-file button's
  purpose is deletion.

v5, 2026-09-30: folds `packet-0-spec/REVIEW-astra-spec-r4.md` per `packet-0-spec/FOLD-PACKET-v5.md`.
The v4 draft is kept by the PM as `spec-errors-and-delete-pass-v4.md`.

- F1: new guard AC-009g. OpenLibrary answers with no hits, Goodreads fails, and the other two
  sources are skipped: expect 200 with empty results, not 502.
- F2: the §6 "the work is gone" check now applies only to whole-book deletes. REQ-108 states
  that the book and its other files stay. AC-115 gains a sibling item, and AC-116 checks that the
  book remains. AC-101 to AC-111 are unchanged.
- F3: AC-009b names the configuration discovery reads (a Hardcover token in the harness
  database). It requires one request each to OpenLibrary, Goodreads and Hardcover and none to
  Google Books.
- F4: PA-007 describes the cited root check as Readarr's mapped-author branch. It records that
  Livrarr uses its own REQ-102 root rules, so a file directly under a usable root stays removable.

v6, 2026-09-30: brought in line with the built change, per `packet-5-as-built/PACKET.md`.
Sources: `packet-3-code-review/code-r2.patch` and `test-fold-r2.patch`; hand-backs
`packet-2a-code/HANDBACK-2a.md` and `packet-2b-code/HANDBACK-2b.md`; the fold summarised in the
session log entry "2026-09-30T21:05Z"; reviews `packet-3-code-review/REVIEW-astra-code-r1.md` (FAIL,
one P2) and `-r2.md` (PASS); the live check `deploy-20260930T221942Z/LIVE-CHECK.md`. Status
`delivered`. v5 is kept at
`build/reviews/errors-and-delete-pass/packet-0-spec/spec-errors-and-delete-pass-v5.md`.

- Every requirement is marked Delivered; none is deferred. Each carries a short "Built" note.
- REQ-003 gains its as-built rule for a failure run that starts while the previous warning is
  leaving (review r1 F1).
- ST-029 and the Stream B file list gain the corrections the builders found. New §7 records those
  corrections, the extra shared-code doors from review r1, and the recorded limits.
- The executive summary says the feature is delivered. The acceptance criteria are ticked, with
  the PDF exceptions noted. No decision changed.

## 0a. Design Principles

- One message per failure, in the app's existing form; never two for one failure.
- Fix where the wrong behaviour is created; one definition per rule, shared by every door.
- When deleting a book, absent a choice, keep the user's files. "Delete File" exists to delete
  its file. Remove only a regular file, reached without following a
  link at its own entry, whose resolved folder is inside the resolved root. Never remove a folder.
- Smallest change; no migration, no new dependency, no new setting.
- Tests drive the real entry path and pin the named observable only.

## 0b. System Truths

All read at `4786c21f` unless stated. "read" = code read; nothing executed.

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-001 | The query client sets query defaults only; there is no mutation- or query-cache error handler anywhere in `frontend/src`. | `frontend/src/App.tsx:70-79`; `rg MutationCache\|QueryCache` (one hit, a test) | — | read |
| ST-002 | Load-failure pattern: `ErrorState` (icon, the server's message for an `ApiError`, else "Something went wrong", optional Retry). Used by Queue and Authors. | `components/Page/ErrorState.tsx:4-29`; `pages/activity/queue/QueuePage.tsx:71`; `pages/authors/AuthorsPage.tsx:120` | A new error widget where `ErrorState` fits. | read |
| ST-003 | Action-failure pattern: `toast.error` in the mutation's `onError`. | `pages/work-detail/components/LibraryFilesTab.tsx:42`; `pages/authors/AuthorsPage.tsx:105`; `QueuePage.tsx:58` | — | read |
| ST-004 | A toast raised again with the same `id` replaces the live one instead of stacking. | `components/Header/NotificationBell.tsx:74-77`, `:103-104`; sonner 2.0.7 `dist/index.mjs:145` (reviewer read) | Stacking a new toast per failure. | read; library confirmed by reviewer |
| ST-005 | One notification container, at the app root; it serves `/read` and `/listen` too. | `App.tsx:444-454`; `spec-small-bugs-pass.md` REQ-001, ST-003 | — | read |
| ST-006 | Of the non-test `useMutation` calls in `frontend/src`, nine have no `onError`: `NotificationBell.tsx:122,129,136`, `EpubReader.tsx:86,96,103`, `AudioPlayer.tsx:110,119,125`. | Python scan of every `useMutation(` block | — | scan run 2026-09-30; reviewer's scan agrees |
| ST-007 | The sleep-timer bookmark passes its own `onError` (a console warning) to `.mutate`, then shows "Bookmark saved" regardless. A cache-level handler reads only the options given to `useMutation`. | `AudioPlayer.tsx:341-353`; query-core 5.96.0 `mutation.ts:276`, `mutationObserver.ts:132` (reviewer read) | A global mutation error handler (DA3). | read; library confirmed by reviewer |
| ST-008 | C1: both pages ignore the query's error, so a failed *request* leaves the book search blank (`showNoResults` needs non-null results) and the author search empty. The author search turns a provider failure into an HTTP error. The free-text book search does not: each provider's error becomes an empty list, the lists are merged, and an empty 200 comes back. So when every book provider fails, the page shows "No results". | `pages/search/SearchPage.tsx:92-96`, `:141-146`, `:243-307`; `pages/search/AuthorSearchPage.tsx:19-26`, `:94-110`; `crates/livrarr-metadata/src/author_service.rs:283-292`; `crates/livrarr-metadata/src/discovery_service.rs:163-178`, `:322`, `:356`, `:436-444` | Treating a rejected `lookupWorks` mock as the all-providers-failed case. | read |
| ST-009 | C2: the EPUB download's failure is swallowed and the page shows "Loading..." while `epubData` is null. The PDF fetch never checks `res.ok`: an error body goes to the viewer ("Failed to load PDF.", no retry). A network failure leaves `file=null`, for which react-pdf 10.4.1 shows "No PDF file specified." | `pages/reader/EpubReader.tsx:148-162`, `:331-337`; `pages/reader/PdfReader.tsx:78-88`, `:155-163`; react-pdf `dist/Document.js:41`, `:290` (reviewer read) | — | read; the non-PDF-body case is inferred |
| ST-010 | C3: five position saves swallow failures: EPUB, PDF, audio debounced save, audio 10-second save while playing, audio "jump" from the resume banner. PDF and EPUB make one save per navigation, after a 2-second pause; audio saves periodically only while playing. None retries a failed save. | `EpubReader.tsx:174-186`; `PdfReader.tsx:47-53`; `AudioPlayer.tsx:193-205`, `:209-224`, `:966-972` | Promising automatic retries. | read |
| ST-011 | C4: EPUB bookmark add, delete and rename have no `onError`. The bookmark toolbar exists only after the book has loaded. | `EpubReader.tsx:86-111`, `:331`, `:477-483` | — | read |
| ST-012 | C5: the bell's list query ignores its error and shows "No notifications"; mark-read, dismiss and dismiss-all have no `onError`. Dismiss also runs when the user closes a path-not-found pop-up. | `NotificationBell.tsx:115-141`, `:175-182`, `:81` | — | read |
| ST-013 | C6: History shows the spinner while the whole-library read has no data, so a failed read spins forever; its own error check comes after. | `pages/activity/history/HistoryPage.tsx:104`, `:108-109`; `hooks/useLibraryWorks.ts:40-45` | — | read |
| ST-014 | C7: import progress polls every 2 s and swallows errors; the progress panel renders only once a poll has succeeded. | `pages/import/ReadarrImportPage.tsx:131-153`, `:794-797` | — | read |
| ST-015 | C8: the health widget returns nothing without data, and requests the summary for every user. The route is admin-only. On a failed refetch the query keeps the old data, so an admin sees stale health. The store exposes `isAdmin`. | `components/Sidebar/Sidebar.tsx:373-381`; `crates/livrarr-handlers/src/system.rs:165-173`; `stores/auth.ts:23`; `Sidebar.tsx:230`; query-core `query.ts:661` (reviewer read) | Showing an error to non-admins. | read |
| ST-016 | `installApiStub` replaces `fetch` only, so the real `apiFetch` runs; it records method and path with the query string. | `frontend/src/test-support/apiStub.tsx:1-14`, `:47-80` | Stubbing `@/api` for the delete-dialog tests. | read |
| ST-020 | Two UI doors delete a book. Book page: `deleteWork(Number(id))`, dialog "Permanently delete "…" and all associated files on disk? This cannot be undone." Books page, selected books: `deleteWork(id)` per id through `Promise.allSettled`, dialog "Permanently delete N works and all associated files on disk. This cannot be undone.", partial failure shown with `toast.warning`. | `pages/work-detail/WorkDetailPage.tsx:115-123`, `:231-240`; `pages/works/WorksPage.tsx:385-402`, `:724-731` | — | read |
| ST-021 | `ConfirmModal` renders `children` under the description and toasts any error thrown by `onConfirm`. | `components/Page/ConfirmModal.tsx:15`, `:34-47`, `:67` | — | read |
| ST-022 | `deleteWork(id)` sends `DELETE /work/{id}` with no options; the route calls `WorkService::delete(user, id)` and always answers `{ warnings: [] }`. | `frontend/src/api/index.ts:212-215`; `crates/livrarr-server/src/router.rs:266-269`; `crates/livrarr-handlers/src/work.rs:1391-1398`; `crates/livrarr-handlers/src/types/work.rs:578-582`; `frontend/src/types/api.ts:415-417` | — | read |
| ST-023 | `WorkService::delete` reads the items, deletes the work row (items cascade), records `workDeleted` with `files_removed = items.len()`, then calls `remove_file(item.path)` and only logs failures. | `crates/livrarr-metadata/src/work_service.rs:1519-1578`; `crates/livrarr-db/migrations/001_initial_schema.sql:112`; `crates/livrarr-domain/src/history_events.rs:427-449` | Recording the count before removal. | read |
| ST-024 | Import stores the path relative to its root folder (`target_relative`). The current layout is `{user_id}/{author}/{title}.{ext}` or `{user_id}/{author}/{title}/{file}`. Older libraries may hold files directly under the root, with no user folder, and the root scan still reads them. The delete never joins the root, so it passes a relative path to the filesystem. The live log has three "failed to delete library file on work delete: No such file" warnings. | `crates/livrarr-library/src/import_workflow.rs:254-258`, `:495-499`, `:1762-1797`; `crates/livrarr-handlers/src/root_folder.rs:136-142`; live DB (`1/Joe Abercrombie/Red Country.epub`); `testdata/logs/livrarr.log.2026-07-18:41`, `livrarr.log.2026-08-18:3179`, `:3184` | Assuming one layout for every stored path. | read; live DB read-only |
| ST-025 | The schema makes the *stored* path unique per `(user, root, path)`, and the writer checks the same thing. Nothing stops a file on disk being replaced, after import, by a link to another file (another book's, or one outside the root). One root folder per media type. | `001_initial_schema.sql:102-118`; `crates/livrarr-db/src/sqlite_library_item.rs:184-190` | Following a link at the item's entry when removing. | read |
| ST-026 | The one item-to-disk definition is `FileService::resolve_path`. It canonicalizes the item, following links; any failure becomes `NotFound`. It then canonicalizes the root (failure: `Io`) and refuses anything outside the root (`Forbidden`). `prepare_email` repeats it inline. Callers: download, stream, stream-token mint, OPDS, cross-format. | `crates/livrarr-library/src/file_service.rs:97-133`, `:145-176`; `crates/livrarr-handlers/src/work.rs:1842`, `:1862`; `crates/livrarr-handlers/src/workfile.rs:127`; `crates/livrarr-handlers/src/opds.rs:495`; `crates/livrarr-library/src/cross_format_service.rs:137` | A third copy; changing these callers' results. | read |
| ST-027 | `livrarr-metadata` does not depend on `livrarr-library`; both depend on `livrarr-domain`, which already uses `std::fs`. | `crates/livrarr-metadata/Cargo.toml` `[dependencies]`; `crates/livrarr-library/Cargo.toml:12-13`; `crates/livrarr-domain/src/perf.rs:51` | Calling `FileServiceImpl` from `WorkServiceImpl`. | read |
| ST-028 | Every place that removes a library file today: whole-book delete (`work_service.rs:1567`, broken per ST-024); Readarr import undo (`crates/livrarr-server/src/readarr_import_workflow.rs:1186-1217`); reorganize clean-up after a move (`crates/livrarr-server/src/import_service.rs:683`, `:765`, `:785`); a test-only API (`api_secondary_impl.rs:704`). Deleting one file from the book page (`DELETE /workfile/{id}`) removes only the record. | as cited; `file_service.rs:70-95`; `tests/behavioral/test_wh_deletion.rs:190-200` | — | read; `rg remove_file` over `crates` |
| ST-029 | `WorkService::delete` has two production callers: the route and list-import undo. Four test stubs implement the trait. `WorkServiceImpl`'s database bound lacks `RootFolderDb`. | `crates/livrarr-domain/src/services/work.rs:456`; `crates/livrarr-handlers/src/work.rs:1396`; `crates/livrarr-metadata/src/list_service.rs:807`; `tests/behavioral/test_responsiveness_bulk.rs:55`, `test_door_gate.rs:802`, `test_consolidation_import_workflow.rs:1554`, `test_consolidation_author_monitor.rs:55`; `work_service.rs:324-342` | — | read. **Incomplete (found in the build):** five more test callers of the method, and `convergence_service.rs` needed the new bound; see [§7](#corrections-found-during-the-build). |
| ST-030 | An existing test pins the current count: two items with no files on disk and `files_removed == 2`. | `tests/behavioral/test_wh_deletion.rs:202-247` | Leaving that assertion as is. | read |
| ST-031 | Precedent for per-file warnings: reorganize returns `"{path}: <what failed>"` strings. | `import_service.rs:590-595`, `:676` | — | read |
| ST-032 | Free-text book search runs four sources at once. OpenLibrary and Goodreads always make a request. Google Books is skipped without an API key, and Hardcover when disabled or without a token. A skip returns an empty success, the same as "searched, found nothing". A source's error becomes an empty list in `take_lookup`. | `crates/livrarr-metadata/src/discovery_service.rs:163-178`, `:322-327`, `:335-354`, `:925-945`, `:957-975`, `:1057-1082`, `:761-800` | Counting a skipped source as a success or as a failure. | read |
| ST-033 | An empty search answer returns before the 15-minute search cache is written, and so does an error from `lookup`. Only non-empty answers are cached. | `discovery_service.rs:416-436`, `:438-449`, `:474-486` | Caching the all-failed answer as empty. | read |
| ST-034 | Upstream-failure convention: `ApiError::BadGateway` answers 502 with its message in the body; "All indexers failed" uses it. `ApiError::Internal` hides its message as "Something went wrong". `WorkServiceError::Enrichment` maps to `Internal`. `WorkServiceError` is matched exhaustively once. | `crates/livrarr-handlers/src/types/api_error.rs:256-258`, `:494`, `:516-523`, `:218-229`; `crates/livrarr-domain/src/services/work.rs:277-294` | Reporting all-failed as 500. | read |
| ST-035 | `lookup` has three production callers: `lookup_filtered`, used by the book-search route (`work.rs:197-200`) and by the manual-import search (`crates/livrarr-handlers/src/manual_import.rs:808-820`); and the eager-match fallback, which treats an error as "no match" (`discovery_service.rs:652-658`). The manual-import page already has an error state for a failed search. | as cited; `frontend/src/pages/manual-import/ManualImportPage.tsx:342-352`; `discovery_service.rs:138-148` | — | read |
| ST-036 | The real lookup route can be driven with scripted provider transport: `build_route_harness_with_provider_details` with a `DiscoveryTransportFixture` whose `scripted_transport` returns a response or a transport error. An existing test pins that one failed source keeps the others' results. | `tests/behavioral/test_ilr_contracts.rs:6087-6100`, `:6101`, `:6827`, `:6917-6930`; `crates/livrarr-http/src/fetcher.rs:133-142` | A mocked discovery service. | read |
| ST-037 | "Delete File" is the one caller of `DELETE /workfile/{id}`. The route calls `FileService::delete`, which deletes the record, writes a `fileDeleted` event and never touches the disk. The dialog says "Are you sure you want to delete this library file?". On failure the tab shows two pop-ups: its own `onError` ("Failed to delete file"), plus the dialog's pop-up for the rethrown error. `ManualImportService::delete_library_item` does the same record-only delete but has no production caller. | `frontend/src/api/index.ts:593-594`; `frontend/src/pages/work-detail/components/LibraryFilesTab.tsx:35-43`, `:201-213`; `components/Page/ConfirmModal.tsx:34-42`; `crates/livrarr-server/src/router.rs:575-577`; `crates/livrarr-handlers/src/workfile.rs:53-60`; `crates/livrarr-library/src/file_service.rs:70-95`; `crates/livrarr-server/src/manual_import_service.rs:70-99`; `rg delete_library_item` | — | read |
| ST-038 | `FileServiceError` maps to `ApiError` in one place; `Io` becomes a 500 that hides its message. `ApiError::Conflict { reason }` answers 409 with the reason in the body. | `crates/livrarr-domain/src/services/file.rs:32-45`; `api_error.rs:335-340`, `:453` | Reporting a refused removal as 500 (the user could not see why). | read |
| ST-039 | Two existing tests call `FileService::delete` on items whose root folder does not exist on disk (`/tmp/livrarr-wh-…`, `/tmp/root`). Under REQ-108 an unusable root keeps the record, so both must seed a real root in a temporary folder. | `tests/behavioral/test_wh_deletion.rs:54-75`, `:141-163`; `tests/behavioral/test_consolidation_file_service.rs:239-259` | Leaving them seeding non-existent roots. | read |

## 0c. Prior Art

Searched `docs/`, `wiki/`, the project to-do list in `build/plans/`, `git log`, the product-gap
report and the Readarr source at `tmp/readarr-source/`.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | Commit `425c6f8c` (2026-05-25, alpha5 audit, "Theme B"). Livrarr had `?deleteFiles=` and an "Also delete files from disk" box in both dialogs; the server ignored it. The audit removed both and made delete always try to remove files. | REQ-101 restores the parameter and box, now honoured; AC-110 tests the whole path from dialog to URL so the choice cannot be dropped again. The old book-page box kept its state between openings; REQ-106 resets it. |
| PA-002 | Project to-do list, entry "Deleting a book never deletes its file", which asked whether delete should remove files at all. | Answered by the PO on 2026-09-30: the user chooses. Part B closes the entry. |
| PA-003 | `wiki/decisions/merge-undo-rewrite-dropped.md:24`: duplicates are resolved by deleting the extra book. | Deleting is now routine, so the default must keep files (REQ-101). |
| PA-004 | `build/reports/ux-gap-scan-2026-09-26.md` § C rows C1–C8 (lines 63–70); row A6, a disabled "delete empty folders" setting (`MediaManagementPage.tsx:629-653`). | Source of Part A. A6 stays unwired; no folder is removed (Q-002). C9 is not approved. |
| PA-005 | `spec-small-bugs-pass.md` REQ-001: one notification container. | Pop-ups from the readers and player reach the user (ST-005). |
| PA-006 | Readarr removes emptied folders after a book-file delete only when `DeleteEmptyFolders` is on (`tmp/readarr-source/src/NzbDrone.Core/MediaFiles/MediaFileDeletionService.cs:211`), which defaults to off (`tmp/readarr-source/src/NzbDrone.Core/Configuration/ConfigService.cs:163`). | Basis for Q-002 (b): Livrarr never removes folders. |
| PA-007 | Readarr's single book-file delete (`tmp/readarr-source/src/Readarr.Api.V1/BookFiles/BookFileController.cs:138-156` → `NzbDrone.Core/MediaFiles/MediaFileDeletionService.cs:61-127`). It has no option. In the mapped-author branch, a missing root folder or one with no subfolders gives 409 and keeps the record (`:66-76`). A file that exists is deleted; if that throws, 500 "Unable to delete book file" and the record stays (`:94-98`, `:122-126`). An absent file, or a missing author folder, still has its record deleted (`:83-87`, `:100-101`). Deletion goes through the recycle bin when one is configured. | REQ-108 copies this shape: no box, the record stays when removal fails, an absent file just loses its record. The cited missing- and empty-root checks belong to Readarr's mapped-author branch; an unmapped file takes the other path and skips them (`BookFileController.cs:148-155`). Livrarr applies its own REQ-102 root and path rules, not Readarr's "root must contain a folder" check, so a regular file directly under a usable Livrarr root stays removable. It departs in two further places. Livrarr answers 409 with the reason for every refused removal, because its 500 hides the message (ST-038). Livrarr has no recycle bin. |

## 1. Problem Statement

Part A:

1. **C1.** Searching for a book on the Search page, or for an author on Add New Author, when the
   search request itself fails (server error or network). Expected: an error and a way to retry.
   Observed: a blank result area (ST-008).
2. **C2.** Opening an EPUB whose download fails. Observed: "Loading..." forever. A PDF shows
   "Failed to load PDF." or "No PDF file specified.", with no retry (ST-009).
3. **C3.** Reading or listening while saving your place fails. Observed: nothing; the place is lost.
4. **C4.** Adding, renaming or deleting a bookmark in the EPUB reader fails. Observed: nothing.
5. **C5.** Opening the bell when the load fails: "No notifications". A failed mark-read or dismiss: nothing.
6. **C6.** Opening History while the whole-library read fails. Observed: an endless spinner.
7. **C7.** A Readarr import is running and the server stops answering. Observed: frozen numbers, or
   no progress panel at all if no poll has succeeded yet.
8. **C8.** An admin's health check fails. Observed: the sidebar summary disappears or goes stale.
9. **All sources fail.** Searching for a book by title when every book source the search tries fails.
   Expected: an error and a way to retry. Observed: "No results", as if nothing matched (ST-032).

Part B:

10. **Deleting a book from its book page, or selected books on the Books page.** The dialog promises
    the files are deleted; they never are (ST-024). The user has no choice.
11. **"Delete File" on a book's files tab.** Expected: the file is deleted from disk. Observed: only
    Livrarr's record goes; the file stays (ST-037).

## 2. Requirements

### Part A — silent failures

Forms (DA1): a failed load shows `ErrorState` (or the same look) with Retry where the content would
have been; a failed action shows `toast.error` from that mutation's own `onError` (ST-002, ST-003).
Failure runs (DA2): the first failure shows the message, later failures add nothing, the next
success clears it, and a failure after that warns again. No global handler (DA3).

- **REQ-001** (Delivered; C1): When the book-search or author-search *request* fails (an HTTP error, or a
  network failure), the results area shows the error (the server's message when there is one) and a
  Retry button. "No results" is not shown. Retry re-runs the same search, with the same term,
  language and raw-mode setting, and the error clears when it succeeds. Book search keeps showing
  its "In Your Library" matches. Door: mount the real page, stub only the network. Built:
  `SearchPage.tsx` and `AuthorSearchPage.tsx` show `ErrorState` with Retry; empty and stale results
  are hidden while the error shows.
- **REQ-002** (Delivered, PDF success path unproven; C2): When the EPUB or PDF download fails (non-2xx status or network error), the reader
  shows "Could not load this book." with a Retry button instead of "Loading..." or the PDF viewer.
  Retry fetches again, and on success the book opens. The PDF reader checks the status before
  handing data to the viewer. Door: mount `EpubReader` / `PdfReader` with `fetch` stubbed. Built:
  the notice is defined once in `EpubReader.tsx` and imported by `PdfReader.tsx`. The PDF error and
  refetch are tested; that a PDF actually opens after Retry is not (PO: out of scope).
- **REQ-003** (Delivered, PDF save test still `fixme`; C3): All five position saves (ST-010) go through one shared helper under
  `frontend/src/pages/reader/`. Each call site keeps its current position, percentage, kind and
  cross-format arguments. The helper applies the DA2 rule with
  `toast.error("Could not save your place.")` under a fixed id (ST-004) and no time-out. The next
  successful save dismisses it. Nothing retries a failed save.

  Built: the helper is `frontend/src/pages/reader/savePosition.ts`; all five sites call it with
  their current arguments. As-built rule for a restart (review r1 F1): a failure run that starts
  while the previous warning is still leaving the screen shows its warning once that warning has
  gone. The helper learns the warning has started to leave from the pop-up library's dismiss
  callback, then waits 300 ms (the library unmounts a leaving pop-up after 200 ms) before showing
  it. A success before then cancels the pending warning. A warning the user closes by hand leaves
  that run silent; the next success re-arms the rule. The save-failed state is shared by every
  reader and the player in one browser tab. The PDF site is wired but its browser test is still
  `fixme`.
- **REQ-004** (Delivered; C4): EPUB bookmark add, rename and delete each get an `onError` toast: "Could not add
  the bookmark", "Could not rename the bookmark", "Could not delete the bookmark". Built in
  `EpubReader.tsx`.
- **REQ-005** (Delivered; C5): When the bell's list load fails, the open popover shows "Could not load
  notifications" with a Retry that reloads the list, instead of "No notifications". Mark-read,
  dismiss and dismiss-all each get an `onError` toast: "Could not mark the notification as read",
  "Could not dismiss the notification", "Could not dismiss notifications". Built in
  `NotificationBell.tsx`. A list that fails to refresh now shows the error instead of the old list,
  and closing a path-not-found pop-up whose dismiss fails now shows the dismiss error.
- **REQ-006** (Delivered; C6): If the whole-library read fails, History shows `ErrorState` instead of the
  spinner. Its Retry re-runs both the history read and the whole-library read. Built in
  `HistoryPage.tsx`.
- **REQ-007** (Delivered; C7): While importing, a failed progress poll shows one inline line in the progress
  area: "Lost contact with Livrarr. Progress may be out of date; still trying." The line shows even
  if no poll has succeeded yet, and it follows DA2. Polling continues as today. Built in
  `ReadarrImportPage.tsx`.
- **REQ-008** (Delivered; C8): The health widget is shown to admins only; for non-admins it shows nothing and
  sends no request. For an admin, when the latest check failed it shows a red "Health check failed"
  line, linking to the status page. In the collapsed sidebar it shows a red dot titled the same.
  This applies to a failed refetch after an earlier success, and the old data is not shown. It
  returns to the normal summary when a check succeeds. Built in `Sidebar.tsx`. The Status page's
  own health query is unchanged.
- **REQ-009** (Delivered; added in v4; server side, built in Stream B): In free-text book search, each source
  ends in one of three states: *skipped* (not configured, so no request), *answered* (with or
  without results) or *failed*.
  - If at least one source was attempted and every attempted source failed, `lookup` returns a new
    `WorkServiceError::AllProvidersFailed`.
  - The route maps it to `ApiError::BadGateway("Every book source failed. Try again in a
    moment.")`, a 502 (ST-034, DA4).
  - REQ-001's page handling shows that message with Retry.
  - A source that fails its configuration read counts as failed.

  Unchanged:
  - any search where a source answered, even with nothing: an empty answer stays "No results";
  - a partial failure keeps the answering sources' results;
  - source selection, ranking and ordering;
  - the identifier path (`isbn:` and provider keys, `discovery_service.rs:385-414`), the
    empty-term and unsupported-language answers, and the search cache (the error is never cached,
    ST-033).

  Other doors (ST-035):
  - the manual-import search now gets the 502 and shows its existing error state;
  - the eager-match fallback treats the error as "no match", as it treats any error today.

  Built: in `discovery_service.rs` Google Books and Hardcover report "skipped" separately from an
  empty answer, and `lookup` raises `AllProvidersFailed` before the cache write. The route maps it
  in `api_error.rs`. The Google Books author-batch search adapts to the new skipped result by
  treating a skip as an empty list, so its behaviour is unchanged.

### Build streams

**Stream A (frontend only): REQ-001 to REQ-008.** Production: `frontend/src/pages/search/SearchPage.tsx`,
`pages/search/AuthorSearchPage.tsx`, `pages/reader/EpubReader.tsx`, `pages/reader/PdfReader.tsx`,
`pages/reader/AudioPlayer.tsx`, one new helper file in `pages/reader/`,
`components/Header/NotificationBell.tsx`, `pages/activity/history/HistoryPage.tsx`,
`pages/import/ReadarrImportPage.tsx`, `components/Sidebar/Sidebar.tsx`. Tests:
`HistoryPage.test.tsx`, `ReadarrImportPage.test.tsx`, and new test files beside those components.

**Stream B (every Rust change plus the delete screens): REQ-009's server change and REQ-101 to REQ-108.**

Production, Rust:
- `crates/livrarr-domain/src/services/work.rs` (delete signature, new error variant);
- `crates/livrarr-domain/src/services/file.rs` (new error variant);
- a new path module in `crates/livrarr-domain/src/` plus its export;
- `crates/livrarr-library/src/file_service.rs`;
- `crates/livrarr-metadata/src/work_service.rs`, `list_service.rs` and `discovery_service.rs`;
- `crates/livrarr-handlers/src/work.rs` and `crates/livrarr-handlers/src/types/api_error.rs`.

Production, frontend: `frontend/src/api/index.ts`, `pages/work-detail/WorkDetailPage.tsx`,
`pages/works/WorksPage.tsx` and `pages/work-detail/components/LibraryFilesTab.tsx`.

Tests: `tests/behavioral/test_ilr_contracts.rs`, `test_wh_deletion.rs`,
`test_consolidation_file_service.rs`, the four stubs in ST-029, and new frontend test files beside
the three pages.

**Overlap: none.** No file appears in both streams. REQ-009's page display rides on Stream A's
REQ-001; its server change is testable alone.

**As built.** Stream A's new helper is `frontend/src/pages/reader/savePosition.ts`. Stream B's new
path module is `crates/livrarr-domain/src/library_path.rs`. Stream B also changed three files
outside its list, for compilation only: `crates/livrarr-metadata/src/convergence_service.rs` (two
trait bounds), `tests/behavioral/test_wh_observer.rs` and
`tests/behavioral/test_consolidation_work_service.rs` (the new argument). See
[§7](#corrections-found-during-the-build).

### Part B — deleting a book, with opt-in file removal, and "Delete File"

- **REQ-101** (Delivered; DB1): `DELETE /api/v1/work/{id}` takes an optional query parameter `deleteFiles`
  (`true`/`false`). Absent or `false`: no file is touched. A value that is not a boolean is refused
  with 400 and nothing is deleted. `deleteWork(id, { deleteFiles })` in `frontend/src/api/index.ts`
  sends `?deleteFiles=true` only when ticked, otherwise the current URL. Built in
  `livrarr-handlers/src/work.rs` and `frontend/src/api/index.ts`.
- **REQ-102** (Delivered; DB2): One module in `livrarr-domain` holds the item-to-disk rules. Its shared first
  step resolves the root folder path. It reports which failed, the root or the item, and keeps the
  original filesystem error.
  - **Read** (current behaviour): canonicalize the item, following links, and require it inside the
    resolved root. `FileService::resolve_path` and `prepare_email` use it. Every caller in ST-026
    gets exactly its current result, error for error: item failure `NotFound` (including when the
    root is missing), root failure `Io`, outside the root `Forbidden`.
  - **Remove** (new): canonicalize the item's *parent folder* and require it inside the resolved
    root. Then read the item's own entry in that folder without following links. The entry is
    removable only if it is a regular file. The whole-book delete uses this rule; nothing else
    changes. The Readarr undo and the ad-hoc joins elsewhere stay as they are.

  Built: `crates/livrarr-domain/src/library_path.rs`, with `resolve_for_read` (read rule) and
  `remove_library_file` (remove rule). `FileService::resolve_path` and `prepare_email` call the read
  rule; the whole-book delete and "Delete File" (REQ-108) call the remove rule.
- **REQ-103** (Delivered; DB4): `WorkService::delete` takes the choice and returns the number of files removed
  and a list of warnings. With removal chosen, it reads each item's root folder, deletes the work as
  today, then applies the remove rule to every item, carrying on past failures. Outcomes per item:
  - Removed: counted.
  - Silent, not counted: the item, or its parent folder, is absent under a usable root, or the file
    vanished during removal.
  - Warned, not counted, one warning `"{relative path}: {reason}"` (ST-031):
    - the root folder cannot be resolved (for example it was moved);
    - the parent folder resolves outside the root;
    - the entry is a link, a folder (including a path that resolves to the root itself) or
      another non-regular file;
    - any other filesystem error.

  No folder is ever removed. The book is deleted whatever happens to its files, and the route
  answers 200 with the warnings. Built in `livrarr-metadata/src/work_service.rs`; the service's
  database bound now includes `RootFolderDb`.
- **REQ-105** (Delivered; DB6): The `workDeleted` history record is written after removal, with
  `files_removed` = files actually removed (0 when keeping files). The test at ST-030 is rewritten to
  this rule. Built: the rewritten test is in `tests/behavioral/test_wh_deletion.rs`.
- **REQ-106** (Delivered): Both dialogs show an "Also delete files from disk" box, unticked each time the dialog
  opens. Descriptions:
  - Book page, unticked: `Delete "{title}" from your library? Its files stay on disk.`
  - Book page, ticked: `Delete "{title}" from your library? Its files will be permanently deleted
    from disk. This cannot be undone.`
  - Books page, unticked: `Delete N works from your library? Their files stay on disk.`
  - Books page, ticked: `Delete N works from your library? Their files will be permanently deleted
    from disk. This cannot be undone.`

  Unticking again restores the unticked text. On the Books page one choice applies to every
  selected book. After a delete:
  - No warnings: the success toast as today.
  - Book page, with warnings: one `toast.warning` "Book deleted, but N files could not be removed",
    listing them.
  - Books page: the existing succeeded/failed summary. A file warning never counts as a failed
    delete. Warnings from every successful delete are gathered into the same `toast.warning`,
    listing up to five and then "and K more".

  Built in `WorkDetailPage.tsx` and `WorksPage.tsx`; both reset the box each time the dialog opens.
- **REQ-107** (Delivered; DB3, Q-001): List-import undo calls `WorkService::delete` with "keep files".
  Built in `livrarr-metadata/src/list_service.rs`.
- **REQ-108** (Delivered; added in v4, DB7): `FileService::delete` (the `DELETE /api/v1/workfile/{id}` route)
  removes the item's file from disk through REQ-102's remove rule, then the record. It deletes only
  that file and its library-item record; the book and its other files stay.
  - Reads the item first (not found: 404 as today), then its root folder.
  - Removed, or absent under a usable root (including vanishing during removal): the record is
    deleted and the `fileDeleted` history event written as today. Answer 200.
  - Any outcome REQ-103 would warn about keeps the record and writes no event. That covers an
    unusable root, a parent folder outside the root, a link, a folder, another non-regular file,
    and any other filesystem error. It answers 409 through a new `FileServiceError::NotRemoved`
    carrying `"{relative path}: {reason}"` (ST-038). The file and record are both still there, so
    the user can retry.
  - No box and no query parameter.

  Book files tab (`LibraryFilesTab.tsx`):
  - The dialog reads `Delete this file from disk? This cannot be undone.`
  - Success shows "File deleted" as today.
  - Failure shows exactly one error pop-up with the server's message: the tab's own `onError`
    pop-up is dropped, and the dialog's pop-up for the rethrown error remains (ST-037).

  The only caller is that tab. Anything else calling the route with an API key gets the same
  behaviour.

  Built: `FileService::delete` in `livrarr-library/src/file_service.rs` removes the file before
  the record; `FileServiceError::NotRemoved` maps to 409 in `api_error.rs`. The tab's own error
  pop-up is dropped in `LibraryFilesTab.tsx`.

## 3. UI/Interface Design

Part A adds only the messages above; the book search shows REQ-009's message through REQ-001.
Part B adds the box to both delete dialogs and the descriptions in REQ-106. The API gains
`deleteFiles`, and the response shape is unchanged. "Delete File" gets the REQ-108 dialog text,
and its route can now answer 409. The book search route can answer 502.

## 4. Non-Requirements

- No global error handler. No change to pages not listed. C9 (audio playback) is not included.
- **Readarr import undo keeps its current behaviour**, including its file removal. It is outside
  this change.
- No recycle bin for "Delete File" (Readarr has one; PA-007). No bulk file delete.
- List-import undo keeps files, per REQ-107 and Q-001.
- No folder is removed. No "delete empty folders" setting, no recycle bin. What a delete does to
  grabs, series or covers is unchanged.
- **The check-then-remove race is not closed.** A folder could be swapped for a link between the
  parent check and the removal. Closing that needs file operations relative to an open folder
  handle, which the Rust standard library does not offer, so it would need a new dependency. Anyone
  able to swap folders inside the library already has write access to its files. The PM records it
  under Security in the to-do list.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | DB3: when a list import is undone, should files its books gained later be removed? | closed, PM 2026-09-30 under the PO's standing direction to make common-sense calls | (a) keep. Undo removes what the import created, and an import creates no files. |
| Q-002 | DB5: remove folders left empty by a delete? | closed, PM 2026-09-30 | (b) never. Readarr does it only behind a setting that is off by default (PA-006); never removing is the smaller change. |
| DA1 | Message form. | closed, spec | Inline error with Retry for loads; pop-up for actions (ST-002, ST-003). C3 is a pop-up because the reader has no spare space and a save is an action. C7 is inline because it is a status on an open panel. |
| DA2 | Repeated failures. | closed, spec | One message per failure run, cleared by the next success; a later failure warns again. C3 uses a fixed pop-up id so nothing stacks (ST-004). Only C7 retries by itself. |
| DA3 | One shared handler versus local ones. | closed, spec: local | A mutation-level handler reaches only C4 and C5 (the rest are queries or direct calls). It would also fire for three audiobook bookmark actions outside scope (ST-006, ST-007). |
| DB1 | How the choice travels. | closed, spec | Query parameter `deleteFiles`, absent = keep (PA-001). A body on `DELETE` or a new route adds surface for no gain. |
| DB2 | The one path definition. | closed, spec | REQ-102, lifted to `livrarr-domain` because the delete lives in a crate that cannot call `livrarr-library` (ST-027). Separate read and remove rules, because following a link is right for reading and wrong for removing (ST-025). |
| DB4 | Partial failure. | closed, spec | REQ-103, REQ-106. |
| DB6 | History count. | closed, spec | REQ-105. |
| DA4 | REQ-009: status code; no source attempted; caching. | closed, spec | **502** with the message, following the app's upstream-failure convention ("All indexers failed", ST-034); a 500 would hide the message. **No source attempted:** that is not "all failed", so the answer stays an empty success; it cannot happen today, since OpenLibrary and Goodreads always make a request (ST-032). **Caching:** the error returns before the cache is written, and empty answers are never cached, so the all-failed case cannot be stored as "no results" (ST-033). |
| DB7 | REQ-108: box or not; failure shape. | closed, PM 2026-09-30 under the PO's standing direction; Readarr checked | No box: the button exists to delete the file, as in Readarr (PA-007). The record stays when removal fails and is removed when the file is already absent, as in Readarr. 409 with the reason rather than Readarr's 500, because Livrarr's 500 hides the message (ST-038). |

## 6. Acceptance Criteria

Frontend: component tests on the real component with its real controls. Only the network is
stubbed, through `fetch` or `frontend/src/test-support/apiStub.tsx` (ST-016). No hand-made component
state, and the save helper is never replaced. The EPUB tests serve a small valid EPUB so the real
toolbar exists (ST-011). Where jsdom cannot drive audio playback, use the project's existing browser
test tooling, not a stub of the component.

Server: the real router through `build_route_harness` in `tests/behavioral/test_ilr_contracts.rs`,
with root folders in temporary folders. Library items are written by the real
`SqliteDb::create_library_item`, the same writer import uses (ST-024); running an import per case is
out of proportion. "The work is gone" means `GET /api/v1/work/{id}` returns 404; every whole-book
removal case (AC-101 to AC-111) checks it. Single-file cases (AC-115 to AC-120) check instead that
the work remains (GET 200). Filesystem changes after seeding (a link, a folder, a moved root) stand for a
user or another tool changing the library after import.

"Red" = fails on `4786c21f`; "guard" = passes there and must keep passing.

As built: every criterion below is met, per code review r1's assessment table and r2's confirmation
(`build/reviews/errors-and-delete-pass/packet-3-code-review/`), except where a note says otherwise.
Server cases are in `tests/behavioral/test_ilr_contracts.rs`, `test_wh_deletion.rs` and
`test_consolidation_file_service.rs`. Frontend cases are in test files beside the changed
components and in `frontend/e2e-stubbed/` (browser tests with a stubbed network). The two
read-only-folder cases (AC-102b, AC-119) ran as a normal user, not skipped.

- [x] **AC-001** (REQ-001, red): for each search, a 500 and a network rejection each show the error
  and Retry, and no "No results". Book search keeps its library matches on screen. Retry sends the
  same term, language and raw flag, and the error clears when it succeeds.
- [x] **AC-002** (REQ-002, red): for EPUB and for PDF, both a 500 and a network rejection show "Could
  not load this book." and Retry, and not "Loading...". Retry fetches again; on success the error
  is gone. As built: the PDF case proves the error and the refetch only; a PDF opening after Retry
  is unproven (PO: out of scope).
- [x] **AC-003** (REQ-003, red): each of the five save sites, driven through its real control or
  event and the real helper, runs failure, failure, success, failure:
  - After the first failure: exactly one toast, "Could not save your place.".
  - After the second failure: still exactly one; the first toast stays while the reader or player
    sits idle past the pause.
  - After the success: no toast.
  - After the last failure: one new toast.
  - Every request keeps that site's current arguments.

  As built: met for EPUB and the three audio sites. The PDF site's browser test is still marked
  `fixme` (skipped); its wiring is checked by code review only. A further browser test,
  `frontend/e2e-stubbed/position-save-overlap.spec.ts`, pins the REQ-003 restart rule through the
  real audio player: it failed on the r1 helper and passes now.
- [x] **AC-004** (REQ-004, red): each rejected bookmark action, triggered from the real toolbar,
  shows its toast once.
- [x] **AC-005** (REQ-005, red): a failed list shows "Could not load notifications", not "No
  notifications". Retry reloads the list and shows it on success. Each of the three actions, when
  rejected, shows its toast once.
- [x] **AC-006** (REQ-006, red): with the whole-library read failing, History shows Retry and no
  spinner; Retry sends both reads again, and the page renders when they succeed.
- [x] **AC-007** (REQ-007, red): the import is driven through the real connect, preview and start,
  then:
  - The first two polls fail before any progress exists; the line shows once.
  - A successful poll removes the line and shows the progress.
  - Two more polls fail while the old numbers are still on screen; the line shows once.
  - The next successful poll removes it.
- [x] **AC-008a** (REQ-008, red): as admin, in both expanded and collapsed sidebars:
  - Cold failure: with an empty query cache, the first check fails and "Health check failed" (or
    the red dot with that title) shows; a later success shows the summary.
  - Stale failure: a success, then a failed refetch shows "Health check failed" and not the old
    summary; a later success restores the summary.
- [x] **AC-008b** (REQ-008, red): a non-admin makes no request to `/system/health-summary` and shows no
  widget.
- [x] **AC-009a** (REQ-009, red): the real `GET /api/v1/work/lookup?term=…`, with the ST-036 fixture.
  Google Books has no key and Hardcover search is off, so OpenLibrary and Goodreads are the
  attempted sources; both answer HTTP 500. Expected: 502, `error` `bad_gateway`, message "Every
  book source failed. Try again in a moment."
- [x] **AC-009b** (REQ-009, red): as AC-009a, and Hardcover is also attempted.
  - Setup: Hardcover is turned on and given a non-empty test token in the harness database's
    metadata configuration. That is the configuration discovery reads (`discovery_service.rs:1057-1082`;
    the harness hands its database to the discovery service at `test_ilr_contracts.rs:6463-6464`).
    The fixture's own `hardcover_search` switch alone does not reach discovery
    (`test_ilr_contracts.rs:6247-6249`). Google Books stays unconfigured.
  - Every attempted source fails by transport error (`ScriptedTransportOutcome::Error`).
  - Expected at the scripted transport: one request each to OpenLibrary, Goodreads and Hardcover,
    and none to Google Books.
  - Expected from the route: 502 with the same message.
- [x] **AC-009c** (REQ-009, guard): every attempted source answers 200 with no hits. Expected: 200,
  empty `results`.
- [x] **AC-009d** (REQ-009, guard): one source fails and another answers with hits. Expected: 200
  with the answering source's results (the existing check at `test_ilr_contracts.rs:6917-6930`
  stays green).
- [x] **AC-009e** (REQ-009): the same term first with every source failing (502, red), then with
  sources answering. The second call returns their results, so nothing empty was cached (guard).
- [x] **AC-009g** (REQ-009, guard): OpenLibrary answers 200 with an empty `docs` list, Goodreads
  fails, and Google Books and Hardcover are skipped. Expected from `GET /api/v1/work/lookup`: 200
  with empty `results`, not 502.
- [x] **AC-009f** (REQ-009, red): the real `POST /api/v1/manualimport/search` with every attempted
  source failing answers 502 with the same message.
- [x] **AC-101** (REQ-101, REQ-105): `DELETE /api/v1/work/{id}` without the parameter, and again with
  `deleteFiles=false`. The file still exists, `warnings` is empty, and the work is gone (all guard).
  `files_removed` is 0 (red).
- [x] **AC-102** (REQ-103, REQ-105, red): with `deleteFiles=true`, one work has four items:
  - a removable file;
  - a folder at the second item's path;
  - a removable file;
  - an item whose file is absent.

  Expected: a 200 response; GET the work returns 404; both files are gone and the folder survives;
  exactly one warning, naming the second item; `files_removed` is 2.
- [x] **AC-102b** (REQ-103, REQ-105, red): with `deleteFiles=true`, one work has four items:
  - a removable file;
  - a regular file whose folder is made read-only after seeding (a user or tool changing
    permissions after import), so the check passes but the removal fails;
  - a removable file in another folder;
  - an item whose file is absent.

  Expected: a 200 response and the work is gone. The two removable files are gone and the
  read-only one survives. Exactly one warning, naming that item. `files_removed` is 2.

  This needs an unprivileged runner. Run as root, the test must not pass silently: it prints a
  skip notice saying that root ignores file permissions.
- [x] **AC-103** (REQ-102, REQ-103, red): same root. Item A's file is replaced by a link to book B's
  file. Deleting A with `deleteFiles=true`: A's work is gone, B's file and work remain, there is
  one warning naming A, and `files_removed` is 0.
- [x] **AC-104** (REQ-102, REQ-103, red): the item's parent folder is a link to a folder outside the
  root. The outside file survives, one warning names the item, `files_removed` is 0, and the work
  is gone.
- [x] **AC-105** (REQ-102, REQ-103, red): the item's path is a link to a file outside the root. The
  outside file survives, one warning names the item, `files_removed` is 0, and the work is gone.
- [x] **AC-106** (REQ-102, REQ-103, red): with the root folder renamed away, `deleteFiles=true` gives
  one warning per item and `files_removed` 0, and the work is gone. The existing download and
  email error results are unchanged (guard).
- [x] **AC-106b** (REQ-102, REQ-103, red): usable root; after seeding, the item's parent folder is
  replaced by a link to itself (a link loop). `deleteFiles=true` gives a 200 response, one warning
  naming the item, and `files_removed` 0, and the work is gone.
- [x] **AC-106c** (REQ-102, REQ-103): same root, the item's parent folder missing. `deleteFiles=true`
  gives a 200 response, no warning, and `files_removed` 0 (red), and the work is gone (guard). For
  both this case and AC-106b, reading or emailing the item gives the same error result as on
  `4786c21f` (guard).
- [x] **AC-107** (REQ-103, red): a work with an ebook in the ebook root and an audiobook in the audiobook
  root: both files are gone, `files_removed` is 2, and the work is gone.
- [x] **AC-108** (REQ-103, red): an item whose stored path is `.` resolves to the root folder. The root
  and its contents remain, there is one warning, `files_removed` is 0, and the work is gone. This
  path cannot come from import; it pins the "never a folder" rule at the root boundary.
- [x] **AC-109** (REQ-101, red): `?deleteFiles=maybe` returns 400 and the work still exists.
- [x] **AC-110** (REQ-101, REQ-106, red): on each page, with the real `deleteWork` behind `apiStub`:
  - The box is unticked on opening, and the description is the unticked text.
  - Ticking shows the ticked text; the description is checked separately from the box label.
  - Unticking restores the unticked text.
  - Tick, then cancel while still ticked. Reopen without remounting the page: the box is unticked
    and the "stay on disk" description is back. Confirming without touching the box sends exactly
    `DELETE /work/{id}`, with no parameter.
  - Books page: after a completed ticked delete, a new selection opens the dialog unticked.
  - Confirming unticked sends exactly `DELETE /work/{id}` per chosen book; ticked sends exactly
    `DELETE /work/{id}?deleteFiles=true`.
  - On the book page, a reply with one warning shows one warning toast naming the file.
- [x] **AC-111** (REQ-106, red): on the Books page, three books selected, the box ticked. Two
  deletes succeed with seven warnings between them; one is rejected. One summary shows two
  deleted and one failed, with five warnings listed and "and 2 more".
- [x] **AC-115** (REQ-108, red): the real `DELETE /api/v1/workfile/{id}` on a regular file in a
  temporary root. The same book has a second item with its own file. Expected:
  - 200, the file is gone, and `GET /api/v1/workfile/{id}` returns 404;
  - `GET /api/v1/work/{work_id}` returns 200;
  - the second item's record and file remain;
  - one `fileDeleted` event is written.
- [x] **AC-116** (REQ-108, guard): the item's file is already absent under a usable root. Expected:
  200, the record is gone, one `fileDeleted` event is written, and `GET /api/v1/work/{work_id}`
  returns 200.
- [x] **AC-117** (REQ-108, red): same root. The item's file is replaced by a link to another book's
  file. Expected: 409, a message beginning with the item's relative path, the record still there
  (GET 200), the other file intact, and no `fileDeleted` event.
- [x] **AC-118** (REQ-108, red): the root folder is renamed away. Expected: 409 naming the item, and
  the record still there.
- [x] **AC-119** (REQ-108, red): the file's folder is made read-only after seeding. Expected: 409
  naming the item, and the file and record both still there. It needs an unprivileged runner; run
  as root, the test prints a skip notice saying root ignores file permissions.
- [x] **AC-120** (REQ-108, red): the real `LibraryFilesTab` behind `apiStub`.
  - The dialog description is "Delete this file from disk? This cannot be undone."
  - Confirming sends exactly `DELETE /workfile/{id}`.
  - A 409 reply shows exactly one error pop-up with the server's message.
  - A 200 reply shows "File deleted".
- [x] **AC-121** (REQ-108): the two tests in ST-039 are rewritten to seed a real root folder. Each
  still pins its record removal and its `fileDeleted` event (guard).
- [x] **AC-112** (REQ-107, guard): list-import undo of a book with a file on disk leaves the file.
  Undo must still pass "keep files" explicitly.
- [x] **AC-113** (REQ-102, guard): the existing tests of the ST-026 callers stay green.
- [x] **AC-114**: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (zero warnings),
  the touched behavioral binaries, the frontend tests for touched components, and the typecheck.

## 7. As built: corrections and limits

The feature was built in two streams, reviewed twice by GPT-6 Astra (r1 FAIL on one P2, r2 PASS),
deployed by hand and checked live by the PO on 2026-09-30. The PO ran the six live checks in
`build/reviews/errors-and-delete-pass/deploy-20260930T221942Z/LIVE-CHECK.md` and each did what it
should; the PM confirmed checks 1–4 (the delete changes) on disk and in the database.

### Corrections found during the build

What the spec got wrong or left out, as the builders and the reviewer found it. None of these
changed a decision or a requirement.

- **ST-029 missed five test callers** of `WorkService::delete`. They could not compile without the
  new argument: `tests/behavioral/test_wh_deletion.rs:252`, `test_wh_observer.rs:177`,
  `test_consolidation_work_service.rs:664`, `:690` and `:707`. Each now passes "keep files"
  (`false`), because two of those tests point their root folders at fixed `/tmp` paths they do not
  own. No assertion changed.
- **`crates/livrarr-metadata/src/convergence_service.rs` needed a trait bound.** Adding
  `RootFolderDb` to `WorkServiceImpl`'s database bound required the same bound in two places there.
  This file was not in the Stream B list. The change is for compilation only.
- **More code paths share the changed code than the spec listed.** Review r1 (§ "Doors, shared
  behavior and scope") added these. Lines are the reviewer's, on the built tree.

| Door | path:line | What it shares | As built |
|------|-----------|----------------|----------|
| Send to email | `crates/livrarr-handlers/src/work.rs:1808` | REQ-102 read rule, through `prepare_email` | results unchanged |
| Download | `crates/livrarr-handlers/src/work.rs:1856` | REQ-102 read rule, through `resolve_path` | results unchanged |
| Stream | `crates/livrarr-handlers/src/work.rs:1876` | REQ-102 read rule | results unchanged |
| Stream-token mint | `crates/livrarr-handlers/src/workfile.rs:127` | REQ-102 read rule | results unchanged |
| OPDS acquisition | `crates/livrarr-handlers/src/opds.rs:495` | REQ-102 read rule | results unchanged |
| Cross-format ebook parsing | `crates/livrarr-library/src/cross_format_service.rs:137` | REQ-102 read rule | results unchanged |
| Google Books author-batch search | `crates/livrarr-metadata/src/discovery_service.rs:584` | REQ-009's new "skipped" result | a skip maps back to an empty list; behaviour unchanged |
| Eager per-file match fallback | `crates/livrarr-metadata/src/discovery_service.rs:690` | REQ-009's new error | an error is still "no match" |

The review found no further production caller that deletes files. Readarr import undo and the
other file-removal paths are unchanged.

### Limits

Recorded and accepted; none is a known defect.

- **PDF paths are unproven end to end** (PO: out of scope). The PDF load error and Retry are
  tested, but a PDF opening after Retry is not. The PDF place-saving browser test is still marked
  `fixme` and skipped; that site's wiring is checked by code review only.
- **The check-then-remove race is accepted** ([§4](#4-non-requirements)).
- **Hidden-tab behaviour of the save warning is untested.** The REQ-003 restart rule waits on the
  pop-up library's dismiss callback and a 300 ms timer. A browser that throttles timers in a hidden
  tab may delay the warning; no test covers it. The rule also depends on the installed pop-up
  library (sonner 2.0.7) unmounting a leaving pop-up after 200 ms.
