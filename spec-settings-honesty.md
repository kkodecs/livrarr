---
feature: settings-honesty
stage: spec
status: delivered
version: 6
type: bugfix
req_ids: [REQ-101, REQ-102, REQ-201, REQ-301, REQ-401, REQ-501, REQ-601, REQ-701, REQ-901]
---

# Bug Spec: settings-honesty

## Executive summary

**Delivered.** All eight remaining items were built, passed code review on the second round,
were deployed to the local app on 2026-10-05, and the PO answered the live check "ok looks
good" on 2026-10-06. They are committed locally as `fcbcb080`, not yet pushed. Every requirement is
delivered and no decision changed. One premise was wrong: the indexer dialog only edits; adding
an indexer is done in the inline form alone ([ST-009](#0b-system-truths)). Code review added three
criteria for behaviours this spec had not named: a save's reply is not undone by an older
settings read, a refreshed language list resets a stale language pick, and an open pop-up follows
a role change ([added in code review](#added-in-code-review)). The [as-built section](#8-as-built)
lists the files changed, the new route, the new shared parts and what stays unused.

Eight small fixes so the web app stops showing controls that do nothing and stops hiding ones
that work. The PO decided nine items on 2026-10-05 and later dropped the setup-wizard hint (item
8), because the wizard never reaches that step ([§4](#4-non-requirements)); this spec turns the
remaining decisions into testable requirements. One server change adds a language list any signed-in user may read; one tightens
the password rule. Everything else is web-app only. No new dependency, no schema change, no
change to any Rust function or trait signature ([ST-022](#0b-system-truths)).

1. **Menu: six "Coming soon" items go.** Bookshelf, Calendar, Cutoff Unmet, Profiles, Custom
   Formats and Development leave the menu and their placeholder pages go; the empty "More"
   heading goes with them. General, Notifications and Tags stay greyed ([REQ-101](#2-requirements)).
   An address that does not exist, including the removed ones, now says "This page does not
   exist." instead of "coming in a future release" ([REQ-102](#2-requirements)).
2. **Settings → UI: the Theme section goes.** Its only working choice was Dark ([REQ-201](#2-requirements)).
3. **Settings → Media Management: the greyed Naming and File Management boxes go.** One line
   says truthfully where imports put files (Readarr Import differs: it uses the root folder chosen
   on its page), and the Root Folders help tip is reworded to agree.
   Today the box says files are not renamed, but ebooks are ([REQ-301](#2-requirements),
   [ST-005](#0b-system-truths)).
4. **Indexer form: Interactive Search can be switched on and off; Automatic Search goes.** Livrarr
   has no automatic search, so that box and its "Auto" badge described nothing ([REQ-401](#2-requirements)).
5. **Settings → Metadata: a Test button for Hardcover, Audnexus and the AI connection.** Each tests
   the saved settings. With unsaved changes, including a changed AI provider or model and a
   default language whose save failed, it asks the user to save first. Today, saving a new token
   leaves the page believing it still has unsaved changes, and changing the provider does not
   count as a change at all (both probed); the fix covers both ([REQ-501](#2-requirements)).
6. **Unmapped Files gets an admin-only menu link under Activity**, and its broken "Go to Settings"
   link is fixed, together with Manual Import's identical "Configure root folder" link
   ([REQ-601](#2-requirements)).
7. **Normal (non-admin) users no longer see five admin pages**: Manual Import, Readarr Import,
   Media Management, Status and Logs. Settings opens on UI for them. On Add New and in the header
   search they now see the languages the admin enabled; today they silently get English only
   (probed). They never receive the other metadata settings ([REQ-701](#2-requirements)).
8. **Setup wizard: dropped by the PO.** Its later steps are never shown ([§4](#4-non-requirements)).
9. **Passwords need 8 characters everywhere.** The server now refuses 6 or 7 when a password is
   set; the 1,024-byte maximum is unchanged. Existing shorter passwords and sessions keep working
   ([REQ-901](#2-requirements)).

**Decisions.** No PO decision is open. The PO approved the three adjacent one-line fixes the v1
spec raised (the "Page Not Found" text, Manual Import's link, the Root Folders help tip) and put
the other normal-user gaps on the to-do list ([§4](#4-non-requirements), [§5](#5-open-questions)).

**Evidence.** Code read at `6f74c6ff`; ten scratch probes (`build/reviews/settings-honesty/packet-0-spec/probes.log`)
and the PM's setup probe (`pm-probe-setup.log`). Each System Truth says what was probed, read or
left unprobed and why. Every fixed path is unprobed because checking it needs the code change. Sizes total about two
days with tests ([files](#build-files-and-sizes)).

## Revision

- v1, 2026-10-05: written from source at main `6f74c6ff` per
  `build/reviews/settings-honesty/packet-0-spec/PACKET.md`. Requirement IDs use the packet's
  letter prefixes; `verify.py` expects digits (see `BLOCKED-spec.md` in that folder).
- v2, 2026-10-05: folds `build/reviews/settings-honesty/packet-0-spec/FOLD-PACKET-v2.md`. v1 is
  kept at `build/reviews/settings-honesty/packet-0-spec/spec-settings-honesty-v1.md`.
  - IDs: the PM resolved `BLOCKED-spec.md` (the packet's letter prefixes were wrong). Requirements
    use digit groups per item (menu 1xx, theme 2xx, naming 3xx, indexers 4xx, test buttons 5xx,
    Unmapped 6xx, normal users 7xx, wizard 8xx, password 9xx); criteria and decisions follow the
    same groups (AC-111 is v1's AC-M1, D-301 is v1's D-N1, and so on).
  - Q-1, PO (a): new REQ-102 and AC-121 to AC-123, "This page does not exist." on the catch-all.
  - Q-2, PO (a): REQ-601 also fixes Manual Import's link; new AC-614.
  - Q-3, PO (a): REQ-301 rewords the Root Folders help tip; new AC-312 and D-302.
  - Q-4, PO: the other normal-user callers of admin-only routes and the guided tour are not in
    scope; recorded in §4.
  - §5 holds no open question.
- v3, 2026-10-05: folds GPT-6 Astra's spec review round 1 (`REVIEW-astra-spec-r1.md`, FAIL, F1 to
  F8) per `FOLD-PACKET-v3.md`. v2 is kept at
  `build/reviews/settings-honesty/packet-0-spec/spec-settings-honesty-v2.md`. Probes P5 to P10 added.
  - F1, PO 2026-10-05 (a): item 8 dropped; REQ-801 and AC-811 removed; ST-020 records that the
    wizard never reaches its later steps (PM probe); §4 lists it.
  - F2: ST-006 and D-301 corrected: Readarr Import uses the root folder chosen on its page; files
    move on a merge, not on a title or author edit, and the sentence promises neither. AC-311
    pins the new wording and its absence of an edit promise.
  - F3: REQ-501 and D-501 count every form writer, including the AI Provider select and "Back to
    preset models"; ST-013 records P9; new AC-519 to AC-521.
  - F4: REQ-501 and new D-503 define a save as two parts; AC-516 and AC-517 rewritten; new AC-522
    and AC-523.
  - F5: new AC-415 to AC-417; matrix maps both add doors, both protocols and untouched edits.
  - F6: REQ-901 and AC-911 to AC-916 add the multi-byte boundary on every door, label ASCII
    fixtures, and pin that a refused change leaves the old password and an existing session working.
  - F7: System Truths now say per clause what was probed (P1 to P10, PM probe), read or unprobed
    and why.
  - F8: ST-018 corrected: the real refusal carries `forbidden`.
- v4, 2026-10-05: folds GPT-6 Astra's spec review round 2 (`REVIEW-astra-spec-r2.md`, FAIL: F7
  open, F9 to F11, F12) per `FOLD-PACKET-v4.md`. v3 is kept at
  `build/reviews/settings-honesty/packet-0-spec/spec-settings-honesty-v3.md`. Probes P11 and P12
  added. Round 1's other findings stay closed.
  - F7: ST-011 and ST-013 now cite a probe for each causal clause (P11, P12 for the cleared
    Audnexus field, both language flags and the two independent writes) or say why a clause is
    structural or unprobed.
  - F9: REQ-501 and D-501 define both language "changed" states as comparisons with their saved
    values, the language list compared in order; new AC-525 and AC-526.
  - F10, PM's choice: while a Save is in progress every settings writer on the Metadata page is
    disabled until both writes settle (REQ-501, D-504); new AC-524.
  - F11: new failure guards AC-721 and AC-722 for Add New and the header search.
  - F12: AC-416 relabelled as a guard; AC-417 stays red.
- v5, 2026-10-05: folds GPT-6 Astra's spec review round 3 (`REVIEW-astra-spec-r3.md`, F10 only)
  per `FOLD-PACKET-v5.md`. v4 is kept at
  `build/reviews/settings-honesty/packet-0-spec/spec-settings-honesty-v4.md`. D-504 is unchanged;
  new AC-527 checks the save lock in every state where only one write is still pending, and the
  matrix maps those states. Nothing else changes.
- v6, 2026-10-06: as built, per `build/reviews/settings-honesty/packet-9-as-built/PACKET.md`. v5 is
  kept at `build/reviews/settings-honesty/packet-0-spec/spec-settings-honesty-v5.md` (sha256
  `14c7fa301c75fe877cd97d5d053758df8af3438e16642694486aacfe657b45be`). Status `delivered`.
  Sources: the commit `fcbcb080`; the test hand-backs in `packet-1-red-tests/` (with
  `BLOCKED-T1.md` and the PM decision in `HANDBACK-T1.md`), `packet-3-tests-fix-r1/` and
  `packet-4-tests-fix-r2/`; GPT-6 Astra's test reviews in `packet-2-tests-review/` (r1 FAIL, r2
  FAIL, r3 PASS); the code hand-backs in `packet-5-code/` (with `BLOCKED-C3.md`),
  `packet-6-knockon-tests/` and `packet-8-code-fix-r1/`; Astra's code reviews
  `packet-7-code-review/REVIEW-astra-code-r1.md` (FAIL: C1, C2, C3) and `REVIEW-astra-code-r2.md`
  (PASS); `deploy-20261005T235801Z/DEPLOYMENT.json` and `LIVE-CHECK.md`; the PO's live check
  ("ok looks good", 2026-10-06). No requirement or decision changes.
  - Each requirement is marked Delivered, and every criterion is checked. REQ-801 stays dropped.
  - Indexer add door corrected (PM decision during the tests stage on `BLOCKED-T1.md`): the dialog
    opens only to edit, the inline form is the only add door, and no opener was added. ST-009,
    REQ-401's shapes, AC-411, AC-415 and the matrix row now say so.
  - AC-528, AC-723 and AC-724 are new: the behaviours code review r1 found (C1, C2, C3), each with
    its test ([added in code review](#added-in-code-review)).
  - New [§8](#8-as-built): files changed, the new route and shared parts, what stays unused, and
    the limits of the evidence. The files table records where the tests landed.
## 0a. Design Principles

- Show only what works. A control that does nothing is hidden, not greyed (except the three
  greyed items the PO kept).
- Hiding a page from a user means every door to it: menu, links inside pages, notifications and
  a typed address. Hiding never replaces server authorization; no server permission changes except
  the one new language route.
- One definition per fact: one sentence for where files go, one rule for "unsaved changes", one
  language-list read for every page that shows the list, one password rule on the server.
- Tests drive the real component with the real router, auth store and API wrapper, with only
  `fetch` stubbed; server tests drive the real `build_router` and auth middleware over the real
  `SqliteDb`. Pin the named observable only.

## 0b. System Truths

Read at `6f74c6ff`. "(packet)" marks a premise taken from the packet or its grounding file
(`build/state/chunk-4-decision-sitting.md` § Grounding), re-opened here. P1 to P10 are in `probes.log`
(P5 to P10 added in v3). "How verified" says which parts a probe drove; "read" means source only, and
"unprobed" carries its reason. A probe is cited only for the inputs it drove.

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-001 | (packet) Greyed items render as grey text with a "Coming Soon" tooltip and no link. Greyed: Bookshelf, Calendar, Cutoff Unmet, General, Profiles, Custom Formats, Notifications, Tags, Development. General, Profiles, Custom Formats, Notifications, Development are also `adminOnly`. Calendar and Cutoff Unmet are the only children of "More". | `frontend/src/components/Sidebar/Sidebar.tsx:306-318`, `:78-83`, `:112-128`, `:154-201` | — | read |
| ST-002 | A group with no visible children renders nothing, in every mode (expanded, collapsed, the phone drawer, which uses the same group component). `adminOnly` is the only per-role filter. The collapsed setting is read inside the group, so the phone drawer follows it too. | `Sidebar.tsx:238-242`, `:244-257`, `:592-616`, `:231` | — | read (render logic); the empty-group case is unprobed: no group is empty today, and making one needs the menu change |
| ST-003 | Each greyed item has a placeholder route rendering `ComingSoonPage`; Profiles, Custom Formats, Development, Calendar, Cutoff Unmet and Bookshelf have no `AdminGuard`. The catch-all renders `ComingSoonPage` titled "Page Not Found", whose text is "This feature is coming in a future release." | `frontend/src/App.tsx:329-375`, `:414-433`, `:436-439`; `frontend/src/components/Page/ComingSoonPage.tsx:5-13` | Telling a user a removed page is "coming". | probed P1: `/no-such-page` (admin and normal user), `/calendar` and `/settings/profiles` (normal user) show the texts stated; other placeholder addresses read only |
| ST-004 | The Theme section offers Dark (a button with no action) and Light (disabled, "Coming Soon"). The store's `theme` stays "dark"; nothing on the page sets it. | `frontend/src/pages/settings/ui/UISettingsPage.tsx:28-46`; `frontend/src/stores/ui.ts:65`, `:86` | — | read |
| ST-005 | (packet) The Naming box shows the stored naming row read from `GET /config/naming` (admin only); nothing else reads that row or calls that route. Its "Rename Files: No" is false for ebooks. The File Management checkboxes are disabled and bound to nothing. | `frontend/src/pages/settings/media-management/MediaManagementPage.tsx:172-175`, `:586-653`; `frontend/src/api/index.ts:492-493`; `crates/livrarr-handlers/src/config.rs:76-87` | — | read; `rg getNamingConfig` over `frontend/src` finds only these |
| ST-006 | Every way a file enters the library, and where it goes (`{user}` is the user's number; Author and Title are made file-safe, characters files cannot use becoming `_`). **Download import** (`import_grab`) and **manual import** use the root folder for the file's type and call `build_target_path`: ebook `{root}/{user}/{Author}/{Title}.{ext}`; audiobook `{root}/{user}/{Author}/{Title}/{original relative name}` (manual import passes the file itself, so its own file name). **Readarr import** uses the one Livrarr root folder the user selects on its page, whatever that folder's type, and names every file `{root}/{user}/{Author}/{Title}.{ext}`, audiobooks included. **Import recovery** completes an import at the path its interrupted import recorded. **Root-folder scan** records files where they already are. **Merging two works** moves the kept work's files with `build_target_path` (`reorganize_work_files`); editing a work's title or author does not move files: the edit handler returns without calling it, and the merge handler is its only route caller. The `PathBuilder` trait (packet) has no implementation. | `crates/livrarr-library/src/import_workflow.rs:1762-1797`, `:2378`, `:2589-2610`, `:630`, `:707`; `crates/livrarr-handlers/src/manual_import.rs:1097-1105`; `frontend/src/pages/import/ReadarrImportPage.tsx:493-511`; `crates/livrarr-server/src/readarr_import_workflow.rs:2298-2302`, `:2962-2968`, `:1420-1437`; `crates/livrarr-server/src/import_service.rs:496`, `:603-611`; `crates/livrarr-handlers/src/work.rs:1038`, `:1435`, `:1525`; `crates/livrarr-handlers/src/root_folder.rs:188-193`; `crates/livrarr-domain/src/util.rs:10-40`; `crates/livrarr-library/src/lib.rs:50-71` | A sentence that is false for any listed door, or that promises files move on a title or author edit. | read; unprobed: each door needs real files on disk and a seeded import, outside a spec seat's scratch probes; the requirement changes wording only, and the criterion pins the wording |
| ST-007 | The Root Folders help tip on the same page says "Livrarr organizes files into Author/Title subfolders within each root", which omits the user folder and is wrong for ebooks. | `MediaManagementPage.tsx:353` | — | read |
| ST-008 | (packet) Book search asks only indexers that are enabled with Interactive Search on; with none, the server answers 200 with no results and the warning "All indexers failed". Automatic Search is stored and returned, and nothing else reads it. | `crates/livrarr-db/src/sqlite_indexer.rs:85-94`; `crates/livrarr-download/src/release_service.rs:72-81`; `crates/livrarr-handlers/src/release.rs:53-62`; `rg enable_automatic_search` over `crates` (types, DB row, handler copy only) | — | DB filter probed P6 (enabled+on listed; enabled+off and disabled+on not). The empty-list reply and "nothing reads Automatic Search" are read only: the search route needs the full app state, and absence of readers is a source search, not a runtime fact |
| ST-009 | Both indexer forms disable both search boxes: the edit dialog and the inline add form shown on the page. The dialog opens only to edit a stored indexer: its only openers are the edit buttons on stored rows, which pass that indexer (`IndexersPage.tsx:396`, `:480`), and nothing opens it empty, so its create branch is unreachable. The inline form is the only add door; it builds the request with `toCreateRequest`, whose form default for Interactive Search is on. Create sends both flags; the server defaults each to true when absent. Update sends both; the server keeps the stored value when one is absent. The list shows "Interactive" and "Auto" badges in the torrent and Usenet tables. | `frontend/src/pages/settings/indexers/IndexersPage.tsx:742-773`, `:1016-1027`, `:64-94`, `:96-106`, `:515`, `:549`, `:356-365`, `:450-455`; `crates/livrarr-handlers/src/types/indexer.rs:38-41`, `:73-74`; `crates/livrarr-handlers/src/indexer.rs:308-309`; `sqlite_indexer.rs:147-152` | Relying on the form library to carry a hidden field. | server defaults probed P5 (absent flags: create true/true, update None/None) and P7 (update with flags absent keeps stored automatic false, interactive true); the handler passing the absent value through is read; the web forms are read; the dialog's openers were read by the tests seat and confirmed by the PM (`packet-1-red-tests/BLOCKED-T1.md`, `HANDBACK-T1.md`), lines at `6f74c6ff` |
| ST-010 | The three test routes are admin-only POSTs that read the saved settings, answer 200 with no body on success, and otherwise answer with a message: Hardcover 400 "Hardcover API token not configured", 502 "Hardcover returned {status} — check API token" or "Hardcover connection failed: …"; Audnexus 502 "Audnexus returned {status}" or "Audnexus connection failed: …"; AI 400 "LLM endpoint not configured", "LLM API key not configured" or "LLM model not configured", 502 "LLM returned {status} (see server logs for details)" or "LLM connection failed: …". None checks the Enabled box. The AI test uses the trusted client and blanks the key it sent from the provider's reply before logging it. 400 and 502 messages reach the page as sent. | `crates/livrarr-server/src/router.rs:231-243`; `crates/livrarr-handlers/src/config.rs:292-422`, `:415`, `:431-464`; `crates/livrarr-handlers/src/types/api_error.rs:497-498`; `frontend/src/api/client.ts:52-76` | Weakening the key blanking or switching the AI test's client. | unprobed: the routes need the full app state (the real builder is private to `router.rs`'s test module, which a spec seat may not edit) and their success and 502 replies need the external services; read only. The page-side message pass-through is probed for a 403 envelope (P10), the same code path |
| ST-011 | Audnexus always has a saved address: the column defaults to `https://api.audnex.us`; the page sends `null` for a cleared field, the request type reads `null` as absent, and the store keeps the current address. | `crates/livrarr-db/migrations/001_initial_schema.sql:156`; `MetadataPage.tsx:198-199`; `crates/livrarr-handlers/src/types/config.rs:198`; `crates/livrarr-db/src/sqlite_config.rs:216` | — | each step probed on its own input: P12a (real page: clearing the field and saving sends `"audnexusUrl":null`), P11 (that body's `null` becomes `None` in the request type), P8 (the store keeps the address when it is absent, and its default). The handler passing the value from request to store is read only: it is a field copy (`config.rs:255`), and driving it needs the full app state (see ST-010) |
| ST-012 | The web app has no wrapper for the test routes. | `frontend/src/api/index.ts:551-560` (metadata wrappers); `rg 'metadata/test'` over `frontend/src`: no hit | — | read |
| ST-013 | Metadata page "unsaved" state lives in three places: the form (react-hook-form 7.72 with a `values` prop built from the saved settings, secret fields always blank), a languages flag and a default-language flag. Both flags are set true by any edit of their control, never by comparison: the languages flag is never cleared; the default-language flag clears on its own successful save. Form writers: typed inputs and the Enabled checkboxes (`register`); the model preset select (`Controller` change); and three programmatic writers with `setValue` and no options: the AI Provider select's handler (sets provider, and endpoint and model from the preset), and "Back to preset models" (sets model). "Custom model" only switches the input. Language toggles show only when Google Books or the AI connection is configured. Save starts the metadata write and, if the default-language flag is set, a separate default-language write; each has its own success and error handling and its own route. While either write is pending the Save button is disabled and reads "Saving..."; every other control stays editable. | `MetadataPage.tsx:116-135`, `:150-157`, `:93-114`, `:232-244`, `:506-517`, `:555-562`, `:620-629`, `:188-215`, `:217-229`, `:348-354`, `:415-456`, `:650-662`; `frontend/package.json:35`; `crates/livrarr-server/src/router.rs:222-229` | Using the form's dirty flag alone, or either language flag, as "unsaved changes". | Form: P2 (real page: a typed token stays after a successful save and the form stays dirty; typing back clears dirty) and P9 (library, stand-in form: `setValue` without options leaves the form clean, with `shouldDirty` it marks dirty and clears at the saved value; a `Controller` change marks dirty); the page's own provider and preset controls are not driven by a probe, because P9 used a stand-in form. Flags and writes: P12 (real page: enabling a language and saving, then saving again, resends the list each time, so the flag is never cleared; a changed default language is sent once and not on the next save; with the metadata write answered 500, the default-language write still went out and was not resent). Read only, as structural render facts: the toggle's visibility condition, the two routes, and Save being the only control disabled while a write is pending (P12 never held a reply; AC-524 drives it) |
| ST-014 | (packet) Unmapped Files has a route and no link. Its only data call lists root folders (admin only). Its "Go to Settings" link goes to `/settings/media-management`; the page lives at `/settings/mediamanagement`. Manual Import has the same wrong link. P1: a normal user at `/unmapped` gets the page, which calls the admin-only list. | `App.tsx:243-250`; `frontend/src/pages/unmapped/UnmappedPage.tsx:30-33`, `:162-163`; `frontend/src/pages/manual-import/ManualImportPage.tsx:836-837`; `App.tsx:293-294` | — | read; P1 |
| ST-015 | `AdminGuard` sends a non-admin to `/` with a replace. The settings address with no sub-page renders Media Management for everyone. | `frontend/src/components/Page/AuthGuard.tsx:32-40`; `App.tsx:285-300` | — | probed P1: a normal user at `/settings/metadata` ends at `/`; at `/settings`, `/system/logs`, `/system/status` and `/unmapped` the page mounts and sends its admin-only calls (403 in the probe); `/import` renders the Manual Import page. What those pages show after the refusal in a browser is unprobed: jsdom never ran the app's retry (P1 note) |
| ST-016 | Doors to the five pages for a normal user: the menu (`Sidebar.tsx:100-109`, `:132-136`, `:207-212`); typed addresses; the path-not-found notification's "Configure path mapping" link, in its pop-up and in the bell list, which goes to Media Management and is sent to the grab's owner, who may be a normal user; the guided tour, which auto-starts for every user once per browser and steps through Metadata, Indexers, Download Clients and Media Management. Readarr Import's and Manual Import's own links and the sidebar health widget's Status link sit on admin-only pages or render for admins only. | `frontend/src/components/Header/NotificationBell.tsx:97-101`, `:273-277`; `crates/livrarr-server/src/jobs/download_poller.rs:243-247`; `frontend/src/components/Page/AppLayout.tsx:16-26`; `frontend/src/components/GuidedTour/tourSteps.ts:19-146`; `frontend/src/pages/import/ReadarrImportPage.tsx:516`; `Sidebar.tsx:373-383` | — | read |
| ST-017 | Web callers of admin-only routes that a normal user can reach (59 admin-only handlers, enumerated by signature): Header search and Add New read the language list from `GET /config/metadata`; the release list on a book page reads `GET /config/mediamanagement` for preferred formats; the Help page reads system status and the log tail; the author page's refresh calls the admin-only author search; Download Clients reads the Prowlarr config. Only the first is in this feature's scope. | `frontend/src/components/Header/Header.tsx:26-34`; `frontend/src/pages/search/SearchPage.tsx:51-78`; `frontend/src/pages/work-detail/components/ReleasesTab.tsx:60-68`; `frontend/src/pages/help/HelpPage.tsx:122-129`; `frontend/src/pages/author-detail/AuthorDetailPage.tsx:80-87`; `crates/livrarr-handlers/src/work.rs:1880-1884`; `frontend/src/pages/settings/download-clients/DownloadClientsPage.tsx:918-922` | — | read; handler list by a signature scan of `crates/livrarr-handlers/src/*.rs` for `RequireAdmin` |
| ST-018 | `RequireAdmin` refuses any non-admin with 403 and the message `forbidden`; the web client keeps that message ("You don't have permission" is only its fallback when a reply has no usable message). `GET /config/metadata` is admin-only. Its reply carries the AI endpoint, provider and model, the Audnexus address, "is set" flags for each key, and provider errors, never a key. A normal user's header and Add New each call it, get 403, and show no language picker (only English is searched); an admin sees the picker. | `crates/livrarr-handlers/src/middleware.rs:4-25`; `crates/livrarr-handlers/src/types/api_error.rs:496`; `frontend/src/api/client.ts:52-76`, `:84`; `config.rs:54-74`, `:124-131`; `SearchPage.tsx:163` | Sending any field of that reply to a normal user. | probed P3 (extractor: normal user Forbidden, admin accepted), P10 (client keeps `forbidden`), P1 (picker absent for a normal user, present for an admin, with the 403 stubbed); the route-to-extractor binding and the reply's fields are read |
| ST-019 | Readarr Import itself is open to every signed-in user on the server; only its origin allow-list and the root-folder list are admin-only. Hiding the page does not change that. | `crates/livrarr-handlers/src/readarr_import.rs:10-47`, `:86-112`; `wiki/architecture/ui-architecture.md:26-27` | Claiming the hide is a security control. | read |
| ST-020 | The setup wizard cannot reach its steps after the account step. Account creation sets the session to signed in at once; `/setup` sits behind `GuestGuard`, which sends a signed-in user to `/`; the wizard advances its own step only after that. So root folders, indexer, download client, metadata and summary never render in the shipped flow. The client step itself is titled "Download Client (qBittorrent)" and always creates qBittorrent. | `frontend/src/stores/auth.ts:107-119`; `frontend/src/App.tsx:107-113`; `frontend/src/components/Page/AuthGuard.tsx:42-53`; `frontend/src/pages/setup/SetupPage.tsx:165-172`, `:545`, `:60-71` | A criterion that claims the wizard reaches the client step. | probed by the PM (`pm-probe-setup.log` in the packet folder: real App, account step succeeds, address becomes `/`, no later step renders) |
| ST-021 | The server checks a password only when one is set: setup, the user's own profile, an admin creating a user, an admin editing a user. It counts bytes, minimum 6, maximum 1,024, and answers 422 "invalid password: …". On the profile and admin-edit doors the check runs before sessions are ended and before anything is written, so a refused change ends no session. Sign-in never checks length. There is no command-line or reset path that sets a password; the only other writer is the empty placeholder admin of a fresh install. The four web forms already require 8. | `crates/livrarr-server/src/auth_service.rs:122-133`, `:269-271`, `:362-379`, `:414-416`, `:474-500`, `:193-216`; `crates/livrarr-handlers/src/types/auth.rs:252-253`; `crates/livrarr-handlers/src/types/api_error.rs:628-630`; `crates/livrarr-server/src/main.rs:48-59` (subcommands: identity cutover only); `crates/livrarr-db/migrations/001_initial_schema.sql:28-32`; `SetupPage.tsx:300-303`; `frontend/src/pages/profile/ProfilePage.tsx:84-86`; `frontend/src/pages/settings/users/UsersPage.tsx:366-369`, `:502-505` | Checking length at sign-in. | probed P4 on admin create only (5 refused; 6, 7 and 1,024 ASCII accepted; 1,025 refused; "ééé", 3 characters and 6 bytes, accepted); the other three doors share the same check by reading; the refusal-before-session-end ordering is read only (the criteria drive it) |
| ST-022 | No Rust function or trait signature changes: REQ-701 adds a handler, a response type and a route; REQ-901 changes a private function's body. So no compiler enumeration of callers was needed. | this spec | — | by construction |
| ST-023 | Test doors. Web: `installApiStub` replaces only `fetch`; `mountWith` mounts one page in a memory router; `App.toaster.test.tsx` mounts the whole real `App` at an address with a session token; `Sidebar.test.tsx` mounts the real sidebar. The app's own query client retries once, which did not run in jsdom (P1), so route tests should assert on the address or on calls, or answer every call. Server: `router.rs`'s test module builds the real `AppState` and `build_router`; `tests/behavioral/test_author_link_doors.rs:42` builds a route harness with an API key and seeds users through the production user writer. Setup is rate-limited to one request per address, and the router tests vary the peer address. | `frontend/src/test-support/apiStub.tsx:55-133`; `frontend/src/App.toaster.test.tsx:17-90`; `frontend/src/components/Sidebar/Sidebar.test.tsx:46-60`; `crates/livrarr-server/src/router.rs:737-800`, `:1215-1247`; `tests/behavioral/test_author_link_doors.rs:42`, `:763-776` | A toy router or hand-built state where the real door exists. | read; the jsdom retry stall observed in P1 |

## 0c. Prior Art

Searched: the work plan chunk 4, the project to-do list in `build/plans/`, `build/reports/`,
GitHub #79, #120 and #122, `wiki/` and `docs/` (`git grep -il` for "coming soon", "admin",
"naming", "interactive search"), `spec-prerelease-trust-pass.md`, and insights 39 and 75.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `build/plans/todo-chunks-2026-10-04.md:72-86` (chunk 4) and `build/state/chunk-4-decision-sitting.md` (PO answers). | Source of the nine items and the decisions of record. |
| PA-002 | `build/reports/ux-gap-scan-2026-09-26.md:51-62`, `:125-145`: A1 (nine items), A2 (unknown page says "coming"), A4, A5, A9, B1 (pages shown, server refuses), B2 (language list, naming `Header.tsx:26-34` as a door). The to-do list in `build/plans/`, lines 117-120 and 235-268. | B2 confirms the header is a second door to the language list (REQ-701). A2 is Q-1. The report read code only; P1 now confirms B1 and B2 in the real app. |
| PA-003 | GitHub #79 (open): "drop Calendar and Cutoff Unmet pages?". | REQ-101 removes both; the issue can be closed with this feature. |
| PA-004 | GitHub #120 (open): configurable naming. | Not built (not approved). REQ-301 only describes today's fixed layout. |
| PA-005 | GitHub #122 (open): bulk edit of works. | No bearing found; listed because the packet named it. |
| PA-006 | `wiki/architecture/ui-architecture.md:9-10`, `:26-27`: "Coming Soon" labels are historical; hiding an admin page does not replace backend authorization. | Principle for REQ-701 and ST-019. |
| PA-007 | `wiki/architecture/grab-system.md:7`, `wiki/domain/release.md:8`: interactive search queries enabled indexers. | Agrees with ST-008. |
| PA-008 | `spec-prerelease-trust-pass.md:155` (ST-106: wizard configures qBittorrent only), `:184-185` (server messages reach the setup form), `:381-385` (REQ-305 setup token). | Item 8 is dropped (ST-020); REQ-901's setup criterion must send the setup token. |
| PA-009 | `wiki/insights/coding-patterns.md:133-138` (lesson 39, settings contracts) and `wiki/insights/process.md:110-113` (lesson 75, run the frontend typecheck). | REQ-701 reads the language list through the existing app-config contract; every web change runs the typecheck. |
| PA-010 | `docs/ROADMAP.md:10`, `:124`: configurable naming is roadmap work (#120). | Same as PA-004. |

## 1. Problem Statement

1. **Any user opens the menu.** Six greyed items (Bookshelf, Calendar, Cutoff Unmet, Profiles,
   Custom Formats, Development) promise features that are not planned; "More" holds only two of
   them (ST-001).
2. **Any user opens Settings → UI.** A Theme choice with one working option and a disabled Light
   button (ST-004).
3. **An admin opens Settings → Media Management.** A greyed Naming box says "Rename Files: No"
   though every ebook is renamed to its title; a greyed File Management box offers three
   checkboxes that do nothing (ST-005, ST-006).
4. **An admin adds or edits an indexer.** Interactive Search, which the book search honours, is
   locked; Automatic Search, which nothing reads, is shown as if it mattered (ST-008, ST-009).
5. **An admin enters a Hardcover token, an Audnexus address or AI settings.** There is no way to
   check them from the page, though the server has test routes (ST-010, ST-012).
6. **An admin wants to scan for unmapped files.** The page exists but nothing links to it; its
   "Go to Settings" button leads to a wrong address (ST-014).
7. **A normal (non-admin) user, after an admin enabled French and German.** The menu offers Manual
   Import, Readarr Import, Media Management, Status and Logs, and Settings opens on Media
   Management; each page's data calls are refused (ST-015). On Add New and in the header search the
   language picker is missing, so they can search only in English and are not told why (ST-018, P1).
8. **The owner runs the setup wizard.** The download-client step offers only qBittorrent and does
   not say the other two exist. Dropped by the PO: the wizard never reaches that step after the
   account step (ST-020, §4).
9. **A user or admin sets a 6- or 7-character password through the server directly.** The web
   forms ask for 8, but the server accepts 6 (ST-021, P4).

## 2. Requirements

Each requirement names its doors, the input shapes and the test door. "Normal user" means a
signed-in user whose role is not admin.

- **REQ-101** **Delivered**. (item 1, menu). Remove Bookshelf, Calendar, Cutoff Unmet, Profiles, Custom Formats
  and Development from the menu definition and remove the "More" group, so no "More" heading shows
  in any mode (ST-002). Remove their six routes (`/shelf`, `/calendar`, `/wanted/cutoff`,
  `/settings/profiles`, `/settings/customformats`, `/settings/development`). General, Notifications
  and Tags keep their greyed items and routes unchanged. A removed address, typed or bookmarked,
  now reaches the catch-all, which shows "Page Not Found" (its body text is REQ-102).
  `ComingSoonPage` stays for the three greyed routes.
  - Shapes: admin and normal user; menu expanded, collapsed, and the phone drawer; each removed
    address typed by hand by each role; `/wanted/missing` (a neighbour of a removed address) still
    opens Missing.
  - Door: menu criteria, the real `Sidebar` with the auth and UI stores set, as `Sidebar.test.tsx`;
    address criteria, the real `App` at the address with a session token, as `App.toaster.test.tsx`.
- **REQ-102** **Delivered**. (item 1, PO answer to v1's Q-1). The catch-all route, shown for any address the app
  does not have (ST-003), shows the title "Page Not Found" and the line "This page does not
  exist." instead of "This feature is coming in a future release." The three kept greyed pages
  (General, Notifications, Tags) keep their own titles and the coming-soon line unchanged.
  - Shapes: a removed address (for example `/calendar`); an address that never existed (for
    example `/no-such-page`); each kept greyed page; admin and normal user. `/settings/general` for
    a normal user still lands on `/` through its `AdminGuard` (unchanged).
  - Door: the real `App` at the address with a session token, as REQ-101.
- **REQ-201** **Delivered**. (item 2, theme). Remove the Theme section from Settings → UI. The stored theme value
  and the rest of the page are unchanged. No reason to keep the section was found: with Light
  gone it offers one fixed choice (ST-004).
  - Shapes: admin and normal user (both reach the page). Unsaved changes: not applicable, the
    page saves each control at once.
  - Door: the real `UISettingsPage` through `mountWith`.
- **REQ-301** **Delivered**. (item 3, naming). On Media Management, remove the File Management box and replace
  the Naming box with a box titled "File locations" holding one sentence (D-301). The page stops
  reading `GET /config/naming` (ST-005); the server route stays, unused.
  - The sentence is true for every door in ST-006 at the moment of import: download import and
    manual import (root folder for the file's type), Readarr import (named as the difference: the
    root folder chosen on its page, and `Title.ext` for every file), import recovery (completes a
    path built by those doors). It makes no promise about later edits: a title or author edit
    moves no file, and a merge re-files with the import rule. A root-folder scan moves nothing,
    so it places no file and the sentence does not cover it. Wording only: no import, edit or
    merge behaviour changes.
  - **Root Folders help tip** (PO answer to v1's Q-3; ST-007): its text becomes the wording in
    D-302, so the page gives one account of where files go.
  - Shapes: admin only (normal users no longer reach the page, REQ-701). The page with the naming
    route failing: not applicable after the change, since the page no longer calls it.
  - Door: the real `MediaManagementPage` through `mountWith` with its remaining calls stubbed.
- **REQ-401** **Delivered**. (item 4, indexers). In both indexer forms (the edit dialog and the
  inline add form, ST-009) the Interactive Search box becomes an ordinary checkbox, like RSS Sync, and its
  value is sent on create and on update. The Automatic Search box is removed from both forms, and
  the web stops sending `enableAutomaticSearch`: the server then defaults it on create and keeps
  the stored value on update (ST-009), so no indexer's stored flag changes. The "Auto" badge is
  removed from both lists (it described a setting nothing reads); the "Interactive" badge stays and
  now follows the box.
  - Shapes: create through the one add door (the inline form; the dialog only edits, ST-009),
    torrent and Usenet, with the box left checked (its default) and unchecked; edit an indexer from on to off and off
    to on; edit another field of an indexer stored on, and of one stored off, without touching the
    box. Normal user: not applicable (the page is admin-only, `App.tsx:301-310`).
  - Door: the real `IndexersPage` through `mountWith`, requests read from the stub's call log.
- **REQ-501** **Delivered**. (item 5, test buttons). Add API wrappers for the three test routes and a **Test**
  button in the Hardcover, Audnexus and LLM Enrichment sections of Settings → Metadata.
  - **One rule for unsaved changes** (D-501): the page has unsaved changes when any form value
    differs from the saved settings it loaded, whatever control wrote it; or the enabled-language
    list differs from the saved list, compared entry by entry in order (its first entry is the
    primary language); or the selected default language differs from the saved one. These are
    comparisons with saved values, not records that an edit happened: enabling a language and
    disabling it again, or changing the default language and changing it back, leaves no unsaved
    change (today both leave a flag set, ST-013, P12). The writers (ST-013) are typed inputs, the
    Enabled checkboxes, the model preset select, the AI Provider select (which also writes the
    endpoint and model from its preset) and "Back to preset models"; "Custom model" alone writes
    nothing. Today the last two do not count (P9); this requirement makes them count. Returning
    every changed value to the saved one, by any control, leaves no unsaved change (P2, P9).
  - **A save has two parts** (D-503). Save sends the service settings (every form value and the
    languages) and, when the default language changed, the default language, as two requests as
    today. Each part is saved only when its own request succeeds: then that part shows the saved
    values (secret fields blank, for the service part) and stops counting as unsaved. A part whose
    request is pending or failed still counts as unsaved and keeps the value the user entered.
    Test is blocked while any changed part is unsaved. Today a saved token stays in its field and
    the page stays dirty (ST-013, P2); this requirement fixes that.
  - **No edits while saving** (D-504). From pressing Save until both writes have settled (each
    succeeded or failed), every settings writer on the page is disabled: every form field and
    both Enabled checkboxes, the AI Provider select, the model select or input, "Custom model" and
    "Back to preset models", every language toggle and the default-language select, as the Save
    button already is (ST-013). When both have settled they are enabled again, whatever the
    outcome. So a reply always acknowledges exactly what the user sees.
  - **With no unsaved changes**, pressing Test sends the one POST, shows "Testing..." on that
    button and disables it until the reply. 200: a success pop-up "Hardcover connection
    successful", "Audnexus connection successful" or "AI connection successful". Any other reply:
    one error pop-up with the server's message (ST-010), for example "Hardcover API token not
    configured", "LLM model not configured", "Audnexus returned 503", or "Unable to reach Livrarr"
    for a network failure.
  - **With unsaved changes**, pressing Test sends nothing and shows one pop-up: "Save your changes
    first. Test checks the saved settings."
  - The tests ignore the Enabled boxes (ST-010): a disabled but configured service is still
    tested. The AI test keeps its trusted client and key blanking unchanged (ST-010).
  - Shapes: each service with and without unsaved changes; saved token present and absent
    ("not configured"); Audnexus "not configured" is not applicable, it always has an address
    (ST-011); each writer above, alone, and then returned to the saved value; after a successful
    save with a token typed; a save with both parts changed where the service part succeeds and
    the default-language part fails, the reverse, and one part held pending; a failed part retried
    successfully; an edit attempted while a save is held; a language enabled then disabled, a
    language list with the same members in a new order, and a default language changed then
    restored, each with no Save between; test replies success, 400, 502 and network failure; a second press while one is
    pending (the button is disabled). Normal user: not applicable (admin-only page).
  - Door: the real `MetadataPage` through `mountWith` with `AppToaster`; requests and pop-ups read
    as the shipped page tests do.
- **REQ-601** **Delivered**. (item 6, Unmapped Files). Add "Unmapped Files" (`/unmapped`) to the Activity group
  after Readarr Import, admin-only. Wrap the `/unmapped` route in `AdminGuard`, as for the other
  admin-only pages (D-601). Its "Go to Settings" link goes to `/settings/mediamanagement`.
  Manual Import's "Configure root folder" link, shown beside a scanned file that no root folder
  can take (`ManualImportPage.tsx:835-841`), has the same wrong address (ST-014) and goes to
  `/settings/mediamanagement` too (PO answer to v1's Q-2). It keeps opening in a new tab.
  - Shapes: admin and normal user; menu expanded, collapsed and phone drawer; `/unmapped` typed by
    each role; the page with no root folders (the link shows) and with some. Manual Import's
    link: admin only, since the page becomes admin-only (REQ-701).
  - Door: `Sidebar` and `App` as REQ-101; the Unmapped link through the real `UnmappedPage` with
    the root folder list answered `[]` and the "Root folder" mode chosen; the Manual Import link
    through the real `ManualImportPage` with a scan answered with one file that no root folder
    takes, as `ManualImportPage.defer.test.tsx` drives that page.
- **REQ-701** **Delivered**. (item 7, normal users). For a normal user:
  - **Menu.** Manual Import, Readarr Import, Media Management, Status and Logs become admin-only.
    A normal user then sees: Library (Works, Series, Authors, Lists, Add New, Missing, Needs
    Review), Activity (Queue, History), Settings (Download Clients, UI, Tags greyed), System
    (About Livrarr). Download Clients is unchanged.
  - **Typed addresses.** `/import`, `/import/readarr`, `/settings/mediamanagement`,
    `/system/status` and `/system/logs` are wrapped in `AdminGuard`: a normal user lands on `/`,
    as at `/settings/metadata` today (ST-015).
  - **Settings with no sub-page.** `/settings` opens Media Management for an admin (unchanged) and
    replaces the address with `/settings/ui` for a normal user.
  - **Links inside pages.** The path-not-found notification's "Configure path mapping" link is not
    shown to a normal user, in the pop-up or the bell list; the rest of the message (including
    "Get AI help") is unchanged. All other links to the five pages are on admin-only pages or
    shown to admins only (ST-016). The guided tour is unchanged: for a normal user its Media
    Management steps now bounce to Works, as its Metadata and Indexers steps already do (§4).
  - **Language list** (D-701). New route `GET /api/v1/config/languages`, readable by every signed-in
    user (session or API key), answering `{"languages": [...]}`: the saved metadata settings'
    enabled language codes, nothing else. Add New and the header search read the list through one
    shared web query on this route, for admins and normal users alike; the Metadata settings page
    keeps `GET /config/metadata`. The pages treat the reply's `languages` exactly as they treat
    today's (first code preselected; picker shown when more than one).
  - **Never rule** (D-702). A normal user never receives any other metadata setting. Mechanism: the
    route's reply is a dedicated response type whose only field is `languages`, built from the
    saved language list alone; no existing response type is reused or extended.
    `GET /config/metadata` and every other admin-only route stay admin-only (ST-018).
  - Shapes: admin and normal user; each of the five pages by menu (expanded, collapsed, phone
    drawer) and by typed address; `/settings` and `/settings/` by each role; the path-not-found
    pop-up and bell entry by each role; languages `["en"]`, `["en","fr","de"]`, `["fr","en"]`
    (first code preselected); the language route failing (500 or a rejected fetch) on a fresh
    load with no language in the address: the pages behave as today when `/config/metadata`
    fails, English only, no picker, no message, and no fallback read of `/config/metadata`
    (unchanged, §4); a signed-out
    request: 401 as every protected route.
  - Door: menu and addresses as REQ-101; the notification through the real `NotificationBell` with
    `AppToaster`, as `NotificationBell.test.tsx`; Add New through the real `SearchPage` and the
    header through the real `Header`, with `fetch` stubbed; the route through the real
    `build_router` and auth middleware over `SqliteDb` with an admin and a normal user (ST-023).
- **REQ-901** **Delivered**. (item 9, password). The server requires at least 8 characters whenever a password is
  set, on all four doors of ST-021, through the one existing check. The minimum counts Unicode
  characters (Rust `chars()`, one per code point), not bytes (D-901). The maximum stays 1,024
  bytes, unchanged. A refusal is 422 with "invalid password: minimum 8 characters" (or the
  existing maximum message), and nothing is written: setup and admin create make no account;
  profile and admin edit leave the old password and every existing session working. Sign-in is
  unchanged, so an existing 6- or 7-character password still works. The web forms already require
  8 and are unchanged.
  - Shapes, on each of setup, own profile, admin create, admin edit: ASCII 7 characters refused,
    8 accepted, 1,024 accepted, 1,025 refused (the last two unchanged); `é` (2 bytes each) 7
    refused, 8 accepted, 512 (1,024 bytes) accepted, 513 (1,026 bytes) refused; profile and admin
    edit with no password field still succeed; a refused profile or admin-edit change with a
    session issued before it. Sign-in with an existing 6-character password succeeds.
  - Door: the real `build_router` and middleware over `SqliteDb`: `POST /setup` with the setup
    token, `POST /user` and `PUT /user/{id}` as an admin, `PUT /auth/profile` as the user,
    `POST /auth/login`. The existing 6-character account is written by the production user writer
    with a `RealAuthCrypto` hash, justified: before this change the server created such accounts,
    and after it no door can (ST-021).

### Build files and sizes

| REQ | Production | Tests | Size |
|-----|------------|-------|------|
| REQ-101 | `frontend/src/components/Sidebar/Sidebar.tsx`, `frontend/src/App.tsx` | `Sidebar.test.tsx` (tracked); a new `frontend/src/App.routes.test.tsx` (not ignored) | S, 2 h |
| REQ-102 | `App.tsx`; the catch-all's text (`ComingSoonPage.tsx` or a small not-found page beside it) | `App.routes.test.tsx` | S, 0.5 h |
| REQ-201 | `frontend/src/pages/settings/ui/UISettingsPage.tsx` | new `UISettingsPage.test.tsx` | S, 0.5 h |
| REQ-301 | `MediaManagementPage.tsx` (boxes and help tip) | new `MediaManagementPage.test.tsx` | S, 1.5 h |
| REQ-401 | `frontend/src/pages/settings/indexers/IndexersPage.tsx` | new `IndexersPage.test.tsx` | S, 2 h |
| REQ-501 | `frontend/src/pages/settings/metadata/MetadataPage.tsx`, `frontend/src/api/index.ts` | new `MetadataPage.test.tsx` | M, 6 h |
| REQ-601 | `Sidebar.tsx`, `App.tsx`, `frontend/src/pages/unmapped/UnmappedPage.tsx`, `frontend/src/pages/manual-import/ManualImportPage.tsx` | `Sidebar.test.tsx`, `App.routes.test.tsx`, new `UnmappedPage.test.tsx`, new sibling `ManualImportPage.links.test.tsx` | S, 1.5 h |
| REQ-701 | `Sidebar.tsx`, `App.tsx`, `NotificationBell.tsx`, `SearchPage.tsx`, `Header.tsx`, `api/index.ts`, `frontend/src/types/api.ts`, one small shared language-query module; server `crates/livrarr-handlers/src/config.rs`, `crates/livrarr-handlers/src/types/config.rs`, `crates/livrarr-server/src/router.rs` | `Sidebar.test.tsx`, `App.routes.test.tsx`, `NotificationBell.test.tsx`, `SearchPage.test.tsx` (tracked), new `frontend/src/components/Header/Header.test.tsx`; server cases in `router.rs`'s test module (tracked); `SearchPage.selectedMetadata.test.tsx` and `MissingPage.test.tsx` updated for the new language read (§8) | M, 6 h |
| REQ-901 | `crates/livrarr-server/src/auth_service.rs` | `router.rs`'s test module | S, 2 h |

**Overlap.** `Sidebar.tsx` and `App.tsx` are edited by REQ-101, REQ-102, REQ-601 and REQ-701: one
coder, in that order. `api/index.ts` is edited by REQ-501 and REQ-701. `router.rs` gets REQ-701's route and both
server test groups. `MediaManagementPage.tsx` is edited only by REQ-301 (REQ-701 guards it in
`App.tsx`). `ManualImportPage.tsx` is edited only by REQ-601 (REQ-701 guards it in `App.tsx`). New files under `frontend/src/` are not ignored by git; new
files under `tests/` are, so server cases go in `router.rs`.

## 3. UI/Interface Design

No new page. New texts: REQ-102's "This page does not exist."; REQ-301's sentence and help tip; REQ-501's three buttons, three success texts and the
save-first text; REQ-601's menu label "Unmapped Files". API: one new route,
`GET /api/v1/config/languages` → `200 {"languages": ["en", …]}` for any signed-in user.

## 4. Non-Requirements

- No light theme, no greyed page built (General, Notifications, Tags or the six removed), no
  configurable naming (#120), no automatic search, no other clients in the wizard.
- Item 8, the setup wizard's download-client hint: dropped by the PO 2026-10-05: the step is never
  reached (ST-020); to-do list entry "setup wizard steps after the account are unreachable". The
  wizard is unchanged.
- No change to import, edit or merge file placement (REQ-301 changes wording only).
- No change to which server routes need an admin, except the new language route. Readarr Import
  stays open to normal users on the server (ST-019).
- Not in scope (PO 2026-10-05, to-do list entry "normal users, part 2"): the other normal-user
  callers of admin-only routes (ST-017: the release list's preferred formats, the Help page's
  status and log tail, the author refresh, Download Clients' Prowlarr config) and the guided
  tour, which auto-starts for normal users and visits admin pages (ST-016). They stay unchanged.
- Unchanged: the silent English fallback when the language list cannot be read; the Metadata page's own read of
  `/config/metadata`; the indexer search behaviour; the stored Automatic Search values; the
  `/config/naming` route; the web forms' password rules.

## 5. Open Questions

No open question. v1's Q-1 to Q-3 were decided by the PO on 2026-10-05, each as option (a):

- Q-1, the catch-all's text: REQ-102.
- Q-2, Manual Import's root-folder link: REQ-601.
- Q-3, the Root Folders help tip: REQ-301, D-302.

## 6. Decisions

| ID | Decision | Status | Resolution |
|----|----------|--------|------------|
| D-301 | REQ-301's sentence. | closed, spec | "When Livrarr imports a book it files each ebook as Author/Title.ext and keeps each audiobook's own file names inside an Author/Title folder, under a folder for your user number in the root folder for that type; Readarr Import instead puts every file, audiobooks included, in the root folder you choose there, as Author/Title.ext." One sentence, with the Readarr difference named, as the packet allows. It describes placement on import only and promises nothing about later edits (ST-006). Characters that files cannot use become "_"; the sentence leaves that out as detail. |
| D-302 | REQ-301's help tip text. | closed, spec | "Where your library files are stored. Add one folder for ebooks and one for audiobooks. Inside each, Livrarr files books by user number and author; File locations below gives the full layout." It points to D-301 instead of restating it, so the page has one account of the layout. Re-checked for v3: true for every import door, including Readarr's chosen root, and silent about edits. |
| D-501 | REQ-501's unsaved-changes rule and the reset after save. | closed, spec | Options: (a) the form's dirty flag alone; (b) dirty flag plus the two language flags, with the page returning to the saved settings after a successful save, and every form writer taking part in the dirty comparison, its dependent fields included; (c) compare every field to the saved settings by hand. **(b)**: (a) misses language edits, stays dirty forever after a save with a token typed (P2), and misses the provider and preset writers, which change values without marking the form dirty (P9); (c) duplicates what the library already does once every writer takes part. Returning a value to the saved one counts as no change (P2, P9). The two language states use the same rule: each is a comparison with its saved value (the list in order, since its first entry is the primary language), because today's flags stay set after a change is undone or saved (P12). |
| D-503 | REQ-501: a save that sends two requests. | closed, spec | Options: (a) one outcome for the whole save; (b) each part saved only on its own success. **(b)**: the two requests and routes are independent (ST-013), so either can succeed while the other fails or is still pending; a single outcome would either let Test run against an unsaved default language or throw away saved service settings. A failed part keeps the user's entry so a retry needs no retyping. |
| D-504 | REQ-501: edits while a save is in progress. | closed, PM (fold packet v4, Astra's small option) | Options: (a) disable every settings writer until both writes settle; (b) keep editing allowed and acknowledge only the submitted values. **(a)**: the user cannot create a version the reply does not describe, so a successful reply can safely show the saved values and a newer secret is never cleared by an older save. Cost: the page is read-only for the length of a save. |
| D-502 | REQ-501's messages. | closed, spec | Pop-ups, as the indexer and download-client tests use. Failure shows the server's message unchanged (ST-010). Save-first is a pop-up on press, so the button stays visible and explains itself. |
| D-601 | REQ-601: a normal user typing `/unmapped`. | closed, spec | `AdminGuard`, as every other page whose only data is admin-only. The PO made the link admin-only; leaving the address open would show a page that fails on load (P1). |
| D-701 | REQ-701's language read. | closed, spec | Options: (a) a new route that returns only the list; (b) open `GET /config/metadata` to normal users and strip fields; (c) a per-role reply from the same route. **(a)**: the packet allows exactly one language-list change, and a separate type makes the never rule structural (D-702). Admins use the same route on Add New and the header, so both pages have one definition. |
| D-702 | REQ-701's never rule. | closed, spec | Declared, not filtered: the reply type has one field. Pinned by AC-719 (key set and absence of every secret and endpoint value) and AC-720 (the admin route still refuses). |
| D-901 | REQ-901: what counts as a character. | closed, spec | Options: (a) Unicode characters; (b) bytes, as today. **(a)**: "8 characters" is what every form says; with bytes, three accented letters pass (P4). The web forms count UTF-16 units, which equal characters except for symbols such as emoji; a password of fewer than 8 such characters can pass the form and be refused by the server, whose message the forms show (`spec-prerelease-trust-pass.md:184`). The 1,024 maximum stays in bytes, so 512 `é` pass and 513 fail; the criteria pin both units. |

## 7. Acceptance Criteria

"Red" = fails on `6f74c6ff`; "guard" = passes there and must keep passing. Count pop-ups as the
shipped tests do. Web criteria stub only `fetch`; admin-only calls answer 403 for a normal user,
as the server does (P3).

**Input-shape matrix.** One row per item; a cell names the criterion or why the shape does not apply.

| Item | Admin / normal user | Menu expanded, collapsed, phone | Typed address | Settings, no sub-page | Unsaved changes | No saved value | Password lengths |
|---|---|---|---|---|---|---|---|
| Menu | AC-111, AC-112, AC-121 to AC-123 | AC-111 | AC-113, AC-121, AC-122 | n/a: Settings index is REQ-701 | n/a: no form | n/a | n/a |
| Theme | AC-211 | n/a: page content | n/a: route unchanged | n/a | n/a: saves at once | n/a | n/a |
| Naming | AC-311, AC-312 (admin); normal: AC-712 | n/a | AC-712 | AC-713 | n/a: no form in the box | n/a: no read | n/a |
| Indexers | AC-411 to AC-417 (admin); normal: n/a, page admin-only. Add door (inline form only, ST-009) x protocols x box checked/unchecked: AC-411, AC-415; toggled edits: AC-412; untouched edits stored on/off: AC-416; flag omitted everywhere: AC-413, AC-417 | n/a | n/a: route unchanged | n/a | n/a: dialog saves on submit | n/a | n/a |
| Test buttons | AC-511 to AC-528 (admin); normal: n/a | n/a | n/a | n/a | typed AC-514, AC-515; provider and preset writers AC-519 to AC-521; language restored with no Save AC-525, default language restored AC-526; save outcomes AC-516, AC-517, AC-522, AC-523; an older settings read arriving after a save AC-528; edit while saving: both writes held AC-524, lone metadata write held AC-527 (a), metadata held after the default language settles AC-527 (b), default language held after metadata settles AC-527 (c) | AC-512; Audnexus n/a (ST-011) | n/a |
| Unmapped | AC-611, AC-612; Manual Import link admin only (AC-614) | AC-611 | AC-612 | n/a | n/a | AC-613, AC-614 (no root folder) | n/a |
| Normal users | AC-711 to AC-724 (role change under an open pop-up AC-724) | AC-711 | AC-712 | AC-713 | n/a | AC-717 (one language); language read failing: 500 and rejected fetch, Add New AC-721, header AC-722; list re-read without the picked language AC-723 | n/a |
| Password | AC-911 to AC-916 | n/a | n/a | n/a | n/a | n/a | ASCII AC-911, AC-912; `é` AC-911, AC-915; refused change keeps old password and session AC-916 |

**REQ-101** (real `Sidebar`; real `App`):
- [x] **AC-111** (red): for an admin, with the menu expanded, collapsed, and in the phone drawer,
  none of Bookshelf, Calendar, Cutoff Unmet, Profiles, Custom Formats, Development shows, and no
  "More" heading shows. General, Notifications and Tags show greyed with the "Coming Soon" title
  and are not links.
- [x] **AC-112** (red): the same for a normal user, where Tags shows greyed and General and
  Notifications do not show (unchanged).
- [x] **AC-113** (red): each of `/shelf`, `/calendar`, `/wanted/cutoff`, `/settings/profiles`,
  `/settings/customformats`, `/settings/development`, opened by an admin and by a normal user,
  shows "Page Not Found" and not the old title. Guard: `/wanted/missing` still opens Missing;
  `/settings/tags` still shows "Tags".

**REQ-102** (real `App`):
- [x] **AC-121** (red): `/calendar`, opened by an admin and by a normal user, shows "Page Not
  Found" and "This page does not exist.", and not "This feature is coming in a future release."
- [x] **AC-122** (red): `/no-such-page`, opened by an admin and by a normal user, shows the same.
- [x] **AC-123** (guard): `/settings/general` and `/settings/notifications` for an admin, and
  `/settings/tags` for an admin and a normal user, each show their own title and "This feature is
  coming in a future release."

**REQ-201** (real `UISettingsPage`):
- [x] **AC-211** (red): no "Theme" heading and no "Light" or "Dark" button; "Date Format" and
  "Relative Dates" still show.

**REQ-301** (real `MediaManagementPage`, admin):
- [x] **AC-311** (red): the page shows D-301's sentence under "File locations", which names Readarr
  Import's chosen root folder and says nothing about edits moving files; it shows none of
  "Naming", "Rename Files", "Author Folder Format", "File Management", "Create empty author
  folders"; and it sends no `GET /config/naming`.
- [x] **AC-312** (red): the Root Folders help tip reads D-302's text and no longer contains
  "Author/Title subfolders".

**REQ-401** (real `IndexersPage`, admin):
- [x] **AC-411** (red): in the edit dialog and in the inline add form the Interactive Search box
  is enabled; unchecking it in the inline add form and saving sends `enableInteractiveSearch:
  false` (the dialog's edit requests are AC-412).
- [x] **AC-412** (red): editing an indexer stored with Interactive Search on: unchecking it sends
  `enableInteractiveSearch: false`; editing one stored off and checking it sends `true`.
- [x] **AC-413** (red): no create or update request carries `enableAutomaticSearch`; neither form
  shows "Automatic Search".
- [x] **AC-414** (red): neither list shows an "Auto" badge for an indexer stored with Automatic
  Search on; an indexer with Interactive Search off shows no "Interactive" badge (guard for the on
  case).
- [x] **AC-415** (red): through the one add door (the inline form; the dialog only edits, ST-009),
  for a torrent and a Usenet indexer, leaving the box checked sends `enableInteractiveSearch: true` and unchecking it sends
  `false`.
- [x] **AC-416** (guard): editing only the name of an indexer stored with Interactive Search on sends
  `true`, and of one stored off sends `false`.
- [x] **AC-417** (red): every request in AC-415 and AC-416 omits `enableAutomaticSearch`.

**REQ-501** (real `MetadataPage`, `AppToaster`):
- [x] **AC-511** (red): with nothing changed, Test in each section sends exactly one POST to its
  route; a 200 shows exactly one pop-up with the success text of REQ-501.
- [x] **AC-512** (red): Hardcover answered 400 "Hardcover API token not configured" shows exactly
  one error pop-up with that text; the AI test answered 400 "LLM model not configured" likewise.
- [x] **AC-513** (red): 502 with a message, and a network failure, each show exactly one error
  pop-up with the server's message or "Unable to reach Livrarr".
- [x] **AC-514** (red): after typing in any field (for example the Audnexus URL), Test sends no
  POST and shows exactly one pop-up "Save your changes first. Test checks the saved settings."
  The same after adding a language, and after changing the default language.
- [x] **AC-515** (red): typing a field and then typing the saved value back: Test sends its POST.
- [x] **AC-516** (red): typing a Hardcover token and saving successfully: the token field is
  blank afterwards and Test sends exactly one POST (today the token stays and the form stays
  dirty, P2). The same after saving a language change.
- [x] **AC-517** (red): a save whose service part is answered 500: Test still asks to save first,
  and the entered values are still shown.
- [x] **AC-518** (red): while a test is pending its button reads "Testing..." and is disabled.
- [x] **AC-519** (red): from saved settings, changing only the AI Provider: each Test sends no
  test and no save request and shows the save-first message.
- [x] **AC-520** (red): with a saved custom model, choosing "Back to preset models": the same.
- [x] **AC-521** (red): after AC-519, returning the provider, endpoint and model to their saved
  values: Test sends exactly one POST.
- [x] **AC-522** (red): with a service field and the default language both changed, Save; (a) the
  service PUT succeeds while the default-language PUT is held, then fails; (b) the reverse order.
  In both, while one part is held and after it fails, Test sends no POST and the failed part's
  entered value is still shown; the succeeded part shows its saved value.
- [x] **AC-523** (red): after AC-522, Save again with the failed part answered 200: the page shows
  the saved values, secret fields blank, and Test sends exactly one POST.
- [x] **AC-524** (red): change Audnexus from its saved value A to B and change the default
  language, press Save, and hold both PUTs. Every writer listed in REQ-501 is disabled; an
  attempt to type C into Audnexus or to change a language leaves B and the chosen default shown.
  Release both PUTs (a) both 200, and (b) metadata 200 and default language 500: in each case
  every writer is enabled again once both have settled.
- [x] **AC-527** (red): the save lock holds while any submitted write is pending. In each case below
  the user changes Audnexus from its saved value A to B, presses Save, and then, while a write is
  held, tries to type C into Audnexus and to change a language; every writer listed in REQ-501
  stays disabled, B and the chosen default language stay shown, and C never replaces B:
  (a) a service-only save (default language unchanged, so one PUT) with that metadata PUT held;
  (b) a two-part save where the default-language PUT is answered first, 200 and separately 500,
  while the metadata PUT is held; (c) a two-part save where the metadata PUT is answered first,
  200 and separately 500, while the default-language PUT is held. In every case the writers are
  enabled again only after the last submitted reply settles, and not before.
- [x] **AC-525** (red): saved languages `["en"]` with Google Books configured (so the toggles
  show): enabling French, then disabling it, with no Save between, Test sends exactly its one
  POST, no PUT, and no save-first pop-up; between the two clicks Test shows the save-first pop-up
  and sends nothing. With saved `["en","fr","de"]`, disabling French and enabling it again (now
  `["en","de","fr"]`) still counts as a change: Test shows the save-first pop-up.
- [x] **AC-526** (red): changing the default language from the saved English to French and back to
  English, with no Save between, Test sends exactly its one POST, no PUT, and no save-first
  pop-up; while French is selected, Test shows the save-first pop-up and sends nothing.

**REQ-601**:
- [x] **AC-611** (red): for an admin, the Activity group shows "Unmapped Files" linking to
  `/unmapped`, expanded, collapsed and in the phone drawer; for a normal user it does not show.
- [x] **AC-612** (red): a normal user at `/unmapped` lands on `/` and no `GET /rootfolder` is sent;
  guard: an admin at `/unmapped` sees "Unmapped Files".
- [x] **AC-613** (red): on the Unmapped page in root-folder mode with no root folders, "Go to
  Settings" points to `/settings/mediamanagement`.
- [x] **AC-614** (red): on Manual Import, a scanned file that no root folder can take shows
  "Configure root folder" pointing to `/settings/mediamanagement`, still opening in a new tab.

**REQ-701**:
- [x] **AC-711** (red): for a normal user, expanded, collapsed and in the phone drawer, the menu
  shows none of Manual Import, Readarr Import, Media Management, Status, Logs; Download Clients,
  UI and About Livrarr still show. Guard: an admin sees all five.
- [x] **AC-712** (red): a normal user at each of `/import`, `/import/readarr`,
  `/settings/mediamanagement`, `/system/status`, `/system/logs` lands on `/`, and none of those
  pages' admin-only calls is sent. Guard: an admin at each sees the page.
- [x] **AC-713** (red): a normal user at `/settings` ends at `/settings/ui` showing "UI Settings";
  guard: an admin at `/settings` sees "Media Management".
- [x] **AC-714** (red): a normal user's path-not-found pop-up and bell entry show no "Configure
  path mapping" link and still show "Get AI help"; guard: an admin's show both.
- [x] **AC-715** (red): a normal user on Add New, with the language route answering
  `["en","fr","de"]`, sees the language picker offering English, French and German, and sends no
  `GET /config/metadata`.
- [x] **AC-716** (red): the same in the header search.
- [x] **AC-717** (guard): with `["en"]` there is no picker, for either role.
- [x] **AC-718** (red): with `["fr","en"]`, Add New preselects French for a normal user.
- [x] **AC-719** (red, server): `GET /api/v1/config/languages` as a normal user and as an admin
  answers 200 with an object whose only key is `languages`, equal to the saved list; with a
  Hardcover token, AI endpoint, AI key and Google Books key saved, none of those values appears
  in the body. Signed out: 401.
- [x] **AC-720** (guard, server): `GET /api/v1/config/metadata` as a normal user still answers 403.
- [x] **AC-721** (red: today the page reads `/config/metadata`): Add New opened fresh at `/search`
  with no `lang` in the address, as an admin and as a normal user, with the language read answered
  500, and separately rejected as a network failure: after it settles the search box still
  searches in English (`GET /work/lookup` without a non-English language), no language picker
  shows, no new error message or pop-up shows, and no `GET /config/metadata` is sent.
- [x] **AC-722** (red, same reason): the same two failures for the header search, for both roles:
  a search still opens `/search?q=…` with no `lang` (English), no picker, no new message, no
  `GET /config/metadata`.

**REQ-901** (real router, `SqliteDb`):
- [x] **AC-911** (red): on each of setup, own profile, admin create and admin edit, an ASCII
  7-character password and seven `é` are each refused with 422 and "invalid password: minimum 8
  characters". Setup and admin create make no account; on profile and admin edit the old password
  still signs in.
- [x] **AC-912** (guard): on each door, ASCII 8 and ASCII 1,024 characters are accepted and the new
  password signs in; ASCII 1,025 is refused.
- [x] **AC-913** (guard): profile and admin edit without a password field still succeed.
- [x] **AC-914** (guard): an account whose existing password has 6 characters signs in.
- [x] **AC-915** (guard): on each door, eight `é` and 512 `é` (1,024 bytes) are accepted and sign in; 513 `é` (1,026 bytes) is refused.
- [x] **AC-916** (red: today the 7-character change is accepted and ends the sessions): on profile
  and admin edit, a session issued before a refused change (7 characters) still authenticates
  afterwards, and the old password still signs in.

### Added in code review

Each pins a behaviour that GPT-6 Astra's code review r1 found and the criteria above did not
name. Each test was red on the main assertion before its fix and green after it
(`build/reviews/settings-honesty/packet-8-code-fix-r1/`, red and green logs per finding).

- [x] **AC-528** (REQ-501; review r1, C1): a save's reply is the saved value, and a read of the
  same settings that is still in flight when the reply arrives never replaces it. With the page
  loaded, the user enters a new Audnexus address; a background read of the settings (started by
  the tab regaining focus) is held with the old value; the user saves. (a) The save succeeds,
  then the older read is answered: the page still shows the saved address, and Test sends its one
  POST with no save-first pop-up. (b) Guard: the older read is answered while the save is held,
  then the save succeeds: the same. (c) The service part saves, the default-language part is held
  and then fails, and the older read is answered between: the saved address and the entered
  default language both stay shown, and Test asks to save first until the default language is
  set back. The same three cases for the default-language read, with the service part held and
  failed in (c). Tests: the six cases under "A settings read sent before a save's reply never
  replaces that reply" (`frontend/src/pages/settings/metadata/MetadataPage.test.tsx:1233`;
  cases at `:1234`, `:1252`, `:1271`, `:1298`, `:1316`, `:1335`). (a) and (c) were red; (b) passed
  before the fix.
- [x] **AC-723** (REQ-701; review r1, C2): the search language follows one rule. A language in the
  address wins (Add New only). Otherwise the pick starts at the first saved language, and it is
  reset to the first saved language whenever the language list is read again with different
  contents; a re-read with the same contents keeps the user's pick. This is the rule the two
  search boxes followed before this feature, when they read the list with the rest of the
  metadata settings. So when a refreshed list drops the picked language, the pick returns to the
  first saved language even though the picker may no longer show. For each role, on Add New and
  in the header search: French picked from `["en","fr"]`, the list re-read (tab regains focus) as
  `["en"]`: no picker shows and a search is sent in English, not French; guard: re-read unchanged,
  French stays picked and a search is sent in French. Tests: "the saved language list changes
  after a language was chosen" in `frontend/src/components/Header/Header.test.tsx:387` (cases
  `:389`, `:418`) and `frontend/src/pages/search/SearchPage.test.tsx:575` (cases `:582`, `:611`).
  A re-read that adds a language or reorders the list is not pinned by a test; Astra's code
  review r2 probed both and an address language
  (`packet-7-code-review/reviewer-scratch/round2-language-ordering.log`).
- [x] **AC-724** (REQ-701; review r1, C3): the path-not-found pop-up's "Configure path mapping"
  link follows the user's current role, in both directions, while the pop-up stays open. A pop-up
  shown to an admin, then the user refreshed as a normal user without remounting anything: still
  one pop-up, the link gone from the pop-up and the bell entry, "Get AI help" still shown. The
  reverse (normal user refreshed as admin): the link appears in both. Tests: "the path mapping
  link follows a role change"
  (`frontend/src/components/Header/NotificationBell.test.tsx:636`, both directions at `:684`). The
  admin-to-normal case was red on the pop-up only.

## 8. As built

The feature was built in one checkout by `fix` subagents on Claude Opus 5.5, which also wrote the
tests (PO approval 2026-10-05). GPT-6 Astra (xhigh) was the sole reviewer: tests r1 and r2 FAIL,
r3 PASS; code r1 FAIL (C1, C2, C3, each probed), r2 PASS. The build was deployed to the local app
at 2026-10-05T23:58Z with no migration (`deploy-20261005T235801Z/DEPLOYMENT.json`); the PO answered
the live check "ok looks good" on 2026-10-06. The commit is `fcbcb080`, local.

### Files changed

From `git show --stat fcbcb080`: 32 files, 19 production and 13 test.

| Area | Production files | Test files |
|---|---|---|
| Server | `crates/livrarr-handlers/src/config.rs` (`get_languages`, `:135`), `crates/livrarr-handlers/src/types/config.rs` (`LanguagesResponse`, `:52`), `crates/livrarr-server/src/router.rs` (route, `:226-229`), `crates/livrarr-server/src/auth_service.rs` (minimum, `:123`) | `router.rs` test module (`:1533` to `:1911`) |
| Menu and routes | `frontend/src/App.tsx` (`SettingsIndex`, `:102`; catch-all, `:430`), `frontend/src/components/Sidebar/Sidebar.tsx`, new `frontend/src/components/Page/NotFoundPage.tsx` | new `frontend/src/App.routes.test.tsx`, `Sidebar.test.tsx` |
| Settings pages | `UISettingsPage.tsx`, `MediaManagementPage.tsx` (File locations, `:577-595`; help tip, `:344`), `IndexersPage.tsx`, `MetadataPage.tsx`, `frontend/src/api/index.ts` (`getLanguages` `:559`; test wrappers `:568-573`), `frontend/src/types/api.ts` (`LanguagesResponse`, `:1256`) | new `UISettingsPage.test.tsx`, `MediaManagementPage.test.tsx`, `IndexersPage.test.tsx`, `MetadataPage.test.tsx` |
| Links and normal users | `UnmappedPage.tsx`, `ManualImportPage.tsx`, `NotificationBell.tsx` (`AdminOnlyItem`, `:48`), `Header.tsx`, `SearchPage.tsx`, new `frontend/src/hooks/useEnabledLanguages.ts` | new `UnmappedPage.test.tsx`, `ManualImportPage.links.test.tsx`, `Header.test.tsx`; `NotificationBell.test.tsx`, `SearchPage.test.tsx`, `SearchPage.selectedMetadata.test.tsx`, `MissingPage.test.tsx` |

### What was added

- **Route** `GET /api/v1/config/languages`, for any signed-in user (session or API key), answering
  `{"languages": [...]}` from a one-field reply type (`crates/livrarr-handlers/src/config.rs:135-143`).
  `GET /config/metadata` stays admin-only.
- **Shared language hook** `useEnabledLanguages` (`frontend/src/hooks/useEnabledLanguages.ts:18`):
  one query on the new route, used by Add New and the header search. It holds both the enabled
  list (English alone while the list is missing or unreadable) and the picked language, with
  AC-723's rule.
- **Not-found page** `NotFoundPage` ("Page Not Found", "This page does not exist."), used only by
  the catch-all; `ComingSoonPage` still serves General, Notifications and Tags.
- **Metadata page**: a `TestButton` per service (`MetadataPage.tsx:121`); one unsaved-changes value
  (`:238`) from the form's dirty state and two drafts compared with their saved values; one save
  lock (`:241`); each save part cancels in-flight reads of its settings before storing its reply
  (`:181-182`, `:193-194`).

### Corrections found during the build

None changed a decision.

- **The indexer dialog only edits (tests stage).** See ST-009. The PM decided within scope to add
  no opener; Interactive Search is enabled and Automatic Search hidden in both forms.
- **Two older tests read the old language route (code stage, `packet-5-code/BLOCKED-C3.md`).**
  `SearchPage.selectedMetadata.test.tsx` answered only `/config/metadata`, and
  `MissingPage.test.tsx` mocked the API without `getLanguages`. Packet 6 changed the first to
  answer `/config/languages` with the same list and added `getLanguages` to the second's mock;
  their assertions are unchanged.
- **Three behaviours the spec had not named (code review r1).** AC-528, AC-723 and AC-724.

### Unchanged and now unused

These stay as they were; each is an unapproved entry in the project to-do list (`build/plans/`).

- The `/config/naming` route (`crates/livrarr-server/src/router.rs:165`), its stored row and the
  web wrapper `getNamingConfig` (`frontend/src/api/index.ts:493`) have no caller in the web app.
- The hook `frontend/src/hooks/useIndexerForm.ts`, which nothing imports, still carries the
  Automatic Search field.
- The setup wizard's steps after the account step are still unreachable (ST-020).
- The other normal-user callers of admin-only routes and the guided tour (ST-016, ST-017) still
  reach admin-only routes ("normal users, part 2").
- Saving languages on the Metadata page does not refresh the search boxes' list until their next
  read (for example when the tab regains focus).

### Limits of the evidence

- The live check covered the screens in `LIVE-CHECK.md`; the PO's answer does not say which were
  opened. A normal-user check needs a non-admin account.
- AC-528's fix cancels any read of the same settings still in flight when the reply arrives, so
  another admin's change saved in that window shows only on the next read. The HTTP request is
  not aborted; its reply is ignored. Astra r2 accepted this.
- AC-724 was driven through the production `refreshUser` only; other ways the role could change
  were not tested.
- The password maximum message still reads "maximum 1024 characters" while the limit counts bytes
  (`crates/livrarr-server/src/auth_service.rs:128-131`); unchanged by this feature.
