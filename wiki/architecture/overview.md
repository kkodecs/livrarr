# Architecture overview

Livrarr manages a Work across ebook and audiobook formats. The authoritative
structure and allowed dependencies are in [root architecture](../../ARCHITECTURE.md);
the intended entity/seam model is [canonical-model.yaml](../../docs/canonical-model.yaml).
This page is navigation, not a second dependency specification.

The documented reviewed baseline has 17 workspace members. `livrarr-server` is the
composition root, with 13 declared production workspace dependencies in the
September 9 inventory. Dependency declarations do not prove runtime wiring.
Older wiki inventories reporting 11 or an entirely unused jobs crate describe a
different source revision. [Baseline and evidence limits](architecture-review-baseline.md).

## Responsibility map

| Concern | Owners |
|---|---|
| Shared records, contracts and deterministic comparison | `domain` |
| SQL, migrations, atomic persistent changes | `db` |
| Outbound HTTP, pacing, breakers, retries and trust classes | `http` |
| Provider-specific requests and parsing | `external-data` |
| Capturing and deciding identity evidence | `identity`; settlement coordination in `metadata` |
| Descriptive metadata and field arbitration | `enrichment` |
| Work lifecycle, discovery, refresh and monitoring | `metadata` |
| Release parsing and scoring | `matching` |
| Indexers, release search and download clients | `download` |
| Organized library files and import | `library` |
| Cover/artifact projection and format-specific tags | `materialize`, `tagwrite`; orchestration wired by server |
| HTTP request/response handling | `handlers` |
| Composition, startup, jobs and router wiring | `server` |

The remaining members are `jobs`, `cli` and `behavioral`. Crate names above omit
the `livrarr-` prefix. The frontend is a React/TypeScript SPA using the HTTP API.

Handlers use narrow service capabilities behind the compiler-enforced dependency
boundary. That boundary prevents direct imports of implementation crates; it
does not, by itself, prove that handlers contain no business decisions.

## Follow a workflow

- [Book creation and identity](work-creation-pipeline.md)
- [Enrichment, covers and tags](enrichment-pipeline.md)
- [Import and recovery](import-pipeline.md)
- [Release downloads](grab-system.md) and [RSS automation](rss-sync.md)
- [Workflow entry-point guide](roads.md)

Use Serena for symbol/reference navigation. The [crate pages](../crates/server.md)
identify ownership and useful starting points without duplicating method signatures.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/overview.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="crate-dependency-graph"></a>
<a id="key-architectural-invariants"></a>
<a id="compile-wall-phase-5"></a>
<a id="composition-root-livrarr-server"></a>
<a id="frontend"></a>
