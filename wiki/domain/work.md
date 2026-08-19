# Work

The primary entity in Livrarr (Principle 1: work-first, not author-first). A Work represents a title — independent of format, edition, or packaging.

## Lifecycle

1. **Created** — via search+add, RSS sync auto-add, or author monitor detection
2. **Identity settled** — deterministically. `settle_identity` is the one identity authority
   (`crates/livrarr-identity/src/async_resolver.rs:125-284`) and the add path calls it
   (`crates/livrarr-metadata/src/work_service.rs:1152`); anchors auto-merge only when the FLM
   gate's title-and-author check passes (`async_resolver.rs:318-353`). **No LLM confirms the
   match.** The only LLM identity-verify function in the tree
   (`async_resolver.rs:46-97`) is background-only, returns immediately whenever a work anchor
   is already present (`:52-58`), and Serena reference-tracking finds no caller for it.
3. **Enriching** — metadata enrichment runs (Background/Manual/HardRefresh)
4. **Enriched** — all available metadata populated
5. **Monitored** (optional) — RSS sync watches for matching releases

### Terminal states — and which enum they live on

The two conditions people look for here — retry exhaustion and Conflict — are **not**
`EnrichmentStatus` values. That enum has exactly four — `Unenriched`,
`Enriched`, `Thin`, `Failed` (`crates/livrarr-domain/src/entities.rs:83-102`). The identity
outcomes that used to sit alongside them were dropped in migration 055 and now live only on
`IdentityStatus` (`:97-101`).

- **Retry exhaustion is per provider, not per work.** A provider whose attempts reach
  `max_attempts` converts to `PermanentFailure { RetryBudgetExhausted }`
  (`crates/livrarr-enrichment/src/provider_queue.rs:586-590`); production sets that to **5**,
  not 3 (`crates/livrarr-server/src/main.rs:833-836`). The work's own status is not changed by
  it.
- **Conflict is an identity state, and no LLM is involved.** `IdentityStatus::Conflict` means
  an identity contradiction is open — a differing confirmed anchor — and is terminal until the
  user resolves it (`crates/livrarr-domain/src/entities.rs:122-124`). It is one of three
  terminal identity states, with `NeedsReview` and `NotFound` (`:125-133`); all three make
  `settle_identity` a no-op (`crates/livrarr-identity/src/async_resolver.rs:145-158`).

## Key Fields

- Title, original title, description, publication date
- ISBN, ASIN, Hardcover key, OpenLibrary key, GoodReads key
- Per-media-type monitoring flags (`monitor_ebook`, `monitor_audiobook`). A Work carries no
  media-type column of its own (`crates/livrarr-domain/src/entities.rs:376-428`) — media type
  lives on the LibraryItem (`:479`) and the Grab (`:588`)
- Enrichment status, identity status, enriched_at (`entities.rs:406-411`)
- merge_generation (CAS guard for atomic enrichment merge)

## Provenance

Every enrichable field has per-field provenance tracking:
- Which provider set it — one of eight, not four: Hardcover, OpenLibrary, Goodreads, Audnexus,
  Llm, Readarr, GoogleBooks, Audible (`crates/livrarr-domain/src/enrichment_types.rs:13-24`)
- Who set it — one of six setters, not three: `Provider`, `User`, `System`, `AutoAdded`,
  `Imported`, `Import` (`enrichment_types.rs:176-198`). `AutoAdded` (author-monitor or series
  auto-add) is deliberately **not** treated as a user lock anchor (`:187-192`)
- User-owned provenance survives manual refresh

## Presentation when a related Author is gone

A Work remains the primary entity even when its related Author row is deleted. The Work-detail door must still render the user-scoped Work from data that survives on the Work row: its stored `author_name`, stored cover state, files, and other metadata. Identity siblings may be empty when no primary Author can be resolved; presentation must not invent one.

This is deliberately different from identity authority. `WorkIdentityRepository::read_captured_identity` is a coherent settlement/decision read and remains fail-closed when the primary Author invariant is unavailable. The HTTP Work-detail handler degrades only that read's `NotFound` after the Work service has already proved the scoped Work exists. The refresh door likewise returns the completed refresh and skips the optional captured-route settlement follow-up when no coherent identity can be read. Other storage errors still propagate. This keeps user-facing presentation available without weakening identity decisions.

## Monitoring

Per-media-type monitoring (not a single boolean):
- `monitor_ebook` — watch for ebook releases via RSS sync
- `monitor_audiobook` — watch for audiobook releases via RSS sync

These are independent. A work can be monitored for ebook only, audiobook only, or both. RSS filter checks release categories (7020 = ebook, 3030 = audiobook) against the corresponding flag.

Series monitoring sets these flags on member works. Unmonitoring a series clears them.

## Semantics

- "Missing" (no file on disk) is NOT the same as "wanted" (monitored for download). Don't conflate these.
- A Work can have both ebook and audiobook releases simultaneously.
- Works belong to Authors (many-to-one primary, with additional authors possible).
- Works belong to exactly one collection (root folder). **Scoping, though, is by `user_id`, not by collection:** every work row carries one (`crates/livrarr-domain/src/entities.rs:378`) and reads are fenced by it — one user cannot fetch or list another's work (`crates/livrarr-db/src/cross_user_isolation_tests.rs:376`, `:388`). The `Work` struct itself carries no root-folder reference (`entities.rs:376-428`); the root folder is on the LibraryItem (`:477`).

## Dedup review

A direct add that matches an established broad identity group does not create a provisional second
Work merely because the incoming volume or route needs human review. The identity settlement claims
the established Work, leaves its current tuple/routes intact, and parks one `GroupIdentity` proposal
against it. Accepting that single-Work proposal applies the proposed tuple and routes to the existing
Work. Repeating the same capture reuses the equivalent pending card; it does not grow the review
queue or create another Work.
