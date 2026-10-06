# User interface

## Executive summary

The web app is a React single-page app served by the backend. Read [state and
interactions](#state-and-interactions) before changing what a page caches or restores, and
[menu, routes and roles](#menu-routes-and-roles) before adding a page, a menu item or a link:
an admin-only page must be hidden from normal users at every door, and hiding it never replaces
server authorization. Settings pages have their own rules in [settings pages](settings-pages.md).

The documented frontend is a React 19/TypeScript SPA built with Vite, with React
Query for server state and client-side routing. Axum serves the built files from
the configured data directory. API and authentication routes are composed by the
backend; non-API frontend navigation falls back to the SPA entry.

The UI is Work-first and presents ebooks/audiobooks together, using the established
Readarr-style dark layout. Navigation inventories and “Coming Soon” labels in the
old page are historical release snapshots, not current feature status.

## State and interactions

- Restore durable state after reload. Do not treat an in-memory cache as the library.
- A paginated Works response contains one page; full-library consumers need the
  explicit page walk and a compatible query key.
- An Add response can precede background completion. Show actual progress and
  recoverable failure; legacy identity badges are not a progress indicator.
- A dismissed notification must persist the server decision. Stable toast IDs
  prevent duplicate display during remounts.
- Use the shared HelpTip component for help. Keep implementation/process vocabulary
  out of user decisions and recovery messages.

The documented alpha auth flow stores a session token locally and returns to login
on unauthorized responses; the token transport and security policy must be verified
at the selected revision before changes. Hiding an admin page does not replace
backend authorization. The historical notification mechanism polls; do not assume
a WebSocket stream exists.

## Menu, routes and roles

Each admin-only page is hidden from a normal (non-admin) user at every door, and the doors agree
on one list (settings-honesty, `fcbcb080`):

- **Menu.** `Sidebar.tsx` marks an item `adminOnly`, and the one filter drops it for a non-admin
  in every mode: expanded, collapsed and the phone drawer
  (`frontend/src/components/Sidebar/Sidebar.tsx:206`). A group with no visible item shows no
  heading.
- **Typed address.** The same pages are wrapped in `AdminGuard`, which sends a non-admin to `/`
  with a replace (`frontend/src/App.tsx`, `frontend/src/components/Page/AuthGuard.tsx`).
- **Links inside pages.** A link to an admin page shown to any user is wrapped so it renders only
  for an admin; the path-not-found notification's "Configure path mapping" link uses
  `AdminOnlyItem`, which reads the role itself, so a pop-up already on screen follows a role
  change (`frontend/src/components/Header/NotificationBell.tsx:48`;
  [lesson 111](../insights/coding-patterns.md#lesson-111)).

Admin-only menu items (`Sidebar.tsx:60-185`): Manual Import, Readarr Import, Unmapped Files,
Media Management, Indexers, Metadata, General and Notifications (both greyed), User Management,
Status and Logs. Each has an `AdminGuard` on its address except Notifications, a greyed
placeholder whose address any user can open. A normal user sees Library, Activity (Queue, History), Settings (Download Clients, UI, Tags greyed) and
System (About Livrarr). `/settings` with no sub-page opens Media Management for an admin and
`/settings/ui` for anyone else (`SettingsIndex`, `frontend/src/App.tsx:102`).

Normal users still reach some admin-only routes from shared pages (the guided tour, the Help
page, the author refresh, the release list's preferred formats, Download Clients' Prowlarr box);
that is open, unapproved work in the project to-do list ("normal users, part 2").

An address the app does not have shows `NotFoundPage`, "Page Not Found" and "This page does not
exist." (`frontend/src/components/Page/NotFoundPage.tsx`, catch-all at `frontend/src/App.tsx:430`).
`ComingSoonPage` is used only by the three greyed pages that remain: General, Notifications and
Tags.

Add New and the header search read the enabled languages from `GET /api/v1/config/languages`,
which any signed-in user may call and which returns only the list, through the one shared hook
`useEnabledLanguages` (`frontend/src/hooks/useEnabledLanguages.ts:18`). A language in the address
wins; otherwise the pick returns to the first saved language whenever the list is read back with
different contents.

Vite compilation is not TypeScript validation. Run the frontend’s actual typecheck
as appropriate to a code change. [Reader/notification lessons](../insights/process.md#lesson-74).

[Search-result rendering](search-results-view.md) ·
[cross-format reading/listening](../domain/cross-format-resume.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/ui-architecture.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="ui-architecture"></a>
<a id="stack"></a>
<a id="design-principles"></a>
<a id="navigation"></a>
<a id="auth-flow"></a>
<a id="key-ui-gotchas"></a>
