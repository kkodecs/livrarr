# Wiki Index

Domain knowledge for the Livrarr project. Grows with each build cycle. Start here, then drill into sections.

> **Wiki is new — verify and correct.** Bulk-ingested April 2026. If wiki content conflicts with code, code wins. Fix the wiki when you spot errors.

## Architecture

- [Overview](architecture/overview.md) — crate dependency graph, key invariants, composition root
- [Enrichment Pipeline](architecture/enrichment-pipeline.md) — provider stack, enrichment modes, provenance, privacy
- [Metadata Pathway](architecture/metadata-pathway.md) — current add/enrich/merge/cover/tag flow, entry points, and improvement opportunities
- [Work-Creation Pipeline](architecture/work-creation-pipeline.md) — the five phases (identify → seed → create → enrich → materialize), the per-door identify matrix, and the M9 convergence gap (series-monitor + Readarr Pending limbo)
- [Grab System](architecture/grab-system.md) — indexers, download clients, import lock, orphan adoption
- [Library Management](architecture/library-management.md) — filesystem layout, import pipeline, tag writing, CWA
- [Import Pipeline](architecture/import-pipeline.md) — scan → classify → copy → tag → CWA → track (detailed)
- [Series Matching](architecture/series-matching.md) — series discovery, author monitoring
- [RSS Sync](architecture/rss-sync.md) — automated release discovery, fuzzy matching, gap detection
- [Usenet Pipeline](architecture/usenet-pipeline.md) — SABnzbd integration, protocol routing
- [UI Architecture](architecture/ui-architecture.md) — React stack, auth flow, Readarr mimicry
- [Identity Review Census](architecture/identity-review-census.md) — the one mint authority and its seven callers, the 6 doors into the one continuation (the two legacy conflict doors were deleted 2026-08-31), why GroupIdentity is unsafe until wave B, what manual import really feeds the matcher

## Domain Entities

- [Metadata Principles](domain/metadata-principles.md) — M1-M10: the governing principles for all metadata handling
- [BIG7 Overview](domain/big7.md) — the seven core entities and their relationships
- [Work](domain/work.md) — primary entity, lifecycle, provenance, semantics, and presentation fallback when a related Author is gone
- [Author](domain/author.md) — lifecycle, monitoring, relationship to works
- [Series](domain/series.md) — GR-backed rows + metadata stubs (sprint-c), reconcile arbitration, persisted rosters, promotion/silent resolution, ST-012 zero-/search
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
- [Goodreads](integrations/goodreads.md) — public-page scraping, disjoint Book/Work id namespaces, bounded unreadable-detail captures, and DataDome constraints

## Deployment

- [Container Permissions (PUID/PGID)](deployment/container-permissions.md) — root-start/drop model, cap set, hardened & rootless modes, Unraid/Proxmox gotchas, upgrade impact

## Decisions

- [Temporary work merge containment](decisions/merge-containment.md) — the September 2026 pause, disabled paths, preserved workflows, and title/author edit limitation
- [Key Decisions](decisions/key-decisions.md) — hardlink policy, config, indexers, AppState, security

## Insights (full text)

`insights.md` is a compact index; the full verbatim text of every insight (including every
"CORRECTED"/amendment note) lives in these theme pages under `insights/`.

- [Architecture](insights/architecture.md) — crate layout, entity model, and system-level structure
- [Coding Patterns](insights/coding-patterns.md) — Rust trait/service patterns, compile-wall mechanics, and idioms used across the workspace
- [Metadata](insights/metadata.md) — metadata source policy, enrichment merge/status rules, and provider-agnostic metadata behavior
- [Data & State](insights/data-and-state.md) — database, migration, and application-state rules
- [Process](insights/process.md) — build-process, prototyping, and operational lessons that aren't specific to one subsystem
- [Identity](insights/identity.md) — the identity matching authority, anchors, generation protocol, and identity-pipeline behavior
- [History & Review](insights/history-and-review.md) — the work-history event log, the identity-review card surface, and the F2 identity-layer cutover ceremony
- [Covers](insights/covers.md) — cover ranking, the cover write gate, and cover-source rules
- [Providers & Transport](insights/providers-and-transport.md) — outbound HTTP queue, rate limiting/breakers, and per-provider API quirks
- [Tests & Fixtures](insights/tests-and-fixtures.md) — test-suite mechanics, shared test-only state, and fixture gotchas

## Quick Reference

- [Insights](insights.md) — index of 101 active learnings; each line links to its full text under `insights/`
- [Log](log.md) — wiki change log
- [Merge/undo rewrite dropped (2026-09-23)](decisions/merge-undo-rewrite-dropped.md) — why the identity-conflict-authority rewrite was dropped, what replaces it, where the parked tree lives
