# Saved reading and listening place

## Executive summary

The ebook reader and the audiobook player never overwrite a saved place they could not read.
As built on 2026-10-04 (`596f4f85`), each one blocks every position save until the current
opening's saved place is known ([the rule](#the-rule)). If the read fails, it shows "Could not
load this book." with **Retry** and a way out that opens at the beginning and saves no progress
for that session ([failed read](#when-the-read-fails)). A reply that arrives after the user has
moved to another book, or for an earlier opening, changes nothing ([late replies](#late-replies)).
Change these readers only with the [tests](#tests-and-limits) green; the evidence is Chromium in a
stubbed harness, not the live app.

## The rule

The server replaces a stored position unconditionally (`crates/livrarr-library/src/file_service.rs:246-281`).
So a reader that opens at the start after a failed read, and then saves, destroys the user's real
place. Both readers therefore hold one save check, and every save door passes it:

- **Ebook:** `saveProgress` is the reader's only save door. It sends nothing unless the save
  permit is the current opening, checked when the save is asked for and again when the 2-second
  wait ends (`frontend/src/pages/reader/EpubReader.tsx:291-312`). The permit is cleared when a
  read starts and set only when the current read succeeds (`:255-279`).
- **Audiobook:** `saveNow` sends a save only for the item on screen, only when its saved place
  is known, and only once that place has been put on the media element
  (`frontend/src/pages/reader/AudioPlayer.tsx:347-369`). The pause and seek saves, the 10-second
  save, both sleep stops and the resume banner's Jump all go through it (`:380`, `:394`, `:1186`).
  Until the place is known the player shows only its top bar and cover with "Loading...", and
  Space and the arrow keys do nothing (`:615-620`).

Both readers read the saved place once per opening and once per Retry, through one shared
definition, `readSavedPlace` (`EpubReader.tsx:65-85`). A 404 or an empty `position` means "no
saved place": open at the start and save as usual. Any other error, or a reply without a
`position` string, is a failed read. The player also rejects a position that is not, in full, a
finite number of seconds of 0 or more (`AudioPlayer.tsx:63-71`).

The audiobook applies the saved place on the first usable metadata from any stream, including a
stream re-minted after an error or refreshed before its token expires (`AudioPlayer.tsx:459-472`).
Each refresh snapshot records whether the place was already applied when it was taken
(`frontend/src/pages/reader/useStreamUrl.ts:54-70`), so a snapshot from before placement does not
put the element back to 0 (`AudioPlayer.tsx:474-486`).

## When the read fails

Both readers show `BookLoadError` with "Could not load this book.", "Your saved place could not
be loaded.", **Retry**, and **Read from the beginning** (ebook) or **Play from the beginning**
(audiobook) (`EpubReader.tsx:957-965`; `AudioPlayer.tsx:671`). Retry reads again. The way out
opens at the start and begins a from-the-beginning session: a note says "Your saved place could
not be loaded. Progress is not being saved this time." and no position is saved until the user
leaves, reloads or opens another book (`EpubReader.tsx:281-289`; `AudioPlayer.tsx:332-345`).
Bookmarks, "Sync to here" and the resume banner work as usual; the banner's Jump moves without
saving. If the ebook's download also failed, the download error wins; its Retry re-runs the download,
and the saved-place read too if that failed.

## Late replies

Only the current opening's current attempt can change what a reader shows or whether it may
save. The ebook gives every opening its own id, including a return to a book opened before in
the same page, and its download and saved-place results count only for that opening
(`EpubReader.tsx:101-107`, `:129-139`). The player numbers each read and ignores a reply for an
older one (`AudioPlayer.tsx:287-292`); a change of book pauses playback and returns to waiting
(`:320-330`). Metadata from the previous book's stream is ignored (`:445-449`). A save already
queued for the previous audiobook is dropped on a change of book.

## Tests and limits

The browser tests are in `frontend/e2e-stubbed/epub-saved-place.spec.ts` and
`frontend/e2e-stubbed/audio-saved-place.spec.ts`. Each criterion they cover, including the
orderings found in code review, is listed in `spec-silent-failures-2.md` §6 and §7.

Limits: Chromium only; no run against the live app. A refresh snapshot taken before placement
while audio is playing is probed at state level only. The PDF reader still swallows a failed
saved-place read and was out of scope. An older defect remains: after a stream refresh fails and
its one recovery works, the audiobook stays stopped with the button on pause
(`build/plans/TODO.md`, unapproved).

## Source

`spec-silent-failures-2.md` v6 (REQ-005, REQ-006, §7); commit `596f4f85`. Related:
[cross-format resume](cross-format-resume.md), [errors and recovery](../patterns/error-handling.md).
