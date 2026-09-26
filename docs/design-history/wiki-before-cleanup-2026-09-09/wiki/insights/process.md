# Process Insights

Build-process, prototyping, and operational lessons that aren't specific to one subsystem.

### 14. **LLM privacy boundary**

14. **LLM privacy boundary:** public metadata OK. Filenames, paths, checksums, prefs, keys, IDs — never.

### 23. **Build infra first**

23. **Build infra first.** Don't start with in-memory fakes unless infra is genuinely complex.

### 24. **No band-aids**

24. **No band-aids.** Fix where data is created wrong, never add downstream workarounds.

### 25. **Prototype external endpoints before writing parsers**

25. **Prototype external endpoints before writing parsers.** curl the URL first. See [metadata-sources](domain/metadata-sources.md) for gotchas.

### 26. **Add duplication check after implementation**

26. **Add duplication check after implementation.** AI will reimplement existing logic.

### 27. **No build commentary in code comments**

27. **No build commentary in code comments.** Comments describe what code IS, not how it got there.

### 35. **Path mapping needs boundary check**

35. **Path mapping needs boundary check.** `starts_with` on strings matches `/data/downloads2` against prefix `/data/downloads`. Use `path_starts_with()` which requires `/` separator or exact match.

### 37. **SSRF**

37. **SSRF: trusted-infrastructure pattern (alpha4 lesson, do not repeat).** `AppState` carries two HTTP clients: `http_client` (unrestricted) and `http_client_safe` (rejects private/loopback IPs via `SsrfSafeResolver`). The split is **trusted-infrastructure vs runtime-derived URLs**, NOT "hardcoded vs user-supplied". Use `http_client` for **admin-configured infrastructure** that may legitimately live on a private IP — download clients (qBittorrent, SABnzbd, Transmission), indexers (Prowlarr, NZBHydra2, Jackett, direct Torznab), admin-approved Readarr origins, LLM endpoints. Use `http_client_safe` for **runtime-derived URLs** — cover proxy fetching arbitrary metadata-provider image URLs, anything from a scraper response, anything where the URL came from outside the admin's configuration. Readarr is explicit: unapproved public origins use its SSRF-safe no-redirect client; an exact normalized origin stored in the admin-managed `readarr_origins` allowlist uses the trusted no-redirect client and may resolve privately. That approval accepts rebind/private-answer risk for the configured origin while preventing API-key forwarding through redirects. Plus there's a `TrustedOrigins` allowlist (built from configured indexers/download-clients at startup) so the grab flow can fetch download URLs from those origins even when the client used is `http_client_safe`. **Reviewer trap:** an audit reviewer (or CC) seeing "user-provided URL" in a handler will instinctively suggest `http_client_safe`. That instinct is wrong here. Admin-configured = trusted = `http_client`. Reverting this distinction breaks every alpha4+ user with private-network infrastructure (the typical Docker/NAS deployment). Alpha3 shipped with that bug; alpha4 was the hotfix. See `wiki/decisions/key-decisions.md` § "SSRF: Trusted Infrastructure Pattern".

### 46. **Door→road wiring is untested — trace it at design**

46. **Door→road wiring is untested — trace it at design.** Behavioral tests cover the *pipeline* (`run_unified`/materialize), not the *door→pipeline wiring*. A handler can compile and pass the whole suite while routing **off** the one road — the add-from-search door set `skip_sync_enrichment` + spawned its own enrich/cover route, caught only by a deep audit, forcing the Option-A cutover. At design, trace every entry door/handler into the canonical pipeline; threading a value into a struct (`candidate_id` into `WorkCandidate`) is **not** the same as the door being wired. 2nd consecutive feature where a signal reached one path but not all of them (cf. metadata-modularization cross-layer threading). A new R1/R2 door is not done until its row exists in `test_door_gate.rs`. (metadata-refactor retro, 2026-06-09)

### 47. **Explicit System Truths pay off**

47. **Explicit System Truths pay off — keep the section content-rich.** A spec enumerating ST-001-style environment facts (provider audio capabilities, GB quota, anti-bot) correlated with **0 missing-system-truth review findings** — the class that recurred in prior features (raw-SQL writers, SABnzbd param, OL language filter). `verify.py` requires the System Truths section; the *content* is what kills the class. Enumerate real environment facts, never a pro-forma placeholder. (metadata-refactor retro, 2026-06-09)

### 49. **Speed baseline (2026-06-10, `d1b8768`) + the a6…**

49. **Speed baseline (2026-06-10, `d1b8768`) + the a6 release gate.** Lookup is parallel (`tokio::join!` in work_service.rs ≈1592, 8s/leg) but the enrichment scatter is **already PARALLEL** — **CORRECTION 2026-06-14:** the original "serial / zero concurrency primitives" claim was WRONG. `dispatch_enrichment` (`livrarr-enrichment/src/provider_queue.rs`) fans every provider into a `JoinSet` with a per-provider `Semaphore` + GCRA `TokenBucket`, present since the 2026-06-04 carve (`808d47a`) — i.e. BEFORE this baseline. So within-work fetchers run concurrently and GR enrichment IS rate-limited (per-provider token bucket, shared `Arc`). The per-work ≈2.2s, if still real, is dominated by LLM/pacing, not serial fetches. What IS serial is **bulk refresh across works** (`refresh_all` serial `for` loop, `handlers/work.rs:631`). The per-work timing needs a re-measure — the baseline doc and the live code disagree. Bulk refresh = serial×serial (144 works ≈ 8min, perfectly linear 3.3s/work). Scan ≈1.3s/row with warm == cold (cache likely not consulted on the scan path). Artifacts: report at `docs/speed-baseline-2026-06-10.md`; harness `scripts/speed-baseline.py` + raw JSONs in `build/reports/` (local, untracked — scripts/ and build/ are gitignored by convention). **PO decisions (2026-06-10):** `v0.1.0-alpha6` ships only when Sprints A–F are ALL complete — alpha 6 the theme and the tag are one thing, no early cut. The scatter parallelization must be the *principled* version (all anchors incl. ASIN resolved in the identity phase first — pairs with #144; pragmatic chained-pair variant rejected). **Ops rule:** bulk refresh on a live library exercises F1 — the 2026-06-10 capture wrote wrong-book merges onto 8/13 foreign works (reverted from the pre-migrate backup; forensic copy `testdata/livrarr.db.f1-damaged-20260610-183104`). Snapshot the DB before any write-heavy run and check the open critical-bug list against the code path first.

### 74. **Frontend session-fixes trio (2026-07-17, all…**

74. **Frontend session-fixes trio (2026-07-17, all live-validated).** (a) **EPUB reader + non-linear spine heads:** a spine opening with `<itemref linear="no"/>` (cover) strands epub.js at location 0 — `rendition.next()` from a non-linear section is a NO-OP, and react-reader's built-in arrows call `next()` directly (they bypass any app-level wrapper like `goNext`). Fix in `EpubReader.tsx`: on `book.ready` with no saved position, `setLocation` to the FIRST LINEAR spine item (`spine.items.find(linear !== "no")`); saved-progress CFIs win either way because both paths route through `setLocation(current === 0 ? … : current)`. `goNext`'s location-0 branch also targets the first linear item (jumping past a non-linear head IS the advance — don't chain `next()`). (b) **Toast dismissal contract (`NotificationBell.tsx`):** a sonner toast's X only closes the toast — the server row stays unread and re-toasts on every bell remount/page load; persistent toasts must carry a stable `id` (re-issue REPLACES instead of stacking duplicates) and an `onDismiss` that calls the real dismiss API. The server-side unread feed correctly excludes dismissed rows (`sqlite_notification.rs:85`) — the resurrection class is purely frontend. (c) **pathNotFound is transiently true during the seedbox→NAS transfer window:** qBit reports complete while the file hasn't reached the local mount; the import later succeeds through the mapping (Worldly Philosophers, grab 51: warned 00:23, imported clean 01:22). Possible refinement (not built): delay/retract the alert across the transfer window. Also learned en route: series "Monitor" buttons must forward the row's `grKey` (the author-series listing serves REAL keys even for rows with no DB id; only genuine stub DB rows are masked to "") — fixed at both doors (`AuthorDetailPage.tsx` SeriesRow + `SeriesPage.tsx` AuthorSeriesExpanderRow), root-caused by the fix agent against my wrong masking hypothesis.

### 75. **`dev-restart.sh` green ≠ CI green for the frontend**

75. **`dev-restart.sh` green ≠ CI green for the frontend: the dev script never typechecks (2026-07-18, broke the first alpha6 release build).** The script runs `npx vite build` directly — vite does NOT run TypeScript. CI's Docker build runs `pnpm build` = `tsc --noEmit && vite build`. A strict-mode-only error (TS2722: an indexed `Record` lookup must be bound to a local before calling — the ternary guard does not narrow a repeated index access) shipped through dev-restart + live validation and killed both release build legs at the frontend stage. Before committing any frontend change (and always before tagging): `cd frontend && npx tsc --noEmit`. Same session, release-ops lessons: (a) a background `gh run watch --exit-status` wrapper whose LAST command is an `echo` reports exit 0 regardless — read the run's real conclusion via `gh run view --json conclusion`, never the wrapper's exit; (b) `git cliff --tag vX` while an OLD tag of the same name still exists splits the release into duplicate headers — regenerate the changelog only when the tag state is final (or after deleting the stale tag); (c) a failed release build that never pushed artifacts is recoverable by delete-tag → fix → re-tag (release-check's own remedy), but once an image is published and pulled, the tag NEVER moves — patch forward. Pending-guess trust fixes shipped in the same alpha6 window (settled-slot rule): the resolver never parks, the list endpoint never serves, and the affirm door 409s a guess for an anchor slot that is already settled (confirmed ledger row or populated works column) — `confirm_anchor_in_tx` overwrites `works.*` unconditionally, so an offered competing guess was one click from silently replacing a user-confirmed identity (found live: work 34's canonical-OL guess vs its user-confirmed OL key). Guess ids in the banner now link out (GR `/book/show/`, OL `/works/`, isbnsearch, Amazon `/dp/`); Hardcover guesses are hidden entirely — hc_key is an internal numeric id with no public page to verify against. Pin: `test_id_completeness_settled_slot_guesses_are_hidden_and_unaffirmable`.
