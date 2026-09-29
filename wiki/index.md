# Livrarr wiki

## Executive summary

Start with [engineering principles](../PRINCIPLES.md) and
[product principles and architecture](../ARCHITECTURE.md). Then read the
[lesson index](insights.md) and the topics relevant to your work. A normal startup
does not require reading this entire wiki. Use [Find a topic](#find-a-topic) to reach
the one current page per subject, [Decisions](#decisions) for the September 2026 merge
containment, the dropped merge/undo rewrite and the automatic identity upgrade, and the [complete page list](#all-pages)
when a page is not in the table.

The root documents set requirements. Wiki pages explain workflows and practical
constraints; source evidence establishes what is implemented. Read dated findings
against their named checkout: the September 2026 reviewed containment source and
today's main are different baselines. Current work and acceptance belong in
[project state](../build/state/), reached through [the handoff](../HANDOFF.md).

## Find a topic

| Question | Start here |
|---|---|
| Where does a responsibility belong? | [Architecture overview](architecture/overview.md), [workflow entry-point guide](architecture/roads.md), [crate guide](crates/server.md) |
| How is a book found, created and identified? | [Book creation and identity](architecture/work-creation-pipeline.md), [identity review reference](architecture/identity-review-census.md), [search-results rendering](architecture/search-results-view.md) |
| How does metadata or a cover change? | [Enrichment, covers and tags](architecture/enrichment-pipeline.md), [metadata pathway](architecture/metadata-pathway.md), [database provider priorities](domain/enrichment-priorities.md), [search-result metadata mapping](domain/search-result-metadata.md) |
| How do files enter the library? | [Import and recovery](architecture/import-pipeline.md), [library management](architecture/library-management.md) |
| How do releases get downloaded automatically? | [Downloads](architecture/grab-system.md), [RSS sync](architecture/rss-sync.md), [Usenet pipeline](architecture/usenet-pipeline.md), [series matching](architecture/series-matching.md) |
| What do the records mean? | [Entity guide](domain/big7.md), [Work](domain/work.md), [Author](domain/author.md), [Series](domain/series.md) |
| How do readers and progress work? | [UI](architecture/ui-architecture.md), [cross-format resume](domain/cross-format-resume.md) |
| What must a change preserve? | [Metadata principles](domain/metadata-principles.md), [engineering patterns](#patterns), [lessons](insights.md) |
| What makes external providers difficult? | [Provider roles](domain/metadata-sources.md), [OpenLibrary](integrations/openlibrary.md), [Hardcover](integrations/hardcover.md), [Google Books](integrations/google-books.md), [Audnexus](integrations/audnexus.md), [Goodreads](integrations/goodreads.md) |
| How should a review be conducted? | [Review method](architecture/codebase-review-method.md), [known Add seed finding](architecture/direct-add-seed-review.md), [September review baseline](architecture/architecture-review-baseline.md), [simplification and evidence limits](architecture/architecture-review-simplification.md) |
| How is deployment configured? | [Container permissions](deployment/container-permissions.md), [key decisions](decisions/key-decisions.md) |
| What happens when an alpha6 library is upgraded, and how do I roll back? | [Upgrading from alpha6](deployment/upgrading.md) |
| What was decided about merging duplicate Works? | [Decisions](#decisions) below |

Further record references: [Release](domain/release.md), [Grab](domain/grab.md),
[LibraryItem](domain/library-item.md), [List](domain/list.md).
Implementation navigation: [domain](crates/domain.md), [database](crates/db.md),
[handlers](crates/handlers.md), [server](crates/server.md).

For browser access to these documents and the current build records, use the
[project document site](deployment/document-site.md).

## Decisions

- [Temporary work merge containment (2026-09-07)](decisions/merge-containment.md) — the September 2026 pause, disabled paths, preserved workflows, and the title/author edit limitation; superseded in part by card-edits-lift (2026-09-25)
- [Merge/undo rewrite dropped (2026-09-23)](decisions/merge-undo-rewrite-dropped.md) — why the identity-conflict-authority rewrite was dropped, what replaces it, where the parked tree lives
- [Automatic identity upgrade (2026-09-27)](decisions/automatic-identity-upgrade.md) — existing libraries convert on first start via migration 091; replaces the manual rehearse-and-approve cutover; odd-case rules; contract/IR amendment pending
- [Key decisions](decisions/key-decisions.md) — hardlink policy, config, indexers, AppState, security

## Patterns

- [Async service pattern](patterns/async-service.md) — trait + impl + stub, trait_variant, stub policy
- [Error handling](patterns/error-handling.md) — error taxonomy, data read policies, retry semantics
- [Test doubles](patterns/test-doubles.md) — no InMemoryDb, test DB helpers, what gets stubbed
- [Migration pattern](patterns/migration-pattern.md) — SQLite migration rules, naming, enum serialization

## Lessons (full text)

[insights.md](insights.md) is the compact index; the full text of every lesson, including
every correction and amendment, lives in these theme pages.

- [Architecture](insights/architecture.md) · [Coding patterns](insights/coding-patterns.md) ·
  [Metadata](insights/metadata.md) · [Data and state](insights/data-and-state.md) ·
  [Process](insights/process.md) · [Identity](insights/identity.md) ·
  [History and review](insights/history-and-review.md) · [Covers](insights/covers.md) ·
  [Providers and transport](insights/providers-and-transport.md) ·
  [Tests and fixtures](insights/tests-and-fixtures.md)

## All pages

- Architecture: [overview](architecture/overview.md), [roads](architecture/roads.md), [work-creation pipeline](architecture/work-creation-pipeline.md), [enrichment pipeline](architecture/enrichment-pipeline.md), [metadata pathway](architecture/metadata-pathway.md), [import pipeline](architecture/import-pipeline.md), [library management](architecture/library-management.md), [grab system](architecture/grab-system.md), [RSS sync](architecture/rss-sync.md), [usenet pipeline](architecture/usenet-pipeline.md), [series matching](architecture/series-matching.md), [UI architecture](architecture/ui-architecture.md), [identity review census](architecture/identity-review-census.md), [search-results view](architecture/search-results-view.md), [codebase review method](architecture/codebase-review-method.md), [direct-add seed review](architecture/direct-add-seed-review.md), [architecture review baseline](architecture/architecture-review-baseline.md), [architecture review simplification](architecture/architecture-review-simplification.md)
- Domain: [BIG7](domain/big7.md), [metadata principles](domain/metadata-principles.md), [Work](domain/work.md), [Author](domain/author.md), [Series](domain/series.md), [Release](domain/release.md), [Grab](domain/grab.md), [LibraryItem](domain/library-item.md), [List](domain/list.md), [cross-format resume](domain/cross-format-resume.md), [metadata sources](domain/metadata-sources.md), [enrichment priorities](domain/enrichment-priorities.md), [search-result metadata](domain/search-result-metadata.md)
- Crates: [domain](crates/domain.md), [db](crates/db.md), [handlers](crates/handlers.md), [server](crates/server.md)
- Integrations: [OpenLibrary](integrations/openlibrary.md), [Google Books](integrations/google-books.md), [Hardcover](integrations/hardcover.md), [Audnexus](integrations/audnexus.md), [Goodreads](integrations/goodreads.md)
- Deployment: [container permissions](deployment/container-permissions.md), [upgrading from alpha6](deployment/upgrading.md), [document site](deployment/document-site.md)
- Quick reference: [lesson index](insights.md), [change log](log.md)

## Maintaining this wiki

- Keep one current explanation per topic; link to it from related pages.
- Put a new lesson in its theme page and add one short discovery link to the index.
  State the rule, why it matters, and the evidence needed to apply it.
- Update the current explanation when a fact changes. Preserve the old account and
  link it as history; do not append contradictory present-tense accounts.
- Keep feature progress, reviews and acceptance in `build/`, not in reference pages.
- Preserve predecessors before revising design or architecture prose. Keep historical
  evidence intact, including corrections and their original sources.
- Check local links and the exact source baseline before claiming behavior.

[Change log](log.md) · [Preserved wiki history](../docs/design-history/wiki-before-cleanup-2026-09-09/README.md)

## Source and history

[Exact revision before cleanup](../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/index.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.
The list-form index that main carried until 2026-09-26 is preserved under
`build/reviews/steward-upkeep-2026-09-26/hand-merge/predecessors/`.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="wiki-index"></a>
<a id="architecture"></a>
<a id="domain-entities"></a>
<a id="integrations"></a>
<a id="deployment"></a>
<a id="insights-full-text"></a>
<a id="quick-reference"></a>
