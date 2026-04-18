# Error Handling Pattern

Governing principle: strict for authoritative state, tolerant for rebuildable state, visible for operators, version-gated for compatibility.

## Error Categories and HTTP Mapping

| Category | HTTP | When |
|----------|------|------|
| ValidationError | 400 | Bad user input |
| AuthenticationError | 401 | Missing/expired token |
| AuthorizationError | 403 | Insufficient permissions |
| NotFound | 404 | Entity doesn't exist |
| Conflict | 409 | Duplicate, stale update, state transition rejected |
| DataCorruption | 500 | Unknown enum (version gate passed) |
| Timeout | 504 | Upstream deadline exceeded |
| TransientError | 503 | SQLITE_BUSY |
| StorageError | 503 | Disk full, SQLITE_IOERR |
| ExternalDependencyError | 502 | Provider failure |

## Data Read Policies

- **Single record:** Strict parse. Return `Err(DataCorruption)` for unknown enums.
- **Bulk list (user-facing):** Skip bad rows, log error, return partial results with `totalRows`/`returnedRows`/`skippedRows`.
- **Internal enumeration:** Strict parse. Quarantine bad rows via raw SQL on primitive columns. Don't skip silently.
- **Cache/rebuildable:** Parse with fallback, invalidate, trigger rebuild.

## Retry Semantics

| Context | Retries | Backoff |
|---------|---------|---------|
| HTTP handlers (SQLITE_BUSY) | 0 (busy_timeout handles it) | — |
| Background jobs (SQLITE_BUSY) | 0 (next tick) | — |
| External APIs (background) | 2 | 1s/3s |
| External APIs (handler) | 1 | 2s |

## Handler Error Response Shape

JSON body must include: stable error code + request ID + short human-readable hint. Never leak: internal paths, stack traces, secrets, raw upstream bodies.

## Cross-Resource Operations (DB + Filesystem)

State machine pattern: persist intent -> temp file -> fsync -> atomic rename -> fsync parent dir -> finalize DB. On failure, leave state machine in current phase for recovery.
