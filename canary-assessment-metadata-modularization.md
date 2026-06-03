# Canary Assessment — `livrarr-providers` extraction (Phase 2, analytical)

**Verdict: GO — no irreducible cycle.** The `livrarr-providers` crate can be extracted with no back-edge to `metadata`/`identity`/`enrichment`/`db`. BUT the extraction is **not a pure code-move** (the plan's original assumption): it requires co-defining the `FetchRequest` port and splitting `llm_scraper` *first*. Both were anticipated by the C′ design (feature-touched ports co-defined with their cut). This assessment is **analytical** (verified imports via code-index over `/mnt/opt/livrarr`), not a built-green extraction — that is deferred to a deliberate session (see "Why not built tonight").

## Verified back-edges and their resolutions

| # | Apparent back-edge | Real nature (verified) | Resolution | Cost |
|---|---|---|---|---|
| 1 | `livrarr_db::MetadataConfig` (hardcover.rs:7,463; llm_caller_service.rs:32; google_books tests) | **Illusory.** `livrarr-db/src/lib.rs:3` does `pub use livrarr_domain::settings::MetadataConfig` — it is a **domain** type re-exported by db. | Change imports `livrarr_db::MetadataConfig` → `livrarr_domain::settings::MetadataConfig`. **No db dependency.** | Trivial (import path) |
| 2 | `EnrichmentContext` / `EnrichmentMode` / `RequestPriority` in `ProviderClient::fetch` (provider_client.rs:26; google_books.rs:163; audible.rs:84 — often as **unused** `_ctx`) | Real: enrichment-policy type in the fetch trait signature. | Change the fetch surface to a providers-local mechanical **`FetchRequest`** (ir-v2 D-009/D-012). The `_ctx` is mostly unused, so impact is mechanical. | **Refactor** (trait sig + clients + callers) |
| 3 | `crate::cover::upscale_cover_url` (provider_client.rs:912) | Real: a pure URL util living in the materialize-bound `cover` module. | Relocate `upscale_cover_url` (pure string util) into `providers` (or `domain`). | Small (move 1 fn) |
| 4 | `crate::llm_scraper::{is_anti_bot_page, clean_html_for_llm, validate_cover_url}` (provider_client, google_books.rs:414, goodreads.rs:482,607,668) | Real, and a **design refinement**: these are *mechanical* helpers (WAF detection, HTML cleaning, URL validation) used by the provider clients — NOT scrape *policy*. | **Split `llm_scraper`**: mechanical helpers → `providers`; the scrape *policy*/prompts/ladder → `identity`. (Phase-1 said `llm_scraper` wholesale→identity; the code shows its helpers are provider mechanism.) | Medium (split a module) |
| 5 | `crate::live_config::LiveMetadataConfig` (provider_client.rs:254; google_books.rs:10) | Wrapper around the (domain) `MetadataConfig`. | Once #1 is recognized, `live_config` moves to `providers` cleanly (verify its own deps during the move). | Small |

**Crucially:** none of the cross-deps are `providers` *calling back into* metadata's resolver / `work_service` / merge **logic**. They are domain value types (#1), a signature param (#2), and pure utils (#3,#4). **No irreducible cycle → GO.**

## Recommended extraction sequence (supersedes the "pure move" canary step)

1. **Import-path fix** — `livrarr_db::MetadataConfig` → `livrarr_domain::settings::MetadataConfig` across the provider files (dissolves the scary db edge). Build green.
2. **Relocate pure utils** — `upscale_cover_url` + the mechanical `llm_scraper` helpers (`is_anti_bot_page`, `clean_html_for_llm`, `validate_cover_url`) into a `providers`-bound location; leave the scrape policy behind (→ identity). Build green.
3. **Define `FetchRequest`** + change `ProviderClient::fetch` to take it instead of `&EnrichmentContext`; map `EnrichmentContext`→`FetchRequest` at the (enrichment-side) call sites. Build green. *(This is the one genuine refactor — the feature-touched port.)*
4. **Create `crates/livrarr-providers`** + move the contract types (`NormalizedWorkDetail`, `ProviderOutcome`) and the now-decoupled modules (`provider_client`, 6 clients, `llm_caller`, `transport_cache`, `language`, `parsers`, `normalize`, the moved utils). Add a `pub use livrarr_providers::*` re-export shim in `metadata/lib.rs` (D-014). Build green.
5. **Canary gate:** `cargo build` + `cargo test` green AND `cargo tree -p livrarr-providers` lists **none** of `livrarr-metadata`, `livrarr-identity`, `livrarr-enrichment`, `livrarr-db`. → commit "extract livrarr-providers (canary GO)". Then delete the shim (AC-021).

Estimated: a focused half-day. Steps 1–2 are trivial; step 3 is the real work; step 4 is mechanical with the shim.

## Design refinements this canary surfaced (feed back into the IR)

- **`llm_scraper` must be SPLIT**, not moved wholesale to identity — its mechanical helpers are provider substrate (ir-v1/v2 module map should reflect mechanism→providers, policy→identity).
- **`MetadataConfig` is already a domain type** (`livrarr_domain::settings`) — the providers crate consumes it from `domain`, no relocation needed. (Strengthens REQ-006: the apparent db edge was a re-export artifact.)
- **`EnrichmentContext` is "theater" for identity's purposes** (PO-confirmed 2026-06-03). Its only functional field is `mode` (Background/Manual/HardRefresh), and every site that *reads* it is **merge logic** — merge-deferral + immediate-vs-deferred + hard-refresh-overwrite (`lib.rs:858-859`, `lib.rs:1652`, `provider_queue.rs:544-545`). That is an **enrichment** concern, not a fetch/identity one; the provider clients themselves take it as `_ctx` (discarded). So: `FetchRequest` carries only **{provider, key, language}**; `mode` **STAYS in enrichment** (it must NOT move to providers or domain — that would scatter enrichment logic into the foundation, the reason to reject D-012 option (b)). `priority` is labeled a "hint" and nothing branches on it — likely droppable. This makes the Step-3 refactor *behavior-preserving* for identity and confined to enrichment's call sites.
- `transport_cache` is already clean (only `livrarr_domain` + `NormalizedWorkDetail`) — moves with zero edits once the contract types relocate.

## Why not built tonight (honest)

The analysis proved the extraction is a real multi-file **refactor** (step 3: the `ProviderClient::fetch` signature across the trait + all clients + enrichment call sites; step 4: the `llm_scraper` split), not the behavior-preserving move the Phase-2 plan assumed. Doing that in an autonomous overnight loop would risk leaving a **broken worktree** for **no added decision value** — the GO/NO-GO is already answered (GO, no cycle) and the prep is identified. Per the run plan's STOP discipline (don't thrash, commit only at green, don't risk the tree), the build is deferred to a deliberate session with PO awareness of the prep. The worktree is left **clean** at the Phase-1 commit.
