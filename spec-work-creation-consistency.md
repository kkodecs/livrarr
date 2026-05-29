---
feature: "work-creation-consistency"
stage: spec
status: draft
version: 4
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018, REQ-019, REQ-020, REQ-021, REQ-022, REQ-023, REQ-024, REQ-025, REQ-026, REQ-027, REQ-028, REQ-029]
related_issues: [97]
---

# Spec: work-creation-consistency

## 0a. Design Principles

Choices we're committing to. If a requirement conflicts, the principle wins.

1. **Harvest first.** Every work-creation path captures every deterministic identifier its source already carries *before* performing any external title/author search. We never reduce a hard, self-provenanced identifier to a fuzzy search string.
2. **Resolve deterministically.** A resolving hard identifier (ISBN, ASIN, or a native key) determines identity without a fuzzy search, and resolves across *all* relevant providers — never a single hardcoded provider.
3. **Confirm only the residue.** User confirmation of a match is required only when the source lacks a resolving identifier. Human-in-the-loop is for what the machine can't resolve, not a tax on every item.
4. **Federated identity.** A Work's identity is the union of its provider anchors (`ol_key`, `gr_key`, `hc_key`) plus edition bridges (`isbn_13`, `asin`). Identifiers are first-class and portable; capturing them is the goal, discarding them is the defect.
5. **Maximize identifiers within the latency budget.** Capture the most identifiers possible for each path's tier, and never re-query a provider for data already in hand.
6. **Provenance order is absolute.** User-set values win; a populated value is never overwritten by a null/empty; lower-provenance guesses never override higher-provenance facts.
7. **Degrade without an LLM.** Identity resolution and field merge must function deterministically with no LLM configured. LLM assistance is an enhancement, never a dependency.

## 0b. System Truths

Facts about external providers we don't control. (Confidence: High = verified in code + provider behavior this cycle.)

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Goodreads (site/scrape) | GR has **no deterministic** identifier lookup. A `gr_key` is obtained deterministically only by harvesting it at the source; otherwise via a fuzzy scrape-search (`isbn:` query *or* title+author) that is **LLM-disambiguated** and subject to anti-bot failure (verified: `provider_client.rs:766-807`). With no LLM, GR is reachable only by a directly-harvested `gr_key`. | Assuming a deterministic ISBN→GR lookup; trusting any search/LLM-derived `gr_key` without verification (REQ-024). | High (verified) |
| ST-002 | Google Books API | Resolves an ISBN to volume(s); provides rich English metadata; queryable by `q=isbn:`. Has **no** canonical work identifier (volumes are edition-scoped). | Treating a GB volume id as a work anchor; relying on GB for work-level identity. | High |
| ST-003 | OpenLibrary API | A valid ISBN resolves to an OL work key (`/isbn/{isbn}.json`) when OL holds it. | Assuming every ISBN resolves — OL has coverage gaps (root of #97). | High |
| ST-004 | Hardcover API | Resolves by ISBN and exposes `hc_key`; supports a canonical-id for provider-side merges. | Assuming HC holds every ISBN. | High |
| ST-005 | Audnexus + Audible APIs | Audiobook identity is **ASIN**-keyed; both are queryable by ASIN (Audible also by title/author). | Bridging audiobooks via ISBN; expecting an ISBN lookup on either. | High |
| ST-006 | Amazon identifier scheme | A print-book ASIN historically **equals the ISBN-10**; Kindle/Audible ASINs are `B`-prefixed. | Storing an ISBN-10-shaped value as an audiobook `asin`. | Medium-High (~90%) |
| ST-007 | EPUB/M4B file formats | EPUB OPF declares `dc:identifier` (often ISBN), language, and series; M4B tags carry ASIN and narrator. The file frequently declares its own hard identity. | Assuming embedded metadata is always present *or* always correct (pirated/hand-edited files exist). | High (present) / Medium (correct) |
| ST-008 | Deployment config | The system may run with **no LLM** configured. | Requiring an LLM for identity resolution or field merge. | High (project rule) |
| ST-009 | Google Books API | GB returns results only with a configured API key (keyless quota is zero). | Relying on keyless GB access. | High |

## 1. Problem Statement

Livrarr creates Works through six paths — Add Work, manual import, list import, Readarr import, series monitor, author monitor. Each uses a *different*, lossy mechanism, and across all of them the system **ignores the best-provenanced data it already has** in favor of weaker guesses:

- **Identifiers the source hands over are discarded.** Manual import never extracts the EPUB's embedded ISBN; list import drops the Goodreads Book Id from the CSV; Readarr import drops the Goodreads work id (`foreign_book_id`) whenever Readarr supplies it. `hc_key` cannot be written at creation by any path at all.
- **Discovery is single-provider or absent.** Manual import and list import resolve against OpenLibrary only; Readarr does no discovery and creates identity-pending works. A book absent from OpenLibrary — even one whose file declares a valid ISBN that Hardcover or Google Books would match instantly — produces **zero candidates and cannot be imported** (issue #97).
- **Users are asked to confirm matches the system could resolve itself.** Manual import forces the user to pick from OpenLibrary fuzzy guesses for every file, when the file's embedded ISBN deterministically identifies the work.
- **Work is done twice.** When discovery does retrieve provider metadata, it is thrown away and re-fetched by enrichment moments later.

The result: works are created with impoverished, inconsistent identity; enrichment re-queries for data already in hand; and the import experience is high-friction and incomplete. This feature makes all six paths **harvest the identifiers the source already carries, resolve them deterministically across all providers, and ask the user only for the unresolved residue** — maximizing captured identifiers at the fastest response per path.

## 2. Requirements

### Identity model & normalization

- **REQ-001**: When a creation path provides any of `ol_key`, `gr_key`, `hc_key`, `isbn_13`, `asin`, the created Work MUST persist each provided value at creation time.
- **REQ-002**: `gr_key` MUST be persisted in canonical bare-numeric form (the leading numeric segment) regardless of the form in which the source supplies it (e.g. `"12345.Some_Slug"` → `"12345"`). Existing drifted `gr_key` rows MUST be normalized by migration. This governs the *persisted identity key* (for dedup); it does NOT constrain the URL the scraper fetches — the scraper MAY retain or cache a slug-form URL to avoid redirect round-trips to Goodreads (an anti-bot cost on an already rate-limited source).
- **REQ-003**: `hc_key` MUST be writable through every creation path, not only through post-creation enrichment.
- **REQ-004**: A harvested `asin` whose form matches an ISBN-10 (`^\d{9}[\dX]$`) **and passes ISBN-10 checksum validation** MUST be treated as an ISBN (converted to ISBN-13, stored as `isbn_13`) and MUST NOT be stored as `asin`. An `asin` matching the shape but **failing** the checksum is a genuine Amazon ASIN and MUST be retained as `asin` (so audiobook lookups still work).
- **REQ-005**: Language values MUST be normalized to a bare ISO 639-1 code with any region/locale subtag stripped (e.g. `en-US` → `en`, `pt-BR` → `pt`), on every ingestion path including file-embedded metadata. A single normalization routine MUST be the authority for all paths, and it — together with the ISBN-10→ISBN-13 conversion (REQ-004) and `gr_key` normalization (REQ-002) — MUST reside in the foundation crate (`livrarr-domain`) so every path and crate can invoke it without violating the workspace dependency rules.

### Harvest

- **REQ-006**: Every work-creation path MUST capture every deterministic identifier its source carries *before* issuing any external title/author search:
  - Manual import → ISBN/ASIN (and language, series) embedded in the file's metadata.
  - List import → the ISBN and the Goodreads Book Id present in the CSV row.
  - Readarr import → ISBN and ASIN when present on the selected Readarr edition (both optional); `foreign_book_id` (as `gr_key`) when Readarr supplies it.
  - Add Work → all identifiers returned by every provider consulted for the user-selected result.
  - Series / author monitor → seed the native anchor their source supplies (`gr_key` / `ol_key`) at create; the shared asynchronous resolver (REQ-022) then converges the Work to the full anchor set. Monitors (background tier) do NOT perform synchronous multi-provider discovery at create.
- **REQ-007**: When a flow consults providers and obtains metadata for a work that is about to be created (e.g. the Add Work cover/identity step), it MUST retain the identifiers and metadata returned, not just the cover image.

### Discovery & resolution

- **REQ-008**: Work discovery MUST query the providers *relevant to the seed identifier and latency tier*, never a single hardcoded provider (the #97 defect). The relevant set:
  - **ISBN seed** → OpenLibrary, Hardcover, Google Books (+ Goodreads when an LLM is available), subject to language routing (REQ-027).
  - **ASIN seed** → Audible (Audnexus is background-only per REQ-021).
  - **Native key** (`ol_key`/`gr_key`) → the owning provider directly, then bridge via any resolved ISBN.
  - **Title/author only** → the enabled English providers (or GB/GR for foreign — REQ-027).
  A provider lacking its prerequisite (no Google Books API key per ST-009; no LLM for Goodreads per ST-001; Audnexus on an interactive path per REQ-021) MUST be excluded from that discovery. The set MUST NOT be narrowed to a single provider for a multi-provider-eligible seed.
- **REQ-009**: A resolving ISBN MUST be usable as a cross-provider bridge, with providers classified by role: it resolves **work anchors** from OpenLibrary (`ol_key`) and Hardcover (`hc_key`); it resolves **edition metadata only** from Google Books (which has no work anchor — ST-002 — and MUST NOT contribute a persisted work anchor); and, only when an LLM is available, a **verified** `gr_key` from Goodreads (ST-001; verification per REQ-024). A resolving ASIN MUST bridge to the audiobook-axis providers (Audible, then Audnexus). No identifier resolves a `gr_key` deterministically (ST-001). The bridge is subject to language routing (REQ-027).
- **REQ-010**: When a source provides a resolving hard identifier, the system MUST resolve work identity from it without requiring a fuzzy title/author search.
- **REQ-022**: Identical seed identifiers MUST **eventually converge** to the same resolved identity and captured identifier set regardless of which creation path produced the Work. Convergence is *eventual* — measured after asynchronous enrichment completes, not necessarily at the moment of creation: the synchronous create result includes only the identifiers resolvable within that path's latency tier (REQ-023); the remainder converge in the background. (The six paths differ only in seed and latency tier, not in how identity is ultimately resolved.)
- **REQ-023**: Discovery depth MUST scale to the path's latency tier without changing the identity result: interactive paths (Add Work, manual-import per-file review) MUST return results without blocking on background-only providers; bulk paths may take per-item seconds; background paths (monitors) are unbounded.

### Confirmation

- **REQ-011**: User confirmation of a match MUST be required only when the source lacks a *resolving* identifier:
  - **Tier A** — a resolving ISBN/ASIN exists → auto-match, **no confirmation prompt**. An ISBN that resolves *only* to anchorless providers (e.g. Google Books — no work anchor) still qualifies as Tier A: the Work is created with the ISBN as its identity (no work anchor) and converges to an anchor later if one appears (REQ-022) — it is NOT left identity-pending.
  - **Tier B** — only embedded/parsed title+author exists (no resolving hard id) → confirmation required, with candidates drawn from **all enabled providers**.
  - **Tier C** — no usable identifier → fuzzy search + confirmation.
- **REQ-012**: A Tier-A auto-match MUST be visible to the user and reversible in a single action (the system does not silently commit a match the user cannot see or override).
- **REQ-013**: In **interactive review paths** (Add Work, manual import), a hard identifier that does not resolve to any work MUST fall through to the confirmation path (Tier B/C); it MUST NOT block or error. (Non-interactive paths follow REQ-026 — create identity-pending, do not fabricate.)

### Enrichment efficiency

- **REQ-014**: When discovery has already retrieved a provider's metadata for a work being created, enrichment MUST NOT re-issue a network query to that same provider for that same data.
- **REQ-015**: A newly created Work MUST be returned to the user already populated with every field derivable from data already in hand (no visible fill-in delay for already-retrieved data). Only work that genuinely requires additional network calls (audiobook narrator/duration via Audnexus, cover-image download) MAY complete in the background.

### Merge & conflict

- **REQ-016**: When multiple providers return a value for the same field, the winning value MUST be chosen deterministically by a configurable per-field provider priority. User-set values MUST always win; a null/empty value MUST never override a populated value.
- **REQ-017**: Field merge MUST produce a correct result with no LLM configured. When an LLM is available it MAY refine field selection and validate identity, but MUST NOT be required for either.
- **REQ-018**: When providers disagree on work identity, resolution MUST be by quorum. **Agreement** means two providers return the *same work anchor* (matching `ol_key`/`gr_key`/`hc_key`) or the *same normalized title+author* — NOT merely that they were queried with the same seed identifier (two providers queried by one ISBN that return *different* works are in **conflict**, not agreement). A majority agreement wins and dissenting providers are dropped; a genuine tie with no majority blocks and is surfaced to the user. Cross-axis works (ebook ISBN-keyed + audiobook ASIN-keyed, which never share an identifier) reconcile deterministically by normalized title+author. Agreement by anchor and by normalized title+author MUST both be evaluated deterministically (no LLM); an LLM is consulted only to adjudicate ambiguity the deterministic rules cannot resolve. A provider that carries no work anchor (e.g. Google Books — ST-002) and was queried by a shared ISBN **corroborates that edition** (same physical ISBN) and contributes its metadata to the merge; a title variation alone (e.g. "(Illustrated Edition)") MUST NOT make it a dissenter. However, an anchorless provider whose returned title+author diverges from the consensus *beyond a similarity threshold* (a genuinely different work, not an edition variant) DOES dissent and its metadata is rejected — edition variance corroborates, a different work does not. Conversely, when two providers agree on normalized title+author but return *conflicting same-type anchors* (e.g. two different `ol_key`s), that anchor conflict is **terminal**: it MUST be raised as a Conflict (REQ-020), overriding the title+author agreement — the anchors MUST NOT be silently merged into one Work.
- **REQ-019**: In the Add Work flow the user's explicit selection MUST count as the strongest identity vote. A provider majority MUST NOT override the user's pick.
- **REQ-020**: When a created or updated Work shares an existing Work's normalized title+author but carries a conflicting anchor of **any** federated type — a different `ol_key`, `gr_key`, *or* `hc_key` — the system MUST raise an identity conflict as observable state, not merely log a warning. The conflict store MUST accept all federated anchor types (today it models only OL-key conflicts — `migration 040`). (Closes the currently-unwired conflict path.)

### Provider role

- **REQ-021**: Audnexus MUST NOT be queried during interactive cover/identity discovery. It MUST run in the background, keyed on a resolved ASIN.

### Trust & robustness

- **REQ-024**: A Goodreads id obtained by any means other than direct harvesting from the source — i.e. via a scrape-search (`isbn:` or title+author) with LLM disambiguation — MUST be verified before it is persisted as a Work's `gr_key`, by fetching its detail page and confirming the title+author match. If verification fails, or the fetch is anti-bot blocked, the id MUST NOT be persisted. (Per ST-001 and the known low exact-match rate of LLM GR-id resolution.)
- **REQ-025**: A provider that times out or is unreachable during resolution MUST be treated as an **abstention**: it neither blocks work creation nor counts as disagreement in the quorum (REQ-018). Identity MAY converge further if that provider recovers on a later pass (REQ-022). A partial provider failure MUST NOT corrupt or downgrade already-resolved identity. A timeout produces a *transient* unresolved state that auto-retries on a later pass (converging per REQ-022) — distinct from a *terminal* identity Conflict (REQ-018/020), which pauses automatic enrichment and requires user resolution. Once a user has manually resolved or interacted with a conflicted Work, automatic background passes MUST NOT silently change its identity. (Retry/backoff mechanics are design-stage.)
- **REQ-026**: Per-item user confirmation (REQ-011 Tiers B/C) applies only to interactive review paths (Add Work, manual import). For paths without a per-item review step (list import, Readarr import, series/author monitors), an item whose identity cannot be resolved at create MUST be created in an identity-pending state and converge via the asynchronous resolver (REQ-022) — it MUST NOT block the batch and MUST NOT be committed with a fabricated identity. Unresolved items in interactive paths MUST be surfaced for user attention (not silently dropped or auto-committed with a guess). If the asynchronous resolver cannot reach a high-confidence deterministic match for an identity-pending item (e.g. a Tier-B item with no resolving hard identifier), the item MUST transition to a **needs-review** state surfaced for interactive user resolution — it MUST NOT remain in an indefinite background-retry loop or be silently orphaned. Persistence/UI mechanics are design-stage.
- **REQ-027**: Discovery and merge MUST be language-aware. For a Work whose identified language is non-English, OpenLibrary and Hardcover MUST NOT contribute metadata to enrichment (English-language metadata leaking into a foreign record is a known corruption — project invariant, wiki insights 12/16); foreign-language metadata routes to Google Books and Goodreads. A provider result whose language is incompatible with the Work's identified language MUST NOT win a field merge (REQ-016). English or unresolved-language works use the full ISBN-axis provider set. (Capturing an *anchor* is distinct from contributing *metadata*; the prohibition is on foreign-language metadata contribution, not anchor capture.)
- **REQ-028**: When a creation attempt is matched to an existing Work — by anchor match, normalized title+author dedup, the adopt path, or a race-loser (`work_service.rs::handle_race_loser`) — any hard identifier the incoming candidate carries that the existing Work lacks MUST be merged onto the existing Work. No match path may silently discard a harvested anchor (it would break convergence, REQ-022).
- **REQ-029**: Every identifier (`isbn_13`, `asin`, `ol_key`, `gr_key`, `hc_key`) MUST be validated against its canonical form before it is persisted or used in a provider query. A malformed, empty, or partial identifier MUST be treated as absent (the Work proceeds without it) — never persisted or sent to a provider. ISBN validation includes length + checksum, not merely stripping non-alphanumerics.

## 3. UI/Interface Design

Minimal UI surface; most change is behavioral. The one new affordance:

- **Tier-A auto-match display (REQ-011, REQ-012):** in manual import, a file resolved by its embedded identifier shows as already-matched (labelled as matched from the file), with a one-click control to reject the auto-match and search instead. The bulk scan view distinguishes "auto-matched" rows from "needs attention" rows (those with no resolving identifier).
- No change to the Add Work search → select → cover flow's *shape*; the change is that the selected result carries its full harvested identity into creation.

UI mockups deferred to the design stage if the auto-match/needs-attention split needs visual definition.

## 4. Non-Requirements

Explicit scope exclusions:

- **Multi-valued identifiers.** A Work stores at most one of each identifier type (one ISBN, one ASIN, etc.). Storing the full set of an edition's ISBNs/ASINs is out of scope.
- **`gb_volume_id` persistence.** Dropped deliberately — redundant with the ISBN we already store (GB is re-queryable by ISBN; a volume id adds unique value only for ISBN-less volumes, which can't bridge anywhere regardless).
- **Changing the provider set.** No new providers; no removal of existing ones (Audnexus is *repositioned*, not removed).
- **Re-architecting enrichment internals.** The retry, CAS-merge, and circuit-breaker machinery is reused as-is; this feature changes what *feeds* it, not how it merges/persists.
- **Author identity.** Author-side identifier harvesting (e.g. Readarr `foreign_author_id`) is noted but out of scope for this work-creation feature.
- **UI redesign** beyond the auto-match / needs-attention affordance.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | In Add Work, does LLM identity-validation run synchronously (a gate before the work is shown) or in the background (a verify that can later flag Conflict)? PO leaning background + conditional (only when harvested anchors disagree). | open | |
| Q-002 | Tier B (embedded title+author, no hard id): always confirm, or auto-match when *all* enabled providers unanimously agree on one work? | resolved | Always confirm — title+author is a guess, not a resolving identifier; no auto-match in Tier B (REQ-011). |
| Q-003 | Does the `gr_key` backfill migration (REQ-002) ship with this feature or as a separate data migration? | open | |
| Q-004 | Provenance granularity for harvested fields — preserve per-provider provenance through the harvest, or accept a coarser single-source tag? (Bears on REQ-016 provenance fidelity.) | open | (design-stage) |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-008): A book present on Hardcover but absent from OpenLibrary, imported from a file carrying its ISBN, yields a match candidate (sourced from Hardcover) — not zero candidates. *(The #97 regression test.)*
- [ ] **AC-002** (REQ-001, REQ-003): A Work created via Readarr import for a book whose Readarr record has `foreign_book_id`, ISBN, and ASIN persists a `gr_key`, `isbn_13`, and `asin` — and `hc_key` is populated once HC resolves the ISBN.
- [ ] **AC-003** (REQ-002): A `gr_key` ingested as `"12345.The_Title"` is persisted as `"12345"`; the backfill migration rewrites pre-existing slug/hyphen `gr_key` rows to bare numeric.
- [ ] **AC-004** (REQ-004): An identifier ingested as `"0439139600"` (ISBN-10 shape) in an `asin` field is stored as `isbn_13` (converted to 13) and the Work's `asin` is empty.
- [ ] **AC-005** (REQ-005): An embedded language value of `"en-US"` is persisted as `"en"`; `"pt-BR"` as `"pt"`.
- [ ] **AC-006** (REQ-006, manual): A scanned EPUB whose OPF declares an ISBN produces a harvested ISBN on the candidate before any title/author search is issued.
- [ ] **AC-007** (REQ-006, list): A Goodreads CSV row with a Book Id and ISBN produces a Work carrying the corresponding `gr_key` and `isbn_13`.
- [ ] **AC-008** (REQ-011 Tier A, REQ-012): A manual-import file whose embedded ISBN resolves is presented as auto-matched with no confirmation prompt, and exposes a single-action override.
- [ ] **AC-009** (REQ-013): A manual-import file whose embedded ISBN resolves to no work falls through to the search+confirm flow without error and without creating an identity-less work.
- [ ] **AC-010** (REQ-014): Creating a Work via Add Work after the discovery harvest issues zero additional provider network calls for fields already retrieved during discovery (verified by provider call count).
- [ ] **AC-011** (REQ-015): Immediately after an Add Work creation returns, the Work exposes all fields derivable from already-harvested data; only narrator/duration and the downloaded cover image may populate afterward.
- [ ] **AC-012** (REQ-016, REQ-017): With no LLM configured, two providers returning different descriptions resolve to the higher-priority provider's value; a provider returning null for a field never clears an existing populated value.
- [ ] **AC-013** (REQ-018): Given three providers where two return a *matching work anchor* (or matching normalized title+author) and one differs, the Work is created from the two-provider agreement and the outlier is dropped with no Conflict. Given a 1-vs-1 split with no majority, the Work is flagged Conflict.
- [ ] **AC-014** (REQ-019): In Add Work, when the user's selected result disagrees with a provider majority, the persisted identity is the user's selection.
- [ ] **AC-015** (REQ-020): Creating a Work with the same normalized title+author as an existing Work but a different `ol_key` produces an observable identity-conflict state (not only a log line).
- [ ] **AC-016** (REQ-021): An interactive add's cover/identity discovery issues no Audnexus request; an Audnexus request is issued only in the background after an ASIN is resolved.
- [ ] **AC-017** (REQ-022): The same source identifiers fed through two different creation paths produce Works that **eventually converge** (after enrichment completes) to identical resolved anchors and captured identifier sets — measured post-enrichment, not at create.
- [ ] **AC-018** (REQ-023): An interactive Add Work search returns candidates without waiting on background-only providers; a slow/blocked provider does not stall the interactive response.
- [ ] **AC-019** (REQ-009, ST-002): A Work resolved by ISBN persists `ol_key` (from OL) and `hc_key` (from HC) as anchors; no Google Books volume id is persisted as a work anchor.
- [ ] **AC-020** (REQ-018): Two providers queried with the same ISBN that return *different* works (different returned anchors / title+author) produce a Conflict, not a merge; three providers where two return a matching anchor and one differs create the Work from the two-agreement and drop the outlier.
- [ ] **AC-021** (REQ-024): A `gr_key` proposed by LLM / title-author / ISBN-scrape whose fetched detail-page title+author does NOT match is not persisted; a matching one is.
- [ ] **AC-022** (REQ-025): When one provider times out mid-resolve, the Work is still created from the responding providers and the timed-out provider produces no Conflict.
- [ ] **AC-023** (REQ-022, monitors): An author-monitored Work (seeded `ol_key`) and a series-monitored Work (seeded `gr_key`) for the same book resolve to a single Work, and it eventually carries both anchors after enrichment.
- [ ] **AC-024** (REQ-007, REQ-010): After Add Work discovery, the created Work carries the specific identifiers each consulted provider returned (e.g. `hc_key` from Hardcover); and when a resolving ISBN is present, no title/author fuzzy-search request is issued (both verified by a provider spy).
- [ ] **AC-025** (REQ-006): For Add Work and Readarr import, the source's deterministic identifier is captured before any title/author search; for series/author monitors, the native anchor is persisted at create.
- [ ] **AC-026** (REQ-013, REQ-026): An unresolved ISBN in list or Readarr import produces an identity-pending Work — not a fabricated match, not a batch failure.
- [ ] **AC-027** (REQ-027): A non-English Work resolved by ISBN does not have OpenLibrary/Hardcover English-language metadata win its fields; its description/title come from a language-compatible provider (Google Books / Goodreads).
- [ ] **AC-028** (REQ-028): A creation attempt that matches an existing Work lacking a `gr_key` merges the incoming `gr_key` onto the existing Work (verified by inspecting persisted anchors), rather than dropping it.
- [ ] **AC-029** (REQ-004): An `asin` of ISBN-10 shape that fails checksum validation is retained as `asin`; one that passes is converted to `isbn_13`.
- [ ] **AC-030** (REQ-008): A provider lacking its prerequisite (no GB key / no LLM for GR / Audnexus on an interactive path) is excluded from that discovery; discovery is never narrowed to a single provider for a multi-provider-eligible seed.
- [ ] **AC-031** (REQ-006): Readarr import with `foreign_book_id` present persists the canonicalized `gr_key`; with it absent, the Work still imports via ISBN/ASIN or identity-pending behavior.
- [ ] **AC-032** (REQ-011): An ISBN that resolves only to Google Books (no work anchor) auto-creates a Work with the ISBN as identity and no work anchor (Tier A, no confirmation) — not left identity-pending.
- [ ] **AC-033** (REQ-018): For a shared ISBN, an anchorless provider returning a genuinely different title+author (beyond the edition-variance threshold) is dropped (metadata rejected); an edition-variant title is merged.
- [ ] **AC-034** (REQ-018, REQ-020): Two providers agreeing on normalized title+author but returning different `ol_key`s raise a Conflict (terminal), not a silent merge.
- [ ] **AC-035** (REQ-020): A title+author match with a conflicting `gr_key` (or `hc_key`) raises an identity conflict that is persisted — the conflict store accepts non-OL anchor kinds.
- [ ] **AC-036** (REQ-026): A non-interactive Tier-B item the resolver cannot deterministically match transitions to needs-review (surfaced for the user), not indefinite pending.
- [ ] **AC-037** (REQ-029): An empty or malformed `isbn_13` / provider key is treated as absent — not persisted, not sent to a provider.
- [ ] **AC-038** (REQ-025): After a provider timeout, a later pass retries and converges; a partial failure does not clear already-resolved anchors/fields; a terminal Conflict pauses automatic identity changes; and after a user resolves a conflict, background passes do not silently re-change identity.
