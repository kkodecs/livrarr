# Settings pages

## Executive summary

What a future change to Settings must keep true, as built by settings-honesty (`fcbcb080`,
2026-10-06). The [Metadata page](#metadata) is the one with real rules: its Test buttons test the
**saved** settings, so Test asks the user to save first whenever anything on the page differs
from what is saved; Save locks the page until both of its requests settle; and a save's reply is
stored only after any older read of the same settings is cancelled. [Media Management](#media-management)
gives one account of where files go, and [Indexers](#indexers) has one working search box. Which
settings pages a normal user sees is in [UI: menu, routes and roles](ui-architecture.md#menu-routes-and-roles).
Full requirements and their tests: `spec-settings-honesty.md` v6.

## Media Management

- The "File locations" box holds one sentence on where imports put files: ebooks as
  Author/Title.ext and audiobooks keeping their own file names inside an Author/Title folder,
  under the user's number in the root folder for that type; Readarr Import puts every file in the
  root folder chosen on its page, as Author/Title.ext
  (`frontend/src/pages/settings/media-management/MediaManagementPage.tsx:577-595`). The Root
  Folders help tip points to it rather than restating it (`:344`). If import placement changes,
  change this sentence with it.
- The page no longer reads `GET /config/naming`. That route, its stored row and the web wrapper
  `getNamingConfig` remain with no web caller (unapproved clean-up in the project to-do list).
  Configurable naming is GitHub #120, not built.

## Indexers

- Interactive Search is an ordinary checkbox in both forms and is sent on create and update. Book
  search asks only indexers that are enabled with it on
  (`crates/livrarr-db/src/sqlite_indexer.rs:85-94`).
- Automatic Search is not shown and not sent; the server defaults it on create and keeps the
  stored value on update. Nothing reads it. The unused hook `frontend/src/hooks/useIndexerForm.ts`
  still carries the field.
- The dialog only edits a stored indexer; the inline form on the page is the only way to add one.

## Metadata

- **Test buttons** for Hardcover, Audnexus and the AI connection
  (`frontend/src/pages/settings/metadata/MetadataPage.tsx:121`) call the admin-only test routes,
  which read the saved settings and ignore the Enabled boxes. 200 shows a success pop-up; any
  other reply shows the server's message.
- **Save first.** The page has unsaved changes when any form value differs from the saved
  settings, whichever control wrote it (typed fields, Enabled boxes, the AI Provider select and
  "Back to preset models" included), or the language list differs from the saved list in order,
  or the default language differs from the saved one (`:238`). Changing a value back counts as no
  change. With unsaved changes, Test sends nothing and shows "Save your changes first. Test checks
  the saved settings."
- **Two-part save.** Save sends the service settings and, when it changed, the default language,
  as two requests. Each part counts as saved only when its own request succeeds; a failed part
  keeps what the user entered. After a successful service save the secret fields are blank.
- **Save lock.** From Save until both requests settle, every settings control is disabled
  (`:241`), so a reply always describes what the user sees.
- **Cancel before store.** Each save part cancels any in-flight read of its settings query, waits
  for the cancellation, then stores the reply (`:181-182`, `:193-194`). Storing the reply with
  `setQueryData` alone let an older background read (for example on window focus) put the
  pre-save value back ([lesson 110](../insights/coding-patterns.md#lesson-110)). The cancelled
  HTTP request still completes; its reply is ignored.
- Saving languages here does not refresh the language list in Add New and the header search
  until their next read (unapproved item in the project to-do list).

## Source

`spec-settings-honesty.md` v6 (REQ-301, REQ-401, REQ-501, AC-528, §8);
`build/reviews/settings-honesty/packet-7-code-review/REVIEW-astra-code-r1.md` (C1).
