# English Work Lifecycle Refactor Handoff

This document is a standalone implementation handoff for refactoring Livrarr's
English work creation and metadata enrichment lifecycle.

It is based on the May 2026 investigation into missing covers and bad metadata
conflicts after the metadata pipeline rewrite. The immediate symptom was that
popular English books such as `A Darkness at Sethanon`, `Artificial Condition`,
`Blood Rites`, and `Breath of the Dragon` were created without covers because a
bad supplemental Goodreads result caused enrichment conflict instead of being
rejected as a bad Goodreads payload.

The design decision here is intentionally narrow: this document covers English
works only.

## Target Decision

For English works, a correct OpenLibrary work key (`ol_key`) should be the
canonical baseline identity anchor.

All English work creation paths should normalize to the same confirmed work
candidate contract before calling `WorkService::add`. The entry path should not
determine metadata quality. Manual add, manual import, list import, Readarr
import, and author monitor should differ only in how they obtain the candidate
and whether they also attach an existing file.

Goodreads, Hardcover, Readarr source metadata, and Audnexus are supplemental
providers. They may improve display metadata, covers, series, descriptions,
ratings, identifiers, or audio metadata, but they should not be able to poison
an otherwise OL-keyed English work.

## Core Principle

The pipeline should separate identity from presentation.

OpenLibrary is the English identity baseline because it gives Livrarr a stable
work key. OpenLibrary metadata can still be messy for capitalization,
punctuation, edition drift, and display quality, so OL should not automatically
win every display field.

The practical policy is:

- identity comes from the user-confirmed or source-confirmed `ol_key`
- display fields can be improved by better sources
- supplemental provider mismatches are provider-level or field-level rejects
- work-level conflict is reserved for uncertain or contradictory core identity

## Current Problem

The current system has a unified enrichment path, but not all creation paths
enter it with equivalent data.

Some paths already have or can obtain `ol_key`, author key, year, and cover URL,
but drop parts of that data before `WorkService::add`. Other paths have rich
source metadata but do not resolve `ol_key` before creation.

This creates two bad outcomes:

1. English works can enter enrichment without the strongest available identity.
2. A bad supplemental provider result can cause a conflict and prevent good
   fields, especially covers, from being applied.

## Creation Paths

| Path | Workflow role | Current OL status | Target behavior |
|------|---------------|-------------------|-----------------|
| Manual add | User searches and selects a work in the UI. | Uses OpenLibrary search for English works and sends `ol_key` to add. | Keep this as the model path. Preserve all candidate fields. |
| Manual import | User imports an existing local file and selects/accepts a match. | Scan lookup finds OL data, but final import drops cover/year and may re-derive author key separately. | Treat as manual add plus file attach. Carry the selected OL candidate through import unchanged. |
| List import | User confirms rows from imported lists. | Usually resolves OL by ISBN first, then title/author search fallback. | Ensure all confirmed English rows have `ol_key` before add unless explicitly impossible. |
| Readarr import | User imports/syncs existing Readarr-managed books. | Has rich source metadata but does not currently set `ol_key`. | Resolve `ol_key` before add using ISBN first, then title/author fallback. Preserve Readarr metadata as supplemental source data. |
| Author monitor | Livrarr auto-adds works for a monitored author. | Fetches OpenLibrary author works and already has work keys. | Keep using the OL work key as the identity anchor. |
| Refresh/retry | Existing work metadata is rechecked. | Does not create a work. Depends on stored identity. | Reuse stored canonical identity. Repair legacy English works missing `ol_key`. |

## Target Candidate Contract

Introduce or enforce a normalized internal concept like
`EnglishConfirmedWorkCandidate`. The exact type name is not important, but the
contract is.

Required for English work creation:

```rust
struct EnglishConfirmedWorkCandidate {
    title: String,
    author_name: String,
    ol_key: String,
    language: String, // normalized to "en"
    provenance: AddProvenance,
}
```

Recommended fields to carry whenever available:

```rust
struct EnglishConfirmedWorkCandidate {
    title: String,
    author_name: String,
    ol_key: String,
    language: String, // normalized to "en"
    provenance: AddProvenance,

    author_ol_key: Option<String>,
    year: Option<i32>,
    cover_url: Option<String>,
    detail_url: Option<String>,

    isbn: Option<String>,
    asin: Option<String>,
    publisher: Option<String>,
    description: Option<String>,
    genres: Vec<String>,
    page_count: Option<i32>,
    rating: Option<f64>,
    rating_count: Option<i32>,

    series_name: Option<String>,
    series_position: Option<f64>,

    source_provider_data: Option<SourceProviderData>,
}
```

Manual import should have the same candidate plus a file path:

```rust
struct ManualImportSelection {
    candidate: EnglishConfirmedWorkCandidate,
    file_path: PathBuf,
    delete_existing: bool,
}
```

The important point is that the selected match is preserved as data. Do not
throw away `cover_url`, `year`, or `author_ol_key` during the transition from UI
selection to backend import.

## Target Flow

All English creation paths should converge before `WorkService::add`:

```text
entry path
  -> parse or receive user/source input
  -> resolve English candidate with correct ol_key
  -> preserve candidate fields
  -> WorkService::add(candidate as AddWorkRequest)
  -> unified enrichment
  -> supplemental providers fill/improve fields
  -> provider-level validation and merge
  -> cover download/cache
  -> file import/tagging when applicable
```

At the atomic level:

```text
manual add    = confirmed English work candidate
manual import = confirmed English work candidate + existing file
```

Batch workflows should be implemented as repeated atomic operations, not as
weaker alternate metadata flows.

## Current Code Facts To Verify During Implementation

These are the concrete locations from the investigation.

Manual add:

- Frontend: `frontend/src/pages/search/SearchPage.tsx`
- Handler: `crates/livrarr-handlers/src/work.rs`
- Domain request: `crates/livrarr-domain/src/services/work.rs`
- Service: `crates/livrarr-metadata/src/work_service.rs`

Current behavior:

- English search uses OpenLibrary through `WorkService::lookup_filtered`.
- The selected result includes `olKey`, title, author, author OL key, year,
  cover URL, language, and detail URL.
- The handler maps these into domain `AddWorkRequest`.
- It currently sets Goodreads key, series, and source provider data to `None`.

Manual import:

- Frontend: `frontend/src/pages/manual-import/ManualImportPage.tsx`
- Handler: `crates/livrarr-handlers/src/manual_import.rs`
- Scan service: `crates/livrarr-server/src/manual_import_scan_service.rs`

Current behavior:

- Manual import scan searches OpenLibrary and obtains `ol_key`, title, author,
  author key, year, and cover URL.
- The frontend `OlMatch` has a cover URL, but the final `ManualImportItem`
  drops it.
- Backend `find_or_create_work` builds `AddWorkRequest` with title, author,
  optional OL key, and language.
- It passes no cover URL, no year, no detail URL, and no source provider data.
- It may look up author OL key separately instead of preserving the selected
  candidate's author key.

List import:

- `crates/livrarr-metadata/src/list_service.rs`

Current behavior:

- Attempts OpenLibrary lookup by ISBN first.
- Falls back to OpenLibrary title/author/year search.
- Builds `AddWorkRequest` with `ol_key`, title, author, author OL key, year,
  cover URL, and imported provenance when found.

Readarr import:

- `crates/livrarr-server/src/readarr_import_workflow.rs`

Current behavior:

- Extracts rich source metadata from Readarr book/edition data.
- Builds `SourceProviderData`.
- Calls `WorkService::add` with title, author, year, language, source metadata,
  series, monitoring flags, import ID, and cover URL.
- Does not set `ol_key`.

Author monitor:

- `crates/livrarr-metadata/src/author_monitor_workflow.rs`

Current behavior:

- Fetches OpenLibrary author works.
- Each work entry has `/works/OL...W`.
- Adds works with `ol_key` and `author_ol_key`.

Goodreads enrichment:

- Goodreads search fallback currently depends on LLM disambiguation.
- Without a known Goodreads key and without LLM selection, Goodreads search is
  skipped or returns not found.
- With bad LLM selection, junk results can be accepted as provider payloads.
- Goodreads search can also be blocked by WAF responses that look like HTTP 202
  with an empty body; those should not be treated as a successful empty search.

## Proposed Implementation Steps

### 1. Preserve manual import candidate fields

Manual import should stop shrinking the selected OpenLibrary match.

Change the manual import DTO path so the final import request carries at least:

- `ol_key`
- `title`
- `author_name`
- `author_ol_key`
- `year`
- `cover_url`
- `language`

Then map those fields into `AddWorkRequest` in `find_or_create_work`.

Pseudocode:

```rust
fn import_single_item(item):
    candidate = item.selected_ol_candidate

    req = AddWorkRequest {
        title: candidate.title,
        author_name: candidate.author_name,
        ol_key: Some(candidate.ol_key),
        author_ol_key: candidate.author_ol_key,
        year: candidate.year,
        cover_url: candidate.cover_url,
        language: Some("en"),
        provenance_setter: Some(Imported or User),
        ..Default::default()
    }

    work = work_service.add(user_id, req)
    import_service.import_single_file(work.id, item.path, item.delete_existing)
```

### 2. Resolve OpenLibrary identity before Readarr add

Readarr imports should resolve `ol_key` before creating English works.

Preferred resolution order:

1. ISBN lookup: `https://openlibrary.org/isbn/{isbn}.json`
2. Follow the returned `works` link to get `/works/OL...W`.
3. If ISBN lookup fails, use OpenLibrary title+author search.
4. If no confident OL result is found, create with a clear missing-OL state only
   if the workflow explicitly allows that fallback.

Readarr's existing data should remain `SourceProviderData`; it should enrich the
work, not replace OL identity.

Pseudocode:

```rust
fn build_readarr_add_request(book, edition):
    source_data = extract_readarr_source_provider_data(book, edition)

    ol_candidate = if edition.isbn13 exists:
        lookup_openlibrary_by_isbn(edition.isbn13)
    else:
        None

    if ol_candidate is None:
        ol_candidate = lookup_openlibrary_by_title_author(book.title, author.name)

    req = AddWorkRequest {
        title: best_title(source_data, ol_candidate),
        author_name: best_author(source_data, ol_candidate),
        ol_key: ol_candidate.ol_key,
        author_ol_key: ol_candidate.author_ol_key,
        year: ol_candidate.year.or(source_data.year),
        cover_url: source_data.cover_url.or(ol_candidate.cover_url),
        language: normalized_language,
        source_provider_data: Some(source_data),
        provenance_setter: Some(Import),
        ..existing_readarr_fields
    }
```

### 3. Enforce OL-keyed English add semantics

`WorkService::add` should treat `ol_key` as the strongest duplicate and identity
signal for English works.

Recommended behavior:

- If `language == en` and `ol_key` is present, first look for an existing work
  by `ol_key`.
- Then fall back to normalized title+author matching.
- If a normalized match exists but has no `ol_key`, attach the incoming `ol_key`
  if the match is otherwise compatible.
- If a normalized match has a different `ol_key`, do not silently merge. That is
  a real identity conflict.

Pseudocode:

```rust
fn add(user_id, req):
    normalized = normalize(req.title, req.author_name)

    if is_english(req.language) and req.ol_key exists:
        existing = db.find_work_by_ol_key(user_id, req.ol_key)
        if existing:
            maybe_enrich_existing(existing, req.source_provider_data)
            return existing

    existing = db.find_by_normalized_match(user_id, normalized)

    if existing:
        if is_english(req.language) and req.ol_key exists:
            if existing.ol_key is None:
                db.set_ol_key(existing.id, req.ol_key)
            else if existing.ol_key != req.ol_key:
                return identity_conflict

        maybe_enrich_existing(existing, req.source_provider_data)
        return existing

    create_work(req)
    run_unified_enrichment(...)
```

### 4. Change enrichment conflict semantics

An OL-keyed English work should not become work-level conflict merely because
Goodreads or another supplemental provider returns junk.

Provider validation should produce one of these outcomes:

- accepted provider payload
- rejected provider payload
- retryable provider failure
- work-level identity conflict

For English works with a trusted `ol_key`, a bad Goodreads result should be
`rejected provider payload`, not `work-level identity conflict`.

Pseudocode:

```rust
fn validate_provider_payload(work, provider, payload):
    if work.language == "en" and work.ol_key exists:
        if provider is supplemental:
            if payload strongly contradicts title/author/ol identity:
                return RejectProviderPayload(reason)
            return AcceptProviderPayload

    if core identity is uncertain and provider contradiction is severe:
        return WorkIdentityConflict

    return AcceptProviderPayload
```

Merge should continue with accepted providers:

```rust
fn merge(work, provider_payloads):
    accepted = payloads.filter(status == accepted)
    rejected = payloads.filter(status == rejected)

    field_winners = choose_field_winners(work, accepted)
    apply_merge(field_winners)
    persist_rejected_provider_diagnostics(rejected)
```

### 5. Fix Goodreads anti-bot handling

Goodreads search should not treat WAF challenge responses as successful empty
searches.

If Goodreads returns HTTP 202 with an empty body or WAF challenge headers, mark
the provider result as retryable anti-bot block, not success/not-found.

Pseudocode:

```rust
fn fetch_goodreads_search(query):
    response = http.get(search_url)

    if response.status == 202 and response.body is empty:
        return RetryableProviderFailure(AntiBotBlock)

    if response.headers["x-amzn-waf-action"] == "challenge":
        return RetryableProviderFailure(AntiBotBlock)

    if response.status is not 2xx:
        return RetryableProviderFailure(HttpStatus(response.status))

    return parse_search_results(response.body)
```

This is separate from the OL-key refactor, but it explains why Goodreads looked
like it had no result during the incident.

### 6. Repair legacy English works

Existing English works created before this refactor may be missing `ol_key` or
stuck in conflict because of a bad supplemental provider payload.

Add a repair path or migration job that:

- finds English works with missing `ol_key`
- resolves OL by ISBN when possible
- falls back to title+author OpenLibrary search
- stores the resolved `ol_key`, `author_ol_key`, year, and cover URL when safe
- clears bad conflict state where only supplemental providers were rejected
- reruns enrichment

Do not blindly rewrite user-defined title, author, or cover fields.

## Field Priority Policy

The exact merge engine implementation can vary, but the intended priority is:

### Identity

Priority:

1. User-confirmed selected candidate
2. OpenLibrary `ol_key`
3. Existing stored canonical identity

Supplemental providers should not override `ol_key`.

### Display Title And Author

Priority:

1. User-defined display fields
2. Selected candidate/source display text
3. Hardcover
4. Goodreads
5. OpenLibrary

Reason: OL is good for identity, but can be rough for capitalization,
punctuation, and presentation.

### Cover

Priority should be quality-aware, not purely provider-order based.

Suggested order:

1. User/manual cover
2. Existing high-quality local cached cover
3. Source-selected cover if it came from the confirmed candidate
4. Hardcover/OpenLibrary/Goodreads by validation and quality

Do not clear a good cover because a supplemental provider failed.

### Description, Genres, Publisher, Page Count, Ratings

Suggested order:

1. User-defined fields
2. Source import data when explicitly trusted for that field
3. Hardcover
4. Goodreads
5. OpenLibrary

### Audio Metadata

Priority:

1. User-defined fields
2. Audnexus
3. Source import data
4. Hardcover/Goodreads where applicable

## Open Questions For Implementation

These are the decisions Claude Code should either implement conservatively or
surface before changing behavior:

1. Should `ol_key` be required for all English creation paths, or only required
   when it is obtainable with high confidence?
2. What confidence threshold should OpenLibrary title+author fallback use when
   ISBN is absent?
3. Should Readarr import block creation when OL resolution fails, or create a
   work in a `missing_identity` state?
4. Where should rejected provider payload diagnostics be stored so users can see
   why Goodreads/Hardcover was ignored?
5. How should existing user-edited fields be detected and protected if the DB
   does not currently track per-field user ownership?

## Acceptance Criteria

The refactor is successful when these are true:

- Manual add and manual import produce equivalent `AddWorkRequest` metadata for
  the same selected English work, except manual import also carries file import
  information.
- Readarr English imports resolve and store `ol_key` before or during work
  creation.
- Author monitor and list import continue to pass OL identity through add.
- Refresh/retry reuse stored canonical identity instead of rediscovering from
  weaker provider data.
- A bad Goodreads result is rejected as a Goodreads payload and does not put an
  OL-keyed English work into work-level conflict.
- A good cover from OL, Hardcover, source data, or an existing cache is not lost
  because another provider failed validation.
- Existing affected popular works can be repaired by resolving/storing `ol_key`
  and rerunning enrichment.

## Suggested Test Cases

Add behavior tests around these scenarios:

1. Manual add and manual import of the same OL-selected English work build
   equivalent work identity fields.
2. Manual import preserves selected `cover_url`, `year`, and `author_ol_key`.
3. Readarr import with ISBN resolves `/isbn/{isbn}.json` to a work key and
   stores `ol_key`.
4. Readarr import without ISBN uses title+author fallback and rejects low-score
   OL candidates.
5. Existing work with matching normalized title/author and no `ol_key` gets the
   incoming compatible `ol_key`.
6. Existing work with different stored `ol_key` returns identity conflict.
7. Goodreads junk result for an OL-keyed English work is rejected without
   blocking cover merge from OL/Hardcover/source data.
8. Goodreads WAF 202 empty response is recorded as retryable anti-bot block, not
   success/not-found.
9. User-defined cover/title/author are not overwritten by provider refresh.

## Implementation Order

Recommended order:

1. Add tests for manual import candidate preservation.
2. Update manual import DTOs and backend mapping.
3. Add tests for Readarr OL resolution.
4. Implement Readarr ISBN/title-author OL resolution before add.
5. Add tests for OL-key duplicate and conflict behavior in `WorkService::add`.
6. Implement OL-key-first add semantics for English works.
7. Add tests for supplemental provider rejection versus work-level conflict.
8. Update enrichment validation/merge conflict semantics.
9. Fix Goodreads WAF retry classification.
10. Add a repair job or admin command for legacy English works missing `ol_key`.

This order fixes the data loss at entry points before changing the broader
merge behavior, then addresses the specific Goodreads failure mode.
