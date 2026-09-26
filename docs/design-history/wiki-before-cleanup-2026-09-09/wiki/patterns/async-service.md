# Async Service Pattern

Services follow a trait + impl shape: the trait in `livrarr-domain`, the production
implementation in the owning crate.

> **The "+ stub" third leg is the exception, not the rule.**
> `livrarr-behavioral/src/stubs.rs` provides seven doubles in total —
> `StubHttpFetcher`, `StubLlmCaller`, `StubEnrichmentWorkflow`, `StubSeriesQueryService`,
> `StubImportWorkflow`, `StubRssSyncWorkflow` and `TagwriteChapterExtractor`. Most service
> traits — `WorkService`, `AuthorService`, `FileService` and the rest — have **no** stub.

## Structure

```rust
// In livrarr-domain/src/services/work.rs — trait definition
// (there is no services.rs; services/ is a module directory)
#[trait_variant::make(Send)]
pub trait WorkService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        candidate: crate::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError>;
    // ...
}

// In livrarr-metadata/src/work_service.rs — production implementation
pub struct WorkServiceImpl { /* dependencies */ }
impl WorkService for WorkServiceImpl { /* real logic */ }
```

Errors are **per-service** (`WorkServiceError`, `FileServiceError`, …) plus the shared
`ServiceError`. There is no single `DomainError` type.

## Rules

- **`trait_variant::make(Send)`** — not `async-trait`. All async traits need Send (tokio multi-threaded runtime).
- **Non-dyn-compatible** — `trait_variant::make(Send)` produces traits that can't be used with `dyn`. Use generics/monomorphization exclusively.
- **No `dyn` on async service traits** — which follows from the rule above rather than from
  discipline: a `trait_variant::make(Send)` trait cannot be made into a trait object at all.
- **"Zero `dyn` for service traits" is not true as stated.** **Exactly two** traits under
  `livrarr-domain/src/services/` are used dynamically. Both are deliberately plain and
  **synchronous** — which is precisely what makes them dyn-safe when the `trait_variant` ones
  are not:
  - **`ChapterExtractor`** (`services/chapter.rs:18`) — held as `Arc<dyn ChapterExtractor>` by
    `ImportWorkflowImpl` and threaded through its helpers
    (`livrarr-library/src/import_workflow.rs:61`, `:69`, `:1853`, `:2143`). The seam exists so
    `livrarr-library` carries no `livrarr-tagwrite` edge; the composition root supplies
    `ChapterExtractorImpl`.
  - **`ProviderCallSink`** (`services/provider_calls.rs:55`) — `Arc<dyn ProviderCallSink>` across
    `livrarr-enrichment`, `livrarr-external-data` and the composition root
    (`livrarr-server/src/main.rs:746`). Its own doc states the design directly: "Deliberately sync
    and dyn-safe (`Arc<dyn ProviderCallSink>`) so any crate can record without a db edge or a
    generics explosion."

  Every other `dyn` under `crates/` is a std or third-party trait object — `Box<dyn Error>`,
  `Box<dyn Future>`, `Box<dyn Iterator>`, `Arc<dyn Fn>`, `Box<dyn tracing_subscriber::Layer>`,
  `&dyn Debug` — never a livrarr service trait. (Enumerated from a `dyn ` text sweep over
  `crates/`, 45 hits; `tests/` and `frontend/` not swept.) **The rule to take from this:** an
  async service trait cannot be `dyn`; when a seam genuinely needs dynamic dispatch, the trait
  is written plain and sync on purpose.
- **AppState uses concrete types via type aliases** — not `Arc<dyn Trait>`, not generics with 12+ type params.

## Stub Policy

| Dependency | Stub? |
|-----------|-------|
| HTTP clients (indexer, metadata, download) | Yes |
| LLM responses | Yes |
| Filesystem operations (testing logic) | Yes |
| Database | **No** — use real SQLite `:memory:` |

## Where Stubs Live

Test stubs in `livrarr-behavioral/src/stubs.rs` — seven of them, listed at the top of this page.
That file also carries the `create_test_user` / `create_second_test_user` fixtures. Cross-crate
behavioral tests live in `livrarr-behavioral`.
