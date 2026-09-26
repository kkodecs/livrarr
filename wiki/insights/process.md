# Process lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="14-llm-privacy-boundary"></a>
<a id="lesson-14"></a>
## 14. Public metadata assistance has limits

Follow root privacy policy: LLM context may contain permitted public metadata, never local filenames/paths/checksums, reading history, preferences, user identifiers or credentials. Typed context values help enforce the boundary. The [root privacy rule](../../ARCHITECTURE.md#privacy-by-default) now explicitly permits integration credentials only for authentication to their configured service; that exception does not put credentials into book metadata or LLM context.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#14-llm-privacy-boundary).

<a id="23-build-infra-first"></a>
<a id="lesson-23"></a>
## 23. Build persistence realistically

Use real SQLite early. A convenient in-memory object model does not reproduce SQL joins, foreign keys, transactions, collation or null semantics. External I/O can be stubbed independently.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#23-build-infra-first).

<a id="24-no-band-aids"></a>
<a id="lesson-24"></a>
## 24. Fix the originating fault

Repair where incorrect data or behavior is created. A downstream workaround can hide the fault while leaving other consumers exposed.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#24-no-band-aids).

<a id="25-prototype-external-endpoints-before-writing-parsers"></a>
<a id="lesson-25"></a>
## 25. Prototype external contracts

Inspect the actual endpoint response before writing a parser or designing around a parameter. A 200 status and one successful end-to-end example do not prove payload shape or each parameter’s semantics.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#25-prototype-external-endpoints-before-writing-parsers).

<a id="26-add-duplication-check-after-implementation"></a>
<a id="lesson-26"></a>
## 26. Review structural duplication

After implementation, inspect whether the change recreated an existing rule or workflow. Function-by-function work can miss a shared owner. Use independent review where the authorized workflow calls for it; this lesson grants no additional agent authority.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#26-add-duplication-check-after-implementation).

<a id="27-no-build-commentary-in-code-comments"></a>
<a id="lesson-27"></a>
## 27. Comments describe the code

Code comments explain behavior, invariants and reasoning needed by maintainers. Keep the narrative of build rounds and agent activity in process records.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#27-no-build-commentary-in-code-comments).

<a id="35-path-mapping-needs-boundary-check"></a>
<a id="lesson-35"></a>
## 35. Path prefix boundaries

A remote mapping prefix must match a complete path component. String starts_with can confuse /data/downloads with /data/downloads2. Preserve separator/exact-match validation.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#35-path-mapping-needs-boundary-check).

<a id="37-ssrf"></a>
<a id="lesson-37"></a>
## 37. HTTP trust follows URL provenance

Administrator-configured infrastructure can legitimately live on a LAN: download clients, indexers, approved Readarr origins and configured LLM endpoints use the trusted transport class. Runtime-derived URLs from provider bodies or request input require SSRF protection. Do not replace every user-configured URL with the public-only resolver.

Readarr origin approval is exact and normalized, accepts private-answer/rebinding risk for that origin, and uses no-redirect requests so API keys cannot be forwarded. Removal restores the public-only rule. Grab trust derives from configured origins rebuilt after changes. This trust distinction does not waive canonical HTTP ownership.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#37-ssrf).

<a id="46-doorroad-wiring-is-untested--trace-it-at-design"></a>
<a id="46-doorroad-wiring-is-untested-trace-it-at-design"></a>
<a id="lesson-46"></a>
## 46. Trace every entry into the workflow

A pipeline test does not prove handlers/jobs actually call it. At design and validation, enumerate each affected entrance and follow its values through the canonical workflow. Adding a field to a struct or recording a call is not proof of persistent behavior. New creation/enrichment doors need real entry-path coverage.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#46-doorroad-wiring-is-untested--trace-it-at-design).

<a id="47-explicit-system-truths-pay-off"></a>
<a id="lesson-47"></a>
## 47. State real system facts

Specs need concrete environment truths: provider capabilities, quotas, anti-bot responses, storage behavior and actual parameter semantics. A placeholder System Truths heading prevents no mistakes.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#47-explicit-system-truths-pay-off).

<a id="49-speed-baseline-2026-06-10-d1b8768--the-a6"></a>
<a id="49-speed-baseline-2026-06-10-d1b8768-the-a6"></a>
<a id="lesson-49"></a>
## 49. Performance evidence and live data

The old serial-scatter claim was refuted: within-Work provider calls were already concurrent. Historical timings and release gates must not become current recommendations. Measure the actual selected path and isolate provider pacing from local concurrency. Before authorized write-heavy experiments, preserve recoverable data and check known defects; no such experiment is authorized by this wiki cleanup.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#49-speed-baseline-2026-06-10-d1b8768--the-a6).

<a id="74-frontend-session-fixes-trio-2026-07-17-all"></a>
<a id="lesson-74"></a>
## 74. Reader, notification and transfer edge cases

An EPUB opening with a non-linear spine item can strand epub.js next at location zero. With no saved position, start at the first linear item; saved progress wins. Programmatic navigation and built-in controls need the same observable behavior.

Toast dismissal must persist the server decision and use a stable toast ID, otherwise unread notifications reappear after remount. A complete seedbox download may not yet exist on the local mount; pathNotFound can be a transient transfer condition. Series Monitor must forward real provider keys, masking only genuine stub keys.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#74-frontend-session-fixes-trio-2026-07-17-all).

<a id="75-dev-restartsh-green--ci-green-for-the-frontend"></a>
<a id="75-dev-restartsh-green-ci-green-for-the-frontend"></a>
<a id="lesson-75"></a>
## 75. Dev build is not frontend typechecking

The documented dev restart runs Vite, which does not typecheck. Verify frontend changes through the project’s TypeScript check/build command before claiming readiness. A shell wrapper ending in a successful print can hide an earlier failed CI watcher: read the actual job conclusion.

Published release tags do not move; patch forward after publication. Generate changelogs against final tag state. Older pending-guess fixes and release-cycle decisions are preserved as history, not new shipping instructions.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md#75-dev-restartsh-green--ci-green-for-the-frontend).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/process.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="process-insights"></a>
