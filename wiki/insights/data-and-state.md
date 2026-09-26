# Data and state lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="5-sqlite-wal-mode-and-one-write-begin-authority"></a>
<a id="lesson-5"></a>
## 5. SQLite writer admission

Production uses WAL and four connections, but SQLite still has one writer. Every write-bearing transaction starts through the shared BEGIN IMMEDIATE authority, allowing contenders to wait under busy_timeout rather than fail during deferred upgrade. Set foreign_keys and busy_timeout on every connection. Keep transactions short and database-only; atomic startup migrations/backfills run before serving.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#5-sqlite-wal-mode-and-one-write-begin-authority).

<a id="17-missing-no-file--wanted-monitored"></a>
<a id="17-missing-no-file-wanted-monitored"></a>
<a id="lesson-17"></a>
## 17. Missing versus wanted

Missing means no file is present. Wanted means the user has enabled download monitoring. Never derive one from the other.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#17-missing-no-file--wanted-monitored).

<a id="18-browser-refresh-wipes-in-memory-state"></a>
<a id="lesson-18"></a>
## 18. Persistent state after reload

A browser refresh loses in-memory state. Restore the intended state from persistent data on mount rather than changing query-cache settings to hide the problem.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#18-browser-refresh-wipes-in-memory-state).

<a id="19-never-edit-applied-migrations"></a>
<a id="lesson-19"></a>
## 19. Immutable migrations

Never edit a migration already shipped. sqlx validates checksums. Add a new migration and preserve the old file; see [migration rules](../patterns/migration-pattern.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#19-never-edit-applied-migrations).

<a id="20-insert-or-replace-is-banned"></a>
<a id="lesson-20"></a>
## 20. SQLite upserts

INSERT OR REPLACE is forbidden: it deletes then inserts, potentially changing identity and cascading foreign keys. Use INSERT ... ON CONFLICT ... DO UPDATE with explicit fields.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#20-insert-or-replace-is-banned).

<a id="21-per-media-type-monitoring"></a>
<a id="lesson-21"></a>
## 21. Independent monitoring

monitor_ebook and monitor_audiobook are independent booleans. A single monitored flag cannot represent the Work-level policy.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#21-per-media-type-monitoring).

<a id="81-one-listworks-response-is-never-the-library"></a>
<a id="lesson-81"></a>
## 81. Pagination is not the whole library

One listWorks response is one page. The historical default request capped at 1,000 recently added Works, making a large library appear empty to downstream filters. Walk pages inside one query, refresh total-page information from each response, and check abort between pages.

Use a distinct query key under the existing works prefix so invalidation still reaches it without changing the cached single-page response shape. Inserts can move offset boundaries during the walk; the recorded policy accepts that drift rather than pretending a client loop is a database snapshot.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md#81-one-listworks-response-is-never-the-library).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/data-and-state.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="data--state-insights"></a>
