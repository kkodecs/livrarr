# Active Insights

Top learnings a fresh CC session needs to know. Distilled from build cycles, retros, and operational experience.

## Architecture

1. **10-crate workspace.** All dependency arrows point toward `livrarr-domain`. `livrarr-server` is the composition root.
2. **BIG7 entities** are Author, Series, Work, Release, Grab, LibraryItem, List. These are the only media entities that matter for core workflows.
3. **Work-first, not author-first.** The Work is the primary entity in the data model, UI, and every workflow.
4. **One app, both formats.** Ebooks and audiobooks managed together. A Work is format-independent.
5. **SQLite WAL mode required.** Every connection must set `journal_mode = WAL`, `foreign_keys = ON`, `busy_timeout = 5000`.

## Coding Patterns

6. **All async services use trait + impl + stub pattern.** Trait in domain, impl in crate, stub in behavioral.
7. **`trait_variant::make(Send)` for async traits.** Not `async-trait`. Produces non-dyn-compatible traits — use generics exclusively.
8. **No SQL outside livrarr-db.** No business logic in handlers. Handlers: validate -> call trait -> map result.
9. **All blocking I/O in `spawn_blocking`.** Never block the async executor.
10. **`chrono` for datetime, never `time`.** Project-wide.

## Metadata

11. **Never use OpenLibrary for foreign language.** OL's foreign language coverage is unreliable. Period.
12. **LLM is a fallback, not the primary path.** Deterministic matching first. LLM resolves ambiguity only.
13. **LLM privacy boundary:** public-provider metadata OK to send. Filenames, paths, checksums, preferences, API keys, user IDs — never.
14. **Identity locked at add-time.** LLM validator confirms provider match when work is added.

## Data & State

15. **"Missing" (no file) is not "wanted" (monitored).** Don't conflate these concepts.
16. **Browser refresh wipes in-memory state.** Restore from persistent source on mount. Don't fiddle with React Query config.
17. **Never edit applied migrations.** sqlx checksum validation fails. Always create new migration files.
18. **INSERT OR REPLACE is banned.** Use `INSERT ... ON CONFLICT (...) DO UPDATE SET ...` for upserts.

## Process

19. **Build infra first.** Don't start with in-memory fakes unless infra is genuinely complex.
20. **No band-aids.** Always fix where data is created wrong, never add downstream workarounds.
21. **Prototype external endpoints before writing parsers.** curl the URL, inspect the response. "200 OK" doesn't mean parseable.
22. **Add duplication check after implementation.** AI writes code function-by-function — it will reimplement existing logic.
23. **No build commentary in code comments.** Comments describe what code IS, not how it got there.

## Security (Deferred to Post-Alpha)

24. SSRF validation, rate limiting, resolver pinning, cross-domain redirect rejection — 7 items deferred. See `build/foundation/security-model-policy.md`.
