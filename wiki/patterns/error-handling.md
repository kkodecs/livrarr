# Errors and recovery

Be strict for authoritative state, tolerant for rebuildable data, visible to
operators and explicit about version compatibility. Map domain errors at the HTTP
boundary; do not leak credentials, paths, stack traces or raw provider bodies.

## Read policy

| Data | Handling |
|---|---|
| One authoritative record | Reject corrupt/unknown values |
| User-facing bulk list | Skip corrupt rows only under the documented partial-result contract; log and expose counts |
| Internal authoritative enumeration | Strict read; quarantine explicitly rather than silently omit |
| Cache/rebuildable data | Invalidate an unreadable entry and rebuild |

Missing related data can permit a specific presentation fallback without weakening
identity decisions. See [Work](../domain/work.md). A malformed provider response is
not an authoritative empty result or a healthy transport success.

## HTTP mapping

The documented ApiError surface maps malformed input to 400, validation to 422,
unauthenticated/forbidden to 401/403, missing to 404, conflicts to 409, provider
failure to 502 and explicit service-capacity refusal to 503. Database corruption
or I/O maps to 500 in the inspected reference. Verify the real enum/mapping when
changing a route; there was no generic Timeout→504 or storage-error→503 mapping.

SQLite busy timeout and shared immediate write admission handle contention; do not
invent a second handler retry policy from historical tables. External pause/retry
classification belongs to the canonical transport and provider boundary. Queue-full
and circuit-open must not be mistaken for permanent absence or spent retry budget.

Cross-resource changes need an explicit recoverable state machine: persistent
intent, temporary bytes, required fsync/rename, and final state. The exact ordering
belongs to its authority; do not reuse an illustrative sequence without checking
the cover/import/undo contract. Keep original good data on failure, expose recovery
and retain ownership through admitted work during cancellation.

## Showing failures in the UI

As of 2026-09-30 (errors-and-delete-pass):

- **Failed load:** show `ErrorState` with Retry where the content would have been.
  Never show an empty state such as "No results" or "No notifications" for a failed
  request.
- **Failed action:** show `toast.error` from that mutation's own `onError`. There is
  no global query or mutation error handler.
- **Repeated background failures:** reading and listening position saves go through
  one helper, `frontend/src/pages/reader/savePosition.ts`. It shows one persistent
  warning per failure run under a fixed toast id. The next success clears the
  warning, and nothing retries. Sonner keeps a dismissed toast mounted for about
  200 ms. A toast re-issued under the same id during that time is merged into the
  leaving one and lost, so the helper waits for `onDismiss` plus a margin before
  showing a new run's warning.
- **Book search:** when every source that was tried fails, the search answers 502
  (`WorkServiceError::AllProvidersFailed`) and the result is never cached. Skipped
  sources are told apart from failed ones.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/patterns/error-handling.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="error-handling-pattern"></a>
<a id="error-categories-and-http-mapping"></a>
<a id="data-read-policies"></a>
<a id="retry-semantics"></a>
<a id="handler-error-response-shape"></a>
<a id="cross-resource-operations-db--filesystem"></a>
