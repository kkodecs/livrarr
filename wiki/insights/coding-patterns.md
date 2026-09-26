# Coding patterns lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="7-trait--impl--stub"></a>
<a id="7-trait-impl-stub"></a>
<a id="lesson-7"></a>
## 7. Service contracts and doubles

Define service contracts in domain and implementations in the owning crate. Use external I/O doubles where appropriate, but real SQLite for persistence. A stub is not required for every service; see [service boundaries](../patterns/async-service.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#7-trait--impl--stub).

<a id="8-trait_variantmakesend"></a>
<a id="lesson-8"></a>
## 8. Async traits

New async service traits use trait_variant::make(Send). They are not dyn-compatible: use generics or enum dispatch. Deliberately synchronous seams such as chapter extraction and provider-call observation can use trait objects; do not generalize the async restriction to every trait.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#8-trait_variantmakesend).

<a id="9-no-sql-outside-livrarr-db"></a>
<a id="lesson-9"></a>
## 9. SQL and business decisions

SQL belongs in livrarr-db. Handlers validate, call service contracts and map responses; shared business decisions belong behind those contracts.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9-no-sql-outside-livrarr-db).

<a id="lesson-9b"></a>
## 9b. Compile wall

Handlers must not depend on db, metadata, tagwrite or download implementation crates. Check the dependency tree. Compilation enforces import boundaries, not the absence of locally reimplemented business rules.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9b-compile-wall).

<a id="9c-arc-pattern"></a>
<a id="9c-arcserviceimpl-pattern"></a>
<a id="lesson-9c"></a>
## 9c. Shared service instances

AppState stores shared services as Arc<Implementation>. A Has* associated type names the inner implementation, and its accessor borrows it via deref coercion. The implementation itself need not be Clone.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9c-arc-pattern).

<a id="9d-circular-dep-oncelock"></a>
<a id="9d-circular-dep-oncelockboxappstate"></a>
<a id="9d-circular-dep-oncelockbox"></a>
<a id="lesson-9d"></a>
## 9d. Explicit construction

Prefer explicit constructor injection. The old OnceLock<Box<AppState>> arrangement solved a layout cycle but was subsequently removed; do not recreate late initialization from that historical workaround. Process-global infrastructure has separate ownership rules.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9d-circular-dep-oncelockbox).

<a id="9e-trait-signature-type-safety"></a>
<a id="lesson-9e"></a>
## 9e. Boundary types

Domain service signatures must not name types from implementation crates. Put appropriate contracts and request types at the shared boundary and convert inside the implementation. A type alias documents intent but does not provide newtype safety.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9e-trait-signature-type-safety).

<a id="9f-accessor-newtype-wrappers-for-orphan-rule"></a>
<a id="lesson-9f"></a>
## 9f. Accessor adapters

When handlers need server-owned state, define a narrow accessor contract and an appropriate adapter/newtype at composition. Keep orphan-rule and ownership constraints explicit. Do not pass all of AppState just to expose one operation.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9f-accessor-newtype-wrappers-for-orphan-rule).

<a id="9g-handler-level-spawning-for-background-work"></a>
<a id="lesson-9g"></a>
## 9g. Background task ownership

Handlers owning cloneable state can spawn background work through service contracts. A borrowed service cannot simply move its caller context into a task. Keep task lifetime, cancellation and errors visible. The removed trigger_monitor no-op is not a usable job entrance.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9g-handler-level-spawning-for-background-work).

<a id="9h-handlers-bind-narrow-has-capability-traits-not"></a>
<a id="lesson-9h"></a>
## 9h. Narrow handler capabilities

Bind each handler to the Has* traits it actually uses. AppContext is a broader route-composition aggregate; its existence does not require every handler to depend on all capabilities.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9h-handlers-bind-narrow-has-capability-traits-not).

<a id="9i-credential-traits-are-isolated-from-settings-traits"></a>
<a id="lesson-9i"></a>
## 9i. Credentials are separate capabilities

Credential-bearing reads are separate from configuration CRUD for indexers and download clients. Inject the credential capability only where an outbound operation needs it; ordinary settings reads should not gain secret access by convenience.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#9i-credential-traits-are-isolated-from-settings-traits).

<a id="10-all-blocking-io-in-spawn_blocking"></a>
<a id="lesson-10"></a>
## 10. Blocking I/O

Run blocking filesystem and media work through spawn_blocking rather than blocking the async executor. The outer task still owns cancellation and must account for already-admitted blocking work.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#10-all-blocking-io-in-spawn_blocking).

<a id="11-chrono-for-datetime"></a>
<a id="lesson-11"></a>
## 11. Datetime convention

Use chrono throughout the project. Do not introduce the time crate as an alternative datetime convention.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#11-chrono-for-datetime).

<a id="36-atomicbool-execution-guard--cancellationtoken"></a>
<a id="36-atomicbool-execution-guard-cancellationtoken"></a>
<a id="lesson-36"></a>
## 36. Run guards and shutdown

A workflow-wide AtomicBool guard prevents overlapping runs of the same shared workflow. If another run owns that global guard, the scheduler returns from the whole tick; ordinary per-user errors warn and continue.

Thread cancellation from the job to the workflow. Backoffs and inter-item sleeps use select with cancellation; checking only between iterations can delay shutdown for the entire sleep. The per-user iteration layer and shared guard scope must agree.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#36-atomicbool-execution-guard--cancellationtoken).

<a id="39-settingsservice-split-into-7-narrow-traits"></a>
<a id="lesson-39"></a>
## 39. Settings contracts

The former settings service was split into narrow app-config, client settings/credentials, indexer settings/credentials, root-folder and remote-mapping contracts. One implementation and shared instance can implement them all. Prowlarr configuration belongs with indexer infrastructure, not user metadata preferences.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#39-settingsservice-split-into-7-narrow-traits).

<a id="40-infraimport_pipeliners-is-pure-utilities"></a>
<a id="lesson-40"></a>
## 40. Import utilities versus orchestration

Server infra/import_pipeline.rs contains utilities taking explicit dependencies, including some explicitly supplied HTTP operations. Service coordination belongs in the import service. The word pipeline in a filename is not permission to accumulate AppState access or business orchestration.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#40-infraimport_pipeliners-is-pure-utilities).

<a id="41-module-level-composite-context-traits-for-cohesive"></a>
<a id="lesson-41"></a>
## 41. Composite handler contracts

A cohesive handler module may name a composite of its actual Has* requirements instead of repeating long bounds. Compose the narrow traits directly, not the entire AppContext; use this where the handlers truly share the same capabilities.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#41-module-level-composite-context-traits-for-cohesive).

<a id="lesson-103"></a>
## 103. Checked invocation facade

A raw keyed-mutex guard does not prove which composition or user it protects.
The accepted F4 design requires a checked cross-crate facade borrowing an
unforgeable invocation capability, created only by the runner after acquisition.
Binding checks the receiving adapter's own mutex Arc identity and user before
protected reads or inner locks; another composition is refused even for the same
numeric user. The capability and facade cannot escape the invocation.

Actual worker bodies stay crate-private. Existing public service entries acquire;
the checked facade forwards to the same workers without implementing the acquiring
public trait. The invocation owns cancellation and retains the same guard: queued
cancellation starts no protected work; after acquisition, stop admitting work,
drain admitted work and settle actual outcomes/evidence before releasing it.

Source: [PM's accepted F4 scope clarification](../../build/design/DECISION-F4-SCOPE-FACADE-PM.md).
**This is accepted design clarification; runtime implementation and review are still pending.**

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md#103-checked-invocation-facade).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/coding-patterns.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="coding-pattern-insights"></a>
