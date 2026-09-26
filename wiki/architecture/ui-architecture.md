# User interface

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
