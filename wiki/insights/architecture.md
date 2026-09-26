# Architecture lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="1-17-crate-workspace"></a>
<a id="lesson-1"></a>
## 1. Workspace boundaries

The workspace has 17 members. Dependencies point toward domain; server is the composition root. Use root ARCHITECTURE.md for allowed edges and its dated inventory for actual declarations. Old counts and unused-crate claims are not current proof.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#1-17-crate-workspace).

<a id="2-big7-entities"></a>
<a id="lesson-2"></a>
## 2. Core entities

Work, Author, Series, Release, Grab, LibraryItem and List describe the core product concepts. Six are user-scoped persistent records; Release is transient. See [the entity guide](../domain/big7.md) for distinctions and navigation.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#2-big7-entities).

<a id="3-work-first-not-author-first"></a>
<a id="lesson-3"></a>
## 3. Work is primary

Model and present the book as a Work. An author is associated metadata, not the required starting point for every user workflow.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#3-work-first-not-author-first).

<a id="4-one-app-both-formats"></a>
<a id="lesson-4"></a>
## 4. One Work spans both formats

Ebooks and audiobooks belong in the same app and can share a Work. Their file records and monitoring policies remain independent.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#4-one-app-both-formats).

<a id="6-collections--root-folders"></a>
<a id="6-collections-root-folders"></a>
<a id="lesson-6"></a>
## 6. Collections and roots

The historical collection concept maps to a root folder. Do not infer a root field on Work: LibraryItem carries the root/path relationship, while user_id supplies ownership. Check the actual surface before using collection terminology.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#6-collections--root-folders).

<a id="38-per-crate-reference-docs-in-wikicrates"></a>
<a id="lesson-38"></a>
## 38. Crate navigation

The four wiki/crates pages explain ownership and starting points. Exact methods, types and caller sets belong to source navigation; copied inventories age quickly and are not a substitute for Serena references.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#38-per-crate-reference-docs-in-wikicrates).

<a id="48-the-canonical-architecture-model-is-live-at"></a>
<a id="lesson-48"></a>
## 48. Canonical architecture model

docs/canonical-model.yaml defines the intended entity spine, legal crate seams, flows and amendments. Feature IR domain_entities lists only the concepts introduced or touched; [] is valid. Marked entity names and workspace dependency edges are gated, while feature-internal conceptual edges are not. Spine or seam changes need an amendments entry.

The canonical checker compares entity names to source; an audit receipt must name its revision. Historical Release and library/tagwrite conformance counts are dated evidence, not a live backlog. Report gate friction rather than weakening a gate locally; resolve it through an amendment or framework fix.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#48-the-canonical-architecture-model-is-live-at).

<a id="64-discoverysearch-is-its-own-service"></a>
<a id="lesson-64"></a>
## 64. Discovery has separate ownership

DiscoveryService owns provider lookup, filtering and eager suggestions; WorkService owns lifecycle operations. They share provider and identity infrastructure without duplicating policy. Old constructor signatures and method counts are historical navigation, not an interface specification.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#64-discoverysearch-is-its-own-service).

<a id="66-interactive-add-is-fast-path--background-completion"></a>
<a id="66-interactive-add-is-fast-path-background-completion"></a>
<a id="lesson-66"></a>
## 66. Add returns before background completion

Interactive Add creates or selects the local Work and then returns while provider capture, enrichment and a conditional delayed refresh continue. Local creation is not complete metadata. Other creation doors can have different timing.

The enriching registry is process-local, so restart recovery must use persistent state. Background follow-ups must carry the correct origin and fresh identity generation. The actual HTTP Add route, not a similarly named service test, needs validation. Search-card art does not imply an explicit manual cover selection. Versioned cover URLs permit immutable image caching; missing covers must remain uncached.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md#66-interactive-add-is-fast-path--background-completion).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/architecture.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="architecture-insights"></a>
