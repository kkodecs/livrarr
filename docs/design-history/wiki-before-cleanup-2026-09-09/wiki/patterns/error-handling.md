# Error Handling Pattern

Governing principle: strict for authoritative state, tolerant for rebuildable state, visible for operators, version-gated for compatibility.

## Error Categories and HTTP Mapping

The concrete type is `ApiError` in `livrarr-handlers/src/types/api_error.rs` — 23 variants. The
rows below are the ones this page has always covered, with the variant that actually produces
each status:

| Category | `ApiError` variant | HTTP | When |
|----------|--------------------|------|------|
| Bad user input | `BadRequest` | 400 | Malformed request |
| Validation failure | `Validation`, `Unprocessable` | **422** | Field-level validation — **not** 400 |
| AuthenticationError | `Unauthorized` | 401 | Missing/expired token |
| AuthorizationError | `Forbidden` | 403 | Insufficient permissions |
| NotFound | `NotFound` | 404 | Entity doesn't exist |
| Conflict | `Conflict`, `ConflictDetailed` | 409 | Duplicate, stale update, state transition rejected |
| DataCorruption | `Db(DbError::DataCorruption)` | 500 | Unknown enum (version gate passed) |
| Service at capacity | `ServiceUnavailable`, `ServiceUnavailableRetry` | 503 | Backpressure; the retry variant carries `Retry-After` |
| ExternalDependencyError | `BadGateway`, `StructuredBadGateway` | 502 | Provider failure |

Two rows this page used to carry have **no** corresponding variant and no such mapping:

- **Timeout → 504.** `ApiError` has no `Timeout` variant and nothing returns `504`.
- **StorageError → 503 (disk full, SQLITE_IOERR).** `DbError::Io` maps to **500**, not 503.
  Likewise `SQLITE_BUSY` produces no 503 — it is absorbed by the connection's `busy_timeout`.

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
