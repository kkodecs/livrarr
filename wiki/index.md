# Wiki Index

Domain knowledge for the Livrarr project. Grows with each build cycle.

## Architecture

- [Overview](architecture/overview.md) — crate dependency graph, key invariants, composition root
- [Enrichment Pipeline](architecture/enrichment-pipeline.md) — provider stack, enrichment modes, provenance, privacy
- [Grab System](architecture/grab-system.md) — indexers, download clients, import lock, orphan adoption
- [Library Management](architecture/library-management.md) — filesystem layout, import pipeline, tag writing, CWA
- [Series Matching](architecture/series-matching.md) — series discovery, author monitoring

## Domain Entities

- [BIG7 Overview](domain/big7.md) — the seven core entities and their relationships
- [Work](domain/work.md) — primary entity, lifecycle, provenance, semantics
- [Author](domain/author.md) — lifecycle, monitoring, relationship to works
- [Grab](domain/grab.md) — download lifecycle, import lock, queue visibility
- [LibraryItem](domain/library-item.md) — file lifecycle, import path, CWA

## Patterns

- [Async Service Pattern](patterns/async-service.md) — trait + impl + stub, trait_variant, stub policy
- [Error Handling](patterns/error-handling.md) — error taxonomy, data read policies, retry semantics
- [Test Doubles](patterns/test-doubles.md) — no InMemoryDb, test DB helpers, what gets stubbed

## Decisions

- [Key Decisions](decisions/key-decisions.md) — hardlink policy, config, indexers, AppState, security

## Quick Reference

- [Insights](insights.md) — top 24 active learnings for every session
- [Log](log.md) — wiki change log
