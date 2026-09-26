# Test Doubles Pattern

## No In-Memory DB

`InMemoryDb` was deleted. All persistence tests use real SQLite `:memory:`. The 1,400+ line InMemoryDb could not faithfully reproduce SQL joins, case-insensitive matching, FK enforcement, transactions, NULL semantics, ordering, or collation (9 known divergences).

## Test DB Helper

`livrarr-db::test_helpers` exposes **exactly one** helper, `create_test_db()`: a single-connection
`sqlite::memory:` pool with `foreign_keys = ON` and `busy_timeout = 5000`, all migrations applied,
plus one extra unique index on `works(user_id, normalized_title, normalized_author)`. Each call
returns a fresh DB.

There is no shared-memory (`cache=shared`) helper and no temp-file helper in that module. A test
needing WAL, lock contention or multi-connection semantics builds its own pool.

## What Gets Stubbed

| Dependency | Stub? | Why |
|-----------|-------|-----|
| HTTP clients | Yes | External API calls are non-deterministic |
| LLM responses | Yes | Expensive, non-deterministic |
| Filesystem ops | Yes | Testing logic, not I/O |
| Database | **No** | Real SQLite catches SQL bugs that stubs miss |

## Test DB Principle

Test DB helpers must apply the same connection pragmas as production. "Real SQLite, but different
SQLite behavior" defeats the purpose.

Note the gap this principle is aimed at: `create_test_db` sets `foreign_keys` and `busy_timeout`,
but production also sets `journal_mode = WAL`, `synchronous = NORMAL`, `journal_size_limit` and
`wal_autocheckpoint`, and runs a 4-connection pool. A `:memory:` DB cannot use WAL, so the two
will never match exactly.
