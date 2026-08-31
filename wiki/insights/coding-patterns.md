# Coding Pattern Insights

Rust trait/service patterns, compile-wall mechanics, and idioms used across the workspace.

### 7. **trait + impl + stub**

7. **trait + impl + stub.** Trait in domain, impl in crate, stub in behavioral. See [async-service](patterns/async-service.md).

### 8. **`trait_variant::make(Send)`**

8. **`trait_variant::make(Send)`** — not `async-trait`. Non-dyn-compatible — use generics/enum dispatch exclusively.

### 9. **No SQL outside livrarr-db**

9. **No SQL outside livrarr-db.** No business logic in handlers. Handlers: validate → call trait → map result.

### 9b. **Compile wall**

9b. **Compile wall.** `livrarr-handlers` must NOT depend on `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`. Handlers are generic over `S: AppContext`. Verify with `cargo tree -p livrarr-handlers`.

### 9c. **Arc<ServiceImpl> pattern**

9c. **Arc<ServiceImpl> pattern.** All service fields in AppState are `Arc<T>`. AppContext type is the inner `T`. Accessor returns `&self.field` — deref coercion handles `&Arc<T>` → `&T`. Service impls don't need Clone.

### 9d. **Circular dep: OnceLock<Box<AppState>>**

9d. **Circular dep: OnceLock<Box<AppState>>.** Services that call functions taking `&AppState` (ImportService, ReadarrImportWorkflow) can't hold `AppState` directly — infinite-size type. Use `OnceLock<Box<AppState>>`: Box is pointer-sized (breaks the compile-time layout cycle), OnceLock allows post-construction init. Call `service.init(state.clone())` after AppState construction. `Arc<OnceLock<...>>` and `OnceLock<AppState>` (without Box) both fail — the compiler still needs AppState's size. **Prefer eliminating OnceLock via explicit constructor injection** (pass `Arc<ServiceImpl>` at construction time) whenever a refactor makes the deps explicit. OnceLock is the escape hatch when full refactoring is impractical. **As of the architecture-excellent sprint, zero OnceLocks remain in the codebase** — `LiveImportService` and `LiveReadarrImportWorkflow` were both refactored to explicit fields. Do not reintroduce.

### 9e. **Trait signature type safety**

9e. **Trait signature type safety.** Service traits in `livrarr-domain/src/services.rs` must not reference types from walled-off crates — `TaggableItem` (livrarr-tagwrite), `Create*DbRequest` (livrarr-db), `TagMetadata` (livrarr-tagwrite) are banned from signatures. Use domain equivalents: `LibraryItem` for `TaggableItem`, domain request structs for DB request types. Note: `WorkId`/`UserId` are safe — they're defined in livrarr-domain, not livrarr-db. Server impls convert at the boundary.

### 9f. **Accessor newtype wrappers for orphan rule**

9f. **Accessor newtype wrappers for orphan rule.** When handlers need server-owned infrastructure (logs, caches, atomics), define a minimal accessor trait in `livrarr-handlers/src/accessors.rs` and a newtype wrapper in server's `state.rs` that delegates to the real type. Required because: compile wall blocks putting the trait in server, orphan rule blocks impl'ing a handler trait on a server type, and `trait_variant::make(Send)` blocks `dyn Trait` (insight 8). Wire the wrapper as the AppContext associated type.

### 9g. **Handler-level spawning for background work**

9g. **Handler-level spawning for background work.** Services receive `&self` and can't clone `AppContext` or move it into `tokio::spawn`. Handlers own `State<S>` and can `state.clone()` + `tokio::spawn`. Use this for fire-and-forget side effects (bibliography refresh, author monitor) and long-running background jobs (bulk refresh). OnceLock<Box<AppState>> (9d) is the escape hatch for when a service *must* access full state; handler spawning is the default. **`AuthorMonitorWorkflow::trigger_monitor()` no longer exists** — the no-op stub and its trait method were deleted in the Phase-1 dead-code cleanup (commit `af709f01`, audit M-006); the trait now has only `run_monitor` (`livrarr-domain/src/services/monitor.rs:31-37`, verified 2026-07-10). The on-demand trigger from handlers uses `tokio::spawn + run_monitor` directly (this pattern).

### 9h. **Handlers bind narrow `Has*` capability traits, not…**

9h. **Handlers bind narrow `Has*` capability traits, not full `AppContext`.** Each handler function uses bounds like `S: HasWorkService + HasAuthorService` — only the capabilities it actually calls. `AppContext` is a blanket supertrait union kept for route composition (where the router needs all capabilities). All capability traits are named `Has*` and live in `livrarr-handlers/src/context.rs`. Adding a new service = add a `Has*` trait in context.rs, impl it on `AppState` in state.rs, and import the narrow bound in the handler.

### 9i. **Credential traits are isolated from settings traits**

9i. **Credential traits are isolated from settings traits.** `DownloadClientCredentialService` (provides `get_with_credentials`) is a separate trait from `DownloadClientSettingsService` (CRUD without secrets). Same split for `IndexerCredentialService` / `IndexerSettingsService`. Jobs/handlers that need plaintext credentials bind the credential trait; read-only or config-only handlers bind only the settings trait. This is compile-time RBAC groundwork — when user-tier handling arrives, credential services won't be injected into user-tier handlers.

### 10. **All blocking I/O in `spawn_blocking`**

10. **All blocking I/O in `spawn_blocking`.** Never block the async executor.

### 11. **`chrono` for datetime**

11. **`chrono` for datetime.** Never `time` crate. Project-wide.

### 36. **AtomicBool execution guard + CancellationToken…**

36. **AtomicBool execution guard + CancellationToken cooperation for background workflows.** Workflows that can be triggered by both a scheduled tick and a user-facing handler (e.g., author monitor) hold an `AtomicBool running`. `swap(true, AcqRel)` returns the prior value — if `true`, return `Err(AlreadyRunning)` immediately. The AtomicBool is **global** (not per-user): `AlreadyRunning` causes the **entire tick** to `return Ok(())` — not skip-and-continue — because the workflow is already executing for some user and attempting others would block on the same lock. All other per-user errors use warn-and-continue (never let one user's failure stop others). User-scoping pattern for scheduled jobs: iterate all users at the job layer, call the workflow per user. **CancellationToken: all sleeps must use `tokio::select!`.** Background workflows receive a `CancellationToken` threaded from scheduler → job tick → per-user workflow. ALL sleeps (inter-item delays, 429 backoffs) must use `tokio::select! { _ = sleep(dur) => {}, _ = cancel.cancelled() => return Ok(partial_report) }` — not just `cancel.is_cancelled()` at iteration boundaries. A bare `tokio::time::sleep()` blocks graceful shutdown for the full duration (a 60s 429 backoff blocks shutdown for a full minute).

### 39. **SettingsService split into 7 narrow traits**

39. **SettingsService split into 7 narrow traits.** The former 34-method god trait is now: `AppConfigService` (user/tenant preferences: naming, media mgmt, metadata config, email, language validation), `DownloadClientSettingsService` (CRUD, no secrets), `DownloadClientCredentialService` (plaintext credential access), `IndexerSettingsService` (CRUD + Prowlarr config + indexer config, no secrets), `IndexerCredentialService` (plaintext credential access), `RootFolderService` (CRUD), `RemotePathMappingService` (CRUD). Prowlarr config lives in `IndexerSettingsService` — NOT `AppConfigService` — because it is indexer infrastructure (admin), not a user preference. Handlers/jobs bind only the trait(s) they need. **Server impl stays one struct:** `LiveSettingsService<DB>` implements all 7 traits; AppState holds a single `Arc<LiveSettingsService>`. Don't split the impl into 7 structs — that complicates DB wiring unnecessarily. Adding a new settings area: new trait in `livrarr-domain/src/services/`, new `impl` block in `settings_service.rs`, new `Has*` in `context.rs`, new `Has*` impl on `AppState`.

### 40. **`infra/import_pipeline.rs` is pure utilities**

40. **`infra/import_pipeline.rs` is pure utilities — never add orchestration here.** After Phase 3 migration, this file contains only free functions that take all dependencies as explicit parameters — no AppState access, no service trait calls, no DB. Some functions do make network calls (`fetch_qbit_content_path`, `fetch_sabnzbd_storage_path`) via an explicitly-passed `HttpClient`, not through the service layer. All import orchestration (coordinating services, mutating state) lives in `LiveImportService`. When adding new import functionality: pure computation/explicit-I/O → free function here; service coordination → method on `LiveImportService`. The file name "pipeline" is misleading — it's a utility module.

### 41. **Module-level composite context traits for cohesive…**

41. **Module-level composite context traits for cohesive handler groups.** Handler modules with many tightly-coupled handlers (`opds.rs`, `manual_import.rs`) define a named composite trait (`OpdsHandlerContext`, `ManualImportHandlerContext`) built directly from `Has*` traits — narrower than `AppContext` but shared across all handlers in the module. Avoids repeating long bound lists per-function and makes the module's capability contract explicit. Pattern: `pub trait XHandlerContext: HasA + HasB + ... + Clone + Send + Sync + 'static {}` with a blanket `impl<T: HasA + HasB + ...> XHandlerContext for T {}`. These traits do NOT extend `AppContext` — they select only the `Has*` traits the module actually uses. Use this pattern when a module has 5+ handlers sharing the same set of services.
