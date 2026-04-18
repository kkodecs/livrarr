# Async Service Pattern

Every service in Livrarr follows the trait + impl + stub pattern.

## Structure

```rust
// In livrarr-domain/src/services.rs — trait definition
#[trait_variant::make(Send)]
pub trait WorkService {
    async fn get_work(&self, user_id: UserId, work_id: WorkId) -> Result<Work, DomainError>;
    async fn add_work(&self, request: AddWorkRequest) -> Result<Work, DomainError>;
    // ...
}

// In livrarr-metadata/src/work_service.rs — production implementation
pub struct WorkServiceImpl { /* dependencies */ }
impl WorkService for WorkServiceImpl { /* real logic */ }

// In livrarr-behavioral/src/stubs.rs — test stub
pub struct StubWorkService { /* configurable responses */ }
impl WorkService for StubWorkService { /* returns configured data */ }
```

## Rules

- **`trait_variant::make(Send)`** — not `async-trait`. All async traits need Send (tokio multi-threaded runtime).
- **Non-dyn-compatible** — `trait_variant::make(Send)` produces traits that can't be used with `dyn`. Use generics/monomorphization exclusively.
- **No `dyn` on async service traits.** The codebase uses zero `dyn` for service traits.
- **AppState uses concrete types via type aliases** — not `Arc<dyn Trait>`, not generics with 12+ type params.

## Stub Policy

| Dependency | Stub? |
|-----------|-------|
| HTTP clients (indexer, metadata, download) | Yes |
| LLM responses | Yes |
| Filesystem operations (testing logic) | Yes |
| Database | **No** — use real SQLite `:memory:` |

## Where Stubs Live

Test stubs in `livrarr-behavioral/src/stubs.rs`. Cross-crate behavioral tests in `livrarr-behavioral`.
