# Data & State Insights

Database, migration, and application-state rules.

### 5. **SQLite WAL mode and one write-begin authority**

5. **SQLite WAL mode and one write-begin authority.** Every connection: `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`. Every write-bearing transaction in `livrarr-db` starts through `pool::begin_write` (`BEGIN IMMEDIATE`), never deferred `pool.begin()`: reserving the single SQLite writer at BEGIN lets contenders wait under `busy_timeout`; a deferred read-to-write upgrade can fail immediately with `SQLITE_BUSY`. Keep transactions DB-only and short; startup cutover/backfill transactions are intentionally atomic and run before serving traffic.

### 17. **"Missing" (no file) ≠ "wanted" (monitored)**

17. **"Missing" (no file) ≠ "wanted" (monitored).** Don't conflate.

### 18. **Browser refresh wipes in-memory state**

18. **Browser refresh wipes in-memory state.** Restore from persistent source on mount.

### 19. **Never edit applied migrations**

19. **Never edit applied migrations.** sqlx checksum validation fails. Always create new migrations. See [migration-pattern](patterns/migration-pattern.md).

### 20. **INSERT OR REPLACE is banned**

20. **INSERT OR REPLACE is banned.** Use `INSERT ... ON CONFLICT (...) DO UPDATE SET ...`.

### 21. **Per-media-type monitoring**

21. **Per-media-type monitoring.** `monitor_ebook` and `monitor_audiobook` are independent booleans, not a single `monitored`.

### 81. **One `listWorks()` response is never "the library"…**

81. **One `listWorks()` response is never "the library" (#177, fixed 75f1d24f).** `GET /work` is paginated — server default page_size 100, hard cap 1000 (`crates/livrarr-handlers/src/types/pagination.rs:20`) — and the frontend's `listWorks()` defaults to `page=1&page_size=1000` sorted `date_added desc` (`frontend/src/api/index.ts:167-170`), i.e. the 1000 most-recently-added works. Any consumer treating that single response as all works silently truncates at 1000 — the Missing page did exactly this and answered "No missing items" on a 7.2k-work library. The fixed pattern: walk all pages inside ONE queryFn under a distinct `"works"`-prefixed query key (`["works","missing-all"]` — the prefix keeps every existing `invalidateQueries({queryKey:["works"]})` site covering it), refreshing the page count from EVERY response (a running list import grows the library mid-walk; freezing page-1's total truncates), abort-checked between pages — `MissingPage.tsx` walk, precedent `WorksPage.tsx:196-231`. Never write the walk's result into the bare `["works"]` entry: Search/Queue/History consume it as a single paginated response (pinned by `MissingPage.test.tsx`'s sibling integration test). Known-latent same-class sites: #180 (SearchPage:80, QueuePage:51, HistoryPage:105, MergeDialog:25). Boundary-shift drift of offset pagination (inserts shifting page boundaries mid-walk) is ACCEPTED — no client-side fix exists.
