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
- **"Zero `dyn` for service traits" is not true as stated.** At least one trait under
  `livrarr-domain/src/services/` is deliberately **sync** and *is* used dynamically:
  `ChapterExtractor` (`services/chapter.rs`) is a plain `Send + Sync` trait whose single method
  is blocking, and `ImportWorkflowImpl` holds it as `Arc<dyn ChapterExtractor>` and takes one in
  its constructor. That `dyn` seam is the point: it lets `livrarr-library` carry no
  `livrarr-tagwrite` edge, with the composition root supplying `ChapterExtractorImpl` as the
  delegate. Sync-by-design traits in `services/` are where a `dyn` seam can live.
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
