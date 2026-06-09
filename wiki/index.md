# Wiki Index

Domain knowledge for the Livrarr project. Grows with each build cycle. Start here, then drill into sections.

> **Wiki is new — verify and correct.** Bulk-ingested April 2026. If wiki content conflicts with code, code wins. Fix the wiki when you spot errors.

## Architecture

- [Overview](architecture/overview.md) — crate dependency graph, key invariants, composition root
- [Enrichment Pipeline](architecture/enrichment-pipeline.md) — provider stack, enrichment modes, provenance, privacy
- [Metadata Pathway](architecture/metadata-pathway.md) — current add/enrich/merge/cover/tag flow, entry points, and improvement opportunities
- [Grab System](architecture/grab-system.md) — indexers, download clients, import lock, orphan adoption
- [Library Management](architecture/library-management.md) — filesystem layout, import pipeline, tag writing, CWA
- [Import Pipeline](architecture/import-pipeline.md) — scan → classify → copy → tag → CWA → track (detailed)
- [Series Matching](architecture/series-matching.md) — series discovery, author monitoring
- [RSS Sync](architecture/rss-sync.md) — automated release discovery, fuzzy matching, gap detection
- [Usenet Pipeline](architecture/usenet-pipeline.md) — SABnzbd integration, protocol routing
- [UI Architecture](architecture/ui-architecture.md) — React stack, auth flow, Readarr mimicry

## Domain Entities

- [Metadata Principles](domain/metadata-principles.md) — M1-M10: the governing principles for all metadata handling
- [BIG7 Overview](domain/big7.md) — the seven core entities and their relationships
- [Work](domain/work.md) — primary entity, lifecycle, provenance, semantics
- [Author](domain/author.md) — lifecycle, monitoring, relationship to works
- [Series](domain/series.md) — Goodreads-sourced, per-media-type monitoring, assignment rules
- [Release](domain/release.md) — transient search results, protocol routing, RSS sync matching
- [Grab](domain/grab.md) — download lifecycle, import lock, queue visibility
- [LibraryItem](domain/library-item.md) — file lifecycle, import path, CWA
- [List](domain/list.md) — bulk import from CSV/URL, preview → confirm → undo
- [Cross-Format Resume](domain/cross-format-resume.md) — kash links, audio-ts coordinate, furthest-mark semantics, gotchas
- [Metadata Sources](domain/metadata-sources.md) — providers, priority, fallback, foreign language gotchas

## Patterns

- [Async Service Pattern](patterns/async-service.md) — trait + impl + stub, trait_variant, stub policy
- [Error Handling](patterns/error-handling.md) — error taxonomy, data read policies, retry semantics
- [Test Doubles](patterns/test-doubles.md) — no InMemoryDb, test DB helpers, what gets stubbed
- [Migration Pattern](patterns/migration-pattern.md) — SQLite migration rules, naming, enum serialization

## Integrations

- [OpenLibrary](integrations/openlibrary.md) — rate limits, anti-patterns, bulk dumps, contribution paths, current operational status
- [Google Books](integrations/google-books.md) — API key, 1000/day quota, fields= and gzip, no contribution path
- [Hardcover](integrations/hardcover.md) — 60/min, GraphQL depth ≤ 3, per-user token, beta API may break
- [Audnexus](integrations/audnexus.md) — 300/min rate limit, 24h cache + 304 revalidation, self-hostable as fallback
- [Goodreads](integrations/goodreads.md) — no API since 2020 (scraping); DataDome anti-bot; **we're currently 5-7x over the polite rate floor**

## Decisions

- [Key Decisions](decisions/key-decisions.md) — hardlink policy, config, indexers, AppState, security

## Quick Reference

- [Insights](insights.md) — 28 active learnings for every session
- [Log](log.md) — wiki change log
