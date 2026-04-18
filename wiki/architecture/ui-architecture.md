# UI Architecture

React 19 SPA consuming the Livrarr REST API. Mimics Readarr's visual design.

## Stack

- **Framework:** React 19 (originally SvelteKit 5 in v2 spec, migrated to React)
- **Build:** Vite
- **State:** React Query for server state
- **Routing:** Client-side SPA routing
- **Serving:** Static files from `{data-dir}/ui/`, served by Axum. All non-API paths fall back to `ui/index.html`.

## Design Principles

1. **Mimic Readarr** — same dark theme, sidebar, toolbar, table/poster/overview views, modal patterns
2. **Works-first** — works are the landing page, not authors (key divergence from Readarr)
3. **Unified media types** — ebook + audiobook shown together per work
4. **API-driven** — pure client-side rendering, no SSR
5. **Desktop-first** — responsive, but desktop is the design target

## Navigation

Three-tier layout: fixed header, collapsible sidebar, scrollable content area.

- **Library:** Works (default), Authors, Add New, Bookshelf (coming soon), Unmapped Files
- **Activity:** Queue, History
- **Settings:** Media Management, Indexers, Download Clients, Metadata, General (coming soon), UI, User Management (admin)
- **System:** Status, Health, Logs

## Auth Flow

1. On load, check for stored session token
2. If no token or 401 from any API call → redirect to login
3. Login: POST /auth/login → store token → redirect to works
4. Token stored in localStorage (XSS surface accepted for alpha — every Servarr app does this. HttpOnly cookies deferred to beta)

## Key UI Gotchas

- **Browser refresh wipes in-memory state.** Restore from persistent source on mount. Don't fiddle with React Query config.
- **Use HelpTip component for tooltips**, not HTML title attributes or custom tooltips.
- **Notification polling:** 30s interval + on window focus. No WebSocket/SignalR.
- **Greyed-out features:** unimplemented features are visible but greyed with "Coming Soon" tooltip. Maintains layout consistency and signals intent.
- **Non-admin view:** read-only for Media Management and Download Clients. Admin-only pages hidden from sidebar entirely.
