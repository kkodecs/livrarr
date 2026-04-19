# Active Insights

Top learnings a fresh CC session needs to know. For deeper coverage see linked wiki pages.

> **This wiki is new (April 2026).** It was bulk-ingested from build artifacts and may contain inaccuracies — especially where specs evolved faster than the ingest could track. If you find something wrong, **fix it**. Update the wiki page, update this file if affected, and append to wiki/log.md. The wiki only gets better if you correct it when you spot errors. Code is always authoritative over wiki content.

## Architecture

1. **11-crate workspace.** All deps point toward `livrarr-domain`. `livrarr-server` is the composition root. `livrarr-handlers` owns all route handlers behind a compile wall. See [overview](architecture/overview.md).
2. **BIG7 entities:** Author, Series, Work, Release, Grab, LibraryItem, List. See [big7](domain/big7.md).
3. **Work-first, not author-first.** The Work is the primary entity everywhere.
4. **One app, both formats.** Ebooks and audiobooks. Per-media-type monitoring (`monitor_ebook`, `monitor_audiobook`).
5. **SQLite WAL mode.** Every connection: `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`.
6. **Collections = root folders.** 1:1 mapping. A collection IS a root folder with a name and shared toggle.

## Coding Patterns

7. **trait + impl + stub.** Trait in domain, impl in crate, stub in behavioral. See [async-service](patterns/async-service.md).
8. **`trait_variant::make(Send)`** — not `async-trait`. Non-dyn-compatible — use generics/enum dispatch exclusively.
9. **No SQL outside livrarr-db.** No business logic in handlers. Handlers: validate → call trait → map result.
9b. **Compile wall.** `livrarr-handlers` must NOT depend on `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`. Handlers are generic over `S: AppContext`. Verify with `cargo tree -p livrarr-handlers`.
9c. **Arc<ServiceImpl> pattern.** All service fields in AppState are `Arc<T>`. AppContext type is the inner `T`. Accessor returns `&self.field` — deref coercion handles `&Arc<T>` → `&T`. Service impls don't need Clone.
9d. **Circular dep: OnceLock<Box<AppState>>.** Services that call functions taking `&AppState` (ImportService, ReadarrImportWorkflow) can't hold `AppState` directly — infinite-size type. Use `OnceLock<Box<AppState>>`: Box is pointer-sized (breaks the compile-time layout cycle), OnceLock allows post-construction init. Call `service.init(state.clone())` after AppState construction. `Arc<OnceLock<...>>` and `OnceLock<AppState>` (without Box) both fail — the compiler still needs AppState's size.
9e. **Trait signature type safety.** Service traits in `livrarr-domain/src/services.rs` must not reference types from walled-off crates — `TaggableItem` (livrarr-tagwrite), `Create*DbRequest` (livrarr-db), `TagMetadata` (livrarr-tagwrite) are banned from signatures. Use domain equivalents: `LibraryItem` for `TaggableItem`, domain request structs for DB request types. Note: `WorkId`/`UserId` are safe — they're defined in livrarr-domain, not livrarr-db. Server impls convert at the boundary.
9f. **Accessor newtype wrappers for orphan rule.** When handlers need server-owned infrastructure (logs, caches, atomics), define a minimal accessor trait in `livrarr-handlers/src/accessors.rs` and a newtype wrapper in server's `state.rs` that delegates to the real type. Required because: compile wall blocks putting the trait in server, orphan rule blocks impl'ing a handler trait on a server type, and `trait_variant::make(Send)` blocks `dyn Trait` (insight 8). Wire the wrapper as the AppContext associated type.
9g. **Handler-level spawning for background work.** Services receive `&self` and can't clone `AppContext` or move it into `tokio::spawn`. Handlers own `State<S>` and can `state.clone()` + `tokio::spawn`. Use this for fire-and-forget side effects (bibliography refresh, author monitor) and long-running background jobs (bulk refresh). OnceLock<Box<AppState>> (9d) is the escape hatch for when a service *must* access full state; handler spawning is the default.
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
