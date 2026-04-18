# Test Doubles Pattern

## No In-Memory DB

`InMemoryDb` was deleted. All persistence tests use real SQLite `:memory:`. The 1,400+ line InMemoryDb could not faithfully reproduce SQL joins, case-insensitive matching, FK enforcement, transactions, NULL semantics, ordering, or collation (9 known divergences).

## Test DB Helpers

### Default (most tests)
Single-connection `:memory:` with `foreign_keys = ON`, `busy_timeout = 5000`, full migrations applied. Each test gets its own fresh DB.

### Shared-memory
For multi-connection semantics. Named in-memory DB with `cache=shared` and 4 connections.

### Temp file
For WAL mode, lock contention, and realistic concurrency testing. Uses tempfile crate — keep TempDir alive for test duration.

## What Gets Stubbed

| Dependency | Stub? | Why |
|-----------|-------|-----|
| HTTP clients | Yes | External API calls are non-deterministic |
| LLM responses | Yes | Expensive, non-deterministic |
| Filesystem ops | Yes | Testing logic, not I/O |
| Database | **No** | Real SQLite catches SQL bugs that stubs miss |

## Test DB Principle

Test DB helpers must apply the same connection pragmas as production. "Real SQLite, but different SQLite behavior" defeats the purpose.
