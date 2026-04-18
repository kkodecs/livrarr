# Wiki Index

Domain knowledge for the Livrarr project. Grows with each build cycle. Start here, then drill into sections.

## Architecture

- [Overview](architecture/overview.md) — crate dependency graph, key invariants, composition root
- [Enrichment Pipeline](architecture/enrichment-pipeline.md) — provider stack, enrichment modes, provenance, privacy
- [Grab System](architecture/grab-system.md) — indexers, download clients, import lock, orphan adoption
- [Library Management](architecture/library-management.md) — filesystem layout, import pipeline, tag writing, CWA
- [Import Pipeline](architecture/import-pipeline.md) — scan → classify → copy → tag → CWA → track (detailed)
- [Series Matching](architecture/series-matching.md) — series discovery, author monitoring
- [RSS Sync](architecture/rss-sync.md) — automated release discovery, fuzzy matching, gap detection
- [Usenet Pipeline](architecture/usenet-pipeline.md) — SABnzbd integration, protocol routing
- [UI Architecture](architecture/ui-architecture.md) — React stack, auth flow, Readarr mimicry

## Domain Entities

- [BIG7 Overview](domain/big7.md) — the seven core entities and their relationships
- [Work](domain/work.md) — primary entity, lifecycle, provenance, semantics
- [Author](domain/author.md) — lifecycle, monitoring, relationship to works
- [Series](domain/series.md) — Goodreads-sourced, per-media-type monitoring, assignment rules
- [Release](domain/release.md) — transient search results, protocol routing, RSS sync matching
- [Grab](domain/grab.md) — download lifecycle, import lock, queue visibility
- [LibraryItem](domain/library-item.md) — file lifecycle, import path, CWA
- [List](domain/list.md) — bulk import from CSV/URL, preview → confirm → undo
- [Metadata Sources](domain/metadata-sources.md) — providers, priority, fallback, foreign language gotchas

## Patterns

- [Async Service Pattern](patterns/async-service.md) — trait + impl + stub, trait_variant, stub policy
- [Error Handling](patterns/error-handling.md) — error taxonomy, data read policies, retry semantics
- [Test Doubles](patterns/test-doubles.md) — no InMemoryDb, test DB helpers, what gets stubbed
- [Migration Pattern](patterns/migration-pattern.md) — SQLite migration rules, naming, enum serialization

## Decisions

- [Key Decisions](decisions/key-decisions.md) — hardlink policy, config, indexers, AppState, security

## Quick Reference

- [Insights](insights.md) — 28 active learnings for every session
- [Log](log.md) — wiki change log
