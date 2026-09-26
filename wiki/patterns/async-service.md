# Service boundaries

Define shared use-case contracts in domain, implementations in the owning crate
and concrete wiring in server. Handlers bind only the Has* capabilities they use.
The dependency boundary is specified in [root architecture](../../ARCHITECTURE.md).

Use `trait_variant::make(Send)` for new async traits. These cannot be trait objects;
use generic or enum dispatch. Deliberately synchronous contracts can be dyn-safe:
chapter extraction and provider-call observation are documented examples, not an
exhaustive forever count of dynamic traits.

AppState shares concrete service implementations through Arc and type aliases.
The capability accessor returns a reference to the inner implementation. Prefer
explicit dependency injection; the old AppState/OnceLock construction workaround
was removed and must not become the default again.

One struct may implement several narrow contracts when that preserves clear
ownership—for example, settings versus credential access. A shared implementation
does not justify giving every caller every capability. Composite handler contracts
should name only the capabilities used by that cohesive module.

External HTTP/LLM/file seams can be doubled when appropriate. A stub is not required
for every service. Persistence tests use [real SQLite](test-doubles.md). Errors are
per-service/domain outcomes mapped at boundaries, not a single invented DomainError.

Use existing ecosystem crates rather than hand-rolling hashing, encoding, formats,
randomness or transport. Shared policy belongs in one authority even where several
protocol or format adapters legitimately implement the boundary.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/patterns/async-service.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="async-service-pattern"></a>
<a id="structure"></a>
<a id="rules"></a>
<a id="stub-policy"></a>
<a id="where-stubs-live"></a>
