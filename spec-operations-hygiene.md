---
feature: operations-hygiene
stage: spec
status: as-built
version: 4
type: bugfix
req_ids: [REQ-101, REQ-102, REQ-103, REQ-201, REQ-202, REQ-301, REQ-401, REQ-402, REQ-403, REQ-404, REQ-501, REQ-601, REQ-602, REQ-603, REQ-701]
---

# Bug Spec: operations-hygiene

## Executive summary

**Delivered, with one item waiting on a push.** All seven fixes were built, passed review, were
deployed to the local app on 2026-10-06 and passed the PO's live check the same day ("ok"). They
are committed locally as `25ca4f30`, not yet pushed. Every requirement is delivered and no decision
changed. Still open: proof that CI runs on the first code push to main and skips a docs-only push
needs that push (AC-513); log pruning at a change of day, a failing database inside a container
and a cold image build were never run ([delivered and deferred](#delivered-and-deferred)). Three
small differences from this spec's wording were accepted: the config loader still also returns the
unknown keys separately, the public health handler's return type changed so it can send a 503 with
the item list, and four lint disable comments remain, not five
([corrections](#corrections-found-during-the-build)). The [as-built section](#8-as-built) names the
new pieces, the tests and the review history.

Seven small fixes so Livrarr tells the truth about its own health, keeps its disk use bounded and
gets two broken checks working again. The PO decided all seven on 2026-10-06, answered the three
open questions the same day, and added one small fix (proxy ranges ending in `/0`). This spec turns
them into testable requirements. Nothing here changes the database schema or adds a runtime
dependency. The frontend gains one development-only lint plugin. Estimated size: about two days
with tests ([sizes](#build-files-and-sizes)).

1. **The health check really checks the database.** Today `/api/v1/health` always answers "database
   is reachable" without looking. After the fix it reads the database, gives up after 2 seconds, and
   answers with HTTP 503 when the read fails, so Docker and other monitors notice. Callers without a
   login get only "ok" or "database check failed", never internal detail
   ([REQ-101, REQ-103](#2-requirements)).
2. **System → Status gets its health list from a new admin-only reply** that holds the database
   check plus the config warnings below. If the admin's login check gets through but the database
   read then fails, the page shows a red database row. When the database is fully down, the page
   cannot load at all, because checking the admin's login needs the database; that is today's
   behaviour and it stays ([REQ-102](#2-requirements), [ST-003](#0b-system-truths)).
3. **The Docker image carries its own health check** on port 8789 inside the container. People who
   run the image without our compose file (Unraid, plain `docker run`) get it with the first
   release image built after this change; no image tag is changed here. Our compose example keeps
   its own health check, because it still selects the alpha6 image, which has none. An admin who
   moves the port or binds to one specific address inside the container must set their own check;
   the docs will say so ([REQ-201, REQ-202](#2-requirements), [D-001](#6-decisions)).
4. **Log files are pruned.** After each change of day Livrarr keeps today's file and the 30 most
   recent earlier log files; after a restart later the same day it keeps today's and 29 earlier. The
   library counts files, not calendar days, so days on which Livrarr did not run do not use up the
   30. An old undated `livrarr.log` counts as a log file and can be deleted
   ([REQ-301](#2-requirements), [D-003](#6-decisions)).
5. **Frontend lint works again.** The React hooks lint plugin is installed with its two classic
   rules only. Of the four places it flags, one is a small visible bug in the phone search
   (fixed, with a test); three are deliberate and get a disable comment that says why. Two stale
   disable comments go. Lint then passes with zero warnings ([REQ-401 to REQ-404](#2-requirements)).
6. **CI runs on every push to main**, except pushes that change only `docs/`, `wiki/`, `build/` or
   Markdown files at the repository root. None of those can change the Docker image. Pull requests
   and manual runs are unchanged; CI stays Docker-only ([REQ-501](#2-requirements)).
7. **Bad `[server] trusted_proxies` entries and unknown config keys** each produce a log warning at
   startup and an amber row naming the entry or key in the Health Checks list on System → Status.
   Livrarr still starts, and a rejected entry is never trusted. The row stays until the admin fixes
   `config.toml` and restarts, because the file is read only at startup. An unknown key's value is
   never shown. A `/0` range now matches every address of its kind instead of crashing a
   development build ([REQ-601 to REQ-603, REQ-701](#2-requirements), [D-002](#6-decisions)).

**No open questions.** The PO's three answers are recorded as D-001 to D-003 in
[§6](#6-decisions).

**Evidence.** Code read at `c05752d8`; probes P1 to P19 in
`build/reviews/operations-hygiene/packet-0-spec/probes.log` (P17 to P19 added for version 2). Each
System Truth says what was probed or read. Corrections to the packet's premises are in
[§0c](#0c-prior-art) and ST-028.

## Revision

- v1, 2026-10-06: written from source at main `c05752d8` per
  `build/reviews/operations-hygiene/packet-0-spec/PACKET.md`.
- v2, 2026-10-06: folded per `build/reviews/operations-hygiene/packet-0-spec/FOLD-PACKET-v2.md`.
  PO answers: Q-1 → D-001 (fixed check plus docs note); Q-2 → D-002 and new REQ-603, AC-618 (`/0`
  matches every address); Q-3 → D-003 (30 files). Review `REVIEW-astra-spec-r1.md`: F1 (keep the
  compose check; REQ-202, AC-216, ST-029, D-201); F2 (schema-read failure fixture through the real
  router; ST-026, D-104, REQ-103, AC-116); F3 (admin handler failure through the real door;
  AC-124, AC-123 now supplemental); F4 (log init error is an API field and stderr only; ST-028,
  REQ-301); F5 (startup wiring through the real binary; AC-317, AC-318); F6 (one-to-one retention
  table with the undated file; ST-027, AC-311 to AC-313, summary item 4); F7 (rejected ranges stay
  untrusted through the real router; AC-617); F8 (non-loopback bind marked not applicable; D-202).
- v3, 2026-10-06: folded per `build/reviews/operations-hygiene/packet-0-spec/FOLD-PACKET-v3.md`.
  Review `REVIEW-astra-spec-r2.md`: F9 (REQ-603 and problem statement 8 now describe both header
  paths truthfully: `X-Real-IP` picks the key, an all-trusted `X-Forwarded-For` chain falls back to
  the peer; new AC-620); F10 (separate IPv4-only and IPv6-only `/0` configurations, each with an
  opposite-family peer that stays untrusted; AC-618 and AC-619 replace v2's combined AC-618).
- v4, 2026-10-06: as built at local commit `25ca4f30`, per
  `build/reviews/operations-hygiene/packet-7-as-built/PACKET.md`. v3 is kept at
  `build/reviews/operations-hygiene/packet-0-spec/spec-operations-hygiene-v3.md`. Status
  `as-built`; executive summary leads with what shipped. Corrected: ST-012 and REQ-401 (installed
  range `~7.1.1`, installed with pnpm 10 from inside `frontend/`); ST-017 and REQ-404 (four
  `exhaustive-deps` disables remain, not five); ST-023 (two return types, see
  [corrections](#corrections-found-during-the-build)); REQ-301 (the retention sentence as written
  after the code-review fold); AC-612 and AC-616 labels; AC-123 written at the code stage. Added:
  AC-715 (tests review r1, the real-binary unknown-key check); acceptance boxes ticked except
  AC-513; [§8 As built](#8-as-built).

## 0a. Design Principles

- One definition per fact. One database check serves Docker and the Status page. One parser decides
  whether a proxy entry is valid, for the rate limiter and for the warning. One function turns the
  loaded config into warnings, for the log and for the page. One constructor builds the log file
  writer, and startup uses it.
- Public replies stay minimal. A route open without login says only what Docker needs; detail goes
  to an admin route and the log (security policy, [PA-006](#0c-prior-art)).
- Warn, do not refuse. A bad optional config entry is skipped and reported; startup refusal stays
  reserved for values the server cannot run with (`config.rs:293-359`).
- Use the library's mechanism (the appender's file cap, GitHub's path filter, the lint plugin's
  rules) rather than hand-rolled code.
- Tests drive the real door: the real `build_router` over the real `SqliteDb`, the real config
  loader on a real file, the real binary for startup wiring, the real component with only `fetch`
  stubbed, a real image build. A failure is induced in the real engine (SQLite refusing a read),
  never injected as an outcome. Pin the named observable only.

## 0b. System Truths

Read at `c05752d8`. "(packet)" marks a premise taken from the packet or its grounding file
(`build/state/chunk-5-grounding-2026-10-06.md`), re-opened here. P-numbers are in `probes.log`.

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-001 | (packet) The health handler ignores its state and returns one fixed item, `database` / `ok` / "database is reachable". It is mounted without login. Callers: System → Status (`getHealth`), the unrouted `HealthPage.tsx`, the compose health check and the docs. An unused `system::routes` also mounts it. | `crates/livrarr-handlers/src/system.rs:24-32`, `:263-265`; `crates/livrarr-server/src/router.rs:59-74`; `frontend/src/api/index.ts:587`; `frontend/src/pages/system/status/StatusPage.tsx:182-190`; `frontend/src/pages/system/health/HealthPage.tsx:23-27`; `docker-compose.yml:27-32`; `docs/llm-context.md:232-237`, `:379` | A health reply that does not depend on a real read. | read |
| ST-002 | (packet) System → Status is admin-only and is the only routed screen with the Health Checks list. `HealthPage.tsx` has no route and no link. The page waits for three replies (status, health, health summary), replaces the whole page with an error and Retry when the status or health call fails, and colours a `warning` item amber. The sidebar health widget reads only the summary, for admins only. | `frontend/src/App.tsx:381-389`; `rg HealthPage frontend/src` finds only the file itself; `StatusPage.tsx:172-205`, `:19-29`, `:237-272`; `frontend/src/components/Sidebar/Sidebar.tsx:340-349` | Requirements written for `HealthPage.tsx`. | read |
| ST-003 | Every signed-in call (session or API key) looks up the session or key in the database before the handler runs; a database error answers 500 with no body, which the web client shows as "Something went wrong". So when the database cannot answer at all, System → Status shows its whole-page error with Retry, and no health list. | `crates/livrarr-server/src/middleware.rs:15-80`; `frontend/src/api/client.ts:81-96`, `:109-111`; `frontend/src/components/Page/ErrorState.tsx:11-24` | Promising an admin a red database row when the database cannot answer at all. | read |
| ST-004 | Handlers cannot reach the database directly; they reach it through a domain trait, a `Has…` capability trait and an `AppState` impl, for example `HasWorkIdentityRepository` with `type = SqliteDb`. `SqliteDb` exposes its pool. | `crates/livrarr-server/src/state.rs:267-272`; `crates/livrarr-handlers/src/context.rs:312-315`; `crates/livrarr-db/src/sqlite.rs:10-21` | A handler that takes a pool. | read |
| ST-005 | The live pool: WAL, `busy_timeout` 5 s, at most 4 connections, sqlx's default acquire wait 30 s. A read of `sqlite_schema` succeeds on a healthy database and while another connection holds a write transaction (WAL readers do not wait). With every connection held, the read waits; a 2 s `tokio` timeout returns after 2.0 s. With connections open, renaming the file away or overwriting its header did not make the read fail. So the check detects a pool that cannot hand out a connection in time and any SQLite error the read raises; it does not detect a deleted, moved or damaged file while connections stay open, nor a full disk (reads still work). The test pool has one connection. | `crates/livrarr-db/src/pool.rs:12-34`; `sqlx-core-0.8.6/src/pool/options.rs:160`; `crates/livrarr-db/src/lib.rs:94-97` | Claiming the check proves the file is intact or writable. | probed P10 (real `create_sqlite_pool`) |
| ST-006 | busybox `wget` in `alpine:3.21` (BusyBox 1.37.0): `--spider` sends GET, not HEAD; exit 0 on 200; exit 1 on 404, 500, 503 and on a refused connection; `-T 2` ends a silent server after 2 s with exit 1; without `-T` it waits until killed. `--tries` and `--no-verbose` are accepted though not listed in its help. | — | Relying on HEAD, or on a non-failure exit for a 503. | probed P8 |
| ST-007 | An axum `get` route answers HEAD with the same status and an empty body (503 in the probe). | — | — | probed P11 (axum 0.8 router) |
| ST-008 | (packet) The runtime image is `alpine:3.21` with no `HEALTHCHECK` and no `USER`; the entrypoint starts as root, then drops to `PUID:PGID` and runs `livrarr --data /config --ui-dir /app/ui`. A health check process therefore runs as root, or as the `user:` an admin set. Port 8789 is exposed. The compose file has a `wget` check on `127.0.0.1:8789/api/v1/health` (interval 30 s, timeout 5 s, retries 3, start 10 s). The Unraid template sets no health keys. Last CI build took 8 to 9 minutes per architecture on GitHub runners. | `Dockerfile:78-108`; `docker/entrypoint.sh:12-16`, `:49-50`; `docker-compose.yml:27-32`; `docker/unraid-template.xml:1-36`; `gh run view` of the 2026-07-08 run | — | read; durations from `gh` |
| ST-009 | `[server] port` and `bind_address` decide the listening address and are used nowhere else. `[server] url_base` is validated and has no effect: no route uses it, and the docs say so. | `crates/livrarr-server/src/main.rs:1152-1162`; `crates/livrarr-server/src/config.rs:302-320`; `rg url_base` over `crates/livrarr-server/src` (config only, plus the unrelated download-client field); `docs/llm-context.md:187` | Building the health URL from `url_base`. | read |
| ST-010 | (packet) The log file is `tracing_appender::rolling::daily(log_dir, "livrarr.log")` with no cap, named `livrarr.log.YYYY-MM-DD` by UTC date; `log_surface` computes the same name for the Status page. Startup first runs `prepare_log_surface` (creates the folder, probes that a file can be created in it) and builds the file layer only when that passed. The builder offers `max_log_files(n)` and a fallible `build`; `rolling::daily` panics where `build` returns an error. | `main.rs:1330-1357`; `crates/livrarr-server/src/log_surface.rs:12-17`, `:23-55`; `tracing-appender-0.2.4/src/rolling.rs:154`, `:366-371`, `:195`; `rolling/builder.rs:233-238`, `:269-271` | — | read; name match probed P9; panic probed P18 |
| ST-011 | Pruning in `tracing-appender` 0.2.4 runs when the appender is built (startup) and when it opens a new day's file (rollover). It counts every regular file whose name starts with `livrarr.log` (so an undated `livrarr.log` or a `livrarr.log.bak` counts and can be deleted), ignores other names and directories, orders by file creation (birth) time, not by the date in the name, and deletes until `n − 1` remain before opening the new file. Files created newest-date-first (a copied folder) lose the newest dates first; a failed delete prints to stderr and continues; a read-only folder makes `build` fail ("failed to create initial log file"). | `rolling.rs:609-611`, `:645-726`, `:728-733`, `:779-794` | Promising calendar-day retention; promising that only dated files are touched. | probed P9 with `n = 30` (startup path; created()-based order confirmed on this filesystem, files 12 ms apart); rollover path read only: the builder has no clock control |
| ST-012 | (packet) Lint today fails: 5 errors (rule `react-hooks/exhaustive-deps` not defined) and 1 warning. With plugin `eslint-plugin-react-hooks` 7.1.1 and only `rules-of-hooks` (error) and `exhaustive-deps` (warn) on eslint 10.1.0 and typescript-eslint 8.58.0: 0 errors, 6 warnings (the four spots and two unused disable directives); `--max-warnings 0` turns that into exit 1. The plugin's full flat preset: 28 errors, 14 warnings. The plugin needs node 18 or later and supports eslint 10. Nothing runs the frontend lint: not the Dockerfile (its build is `tsc` plus Vite), not the scripts or workflows, and `verify.py`'s lint command is clippy. As built: the range saved is `~7.1.1` (lockfile resolves 7.1.1), installed with the project's pinned pnpm 10 from inside `frontend/`; from the repository root the host's pnpm 11 refused with `ERR_PNPM_UNEXPECTED_STORE` and changed nothing (`packet-4-code/HANDBACK-C2.md`; `frontend/package.json:54` at `25ca4f30`). | `frontend/package.json:10`, `:44-61`; `frontend/eslint.config.js:4-19`; `Dockerfile:14`; `~/Projects/kk-build/config.yaml:9` | Turning on the full preset (not approved). | probed P1 to P4, P16 |
| ST-013 | `Header.tsx:31-40`: the click-outside effect reads `mobileSearchOpen` but lists only `langOpen`. On a phone: open search, open the language list, close search with X; `langOpen` stays true and no listener is attached, so reopening search shows the language list already open. With `mobileSearchOpen` listed, closing search attaches the listener, and the next tap (on the search button) closes the list first. | `frontend/src/components/Header/Header.tsx:19-22`, `:31-40`, `:88-90`, `:140-150`, `:166-172`, `:178-196`, `:205-210` | — | read; unprobed: the fix is the test's subject |
| ST-014 | `NotificationBell.tsx:82-138` omits `dismiss` and `setRpmHighlight`. Both callbacks it uses are stable: `dismiss.mutate` is a `useCallback` on an observer held in state, and `setRpmHighlight` is a store action created once. The `dismiss` object itself changes every render, so listing it would rerun the effect each render. No stale data. | `NotificationBell.tsx:57`, `:106`, `:110`, `:162-170`; `frontend/node_modules/.pnpm/@tanstack+react-query@5.96.0_react@19.2.4/node_modules/@tanstack/react-query/src/useMutation.ts:30-36`, `:52-59`, `:68`; `frontend/src/stores/ui.ts:88` | — | read |
| ST-015 | `EpubReader.tsx:257-279` omits `libraryItemId`; `openingId` changes in the same render whenever `libraryItemId` changes, and a comment at `:278` says so. No stale data. | `frontend/src/pages/reader/EpubReader.tsx:104-107`, `:257-279` | — | read |
| ST-016 | `useStreamUrl.ts:50-90` omits `audioRef`; its only caller passes a `useRef` object, whose identity never changes, and a comment at `:88-89` says so. No stale data. | `frontend/src/pages/reader/useStreamUrl.ts:34-39`, `:50-90`; `frontend/src/pages/reader/AudioPlayer.tsx:109`, `:239-244` | — | read |
| ST-017 | Two disable comments suppress nothing: `AppLayout.tsx:35` (an effect with no dependency list) and `EpubReader.tsx:3` (`no-explicit-any` is off in the config). There are five `exhaustive-deps` disables in all, including `AppLayout.tsx:35`; the other four are still needed (not reported unused): `AppLayout.tsx:26`, `Sidebar.tsx:534`, `LogsPage.tsx:84`, `WorksPage.tsx:359`. (v1 to v3 said "the other five"; corrected in v4 from the red-test hand-back `packet-1-red-tests/HANDBACK-T3.md`.) | `frontend/src/components/Page/AppLayout.tsx:28-35`; `EpubReader.tsx:3-4`; `eslint.config.js:9`; `git grep eslint-disable` over `frontend/src` at `c05752d8` and `25ca4f30` | — | probed P2; count re-read at `25ca4f30` |
| ST-018 | (packet) CI runs only on pull requests and by hand; it builds the Docker image for amd64 and arm64 without pushing. Last run 2026-07-08, by hand. GitHub skips a push only when every changed path matches `paths-ignore`; a push to an existing branch is judged by the two-dot diff between the old and new branch head, so several commits count together. `*.md` matches root files only; `docs/**` matches the folder and its subfolders. Over 1,000 commits or a diff timeout always runs; over 3,000 files can skip. The filter sits per event. Since 2026-09-22, 40 commits reached main; 24 touch only `docs/`, `wiki/`, `build/` or root Markdown. | `.github/workflows/ci.yml:3-38`; `gh run list --workflow ci.yml`; GitHub workflow-syntax documentation | Ignoring a path the image is built from. | P7 (documentation read), P6c (commit count); the trigger itself is unprobed: it needs a push |
| ST-019 | The image is built from `frontend/`, `Cargo.toml`, `Cargo.lock`, `.cargo/`, `crates/` and `docker/entrypoint.sh` only. No Markdown file is tracked under `crates/` or `frontend/`, no crate includes a `.md` file, and the web app imports none. | `Dockerfile:10-14`, `:40-42`, `:63-65`, `:96-97`; `.dockerignore:26-28` | — | probed P5 (searches with known-hit controls) |
| ST-020 | (packet) The router drops every `trusted_proxies` entry `IpNet::parse` rejects, silently. The parser does not trim, rejects an empty string, a host name, a port and a missing or non-numeric prefix, but accepts any prefix up to 255: `10.0.0.0/33` becomes a single-address match (it contains 10.0.0.0 and not 10.0.0.1) and `fd00::/129` likewise. A trusted peer's `X-Real-IP` picks the rate-limit key; an untrusted peer is keyed by its own address, so every user behind an untrusted proxy shares the proxy's address in the login, setup and global rate limits. | `crates/livrarr-server/src/router.rs:22-29`, `:31-55`; `crates/livrarr-server/src/rate_limit.rs:44-54`, `:22-42`, `:85-109` | — | probed P12 (every shape in the packet list) |
| ST-021 | A `/0` range (`0.0.0.0/0`, `::/0`) parses, and `contains` then shifts by the full width: a debug build panics ("attempt to shift left with overflow"); the release profile has no overflow checks, so by Rust's wrapping rule it would match only the network address itself. | `rate_limit.rs:28`, `:37`; `Cargo.toml:77-81` | — | debug panic probed P12; release behaviour read, unprobed (needs a release build; REQ-603 removes the shift for `/0`) |
| ST-022 | Unknown keys are names only, never values: an unknown root key or table as `key`, an unknown key in a known section as `section.key`, in sorted order. They are logged at WARN after logging starts, as "Unknown config key: {key}"; tracked tests look for that prefix. The log cleanser masks a secret-shaped key name (a quoted key written like a URL query that carries an API key, the shape the existing log test uses at `test_fresh_author_index.rs:2388-2392`; P13 shows the value part replaced by `[REDACTED]`) and is idempotent. | `config.rs:366-407`; `main.rs:721-735`, `:1255-1278`; `crates/livrarr-server/tests/test_fresh_author_index.rs:2385-2443`, `:2412` | Showing any value from the file. | probed P13 |
| ST-023 | Signature changes and their callers. The design changes one generic bound: `system::health` from `S: Clone + Send + Sync` to `S: HasDatabaseHealth`. The compiler's only error site is the unused `system::routes` (`system.rs:263`), which needs the same bound. Everything else is added (a domain trait and its `SqliteDb` impl, two capability traits and their `AppState` impls, a handler, a route, functions in `config.rs`, `rate_limit.rs` and `log_surface.rs`, a test-support function in `livrarr-db`'s `test_helpers`), or moved without a signature change (`load_config` from `main.rs` to `config.rs`). `IpNet::contains` keeps its signature. `AppConfig` gains a field skipped by serde; nothing builds it with a struct literal, so its eight `AppState` construction sites are untouched. **As built (`25ca4f30`), two differences, both accepted by the PM:** (1) `system::health` also changed its return type, from `Result<Json<…>, ApiError>` to `(StatusCode, Json<Vec<HealthCheckResult>>)` (`crates/livrarr-handlers/src/system.rs:61-63`), because `ApiError` cannot send a 503 whose body is REQ-101's item list; nothing else calls it. (2) `load_config` kept its return type `Result<(AppConfig, Vec<String>), String>` (`crates/livrarr-server/src/config.rs:422`) while `AppConfig` also carries the keys in `unknown_keys` (`config.rs:35-36`, set at `:444`), so the keys exist twice; `main.rs:721` ignores the tuple copy. `system::routes` took the bound at `system.rs:327`. | stub changes in a scratch worktree; as-built differences from `packet-4-code/HANDBACK-C1.md` | — | probed P14 (`cargo check --workspace --all-targets`, two runs); the v2 additions add functions only; as built read at `25ca4f30` |
| ST-024 | Test doors. Server: `router.rs`'s test module builds the real `AppState` and `build_router` over the real `SqliteDb` (`create_test_db`, one in-memory connection), signs in an owner and normal users and varies the peer address; `AppState.config` is a public field. Logs: `log_surface.rs` has a test module. Binary: `test_fresh_author_index.rs` writes `config.toml` into a scratch data folder, starts the real binary with `--data`, waits until `GET /api/v1/health` answers 200, creates the first admin through `/setup`, calls the API with that token, and reads stdout and stderr. `livrarr-server` already has `livrarr-db` with `test-helpers` as a dev-dependency. Web: `installApiStub` replaces only `fetch`; `mountWith` mounts one page; `Header.test.tsx` dispatches DOM events. | `router.rs:740-775`, `:930-936`, `:1219-1224`, `:1354`, `:1405-1436`; `crates/livrarr-db/src/lib.rs:82-97`; `log_surface.rs:58-89`; `test_fresh_author_index.rs:63-65`, `:96-117`, `:134-160`, `:239`, `:2132`; `crates/livrarr-server/Cargo.toml:66`; `frontend/src/test-support/apiStub.tsx:55`, `:105`; `frontend/src/components/Header/Header.test.tsx:162` | A toy router, an injected outcome, or hand-built state where the real door exists. | read |
| ST-025 | The web app's query client refetches every query on window focus and retries once. System → Status's three queries each render their own data; config warnings are fixed for the life of the process. | `frontend/src/App.tsx:71-80`; `StatusPage.tsx:172-205` | — | read |
| ST-026 | A health-read failure with a connection available can be produced in the real SQLite engine. `livrarr-db` depends on `libsqlite3-sys` 0.30; sqlx 0.8.6 exposes the raw connection (`lock_handle`, `as_raw_handle`). An authorizer installed with `sqlite3_set_authorizer` on the test database's one connection, refusing `SQLITE_READ` of the schema table, makes `SELECT count(*) FROM sqlite_schema` fail with "(code: 23) not authorized", also when that statement was prepared and cached before (SQLite re-prepares it). The authorizer reports the table as `sqlite_schema` with an empty column name for `count(*)`. While it is installed, the pool still hands out the connection, and the real `get_session`, `get_user` and `get_user_by_api_key_hash` (the login checks) succeed. Removing the authorizer makes the read succeed again. | `crates/livrarr-db/Cargo.toml:20`; `sqlx-sqlite-0.8.6/src/connection/mod.rs:64`, `:198`, `:378`; `crates/livrarr-server/src/middleware.rs:29-38`, `:65-73`; `crates/livrarr-db/src/sqlite_session.rs:31`; `crates/livrarr-db/src/sqlite_user.rs:55`, `:73` | A failure fixture that replaces the check's outcome; a check that only acquires a connection or reads no table. | probed P17 (real `create_test_db`, real session and user reads; at the database layer, because the routes do not exist yet) |
| ST-027 | With `max_log_files(31)` and seeded files dated the days before today, created oldest first 12 ms apart: no folder → 1 file (today's); empty folder → 1; 29 → 30 (all kept); 30 → 31 (all kept); 31 → 31 (oldest deleted); 85 → 31 (the 30 newest kept); 40 files on alternate days → 31 (today's and the 30 newest); today's file plus 35 earlier (same-day restart) → 30 (today's and the 29 newest). With `livrarr.log` created first, then `notes.txt`, `livrarr.log.bak`, a directory named `livrarr.log.2020-01-01`, then 30 dated files: 33 entries remain; `livrarr.log` and `livrarr.log.bak` are deleted; `notes.txt`, the directory and all 30 dated files stay. A directory named as today's log file makes `build` fail in a writable folder with "failed to create initial log file: Is a directory (os error 21)"; `prepare_log_surface` does not catch this, and `rolling::daily` on the same folder panics ("initializing rolling file appender failed"). | `tracing-appender-0.2.4/src/rolling.rs:154`; `log_surface.rs:23-56` | — | probed P18 (real builder, real folders, real UTC date) |
| ST-028 | The log init error reaches `GET /api/v1/system/status` as `logInitError` and stderr only. The web type `SystemStatus` has no such field and System → Status does not show it; the startup comment calls the page display "later". | `crates/livrarr-handlers/src/system.rs:42-55`; `crates/livrarr-handlers/src/types/system.rs:14-16`, `:23-29`; `log_surface.rs:48-50`; `main.rs:1331-1333`; `frontend/src/types/api.ts:1271-1280`; `StatusPage.tsx:208-285` | Promising the Status page shows the log init error (not approved). | read |
| ST-029 | The compose example, its copy in `docs/llm-context.md` and the Unraid template all select `ghcr.io/kkodecs/livrarr:0.1.0-alpha6`. That release's Dockerfile has no `HEALTHCHECK`. A change to main's Dockerfile reaches those users only through a later release image. | `docker-compose.yml:3`; `docs/llm-context.md:208`; `docker/unraid-template.xml:10`; `git show v0.1.0-alpha6:Dockerfile` (no `HEALTHCHECK` line) | Removing the compose health check while the example selects alpha6; changing an image tag in this feature. | read |

## 0c. Prior Art

Searched: work-plan chunk 5, the to-do list's Operations rows (both in `build/plans/`),
`build/reports/code-health-2026-09-26.md`, `spec-prerelease-trust-pass.md`,
`spec-security-before-release.md`, `build/foundation/security-model-policy.md`, `CHANGELOG.md`,
`docker-compose.yml`, `docker/unraid-template.xml`, `README.md`, `wiki/insights.md`, and
`git grep -il` over `docs/` and `wiki/` for "health", "trusted_proxies", "log", "eslint" and "CI".

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `build/plans/todo-chunks-2026-10-04.md:90-102` (chunk 5) and the Operations section of the to-do list in `build/plans/` (lines 213-235, 286). | Source of the items. The to-do list says the last CI run was 2026-07-18; `gh` shows 2026-07-08 (ST-018). The grounding file counts 10 of 40 recent commits touching code; with REQ-501's patterns 16 would run CI, because changes under `.claude/`, `tests/` and `scripts/` count as code here (P6c). The orphaned-process row is not one of the seven decisions and is not specified here. |
| PA-002 | `build/reports/code-health-2026-09-26.md:9-24`, `:49-67`: lint red since April; fix by installing the plugin or dropping the comments. | Same finding as ST-012; the PO chose installing the two classic rules. |
| PA-003 | `spec-prerelease-trust-pass.md:176` (ST-205): unknown-key warnings were dropped until that feature moved them after logging starts and fixed the TOML parse. | REQ-701 builds on that loader; the log text "Unknown config key:" stays, because its tests look for it (ST-022). |
| PA-004 | `spec-security-before-release.md` and `wiki/patterns/log-redaction.md:6`, `:23`: every log sink passes through `cleanse_log_line`. | REQ-602 passes each warning through the same cleanser before it reaches the page, so the page and the log show the same text (ST-022). |
| PA-005 | `CHANGELOG.md:699` ("M6: … wget healthcheck") and `docker-compose.yml:27-32`; the same block in `docs/llm-context.md:232-237`. | The compose check is the precedent for REQ-201's command. REQ-202 keeps it unchanged, because the example selects an image without a built-in check (ST-029). |
| PA-006 | `build/foundation/security-model-policy.md:33` (health is pre-auth, `{"status":"ok"}` only) and `:184` (split health: public minimal, admin detailed); `wiki/decisions/key-decisions.md:57` (only login, setup and health are anonymous). | Basis for D-101: the public reply stays minimal, detail moves to an admin route. Today's public reply is a one-item list, not `{"status":"ok"}`; the list shape is kept for existing monitors and noted here, not changed. |
| PA-007 | `README.md:92-101` (config example names port and bind address, "map externally in compose"); `docs/llm-context.md:175-190` (config keys, `trusted_proxies`, `url_base` has no effect), `:244` (daily log file), `:379` (endpoint table). | Doc lines REQ-202, REQ-301 and REQ-602 update. |
| PA-008 | `docker/unraid-template.xml:1-36`: no health settings; caps in `ExtraParams`; image `0.1.0-alpha6`. | Unraid needs no file change; it gets the image check with the first release image that carries it (ST-029). |
| PA-009 | `wiki/insights.md` lessons 75 (`wiki/insights/process.md:108-117`: run the typecheck; read the real CI conclusion) and 83 (reliable tracing capture). | Every web change runs the typecheck; REQ-501 is checked by reading run conclusions with `gh`, not by a wrapper's output. |
| PA-010 | `wiki/deployment/container-permissions.md:47`: a real-image check run as uid 99 got health 200. | Precedent for REQ-201's real-image verification. |

## 1. Problem Statement

1. **Docker, or an uptime monitor, polls `/api/v1/health` while the database cannot answer.**
   Expected: a failure status. Observed: 200 "database is reachable" every time, because the
   handler never reads the database (ST-001).
2. **An admin runs the image without our compose file** (Unraid, `docker run`, their own compose).
   Expected: Docker shows healthy or unhealthy. Observed: no health status at all (ST-008).
3. **Livrarr runs for months.** Expected: old log files go away. Observed: one file per day forever;
   the dev box has 85 files, 149 MB (packet; ST-010).
4. **A developer runs `pnpm -C frontend lint`.** Expected: a working check. Observed: 5 errors for a
   rule that is not installed; real missing dependencies are invisible (ST-012). One of them is a
   visible phone bug: open search, open the language list, close search, reopen it, and the
   language list is already open (ST-013).
5. **The maintainer pushes code straight to main.** Expected: the Docker build is checked. Observed:
   CI runs only on pull requests and by hand; the last run was 2026-07-08 (ST-018).
6. **An admin who wrote `nginx` in `trusted_proxies` restarts Livrarr and opens System → Status.**
   Expected: told the entry was ignored. Observed: nothing anywhere; every user behind the proxy now
   shares one rate-limit bucket (ST-020).
7. **An admin who misspelled a key (say `[server] prot = 9000`) restarts and opens System → Status.**
   Expected: told the key does nothing. Observed: one WARN line in the log only (ST-022).
8. **An admin who wrote `0.0.0.0/0` in `trusted_proxies` to trust every proxy runs a development
   build and logs in through a proxy that sends `X-Real-IP`.** Expected: the login is
   rate-limited by the `X-Real-IP` address. Observed: the request handler panics (ST-021).

## 2. Requirements

"Signed-out", "normal user" and "admin" are the three callers. Each requirement names its input
shapes and its test door. Racing replies: System → Status keeps its three replies and the rule that
the page waits for all three; only the health list's source changes, from one reply to another
single reply, and its content is fixed for the process lifetime, so no new ordering exists
(ST-025). Not applicable beyond that.

"Schema reads refused" below names the failure fixture of ST-026: a test-support function in
`livrarr-db`'s `test_helpers` installs, and later removes, a SQLite authorizer on the test
database's one connection that refuses reads of the schema table. Logins keep working under it.

- **REQ-101** (item 1, public health). `GET /api/v1/health` runs the shared database check
  (REQ-103). Success: 200 with today's body, one item `{"source":"database","checkType":"ok","message":"database is reachable"}`.
  Failure: 503 with one item `{"source":"database","checkType":"error","message":"database check failed"}`
  and nothing else: no error text, path or config warning (D-102). Each failure is logged at WARN
  with the detail. HEAD answers the same status with no body (ST-007). The route stays public.
  - Shapes: database fine; schema reads refused with a connection available; pool closed; check
    slower than the bound (every connection held); caller signed-out, normal user, admin (same
    reply for all); GET and HEAD.
  - Door: the real `build_router` over the real `SqliteDb` in `router.rs`'s test module.
- **REQ-102** (items 1, 6, 7, the Status page's list). New `GET /api/v1/system/health`, admin only
  (`RequireAdmin`). Reply: a list whose first item is the database check (REQ-103) and whose
  further items are the config warnings (REQ-602), each `{"source":"config","checkType":"warning","message":…}`.
  On a check failure the reply is still 200; its database item is `error` with the message
  "database check failed: {detail}" (cleansed), and the config warnings still follow. System →
  Status reads this route instead of `/health` for its Health Checks list; everything else on the
  page is unchanged. `HealthPage.tsx` stays unrouted and unchanged (packet).
  - What an admin sees when the database check fails: if the database cannot answer at all, the
    login check fails first and the page shows its error with Retry, as today (ST-003); if the
    login check gets through and the check then fails (a read SQLite refuses, or a pool too busy
    for 2 seconds), the list shows a red `error` row and the rest of the page.
  - Shapes: admin by session and by API key; normal user (403); signed-out (401); no warnings;
    warnings present; database item ok and error (schema reads refused).
  - Door: server, the real router with real authentication; page, the real `StatusPage` through
    `mountWith` with `fetch` stubbed.
- **REQ-103** (item 1, the one check). One handler-side function runs the domain
  `check_database` under a 2-second timeout and returns the database item. `check_database` runs
  a read of the schema table through the live pool (`SELECT count(*) FROM sqlite_schema`); getting
  a connection alone, or a statement that reads no table, does not satisfy it. Both routes call the
  function. A timeout counts as a failure with the detail "did not answer within 2 seconds". What
  it detects and does not detect is ST-005; the spec promises nothing more.
  - Door: REQ-101's and REQ-102's doors, with schema reads refused for the error case (D-104).
    The closed-pool detail is also driven by calling the function on the real `AppState`, because
    no request can pass the login check on a closed pool (ST-003); that criterion is supplemental.
- **REQ-201** (item 2, image check). The runtime stage gets
  `HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["wget","-q","-T","4","--spider","http://127.0.0.1:8789/api/v1/health"]`
  (exec form, no shell). Bounds nest: server check 2 s, `wget` 4 s, Docker 5 s (ST-006). The 60 s
  start period covers migrations and the pre-upgrade copy before the listener opens. Users of
  Unraid and plain `docker run` get the check with the first release image built after this
  change; this feature changes no image tag (ST-029).
  - Shapes: default port and bind address (healthy); still starting (`starting`); `url_base` set
    (healthy, ST-009); `bind_address = "127.0.0.1"` (healthy); `port` changed (unhealthy;
    documented, D-001); `bind_address` set to one non-loopback address (unhealthy; documented,
    D-001; no criterion, D-202); database failing (unhealthy, by REQ-101 and ST-006; not driven in
    a container).
  - Door: a real local image build and container runs (no push), `docker inspect` for status.
    Cost: one build (about 10 to 30 minutes on this box, estimated from CI's 8 to 9 minutes; not
    measured here) and five short runs.
- **REQ-202** (item 2, the other copies). `docker-compose.yml` and the compose example in
  `docs/llm-context.md` keep their `image:` line and `healthcheck:` block unchanged: they select
  alpha6, which has no built-in check (ST-029), so removing the block would leave their users with
  no health status (D-201). The compose check overrides the image's check and tests the same
  address. Add one sentence to the README's configuration section and to `docs/llm-context.md`:
  an admin who changes `port`, or sets `bind_address` to one specific address, inside the
  container should override the health check. The Unraid template is unchanged (PA-008).
- **REQ-301** (item 3, log pruning). Build the file appender with the builder: daily rotation,
  prefix `livrarr.log`, `max_log_files(31)`, in one new `log_surface` function that returns the
  appender or the build error (as built: `log_surface::build_file_appender`); `main.rs` calls it in place of `rolling::daily`
  (`main.rs:1338`), so startup and the tests use the same constructor. The constant is fixed in
  code (not a setting). How "30 days" maps (D-003, D-301): after each change of day the folder
  holds today's file and the 30 most recent earlier log files; after a restart later the same day,
  today's and 29 earlier. Days on which Livrarr did not run leave no file and use none of the 30,
  so the oldest kept file can be older than 30 calendar days. Pruning follows file creation time
  and counts every file whose name starts with `livrarr.log`, so an old undated `livrarr.log` is
  deleted like a dated one (ST-011, ST-027); the docs line at `docs/llm-context.md:244` says so in
  one sentence. As built, that sentence (`docs/llm-context.md:245` at `25ca4f30`, after the
  code-review P3 fold) reads: "After each change of day Livrarr keeps today's log file and the 30
  most recent earlier files, and after a restart later the same day it keeps today's file and the
  29 most recent earlier ones, ordered by file creation time and counting every file whose name
  starts with `livrarr.log` (so an old undated `livrarr.log` is deleted like a dated one); days
  Livrarr did not run leave no file and use none of the 30". If `build` fails, the server starts without the file layer, prints the error to
  stderr and records it in the existing `logInitError` field of `GET /api/v1/system/status`, as a
  log-folder failure is handled today (`log_surface.rs:48-50`), instead of panicking. The Status
  page does not show that field today (ST-028); showing it is not part of this feature.
  - Shapes: no log folder; empty folder; 29, 30, 31 and 85 dated files; an undated `livrarr.log`
    and other files (`livrarr.log.bak`, `notes.txt`, a directory); days not run (gaps); same-day
    restart; `build` failing in a writable folder (a directory named as today's file) and in a
    read-only folder; the active file name; startup through the real binary.
  - Not driven: pruning at rollover (no clock control; library source `rolling.rs:728-733`); one
    undeletable file in a writable folder (needs root to set up; the library prints to stderr and
    continues, `rolling.rs:717-724`).
  - Doors: the file matrix through the real `log_surface` function on real temporary folders,
    files created oldest first at least 12 ms apart (ST-027); startup wiring and the build failure
    through the real binary in `test_fresh_author_index.rs` (ST-024).
- **REQ-401** (item 4, the plugin). Add `eslint-plugin-react-hooks` (7.1.x) to `devDependencies`
  (as built: `~7.1.1`, ST-012)
  and register it in `eslint.config.js` with exactly `react-hooks/rules-of-hooks: "error"` and
  `react-hooks/exhaustive-deps: "warn"`; no preset. Change the `lint` script to
  `eslint src/ --max-warnings 0` (D-401). Nothing else runs lint today and nothing is added to run
  it (ST-012; not approved for CI).
- **REQ-402** (item 4, `Header.tsx:40`, a real bug). List `mobileSearchOpen` in the effect's
  dependencies. Visible result: on a phone, opening search, opening the language list, closing
  search with X and reopening search shows the language list closed. Today it shows it open
  (ST-013). Door: the real `Header` in `Header.test.tsx`, with the language route stubbed to two
  or more languages and a tap dispatched as `mousedown` then `click`.
- **REQ-403** (item 4, three deliberate omissions). At `NotificationBell.tsx:138`,
  `EpubReader.tsx:279` and `useStreamUrl.ts:90` add
  `// eslint-disable-next-line react-hooks/exhaustive-deps -- {reason}` on the line before the
  dependency list, with these reasons: "dismiss.mutate and setRpmHighlight keep their identity;
  rerun only when the unread list changes" (ST-014); "libraryItemId changes only together with
  openingId" (ST-015, replacing the comment at `:278`); "audioRef is a ref object; libraryItemId is
  the only real dependency" (ST-016, replacing `:88-89`). No behaviour changes.
- **REQ-404** (item 4, stale comments). Remove the disable comments at `AppLayout.tsx:35` and
  `EpubReader.tsx:3` (the `type Rendition = any` line stays). The other four `exhaustive-deps`
  disables stay (ST-017; v3 said five).
- **REQ-501** (item 5, CI trigger). Add a `push` trigger for `main` with
  `paths-ignore: ['docs/**', 'wiki/**', 'build/**', '*.md']`. `pull_request` and
  `workflow_dispatch` stay as they are, with no path filter; the job is unchanged. None of the four
  patterns matches an image input (ST-019), and `.github/workflows/ci.yml` matches none, so a
  workflow-only push runs CI.
  - Shapes: code-only push (runs); docs-only push (skipped); mixed push (runs); workflow-only push
    (runs); a push of several commits that together touch code (runs, ST-018); pull request (runs,
    unchanged); manual run (runs, unchanged).
  - Door: before the push, the workflow file parsed and its `on:` block compared; after, the
    first pushes read with `gh run list` (a push needs the PO's word). No workflow linter is
    installed here; `actionlint` in a scratch folder is optional.
- **REQ-601** (item 6, the parser). `IpNet::parse` trims surrounding spaces and rejects a prefix
  above 32 (IPv4) or 128 (IPv6); every other accepted shape is unchanged. A new
  `parse_trusted_proxies(entries) -> (accepted, rejected entries)` in `rate_limit.rs` is used by
  the router (replacing `router.rs:22-28`) and by the warnings (REQ-602), so the trust decision and
  the warning come from one rule and one loaded config. Duplicates are accepted without a warning;
  an empty list gives no warning.
  - Shape results: `""` rejected; `" 10.0.0.5 "` accepted (D-602); `10.0.0.1`, `10.0.0.0/8`,
    `::1`, `fd00::/8` accepted; `0.0.0.0/0` and `::/0` accepted (REQ-603); `10.0.0.0/33`,
    `fd00::/129`, `10.0.0.0/`, `nginx`, `10.0.0.1:443`, `[::1]:443` rejected; a duplicate accepted;
    the default empty list accepted.
  - Behaviour change on other doors: `10.0.0.0/33` and `fd00::/129` used to trust their network
    address alone and now trust nothing; a padded entry used to trust nothing and now trusts its
    address.
- **REQ-602** (items 6 and 7, where warnings live). `load_config` moves from `main.rs` into
  `config.rs` unchanged in behaviour and records the unknown keys in `AppConfig` (a field serde
  skips). One function, `config_warnings(&AppConfig) -> Vec<String>`, produces every message: first
  each rejected proxy entry, in file order, as
  `Ignored [server] trusted_proxies entry "{entry}": use an IP address or range such as 172.18.0.0/16; host names and ports are not supported`,
  then each unknown key, in the loader's sorted order, as `Unknown config key: {key}`; each message
  is passed through `cleanse_log_line`. `main.rs` logs each at WARN once at startup (replacing
  `main.rs:733-735`). The admin route (REQ-102) reads them through `HasConfigWarnings`, which
  `AppState` implements by calling the same function on `AppState.config`. Startup never stops for
  them. The warning stays until the file is fixed and Livrarr restarts; the page says nothing about
  restarting (no new text approved).
  - Door: `config.toml` written to a temporary folder, read by the real `load_config`, placed in the
    real `AppState.config`, then the real router; log lines through the real binary in
    `test_fresh_author_index.rs`.
- **REQ-603** (item 6, `/0` ranges; PO, D-002). `IpNet::contains` returns true for every address
  of the range's own kind when the prefix is 0: `0.0.0.0/0` matches every IPv4 address and `::/0`
  every IPv6 address. Other prefixes keep today's rule; prefixes above 32 or 128 never reach
  `contains`, because REQ-601 rejects them. A peer of the other kind is not matched. `/0` entries
  are accepted without a warning. How headers are read is unchanged.
  - Behaviour change: today a debug build panics on such an entry and a release build would, by
    reading, trust only the network address (ST-021). After the fix every peer of that kind is
    trusted, and the existing key rule applies (`crates/livrarr-server/src/rate_limit.rs:93-112`):
    a valid `X-Real-IP` is the rate-limit key (`:98-104`); without it, the `X-Forwarded-For` chain
    is walked from the right and its first untrusted address is the key (`:68-79`, `:106-110`).
    Under a `/0` entry every address of that kind in the chain is trusted, so an all-same-kind
    chain falls back to the peer's own address (`:78`). So a proxy that sends `X-Real-IP` gets
    per-client buckets; one that sends only `X-Forwarded-For` does not. Changing that walk is not
    part of this feature.
  - Shapes: IPv4-only `/0` with an IPv4 peer (trusted) and an IPv6 peer (untrusted); IPv6-only
    `/0` likewise, reversed; `X-Real-IP` present; only `X-Forwarded-For`, all of the entry's kind.
  - Door: the real loader and the real router, as REQ-601, with a fresh router per sequence.
- **REQ-701** (item 7, unknown keys). Every unknown key `unknown_keys` reports gets an amber row:
  an unknown top-level table, a top-level key, a key inside a known table. Never the value: the
  messages are built from key names alone (D-701).

### Build files and sizes

| REQ | Production files | Test files | Size |
|-----|------------------|------------|------|
| REQ-101, REQ-103 | `crates/livrarr-domain/src/services/mod.rs`, new `services/database_health.rs`; `crates/livrarr-db/src/lib.rs` (also the test-support authorizer function in `test_helpers`, ST-026), new `sqlite_database_health.rs`; `crates/livrarr-handlers/src/context.rs`, `system.rs`; `crates/livrarr-server/src/state.rs`, `router.rs`; `docs/llm-context.md` (endpoint table) | `router.rs` test module (tracked) | S, 4 h |
| REQ-102 | `system.rs`, `context.rs`, `state.rs`, `router.rs`; `frontend/src/api/index.ts`, `frontend/src/pages/system/status/StatusPage.tsx` | `router.rs` test module; new `frontend/src/pages/system/status/StatusPage.test.tsx` (not ignored) | S, 2.5 h |
| REQ-201, REQ-202 | `Dockerfile`, `README.md`, `docs/llm-context.md` (one sentence each; compose blocks untouched) | real image run (no file) | S, 1 h plus the build |
| REQ-301 | `crates/livrarr-server/src/log_surface.rs`, `main.rs`, `docs/llm-context.md` | `log_surface.rs` test module; `test_fresh_author_index.rs` (tracked) | S, 2.5 h |
| REQ-401 to REQ-404 | `frontend/package.json`, `frontend/pnpm-lock.yaml`, `frontend/eslint.config.js`, `Header.tsx`, `NotificationBell.tsx`, `EpubReader.tsx`, `useStreamUrl.ts`, `AppLayout.tsx` | `Header.test.tsx` (tracked) | S, 2 h |
| REQ-501 | `.github/workflows/ci.yml` | after-push observation | S, 0.5 h |
| REQ-601 to REQ-603, REQ-701 | `rate_limit.rs`, `router.rs`, `config.rs`, `main.rs`, `state.rs`, `context.rs`, `system.rs`, `docs/llm-context.md` (`trusted_proxies` line) | `router.rs` test module; `test_fresh_author_index.rs` (tracked) | M, 5 h |

**Overlap.** `router.rs`, `state.rs`, `context.rs` and `system.rs` are edited by REQ-101/102 and
REQ-602; `main.rs` by REQ-301 and REQ-602; `config.rs` by REQ-602 and REQ-701; `rate_limit.rs` by
REQ-601 and REQ-603; `test_fresh_author_index.rs` by REQ-301 and REQ-602; `StatusPage.tsx` by
REQ-102 only, but it shows REQ-602's rows; `docs/llm-context.md` by REQ-101, REQ-202, REQ-301 and
REQ-602. One coder, server items in the order 103, 101, 602, 102, 301. New files under `tests/`
are ignored by git, so server cases go in the tracked test modules above.

## 3. UI/Interface Design

No new page. System → Status shows the same Health Checks list with new rows: a red `error`
database row (REQ-102) and amber `warning` rows with source "config" (REQ-602). API: new
`GET /api/v1/system/health` (admin) → `200 [ {source, checkType, message}, … ]`; `GET /api/v1/health`
may now answer `503`. Image: a `HEALTHCHECK`. New log lines: the proxy warning and the database
check failure.

## 4. Non-Requirements

- No retention setting; no pop-up for config warnings; no refusal to start on a bad proxy entry; no
  config warnings in the sidebar; no route for `HealthPage.tsx`; no lint, tests or anything but the
  Docker build in CI; not the plugin's full preset (all not approved, packet).
- No display of the log init error on System → Status (not approved; ST-028).
- No image tag change in `docker-compose.yml`, `docs/llm-context.md` or the Unraid template, and no
  removal of the compose health check (D-201).
- No use of `[server] url_base` (roadmap item, ST-009); no handling of IPv4 peers seen as
  IPv6-mapped addresses; no CI concurrency settings.
- The public health reply keeps its list shape rather than the policy's `{"status":"ok"}` (PA-006).
- The orphaned test processes on the dev box (chunk 5's first row) are not part of this spec.

## 5. Open Questions

None. The three questions in version 1 were answered by the PO on 2026-10-06 and are recorded as
D-001 to D-003 in [§6](#6-decisions).

## 6. Decisions

| ID | Decision | Status | Resolution |
|----|----------|--------|------------|
| D-001 | (was Q-1) The image check after a port or bind change inside the container. | closed, PO 2026-10-06 | A fixed check on `127.0.0.1:8789`, plus a docs note telling an admin who moves the internal port, or binds to one specific address, to override it (REQ-201, REQ-202). No `livrarr health-check` command. |
| D-002 | (was Q-2) `/0` proxy ranges. | closed, PO 2026-10-06 | Fix: `/0` matches every address of its kind (REQ-603, AC-618 to AC-620). Consistent with REQ-601: prefixes above 32 or 128 are rejected and warned about, `/0` is a valid prefix and is accepted silently, and both decisions are pinned through the real router (AC-617, AC-618). |
| D-003 | (was Q-3) "30 days" for log files. | closed, PO 2026-10-06 | The library cap: today's file plus the 30 most recent earlier files (REQ-301). No calendar-age pruning. |
| D-101 | How config warnings reach the Health Checks list for admins only. | closed, spec | Options: (a) a new admin route feeding the list, public route unchanged in shape; (b) the public route adds warnings when the caller is an admin; (c) add warnings to the health summary. **(a)**: (b) puts an admin check inside a public route and makes the never rule depend on it; (c) is also read by the sidebar (not approved) and would make the list combine two replies. Matches the policy's planned split (PA-006). |
| D-102 | Never rule: config warnings and error detail never reach a caller without an admin session. | closed, spec | Declared: the public handler's only capability is the database check (`HasDatabaseHealth`), so it cannot reach warnings; its failure message is a fixed string. The admin route requires `RequireAdmin`. Pinned by AC-115, AC-116, AC-122 and AC-124. |
| D-103 | Check bound. | closed, spec | 2 s (ST-005), under `wget -T 4` and Docker's 5 s. |
| D-104 | How a test makes the health read fail while a connection is available and logins work. | closed, spec | A SQLite authorizer refusing reads of the schema table, installed on the test database's connection by a `livrarr-db` test-support function (ST-026). It is a real refusal by the real engine of the real statement, so a check that only acquires a connection, or reads no table, passes under it and fails the criterion. Rejected: changing the check to read a table a test could rename (the check would be shaped by its test); closing the pool (it also blocks login, ST-003). |
| D-201 | The compose health check. | closed, spec (review F1) | Kept unchanged while the example selects alpha6 (ST-029). Removing it becomes possible once the examples select a release image that carries REQ-201's check; that is a later change, not part of this feature. |
| D-202 | A criterion for `bind_address` set to one non-loopback address inside the container. | closed, spec (review F8) | Not applicable. The behaviour is the documented limit of D-001, and its requirement is the docs sentence pinned by AC-216. The only test is a real image run with a fixed container address on a user-defined network, and its mechanism (nothing listens on `127.0.0.1:8789`, so `wget` gets a refused connection and exits 1, ST-006) is the one AC-214 already drives with a changed port. |
| D-301 | `max_log_files` value. | closed, spec | 31: the library keeps `n − 1` before opening the new file (ST-011, ST-027), so 31 keeps today plus 30 earlier files after a day change and never fewer than 30 files once 30 exist. |
| D-401 | Lint severity. | closed, spec | `exhaustive-deps` stays a warning (the plugin's default) and the script fails on any warning, so unused disable directives (also warnings) fail too. Reason: nothing runs lint automatically (ST-012), so a passing exit code is the only signal. |
| D-402 | Deliberate omissions: disable comment or listing the stable values. | closed, packet | Disable comment with a reason, as the packet requires. |
| D-601 | Proxy warning text. | closed, spec | The REQ-602 text: it names the entry and says what works, including the host-name case. |
| D-602 | Padded entries. | closed, spec | Trim and accept: the meaning is unambiguous, and warning about spaces adds nothing the admin needs. |
| D-701 | Never rule: an unknown key's value is never shown. | closed, spec | Structural: `unknown_keys` returns names only (ST-022) and `config_warnings` never sees the table. A secret-shaped key name is masked by the cleanser. Pinned by AC-711 and AC-712. |

## 7. Acceptance Criteria

"Red" = fails on `c05752d8`; "guard" = passes there and must keep passing. As built, every box
below is ticked except AC-513, which waits on a push; two labels were wrong (AC-612, AC-616) and
are corrected in place. Server criteria use the
real `build_router` and `SqliteDb` from `router.rs`'s test module; config criteria write
`config.toml` to a temporary folder and set `AppState.config` from the real `load_config`. "Schema
reads refused" is the ST-026 fixture; criteria that use it also remove it and check recovery.

**Doors below the production entry, and why.** AC-123 calls the shared check directly, because a
closed pool also blocks login (ST-003); AC-124 covers the admin handler through the real door, so
AC-123 is supplemental. AC-311 to AC-316 call the real `log_surface` constructor for the file
matrix; AC-317 and AC-318 prove through the real binary that startup uses it. AC-412 and AC-413
read the lint configuration; AC-411 runs the real lint command. AC-511 and AC-512 read the
workflow file; AC-513 is the real door and needs a push, which needs the PO's word.

**Input-shape matrix.**

| Item | Shapes and criteria |
|---|---|
| Health | fine AC-111; schema reads refused AC-116, AC-124; pool closed AC-112; slower than bound AC-113; signed-out, normal, admin AC-111, AC-115, AC-116; GET and HEAD AC-114; admin route roles AC-121, AC-122; closed-pool detail AC-123 (supplemental); page AC-131 to AC-134 |
| Docker | default AC-212; starting AC-213; port changed AC-214; bind 127.0.0.1 and `url_base` AC-215; non-loopback bind: not applicable (D-202); config of the check AC-211; deployment copies and docs AC-216 |
| Logs | no folder, empty folder, 29, 30, 31, 85 AC-311; undated and other files AC-312; gaps AC-313; name AC-314; same-day restart AC-315; build failing AC-316; startup AC-317; startup with build failing AC-318; one undeletable file and rollover: not applicable, see REQ-301 |
| Lint | AC-411 to AC-414, AC-421, AC-422, AC-431, AC-441 |
| CI | code-only, docs-only, mixed, workflow-only, several commits, pull request, manual: AC-511 to AC-513 |
| Proxy entries | every shape in REQ-601 AC-611; startup AC-612; padded and host-name trust AC-613; over-wide ranges untrusted AC-617; `/0`, each kind alone with a same-kind and an other-kind peer AC-618, AC-619; `/0` with only `X-Forwarded-For` AC-620; log AC-614; restart AC-615; never AC-115, AC-616 |
| Unknown keys | unknown table, key in known table, root key, secret-looking value AC-711; secret-shaped name AC-712; log text AC-713; none AC-714 |

**REQ-101, REQ-103** (real router):
- [x] **AC-111** (guard): signed-out, a normal user and an admin each get 200 from
  `GET /api/v1/health` with exactly one item, `database` / `ok`.
- [x] **AC-112** (red): with the state's pool closed before the request, a signed-out GET gets 503
  and one item whose `checkType` is `error` and whose `message` is exactly "database check failed".
- [x] **AC-113** (red): with the test holding the pool's only connection, a signed-out GET gets 503
  and the reply arrives in under 3 seconds.
- [x] **AC-114** (red): HEAD gets the same status as GET (200 healthy, 503 with the pool closed)
  and an empty body.
- [x] **AC-115** (guard, never rule): with `config.toml` holding `trusted_proxies = ["nginx-marker-615"]`
  and an unknown key `zz_marker_615 = 1`, the signed-out, normal-user and admin replies from
  `/api/v1/health` contain neither marker and have one item.
- [x] **AC-116** (red): with schema reads refused, a signed-out GET gets 503 and a body that is
  exactly one item, `{"source":"database","checkType":"error","message":"database check failed"}`;
  after the fixture is removed, the same GET gets 200 with `database` / `ok`.

**REQ-102** (real router; real `StatusPage`):
- [x] **AC-121** (red): an admin, by session and by API key, gets 200 from
  `GET /api/v1/system/health`; the first item is `database` / `ok`.
- [x] **AC-122** (red): a normal user gets 403 and a signed-out caller 401. Today the route does
  not exist and the API's not-found fallback answers (`router.rs:677`).
- [x] **AC-123** (red, supplemental; written at the code stage, because the shared check did not
  exist when the red tests were written): the shared check, called with the real `AppState` whose pool
  is closed, returns `database` / `error` with a message starting "database check failed: ".
- [x] **AC-124** (red): with `config.toml` holding one rejected proxy entry and schema reads
  refused, an admin signed in by session gets 200 from `GET /api/v1/system/health`; the first item
  is `database` / `error` with a message that starts "database check failed: " and is longer than
  that prefix; the next item is the proxy warning. From the same state, a signed-out
  `GET /api/v1/health` gets 503 with the minimal item of AC-116.
- [x] **AC-131** (red): the page sends `GET /api/v1/system/health` and no `GET /api/v1/health`.
- [x] **AC-132** (red): with that reply holding a `config` / `warning` item, the page shows its
  message in a row whose label reads "warning" in the amber class.
- [x] **AC-133** (red): with the database item `error`, the page shows that row in red and still
  shows the Version row.
- [x] **AC-134** (guard): with `/system/status` answering 500, the page shows "Something went
  wrong" and Retry (ST-003).

**REQ-201, REQ-202** (real local image, no push; files):
- [x] **AC-211** (red): `docker inspect` shows the image's health check test, interval 30 s,
  timeout 5 s, start period 60 s and retries 3 as in REQ-201.
- [x] **AC-212** (red): a container with an empty `/config` volume reports `healthy` within 90 s.
- [x] **AC-213** (red): read right after start, the status is `starting`.
- [x] **AC-214** (red): with `[server] port = 9000`, the status is `unhealthy` after the start
  period and three failed checks.
- [x] **AC-215** (red): with `bind_address = "127.0.0.1"`, and separately with `url_base = "/livrarr"`,
  the status is `healthy`.
- [x] **AC-216** (red): the README and `docs/llm-context.md` carry the override sentence of
  REQ-202. `docker-compose.yml` and `docker/unraid-template.xml` have no diff against `c05752d8`,
  and the compose example in `docs/llm-context.md` keeps its `image:` line and `healthcheck:` block
  as at `c05752d8`. Removing either compose health check while its `image:` line still selects
  alpha6 fails this criterion.

**REQ-301** (real `log_surface` function on real folders; real binary for AC-317 and AC-318).
Seeded files are named for the days before today (UTC) and created oldest first at least 12 ms
apart; "the N newest" means the N seeded files with the latest dates.
- [x] **AC-311** (red): one case per row; the folder then holds exactly these names.

  | Starting folder | Entries after | Names after |
  |---|---|---|
  | does not exist | 1 | today's file |
  | empty | 1 | today's file |
  | 29 dated files | 30 | today's file and all 29 |
  | 30 dated files | 31 | today's file and all 30 |
  | 31 dated files | 31 | today's file and the 30 newest |
  | 85 dated files | 31 | today's file and the 30 newest |

- [x] **AC-312** (red): created in this order: `livrarr.log`, `notes.txt`, `livrarr.log.bak`, a
  directory named `livrarr.log.2020-01-01`, then 30 dated files. Afterwards there are 33 entries:
  `livrarr.log` and `livrarr.log.bak` are gone; `notes.txt`, the directory, all 30 dated files and
  today's file remain.
- [x] **AC-313** (red): with 40 files dated every other day before today, the folder holds 31
  entries: today's file and the 30 newest.
- [x] **AC-314** (guard): the file opened is named as `log_surface::active_log_path` computes.
- [x] **AC-315** (red): with today's file and 35 earlier files (today's created last), the folder
  holds 30 entries: today's file and the 29 newest earlier files.
- [x] **AC-316** (red): with a directory named as today's file in a writable folder, and
  separately with a read-only folder, the function returns an error containing "failed to create
  initial log file" and does not panic.
- [x] **AC-317** (red): the real binary started on a data folder whose `logs/` holds 85 dated files
  serves `GET /api/v1/health`; after it stops, `logs/` holds exactly 31 entries: today's file and
  the 30 newest. Leaving `rolling::daily` in `main.rs` fails this criterion.
- [x] **AC-318** (red): the real binary started on a data folder whose `logs/` holds a directory
  named as today's file serves `GET /api/v1/health`; the first admin's
  `GET /api/v1/system/status` has a `logInitError` containing "failed to create initial log file",
  and stderr contains the same text. Today the binary panics before it serves.

**REQ-401 to REQ-404** (lint and real components):
- [x] **AC-411** (red): `pnpm --pm-on-fail=ignore -C frontend lint` exits 0 with no problems.
- [x] **AC-412** (red): `eslint --print-config src/App.tsx` shows `react-hooks/rules-of-hooks` as
  error, `react-hooks/exhaustive-deps` as warn, and no other `react-hooks/` rule.
- [x] **AC-413** (red): a component with a missing effect dependency given on stdin
  (`--stdin --stdin-filename src/zz.tsx`) makes the lint command exit non-zero.
- [x] **AC-414** (guard): the typecheck and the existing `NotificationBell`, `Header` and reader
  tests pass.
- [x] **AC-421** (red): in the real `Header` with two languages: open search, open the language
  list, close with X, tap the search button (`mousedown` then `click`); the language list is closed.
- [x] **AC-422** (guard): on the desktop form, an open language list closes on a `mousedown`
  outside it.
- [x] **AC-431** (red): each of the three effects in REQ-403 has the disable comment with its
  reason directly above its dependency list; no other disable comment was added.
- [x] **AC-441** (red): the comments at `AppLayout.tsx:35` and `EpubReader.tsx:3` are gone.

**REQ-501:**
- [x] **AC-511** (red): the parsed workflow has `on.push.branches == ["main"]`,
  `on.push.paths-ignore` equal to the four patterns, and `pull_request` and `workflow_dispatch`
  as before with no path filter.
- [x] **AC-512** (red): no tracked file under an image input (ST-019) matches any of the four
  patterns, checked with GitHub's rule (`*` stops at `/`).
- [ ] **AC-513** (red, after push, PO's word): `gh run list` shows a run for the first code push
  and none for a docs-only push; a later pull request still runs.

**REQ-601 to REQ-603** (real loader and router). "Bucket test from peer P" means: five failed
logins from peer P with `X-Real-IP` set to client A, then one with client B. "Trusted" means the
sixth is not answered 429; "untrusted" means it is. The "forwarded-for bucket test" is the same with
`X-Forwarded-For` in place of `X-Real-IP`. In AC-618 to AC-620 each bucket test runs on a fresh
router built from the loaded config.
- [x] **AC-611** (red): with all REQ-601 shapes in `trusted_proxies`, the admin reply holds exactly
  one `config` / `warning` row per rejected entry, naming it as in REQ-602, and none for the
  accepted ones, the duplicate or an empty list.
- [x] **AC-612** (labelled red through v3; a guard in practice: today's loader already accepts the
  file, so the test passes on `c05752d8`): `load_config` succeeds for that file (startup continues).
- [x] **AC-613** (red): with `trusted_proxies = [" 10.0.0.5 ", "nginx"]`, the bucket test from
  peer 10.0.0.5 (A = 203.0.113.1, B = 203.0.113.2) is trusted; from peer 10.0.0.9 it is untrusted
  (control).
- [x] **AC-614** (red): the real binary started with `nginx` in `trusted_proxies` writes one WARN
  line with the REQ-602 text to stdout.
- [x] **AC-615** (red): the row is in two consecutive replies; after the file is rewritten without
  the entry and loaded again into a new router (a restart), it is gone.
- [x] **AC-616** (labelled guard through v3; red in practice: the route did not exist, so the
  normal user got 404, not 403; never rule): a normal user gets 403 from `/api/v1/system/health` with
  warnings configured.
- [x] **AC-617** (red): with `trusted_proxies = ["10.0.0.0/33", "fd00::/129"]` loaded once into one
  router, the admin reply holds a warning row naming each entry, and the bucket test is untrusted
  from peer 10.0.0.0 (A = 203.0.113.1, B = 203.0.113.2) and from peer `fd00::` (A = 2001:db8::1,
  B = 2001:db8::2). Control, a separate config `["10.0.0.0/8", "fd00::/8"]`: no warning rows, and
  the same two bucket tests are trusted.
- [x] **AC-618** (red): with only `trusted_proxies = ["0.0.0.0/0"]`, the admin reply holds no
  `config` row; the bucket test is trusted from peer 192.0.2.7 (A = 203.0.113.1, B = 203.0.113.2)
  and untrusted from peer 2001:db8::7 (A = 2001:db8::1, B = 2001:db8::2). Today the test build
  panics in the rate limiter on the IPv4 peer.
- [x] **AC-619** (red): with only `trusted_proxies = ["::/0"]`, the admin reply holds no `config`
  row; the bucket test is trusted from peer 2001:db8::7 (A = 2001:db8::1, B = 2001:db8::2) and
  untrusted from peer 192.0.2.7 (A = 203.0.113.1, B = 203.0.113.2). Today the test build panics
  on the IPv6 peer.
- [x] **AC-620** (red): with only `["0.0.0.0/0"]`, the forwarded-for bucket test from peer
  192.0.2.7 (A = 203.0.113.1, B = 203.0.113.2) is untrusted, and the bucket test from the same peer
  is trusted (control). With only `["::/0"]`, the forwarded-for bucket test from peer 2001:db8::7
  (A = 2001:db8::1, B = 2001:db8::2) is untrusted, and the bucket test is trusted (control).
  Neither config gives a `config` row. Today the test build panics in the rate limiter.

**REQ-701:**
- [x] **AC-711** (red): with `[foo]`, a root `nginx = 1` and `[server] api_kye = "sk-live-ac711Marker"`,
  the admin reply holds "Unknown config key: foo", "Unknown config key: nginx" and
  "Unknown config key: server.api_kye", and the body does not contain `ac711Marker`.
- [x] **AC-712** (red): with a quoted key shaped like the one in `test_fresh_author_index.rs:2388-2392` whose secret
  part is `ac712Marker`, its row contains `[REDACTED]` and
  the body does not contain `ac712Marker`.
- [x] **AC-713** (guard): the existing unknown-key log tests in `test_fresh_author_index.rs` pass.
- [x] **AC-714** (red): with no `config.toml`, the admin reply holds only the database item.

### Added in tests review

- [x] **AC-715** (red; tests review r1, finding T1): the real binary started with the AC-711 inputs and
  AC-712's secret-shaped quoted key (marker `ac712Marker`), after first-admin setup, answers
  `GET /api/v1/system/health` with 200, the three exact "Unknown config key: …" rows and one row
  for the secret-shaped key containing `[REDACTED]`, and the body holds neither marker. It proves
  that startup itself stores the unknown keys in the running state, which the router tests cannot
  show because they load the config through a seam (§8). Test:
  `startup_keeps_unknown_keys_for_the_admin_health_list`
  (`crates/livrarr-server/tests/test_fresh_author_index.rs:1097` at `25ca4f30`).

## 8. As built

The feature was built in one checkout by `fix` subagents on Claude Opus 5.5 (effort high), which
also wrote the tests. GPT-6 Astra (xhigh) was the sole reviewer. It was deployed to the local app
at 2026-10-06T18:55:19Z from the reviewed working tree plus the docs fold, debug profile, no
migration (`build/reviews/operations-hygiene/deploy-20261006T185446Z/DEPLOYMENT.json`); the PO
answered the live check "ok" the same day. The commit is `25ca4f30`, local; its tree-diff hash
matches the deployed build. Line references below are at `25ca4f30`.

### Review history

| Stage | Round | Verdict | Findings |
|---|---|---|---|
| Spec | r1 | FAIL | 8 P2 (F1 to F8; folded in v2) |
| Spec | r2 | FAIL | 2 P2 (F9, F10; folded in v3) |
| Spec | r3 | PASS | none blocking |
| Tests | r1 | FAIL | 3 P2, 1 P3 (T1 to T4; fixed in packet 3) |
| Tests | r2 | PASS | all four closed |
| Code | r1 | PASS | one P3 (docs: the same-day restart exception), folded without re-review |

Reviews are in `build/reviews/operations-hygiene/` (`REVIEW-astra-*.md` under the packet folders,
`review-*-openai-r*.json` at the top).

### What was added

| Piece | Where | What it is |
|---|---|---|
| `DatabaseHealth::check_database` | `crates/livrarr-domain/src/services/database_health.rs:5`, `:8` | Domain trait for the one database check. |
| `impl DatabaseHealth for SqliteDb` | `crates/livrarr-db/src/sqlite_database_health.rs:7-9` | Runs `SELECT count(*) FROM sqlite_schema` through the live pool. |
| `HasDatabaseHealth` | `crates/livrarr-handlers/src/context.rs:297`; `AppState` impl `crates/livrarr-server/src/state.rs:780` | Capability trait; the public handler's only capability. |
| `HasConfigWarnings` | `context.rs:304`; `AppState` impl `state.rs:787-789` | Capability trait; `AppState` calls `config::config_warnings` on `AppState.config`. |
| `system::database_check` | `crates/livrarr-handlers/src/system.rs:33` (2 s bound `:28`; timeout detail `:48`) | Shared check: `ok`, or `error` with "database check failed: {detail}", cleansed; logs each failure at WARN. |
| `system::health` | `system.rs:61-76` | Public `GET /api/v1/health`: 200 with the database item, or 503 with the fixed minimal item. |
| `system::admin_health` | `system.rs:80-96`; route `crates/livrarr-server/src/router.rs:458-459` | Admin-only `GET /api/v1/system/health` (`RequireAdmin`): database item with detail, then one `config` / `warning` item per warning. |
| `parse_trusted_proxies` | `crates/livrarr-server/src/rate_limit.rs:73`; router use `router.rs:22-24` | Splits entries into accepted ranges and rejected entries; router trust and the warnings share it. `IpNet::parse` trims and caps prefixes (`rate_limit.rs:53-54`); `contains` matches every same-kind address for `/0` before any shift (`:25`, `:35`). |
| `config::load_config` | `crates/livrarr-server/src/config.rs:422` (moved from `main.rs`); startup call `main.rs:721` | The startup loader; also sets `AppConfig.unknown_keys` (`config.rs:35-36`, `:444`). |
| `config_warnings` | `config.rs:456`; startup log loop `main.rs:733` | Rejected proxy entries in file order, then unknown keys, each cleansed. |
| `log_surface::build_file_appender` | `crates/livrarr-server/src/log_surface.rs:22-28` (`MAX_LOG_FILES = 31`, `:18`); startup `main.rs:1311` | Daily appender, prefix `livrarr.log`, capped; a build error goes to stderr and `logInitError` and startup continues without the file layer. |
| `set_schema_reads_refused` | `crates/livrarr-db/src/lib.rs:141` (`test_helpers`) | The ST-026 test fixture. |
| Web: `getSystemHealth` | `frontend/src/api/index.ts:588-589`; `StatusPage.tsx:188-189` (query key `system-health`) | System → Status reads the admin route; the unrouted `HealthPage.tsx` keeps the `health` key. |
| Image, CI, lint | `Dockerfile:111-112`; `.github/workflows/ci.yml:4-6`; `frontend/eslint.config.js:3`, `:10-14`; `frontend/package.json:10`, `:54`; `Header.tsx:40` | As REQ-201, REQ-501, REQ-401 and REQ-402. |
| Docs | `README.md:104`; `docs/llm-context.md:186`, `:245`, `:246`, `:381-382` | Override sentence, `trusted_proxies` line, retention sentence, endpoint rows. |

### Corrections found during the build

None changed a decision; the PM accepted each as routine.

- **`load_config` keeps its tuple return** (code stage, `packet-4-code/HANDBACK-C1.md`). ST-023 said
  "moved without a signature change", so the return type stayed `(AppConfig, Vec<String>)` while
  `AppConfig` also carries `unknown_keys`. Startup ignores the tuple copy. Dropping it would be a
  signature change this spec does not name.
- **`system::health` returns `(StatusCode, Json<…>)`** (code stage). `ApiError` cannot send a 503
  whose body is the item list REQ-101 requires. No other caller.
- **`main.rs` file-layer setup reshaped** (code stage) into an `Option` built from
  `build_file_appender` and mapped into the layer (`main.rs:1300-1330`). Shape only.
- **Lint plugin install** (code stage, `packet-4-code/HANDBACK-C2.md`). See ST-012: range
  `~7.1.1`, pnpm 10 from inside `frontend/`.
- **Four `exhaustive-deps` disables remain, not five** (tests stage). See ST-017.
- **AC-612 and AC-616 labels** (tests stage, `packet-1-red-tests/HANDBACK-T1.md`). AC-612 passes on
  the old code (a guard in practice); AC-616 was red, because the route did not exist.
- **Docs wording** (code stage, `HANDBACK-C3.md`). The override sentence says "your own
  `healthcheck` block" rather than the literal `healthcheck:`, so the AC-216 check, which looks for
  `healthcheck:`, still sees only the unchanged compose example.

### Tests as built

| Area | Where | Notes |
|---|---|---|
| Health, admin list, proxies, unknown keys | `crates/livrarr-server/src/router.rs` test module, `:1930` to `:3042` | AC-111 to AC-124, AC-611 to AC-620, AC-711, AC-712, AC-714. AC-123 is `shared_database_check_reports_a_closed_pool_with_its_detail` (`:3026`), written by the code seat. |
| Log retention matrix | `crates/livrarr-server/src/log_surface.rs` test module, `:249` to `:433` | AC-311 to AC-316. |
| Real binary | `crates/livrarr-server/tests/test_fresh_author_index.rs:1052` (AC-614), `:1097` (AC-715), `:1194` (AC-317), `:1240` (AC-318) | AC-713 is the nine existing unknown-key log tests. |
| Web | `frontend/src/pages/system/status/StatusPage.test.tsx:167-254` (new; AC-131 to AC-134, plus an all-down case); `frontend/src/components/Header/Header.test.tsx:475`, `:519` | AC-421, AC-422. |
| Commands | evidence scripts in the packet folders | AC-211 to AC-216, AC-411 to AC-413, AC-431, AC-441, AC-511, AC-512. |

- **Two test seams.** The red tests had to compile against `c05752d8`, where neither production
  function existed or was reachable. `load_config_from` (`router.rs:1938-1939`) is the config seam:
  `load_config` was private to the binary, so the router tests repeated its steps; at the code
  stage its body became one call to `crate::config::load_config`. `file_appender`
  (`log_surface.rs:122-123`) is the log seam: the spec did not name the new constructor, so the
  tests called a helper whose body was today's `rolling::daily`; at the code stage it became one
  call to `build_file_appender`. The code packet allowed only those two one-line bodies. AC-317,
  AC-318 and AC-715 drive the real binary, so editing a seam instead of production would still
  fail. The seam's doc comment at `router.rs:1934-1937` still describes the old repeated steps.
- **AC-123 at the code stage.** It calls the shared check directly, which did not exist until the
  code; the code seat showed it red by stubbing the check to return `ok` without reading.
- **AC-715 after tests review r1.** Finding T1: with the seam in place, a fix that left startup on
  its old loader would pass every router test while the running app showed no unknown-key rows.
- **UTC-midnight retry** (tests review r1, finding T4, P3). `within_one_utc_day`
  (`log_surface.rs:136`; `test_fresh_author_index.rs:1166`) captures one UTC date per attempt and
  judges the result only if the date is unchanged at the end; otherwise it runs once more on a
  fresh folder. The retry branch itself has not run.
- **Knock-on test change** (packet 5). After the code, `frontend/src/App.routes.test.tsx` timed
  out 8 cases, because its fetch stub answered the new `/system/health` route with 404 and the
  Status page then showed its error screen. The stub now treats it as an admin-only route
  (`:46`, reply `:104-109`) and the Status page's admin calls include it (`:330`), which adds an
  assertion that a normal user never sends it. No assertion weakened, no timeout raised.

### Delivered and deferred

| Item | State | Evidence |
|---|---|---|
| REQ-101 to REQ-103, REQ-301, REQ-401 to REQ-404, REQ-601 to REQ-603, REQ-701 | Delivered | Final tree: whole frontend suite 44 files, 429 tests pass; lint exit 0; `verify.py code` PASS; fmt, clippy and typecheck clean (`packet-6-code-review/PM-RUNS.md`). The one workspace test failure seen was an enrichment test this feature does not touch; it passed three times alone. |
| REQ-201, REQ-202 | Delivered | Real local image built in 556 s (single platform, partly from the build cache); AC-211 to AC-215 passed; compose and Unraid unchanged (`packet-4-code/HANDBACK-C3.md`). |
| REQ-501 | Delivered in the file; run pending | AC-511 and AC-512 pass. **AC-513 pending:** a run on the first code push and none on a docs-only push need that push. |
| Log pruning at a change of day | Not probed | No clock control in the library; startup pruning is proven (AC-317, and the live start went from 85+ files to 30, a same-day restart). |
| A failing database inside a container | Not probed | The image was only run healthy or with a moved port; REQ-101 and ST-006 give the expected `unhealthy`. |
| A cold image build | Not probed | The 556 s build reused cached layers. |

### Limits of the evidence

- Tests ran in debug builds only; the release result of a `/0` entry follows from the
  short-circuit before the shift (`rate_limit.rs:25`, `:35`) and is read, not run.
- Behaviour change on other paths through the shared parser, pinned by AC-613 and AC-617 but not
  observed live: `10.0.0.0/33` and `fd00::/129` now trust nothing (before, their own address), and
  a padded entry is now trusted.
- The identity of `dismiss.mutate` and `setRpmHighlight`, and `libraryItemId` changing only with
  `openingId`, rest on the readings in ST-014 and ST-015; no test mounts `EpubReader`,
  `useStreamUrl` or `AppLayout`, whose edits are comments only.
- `actionlint` was not run; the workflow was parsed with PyYAML and checked by the AC-511 and
  AC-512 scripts.
- The live check covered the screens in `deploy-20261006T185446Z/LIVE-CHECK.md`; the PO's "ok"
  does not say which were opened, and the optional config-warning row was not reported.
