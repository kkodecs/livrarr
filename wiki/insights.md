# Active Insights

Top learnings a fresh CC session needs to know. For deeper coverage see linked wiki pages.

## Architecture

1. **10-crate workspace.** All deps point toward `livrarr-domain`. `livrarr-server` is the composition root. See [overview](architecture/overview.md).
2. **BIG7 entities:** Author, Series, Work, Release, Grab, LibraryItem, List. See [big7](domain/big7.md).
3. **Work-first, not author-first.** The Work is the primary entity everywhere.
4. **One app, both formats.** Ebooks and audiobooks. Per-media-type monitoring (`monitor_ebook`, `monitor_audiobook`).
5. **SQLite WAL mode.** Every connection: `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`.
6. **Collections = root folders.** 1:1 mapping. A collection IS a root folder with a name and shared toggle.

## Coding Patterns

7. **trait + impl + stub.** Trait in domain, impl in crate, stub in behavioral. See [async-service](patterns/async-service.md).
8. **`trait_variant::make(Send)`** — not `async-trait`. Non-dyn-compatible — use generics/enum dispatch exclusively.
9. **No SQL outside livrarr-db.** No business logic in handlers. Handlers: validate → call trait → map result.
10. **All blocking I/O in `spawn_blocking`.** Never block the async executor.
11. **`chrono` for datetime.** Never `time` crate. Project-wide.

## Metadata

12. **Never use OpenLibrary for foreign language.** Period. See [metadata-sources](domain/metadata-sources.md).
13. **LLM is a fallback.** Deterministic matching first. LLM resolves ambiguity only. Fully functional without LLM.
14. **LLM privacy boundary:** public metadata OK. Filenames, paths, checksums, prefs, keys, IDs — never.
15. **Identity locked at add-time.** LLM validator confirms provider match when work is added.
16. **Foreign works skip English enrichment.** `metadata_source` stored on work — refresh skipped for foreign-source works.

## Data & State

17. **"Missing" (no file) ≠ "wanted" (monitored).** Don't conflate.
18. **Browser refresh wipes in-memory state.** Restore from persistent source on mount.
19. **Never edit applied migrations.** sqlx checksum validation fails. Always create new migrations. See [migration-pattern](patterns/migration-pattern.md).
20. **INSERT OR REPLACE is banned.** Use `INSERT ... ON CONFLICT (...) DO UPDATE SET ...`.
21. **Per-media-type monitoring.** `monitor_ebook` and `monitor_audiobook` are independent booleans, not a single `monitored`.
22. **Enrichment timeout is 10s per provider** (amended from 3s). Hardcover GraphQL regularly takes 2-5s.

## Process

23. **Build infra first.** Don't start with in-memory fakes unless infra is genuinely complex.
24. **No band-aids.** Fix where data is created wrong, never add downstream workarounds.
25. **Prototype external endpoints before writing parsers.** curl the URL first. See [metadata-sources](domain/metadata-sources.md) for gotchas.
26. **Add duplication check after implementation.** AI will reimplement existing logic.
27. **No build commentary in code comments.** Comments describe what code IS, not how it got there.
28. **SABnzbd `search` parameter searches by name, not nzo_id.** See [usenet-pipeline](architecture/usenet-pipeline.md).
