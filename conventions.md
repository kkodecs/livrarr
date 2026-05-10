# Livrarr — Conventions

Extracted from observed patterns. For checkable rules, see `build/foundation/standards.md`.

## Naming

- **Crates:** `livrarr-{name}` (hyphenated)
- **Traits:** `{Resource}Db` for persistence, `{Service}Service` for orchestration
- **Handlers:** one file per resource group under `handlers/`
- **Files:** snake_case for Rust, kebab-case for frontend components
- **DB enums (single-word):** `lowercase` — e.g., `ebook`, `audiobook`
- **DB enums (multi-word):** `snake_case` — e.g., `will_retry`, `original_title`
- **API enums:** `#[serde(rename_all = "snake_case")]` or explicit renames

## Project Structure

```
crates/
  livrarr-domain/       # types, traits, zero dependencies
  livrarr-db/           # SqliteDb impls of *Db traits
  livrarr-http/         # SSRF, rate limiting, HTTP client
  livrarr-metadata/     # providers, enrichment, LLM
  livrarr-matching/     # release parsing, scoring, reconciliation
  livrarr-download/     # torznab, grab, download clients
  livrarr-library/      # import workflow, tag writing, CWA
  livrarr-handlers/     # Axum handlers, DTOs, middleware
  livrarr-server/       # composition root, main, state, jobs
  livrarr-tagwrite/     # EPUB/audiobook metadata writing
frontend/
  src/
    pages/              # route-level components
    components/         # shared UI components
    stores/             # zustand stores
    api/                # API client, types
    hooks/              # custom React hooks
    utils/              # formatting, pagination
```

## Patterns

- **Handler shape:** validate → call trait → map result. No business logic, SQL, or file I/O.
- **Response DTOs:** distinct from domain structs. Never serialize domain types directly to API.
- **Error mapping:** domain errors → handler AppError → HTTP status. Taxonomy in standards.md.
- **Async traits:** native `async fn` + `#[trait_variant::make(Send)]`. No `#[async_trait]` on new code.
- **Test DB:** real SQLite `:memory:` via `create_test_db()`. No in-memory fakes.
- **Stubs:** HTTP clients, LLM, filesystem logic. Never DB.
- **File I/O:** always inside `tokio::spawn_blocking`.
- **Temp files:** `tempfile` crate. Never PID/timestamp-based names.

## Git

- **Branch:** feature branches off `main`
- **Commits:** imperative mood, lowercase prefix (`fix:`, `feat:`, `chore:`, `docs:`)
- **No force push to main**

## Frontend

- **Framework:** React 19 + TypeScript + Vite + TailwindCSS
- **State:** zustand stores, React Query for server state
- **Tooltips:** `HelpTip` component, not HTML `title` or custom tooltips
- **Formatting:** utility functions in `utils/format.ts`
- **Import order:** React → third-party → local components → local utils → types
