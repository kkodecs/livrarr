# Metadata Pathway

This document explains how metadata moves through Livrarr today, where the
major accuracy and speed decisions happen, and where the pipeline can improve.

Code is the source of truth. Some older wiki pages describe intended behavior
from previous pipeline phases; this document is based on the current
implementation in `crates/livrarr-metadata`, `crates/livrarr-handlers`,
`crates/livrarr-server`, and `crates/livrarr-db`.

## Goals

The metadata pathway has four jobs:

1. Create or find the canonical `Work`.
2. Gather normalized metadata from external and source providers.
3. Merge provider fields without overwriting protected user metadata.
4. Materialize the result: database fields, provenance, cover cache, and file
   tags.

The practical quality target is that every entry path should converge on the
same final metadata for the same work. Manual import, Readarr import, list
import, author monitor, and direct add should differ only in the seed data they
start with.

## High-Level Flow

```text
entry point
  -> WorkService::add(...) or WorkService::refresh(...)
  -> WorkServiceImpl::run_unified_enrichment(...)
  -> EnrichmentWorkflowImpl::enrich_work(...)
  -> EnrichmentServiceImpl::enrich_work(...)
  -> DefaultProviderQueue::dispatch_enrichment(...)
  -> provider clients fetch normalized metadata
  -> retry state persists provider outcomes and success payloads
  -> LLM validator optionally checks identity and rejects bad payloads
  -> MergeEngine chooses field winners
  -> SqliteWorkRepository::apply_enrichment_merge(...)
  -> cover download
  -> library item retagging
```

## Entry Points

| Entry point | Application workflow role | Metadata path |
|-------------|---------------------------|---------------|
| Direct work add | A user explicitly adds a work from search or another UI add flow. This is the normal interactive "put this title in my library" path. | Handler calls `WorkService::add`; enrichment runs synchronously before the add result is returned. |
| Manual import | A user imports local files that already exist on disk. Livrarr must identify or create the matching work before copying/tagging the file. | `manual_import::import` -> `find_or_create_work` -> `WorkService::add` -> file import. |
| List import | A user confirms rows from an imported list, such as a Goodreads/Hardcover CSV or external list preview. Each selected row becomes a work. | `ListService::confirm` -> lookup/row normalization -> `WorkService::add` -> import tagging. |
| Readarr import | Livrarr ingests an existing Readarr library or Readarr-managed books. This is a migration/sync path with comparatively rich source metadata. | Readarr workflow builds `SourceProviderData` -> `WorkService::add` -> injected `Readarr` provider outcome. |
| Author monitor auto-add | Livrarr discovers new or missing works for a monitored author and adds them without a direct per-title user action. | Author monitor fetches provider bibliography -> filters/dedupes -> `WorkService::add`. |
| Refresh and bulk refresh | A user or maintenance workflow asks Livrarr to re-check metadata for existing works. This does not create a new work. | `WorkService::refresh` resets enrichment/retry state -> `run_unified_enrichment`. |
| Background retry job | Scheduled recovery for incomplete, failed, stale, or retryable enrichment work. This keeps metadata moving after transient provider failures or interrupted adds. | Job finds due work -> `EnrichmentWorkflow::enrich_work(..., Background)` -> cover download if available. |
| RSS sync | Automated release discovery for monitored works. It primarily consumes metadata for matching rather than creating arbitrary metadata records. | RSS workflow matches releases against existing works; enrichment is indirect/downstream of imports or adds. |

### Direct Work Add

API handlers in `crates/livrarr-handlers/src/work.rs` call `WorkService::add`.
This is the normal path for user-initiated adds from search or UI flows.

The add request may include provider keys, title, author, year, language,
monitor flags, cover URL, series fields, and optional `SourceProviderData`.

### Manual Import

Manual import lives in `crates/livrarr-handlers/src/manual_import.rs`.

Current shape:

```text
manual_import::import
  -> import_single_item
  -> find_or_create_work
  -> WorkService::add
  -> import_service.import_single_file
```

Manual import is metadata-sparse today. `find_or_create_work` builds an
`AddWorkRequest` with title, author, optional OpenLibrary keys, and language,
but it passes no Goodreads key, no cover URL, and no source provider data.

That means manual import depends heavily on enrichment providers to recover
identifiers, descriptions, series, and covers. If provider discovery or
validation fails, the imported file can succeed while the work remains poorly
enriched.

### List Import

List import lives in `crates/livrarr-metadata/src/list_service.rs`.

Current shape:

```text
ListService::confirm
  -> load preview rows
  -> ol_lookup(...)
  -> WorkService::add
  -> tag work with import id
```

Rows are processed with bounded concurrency. The entry seed usually comes from
CSV/list fields plus OpenLibrary lookup results.

### Readarr Import

Readarr import lives in `crates/livrarr-server/src/readarr_import_workflow.rs`.

This is the richest entry path. It builds `SourceProviderData` from Readarr
book and edition data:

- description
- ISBN and ASIN
- publisher
- genres
- page count
- rating and rating count
- cover URL
- series name and position

`WorkService::add` injects that source payload into the enrichment pipeline as
a synthetic `Readarr` provider success. The merge engine then arbitrates it
against Hardcover, Goodreads, OpenLibrary, and Audnexus instead of blindly
trusting it.

### Author Monitor Auto-Add

Author monitoring lives in
`crates/livrarr-metadata/src/author_monitor_workflow.rs`.

Current shape:

```text
AuthorMonitorWorkflow::run
  -> fetch OpenLibrary author works
  -> filter and dedupe
  -> WorkService::add with provenance_setter = AutoAdded
```

This path usually starts from OpenLibrary author/work data and then runs the
same enrichment flow as other adds.

### Refresh and Bulk Refresh

Single-work refresh and bulk refresh enter through work handlers and call
`WorkService::refresh`.

Current shape:

```text
WorkService::refresh
  -> db.reset_enrichment_for_refresh
  -> enrichment.reset_for_manual_refresh
  -> run_unified_enrichment(work, None)
```

Refresh clears retry state enough to allow provider calls to run again, then
uses the same unified enrichment path.

### Background Retry Job

The retry job lives in `crates/livrarr-server/src/jobs/enrichment.rs`.

It finds:

- works with retryable provider states due now
- stale unenriched works from interrupted adds
- failed works without provider retry state

It then calls `EnrichmentWorkflow::enrich_work(..., EnrichmentMode::Background)`
under a timeout and downloads covers when the refreshed work has a cover URL.

### RSS Sync

RSS sync lives in `crates/livrarr-metadata/src/rss_sync_workflow.rs`.

RSS sync is metadata-adjacent rather than a primary enrichment entry point. It
uses existing work metadata to match releases and trigger grabs/imports.
Metadata quality directly affects RSS matching accuracy, but RSS sync does not
itself run the full enrichment pipeline for arbitrary works.

## WorkService Add Path

`WorkServiceImpl::add` is the front door for work creation.

Principal responsibilities:

- clean title and author
- normalize title and author for matching
- detect existing works before insert
- create missing author records
- extract Goodreads key from a valid Goodreads detail URL
- create the `works` row
- write add-time provenance
- run synchronous enrichment
- return the post-enrichment work

Pseudocode:

```rust
fn add(user_id, req):
    cleaned_title = clean_title(req.title)
    cleaned_author = clean_author(req.author_name)

    if cleaned_title is empty:
        return validation error

    normalized_title = normalize_for_matching(cleaned_title)
    normalized_author = normalize_for_matching(cleaned_author)

    existing = db.find_by_normalized_match(user_id, normalized_title, normalized_author)

    source_provider_data = req.source_provider_data

    if existing work found:
        if source_provider_data exists:
            run_unified_enrichment(existing, source_provider_data)
            return reloaded existing work
        return existing work

    author_id = find_or_create_author(cleaned_author)

    gr_key = req.gr_key
        or extract_gr_key(req.detail_url) if detail_url is a valid Goodreads URL

    work = db.create_work(
        title = cleaned_title,
        author = cleaned_author,
        normalized identity,
        provider keys,
        cover_url,
        language,
        series,
        monitor flags,
        import id,
    )

    if insert lost a race to another caller:
        if source_provider_data exists:
            run_unified_enrichment(existing, source_provider_data)
        return existing work

    write_addtime_provenance(work, req.provenance_setter or User)

    status = run_unified_enrichment(work, source_provider_data)
    updated_work = db.get_work(work.id)

    return AddWorkResult(updated_work, status)
```

Important behavior:

- Add is synchronous from the caller's perspective. Enrichment failure does not
  fail the add, but the add waits for the enrichment attempt to finish.
- Existing-work matches only re-enrich when the caller supplies
  `SourceProviderData`. A sparse duplicate add returns the current existing
  state without new provider calls.
- Goodreads `detail_url` is not stored directly on `Work`; the add path extracts
  `gr_key` so later Goodreads enrichment can use a direct detail URL.

## Unified Enrichment

`WorkServiceImpl::run_unified_enrichment` is the post-add and post-refresh
materialization wrapper.

Pseudocode:

```rust
fn run_unified_enrichment(user_id, work, source_provider_data):
    if source_provider_data exists:
        enrichment.inject_source_data(user_id, work.id, source_provider_data)

    result = enrichment.enrich_work(user_id, work.id, Background)
    if result is error:
        return Failed

    post_enrich_work = db.get_work(user_id, work.id)
    if reload fails:
        return Failed

    if !post_enrich_work.cover_manual and post_enrich_work.cover_url exists:
        download_cover_to_disk(
            cover_url,
            data_dir / "covers" / user_id,
            work.id,
        )
        remove stale thumbnail on success

    items = db.list_taggable_items_by_work(user_id, work.id)
    if items not empty:
        tag_results = tag_service.retag_library_items(post_enrich_work, items)
        update tag status and merge_generation per item

    return result.enrichment_status
```

Important behavior:

- It never returns an error to the caller. Failures are logged and become
  `EnrichmentStatus::Failed` or non-fatal side effects.
- Cover download is downstream of a merged `cover_url`. Providers do not write
  cover files directly.
- Tag sync is downstream of enrichment. Existing library items should converge
  to the latest database metadata after enrichment.

## Core Enrichment Service

`EnrichmentServiceImpl::enrich_work` owns provider dispatch, validation, merge,
and CAS application.

Pseudocode:

```rust
fn enrich_work(user_id, work_id, mode):
    acquire per-work lock

    work = db.get_work(user_id, work_id)
    generation = db.get_merge_generation(user_id, work_id)

    scatter = queue.dispatch_enrichment(work, mode)

    if source data was injected:
        scatter.outcomes[Readarr] = Success(normalized_source_data)

    current_work = db.get_work(user_id, work_id)
    current_provenance = db.list_work_provenance(user_id, work_id)

    provider_outcomes = classify(scatter.outcomes)

    if scatter.deferred and mode == Background:
        return without merge

    reconstructed = {}
    for each provider outcome:
        if Success:
            read normalized_payload_json from provider_retry_state
        else:
            keep class with no payload

    validation = validator.validate(current_work, current_provenance, reconstructed)
    if validator errors:
        log and pass original outcomes through

    if validation rejected all Success payloads:
        db.apply_enrichment_merge(status = Conflict, no field update)
        return Conflict

    priority_model = PriorityModel::for_language(current_work.language)

    for attempt in 0..3:
        merge = merge_engine.merge(
            current_work,
            current_provenance,
            validation.reconstructed,
            mode,
            priority_model,
        )

        outcome = db.apply_enrichment_merge(
            expected_merge_generation = generation,
            work_update = merge.work_update,
            status = merge.enrichment_status,
            provenance changes,
            external id changes,
        )

        if outcome is Applied, NoChange, or Deferred:
            return reloaded work and merge status

        if outcome is Superseded:
            reload current_work, provenance, and generation
            retry

    return MergeSuperseded
```

Important behavior:

- The per-work lock prevents concurrent enrichment in one process.
- `merge_generation` gives database-level compare-and-swap protection against
  concurrent writers.
- Success payloads are reconstructed from `provider_retry_state`, not trusted
  only from in-memory provider results.
- LLM validation is identity protection, not the primary field selector. If it
  fails internally, the pipeline logs and continues.

## Provider Queue

`DefaultProviderQueue::dispatch_enrichment` fans out to providers and records
phase-1 outcomes.

Pseudocode:

```rust
fn dispatch_enrichment(work, context):
    to_dispatch = []
    suppressed_open = []

    for provider in registered providers:
        if provider is not applicable to work:
            continue

        if provider already has terminal retry state:
            continue

        if provider circuit breaker is Open:
            suppressed_open.push(provider)
            continue

        to_dispatch.push(provider)

    spawn one task per provider:
        acquire provider concurrency permit
        acquire provider rate-limit token
        call provider_client.fetch(work, context)

    gather all tasks:
        missing or panicked provider task -> PermanentFailure(ProviderPanic)

    for each provider outcome:
        apply retry budget rules
        persist provider_retry_state
        update circuit breaker
        add to outcome map

    for open-circuit providers:
        persist Suppressed outcome

    conflict_present = any outcome is Conflict

    if conflict_present:
        deferred = false
    else if mode == Background:
        deferred = any outcome cannot merge
    else:
        deferred = false

    return ScatterGatherResult(outcomes, merge_eligible, deferred)
```

Provider outcome classes:

- `Success`
- `NotFound`
- `NotConfigured`
- `WillRetry`
- `PermanentFailure`
- `Conflict`
- `Suppressed`

Speed controls:

- per-provider concurrency semaphore
- GCRA token bucket rate limiter
- circuit breaker
- retry budget
- background deferral when a provider outcome is not merge-safe

## Provider Clients

Provider clients live in `crates/livrarr-metadata/src/provider_client.rs` and
normalize source-specific results into `NormalizedWorkDetail`.

Current providers:

- `Hardcover`: GraphQL metadata provider. Uses result selection and edition
  fetching for richer identifiers and covers.
- `Goodreads`: HTML search/detail path. Direct detail is preferred when
  `work.gr_key` exists. Search requires LLM disambiguation for candidate choice;
  if the LLM cannot select a candidate, Goodreads returns no payload.
- `OpenLibrary`: REST metadata provider. Useful for OL keys, descriptions, and
  ISBNs. It currently does not produce a cover URL in the normalized detail.
- `Audnexus`: audiobook-focused provider for narration, duration, and ASIN-like
  audio metadata.
- `Readarr`: synthetic provider produced from injected `SourceProviderData`, not
  a network provider in this queue.

## Merge Rules

The merge engine resolves provider results field by field.

Fields considered:

- title, subtitle, original title
- author name
- description
- year
- series name and position
- genres and language
- page count and duration
- publisher and publish date
- OL, HC, GR keys
- ISBN13 and ASIN
- narrator, narration type, abridged
- rating and rating count
- cover URL

Priority model:

```text
English content/description/cover:
  Hardcover -> Goodreads -> Readarr -> OpenLibrary

Foreign content/description/cover:
  Goodreads -> Hardcover -> Readarr -> OpenLibrary

Audio:
  Audnexus -> Hardcover
```

Core protection rules:

- If any provider outcome is `Conflict`, the merge result is `Conflict`.
- Existing non-empty title and author are preserved as identity fields.
- User-owned fields are preserved.
- Manual covers are preserved.
- Background mode merges only merge-safe outcomes.
- Manual and hard refresh modes may merge all non-conflict outcomes.
- If no provider wins a field, current values are retained where possible.

The merge output includes:

- field updates for `works`
- field provenance upserts
- stale provenance deletes
- external ID upserts
- final enrichment status
- enrichment source string

## Database Apply

`SqliteWorkRepository::apply_enrichment_merge` applies the merge atomically.

Current responsibilities:

- read current `merge_generation`
- reject stale writes with `ApplyMergeOutcome::Superseded`
- update enrichable work fields
- set enrichment status and enriched timestamp
- increment `merge_generation`
- upsert field provenance
- delete stale provenance
- upsert external IDs

The update path writes nullable fields as provided by the merge output. This
makes merge correctness important: a `None` in the resolved update can clear a
database field.

## Persistent State

Important tables and files:

- `works`: canonical metadata, status, cover URL, language, provider keys,
  merge generation.
- `provider_retry_state`: per-provider outcome state, next retry, suppression,
  and normalized success payload JSON.
- `work_metadata_provenance`: field-level ownership and provider provenance.
- external ID tables: additional ISBNs, ASINs, and provider identifiers.
- library item rows: tag sync status and tag generation.
- cover cache: `data_dir/covers/{user_id}/{work_id}...`.

## Where Accuracy Is Won Or Lost

### Seed Data Quality

Readarr import starts with rich seed data. Manual import currently starts with
sparse seed data. This creates different odds of successful enrichment even
though both eventually call `WorkService::add`.

Improvement direction:

- pass scanned/manual import cover URLs into `AddWorkRequest`
- preserve ISBN/ASIN parsed from files as source data
- preserve user-selected provider detail URLs as canonical keys
- include file-derived metadata as a low-priority source provider with clear
  provenance

### Candidate Selection

Goodreads search currently depends on LLM disambiguation. If LLM selection is
unavailable or inconclusive, the provider returns no payload even when search
results exist.

Improvement direction:

- add deterministic candidate scoring before LLM selection
- use title, author, year, language, known keys, ISBN, and edition signals
- use the LLM only for ambiguous candidates above a minimum score
- persist candidate diagnostics so failures are explainable

### Provider Payload Completeness

Provider successes can still be incomplete. OpenLibrary currently does not
produce `cover_url` in normalized output. Hardcover and Goodreads can return
payloads without covers depending on match quality and source availability.

Improvement direction:

- separate "identity metadata" from "asset resolution"
- run a dedicated cover resolver after identity is known
- try cover by ISBN, OL key, GR key, HC key, and provider image fields
- track cover quality and dimensions, not just URL presence

### Validation Strictness

LLM validation can reject provider payloads or null selected fields. This
protects identity, but if the anchor data is sparse or stale it can discard
useful metadata.

Improvement direction:

- expose validation rejection reasons in structured logs/UI
- distinguish identity mismatch from field-level uncertainty
- allow cover-only acceptance when identity is already established by another
  provider key
- avoid all-or-nothing rejection when one field conflicts but other fields are
  useful

### Provenance And Protected Fields

User-owned fields, manual covers, and identity fields are intentionally hard to
overwrite. This is correct, but it can make refresh appear ineffective when bad
metadata was previously marked user-owned.

Improvement direction:

- show field provenance in debug/admin views
- provide a "release field ownership" action for selected fields
- make hard refresh semantics explicit: which protections remain and which are
  reset

## Where Speed Is Won Or Lost

### Synchronous Add

Every `WorkService::add` runs enrichment before returning. This improves final
state consistency but makes manual and bulk operations wait on provider latency.

Improvement direction:

- keep synchronous add for single-user clarity, but stream progress events
- make provider time budgets explicit per entry point
- for bulk import, consider "identity gate sync, enrichment continuation async"
  only if UI and retry semantics make partial states obvious

### Provider Fan-Out

The queue already parallelizes providers per work, while respecting provider
concurrency and rate limits. Bulk operations add another layer of concurrency
between works.

Improvement direction:

- prioritize direct-key lookups over fuzzy searches
- skip low-value providers when a high-confidence provider already supplies all
  requested fields, especially in manual flows
- use per-field completeness to decide whether another provider is needed
- cache provider detail responses by stable provider key

### Retry State

Terminal retry state prevents repeated provider calls across restarts. This is
good for stability but can surprise manual re-runs if refresh does not clear the
right rows or if a previous terminal state was based on a transient parser/API
problem.

Improvement direction:

- make manual refresh and hard refresh behavior visibly clear
- record provider version/parser version in retry state
- invalidate retry state when matching logic or parser logic changes
- allow targeted provider retry for one work

### LLM Dependence

LLM calls improve ambiguous matching but add latency and failure modes. The
current Goodreads path can become all-or-nothing around LLM candidate selection.

Improvement direction:

- deterministic scoring first
- LLM only when deterministic confidence is too close to call
- cache LLM decisions keyed by normalized candidate set
- degrade to a safe deterministic winner only above a high confidence threshold

## Current Risk Areas

Manual import is the biggest convergence risk because it does not pass rich
source metadata into add/enrichment.

Cover resolution is coupled to provider field merge. If no provider wins
`cover_url`, cover download never starts even if a cover could be found from an
ISBN or provider key.

Provider retry state is durable and intentionally suppresses repeated work. That
means pipeline changes should have a way to invalidate stale terminal outcomes.

Goodreads direct lookup depends on `gr_key`. If the entry path drops a selected
Goodreads detail URL before `WorkService::add`, the provider falls back to
title/author search and may fail to select the same edition.

The merge apply step can clear nullable fields if the merge output resolves
them to `None`. Merge output should be tested carefully around fallback and
provenance-delete paths.

## Improvement Backlog

Highest leverage accuracy work:

1. Normalize all entry paths to pass the richest available seed data.
2. Add deterministic Goodreads candidate scoring before LLM disambiguation.
3. Split cover resolution into a dedicated post-identity asset resolver.
4. Add structured provider/validator diagnostics for "why no field won".
5. Version provider retry state so metadata pipeline changes can invalidate
   stale terminal outcomes.

Highest leverage speed work:

1. Prefer direct-key provider calls and avoid fuzzy search when possible.
2. Add response caching for provider detail lookups.
3. Add field-completeness short-circuiting for lower-priority providers.
4. Cache LLM candidate decisions.
5. Add progress reporting for synchronous add and bulk import.

## Debugging Checklist

When a work is missing metadata or cover art:

1. Check the entry path and seed data. Did the add request include keys, ISBN,
   cover URL, language, and source provider data?
2. Check `provider_retry_state`. Which providers ran, which were skipped, and
   which payloads were persisted?
3. Check validator decisions. Were success payloads rejected or fields nulled?
4. Check merge priority. Did a higher-priority provider have an empty or stale
   value that affected field resolution?
5. Check field provenance. Was the field user-owned, manually covered, or
   otherwise protected?
6. Check final `works.cover_url`. If it is empty, cover download never had a URL
   to fetch.
7. Check cover cache files under `data_dir/covers/{user_id}`.
8. Check tag sync status if the database metadata is correct but the file tags
   are stale.
