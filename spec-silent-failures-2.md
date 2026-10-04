---
feature: silent-failures-2
stage: spec
status: delivered
version: 6
type: bugfix
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006]
---

# Bug Spec: silent-failures-2

## Executive summary

**Delivered.** All six fixes were built, passed code review on the third round, and were
deployed to the local app for the PO on 2026-10-04. They were committed as `596f4f85` and pushed
to `main` the same day. The PO did not report running the live check; after the deploy the PO
said "lets move on", and the PM committed and pushed. Every requirement is delivered and no
decision changed. Code review added five criteria for orderings this spec had not named
([AC-066 to AC-070](#added-in-code-review)). The [as-built section](#7-as-built-corrections-and-limits)
records what the spec missed, one accepted behaviour change, the limits of the evidence, and an
older audiobook defect the PO left for later.

Six places in the web app where something fails and the user is told nothing, or is told
twice. Each gets exactly one message, in a form the app already uses. All six are frontend
only: no server change, no new dependency, no new setting. The two "saved place" items and the
playback item are the largest, half a day to a day each with their browser tests
([sizes](#build-files-and-sizes)).

1. **Deleting a book from its book page fails: two error pop-ups.** After the fix there is one,
   showing the server's message, as the "Delete File" button already does ([REQ-001](#2-requirements)).
2. **Adding, renaming or deleting an audiobook bookmark fails: nothing.** After the fix each
   shows the same message as the EPUB reader. Starting the sleep timer today says "Bookmark
   saved" even when saving that bookmark failed; after the fix it says so only on success
   ([REQ-002](#2-requirements)).
3. **Pressing play on an audiobook the browser cannot play: the button switches to "pause"
   while nothing plays.** After the fix the button stays on "play" and one message says
   "Could not play this audiobook. Try reloading the page." The player's existing message for
   a broken stream uses the same pop-up, so one failure never shows two, and it also puts the
   button back on "play" if the stream breaks while listening ([REQ-003](#2-requirements)).
4. **The bell's unread count when checking for notifications fails.** Today the count vanishes
   on a first failure (looks like "nothing new") or freezes at the old number. After the fix the
   bell shows a small warning mark instead of the number, and opening it explains "Could not
   check for new notifications; still trying." No pop-up. The next successful check restores the
   count ([REQ-004](#2-requirements)).
5. **Opening an EPUB when loading the saved reading position fails.** Today the book opens at
   the beginning, and the first page turn overwrites the user's real saved place on the server
   (probed, [ST-018](#0b-system-truths)). After the fix the reader shows "Could not load this
   book." with two choices: **Retry**, and **Read from the beginning**, which opens the book with
   a note that progress is not being saved this time. Nothing is saved until the user leaves or
   reloads, so the real saved place survives ([REQ-005](#2-requirements)).
6. **Opening an audiobook when loading the saved listening position fails** (added by the PO).
   Today the player starts at 0:00 with no message, and listening then overwrites the saved
   place: the ten-second save while playing and the save after pausing both send it (probed,
   [ST-023](#0b-system-truths)). After the fix the player behaves as item 5: the same screen
   with **Retry** and **Play from the beginning**, and no save during a from-the-beginning
   session. Until the saved place is known, no button or key can start playback
   ([REQ-006](#2-requirements)).

For both readers, only the book now on screen can change what the reader shows: a late reply
about a book the user has already left changes nothing ([REQ-005, REQ-006](#2-requirements)).
Every way a request can fail is listed against a test in one
[table](#failure-shape-matrix).

**Decisions.** The PO decided item 5 on 2026-10-04: don't open the book, show Retry, and add a
way out of the error screen that does not save progress ([D4](#5-decisions)). This spec decides
the rest of that session's rules ([D6](#5-decisions)): bookmarks still save, the "Sync to
here" and resume-banner actions still work, and only the saved place itself is never written.
No PO decision is open.

**Spec-stage evidence.** Code reads at `457dc333`, and six scratch probes driving the real components
(`build/reviews/silent-failures-2/packet-0-spec/probes.log` for P1–P5, `probes-v2.log` for P6).
The probes reproduce today's faults. Only items 1, 2 and 4 also had their fix checked, and then
through a stand-in; the fixed paths of items 3, 5 and 6 are unprobed, because checking them
needs an edit to the reader itself. Real-browser behaviour of audio playback and epub.js was not
probed; the rules that depend on it are marked ([System Truths](#0b-system-truths)). As built,
the stubbed browser tests drive the fixed paths of items 2, 3, 5 and 6 in Chromium
([limits](#limits)).

## Revision

- v1, 2026-10-04: written from source at main `457dc333` per
  `build/reviews/silent-failures-2/packet-0-spec/PACKET.md`.
- v2, 2026-10-04: folds the PO's answers per `FOLD-PACKET-v2.md` in the same folder. v1 is kept
  at `build/reviews/silent-failures-2/packet-0-spec/spec-silent-failures-2-v1.md`.
  - D4: the PO chose option (a), "Don't open; show Retry", adding: "but avoid the loop? there
    should be a way to break out". Asked whether progress should be saved after taking the way
    out, the PO answered "Don't save (Recommended)". REQ-005 is rewritten to that; D6 is new.
  - Item 6 added: the PO approved adding the audiobook's saved-place failure, "same as before",
    confirmed as "Yes, add it, same behaviour". New problem statement 6, REQ-006, ST-022 to
    ST-027, probe P6, and acceptance criteria AC-029 to AC-045. The audio player's saved-place
    reads leave §4 Non-Requirements.
- v3, 2026-10-04: answers GPT-6 Astra's spec review r1 (FAIL; six P2, one P3) per
  `FOLD-PACKET-v3.md`. v2 is kept at
  `build/reviews/silent-failures-2/packet-0-spec/spec-silent-failures-2-v2.md`. No PO decision
  is reopened.
  - F1: REQ-006 lists every door that can start playback or save, with a criterion each
    (keys, chapter controls, both sleep stops). New ST-028.
  - F2: REQ-005 and REQ-006 add the current-item rule and late-reply cases for both formats.
    New ST-029.
  - F3, F4: REQ-003 makes a rejected start report on its own and a permanent stream failure
    reset the button; AC-018 now holds the start genuinely pending. New ST-030.
  - F5: the [failure-shape matrix](#failure-shape-matrix); D7 decides what counts as a usable
    saved place; ST-003 corrects the 401 statement. New ST-031.
  - F6: AC-016 is replaced by three phases timed against Sonner's exit window. New ST-032.
  - F7: the "How verified" column separates reproducing today's fault from checking the fixed
    path; ST-018's finished-state threshold is qualified.
  - AC-001 to AC-045 keep their numbers. AC-004, AC-013, AC-014, AC-018, AC-034, AC-037 and
    AC-041 gain cases; AC-016 is replaced; AC-046 to AC-065 are new.
- v4, 2026-10-04: answers GPT-6 Astra's closure check r2 (FAIL; two P2, one P3) per
  `FOLD-PACKET-v4.md`. v3 is kept at
  `build/reviews/silent-failures-2/packet-0-spec/spec-silent-failures-2-v3.md`. No requirement
  or decision changes.
  - F1: AC-060 records saves to both items after the change of item; item 1 has a distinctive
    saved place. ST-028 names which carried-over door keeps the old item's save function.
  - F2: AC-061 requires item 2's own banner to appear and stay, and nothing from item 1.
  - F5: ST-003, ST-007 and §6 state the 401 order and the per-call callback condition alike.
- v5, 2026-10-04: corrects the pop-up's exit window per
  `build/reviews/silent-failures-2/packet-5-tests-fix-r2/PACKET.md`. v4 is kept at
  `build/reviews/silent-failures-2/packet-0-spec/spec-silent-failures-2-v4.md`. No requirement,
  decision or other criterion changes.
  - ST-012: after the leaving pop-up's element is removed, Sonner still has about two animation
    frames pending that delete a new pop-up with the same id. The browser test showed this: a
    press with the page clock paused right after the element went lost its new pop-up.
  - REQ-003 and D2: the window in which a report may show nothing new extends to about two
    frames after the pop-up has gone.
  - AC-016 phase (3): the clock steps a further 100 ms after the last element has gone, before
    the press.
- v6, 2026-10-04: as built, per `build/reviews/silent-failures-2/packet-11-as-built/PACKET.md`.
  v5 is kept at `build/reviews/silent-failures-2/packet-0-spec/spec-silent-failures-2-v5.md`
  (sha256 `d9b3526d0fef144483a417eaf100eaac3fc99813805d1f38c84a9ea49ec273dc`). Status
  `delivered`. Sources: the commit `596f4f85`; the code hand-backs and evidence in
  `packet-4a-code/` (with `probes-r1/`), `packet-4b-code/`, `packet-7a-code-fix-r1/`,
  `packet-7b-code-fix-r1/`, `packet-9a-code-fix-r2/` and `packet-9b-code-fix-r2/`; the test fixes
  in `packet-5-tests-fix-r2/`; GPT-6 Astra's code reviews `packet-6-code-review/REVIEW-astra-code-r1.md`
  (FAIL: F1, F2, F3), `packet-8-code-review-r2/REVIEW-astra-code-r2.md` (FAIL: F1 narrowed, F3-R2)
  and `packet-10-code-review-r3/REVIEW-astra-code-r3.md` (PASS); the PM session log from
  2026-10-04T20:45Z; and `deploy-20261004T225130Z/DEPLOYMENT.json` and `LIVE-CHECK.md`. No
  requirement or decision changes.
  - Each requirement is marked Delivered, and every criterion is checked.
  - AC-066 to AC-070 are new: the orderings code review r1 and r2 found, each with its test.
  - New [§7](#7-as-built-corrections-and-limits): what the spec missed, the accepted behaviour
    change, the limits, and the older defect left for later. The files table records where the
    tests landed.

## 0a. Design Principles

- One message per failure, in the app's existing form; never two for one failure.
- A failed read must never lead to overwriting good data with a default.
- An error screen always offers a way forward that does not depend on the failing request.
- Fix where the failure is swallowed; where several doors report the same failure, they share
  one definition.
- Smallest change; frontend only; reuse the shipped wording and components.
- Tests drive the real component with only the network stubbed, and pin the named observable.

## 0b. System Truths

Read at `457dc333`. "P1"…"P6" are the probes in `probes.log` (P1–P5) and `probes-v2.log` (P6).
Premises taken from a packet are marked "(packet)" and were re-opened. In "How verified",
"reproduced" means a probe showed today's fault; "fixed path" says whether removing the named
cause was checked, and how. No fixed path was checked in a real browser.

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-001 | `ConfirmModal` awaits `onConfirm`; on success it closes, on a throw it shows `toast.error(err.message)` and stays open. Its confirm button is disabled and reads "Processing..." while pending. | `frontend/src/components/Page/ConfirmModal.tsx:34-47`, `:92`, `:99` | Changing `ConfirmModal` for this feature (12 callers). | read; P1 |
| ST-002 | (packet) The book-page delete has its own `onError` "Failed to delete work" and is called through `mutateAsync` from the dialog. A mutation runs its `onError`, then rethrows to the caller, so both report. P1: a 500, a network failure and a 404 each show two error pop-ups; the dialog stays open. | `frontend/src/pages/work-detail/WorkDetailPage.tsx:116-139`, `:138`, `:261-263`; query-core 5.96.0 `src/mutation.ts:273-325` | — | read; reproduced P1; fixed path: stand-in (the same dialog and call through a mutation with no `onError`) shows one pop-up, `probes.log:17` |
| ST-003 | Messages the user sees from `apiFetch`: network failure "Unable to reach Livrarr"; otherwise the server's `message`, which is "not found" for 404 and "Something went wrong" for 500; a non-JSON error body gets a fixed fallback. A 401 first clears the session, which at once marks the user signed out in the auth store, and then rejects with an error "Session expired". Error handling then runs as for any other status, under its usual conditions: a mutation's own `onError` runs whatever the caller does; a per-call `mutate(..., { onError })` callback runs only while its component is still mounted (ST-007). The pop-up container sits outside the signed-in routes, so a pop-up can show as the app signs out. | `frontend/src/api/client.ts:128-136`, `:51-97`, `:100-110`, `:28-31`; `frontend/src/stores/auth.ts:144-150`; query-core `src/mutation.ts:287-296`, `src/mutationObserver.ts:164`; `frontend/src/App.tsx:444-454`; `crates/livrarr-handlers/src/types/api_error.rs:456`, `:520-527` | — | read; P1 |
| ST-004 | A 200 with an empty body makes `apiFetch` return `undefined`. The delete's `onSuccess` destructures `{ warnings }`, throws a TypeError, and the mutation treats the delete as failed. P1: the user sees the TypeError text plus "Failed to delete work", although the server deleted the book. The route always sends `{ warnings }`. | `client.ts:113-125`; `WorkDetailPage.tsx:118`; `mutation.ts:246-251`, `:273-325`; `crates/livrarr-handlers/src/work.rs:1409-1411` | Reporting a completed delete as failed. | read; reproduced P1 |
| ST-005 | (packet) The audiobook bookmark add, delete and rename mutations have `onSuccess` only, and the app has no global mutation error handler. P2: each rejected action sends its request and shows nothing; with errors reported at the mutation cache each shows exactly one pop-up, so nothing else swallows them. | `frontend/src/pages/reader/AudioPlayer.tsx:110-132`; `frontend/src/App.tsx:70-79` | A global handler (prior spec DA3). | read; reproduced P2; fixed path: stand-in (errors reported at the mutation cache, not in the player) shows one pop-up each, `probes.log:33-35` |
| ST-006 | Doors into those mutations: Add bookmark `:803-820`; rename form `:899-904`; delete `:937-941`; the sleep bookmark `createSleepBookmark` `:319-350`, called by each "N minutes" option (`:698-708` → `:353-354`) and "End of chapter" (`:682-687`). (packet) The sleep path passes its own `onError` (a console warning, `:345`) and then shows `toast("Bookmark saved")` unconditionally (`:349`). It shares `createBookmarkMut`, so it falls inside item 2. P2: a failed sleep bookmark shows "Bookmark saved"; with the error reported it shows the error *and* "Bookmark saved". | as cited | Leaving "Bookmark saved" unconditional once errors are reported. | read; reproduced P2; fixed path: same stand-in, `probes.log:36` |
| ST-007 | Both a `useMutation` `onError` and a per-call `mutate(..., { onError })` run. Per-call callbacks run only while the component is mounted, and a later `mutate` on the same hook replaces the stored per-call callbacks. A 401 follows the same rules: the session is cleared first, then the rejection reaches the mutation's `onError`, and a per-call callback only if its component is still mounted (ST-003). | query-core `src/mutationObserver.ts:128-143`, `:161-175`, `:196-198`; `src/mutation.ts:287-296` | Relying on a per-call `onSuccess` for "Bookmark saved" if another create can start before it settles. | read |
| ST-008 | Every `play()` call in the player swallows its rejection: `togglePlay` `:235` (button `:610-619`, space key `:421-423`); the restore after a stream-address refresh `:277`; a chapter-list click `:852`. `togglePlay` sets `playing` to its opposite regardless (`:237`); the chapter click sets it true regardless (`:853`); the restore leaves it as it was. `playing` is set only by hand (`:237`, `:259`, `:370`, `:853`, `:984`); no listener follows the element's own play or pause. P3: a rejected `play()` turns the icon to Pause, with no message. | `AudioPlayer.tsx` as cited | — | read; reproduced P3 in jsdom with a stand-in `play()` rejection. Fixed path **unprobed**: reporting the rejection needs an edit to `AudioPlayer.tsx`, which the spec seat may not make (`probes.log:45`) |
| ST-009 | The existing media error door: `<audio onError>` → controller. The first error re-mints the stream address silently; the second calls `onPermanentFailure`, which shows `toast.error("Playback error — try reloading the page")` with no id. P3: two error events give exactly that, and pressing play afterwards still shows Pause. | `AudioPlayer.tsx:985`; `frontend/src/pages/reader/useStreamUrl.ts:49-51`, `:65-67`; `frontend/src/pages/reader/streamTokenController.ts:77-85` | Two pop-ups for one failure. | read; reproduced P3 with hand-fired error events (`probes.log:39-41`); fixed path **unprobed** (as ST-008) |
| ST-010 | When the stream address cannot be obtained, the player has no source: `useStreamUrl` does not pass the controller's `onInitialMintError`, and the controller keeps retrying with backoff. | `useStreamUrl.ts:33-52`; `streamTokenController.ts:40`, `:106-109`, `:131-141` | Calling `play()` with no source and trusting its outcome. | read. **Unverified:** what Chromium's `play()` does with no source (reject or stay pending); REQ-003 avoids depending on it. |
| ST-011 | The element preloads metadata, so a stream that cannot load usually fails, and shows the ST-009 message, before any press. A later `play()` then rejects. | `AudioPlayer.tsx:986`; HTML media rules | — | **unprobed** (needs a real browser; the packet allows vitest probes only) |
| ST-012 | Sonner 2.0.7: a pop-up lives 4000 ms by default; the timer pauses while the pointer is over the pop-ups. A re-issue under a live pop-up's id replaces it in place. A pop-up that starts leaving (expiry, or its close button) is marked `data-removed="true"` at once and unmounts 200 ms later; a same-id re-issue in that time is merged into the leaving one and lost. After the element is removed, about two animation frames are still pending that mark any pop-up with that id as deleted, so a same-id re-issue in those frames is lost too: removal calls `ToastState.dismiss`, which defers through `requestAnimationFrame`, and the toaster defers the deletion through another. The app's container sets no duration and shows a close button. | sonner `dist/index.mjs:146-160`, `:417`, `:425`, `:567-574`, `:600-609`, `:654`, `:1128-1136`, `:182-188`, `:947-966`; `wiki/patterns/error-handling.md:48-55`; `App.tsx:444-454` | Promising a new warning inside the 200 ms window or the two frames after it. | read |
| ST-013 | (packet) The bell's unread poll runs every 30 s and on window focus, and never reads its error. The count is `data?.length ?? 0`. The app's client retries a failed query once; the test client does not. P4: a first failure shows no badge, no message; after a success, a 500 and then a network failure both leave the old count; the query is in `error` state both times. A 200 with no body is already a query error; a 200 `{}` reads as zero. | `frontend/src/components/Header/NotificationBell.tsx:46-53`, `:157`, `:163-167`; `App.tsx:75`; `frontend/src/test-support/apiStub.tsx:90-94`; query-core `src/query.ts:560-567` | — | read; reproduced P4; fixed path: the probe read the query's own error state, which a fix would read; the bell's new mark itself is unprobed (`probes.log:50-62`) |
| ST-014 | The bell's full list is a separate request, made only while the bell is open, with its own "Could not load notifications" and Retry. | `NotificationBell.tsx:115-125`, `:193-206` | Two reports of one outage in one open bell. | read |
| ST-015 | Precedent for a background check: the sidebar health widget replaces the summary with a red "Health check failed" line on a first failure and on a failed refetch after a success, and restores it on the next success. No pop-up. | `frontend/src/components/Sidebar/Sidebar.tsx:373-407`; `Sidebar.test.tsx:121-166` | — | read |
| ST-016 | (packet) The EPUB reader reads the saved place once on mount and swallows any error. The reader is held at "Loading..." until that read settles, then opens at location 0. A 404 means "no saved place yet" and takes the same path. P5: a 500, a network failure and an empty 200 each open at location 0 with no message, exactly as a 404 does. | `frontend/src/pages/reader/EpubReader.tsx:173-181`, `:349-355`; `crates/livrarr-handlers/src/workfile.rs:62-77` | Showing an error for a 404. | read; reproduced P5 with a stand-in for the epub.js view. Fixed path **unprobed**: removing the fallback needs an edit to `EpubReader.tsx` (`probes.log:66-76`) |
| ST-017 | Every relocation with a percentage above 0 schedules a save 2 s later, however the location was reached. The kind is "seek" during the first 3 s after opening, otherwise "progress". This is the reader's only save door: `saveProgress` is its only caller of `savePosition`. | `EpubReader.tsx:125`, `:184-197`, `:690-706`; `rg savePosition\(` over `frontend/src/pages/reader/` | — | read; reproduced P5 (stand-in view) |
| ST-018 | The server replaces the stored position unconditionally. "Finished" is set when a save reaches 98 % and cleared when a later save is below 95 %, so a lower percentage from 99 % to 97 % keeps it. P5: one page turn after a failed load sends `PUT /workfile/1/progress` with an early-book position 2 s later, which replaces the user's real place. | `crates/livrarr-library/src/file_service.rs:246-281`; `crates/livrarr-db/src/sqlite_playback_progress.rs:81-102`, `:88-91`, `:112-133`, `:119-122` | Opening the reader with saving enabled after a failed read. | read; reproduced P5 (relocation fired by a stand-in for epub.js; whether epub.js fires it on the first page without a user action is unprobed). Fixed path **unprobed** (as ST-016) |
| ST-019 | The load-failure screen exists: `BookLoadError`, "Could not load this book." with Retry and no other action; the download's Retry re-runs the download only (the saved-place read's effect depends on the item id alone). The download failure screen is checked before the "Loading..." gate. `PdfReader` imports it and passes only `onRetry`. | `EpubReader.tsx:338-347`, `:156-171`, `:181`, `:819-835`; `frontend/src/pages/reader/PdfReader.tsx:5`, `:95-102` | Changing `BookLoadError`'s props in a way that breaks the PDF reader. | read |
| ST-020 | Test doors. Vitest: `installApiStub` replaces `fetch` only, so the real `apiFetch` runs; `AppToaster` is the app's container. Browser: the stubbed harness answers every `/api` request, including the `<audio>` stream, can abort one as a network error, serves a silent WAV, records every pop-up, and has a helper reading the EPUB's current position. The EPUB bookmark tests are the precedent for item 2; the audio save tests open `/listen/1?workId=1` with a 404 saved place. | `apiStub.tsx:47-88`; `frontend/src/test-support/toasts.tsx:9-20`; `frontend/e2e-stubbed/stubbedApi.ts:52-71`, `:106-167`, `:169-243`; `frontend/e2e-stubbed/position-saves.spec.ts:27-30`, `:100-134`, `:280-332`; `frontend/e2e-stubbed/readers.spec.ts:131-196` | A stubbed component or `@/api` module. | read |
| ST-021 | jsdom has no media playback and no `ResizeObserver` (the sleep-timer popover needs one), and epub.js cannot render there. | P2 failed with `ResizeObserver is not defined` until stubbed; P3, P5 and P6 needed stand-ins | Proving REQ-003, REQ-005 or REQ-006 in jsdom. | P2, P3, P5, P6 |
| ST-022 | (fold packet: "`fetchPrompt(0)` on any error", confirmed) The audio player reads the saved place twice. On mount (and on each change of item) it reads it, restores a number above 0, and then asks for the cross-format prompt; any error goes to `fetchPrompt(0)`, and a reply without a usable position starts at 0. On the element's first metadata load it reads it again and swallows any error. P6: a 500, a network failure and a body-less 200 each open at 0:00 with no message, exactly as a 404, with two reads and a prompt request with `current_ts=0`. | `AudioPlayer.tsx:162-190`, `:189`, `:178-187`, `:265-290`, `:281-288` | Leaving a second, separately swallowed read beside the fixed one. | read; reproduced P6 in jsdom with emulated media. Fixed path **unprobed**: needs an edit to `AudioPlayer.tsx` (`probes-v2.log:38`) |
| ST-023 | Every door that saves the listening position: the 2-second debounced `saveProgress`, called by pause (`:231-233`), seek (`:292-299`), the chapter-end sleep stop (`:257-262`) and the sleep timer's end (`:368-376`); the 10-second save while playing (`:207-221`); and the resume banner's Jump, which saves at once (`:956-968`). No save on unload: unmount clears a pending debounced save (`:223-227`). The server replaces an audiobook's position as for an ebook. P6: after a failed read, playing sends `PUT /workfile/1/progress` with position "12" from the 10-second save, and pausing sends it again 2 s later; pausing and leaving at once sends nothing. | `AudioPlayer.tsx` as cited, `:192-205`; `file_service.rs:263-280`; `sqlite_playback_progress.rs:165-179` | Any of these doors sending while the saved place is unknown. | read; reproduced P6 for the 10-second save and pause only (play emulated; finite duration supplied). Seek, both sleep stops and Jump: read only. Fixed path **unprobed** (as ST-022) |
| ST-024 | The player renders its controls at once; nothing waits for the saved-place read. P6: with the read held, pressing play and listening 10 s sends a save (position "9") before the read returns; the player then jumps to the saved place when it lands. | `AudioPlayer.tsx:466-488`, `:600-619`; P6 slow case | Playing, and so saving, before the read settles. | read; reproduced P6 (emulated media); fixed path **unprobed** (as ST-022) |
| ST-025 | Writes that are not the saved place: creating a bookmark inserts a bookmark row only; "Sync to here" sets the linked pair's furthest mark (it may go down) and clears both declines; the banner's Stay records a decline threshold for this format. None touches the stored position. | `crates/livrarr-library/src/bookmark_service.rs:36-69`; `crates/livrarr-db/src/sqlite_bookmarks.rs:74-96`; `crates/livrarr-library/src/cross_format_service.rs:237-283`; `crates/livrarr-db/src/sqlite_cross_format_state.rs:58-80`, `:112-126` | Blocking these in a from-the-beginning session on the ground that they overwrite the saved place. | read |
| ST-026 | The resume prompt is a read: the server compares the pair's furthest mark with `current_ts` and offers a later place, or nothing. Progress saves of kind "progress" carry `cross_format_ts` and raise the furthest mark. EPUB: the prompt fires once the reader is loaded, anchors are known and a first location is known; Jump only moves the reader, so its save goes through the relocation door. Audio: Jump saves directly (ST-023). Both readers' "Sync to here" ignores its outcome and shows "Position synced". | `cross_format_service.rs:163-225`; `sqlite_playback_progress.rs:76-79`, `:107-110`; `EpubReader.tsx:205-217`, `:768-781`, `:470-489`; `AudioPlayer.tsx:166-175`, `:952-976`, `:786-796` | — | read |
| ST-027 | The audio player can change item without unmounting: the stubbed save test opens the next audiobook by pushing `/listen/{id}` in the same page, and the mount read re-runs per item. The EPUB reader keys its read on the item id too. | `position-saves.spec.ts:438-445`; `AudioPlayer.tsx:190`; `EpubReader.tsx:181`; `frontend/src/pages/reader/ListenPage.tsx:50-57` | Keeping a from-the-beginning session across a change of item. | read |
| ST-028 | Player doors besides the play button. The key handler is registered on the document for the life of the player and ignores only key presses in text fields: Space toggles play (`togglePlay`), ArrowLeft and ArrowRight skip, the other keys change volume, mute, speed or fullscreen. Previous and next chapter, skip, and a bookmark row only move the place; a chapter-list entry moves and plays. The sleep timer and the "End of chapter" flag are not cleared on a change of item, and `playing` is never reset by one, so the 10-second save re-arms under the new item's id while it is true. The sleep timer keeps the save function of the item it was started on, whose id is fixed when it is made, while reading the shared media element; so at its end it pauses and saves the new item's time to the old item. The chapter-end stop and the 10-second save use the current item's id. | `AudioPlayer.tsx:411-460`, `:414-418`, `:421-430`, `:240-250`, `:582-598`, `:631-644`, `:849-853`, `:892-895`, `:353-380`, `:390-394`, `:257-262`, `:207-221`, `:192-205`, `:362-376`, `:252-263`; `ListenPage.tsx:51-56` | A rule that blocks only the play button. | read; the cross-item sends are read from code, unprobed |
| ST-029 | Late replies. Both saved-place reads write their result with no check that the item is still the one opened: the audio callback sets the time, moves the element and requests the prompt; the EPUB callback sets the location and marks the read done, and nothing resets that mark on a change of item. Both reader pages render the reader without an item key, and `/read/:id` and `/listen/:id` are single routes, so a same-page change of item keeps the component and its state. | `AudioPlayer.tsx:165-190`, `:170-174`, `:183-184`; `EpubReader.tsx:174-181`; `ReaderPage.tsx:49-54`; `ListenPage.tsx:50-57`; `App.tsx:126`, `:136` | Trusting a reply because it arrived last. | read; unprobed |
| ST-030 | Media rules (HTML standard; **unprobed** in Chromium, ST-021): a pending `play()` is rejected with `NotSupportedError` when the source fails, and with `AbortError` on `pause()` or when a new source loads; `play()` on an element whose source failed is rejected at once; loading a new source pauses the element. So a held stream reply keeps a start pending, and a stream-address refresh pauses playback until the restore at `:277` runs after the new metadata. The controller schedules a refresh `max(1 s, expiry − now − 5 min)` after each mint, so a token with a near expiry triggers a real refresh after 1 s. | `AudioPlayer.tsx:270-279`; `streamTokenController.ts:44-46`, `:93-129` | Treating a held start as settled; a test that pauses before a start is pending. | read; unprobed |
| ST-031 | Reply shapes. `apiFetch` casts parsed JSON without checking it and turns a body that is not JSON into "no body". The server's progress route always sends `position` as a string, possibly empty if a client stored one; the token route always sends `token` and `exp`. The player only ever stores `String(time)` of a finite time from the element, or the resume prompt's position. Today: a delete reply `{}`, `null` or a `warnings` that is not a list throws in `onSuccess`; an unread reply `{ "items": {} }` throws in the bell's effect, and `{ "items": "ab" }` counts as 2. | `client.ts:113-125`; `workfile.rs:62-77`, `:122-130`; `crates/livrarr-handlers/src/stream_token.rs:160-167`; `AudioPlayer.tsx:199-201`, `:212-216`, `:962-964`; `WorkDetailPage.tsx:118-136`; `NotificationBell.tsx:49`, `:57-59`, `:157` | Treating "the reply parsed" as "the reply is usable". | read |
| ST-032 | The browser harness's handler answers at once; a test holds a reply by adding its own route after `stubApi`, which then takes that request, as the overlapping-save test does. That test also pauses the page clock and steps it, and finds leaving pop-ups by `data-removed="true"`. | `stubbedApi.ts:26`, `:143-146`; `frontend/e2e-stubbed/position-save-overlap.spec.ts:31-74`, `:118-132`, `:162` | — | read |

## 0c. Prior Art

Searched `wiki/`, `docs/`, the project to-do list in `build/plans/`, the errors-and-delete-pass
spec and reports, the product-gap report and `CLAUDE.md`.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `spec-errors-and-delete-pass.md` REQ-108: "Delete File" on the files tab had the same double pop-up; the tab's own `onError` was dropped and the dialog's pop-up with the server's message kept (`frontend/src/pages/work-detail/components/LibraryFilesTab.tsx:35-42`, `:209-212`). | REQ-001 copies it on the same page, so both delete buttons there behave alike (D1). |
| PA-002 | Same spec REQ-004: EPUB bookmark messages, shipped at `EpubReader.tsx:98`, `:106`, `:117`, tested at `readers.spec.ts:131-196`. Its ST-006 and ST-007 named the audiobook mutations and the sleep-timer path, and left them out. | REQ-002 reuses the three strings exactly and the same test shape. |
| PA-003 | Same spec REQ-008 (health widget) and REQ-007 (import progress line "…; still trying."). | REQ-004 follows them: a status mark, no pop-up, cleared by the next success (D3). |
| PA-004 | Same spec REQ-002 (`BookLoadError`) and REQ-003 (fixed-id save warning with the wait-for-exit rule). | REQ-005 and REQ-006 reuse `BookLoadError`, extended with a second action. REQ-003 uses a fixed id but not the wait-for-exit rule ([D2](#5-decisions)). |
| PA-005 | `wiki/patterns/error-handling.md:43-55`: failed load → inline error with Retry; failed action → `toast.error` from the mutation's own `onError`; no global handler. | Basis for D1–D6. REQ-001 departs from "the mutation's own `onError`" because of PA-001 (D1). |
| PA-006 | The project to-do list in `build/plans/`, lines 125-143, "Noticed during errors-and-delete-pass", from `build/reviews/errors-and-delete-pass/packet-0-spec/REPORT.md:47-51`; product-gap report row C9 (`build/reports/ux-gap-scan-2026-09-26.md:71`). | Source of items 1–5. Item 6 is the PO's addition of 2026-10-04. |
| PA-007 | `CLAUDE.md` retro lessons (lines 234-241): fixed-id pop-up lifecycle, probe the "why it's silent" truths, list every input shape. | Applied in §6 and the probes. |
| PA-008 | The audio save tests in `position-saves.spec.ts:280-466` (seek bar, ten-second save, resume-banner Jump, next audiobook in the same page). | REQ-006's tests use the same harness and helpers; those tests answer the saved place with a 404 and stay green. |

## 1. Problem Statement

1. **Deleting a book from its book page, when the delete request fails** (server error, network
   failure, or the book already gone). Expected: one error. Observed: two error pop-ups, for
   example "Something went wrong" and "Failed to delete work" (ST-002). If the reply arrives with
   no body, the book is deleted but the user is told it failed (ST-004).
2. **In the audiobook player, adding, renaming or deleting a bookmark when the request fails.**
   Observed: nothing (ST-005). **Starting the sleep timer when saving its bookmark fails.**
   Observed: "Bookmark saved" (ST-006).
3. **Pressing play (button, space key, or a chapter in the chapter list) on an audiobook the
   browser cannot play** (the stream cannot load, cannot be decoded, or no stream address could
   be obtained). Expected: the button stays on play and the user is told. Observed: the button
   switches to pause; nothing plays (ST-008). **Listening when the stream breaks for good** (for
   example after a stream-address refresh fails): the message shows, but the button stays on
   pause although playback has stopped (ST-009, ST-030).
4. **The header bell when its 30-second check for unread notifications fails.** On a first
   failure there is no badge, which reads as "nothing new". After earlier successes the old count
   stays. Nothing says the check failed (ST-013).
5. **Opening an EPUB when reading the saved position fails** (server error, network failure,
   or a reply that gives no usable place, D7).
   Observed: the book opens at the beginning with no message, and the first page turn replaces
   the user's saved place on the server (ST-016 to ST-018).
6. **Opening an audiobook when reading the saved position fails** (server error, network
   failure, or a reply that gives no usable place, D7). Expected, for example: the user stopped at 3 h 10 min and is told the place could
   not be loaded. Observed: the player shows 0:00 with no message; pressing play and listening
   for ten seconds, or pausing, replaces the 3 h 10 min place on the server with a place near the
   start (ST-022, ST-023). A slow read has the same effect if the user presses play, or the
   space key, before it returns (ST-024, ST-028).

## 2. Requirements

Each requirement lists the real door its tests must drive. Messages are in [D5](#5-decisions).

- **REQ-001** **Delivered**. (item 1): When the book-page delete fails, the user sees exactly one error pop-up,
  the dialog's, carrying the server's message (ST-001, ST-003); the delete mutation's own
  `onError` is removed (`WorkDetailPage.tsx:138`). The dialog stays open, so the user can retry
  or cancel. A 200 reply always means the book was deleted (ST-031): a body that is empty, not
  JSON, `null`, `{}`, or has a `warnings` value that is not a list counts as a delete with no
  warnings: "Work deleted" and the move to the home page, as today for `{ warnings: [] }`.
  Unchanged: the success and warning pop-ups, the box and its texts, every other
  `ConfirmModal` caller. Door: the real `WorkDetailPage` with the real `deleteWork` behind
  `installApiStub` and `AppToaster`, in `WorkDetailPage.delete.test.tsx`.
- **REQ-002** **Delivered**. (item 2): The three audiobook bookmark mutations each get an `onError` pop-up with
  the EPUB reader's text: "Could not add the bookmark", "Could not rename the bookmark", "Could
  not delete the bookmark". The sleep bookmark (both doors in ST-006) shows "Bookmark saved"
  only after its create succeeds; on failure the user sees the add message once and no
  "Bookmark saved". The "Bookmark saved" must not depend on a per-call callback that a later
  bookmark action can replace (ST-007). The sleep timer itself starts whether or not the bookmark
  saves. These pop-ups have no fixed id, as on the EPUB reader, so each failure shows its own.
  Unchanged: the 60-second duplicate guards (`AudioPlayer.tsx:321-334`, `:348`). Door: the
  real `AudioPlayer` in the stubbed browser harness, beside the EPUB bookmark tests (ST-020); a
  vitest test of the real player with only `fetch` stubbed and a `ResizeObserver` shim is
  acceptable (ST-021).
- **REQ-003** **Delivered**. (item 3, D2): One definition reports a playback failure: a pop-up under the fixed id
  `playback-failed` with "Could not play this audiobook. Try reloading the page.", in
  `frontend/src/pages/reader/`. It is used by:
  - **every start that fails**: the three `play()` sites in ST-008 (play button and Space
    through `togglePlay`, a chapter-list entry, the restore after a refresh) and REQ-006's
    "Play from the beginning". When `play()` rejects, the button shows play (`playing` false)
    and the start door reports the failure itself, whether or not a broken-stream report has
    already appeared. A rejection named `AbortError` (the start was cut short by a pause or a
    new source, ST-030) is not a failure and changes nothing.
  - **pressing play, or a chapter, with no stream address** (ST-010), including while the
    token request is still unanswered. This reports without calling `play()`; the button stays
    on play.
  - **the controller's permanent failure** (ST-009), which today shows its own text without an
    id. It also sets the button to play, whether or not a start is pending and including after
    playback was running.

  The button still turns to pause at once when pressed and goes back to play only on a failure
  above. Because the reports share one id, one failure seen by several doors shows one pop-up
  (ST-012): a report while the pop-up is on screen replaces it. A report during the 200 ms
  while it is leaving, and for about two frames after it has gone, may show nothing new; the
  play button is the lasting signal there. After that, the next failure shows a new one; nothing reports only once. The pop-up
  expires after the normal 4 s. Unchanged: re-minting and its backoff, the silent first token
  failure while nobody presses play (§4), saves, and a refresh that restores position and
  playback. Door: the real `AudioPlayer` in the stubbed browser harness, with the stream
  answered by a failure, by unplayable bytes, by the silent WAV, or held (ST-020, ST-032). A
  refresh is driven by a token with a near expiry (ST-030), which reaches the restore's success
  path; a restore whose `play()` rejects has no reliable trigger, so review checks that it uses
  the shared definition.
- **REQ-004** **Delivered**, with the warning's rule made exact in review ([§7](#corrections-found-during-the-build)). (item 4, D3): While the bell's most recent unread check failed, including the very
  first one, the bell shows a warning mark in place of the count: a small red "!" with the
  accessible name "Could not check for new notifications". Opening the bell shows the line "Could
  not check for new notifications; still trying." above the list, unless the list itself is
  showing its own load error (ST-014), in which case only that error shows. No pop-up, ever, for
  this check. The next successful check restores the count and removes the mark and the line. A
  200 reply whose `items` is missing or not a list counts as a failed check (ST-031). Polling, its interval and the
  pop-ups for path-not-found and identity-review notifications are unchanged. Door: the real
  `NotificationBell` behind `installApiStub` with fake timers, as `Sidebar.test.tsx` does, in
  `NotificationBell.test.tsx`.
- **REQ-005** **Delivered**, with an identity per opening added in review ([§7](#corrections-found-during-the-build)). (item 5, D4, D6): The EPUB reader's saved-place read.
  - **Failure.** Any reply except a 404 that gives no usable saved place ([D7](#5-decisions)) is
    a failed read: a network failure, a server error, a client error other than 404, or a 200
    whose body is missing, not JSON, `null`, or has no `position` string. A 200 whose `position`
    is an empty string is "no saved place", as a 404. The reader does not open the book, so
    nothing can be saved. Once the download has arrived, it shows the saved-place error screen:
    `BookLoadError`'s "Could not load this book.", the line "Your saved place could not be
    loaded.", and two actions, **Retry** and **Read from the beginning**. Both show after every
    failed attempt. While the download is still arriving the reader shows "Loading..." as today.
  - **Download failure wins.** If the download failed, today's screen shows ("Could not load
    this book." with Retry only), whatever the saved-place read did. Its Retry re-runs the
    download, and the saved-place read too if that failed (today it re-runs the download only,
    ST-019).
  - **Retry** shows "Loading..." and reads the saved place again. On success the book opens at
    the saved place and saves as usual; on failure the error screen returns.
  - **Read from the beginning** opens the book exactly as for "no saved place" (location 0, then
    the first linear section, `EpubReader.tsx:726-728`) and starts a **from-the-beginning
    session**. Throughout it a note shows in the reader: "Your saved place could not be loaded.
    Progress is not being saved this time." The reader sends no `PUT /workfile/{id}/progress`
    for the rest of the session, by blocking its one save door (`saveProgress`, ST-017). The
    session ends when the reader closes (leaving the page), on a reload, or when the item
    changes (ST-027); the next open reads the saved place afresh. There is no Retry inside the
    session; reloading is the way to try again.
  - **In a from-the-beginning session** (D6): bookmarks add, rename and delete as usual; the
    resume banner appears as usual, its Jump moves the reader and sends no save, and its Stay
    records the decline as today; "Sync to here" works as today (ST-025, ST-026).
  - **Current item only** (ST-029). Only the current item's current attempt may change the
    reader's location, its loading or error screen, whether saving is allowed, or the resume
    prompt. A reply about an item the user has left changes nothing. On a change of item in the
    same page the reader shows "Loading..." again and reads the new item's saved place as on a
    first open; any from-the-beginning session ends (D6).
  - A 404 keeps today's behaviour: open at the beginning, no message, saving as usual.
  - Unchanged: the download error path apart from its Retry, the save helper, the PDF reader.
    `BookLoadError` gains an optional detail line and an optional second action; with only
    `onRetry` it renders as today, so the PDF reader (`PdfReader.tsx:97`) is untouched. Door:
    the real `EpubReader` in the stubbed browser harness serving `sample.epub` (ST-020); epub.js
    needs a browser (ST-021).
- **REQ-006** **Delivered**, with the first placement tied to each restore snapshot in review ([§7](#corrections-found-during-the-build)). (item 6, D4, D6): The audiobook player's saved-place read gets the same behaviour.
  - **One read.** The saved place is read once per opened item (and once per Retry). The first
    metadata load applies that read's result instead of making a second request (ST-022). The
    player starts at the saved place whichever arrives first, the read or the stream's metadata.
  - **Waiting.** Until the read settles the player shows its top bar and cover with
    "Loading..."; none of its other controls, panels or the resume banner show (ST-024). The
    stream address is still fetched meanwhile.
  - **No door plays or saves while the saved place is unknown**, that is while waiting, during a
    held Retry and on the error screen until the user chooses one of its two actions. The doors
    are in the table below (ST-023, ST-028); Space,
    ArrowLeft and ArrowRight do nothing then, and the 10-second save and both sleep stops send
    nothing, including when they carry over from the previous item after a change of item.
  - **Failure.** Any reply except a 404 that gives no usable saved place (D7) shows the
    saved-place error screen: a network failure, a server error, a client error other than
    404, or a 200 whose body is missing, not JSON, `null`, has no `position` string, or whose
    position is not, in full, a finite decimal number of 0 or more (for example "abc",
    "12abc", "-5", "Infinity", "NaN"). A 200 whose `position` is an empty string is "no saved
    place", as a 404. The screen shows "Could not load this book.", "Your saved place could not
    be loaded.", **Retry** and **Play from the beginning**, after every failed attempt. The cross-format prompt is not
    requested while the screen shows (today it is, with `current_ts=0`, ST-022).
  - **Retry** shows "Loading..." and reads again. On success the player opens at the saved
    place, requests the prompt with that time as today, and saves as usual.
  - **Play from the beginning** opens the player at 0:00, starts playback through REQ-003's
    start path (a failed start shows REQ-003's message and the button stays on play), requests
    the prompt with `current_ts=0` as for a 404, and starts a from-the-beginning session with
    the same note as REQ-005. The player sends no `PUT /workfile/{id}/progress` for the rest of
    the session: every save door in the table below (ST-023) passes one check in the player and
    sends nothing. The session ends as in REQ-005, including a change of item in the same page
    (ST-027).
  - **In a from-the-beginning session** (D6): bookmarks, including the sleep-timer bookmark, save
    as usual; the banner's Jump moves the player and sends no save; Stay and "Sync to here" work
    as today (ST-025, ST-026).
  - **Current item only**, as REQ-005 (ST-029): a reply about an item the user has left, to the
    saved-place read or to the prompt request, changes nothing; the player returns to waiting on
    a change of item.
  - A 404 keeps today's behaviour: open at 0:00, no message, prompt with `current_ts=0`, saving
    as usual. Door: the real `AudioPlayer` at `/listen/1?workId=1` in the stubbed browser
    harness with the silent WAV, as the audio save tests do (PA-008, ST-020).

  | Door (`AudioPlayer.tsx`) | Today | Saved place unknown | From-the-beginning session |
  |---|---|---|---|
  | Play button `:610-619` | plays; pausing saves | not shown: AC-034, AC-041 | pause sends nothing: AC-037 |
  | Space `:421-423` | as the play button | does nothing: AC-041, AC-057 | pause sends nothing: AC-037 |
  | ArrowLeft, ArrowRight `:425-430` | move only | do nothing: AC-041, AC-057 | move only, as today |
  | Previous, next chapter `:582-598`, `:631-644` | move only | not shown: AC-034 | move only: AC-037 |
  | Chapter-list entry `:849-853` | moves and plays | not shown: AC-034 | plays as REQ-003; no save: AC-037 |
  | Seek bar `:292-299` | saves after 2 s | not shown: AC-034 | sends nothing: AC-037 |
  | 10-second save `:207-221` | saves while `playing` | sends nothing: AC-060 | sends nothing: AC-037 |
  | Chapter-end stop `:257-262` | pauses, saves after 2 s | needs playback, which no door can start | pauses, sends nothing: AC-058 |
  | Sleep-timer end `:368-376` | pauses, saves after 2 s | sends nothing: AC-060 | pauses, sends nothing: AC-059 |
  | Banner Jump `:956-968` | moves, saves at once | not shown: AC-034 | moves, sends nothing: AC-039 |
  | Restore after a refresh `:274-278` | plays if it was playing | needs playback first | plays as REQ-003; no save of its own |

### Build files and sizes

| REQ | Production | Tests | Size |
|-----|------------|-------|------|
| REQ-001 | `frontend/src/pages/work-detail/WorkDetailPage.tsx` | `WorkDetailPage.delete.test.tsx` (tracked) | S, ~1 h |
| REQ-002 | `frontend/src/pages/reader/AudioPlayer.tsx` | a new `frontend/e2e-stubbed/` spec or `readers.spec.ts` | S, ~2 h |
| REQ-003 | `AudioPlayer.tsx`, `useStreamUrl.ts`, optionally one small new module in `pages/reader/` | a new `frontend/e2e-stubbed/` spec | M, ~6 h |
| REQ-004 | `frontend/src/components/Header/NotificationBell.tsx` | `NotificationBell.test.tsx` (tracked) | S, ~2 h |
| REQ-005 | `frontend/src/pages/reader/EpubReader.tsx` (reader and `BookLoadError`) | `readers.spec.ts` (tracked) | M, ~4 h |
| REQ-006 | `AudioPlayer.tsx`; imports `BookLoadError` from `EpubReader.tsx` as `PdfReader.tsx` does | `position-saves.spec.ts` (tracked) or a new `frontend/e2e-stubbed/` spec | M, ~7 h |

**Overlap:** REQ-002, REQ-003 and REQ-006 all edit `AudioPlayer.tsx`; give them to one coder,
REQ-006 first, because it changes what the player renders while loading. REQ-006 also depends on
REQ-005's `BookLoadError` change, and its way out uses REQ-003's start path; the same coder
should take REQ-005 or land it first. New files under `frontend/e2e-stubbed/` and
`frontend/src/` are not ignored by git.

**As built** (`596f4f85`): production as planned, with the one new module
`frontend/src/pages/reader/playbackFailure.ts` (REQ-003's reporter). `AudioPlayer.tsx` also
imports `readSavedPlace`, `SAVED_PLACE_DETAIL` and `FROM_START_NOTE` from `EpubReader.tsx`, so
both readers share one definition of the read and its texts. The browser tests went into new
tracked specs: `audio-bookmarks.spec.ts` (REQ-002), `audio-playback-failure.spec.ts` (REQ-003),
`epub-saved-place.spec.ts` (REQ-005) and `audio-saved-place.spec.ts` (REQ-006), all in
`frontend/e2e-stubbed/`, with new helpers `audioPlayer.ts` and `epubReader.ts`. The epub.js
position helpers moved from `position-saves.spec.ts` into `epubReader.ts`; `stubbedApi.ts`,
`frontend/src/test-support/apiStub.tsx` and `toasts.tsx` gained support code. REQ-001 and
REQ-004 stayed in the planned vitest files.

## 3. UI/Interface Design

No new screens. New texts are in D5. REQ-004 adds the mark on the bell and one line in its
popover. REQ-005 and REQ-006 reuse the load-error screen with one more line and one more button,
and add a note line inside the reader or player during a from-the-beginning session; it stays
for the session and has no close button. While its saved place is loading, the audio player
shows only its top bar and cover with "Loading...". No API change.

## 4. Non-Requirements

- No change to `ConfirmModal` or its other callers; the same double pop-up on ten other dialogs
  is recorded for later, not fixed here.
- No global error handler.
- The PDF reader's swallowed saved-position read, the "Position synced" pop-ups (shown even when
  the sync fails), the PDF reader, and the silent first stream-address failure on opening the
  player stay as they are.
- The player's button does not start following pauses made outside the player (for example,
  media keys).
- Nothing retries a failed play, bookmark, delete or saved-place read by itself, and a
  from-the-beginning session does not try the saved place again in the background.

## 5. Decisions

| ID | Decision | Status | Resolution |
|----|----------|--------|------------|
| D1 | Item 1: which pop-up stays; wording. | closed, spec | Options: (a) drop the page's `onError` and keep the dialog's pop-up with the server's message; (b) keep the page's message and swallow the error in `onConfirm`, which closes the dialog on failure; (c) change `ConfirmModal`, which touches 12 callers. **(a)**: one-line change, keeps the dialog open for a retry, and matches "Delete File" on the same page (PA-001). Departs from the wiki's "mutation's own `onError`" rule (PA-005) for that reason. Cost: a 404 reads "not found" and a 500 "Something went wrong", shown over the open "Delete Work" dialog. |
| D2 | Item 3: what the user sees; button state. | closed, spec | Options for the button: (a) turn to pause at once and back to play on failure; (b) wait for `play()` to settle; (c) follow the element's play and pause events. **(a)**: smallest, keeps the instant response, and does not depend on browser details we could not probe (ST-010, ST-011). Message: one pop-up under a fixed id shared with the existing broken-stream message, so one failure never shows two (ST-009, ST-012). Each failed start reports itself, so a start that fails before any broken-stream report is still told; a permanent stream failure also resets the button, since playback has stopped (ST-030). The save helper's wait-for-exit rule is not copied: the button is the lasting signal, so a lost re-issue inside 200 ms, or for about two frames after the pop-up has gone, costs little, and the next failure after that shows a new one (AC-016). |
| D3 | Item 4: how a failed check shows; when it clears. | closed, spec | Options: (a) mark on the bell plus an explanatory line when opened; (b) a pop-up once per failure run; (c) keep the old count and add a mark. **(a)**: no pop-up every 30 s, matches the health widget (PA-003, ST-015), and never shows a number we could not check. Clears on the next successful check. |
| D4 | Items 5 and 6: the message, and the saved position after a failed read. | **decided — PO, 2026-10-04** | v1 offered (a) don't open, show Retry; (b) open at the beginning with a warning, saving nothing until the saved place loads; (c) open at the beginning and save as usual. The PO chose **(a)**, "Don't open; show Retry", adding: "but avoid the loop? there should be a way to break out". On saving after the way out, the PO answered "Don't save (Recommended)". For item 6 the PO said "same as before", confirmed as "Yes, add it, same behaviour". Written into REQ-005 and REQ-006: the error screen with Retry and a way out on every failure; the way out opens at the beginning and saves no position for the rest of the session. |
| D5 | Wording across the six. | closed, spec | Short "Could not …" sentences, as in the shipped eight. REQ-001: the server's message (no new text). REQ-002: "Could not add the bookmark", "Could not rename the bookmark", "Could not delete the bookmark" (identical to the EPUB reader). REQ-003: "Could not play this audiobook. Try reloading the page." It replaces "Playback error — try reloading the page" so both doors say one thing. REQ-004: "Could not check for new notifications; still trying." and the mark's name "Could not check for new notifications". REQ-005 and REQ-006: "Could not load this book." (existing), the added line "Your saved place could not be loaded." so the way out makes sense, the buttons "Retry", "Read from the beginning" (ebook) and "Play from the beginning" (audiobook), and the session note "Your saved place could not be loaded. Progress is not being saved this time." |
| D6 | Items 5 and 6: what else happens in a from-the-beginning session. | closed, spec | **Session end:** leaving the reader, a reload, or a change of item (ST-027). Ending it on navigation keeps the rule simple and matches "this time" in the note. **Bookmarks** (including the sleep-timer bookmark) save as usual: they are separate records the user asks for, and creating one never touches the saved place (ST-025). **Resume banner:** shown as usual after the way out, because it reads the other format's furthest place and may lead the user back near their real place (ST-026). Its Jump moves the reader or player and sends no save, as the PO's "Don't save" requires; the EPUB's Jump already saves only through the blocked relocation door, and the player's direct save (`AudioPlayer.tsx:962`) passes the same check. Its Stay records a decline, which is not a position. **Sync to here** works as today: it is an explicit command and writes the pair's furthest mark, not the saved place (ST-025). Cost: syncing from an early place lowers the furthest mark, which is the command's documented purpose. **Furthest mark:** because no progress save is sent, the session's reading or listening does not raise the furthest mark (ST-026). **Audio while loading:** options (a) hold the player at "Loading..." until the read settles; (b) allow playing and only block saves until then. **(a)**: the same as the EPUB reader, and no partial state where a later failure must undo playback (ST-024). |
| D7 | Items 5 and 6: what counts as a usable saved place. | closed, spec | The server always sends `position` as a string (ST-031), so a reply of another shape did not come from it whole. **Usable:** EPUB, any non-empty string; audio, a string that is in full a finite decimal number of 0 or more ("0", "60", "12.5"). **No saved place, as a 404:** an empty string; there is no place to protect, and the server can store one only if a client sent it. **Failed read:** everything else: no body, not JSON, `null`, `{}`, a non-string `position`, and for audio "abc", "12abc" (today read as 12), "-5", "Infinity", "NaN". Options for audio were (a) whole-string number, (b) today's `parseFloat`, which accepts "12abc", (c) clamp bad values to 0, which reopens the overwrite. **(a)**: the player never stores anything else (ST-031). Cost: if such a value were ever stored, every open shows the error and the way out never replaces it; the user can still listen, and no client path writes one. Whether epub.js can display a given string is unchanged. |

## 6. Acceptance Criteria

"Red" = fails on `457dc333`; "guard" = passes there and must keep passing; "new" = pins new
behaviour that has no counterpart today. Count pop-ups the way the shipped tests do: wait for
the first, let a second one render, then count everything on screen and everything added
(`stubbedApi.ts:207-243`; `WorkDetailPage.delete.test.tsx:245-255`). "Held" means the test
holds that reply with its own route and answers it later; timed waits may step the paused page
clock (ST-032). "No save" means no `PUT /workfile/{id}/progress`. "Same page" means a change of
item by `pushState` and `popstate`, as `position-saves.spec.ts:440-443` does. A 401 first clears the
session, signing the user out, and then rejects like the other statuses; the mutation's own
error handling still runs, and a per-call callback only while its component is mounted (ST-003,
ST-007). The sign-out is unchanged and is not tested here, so a 401 has no criterion of its own. AC-001 to AC-045 keep
their numbers; v3 adds AC-046 to AC-065.

As built, every criterion is met: the PM's last runs at the reviewed revision passed the whole
stubbed browser suite (121 passed, 1 skipped, the skip being an older PDF case,
`position-saves.spec.ts:198`) and the whole vitest suite (241 tests in 35 files), with typecheck
exit 0 (`packet-9a-code-fix-r2/pm-rerun-suite.log`, `packet-9b-code-fix-r2/suite-vitest.log`,
`packet-9a-code-fix-r2/checks.log`). Code review added AC-066 to AC-070
([added in code review](#added-in-code-review)).

### Failure-shape matrix

One row per request; each cell names the criterion, or says why the shape does not apply.

| Request | Network failure | Server error | Client error | Empty or malformed body | Slow reply, user acts | Second failure while the first pop-up leaves |
|---|---|---|---|---|---|---|
| Book delete | AC-002 | AC-001 | AC-003 | AC-004, AC-046 | AC-005 | n/a: no fixed id; each failure shows its own |
| Bookmark add | AC-007 | AC-007 | AC-047 | n/a: reply not read | AC-012 | n/a: no fixed id |
| Bookmark rename | AC-047 | AC-008 | AC-047 | n/a: reply not read | AC-048 | n/a: no fixed id |
| Bookmark delete | AC-047 | AC-009 | AC-009 | n/a: reply not read | AC-048 | n/a: no fixed id |
| Sleep bookmark (both doors) | AC-047 | AC-010 | AC-047 | n/a: reply not read | AC-012 | n/a: no fixed id |
| Audio stream (`<audio>`) | AC-013 | AC-013 | AC-013 | AC-013 (bytes that are not audio) | AC-049, AC-018 | AC-016 |
| Stream token | AC-014 | AC-014 | AC-014 | AC-014 (no body); `{}`: n/a, the route always sends both fields (ST-031) and the controller is unchanged | AC-014 (held) | AC-016 (shared pop-up) |
| Unread check | AC-020 | AC-019, AC-020 | AC-052 | AC-021, AC-052 | n/a: the user has no action on the check; the mark changes only when a check completes | n/a: no pop-up |
| EPUB saved place | AC-023 | AC-023 | AC-053; 404 is "no saved place": AC-026 | AC-023, AC-053, AC-054 | AC-025, AC-027, AC-063, AC-064 | n/a: no pop-up; the screen stays until an action |
| Audio saved place | AC-034 | AC-034 | AC-055; 404: AC-042 | AC-034, AC-044, AC-055, AC-056 | AC-036, AC-041, AC-057, AC-060, AC-061, AC-062 | n/a: no pop-up; the screen stays until an action |

**REQ-001** (vitest, real page behind `installApiStub`, `AppToaster` mounted):
- [x] **AC-001** (red): the delete answers 500: exactly one error pop-up, "Something went wrong";
  no "Failed to delete work"; the dialog is still open.
- [x] **AC-002** (red): network failure: exactly one error pop-up, "Unable to reach Livrarr".
- [x] **AC-003** (red): 404: exactly one error pop-up, "not found".
- [x] **AC-004** (red): 200 with no body: no error pop-up; one "Work deleted"; the page moves to
  the home page.
- [x] **AC-005** (red): slow reply: the delete is held, the user cancels the dialog, then the reply
  is a 500: exactly one error pop-up.
- [x] **AC-006** (guard): `{ warnings: [] }` shows "Work deleted"; the existing warning test stays
  green.
- [x] **AC-046** (red): 200 answering `{}`, `null`, `{ "warnings": "x" }`, and a body that is not
  JSON: each shows no error pop-up and exactly one "Work deleted", and the page moves to the home
  page.

**REQ-002** (real player, stream answered with the silent WAV):
- [x] **AC-007** (red): Add bookmark answered 500, and again with a network failure: each shows
  exactly one "Could not add the bookmark".
- [x] **AC-008** (red): rename answered 500: exactly one "Could not rename the bookmark".
- [x] **AC-009** (red): delete answered 500, and again 404: each shows exactly one "Could not
  delete the bookmark".
- [x] **AC-010** (red): "5 minutes" with the create answered 500: exactly one pop-up, "Could not
  add the bookmark"; no "Bookmark saved"; the timer's remaining time shows. The same for "End of
  chapter" with one chapter served.
- [x] **AC-011** (guard): "5 minutes" with the create succeeding: exactly one "Bookmark saved".
- [x] **AC-012** (new): slow reply: the sleep bookmark's create is held; the user presses Add
  bookmark, which succeeds; then the sleep create succeeds. Exactly one "Bookmark saved" and no
  error.
- [x] **AC-047** (red): Add bookmark answered 400; rename answered with a network failure and
  with 404; delete answered with a network failure; "5 minutes" with the create answered with a
  network failure and with 400. Each shows exactly one matching message, and the sleep cases
  show no "Bookmark saved" and the timer's remaining time.
- [x] **AC-048** (red): rename and delete each held; the user closes the bookmark panel; the
  reply is a 500: exactly one matching message.

**REQ-003** (stubbed browser harness):
- [x] **AC-013** (red): the stream answers 404 on every request. After the page settles, press
  play: within 2 s the button shows play, not pause, and exactly one pop-up is on screen, "Could
  not play this audiobook. Try reloading the page." The same with the stream answering 500,
  aborted as a network error, and answering bytes that are not audio.
- [x] **AC-014** (red): the stream-token request answers 500, so there is no stream address:
  press play; the button shows play and exactly one such pop-up shows. The same with the token
  request answering with a network failure, 404, a 200 with no body, and while it is held.
- [x] **AC-015** (red): as AC-013 with one chapter served: clicking the chapter leaves the button
  on play and shows exactly one such pop-up.
- [x] **AC-016** (red): the pop-up's life, in three phases, with the page clock paused and the
  setup of AC-049, the error re-mint's token request held throughout so no broken-stream report
  interferes. (1) **Live:** after the first failed press shows the pop-up, press play again:
  exactly one such pop-up on screen, none added by the second press, and the button shows play.
  (2) **Leaving:** press the pop-up's close button; as soon as it carries `data-removed="true"`,
  and within 100 ms of page time, press play: the button shows play; a new pop-up may or may not
  appear, and no more than one such pop-up is on screen apart from the leaving one. (3) **Gone:**
  step the clock until no element with that text remains, then a further 100 ms, then press
  play: exactly one new such pop-up is added and on screen, and the button shows play.
- [x] **AC-017** (red): no press; two media failures reach the controller (AC-013's stream). The
  one pop-up now carries the shared text.
- [x] **AC-018** (guard): with the silent WAV, press play: the button shows pause, the media
  element is playing, and no pop-up is added. Then, in a fresh page with the stream held so the
  start is still pending (ST-030), press play and then pause: the button shows play and no pop-up
  is added; after the stream is answered with the WAV, the element stays paused.
- [x] **AC-049** (red): a failed start reports itself. The first stream request and the error
  re-mint's token request are held; the saved place answers 404. No pop-up shows before the
  press. Press play (the button shows pause), then answer the stream with 404: before the token
  request is answered, the button shows play and exactly one "Could not play this audiobook. Try
  reloading the page." is on screen. The same with Space (focus on the page body) and with a
  chapter-list entry (one chapter served). Then answer the token and let its stream answer 404:
  still exactly one such pop-up on screen and none added.
- [x] **AC-050** (red): a permanent failure after playback was running. The first token expires
  soon, so a refresh follows after 1 s (ST-030); its stream is the silent WAV. Press play and
  confirm the element is playing. The refresh's token and the error re-mint's token, both with
  far expiries, get streams answering 404: exactly one such pop-up shows and the button shows play.
- [x] **AC-051** (guard): a refresh that restores playback: as AC-050, but the refresh's stream
  is the silent WAV. After the refresh the element is playing, its time is at least the time
  read just before the refresh, the button shows pause, and no pop-up is added.

**REQ-004** (vitest, fake timers, real bell):
- [x] **AC-019** (red): the first check answers 500: no count; the mark named "Could not check
  for new notifications" shows. Opening the bell (full list answering an empty page) shows "Could
  not check for new notifications; still trying." No pop-up.
- [x] **AC-020** (red): first check: 2 unread, and the count shows "2". Next check: 500, so the
  count is gone and the mark shows. Next: network failure, and the mark still shows. Next: 1
  unread, so the count shows "1" and the mark and line are gone. No pop-up at any step.
- [x] **AC-021** (red): a check answering 200 `{}` shows the mark.
- [x] **AC-022** (new): the check has failed and the bell is open while the full list also fails:
  "Could not load notifications" with Retry shows and the "still trying" line does not.
- [x] **AC-052** (red): a first check answering 404, a 200 with no body, `{ "items": {} }`, and
  `{ "items": "ab" }`: each shows the mark and no count, and no pop-up.

**REQ-005** (stubbed browser harness, `sample.epub`):
- [x] **AC-023** (red): the saved-place read answers 500: "Could not load this book.", "Your
  saved place could not be loaded.", Retry and "Read from the beginning" show; the book does not
  open; no save within 5 s. The same for a network failure and for a 200 with no body.
- [x] **AC-024** (red): from AC-023, Retry sends the saved-place read again. It answers the saved
  position, and the book opens there (the rendition's start position is that position,
  `position-saves.spec.ts:100-134`); a page turn then saves as usual.
- [x] **AC-025** (red): Retry is held 2 s and then fails with a 500: "Loading..." shows while it
  is held, then the error screen returns with both actions, and no save is sent.
- [x] **AC-026** (guard): the read answers 404: the book opens at the beginning with no message,
  and a page turn saves as today.
- [x] **AC-027** (guard): slow read: held 3 s, then the saved position. "Loading..." shows
  throughout, no save is sent before it opens, and it opens at the saved position.
- [x] **AC-029** (new): from AC-023, "Read from the beginning": the book opens at the same start
  as for a 404, and the note "Your saved place could not be loaded. Progress is not being saved
  this time." shows. Turn the page three times over 5 s, then wait 5 s: no save, and the note
  still shows.
- [x] **AC-030** (new): in that session, Add bookmark sends `POST /workfile/1/bookmarks` and no
  save follows within 5 s.
- [x] **AC-031** (new): in that session, with anchors and a resume prompt served, press Jump on
  the banner: the reader moves to the prompt's position and no save follows within 5 s.
- [x] **AC-032** (new): the session ends on reload: reload with the read now answering the saved
  position; the book opens there, the note is absent, and a page turn saves as usual.
- [x] **AC-033** (red): the download answers 500 and the saved-place read answers 500: "Could not
  load this book." with Retry only, no "Read from the beginning". Retry with both now answering
  opens the book at the saved position (today it opens at the beginning).
- [x] **AC-053** (red): the read answers 403, and a 200 with `null`, `{}`, `{ "position": 12 }`
  and `{ "position": null }`: each shows the two-action error screen and no save within 5 s.
  From the `{}` case, Retry answering the saved position opens the book there; from the
  `{ "position": 12 }` case, "Read from the beginning" opens with the note, and three page turns
  send no save.
- [x] **AC-054** (guard): a 200 `{ "position": "" }` opens at the beginning with no message, and a
  page turn saves as usual.
- [x] **AC-063** (new): a late success after a change of item. Item 1's read is held; open item 2
  in the same page; its read answers 500; choose "Read from the beginning"; then answer item 1's
  read with its saved position. The reader stays at item 2's start (the rendition's position is
  not item 1's), the note still shows, and three page turns over 5 s send no save to either item.
- [x] **AC-064** (new): a late failure after a change of item. As AC-063, but item 2's read
  answers its saved position and the book opens there; then item 1's read answers 500. No error
  screen shows, the book stays at item 2's position, and a page turn saves to
  `/workfile/2/progress`.
- [x] **AC-065** (new): the session ends on a change of item. From AC-029's session on item 1,
  open item 2 in the same page with its read answering a saved position: "Loading..." shows until
  it opens there, the note is absent, and a page turn saves to `/workfile/2/progress`.

**REQ-006** (stubbed browser harness at `/listen/1?workId=1`, silent WAV; "the controls" means the
play button, previous and next chapter, the chapter-list toggle, the seek bar, the sleep timer,
Add bookmark and the resume banner, with two chapters and a resume prompt served):
- [x] **AC-034** (red): the saved-place read answers 500: "Could not load this book.", "Your
  saved place could not be loaded.", Retry and "Play from the beginning" show, and none of the
  controls is present; no save and no cross-format prompt request within 12 s. The same for a
  network failure, a 200 with no body, and a 200 whose position is "abc".
- [x] **AC-035** (red): from AC-034, Retry answers the saved position "60": the player opens,
  the media element's time is 60 once its metadata has loaded, the prompt request carries
  `current_ts=60`, and the saved place was requested exactly once per attempt. Pressing play and
  waiting 11 s sends a save as usual.
- [x] **AC-036** (red): Retry is held 2 s and then fails with a 500: "Loading..." shows while it
  is held, then the error screen returns with both actions, and no save is sent.
- [x] **AC-037** (new): from AC-034, "Play from the beginning": the media element is playing from
  0, the button shows pause, the note shows, and the prompt request carries `current_ts=0`. Then
  play 12 s; pause with the button and wait 3 s; play and pause with Space (focus on the page
  body) and wait 3 s; move the seek bar and wait 3 s; press next chapter; click the first
  chapter-list entry, which plays, then pause and wait 3 s: no save at any point.
- [x] **AC-038** (new): in that session, Add bookmark sends `POST /workfile/1/bookmarks`, and "5
  minutes" on the sleep timer sends its bookmark and shows "Bookmark saved"; no save follows
  within 3 s.
- [x] **AC-039** (new): in that session with a resume prompt served (position "12.5"), press
  Jump: the media element's time is 12.5 and no save follows within 3 s.
- [x] **AC-040** (new): the session ends on a change of item: open item 2 in the same page as
  `position-saves.spec.ts:438-445` does, its read answering 404; press play and wait 11 s: a save
  to `/workfile/2/progress` is sent. The same on reload of item 1 with the read now answering
  "60": it opens at 60 and saves as usual.
- [x] **AC-041** (red): slow read: held 12 s, then "60". "Loading..." shows throughout and none
  of the controls is present; pressing Space, ArrowLeft and ArrowRight with focus on the page
  body leaves the element paused at time 0; no save and no prompt request is sent while it is
  held; the player then opens at 60.
- [x] **AC-042** (guard): the read answers 404: the player opens at 0:00 with no message, and the
  existing audio save tests in `position-saves.spec.ts:280-466` stay green.
- [x] **AC-043** (new): "Play from the beginning" while the stream answers 404: the button shows
  play and exactly one pop-up shows, REQ-003's "Could not play this audiobook. Try reloading the
  page."; the note shows.
- [x] **AC-044** (guard): a 200 whose position is "0" opens at 0:00 with no message and saves as
  usual; it is a known place, not a failure.
- [x] **AC-045** (new): the saved place is requested once per open: with a 200 "60" and no
  Retry, exactly one `GET /workfile/1/progress` is sent by the time the media element's metadata
  has loaded (today two, ST-022).
- [x] **AC-055** (red): the read answers 403, and a 200 with `null`, `{}`, `{ "position": 60 }`,
  `{ "position": null }`, and the positions "12abc", "-5", "Infinity" and "NaN": each shows the
  error screen, with no save and no prompt request within 12 s. From the `{}` case, Retry
  answering "60" opens at 60; from the "12abc" case, "Play from the beginning" plays from 0, and
  12 s of playing and a pause send no save.
- [x] **AC-056** (guard): a 200 whose position is "" opens at 0:00 with no message, requests the
  prompt with `current_ts=0`, and saves as usual.
- [x] **AC-057** (red): on AC-034's error screen, and again during a Retry held 12 s, pressing
  Space, ArrowLeft and ArrowRight with focus on the page body leaves the element paused at time
  0; no save and no prompt request is sent.
- [x] **AC-058** (new): in a from-the-beginning session with the first chapter ending at 3 s,
  choose "End of chapter" on the sleep timer and play: the element pauses at the chapter end and
  no save follows within 3 s.
- [x] **AC-059** (new): in a from-the-beginning session, choose "5 minutes" and play; step the
  page clock past 5 minutes: the element pauses, the timer ends, and no save follows within 3 s.
- [x] **AC-060** (red): doors carried over a change of item. Item 1's read answers "300" (the
  served WAV is longer than that), and item 1 plays from there with "5 minutes" on the sleep
  timer; step the clock 4 min 50 s. Open item 2 in the same page with its read held, and from
  that moment record every save to any item. Wait until item 2's metadata has loaded, so the
  media element holds item 2's stream; then step the clock 20 s, past the timer's end, and 3 s
  more for the 2-second wait before a save. Item 2's read stays held throughout. No save to
  `/workfile/1/progress` or `/workfile/2/progress` is recorded. A save to item 1 with a position
  below 300 would be item 2's time written over item 1's place (ST-028).
- [x] **AC-061** (new): a late success after a change of item. Item 1's read is held; open item 2
  in the same page; its read answers 500; choose "Play from the beginning". Item 2's prompt
  request (`current_ts=0`) is answered with a prompt whose label only item 2 uses, and that
  banner appears. Then answer item 1's read with "60". The element's time is not moved to 60;
  the note still shows; item 2's banner still shows with the same label; no banner from item 1
  appears and no prompt request for item 1 is sent after the change; and 12 s of playing and a
  pause send no save to either item.
- [x] **AC-062** (new): a late failure after a change of item. Item 1's read is held; item 2's
  read answers "30" and the player opens at 30; then item 1's read answers 500. No error screen
  shows, the controls stay, and playing 11 s sends a save to `/workfile/2/progress`.

### Added in code review

Each pins an ordering that code review found and the criteria above did not name; each test was
red before its fix and green twice after it.

- [x] **AC-066** (REQ-006; review r1, F1): the saved place "60" is known before any stream
  metadata, and the first usable metadata comes from another stream. (a) The first stream is
  held and then answers 404, so the player re-mints; (b) the first token expires soon and the
  first stream is held, so a proactive refresh's stream loads first. In both, the media
  element's time is 60, the saved place was requested exactly once, and playing sends a periodic
  save from there. Tests: "a stream re-minted after the first one fails starts at the saved
  place, and saves from there" (`frontend/e2e-stubbed/audio-saved-place.spec.ts:873`) and "a
  stream refreshed before the first one loads starts at the saved place, and saves from there"
  (`:902`).
- [x] **AC-067** (REQ-006; review r2, F1): a refresh snapshot taken before the saved place was
  applied, whose mint finishes after it. The saved place answers "60"; the first stream and the
  refresh's token request are both held; the first stream is released and the element is at 60;
  then the token is released and the refreshed stream's metadata loads. The element is still at
  60, the saved place was requested exactly once, and playing sends a periodic save from there.
  Test: "a refresh asked for before the saved place was applied keeps the saved place, and saves
  from there" (`audio-saved-place.spec.ts:939`).
- [x] **AC-068** (REQ-005; review r1, F2): returning to an ebook in the same page. (a) Item 1
  fails its read and the user chooses "Read from the beginning"; item 2 opens with its read and
  download held; item 1 opens again with its new read held. "Loading..." shows, no book and no
  note show, and nothing saves while it is held; the read then fails, the error screen shows and
  nothing saves; after "Read from the beginning" again the note shows and page turns save to
  neither item. (b) The same return after item 1 was open at its saved place: "Loading..." while
  the new read is held and no save; once it answers, the book opens at the saved place and a page
  turn saves. Tests: `after a "Read from the beginning" session on A and a visit to B, A waits for
  its new read and saves nothing` (`frontend/e2e-stubbed/epub-saved-place.spec.ts:652`) and
  "after A was open at its saved place and a visit to B, A waits for its new read" (`:694`).
- [x] **AC-069** (REQ-004; review r1, F3): after a first failed check, the mark and the "still
  trying" line stay while the next poll is held, after that poll fails, and while a focus
  refetch is held; when it succeeds the count shows "1" and both are gone. No pop-up. Test:
  "after a first failed check, the mark and the line stay while the next check is pending, until
  a check succeeds" (`frontend/src/components/Header/NotificationBell.test.tsx:404`).
- [x] **AC-070** (REQ-004; review r2, F3-R2): with `Date.now()` held still, so every check
  finishes at one clock time, a success ("2"), then a failure (mark and line, no count), then a
  success ("1", no mark, no line). No pop-up. Test: "with every check finishing at the same clock
  time, a failure after a success shows the mark and the line until the next success"
  (`NotificationBell.test.tsx:474`).

**All:**
- [x] **AC-028**: the vitest files for the touched components, the touched `e2e-stubbed` specs
  (`pnpm exec playwright test --config playwright.stubbed.config.ts`) and `pnpm typecheck`
  pass. No Rust file changes.

## 7. As built: corrections and limits

The feature was built by two coders in one checkout, both `fix` subagents on Claude Opus 5.5:
packet 4a took REQ-002, REQ-003, REQ-005 and REQ-006, and packet 4b took REQ-001 and REQ-004.
Packet 5 fixed three browser tests and folded spec v5. GPT-6 Astra (xhigh, sole reviewer)
reviewed the code three times: r1 FAIL (F1 and F2 at P1, F3 at P2), r2 FAIL (F1 narrowed at P1,
F3-R2 at P3), r3 PASS. The fixes were packets 7a and 7b, then 9a and 9b. Only the web app's files
were deployed, to the local app at 2026-10-04T22:51Z, with no server restart and no database
change (`build/reviews/silent-failures-2/deploy-20261004T225130Z/DEPLOYMENT.json`). The PO did
not report running the live check in `LIVE-CHECK.md`; after the deploy the PO said "lets move
on", and the PM committed `596f4f85` and pushed it to `origin/main`.

### Corrections found during the build

What the spec missed, as the builders and the reviewer found it. None of these changed a
decision.

- **The player had to tie each restore snapshot to whether the saved place was already applied
  (review r1 F1, r2 F1).** REQ-006 said the first metadata load applies the read's result, and
  ST-022 and ST-030 treated a stream refresh's restore as a separate path. When the saved place
  was known before any metadata, a stream that the controller re-minted after an error, or
  refreshed before expiry, supplied the first metadata, and the restore put back its snapshot's
  0 instead; the save door then allowed saves from 0 (r1). After the first fix, a refresh whose
  snapshot was taken before the place was applied, and whose token arrived after, still restored
  0 (r2). As built, `placedSeqRef` records which read's place is on the element
  (`frontend/src/pages/reader/AudioPlayer.tsx:211-214`). The first usable metadata from any
  stream applies it (`:459-472`), and the one save check requires it (`:351-367`). Each refresh
  snapshot is tagged when it is taken (`frontend/src/pages/reader/useStreamUrl.ts:54`, `:65`,
  `:70`; `AudioPlayer.tsx:236-238`), and a snapshot from before placement restores the applied
  place (`AudioPlayer.tsx:474-486`). The stream-token controller is unchanged. Tests AC-066 and
  AC-067.
- **The player ignores metadata from another item's stream (packet 7a).** While a new item's
  token is minted, the element can still load the previous item's stream. Metadata whose source
  is not the current item's stream path is ignored (`AudioPlayer.tsx:445-449`, using
  `streamPath`, `useStreamUrl.ts:9`). Astra r2 judged this appropriate; no browser test drives a
  late metadata event from the previous stream.
- **The ebook reader needed an identity per opening, not per item (review r1 F2).** ST-029 and
  REQ-005 keyed results on the item. Opening A, then B, then A again in the same page reused A's
  earlier result while A's new read was outstanding; after a from-the-beginning session on A,
  the note showed again while saves were allowed. As built, each opening has its own id,
  including a return to an item opened before (`frontend/src/pages/reader/EpubReader.tsx:101-107`).
  The download and saved-place results count only for their opening (`:129-139`). The save permit
  is cleared when a read starts, set only on a current success, and checked both when a save is
  asked for and when it is sent (`:122-125`, `:255-279`, `:291-312`). Astra r2 found the audio
  player free of this shape: each change of item overwrites its one saved-place slot with
  "loading" before the read (`AudioPlayer.tsx:287-290`). Test AC-068.
- **The bell's warning follows the query's outcome and its error count, not `isError` or clock
  times (review r1 F3, r2 F3-R2).** ST-013 assumed the query's error state lasts until the next
  success. TanStack Query puts an errored query with no data back to `pending` when it refetches,
  and `isError` turns false
  (`frontend/node_modules/.pnpm/@tanstack+query-core@5.96.0/node_modules/@tanstack/query-core/src/query.ts:639`,
  `:701`), so the mark vanished during the next check (r1). The first fix compared
  `errorUpdatedAt` with `dataUpdatedAt`, which cannot order a success and a failure that finish
  in the same millisecond (r2). As built, the warning shows when `status` is `error`, or `pending`
  with `errorUpdateCount` above 0 (`frontend/src/components/Header/NotificationBell.tsx:50-53`,
  `:169-177`). Tests AC-069 and AC-070.
- **Three browser tests were wrong at the code stage (fixed in `packet-5-tests-fix-r2/`).** No
  correct code could pass them. AC-016 phase (3) pressed play before Sonner's two pending frames
  after removal had run, so the new pop-up was deleted; v5 records this in ST-012, and the test
  steps the clock 100 ms first (`frontend/e2e-stubbed/audio-playback-failure.spec.ts:444`).
  AC-050 waited for a pop-up under a paused page clock that it never stepped, and Sonner adds
  each pop-up from a timer (`sonner@2.0.7` `dist/index.mjs:969`); the test steps 50 ms
  (`audio-playback-failure.spec.ts:557`). AC-031 expected Jump to land exactly on a page start
  measured with no banner and no note on screen; with them, pages break differently, and Jump
  landed on the same spot in a normal session. The test now checks that the reader's displayed
  range contains the prompt's position (`frontend/e2e-stubbed/epub-saved-place.spec.ts:454`,
  helper `renditionShows`, `frontend/e2e-stubbed/epubReader.ts:34`). Each fixed test still fails
  for the wrong fix it guards against (`packet-5-tests-fix-r2/mutation-ac016.log`,
  `mutation-ac050.log`, `mutation-ac031.log`).

### Accepted behaviour change

A position save already queued for the previous audiobook is dropped on a change of book: the
one save check rejects an item that is no longer on screen when the save is sent
(`AudioPlayer.tsx:359-365`, reached when the 2-second wait ends, `:379-381`). The app already dropped a queued save on leaving the player. Astra
r1 judged this acceptable: the spec forbids carried-over doors from writing the new book's time
to the old one, and nothing requires the old save to be sent.

### Limits

- **No real-browser run against the live app.** The PO did not report running the live check
  (`deploy-20261004T225130Z/LIVE-CHECK.md`). All browser evidence is the stubbed harness.
- **Chromium only.** Other browsers' media and epub.js behaviour is untested.
- **A refresh restore whose start is rejected has no test trigger.** Review confirmed that it
  uses the shared reporter (REQ-003).
- **A snapshot taken before placement while playing** is probed at state level only, by Astra
  r3's `packet-10-code-review-r3/reviewer-state-probes.cjs`; the browser case (AC-067) holds a
  paused player.
- **The bell with the app's own query client** (one retry) is probed at state level only by
  Astra r3; the vitest client does not retry.

### Older defect left for later

After a stream refresh fails and its one recovery works, the audiobook stays stopped with the
button on pause and no message. The 4a probe showed it with a start still pending when the
refresh fired (`build/reviews/silent-failures-2/packet-4a-code/probes-r1/probe-2.log`, third
case). The cause, per the coder, is that the recovery records whether audio was playing only
after the failed swap has paused it (`frontend/src/pages/reader/streamTokenController.ts:94`);
that file is unchanged by this feature. The PO decided on 2026-10-04 to "Leave for later
(Recommended)"; it is recorded as unapproved in the project's running to-do list (`build/plans/`).
