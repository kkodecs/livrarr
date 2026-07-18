## [0.1.0-alpha6] - 2026-07-18

### 🐛 Bug Fixes

- *(ui)* Narrow the possible-match link lookup for the strict tsc gate
## [0.1.0-alpha6] - 2026-07-18

### 🚀 Features

- *(work-creation-consistency)* Identity federation + Layers 0-2 + Stage-5 review fixes
- *(work-creation-consistency)* Stage-5 green — resolver fan-out wired + Readarr seam + PO-testing fixes
- *(metadata-modularization)* Extract livrarr-external-data crate (canary GO)
- *(metadata-modularization)* Extract livrarr-identity crate (Track-1 move)
- *(metadata-modularization)* 4b step 1 — identity_status foundation (domain + DB)
- *(metadata-modularization)* 4b step 2 — RED behavioral tests for the status two-state split
- *(metadata-modularization)* 4b step 3 — text-only enrichment classifier (REQ-019)
- *(metadata-modularization)* 4b step 4 — derive/persist + gate the identity badge (REQ-014/015/016)
- *(metadata-modularization)* 4b step 5 — surface identity_status through the API (plumbing)
- *(metadata-modularization)* 4b step 6 — Book Information UI (two-state badges)
- *(metadata-modularization)* WCC chunk D — run_quorum anchored-cluster winner rule (REQ-018/020)
- *(metadata-modularization)* WCC chunk A — discovery fan-out adds Goodreads (autocomplete) + interleave
- *(metadata-modularization)* WCC chunk B — cached-payload reuse + trust-the-pick (instant add)
- *(metadata-modularization)* WCC chunk C — Tier-A manual-import auto-match (#97)
- *(metadata-refactor)* S4 — enrich_work one-road core (reuse + zero-LLM + resolve_status)
- *(metadata-refactor)* S7 — run_unified materialize + door-cutover + fork deletion
- *(metadata-refactor)* Add-from-search door onto the one road (Option A)
- *(metadata-refactor)* S6 — delete background retry job; user-triggered "Retry Incomplete" (REQ-001/REQ-011)
- *(architecture)* Author the canonical model — entity spine, seams, data flow
- *(search)* Add directly from results — drop the pre-add cover picker
- *(sprint-b)* Metadata correctness — anchor-grounded enrichment, dissents, identity completion
- *(sprint-c)* Series reconcile — full catalog, stubs, rosters, promotion
- *(sprint-d)* Seeds & doors — single SeedBuilder, per-door language, junk screen, F8 cleanup
- *(sprint-e)* Gate per-refresh identity re-chase for confirmed works
- *(identity)* Unified-identity-path engine + code-review fixes (R-001/R-002/R-003)
- *(identity)* Id-completeness — settle/converge wiring, background convergence job, affirm API + UI
- *(perf)* Diagnostics layer, GR identity fix, single cover decode
- *(metadata)* Phase 3 — outbound rate-limit queue engine (packet 1)
- *(metadata)* Phase 3 B0 — Goodreads anti-ban (1.5s pacing, drop ISBN /search tier)
- *(metadata)* Phase 3 B2 — circuit breaker at the outbound queue
- *(metadata)* Phase 3 B3 — cover pacing + cover paths onto the queue
- *(metadata)* Phase 3 B4 — request priorities wired end to end
- *(config)* Phase 5 H — per-install default language for new books
- *(domain)* Phase 5 A — identity matching authority (inert) + trap corpus
- *(test)* Phase 5 C — old-vs-new matching decision-diff harness
- *(work)* Phase 5 I — merge-two-works action with preview
- *(identity)* Phase 5 D — identity engine rewired onto matching authority
- *(matching)* Phase 5 F — recognition matcher fixes
- *(metadata)* Phase 5 G — GR unlock, HC Tier-2 LLM pick deleted, dead scaffolding removed
- *(matching)* Phase 5 E — one stored identity key, authority-routed adopt and dedup, key recompute
- *(ui)* Phase 5 J2 — identity review surface, conflicts wiring, RSS language-skip notifications
- *(covers)* Consolidate the cover pipeline — one rank, crash-safe write gate, honest provenance, single layout
- *(convergence)* Enable background convergence by default
- *(import)* Consolidate manual/Readarr/scan imports onto one core
- *(convergence)* Spawn an immediate attempt when a batch door creates a work
- *(hardcover)* Fetch-by-hc_key enrichment path (#145)
- *(docker)* Configurable PUID/PGID + Unraid & Proxmox support (#158, #105, #106)
- Responsiveness (fast add, provider cache, versioned covers) + author-dedup (adoption gate, merge)

### 🐛 Bug Fixes

- *(goodreads)* Discover books via the WAF-free autocomplete endpoint
- *(discovery)* Book search fans out across all providers (#97)
- *(metadata-refactor)* S4 review — candidate-reuse falls back to network on any DB error
- Keep qBit add-failure body check alongside 2xx acceptance
- Dispatch resolved magnet to download client instead of original URL
- *(manual-import)* Never fuse loose m4bs into a same-directory group
- *(identity)* Different work-keys veto a fuzzy-title merge (C1)
- *(identity)* Respect user picks in conflict detection; make resolve/dismiss take effect
- *(enrichment)* Readarr import metadata now reaches the merge (M-010)
- *(identity)* Affirming an anchor sets the badge synchronously (M-020)
- *(identity)* Remediate Phase 0 cross-family review findings
- *(identity)* Close round-2 review items — TOCTOU guard, typed error, gap-fill dedup
- *(metadata)* Phase 4 A-C — delete dead pacing_queue; empty-list guard + GR cover gate at the merge chokepoint
- *(db)* Phase 4 D — merge_generation predicate on the apply_enrichment_merge UPDATEs
- *(metadata)* Phase 4 E-F — honest ConvergeOutcome + error backoff; bulk refresh rides the queue at Low
- *(goodreads)* One malformed autocomplete entry no longer erases the whole hit list
- *(goodreads)* Payload title prefers GR's own bare form over search-card decoration
- *(materialize)* First cover acquisition works on every door
- *(materialize)* A changed cover pick replaces the stale file from the prior URL
- *(goodreads)* Autocomplete queries title only — the appended author poisoned ranking
- *(goodreads)* Series-page parser rewritten for the 2026-07 React layout
- *(goodreads)* Picker routes title decisions through the matching authority
- Scrub leaked PII from tracked files, add a pre-commit PII check
- *(matching)* Gate RSS auto-grab on the existing no-author hard gate (#162)
- Trivial batch — poster series line (#109), drop dead enrichment_retry_count (#137)
- *(works)* Refresh-All honors the active filters + surfaces the real error (#135)
- *(bibliography)* Guard LLM title cleanup against garbled rewrites (#53)
- *(enrichment)* Guard all non-audio merge fields against wrong-language editions (#133)
- Audiobook cover upload button (#139) + normalize locale tags before language gate (#96)
- *(hooks)* Use absolute path for pii-check PreToolUse hook
- *(rss)* Cap RSS auto-grab retries so a failing import stops re-downloading
- *(import)* Advance Readarr import progress per-item, not after the phase
- *(metadata)* Stop foreign-language editions leaking onto author pages (#112)
- *(logging)* Redact secrets from URLs in logs (#76)
- *(rss)* Score-gate before language check so skip count reflects real matches
- Quality wave 2 — six behavior fixes, red-pinned first (probe #2,#3,#9p1,#23,#33-#35,#36-adjacent)
- Quality wave 2a — one shared qBit state classifier (probe #1, D1 ratified table)
- Quality wave 2d + #36 — swallowed-writes sweep (D2) + one-tx work+anchor creation
- Indexer citizenship — origin-keyed pacing, 429 cooldown, live search cache
- Identity-fix unit — settle-road title trust, try-again wiring, honest tooltip
- Matching-conformance — one shared authority picker, kill the loose 0.75 scorers
- Openlibrary edition picker — prefer title+language match over first-ISBN
- Series list — dedup partially-linked series + unify promote UI
- Import completeness — count all on-disk files, not just recognized media
- SABnzbd path mapping — strip trailing slashes before joining
- Openlibrary search — log a dropped tier-3 result instead of failing silently
- *(covers)* Failed add-time downloads no longer create permanent user-locks
- *(goodreads)* ISBN-first search tier + detail parser for the Next.js page shape
- *(ui)* Series monitor keys, non-linear EPUB covers, notification toast dismissal
- *(identity)* Make the possible-matches surface trustworthy

### 💼 Other

- *(metadata-modularization)* Phase 1 artifacts — spec + IR v1/v2 (C′ structured extraction)
- Merge feat/metadata-modularization into wcc-stage5-green

Lands the metadata-pipeline modularization + the full WCC re-integration:
- 3-crate extraction (livrarr-external-data / -identity / -enrichment) on a
  shared substrate, one-way identity->enrichment boundary.
- WCC chunks D (run_quorum winner rule) / A (4-way Add-Work discovery + Goodreads
  autocomplete) / B (cached-payload reuse + trust-the-pick) / C (Tier-A
  manual-import auto-match, #97).
- 4b two-state-machine status split + the status-backport-drop: EnrichmentStatus
  is enrichment-only; IdentityStatus owns identity incl. NotFound (Unverified).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>

# Conflicts:
#	crates/livrarr-external-data/src/provider_client.rs
- Audiobook sync test

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
- Merge pull request #140 from kkodecs/wcc-stage5-green

Wcc stage5 green
- Support qBittorrent 5.2 session cookies in qbit_login
- Use returned qBittorrent auth cookie in server callers
- Accept qBittorrent 5.2 session cookies and 2xx responses
- Fix qBittorrent 5.2 authentication in download client test
- Fix qBittorrent 5.2 authentication in queue service
- Merge pull request #113 from Jandalslap/qbt-5.2-fix

Qbt 5.2 fix - Fix qBittorrent 5.2 authentication compatibility
- Merge pull request #136 from Vandypointe2/fix-qbit-magnet-redirect

Handle Prowlarr magnet redirects for qBittorrent
- Merge p5/unit-a: Phase 5 Unit A — identity matching authority
- Merge p5/unit-j: Phase 5 Unit J — AC-012 pin test
- Merge p5/unit-i: Phase 5 Unit I — merge-two-works action
- Merge p5/unit-d: Phase 5 Unit D — identity rewire onto matching authority
- Merge p5/unit-f: Phase 5 Unit F — recognition matcher fixes
- Merge p5/unit-g: Phase 5 Unit G — GR unlock, Tier-2 delete, dead code
- Merge p5/unit-e: Phase 5 Unit E — identity key, adopt/dedup, recompute
- Merge p5/unit-j2: Phase 5 Unit J2 — grey-park review surface
- Merge branch 'p6/n3-cover-fastfail' into metadata-remediation
- Merge branch 'p6/n2-cover-consolidation' into metadata-remediation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01D3CYzhHwXJTSuVcQWC1tDX
- Merge metadata-remediation into main — the 2026-06-28 audit remediation epoch

Phases 0-5 (stuck-state fixes, cleanup, provider gateways, global outbound
queue, data completeness + convergence, one matching authority) plus the
post-phase N-units (GR series parser rewrite, cover fast-fail, picker via
authority, cover pipeline consolidation) and the id-completeness feature.
Workspace 1331/0 at merge.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01D3CYzhHwXJTSuVcQWC1tDX
- Merge pull request #161 from kkodecs/import-consolidation

Consolidate import workflow, cover write-gate, and convergence defaults
- Merge bugfix/trivial-jul05 into main — poster series line (#109) + drop dead enrichment_retry_count (#137)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01D3CYzhHwXJTSuVcQWC1tDX
- Merge pull request #165 from kkodecs/bugfix/135-refresh-filter

fix(works): Refresh-All honors filters + surfaces real error (#135)
- Merge pull request #166 from kkodecs/bugfix/53-biblio-title-guard

fix(bibliography): guard LLM title cleanup against garbled rewrites (#53)
- Merge pull request #167 from kkodecs/bugfix/145-hc-fetch-by-key

feat(hardcover): fetch-by-hc_key enrichment path (#145)
- Merge pull request #168 from kkodecs/bugfix/133-language-merge-guard

fix(enrichment): guard all non-audio merge fields against wrong-language editions (#133)
- Merge pull request #169 from kkodecs/chore/canonical-s1-amendment

docs(canonical-model): record S1 conformance closure (#143)
- Merge pull request #170 from kkodecs/bugfix/easy-96-139

fix: audiobook cover upload button (#139) + normalize locale tags before language gate (#96)
- Merge pull request #171 from kkodecs/chore/hook-abspath

fix(hooks): use absolute path for pii-check PreToolUse hook

### 🚜 Refactor

- *(metadata-modularization)* AC-021 — drop external-data shim, direct imports
- *(metadata-modularization)* 4a — extract livrarr-enrichment (behavior-preserving carve)
- *(metadata-modularization)* Status-backport-drop — EnrichmentStatus sheds identity variants
- *(metadata-refactor)* S3b — complete download_cover_to_disk relocation (D-002)
- *(metadata)* Phase 2 — dedup provider code, shrink the god file
- *(metadata)* Phase 3 Step A — route fetcher onto the outbound queue
- *(metadata)* Phase 3 B1 — Hardcover onto the outbound queue (template)
- *(metadata)* Phase 3 B1 — all five remaining providers onto the outbound queue
- *(metadata)* Phase 3 C — enrichment queue sheds its transport duties
- *(domain)* Phase 4 G — delete stranded PacingLane/ProviderCallOutcome
- *(dead-code)* Delete the AR-08 batch + AR-13 dead trait, with cascade
- *(startup)* Startup passes on JobRunner rails; drop dead LLM merge-engine args
- *(metadata)* S1 — drop dead fields, shrink WorkServiceImpl to <D,E,H,L>
- *(metadata)* S2a — discovery concern extracted to discovery_service.rs
- *(metadata)* S2b — DiscoveryService trait; god struct down to <D,E,H>
- Delete idle suppression machinery + door-gate behavioral suite
- Quality wave 1 — dead code, dedup, workspace deps (probe #4-#18, #38, #39a)
- Quality wave 3 items 1-5 — five pure moves (probe #24,#25,#27,#28,#29)
- Split series_query_service into cohesive submodules
- Split WorkDetailPage into per-component files
- Extract update-notifier version check into a tested util

### 📚 Documentation

- Update llm-context.md for accuracy
- Refresh README to alpha5 (Quick Start pin, multi-arch, built-in readers)
- Update llm-context.md for accuracy
- *(goodreads)* Record live validation of the autocomplete fix
- *(metadata-modularization)* Phase 2 — analytical canary assessment (GO, no cycle)
- *(metadata-modularization)* Fold PO design review (2026-06-03)
- *(metadata-modularization)* Rename crate livrarr-providers → livrarr-external-data (PO)
- *(metadata-modularization)* Pre-build verification scenarios (grounded in live DB)
- *(metadata-modularization)* E/F reconcile — align spec + IR to as-built
- *(metadata-modularization)* Mark status-backport-drop Delivered in the spec
- As-built status + domain wiki page for audiobook sync
- *(wiki)* Metadata-refactor retro lessons — door→road wiring + System Truths
- *(wiki)* 13->17 crate corrections + canonical-model pointers
- *(wiki)* Post-merge addendum — audit baseline, conformance issues, gate-friction directive
- Land session working artifacts from the modularization/WCC/audit arcs
- *(roadmap)* Alpha-6 restructure + 2026-06-10 reality sync
- *(roadmap)* Alpha-6 sprint structure — audit-grounded statuses (A–F)
- Backfill CHANGELOG for alpha5; draft alpha-6 release notes
- *(roadmap)* A6 release gate — cut moves behind Sprint B + scatter parallelization
- *(roadmap)* Correct the a6 release gate — cut when Sprints A-F all complete
- *(wiki)* Series stubs/rosters/promotion domain law (sprint-c follow-up)
- *(convergence)* Persist stage-0 verdicts + convergence spec-notes; correct insight 55
- *(architecture)* Land architecture-review, reconciled-plan, stage-0 work-order + wiki updates
- Add metadata system audit (2026-06-28)
- *(metadata)* Phase 3 queue design (locked v4) + Phase 1/3 handoffs
- *(metadata)* Phase 3 foundation plan (rev 5) + build handoff
- *(wiki)* Phase 3+4 insights and change log; Phase 4 handoff
- *(phase5)* Matching authority — spec, inventory, precedent research, wiki
- *(wiki)* Log N3 close-out
- *(wiki)* N2 close-out — insight 63, amendments 51/52, pathway corrections
- Architecture-review prep — briefing doc, wiki corrections (trait count, import_pipeline network claim, canonical-model path)
- Land the flow contract — roads.md + its adversarial verification record
- Move principles.md into tracked docs/ (was gitignored under build/)
- Architecture review 2026-07-04 + type-visibility rule
- Fold product principles into ARCHITECTURE.md Part 1, retire the duplicate
- *(canonical-model)* Record S1 conformance closure in amendments log (#143)
- *(tests)* Review fix — test headers name DiscoveryService for the moved seams
- Work-service-split + orphan-cleanup record; audit reconciliation; wiki updates
- PO ratifications (D1, D2, wave-3 scope) + wiki fix-up post quality waves
- Record #36 as incomplete (pinned, not implemented) — caught at plan audit
- Insight 73 — subtitle-trust-gaps findings
- Move stray design/spec/plan artifacts out of repo root into docs/
- *(changelog)* Regenerate for 0.1.0-alpha6, stop dropping non-conventional commits
- *(wiki)* Insights 63/73 amendments + insight 74 (session findings 2026-07-17)
- *(changelog)* Fold the post-bump fix set into 0.1.0-alpha6

### ⚡ Performance

- *(covers)* Phase1 fails fast on dead hosts — 600ms connect budget + per-run negative host cache

### 🧪 Testing

- *(behavioral)* Land + register the cross-format-resume, S6-retry, and IDU bulk-identity suites
- *(behavioral)* Phase 5 J — AC-012 pin: parked works never enrich via convergence
- *(import-consolidation)* Catch up behavioral coverage for 50ac82b
- *(e2e)* Add Playwright browser-E2E runner + smoke test (#159)
- *(behavioral)* Orphan cleanup — register 7, park 3, delete 20, add manifest guard
- *(behavioral)* Close the parked-test list — port e2, drop g2/cup/phase3a

### ⚙️ Miscellaneous Tasks

- *(metadata-refactor)* Green base for parallel S3b–S8 build
- *(metadata-refactor)* Signature-lock — implement has_pending_or_running (REQ-011)
- Native arm64 runner, per-platform buildx caches, 45m timeout; node24 action bumps
- Repo cleanup + add PRINCIPLES.md and ARCHITECTURE.md
- *(metadata)* Freeze in-flight metadata WIP as pre-remediation baseline
- *(metadata)* Phase 1 cleanup — dead code, dead cache, audiobook cover dims
- *(docker)* Cache Rust dependency builds with cargo-chef (#163)
- Stop tracking the PII deny-list; the hook reads it from local disk only
- Demote per-tick job 'tick completed' log to trace
- *(release)* 0.1.0-alpha6
## [0.1.0-alpha5] - 2026-05-28

### 🚀 Features

- Metadata system phase 1 — domain model redesign
- Metadata system phase 2 — DB schema + WorkDbCreate trait split
- Metadata system phase 3a — work creation gate
- Metadata system phase 4 — provider pipeline unification
- Metadata system phase 5 — merge engine LLM arbitration
- Metadata system phase 3b — unified enrichment integration
- Metadata system phase 6 — bypass site migration
- Metadata system phase 7 — background jobs + tag convergence
- Add type/trait stubs for english-work-lifecycle (4a-prep)
- Implement text_norm and cover_gate (29/29 pure-function tests green)
- Implement bulk_resolver (8/8 concurrency tests green)
- Add DB migrations + repository impls for identity anchors and conflicts
- Implement LiveEnglishIdentityResolver + score_candidate
- Implement llm::ask_same_book for cover gate LLM tiebreaker
- Wire LiveIdentityConflictService + handler endpoints
- Add OL-key-first dedup + anchor write to work_service::add
- Enrichment cover_gate integration + repair job
- Complete EWL signature migration + identity pipeline + review fixes
- Recently-downloaded sort + URL protocol dropdowns
- Multi-cover — trust-aware cover system with picker UI
- Transmission download client support (#17)
- Google Books foreign-language metadata enrichment
- Playback enhancements — chapters, bookmarks, progress lifecycle
- Search fallback chain — GB-first discovery, Audible provider, ISBN bridge, cover picker (#73)
- UI quick wins — language filter, GB onboarding, overview density, cover play button (#72, #57, #71)
- System status page — infrastructure health summary + sidebar indicator (#74)
- Poster view shows series name + link below author

### 🐛 Bug Fixes

- Support native arm64 Docker builds
- Remove musl.cc cross-compiler, rely on QEMU for arm64 builds
- M9 bounded concurrency + M2 enrichment for matched Readarr imports
- M2 — re-enrich on ON CONFLICT race-loser path too
- Single-flight guard for Readarr import closes the M2/M8 race
- Dedupe Readarr preps by normalized identity + RAII slot guard
- Address round-1 cross-family review findings
- Address round-2 review findings (Codex P1s)
- Supersede_ol_anchor validates old != new (round-3 nit)
- Add z-50 to modal overlays so poster checkboxes don't bleed through
- Remove layout-shifting loading indicator on works poster refetch
- Make media type filter server-side across all 7 layers
- Remove file size safety check from Readarr import undo
- Version display includes alpha tag, version alert now works
- API key fields show 'leave blank to keep' when key is saved
- Use US flag emoji for English language indicator
- Alpha5 cross-family audit — 12 hardening fixes + dead-code comments
- Check qBit response body for Ok instead of trusting HTTP 200 (#85)
- PID file deadlock on container restart (#86)
- Rebuild TrustedOrigins after indexer/download client CRUD (#87)
- QBit grab fetches .torrent server-side instead of passing URL (#88)
- Auto-migrate deprecated gemini-3.1-flash-lite-preview model name (#89)
- Always render progress bar in poster view to equalize tile heights (#90)
- Right-arrow key now advances past EPUB cover page (#91)
- Wire monitoring toggle on works overview ebook/audiobook chips (#92)
- CJK title matching — bigram tokenization for non-Latin scripts (#93)
- Treat ENOENT as success when deleting library files (#94)
- TypeScript error from olKey type change in bibliography mutation
- Guided tour — add Google Books step alongside Hardcover and LLM (#72)
- OL + Audnexus clients populate cover_url for cover-alternatives picker
- *(#95)* Audiobook cover pipeline — media-aware resolution + priority + DB write
- Expose audiobook cover mtime so UI cache busts after metadata refresh
- *(db)* Exclude self in is_livrarr_process to prevent PID self-deadlock
- *(tagwrite)* Skip m4b/mp3 — upstream writers OOM on large files

### 💼 Other

- Revert "fix: remove musl.cc cross-compiler, rely on QEMU for arm64 builds"

This reverts commit dd29dbc3c70f5ca160a730f281f3c6a4886fcdfd.
- Merge pull request #37 from eskimoprince/fix/arm64-native-build

Fix/arm64 native build
- Merge pull request #46 from kkodecs/feature/english-work-lifecycle

feat: English Work Lifecycle (EWL) identity pipeline
- Merge feature/ui-quick-wins into main
- Merge feature/system-status-update into main
- Revert "docs: update README banner — alpha5 is out, upgrade instructions inline"

This reverts commit 556361b90b61bf0a6e96969b436745c310856eec.

### 🚜 Refactor

- Convention compliance sweep — fix memory leaks, secret redaction, blocking I/O
- DRY sweep + library audit — deduplicate, remove unnecessary deps
- Split title_cleanup — pure functions move to livrarr-domain

### 📚 Documentation

- Metadata principles M1-M10 and system redesign spec
- Finalize metadata system design — M9 bounded concurrency, M10 cleanup
- Add english-work-lifecycle handoff and metadata pathway wiki pages
- Mention NZBHydra2, Jackett, Prowlarr as supported indexers
- Update wiki foreign-language pipeline for shipped Google Books
- *(wiki)* GR LLM requirement + SSRF trusted-infrastructure pattern
- *(readme)* Service notice for OL 403 block
- Update Discord invite link
- Expand README banner — explain OL 403 issue, upgrade path to alpha5
- Update README banner — alpha5 is out, upgrade instructions inline
- Update README banner — alpha5 released, upgrade command inline

### 🎨 Styling

- Cargo fmt — unwrap one short line in work_service::get_detail

### 🧪 Testing

- Behavioral contracts for metadata system redesign (phases 1-7)
- Convert MergeEngine tests to generic dispatch after async cutover

### ⚙️ Miscellaneous Tasks

- Add PR build matrix for amd64 and arm64
- Gitignore GEMINI.md and graphify-out/
- Remove stray review_output.json artifact from cwd
- Mirror cross-toolchain from GitHub release, fix file permissions
- Add workflow_dispatch trigger to CI
- Register 20 ewl behavioral test targets in livrarr-behavioral
- Add diagnostic logging to automatic import path
- Update Gemini default to stable gemini-3.1-flash-lite
- Gitignore .understand-anything and kash tool directories
- Bump workspace crates to 0.1.0-alpha5
- Native arm64 matrix CI + bump frontend version to alpha5
## [toolchain] - 2026-04-29

### 📚 Documentation

- Remove personal domain from CHANGELOG
## [0.1.0-alpha4] - 2026-04-29

### 🐛 Bug Fixes

- SSRF trusted origins, manual import dedup, download poller fix

### 📚 Documentation

- Alpha3 release artifacts — CHANGELOG, README, ROADMAP
- Alpha4 release artifacts — CHANGELOG, README, ROADMAP
## [0.1.0-alpha3] - 2026-04-25

### 🚀 Features

- Readarr import — rich preview, file scope, path translation, cover download
- Wave 4 — WI-01 isolation tests, WI-09P2 services, WI-13 cleanup
- Metadata overhaul Phase 1 — queue + provenance + merge engine + retry state + external IDs
- Metadata overhaul Phase 1.5 tracer — Audnexus through ProviderClient
- Metadata overhaul Phase 1.5 — lift Hardcover + OpenLibrary into livrarr-metadata; add 3 ProviderClient variants
- Metadata overhaul Phase 1.5 — wire DefaultProviderQueue + EnrichmentServiceImpl into AppState
- Metadata overhaul Phase 1.5 — first call-site cutover (single-work refresh, English path)
- Metadata overhaul Phase 1.5 — migrate refresh_all English path
- Metadata overhaul Phase 1.5 — per-provider rate limit + concurrency in DefaultProviderQueue
- Metadata overhaul Phase 1.5 — migrate remaining English call sites; disable legacy retry job
- Metadata overhaul Phase 1.5 — queue-aware enrichment_retry_tick
- Metadata overhaul Phase 1.5 — real GoodreadsClient
- LLM-throughout pass — identity-lock at add-time + LLM validator at enrichment
- Phase 4 Wave 1 — service layer migration for author + series handlers
- Phase 4 Wave 2 — work.rs CRUD handlers routed through WorkService
- Phase 4 Wave 3 — refresh handlers routed through WorkService
- Phase 4 Wave 4 — release search/grab + queue remove through services
- Phase 4 Waves 5+6 — import workflow, matching crate, RSS sync service
- Phase 4A Step 1 — WorkService gap-fill with HttpFetcher, covers, lookup
- Phase 4A Steps 2-4 — ListService, AuthorMonitor, FileService rewrites
- Phase 4A Step 5 — handler cutover to service layer
- Readarr_import handler cutover + write_addtime_provenance consolidation
- Readarr_import handler cutover + write_addtime_provenance consolidation
- Phase 5 compile wall — extract all handlers into livrarr-handlers crate
- LLM-filtered search results + server-side pagination + bibliography toggle + seedbox sync fix
- Cover pipeline, reader/player controls, search improvements, bug fixes
- Alpha3 pre-release — security fixes, cover pipeline, GR JSON API, Docker improvements

### 🐛 Bug Fixes

- Prowlarr import uses proxy URLs; fix indexer test route
- Version is 0.1.0-alpha2, make version text more visible
- Renumber list_import_previews migration to 020
- Add missing imported_at column to library_items and log internal errors
- Renumber series_monitoring migration to 023
- Support multi-part audiobooks in Readarr import
- WI-10 migration integrity + WI-12 OPF/tag-write correctness
- WI-02/04/06 auth hardening + WI-09P1 SQL boundary enforcement
- WI-03/05/07/08/11 logic/SSRF/durability/IO/perf + OPDS password auth
- Revert modified existing migrations, drop broken CASCADE FK migration
- WI-06 remainder + CASCADE FK migration 026
- Revert migration 021 to original + delete broken migration 026
- Migration 026 — use TEMP tables to preserve attribution during CASCADE FK swap
- Misc UX + refresh_all foreign-work routing
- Bump MAX_SCHEMA_VERSION to 30 for migrations 028-030
- HardcoverClient — map "no match" errors to NotFound instead of WillRetry
- Cover download race condition + bibliography response shape
- Resolve 16 bugs from fix-plan — shared Torznab parser, grab dispatch, covers, language normalization
- Phase 1 security fixes — SQL injection, API key encoding, rate limiter, poller scoping
- Phase 1-6 fixes from codebase review v3 — 37 items across security, provider logic, domain types, handlers, infrastructure
- Complete fix plan v3.1 — PATCH semantics, validation, queue summary, dead code cleanup
- Candidate-aware M3 parser fixes RSS sync matching failures
- Goodreads tests skip gracefully when HTML fixtures are missing
- Replace Goodreads fixture tests with live-fetch integration tests
- Frontend test typos — librarr→livrarr, missing mock export, wrong error type
- C118 — backend exposes format field, remove frontend regex parsing
- Import size check before format filter, bare-word format detection, seedbox sync
- Use SSRF-safe HTTP client for qBit torrents/info endpoint
- GR series + author link — JSON API primary, HTML fallback
- Readarr import enrichment, identity lock, cover cleanup, UI fixes
- Cover priority HC-first, shared work dedup, GR match safety, UI polish

### 💼 Other

- Search/indexer UX: inline Add button, test saved indexers

- Search results: always-visible "Add" button on each row (no click-to-reveal)
- Indexers page: test button (lightning bolt) on each saved indexer row
  with loading state and success/error toast
- Priority tooltip already existed (no change needed)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Prowlarr import, audiobook grouping, cover & title fixes

- Prowlarr import: one-click import of indexers and download clients
  from Prowlarr on settings pages. Saves credentials for reuse.
  Dedup by URL (indexers) and host+port+type (download clients).
- Guided tour: new steps for Prowlarr import on indexer and
  download client pages.
- Manual import: group multi-file audiobooks (MP3s) by folder —
  single identification per folder instead of per-file. Use folder
  path for LLM/OL matching.
- Manual import: auto-search on inline reassign (was showing empty
  results because search never fired on pre-filled query).
- Covers: use OL -L.jpg (large) instead of -S.jpg (small) for
  manual import scan results — fixes blurry covers.
- Covers: delete stale thumbnails when cover changes (enrichment,
  download, upload) so new cover shows immediately.
- Enrichment: strip trailing parentheticals from title before
  Hardcover search — fixes "no results" for OL titles like
  "The Great Hunt (The Wheel of Time Book 2)". Hardcover's
  canonical title overwrites the stored title on success.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Indexer test UX: green/red bolt, HTML detection, book search fixes

- Lightning bolt icon turns green on success, red on failure
- Optimistically update Book badge in cache on successful test
  (no refetch delay)
- Detect HTML responses in indexer caps test and show clear error
  message pointing to Prowlarr proxy workaround
- Skip "no book categories" warning when book search is already
  reported as supported (Prowlarr proxies report book-search
  available but don't list individual categories)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- UX fixes: DLC test icons, search Add button, credential handling

- Download clients: lightning bolt test icons (green/red) on table rows
- Download client Prowlarr import: filter masked credentials (********),
  import as disabled, persistent warning toast to enter creds manually
- Indexer Prowlarr import: default interactive/automatic search to true
  (Prowlarr API doesn't have these fields)
- Search results: inline Add button per row, removed click-to-expand,
  per-row spinner (was spinning all buttons)
- Grab error: show actual error message instead of generic toast
- Toaster: expand mode with gap to prevent overlapping

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Send to Email/Kindle, EPUB repair integration, enrichment fixes (#2)

- SMTP email integration with Gmail/Outlook presets, test email endpoint,
  per-file send button on Library Files tab, auto-send on import
- Integrated repub crate for EPUB repair: XML declarations, mimetype fix,
  dc:language/identifier, proprietary metadata stripping, DRM detection
- Enrichment now defaults dc:language from metadata config languages
- Bulk refresh now retags library files (was DB-only, files unchanged)
- Cover handling: find_cover_path_in_opf uses manifest heuristics when
  <meta name="cover"> is absent, stale Livrarr cover duplicates cleaned
- Guided tour updated with Kindle setup step

Co-authored-by: kkodecs <kkodecs@users.noreply.github.com>
Co-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Foreign language support: search, enrichment, and UI

- 8 non-English languages (FR, DE, ES, NL, IT, JA, KO, PL) with LLM-scraped search
- Goodreads detail page enrichment for foreign works: description, genres, page count,
  publisher, rating, high-res cover via LLM extraction
- Server-side DetailUrlCache bridges search→add without exposing Goodreads URLs to frontend
- HTML cleaner preserves <a href> for detail URL extraction alongside <img src>
- Language-aware enrichment prompt rejects wrong-language descriptions
- Language persisted on work at creation time
- Provider registry with enum dispatch, cover proxy infrastructure, language settings UI
- Migrations: 012_add_metadata_source, 013_add_detail_url

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Add version footer and update notification to sidebar

- Show livrarr:0.1.0-alpha3 at bottom of sidebar, linked to GitHub repo
- Check GitHub releases API for newer versions on load
- Display "Update: livrarr:X.Y.Z" banner when new release available
- Works in both expanded and collapsed sidebar modes
- Bump server version to 0.1.0-alpha3

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Add help page with AI prompt builder, log tail API, and about section

- Help page (/help) accessible via ? icon in top nav
- AI Help: editable prompt with instance details, recent logs, and link
  to GitHub-hosted context file (docs/llm-context.md)
- One-click copy to clipboard for pasting into any AI assistant
- Setup guide button launches the configuration joyride (no longer auto-starts)
- Log tail API: in-memory ring buffer captures recent tracing output,
  exposed at GET /system/logs/tail?lines=N (admin only)
- About Livrarr: collapsible section with project background
- Documentation section with GitHub link

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- System UI: about page, logs page with search and level control, status/health merge

- Move About Livrarr from help page to /system/about
- Merge Health page into Status page (single page, two sections)
- Implement Logs page with search, highlight, match navigation
- Add log level controls: Show (client-side filter) and Capture (server-side)
- Write logs to {data_dir}/logs/livrarr.txt (Servarr convention)
- Add runtime log level API (PUT /system/logs/level) with reload handle
- Remote path mapping host field now dropdown from configured download clients
- Manual imports now record history events (imported/importFailed)
- Auto-start guided tour on first visit after setup
- Author detail: rename Monitored to Monitor, add HelpTip tooltips

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Security hardening: SSRF protection, rate limiting, session invalidation, pagination

Address findings from independent security audit (Gemini 3.1 Pro + GPT-5.4):

SSRF (H1-H3):
- Add SsrfSafeResolver custom DNS resolver that rejects private/reserved IPs
  at connection time, preventing redirect-based and DNS-rebinding SSRF
- New http_client_safe in AppState for user-supplied URL fetches
- Pre-request validate_url() kept in grab handler for fast-fail (defense in depth)
- Admin endpoints use regular client (legitimate LAN connections)

Rate limiting (H4):
- tower_governor middleware: login 5/min per IP, global 100 RPS per peer IP
- PeerIpKeyExtractor default (no spoofable header trust)

Session invalidation (H5):
- Delete all sessions BEFORE password update (safe failure ordering)
- Error propagated, not swallowed

Pagination (M4):
- Paginated DB methods for works, library items, history, notifications
- PaginationQuery with defaults (page=1, page_size=50, max 500)
- Frontend updated with PaginatedResponse<T> wrapper
- Batch IN query for library item enrichment (no N+1)

Other fixes:
- M5: RefreshGuard RAII with panic-safe Drop for refresh dedup
- M6: Docker compose-native mem_limit/cpus, wget healthcheck
- L1: RequireAdmin on get_prowlarr
- L2/L3: .without_url() on SABnzbd errors, removed Prowlarr raw body log

Reviewed by Gemini 3.1 Pro (PASS) and GPT-5.4 (3 rounds).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Import pipeline: fix data loss, stuck grabs, crash recovery, atomic fallback

Address critical and high findings from import pipeline adversarial review
(Claude Opus 4.6, GPT-5.4, Gemini 3.1 Pro):

Critical fixes:
- C-1: Stop deleting files on DB error in both single-file and MP3 batch
  import paths. Add retry recovery: existing file with no library_item
  gets validated (regular, non-zero) and adopted into DB.
- C-2: Fix false ImportFailed on crash recovery. Split skipped_count into
  skipped_dedup (success) vs failed_count. Dedup-skips count as Imported.
- C-3: Pollers now set ImportFailed on import_grab error instead of
  leaving grabs stuck in 'importing' state forever.

High fixes:
- H-1: Allow retry of ImportFailed grabs via try_set_importing CAS.
- H-2: Tag-fail fallback uses .fallback.tmp → atomic rename instead of
  writing directly to final path. Prevents partial-file corruption.
- H-4: Persist raw remote content_path on grab record (migration 014).
  import_grab uses persisted path with fresh path mapping, avoiding
  re-query race when download client removes torrent/NZB.

Medium fix:
- M-2: Clean partial .tmp files on copy failure in both import paths.

Reviewed by Gemini 3.1 Pro (VERIFIED) and GPT-5.4 (VERIFIED).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Import pipeline: per-work lock, async imports, fsync, path normalize, temp sweep

Alpha3 import hardening (H-5, M-6, H-7, M-5, M-4):

H-5: Per-work import lock via DashMap<(UserId,WorkId), Arc<Mutex<()>>>
  prevents concurrent imports of same work from racing on target paths.
  Second import waits, then dedup-skips. Unrelated imports remain concurrent.

M-6: Decouple poller from import execution. import_grab now spawned as
  separate tokio task with Semaphore(2) acquired inside the task. Poller
  continues discovering completions immediately. Prevents 5GB audiobook
  copy from blocking the entire poll cycle.

H-7: fsync on EPUB tag write — sync_all() on temp file before rename,
  plus parent directory sync after rename. Prevents silent EPUB corruption
  on power loss. Especially important for NAS/overlayfs deployments.

M-5: Normalize Windows backslash paths in remote path mapping. Both the
  content path and mapping remote_path are normalized before comparison.
  Fixes silent match failures for Windows download client users.

M-4: Startup sweep of stale temp files from root folders. Only removes
  app-owned patterns (*.fallback.tmp, *.tagwrite.*.tmp) older than 1 hour.
  Runs in spawn_blocking to avoid blocking async runtime on large dirs.

Reviewed by Gemini 3.1 Pro and GPT-5.4 (plan approved with adjustments
incorporated: DashMap for H-5, semaphore inside spawn for M-6, narrow
patterns for M-4, both-sides normalize for M-5, dir fsync for H-7).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- UX polish: notifications, queue columns, poster zoom, release chips, help prompt

PathNotFound notification rewrite, queue column reorder, works poster zoom
slider (2-8 cols), release search format chips, empty category sections,
help page prompt template update, llm-context Do Not Hallucinate section,
remote path mapping joyride, PathMappingResult diagnostics struct.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Foreign language pipeline v2: direct Goodreads parsing replaces LLM scraping

Search and enrichment for foreign works now use regex + JSON-LD parsing
instead of sending HTML to an LLM. 10-20x faster (<1.5s vs 5-25s search,
<3s vs 16-31s enrichment), better data quality (95% covers vs 0%, 100%
detail URLs vs 17%). LLM kept as automatic fallback if parsing fails.

New goodreads.rs module with 99 tests against 40 HTML fixtures across
de/es/fr/pl. Rate limiter (1 req/s, burst 5) for outbound requests.
SSRF validation on detail URLs and cover URLs.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Foreign language pipeline: spec gap fixes (11 of 19 items)

Series parsing from titles and detail pages (aria-label regex). Rating
in search results. detail_url passed through API directly (cache removed).
SRU providers and ProviderRegistry deleted. Cover SSRF restricted to HTTPS.
Parser drift warning with GitHub issue link. Thumbnail saved on add for
foreign works, kept after hi-res enrichment. View on Goodreads link in
work detail. SRP tip banner for foreign search. Zoom +/- buttons clickable.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Mobile UI: responsive layout for all 27 pages

Sidebar becomes slide-out drawer with backdrop on mobile. Header gets
hamburger menu toggle and mobile search overlay with language selector.
All pages responsive: tables hide columns progressively (sm/md/lg),
forms stack vertically, grids adapt, poster view 2-col on mobile.
Same React components with Tailwind responsive classes — no separate
mobile codebase.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Merge pull request #3 from kkodecs/mobile-ui

Mobile UI + foreign language pipeline spec fixes
- Grab search cache, portal HelpTip, author tooltips

- Add in-memory grab search cache (24h TTL, keyed by query+indexer_id)
  with cacheOnly mount check and refresh bypass
- Rewrite HelpTip to portal-based rendering (createPortal to body)
  to escape overflow:hidden and modal z-index stacking
- Add Monitor/Monitor New HelpTips to Authors list page, Author
  detail header, and Edit Author modal
- Show cache age and Refresh button on releases tab

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Review fixes: throttled cache eviction, error states, consolidated query

Cross-family adversarial review (Gemini 3.1 Pro + GPT-5.4, 3 rounds):
- Throttle cache retain() to once per 5min instead of every put()
- Cache key changed to (title, author, indexer_id) tuple, no delimiter
- Add error state for failed searches (no prior results)
- Add inline error banner for refresh failures with existing results
- Use backend cacheAgeSeconds in cache age display
- Distinguish empty cache from empty search results (hasSearched)
- Consolidate doSearch/handleRefresh into runQuery()
- HelpTip: add scroll/resize listeners for position updates
- Simplify hasSearched useEffect, remove dead code

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Book matching engine: replace LLM filename parser with deterministic M1+M2+M3+M4 pipeline

New matching module (crates/livrarr-server/src/matching/):
- M1: Embedded metadata (EPUB OPF, M4B atoms, MP3 ID3) with field-level
  sanity filters, XML entity decoding, MP3 TALB-over-TIT2 preference
- M2: Path parsing (Audiobookshelf-inspired) with noise dir collapsing,
  ignore list, signal-based author/series classification
- M3: 22-pattern regex cascade (Readarr + extensions) with pre-cleaning
  pipeline and side-channel metadata preservation
- M4: Weighted composite scoring (title 45%, author 40%, year 10%,
  series 5%) with Unicode NFKD normalization, author canonicalization,
  token-set similarity, hard gates, weight renormalization
- Reconciliation: union-find clustering, source-trust ranking,
  supplementary field merging, combinatorial fallback utilities

Integration:
- Manual import scan uses matching engine instead of LLM
- Author OL key resolved during import (fixes bibliography bug)
- OL search checks all results for duplicates (not just first)
- Search term cleaned: parentheticals stripped, long subtitles truncated
- Local sort by author/series/title restores logical display order
- Author OL key lookups cached per-import to avoid N+1
- Language tag shown in scan UI for foreign-language detection
- Duplicate detection uses existingWorkId (not just hasExistingMediaType)
- LLM parser code archived at matching/llm_parser_archived.rs
- Extracted reusable lookup_ol_authors() from author handler

Design reviewed through 3+ rounds of cross-family adversarial review
(Gemini 3.1 Pro + GPT-5.4).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- RSS Sync: full implementation with UI review fixes

RSS sync feature: per-indexer RSS fetching, M3+M4 matching pipeline,
two-phase dedup, gap detection, grab with notifications. Per-type
monitoring (monitorEbook/monitorAudiobook) replaces single monitored flag.

UI review fixes:
- Debug/trace logging throughout RSS sync pipeline for observability
- Skip releases published before work was added (RSS-FILTER-004)
- Format preference filtering via M3 side channel (RSS-FILTER-005)
- Monitoring indicators on Works list and Work detail pages (green/orange/purple/gray)
- Clickable monitor toggles on Work detail header
- Grey out non-functional Interactive Search / Automatic Search toggles

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Rename hardcover_id→hc_key, add gr_key columns
- Merge pull request #4 from kkodecs/feature/list-imports

list-imports: rename hardcover_id→hc_key, add gr_key columns
- File playback: OPDS catalog, download streaming, playback progress

OPDS 1.2 catalog with Basic Auth (API key as password) for e-reader
integration (KOReader, Thorium, Moon+ Reader). 8 routes: root nav,
recent acquisitions, author browse, search, OpenSearch descriptor,
cover proxy, and file download — all user-scoped.

File download endpoint with byte-range support via tower_http ServeFile.
Path traversal protection via canonicalization + root containment check.

Playback progress tracking (position + percentage) for ebook reading
and audiobook listening. Supports EPUB CFI, PDF page numbers, and
audio timestamps.

CSP updated for epub.js/PDF.js: blob: in script-src, worker-src,
frame-src.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Merge pull request #5 from kkodecs/feature/file-playback

File playback: OPDS catalog, download streaming, playback progress
- Readarr library import: full three-phase flow with undo

Import existing Readarr/Bookshelf library into Livrarr — authors, works,
and files. Three-phase flow (connect → preview → execute) with conservative
dedup, hardlink-first file ops, import tracking, and best-effort undo.

Backend: Readarr API client, 6 endpoints, migration 017 (imports table +
import_id columns), ImportDb trait. Frontend: full import page with
connection, preview, progress polling, history, and undo confirmation.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Merge pull request #6 from kkodecs/feature/readarr-import

Readarr library import: three-phase flow with undo
- Series monitoring: GR-powered series tracking with full UI

- New first-class `series` table (migration 019) with per-series
  ebook/audiobook monitoring flags
- GR author ID resolution (two-step: search → user picks)
- GR series list scraping with pagination and caching
- GR series detail scraping: primary works only (integer positions),
  HTML entity decoding, collection/omnibus filtering, UTF-8 safe
- Background worker creates works from series with enrichment
- Series assignment guard (most-specific series wins by work_count)
- 6 new API endpoints: resolve-gr, list/refresh/monitor series,
  get/update series detail
- Series list page with cover images, monitoring status, Add Series
  modal with inline author browser
- Series detail page with ebook/audiobook toggle buttons and
  per-format file counts (x/total)
- Shared MediaStatusRow component (Book/Headphones/Missing/search icon)
  extracted from WorksPage, used everywhere consistently
- Author detail: inline monitoring toggles replace Edit modal,
  MediaStatusRow on work rows, series section with monitor buttons
- Release search auto-executes on tab open, paginated at 10/page
- Work detail response includes seriesId

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Frontend readers: EPUB, PDF, audiobook player (#7)

* Frontend readers: EPUB reader, PDF viewer, audiobook player

EPUB reader using react-reader (epub.js wrapper) with dark/light theme,
font size control, TOC sidebar, and position persistence via CFI.

PDF viewer using react-pdf (PDF.js wrapper) with page navigation,
zoom controls, and page number persistence.

Audiobook player with HTML5 audio, album-art layout, play/pause,
seek bar, playback speed (0.5x-3x), skip forward/back, volume
control, and periodic position persistence.

All readers are full-page routes outside AppLayout but inside
AuthGuard. Read/Listen buttons added to work detail library files
table. Progress syncs to server via trailing debounce.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>

* Fix readers: token-auth streaming, blob fetch for EPUB/PDF, cascade FK

Audio player: replace full-blob download with token-authenticated
stream endpoint (/api/v1/stream/{id}?token=) for native byte-range
streaming. 297MB M4B files now play instantly instead of hanging.

EPUB reader: fetch as ArrayBuffer with auth headers instead of
passing URL with epubInitOptions (which didn't forward auth).

PDF reader: fetch as ArrayBuffer with auth headers, remove missing
CSS imports for react-pdf v10.

Migration 019: recreate playback_progress table with ON DELETE CASCADE
on foreign keys to prevent constraint violations when deleting
library items.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>

---------

Co-authored-by: kkodecs <kkodecs@users.noreply.github.com>
Co-authored-by: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- CSV import from Goodreads and Hardcover
- Merge pull request #8 from kkodecs/impl/list-imports

list-imports: CSV import from Goodreads and Hardcover
- Merge main into feature/series-monitoring

Resolve import conflict in frontend/src/api/index.ts (both series
and list-import types needed).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Merge pull request #9 from kkodecs/feature/series-monitoring

Series monitoring: GR-powered series tracking
- Remove PII from tracked files

### 🚜 Refactor

- Architecture cleanup — split monoliths, eliminate duplicate handler layer
- Architecture-excellent sprint — capability traits, settings split, OnceLock elimination, user-scoped monitor
- Credential isolation + Prowlarr config placement
- Eliminate last OnceLock — LiveReadarrImportWorkflow uses explicit fields
- Move import_single_file into LiveImportService

### 📚 Documentation

- Scaffold docs/ with architecture, API, and roadmap
- Populate wiki/ with domain knowledge from build artifacts
- *(wiki)* Deep ingest from 17 specs, policies, and build analyses
- *(wiki)* Add accuracy disclaimer — wiki is new, fix when wrong

### 🧪 Testing

- Phase 4A stress tests — edge cases and boundary conditions (Codex/OpenAI)

### ⚙️ Miscellaneous Tasks

- Cargo fmt pass
- Fix all clippy warnings across livrarr-server
- Phase 4A Step 6 — dead code cleanup after handler cutover
- Dead code sweep — warnings down from 10 to 2
- Handoff — codebase review complete, fix plan in progress
- Handoff — fix plan v3 Phases 1-6 complete, 37 fixes, 10 deferred
- Optimize Docker image size — 112MB → 76MB
- Gitignore readarr-source directory
- Remove HANDOFF.md from tracking
## [0.1.0-alpha2] - 2026-04-07

### 🐛 Bug Fixes

- Add archive.org to CSP img-src (OL redirects covers there)
- CSP img-src wildcard for archive.org subdomains (ia601601.us.archive.org etc)
- CSP img-src needs both archive.org and *.archive.org

### 💼 Other

- Fix bugs + add onboarding wizard and UI improvements

Bug fixes:
- URL doubling on download client creation (normalize_host strips scheme)
- Indexer dialog stale form state (unified handleClose across close paths)
- Browser tab title "Librarr" → "Livrarr"

Onboarding wizard:
- React Joyride guided tour: Metadata → Indexers → Download Clients
- LLM provider dropdowns with auto-fill endpoint/model, "Get API key" links
- Inline add forms on indexers + download clients pages
- Priority tooltips on indexer table headers and forms
- Compact empty states, removed redundant "+ Add" buttons

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Implement Phase 1 cross-cutting policies from code review

Phase 1D — async_trait migration:
- Replace #[async_trait] with native async fn + #[trait_variant::make(Send)]
- Remove async-trait dep from all 6 crates, add trait-variant
- Remove #![allow(async_fn_in_trait)] from 7 crates

Phase 1A — Error handling policy:
- Expand DbError: NotFound{entity}, Conflict, DataCorruption, IncompatibleData,
  Io(Box<dyn Error>) with source preservation
- Fix SQLite PRAGMAs: busy_timeout 5s, synchronous=NORMAL, journal_size_limit,
  wal_autocheckpoint per error-handling-policy.md
- Add _livrarr_meta table (schema_version, data_version) + startup version gate
- Add startup permission check, PID lock, VACUUM INTO backup, 3-version retention
- Fix complete_setup WHERE id=1 hardcoding → WHERE setup_pending=1
- Fix set_grab_download_id to return NotFound on missing row
- Fix 5 authoritative JSON unwrap_or_default → proper error propagation
- Audit 24 let _ = on DB ops → if let Err(e) = ... { tracing::warn!(...) }

Phase 1B — Test doubles policy:
- Delete InMemoryDb (1,959 lines) — all tests use SQLite :memory: via create_test_db()
- Migrate api_secondary_impl.rs + auth_impl.rs to SqliteDb
- Remove #![allow(dead_code, unused_variables)] from all 9 crates; fix 8 warnings
- Gate pub mod tests in metadata behind #[cfg(test)]
- Remove pub use livrarr_domain::* glob re-exports from 4 crates

Phase 1C — Security model (core):
- Add #[serde(skip_serializing)] on User/DownloadClient/Indexer secret fields
- Custom Debug impls with [REDACTED] for secret-bearing structs
- Add RequireAdmin extractor, apply to all admin-only routes
- Add security headers: CSP, X-Frame-Options, nosniff, Referrer-Policy
- Metadata config API returns *_set booleans instead of plaintext secrets
- Sanitize error responses — log server-side, return generic messages

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Implement Phase 2 pre-release review fixes (19 groups, 105 findings)

Authorization: RequireAdmin on all admin endpoints (config, download_client,
indexer, root_folder, author search).

Security: password inputs for secrets, test-saved-client endpoint reads creds
from DB, Session token_hash serde skip, lockout map LRU eviction (10K cap).

Error handling: enum parser errors instead of silent defaults, idempotent
library item upsert, DB write gates on file op success, enrichment/history
failures logged, safe JSON parsing in frontend.

Frontend: useQuery for GET searches, useMemo for expensive computations,
FORMAT_REGEX_CACHE, shared sortWorks/workName utils, AbortController on
unmount, Zustand UI store, tour state fix, greyed sidebar items, sessionStorage
setup persistence (secrets excluded), auth store refresh after profile update.

Code cleanup: cfg(test) gating, ClientPreset deletion, shared map_db_err_with,
type aliases for duplicate structs, ComingSoonBadge/SetupGuide deletion.

Backend: tokio::fs in async paths, per-file spawn_blocking, scan depth/entry
limits, hardcoded values extracted to constants, quick-xml dep for tagwrite.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Fix P0 security + P1 correctness from round 3 code review

Security:
- Bibliography ownership check before cache read/delete (P0)
- Session token_hash: serde(skip_serializing) + redacted Debug
- Custom Debug impls expanded to show all non-sensitive fields
- sanitize_path_component fallback goes through sanitize_inner
- RequireAdmin on download_client/indexer update endpoints
- Cover upload capped at 1MB (413 on exceed)
- Auth init preserves token on network failure (only clears on 401)
- apiFetch/apiUpload 401 syncs Zustand auth state

Correctness:
- Grab upsert preserves row ID (UPDATE instead of Delete+Insert)
- Library item conflict returns Constraint error on work_id mismatch
- Notification lookup uses direct SELECT instead of list-then-filter
- Download: structural bencode parsing for info dict (no more 4:infod scan)
- Download: trailing decoder errors propagated instead of .ok()
- useClientForm checks any credential field, not hardcoded implementations
- DownloadClientsPage wired to testSavedDownloadClient
- SearchPage single search trigger (removed double-fire)

Cleanup:
- Removed duplicate client_type field from DownloadClient
- pub use livrarr_domain::* replaced with explicit imports in metadata + db
- OpenLibraryProvider/OlSearchService gated behind #[cfg(test)]
- test_helpers gated behind feature flag
- Stale create_pool mock removed
- Bounded eviction comment fixed (was misleadingly called "LRU")
- MP3 writer: stop removing COMM frames (not ours to delete)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Rewrite EPUB tag handling with DOM-style quick-xml parser

Replaces regex-lite with quick-xml for all OPF/container.xml parsing:
- DOM-style event collection → modify → serialize (no silent truncation)
- All parse/write errors propagated via Result (was: break + return partial)
- Namespace prefixes handled natively (fixes opf:metadata matching)
- Entity encoding handled by quick-xml (fixes double-escaping)
- Single and double-quoted XML attributes supported
- Non-ISBN dc:identifier elements preserved during metadata update
- Cover manifest <item> added when inserting new cover
- OPF cover path discovered from manifest (not hardcoded)
- MAX_XML_READ_BYTES (10MB) cap on container.xml and OPF reads
- MAX_EPUB_ENTRY_BYTES (50MB) cap on zip entry reads
- Unique temp filenames (PID + timestamp) to prevent TOCTOU collisions
- Archive dropped before rename (Windows file handle compat)
- Directory entries preserved in ZIP rewrite
- regex-lite dependency removed

Also:
- Batch write_tags_batch now uses per-file spawn_blocking (was single task)
- BatchAborted error now correctly constructed with file context
- TagWriteStatus gains Clone, Copy, PartialEq derives
- MP3: only removes own COMM frame (description: "livrarr"), preserves others
- MP3: propagates read errors (was: silently creates empty tag on any error)
- MP3: restores publisher/ISBN/series TXXX frame writes

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Code quality sweep: remove stale code, optimize hot paths, fix nits

- Remove EnrichmentStatus::as_str (duplicated serde rename_all)
- Remove stale "Domain Functions" comment and banner blocks
- Optimize derive_sort_name: rsplit_once instead of Vec collect
- Rename DetectedWork.ol_key to provider_key (provider-neutral)
- Remove HTTP 202 references from job trait docs
- Fix HTTP builder: move user_agent instead of clone (self consumed)
- Remove stale "configurable middleware" doc claim
- Fix parseInt greedy parsing in category form (strict numeric validation)
- Fix useSort: type-check before numeric subtraction

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Fix regressions, DB concurrency, data integrity, and frontend correctness

Regressions:
- btmh multihash prefix: strip "1220" not "12" (SHA-256 varint)
- derive_sort_name: trim both parts after rsplit_once (consecutive spaces)
- OPF href resolution: resolve relative to OPF parent directory
- EPUB cover meta: only write when cover is actually being embedded

DB concurrency:
- try_set_importing: remove 'importing' from guard (prevents double-acquire)
- upsert_grab: atomic INSERT...ON CONFLICT replaces racy SELECT-then-write
- create_notification: explicit NULL/non-NULL dedup + INSERT OR IGNORE race guard
- complete_setup: check rows_affected() to prevent concurrent completion

Data integrity:
- update_work_enrichment: propagate JSON serialization errors (was .ok())
- read_cover_bytes: fix path to covers/{id}.jpg (was MediaCover/{id}/cover.jpg)
- import_single_item: propagate DB errors (was unwrap_or_default)

Frontend:
- SearchPage: useQuery replaces mutation+effect (fixes permanent query drop)
- IndexersPage: send empty string to clear API keys (was omitting field)
- WorksPage: div+onClick replaces nested Link elements
- apiUpload: safe JSON parsing matching apiFetch pattern

Other:
- MP3 COMM: targeted removal by description "livrarr" (preserves other frames)
- HTTP: default 30s timeout when none configured
- LlmErrorKind: add InvalidResponse variant (was mapped to RequestFailed)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Fix guided tour, auth guards, cover consistency, and CSP

- Joyride: hideOverlay, disableFocusTrap, dismissKeyAction:false so users
  can interact with the page during the tour
- Joyride: handle TARGET_NOT_FOUND by skipping to next step (not hanging)
- Joyride: targetWaitTimeout 3s for post-navigation DOM settling
- AuthGuard/GuestGuard: return null during "loading" state (was rendering
  children before auth resolved, causing flash of wrong page)
- Setup action clears tour-completed localStorage flag for fresh starts
- CSP: allow img-src from covers.openlibrary.org for search thumbnails
- WorksPage: use getCoverUrl (same as detail page), remove getCoverThumbUrl

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
- Fix P0 remote path mapping + 3 critical P1s for alpha2

P0: resolve_remote_path now normalizes trailing slashes on remote mapping
paths before boundary check. Previously rejected all files under mappings
configured with trailing "/" (e.g., /data/ instead of /data).

P1: Silent credential wipe — download client update payload now omits
empty password/apiKey fields entirely instead of sending null. Backend
preserves existing credentials when fields are absent.

P1: Remote path mapping endpoints (CRUD) now require admin role.
Previously any authenticated user could rewrite system-wide mappings.

P1: Download client "Test" now uses form.isDirty to decide between
testing saved config vs form values. Previously tested old DB config
even when host/port/URL had been changed, giving false success.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>

### 📚 Documentation

- Update compose example to use alpha2 tag (no :latest yet)
## [0.1.0-alpha1] - 2026-04-06

### 💼 Other

- Initial release: Livrarr v0.1.0-alpha1
