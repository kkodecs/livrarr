# Errors and recovery

## Executive summary

Be strict with data that is the source of truth, tolerant with data that can be rebuilt, and
always show a failure to the user or operator ([read policy](#read-policy),
[HTTP mapping](#http-mapping)). In the web app, a failed load shows an inline error with Retry
and a failed action shows one pop-up from its own handler; there is no global handler
([showing failures in the UI](#showing-failures-in-the-ui)). Sonner, the pop-up library, has
two timing traps: a re-issue under the same id while a pop-up is leaving or just gone is lost,
and a test with a paused clock must step it before a pop-up appears or after one is removed
([Sonner under a paused test clock](#sonner-under-a-paused-test-clock)).

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
- <a id="sonner-under-a-paused-test-clock"></a>**Sonner under a paused test clock**
  (2026-10-04, silent-failures-2). Sonner 2.0.7 adds a toast from a `setTimeout`, and it
  finishes a removal over two animation frames after the element has gone: `removeToast` calls
  `ToastState.dismiss`, which waits one frame, and the toaster waits another before it marks
  every toast with that id as deleted. A same-id toast issued in those frames is deleted too
  (`frontend/node_modules/.pnpm/sonner@2.0.7_react-dom@19.2.4_react@19.2.4__react@19.2.4/node_modules/sonner/dist/index.mjs:947-969`,
  `:182-188`). A Playwright test that pauses the page clock must therefore step it after an
  action that reports, before it looks for the toast (50 ms is enough), and step it again after
  a toast has gone, before it expects a same-id toast to show (100 ms). Examples:
  `frontend/e2e-stubbed/audio-playback-failure.spec.ts:444`, `:557`. In real time the second
  window is about two frames, so the app's fixed-id playback-failure pop-up
  (`frontend/src/pages/reader/playbackFailure.ts:10-14`) treats a report there as lost and
  relies on the play button as the lasting signal (`spec-silent-failures-2.md` D2).
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
