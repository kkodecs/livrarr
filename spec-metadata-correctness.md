---
feature: "metadata-correctness"
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018, REQ-019, REQ-020]
---

# Spec: metadata-correctness (Sprint B)

Sprint B of the alpha-6 correctness cycle. Scope is the PO-settled roadmap Sprint B table
(`docs/ROADMAP.md`), re-grounded in fresh evidence sampled 2026-06-10 from current `main`
and the F1 forensic database. Where the evidence contradicts the 2026-06-07 audit (two
items already fixed on main, one item's root cause reassigned), this spec follows the
evidence; the deltas are recorded in the System Truths and the Problem Statement.

## 0a. Design Principles

Choices we're committing to. If a requirement conflicts, the principle wins.

- **Anchors are identity.** Only the identity track (add-time resolution, identity
  completion, user edit) may create, change, or clear a work's anchors. Enrichment
  consumes anchors; it never produces them. This operationalizes two existing canonical
  invariants (one-way identity→enrichment; anchor monotonicity) that the live refresh
  path violates today.
- **A skipped provider beats a wrong merge.** On refresh, fetching nothing from a
  provider is acceptable; writing another book's data never is. Correctness over
  coverage.
- **Measure before tuning.** Instrumentation (#131) is the sprint opener so Sprint E's
  speed work is measured against the 2026-06-10 baseline with real per-provider data.
- **One road stays one road.** Every fix lands at the existing single-pipeline
  chokepoints (merge engine, identity resolver, the one save home). No new paths, no
  per-door special cases.
- **Conformance is product work.** The canonical-model warm-ups (#141, #143) are
  deliverables of this sprint, not overhead; Sprint B is the first feature through the
  armed gates.

## 0b. System Truths

Facts about the environment we don't control (or inherited state we treat as given).
All sampled 2026-06-10 on `main` @ `ac296ad` unless noted.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Forensic DB `testdata/livrarr.db.f1-damaged-20260610-183104`: 70 damaged (work,field) pairs across 10/13 foreign works; per-field provenance attributes goodreads=32, google_books=16, HC/OL=0 (e.g. work 2064 `series_name` NULL→'Bridgertons', source=goodreads, set_at 18:09:30) | The 2026-06-10 corruption is wrong-BOOK adoption via GR/GB, not an HC/OL language-policy leak | Framing F1 as "add the missing HC/OL guard" — that guard already held | High |
| ST-002 | `livrarr-enrichment/src/lib.rs:486-498` `drop_language_incompatible_providers`, called at the `merge()` chokepoint; doc comment at :443-444 confirms cached + network paths share it | The foreign HC/OL drop is already enforced on ALL merge paths (audit DP5's network-path gap is closed on main) | Re-implementing the HC/OL filter; treating audit file:line claims as current | High (read directly) |
| ST-003 | `livrarr-external-data` fetch chains: Goodreads `livrarr-external-data/src/provider_client.rs:758-861` — gr_key → isbn → LLM key lookup → **title/author search**; GB `livrarr-external-data/src/google_books.rs:177-200` — isbn → title/author (+langRestrict on title path only) | Every provider fetch today falls back to text search when anchors are missing — the wrong-book vector | Assuming refresh is anchor-safe today; assuming `langRestrict` protects the ISBN path | High |
| ST-004 | `livrarr-db/src/sqlite_work.rs:912-924` (read directly): `hc_key = ?, gr_key = ?, ol_key = ?, isbn_13 = ?, cover_url = ?` raw binds; only `asin = COALESCE(?, asin)` | The enrichment merge UPDATE can NULL or replace anchors and cover_url whenever the update carries None/new values | Treating anchor loss as historical-only; any fix that leaves the raw binds in place | High (read directly) |
| ST-005 | Clean DB sample (SQL over `testdata/livrarr.db` copy): 140 enriched works; gr_key NULL on 86, ol_key NULL on 6, hc_key NULL on 8, isbn_13 NULL on 1 | A large minority of the live library is gr_key-less; anchor-grounded refresh starves them unless identity completion exists | Shipping REQ-006 without REQ-008 | High |
| ST-006 | Forensic DB: `work_identity_anchors` identical clean vs damaged (user-confirmed anchors survived); inline `works.gr_key/isbn_13` gained wrong values on 8/2 works; `external_ids` gained 3 rows in the incident (ASINs on works 2067/2133/2150; sampled clean=0 vs damaged=3 for the 13 foreign works). Merge emits external-ID upserts: `livrarr-enrichment/src/lib.rs:952-980` → `sqlite_work.rs:1046` | Anchor state is TRIPLE-stored — inline works columns, the `work_identity_anchors` ledger, and the `external_ids` table; the ledger held, enrichment writes the other two | Designs or ACs that treat inline columns as the only anchor store | High |
| ST-007 | `testdata/logs/` listing: daily `livrarr.log.YYYY-MM-DD` files continuous 2026-04-20 → today (last write 20:34 today); `livrarr.txt` frozen 2026-04-20 at 1.6 GB; status page reports `livrarr.txt` (`livrarr-handlers/src/system.rs:37`); `main.rs` logging init swallows dir-creation errors via `.ok()` | File logging is alive; the "dead" artifact is the stale `livrarr.txt` pointer; dir-creation failure (#102) is silent by construction | "Reviving" file logging by rebuilding the appender; leaving `.ok()` swallowing #102-class failures | High |
| ST-008 | `livrarr-handlers/src/system.rs:110` → `ProviderHealthState` (`livrarr-server/src/state.rs:665`): in-memory ok/error + last_error, 1h TTL; `provider_retry_state` table exists, unsurfaced; enrichment pipeline has zero per-provider timing (only scan-path gb_ms/ol_ms tracing line) | No persisted per-provider latency/outcome data exists anywhere today | Sprint E claiming before/after numbers without REQ-001 landing first | High |
| ST-009 | `transport_cache.rs` TTL 300s, consume-once, add-path only; lookup cache 15 min, UI search only; scan path (`eager_match_by_author`) calls providers directly — matches baseline warm==cold observation | There is NO 24h enrichment cache; the roadmap/E-sprint "24h (work,provider) cache" premise is wrong | Specing cache-hit-rate work against a cache that doesn't exist | High |
| ST-010 | Speed baseline `docs/speed-baseline-2026-06-10.md`: provider RTT medians GB 340ms / HC 310ms / Audnexus 30ms; serial scatter ≈2.2s/work; bulk 144 works ≈ 8 min | Per-call recording overhead is negligible relative to RTTs; baseline numbers exist for E's before/after | — | High |
| ST-011 | `livrarr-server/src/main.rs:311-322`: applicability closure — non-English works dispatch only Goodreads, Audnexus, GoogleBooks, Audible | HC/OL never even queried for foreign works (policy: metadata-refactor REQ-014); GR+GB are the foreign data sources | Adding HC/OL to foreign scatter "for coverage" | High |
| ST-012 | Code: `livrarr-external-data/src/provider_client.rs:603-604` — "first-hit matching is unreliable, and LLM judgment is required. Without an LLM configured, this path returns `NotFound`." The `/search`→202 WAF block and autocomplete cutover are recorded in-repo: `docs/goodreads-autocomplete-validation.md` (live validation) + commits `a21c643`/`33ba983`. Re-probing GR raw is policy-forbidden (account burn) | Goodreads: autocomplete is the only sanctioned endpoint; disambiguation REQUIRES LLM (NotFound without — intentional, not a bug) | Tests/prototypes hitting GR `/search` or probing GR raw; first-hit fallback designs | High (code-read + in-repo validation doc) |
| ST-013 | `works.metadata_source`: added by migration 012; zero readers in any `.rs` file; not on the `Work` struct (`enrichment_source` is) | The column is dead weight; no foreign-skip gate references it (the wiki claim is stale) | New code reading it; "wiring" it without a consumer-driven need | High |
| ST-014 | Code (read directly): `livrarr-external-data/src/google_books.rs:165-174` — no configured API key → `ProviderOutcome::NotConfigured`, the client never calls GB keyless; `google_books.rs:500-504` — the 429 branch documents "Google Books free tier is ~1000 req/day; once spent, every call 429s until the daily reset". Corroborated by live 2026-05 observations recorded at `spec-metadata-refactor.md:34` (ST-004) and `spec-work-creation-consistency.md:38` (ST-009) | Google Books requires an API key (keyless = NotConfigured, zero outbound calls) and has a finite ~1k/day budget enforced by 429s until daily reset | Keyless-GB assumptions in tests; unbudgeted GB call patterns | High for key-required (code-read); quota ceiling is Google-side — observed, not guaranteed |
| ST-015 | `docs/ROADMAP.md:165` — issue #21 marked ⛔ "OL UA cooperation paused" (standing PO decision, 2026-05-25, pending OL's reply); operational constraints documented in `wiki/integrations/openlibrary.md` | OpenLibrary is probed app-only; no new OL UA identifiers may be tested or deployed | Raw OL probing; introducing new OL UA strings in tests or prototypes | High (policy record in-repo; the truth IS the standing rule, not an empirical provider claim) |

## 1. Problem Statement

Alpha 6 is the correctness release, and metadata is where correctness fails today. On
2026-06-10 a single bulk refresh wrote wrong-book and wrong-language values onto 10 of
the library's 13 foreign works — Pan Tadeusz became Bridgertons #6, Wiedźmin got an
English statistics textbook's description — and stamped wrong identity anchors onto 8 of
them (ST-001). The mechanism: works lacking a `gr_key` (86 of 140 enriched works, ST-005)
hit the providers' title/author fuzzy-search fallback on refresh (ST-003), adopted
whatever matched, and the merge wrote both the fields and the wrong anchors because the
write site offers no protection (ST-004). The damage loop is self-sustaining: each
refresh where a provider misses can NULL an anchor, and each NULLed anchor makes the next
refresh fuzzy-search.

Around that core sit adjacent correctness defects from the metadata-lifecycle audit, all
re-verified against current main: one provider's dissent on a single field still discards
a work's entire merge (#110); "Refresh All" ignores the user's view filters, has no
language scoping, and its in-progress guard leaks permanently on panic (#135); GB-sourced
add results land without anchors and skip sync enrichment entirely (#144); cover
dimensions are never recorded despite a finished writer (#134); a dead `metadata_source`
column invites drift (ST-013); and manual import still declines valid candidates over
variant title forms (#132).

Finally, the system is blind: no per-provider latency or outcome data is persisted
anywhere (ST-008), the status page shows a stale log pointer (ST-007), and Sprint E's
planned speed work has no instrumentation to measure against the captured baseline.
Sprint B fixes the correctness core and lands the instrumentation first, so E's
parallelization is both safe (anchors resolved before scatter — the PO's principled cut)
and measurable.

Two architecture conformance warm-ups (#141 rename, #143 save-home routing) ride along as
the first deliberate exercise of the armed canonical-model gates.

## 2. Requirements

### Group A — Instrumentation (#131, sprint opener)

- **REQ-001** — Per-provider call records. Every provider fetch attempt made by lookup,
  identity resolution, and enrichment (including cover fetches) — whether it goes to the
  network, is served from cache, or is skipped — produces a
  persisted record: provider, operation (lookup | identity | enrich | cover), timestamp,
  duration, and outcome class (success | not_found | rate_limited | timeout | error |
  skipped_no_anchor | skipped_policy | llm_rejected | cached). Records survive restart and are retained
  bounded (default: 30 days or 100k records, whichever is hit first; oldest evicted).
- **REQ-002** — Status page provider panel. The system-status page shows, per provider,
  over a rolling 24h window: call count, success rate, median latency, last error
  (message + time), and last success time. This replaces the current ok/error-only view.
- **REQ-003** — Log surface tells the truth. The status page reports the actual active
  log file path (the daily rolling file) and its last-write time. A failure to create or
  write the log directory at startup is surfaced as a visible status-page error and a
  stderr message — never silently swallowed (#102's vector).

### Group B — Canonical-model conformance warm-ups

- **REQ-004** — `Release` rename (#141). The domain entity representing an indexer search
  result eligible for grabbing is a public type named `Release`; no type named
  `ReleaseSearchResult` remains. Search, grab, and RSS behavior is unchanged.
- **REQ-005** — Import save routes through the save home (#143). The import-time
  metadata/tag persistence step is performed via `livrarr-materialize` (the one save
  home); `livrarr-library` no longer depends on `livrarr-tagwrite` directly. Observable
  import behavior is unchanged (EPUB tag writes load-bearing; audio tag writes remain
  disabled).

### Group C — Refresh correctness (F1 root cause, F4, #144, policy pins)

- **REQ-006** — Anchor-grounded enrichment fetches. Enrichment-phase provider fetches
  (any door, any mode — add, refresh, bulk, retry) use only the work's stored anchors:
  GoogleBooks by isbn_13; Goodreads by gr_key; Hardcover by hc_key/isbn_13; OpenLibrary
  by ol_key/isbn_13; Audnexus/Audible by asin. A provider for which the work has no
  usable anchor is skipped and the skip is recorded (outcome `skipped_no_anchor`,
  REQ-001). No title/author text search occurs in any enrichment fetch; text search
  exists only in lookup and identity resolution.
- **REQ-007** — Enrichment never writes identity. The enrichment merge cannot create,
  mutate, or clear anchors (gr_key, ol_key, hc_key, isbn_13, asin) or any identity state,
  on any path — across all three anchor stores: the inline work columns, the
  `work_identity_anchors` ledger, and the `external_ids` table (ST-006). Anchor changes
  flow exclusively through the identity track — add-time
  resolution, identity completion (REQ-008), or user edit — honoring anchor monotonicity
  (new anchor types append; established anchors change only by user edit; contradictions
  raise to the user).
- **REQ-008** — Identity anchor completion on refresh. Refreshing a work runs identity
  anchor-completion before the enrichment scatter: missing anchors are resolved with
  identity-grade rigor (deterministic matching first; LLM disambiguation where the
  provider requires it, e.g. Goodreads; no naive first-hit adoption). A completion
  failure for one provider leaves that anchor absent (and that provider skipped per
  REQ-006) — it never falls back to fuzzy adoption. Anchor-less works thereby converge
  over successive refreshes instead of starving permanently (ST-005's 86 works).
  Completion attempts are recorded (REQ-001) and bounded: a failed or not-found
  completion is not re-attempted automatically on every refresh — re-attempts follow
  the existing per-provider retry-suppression/backoff semantics and resume when the
  work's identity inputs change (user edit, new anchor) or the user explicitly retries.
  Bulk refresh never re-runs completion unboundedly for unresolvable works.
- **REQ-009** — No-clobber write guard. A merge-driven work update never overwrites a
  populated column with NULL/empty. This existing provenance invariant is extended to
  the anchor columns, `cover_url`, and `language` at the write site (ST-004), as
  defense-in-depth beneath REQ-007. (`language` is load-bearing: a NULLed language flips
  a foreign work onto the English provider policy.)
- **REQ-010** — Anchor-less add gets identity + enrichment (#144). Works added from
  lookup results that carry no candidate payload/anchors (today: GB-sourced results,
  systematically) receive add-time identity resolution that populates all resolvable
  anchors — including ASIN — followed by the standard synchronous enrichment leg. No add
  door produces a permanently-unenriched work while providers are reachable. (This is
  the add-path half of "identity resolves all anchors first," the prerequisite for
  Sprint E's parallel scatter.)
- **REQ-011** — Discovery language regression pin. Discovery results never carry a
  language inferred from the query term (#11; already fixed on main) — pinned by a
  behavioral test so it cannot regress.
- **REQ-012** — Foreign HC/OL exclusion policy pin. No OpenLibrary or Hardcover metadata
  is ever written onto a foreign-language work, on any path (metadata-refactor REQ-014
  policy). Already enforced at the single merge chokepoint (ST-002); this spec pins it
  with behavioral tests and keeps the rule documented at that site.
- **REQ-013** — Known-incompatible language values are dissent. On a foreign-language
  work, a provider text value (description, subtitle, series_name, genres) whose payload
  language is known and incompatible with the work's language is not written; it is
  recorded as dissent (REQ-014 semantics). Unknown payload language is not treated as
  incompatible. *(Defense-in-depth; adopted — Q-002.)*

### Group D — Per-field conflict (F5 / #110)

- **REQ-014** — Per-field/per-provider conflict semantics. When a provider's
  contribution conflicts — at payload level (the provider appears to describe a
  different book) or at field level (contradictory values) — only that provider's (or
  that field's) contribution is excluded; the dissent is recorded queryably (providers,
  fields, values) and the remaining contributions merge normally per the priority
  model. A dissent never blocks the work's merge; the whole-work enrichment Conflict
  outcome is retired. (The detection granularity follows the existing Conflict-outcome
  definition — settled at design; the behavioral contract is: dissent isolates, never
  discards the merge.) Identity-level conflicts (IdentityConflict) keep their existing
  user-facing flow, unchanged. No new conflict UI this sprint.

### Group E — Refresh All (#135)

- **REQ-015** — Refresh All respects the active view. `WorkFilter` gains a `language`
  field; the library view gains a language filter facet; "Refresh All" operates on
  exactly the works selected by the active library filters (language, monitored,
  enrichment status, media type) — refresh-what-I-see.
- **REQ-016** — Bulk guard cannot leak. The bulk-refresh in-progress guard releases
  robustly: a panic or early exit during the loop never leaves the system permanently
  refusing (409) subsequent bulk refreshes — a new bulk refresh is accepted without a
  restart once the failed run is dead. Genuine concurrent duplicates are still rejected
  while a run is live.

### Group F — Covers + cleanup (#134 F7, metadata_source)

- **REQ-017** — Cover dimensions recorded. Cover resolution records the stored cover's
  width and height (wiring the existing `update_cover_dimensions` writer); newly
  resolved covers populate dimensions immediately, and existing covers backfill when
  refresh re-resolves them. Dimensions are read from the stored image file regardless
  of cover trust — a user-locked cover gains dimensions during refresh without any
  network resolution and without its URL or trust changing. Dimensions are exposed
  through the existing work API fields.
- **REQ-018** — Delete `works.metadata_source`. A new migration drops the column
  (zero readers, ST-013). Applied migrations are never edited.
- **REQ-019** — User-cover survival regression pin. A user-set cover survives every
  refresh and bulk refresh (CoverTrust::User honored on the one road; already
  implemented) — pinned by a behavioral test.

### Group G — Manual-import matching (#132 remainder)

- **REQ-020** — Variant-title auto-match. Tier-A auto-match accepts variant title forms
  that today decline valid candidates: "(Unabridged)" suffixes, ": <subtitle>, Book N"
  forms, translated-subtitle variants, and diacritic-only differences. The four
  documented staging repro cases (Q-001) auto-match to their correct candidates; the
  existing match suite shows no new false positives (variant acceptance never matches a
  different book).

## 3. UI/Interface Design

Small surface; no mockups (consistent with existing component patterns — PO may request
mockups instead):

- **Status page:** provider panel per REQ-002 (table: provider, 24h calls, success rate,
  median latency, last error + when, last success); log section per REQ-003 (actual
  rolling file path + last-write time; loud error state when the log directory is
  unwritable).
- **Library:** language filter facet alongside the existing filters (REQ-015); "Refresh
  All" keeps its current button — its scope simply follows the active filters; no modal.
- No new conflict UI (REQ-014 records dissent in data only).

## 4. Non-Requirements

Explicit scope exclusions:

- **Enrichment-scatter parallelization, pacing/budget tuning, cache work** — Sprint E
  (B lands the prerequisites: anchors-at-identity + instrumentation).
- **Bulk-refresh pipelining / per-work flag granularity beyond leak-proofing** — post-a6.
- **Series cluster (F6: #58/#112/#111/#52/#109)** — Sprint C.
- **SeedBuilder / per-door language seeds (F2), author-biblio quality screen (#53),
  god-object split + dead `MetadataProvider` trait (F8)** — Sprint D.
- **GR resolution ladder** (proper wrong-book fix for GR *search*) — deferred post-beta
  (REQ-018 of the metadata-refactor spec's deferral list; #11's remaining half).
- **Held/unverified-work covers product call** — already answered by metadata-refactor
  REQ-015 (covers independent of identity); implementation check rides that thread, not
  this sprint.
- **Log viewer / log-file management UI; deleting the dev box's 1.6 GB `livrarr.txt`**
  — ops note, manual cleanup.
- **Re-enriching or repairing data damaged before this sprint** beyond what REQ-008's
  convergence naturally heals — no one-off migration scripts for metadata values.
- **mp3-specific import/grouping work** — out of scope by standing PO priority.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Enumerate the four #132 staging repro cases (file/title → expected candidate) so REQ-020's ACs are concrete | resolved | AMENDED (PO, 2026-06-11): the original manual-import dataset still exists as a folder — AC-022 is validated by running manual import over that full set at the Test stage (previously-declined titles auto-match, no new false positives); no hand-pinned fixtures. The four form classes bind the requirement and stand as the behavioral unit slice. (Original resolution — four fixtures pinned from the staging set — superseded.) |
| Q-002 | Adopt REQ-013 (known-incompatible-language dissent) as specced, or drop as redundant once REQ-006/007/008 land? Risk to weigh: over-blocking legitimate English descriptions on foreign editions vs. residual wrong-edition anchors | resolved | Adopted — REQ-013 stands; starvation risk accepted per P2 "a book's language is sacred" (PO, 2026-06-10) |
| Q-003 | REQ-001 retention bound: 30 days / 100k records acceptable? | resolved | Confirmed: 30 days / 100k, oldest evicted (PO, 2026-06-10) |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Every operation class produces persisted records — a search
  produces lookup records for each provider queried; an add that runs identity
  resolution produces identity records; a refresh produces enrich records for each
  provider attempted or skipped; cover resolution produces cover records — each record
  with provider, operation, duration, and outcome class populated, and all present
  after a server restart.
- [ ] **AC-002** (REQ-001): With retention bounds exceeded (simulated), oldest records
  are evicted and the store stays within bounds.
- [ ] **AC-003** (REQ-002): Status page shows per-provider 24h call count, success rate,
  median latency, last error + time, last success time, fed by REQ-001 records.
- [ ] **AC-004** (REQ-003): Status page shows the actual daily rolling log path and a
  last-write timestamp that advances with activity; pointing the data dir at an
  unwritable location yields a visible status-page error and a stderr message at startup.
- [ ] **AC-005** (REQ-004): No Rust symbol (type/trait/function/field) named
  `ReleaseSearchResult` remains in the workspace crates' code; a public `Release` domain
  type exists; search/grab/RSS behavioral tests pass unchanged. (Docs, spec prose, and
  the canonical model's amendments history are exempt from the name ban.)
- [ ] **AC-006** (REQ-005): `cargo tree -p livrarr-library` shows no `livrarr-tagwrite`
  dependency; import behavioral tests (EPUB tags written, audio skipped) pass unchanged.
- [ ] **AC-007** (REQ-006): A work with no gr_key refreshed with Goodreads reachable
  performs no Goodreads fetch; the outcome record shows `skipped_no_anchor`; no
  title/author search request is constructible from any enrichment fetch path.
- [ ] **AC-008** (REQ-006/007, F1 fixture): Given the forensic F1 starting state (foreign
  works, gr_key NULL — e.g. Pan Tadeusz), a bulk refresh with all providers reachable
  writes zero cross-book field values and zero anchor changes; `series_name` stays NULL
  rather than becoming another book's series.
- [ ] **AC-009** (REQ-007): A merge whose provider payloads contain anchor values leaves
  all three anchor stores untouched: inline work anchor columns byte-identical, zero
  external-ID upserts emitted (the `external_ids` row set unchanged), zero
  `work_identity_anchors` writes.
- [ ] **AC-010** (REQ-008): A work missing gr_key whose identity completion
  deterministically resolves (or LLM-confirms) gains the correct gr_key on refresh, and
  the subsequent scatter fetches Goodreads by that key; a completion that cannot confirm
  leaves the anchor absent and the provider skipped. A work whose completion returned
  not-found is not re-completed on the immediately following refresh (suppression
  observed); a user-initiated retry re-attempts it.
- [ ] **AC-011** (REQ-009): A merge update carrying NULL for a populated anchor,
  cover_url, or language leaves the stored value unchanged.
- [ ] **AC-012** (REQ-010): Adding a GB-sourced lookup result (no candidate anchors)
  produces a work with resolved anchors (including ASIN when resolvable) and
  `enrichment_status` ≠ unenriched while providers are reachable.
- [ ] **AC-013** (REQ-011): A discovery query in language X for a title yields results
  whose language is not stamped X by the query (regression test on the #11 case shape).
- [ ] **AC-014** (REQ-012): A foreign work merged with HC/OL payloads present in input
  shows zero HC/OL-sourced field writes, on cached and network paths alike.
- [ ] **AC-015** (REQ-013): A known-English description payload on a French work is
  recorded as dissent and not written; an unknown-language payload is unaffected by
  this rule.
- [ ] **AC-016** (REQ-014): A merge where one provider's contribution conflicts writes
  all non-conflicting contributions, records the dissent (providers, fields, values),
  and does not block; no enrichment outcome can discard the whole merge over a dissent.
- [ ] **AC-017** (REQ-015): "Refresh All" scopes to each active facet — language=fr
  refreshes exactly the French works; monitored=true exactly the monitored works; an
  enrichment-status filter exactly that status; media_type likewise; combined facets
  intersect; with no filters, all works.
- [ ] **AC-018** (REQ-016): A bulk-refresh task killed/panicked mid-loop does not leave
  the system returning 409 for a subsequent bulk refresh (no restart required); starting
  a second bulk refresh while one is genuinely running still returns 409.
- [ ] **AC-019** (REQ-017): A newly resolved cover row has non-NULL width/height
  matching the stored image; a pre-existing cover gains dimensions after its next
  refresh re-resolve; a user-locked cover gains dimensions on its next refresh with
  URL and trust unchanged.
- [ ] **AC-020** (REQ-018): A new migration drops `works.metadata_source`; the column is
  absent post-migrate; no code references it.
- [ ] **AC-021** (REQ-019): A user-set cover survives single and bulk refresh
  byte-identically (URL unchanged, trust stays User).
- [ ] **AC-022** (REQ-020): Running manual import over the PO's original #132 dataset
  folder (Test stage) auto-matches the previously-declined variant-title cases at
  Tier-A with no new false-positive matches; the four form-class behavioral tests
  and the existing match suite pass (Q-001 as amended 2026-06-11).
