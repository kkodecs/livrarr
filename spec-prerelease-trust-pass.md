---
feature: prerelease-trust-pass
stage: spec
status: delivered
version: 6
type: bugfix
req_ids: [REQ-101, REQ-102, REQ-103, REQ-104, REQ-105, REQ-106, REQ-107, REQ-201, REQ-202, REQ-203, REQ-204, REQ-301, REQ-302, REQ-303, REQ-304, REQ-305, REQ-306]
---

# Bug Spec: prerelease-trust-pass

## Executive summary

**Delivered.** All three parts were built, passed code review on the second round, and were
checked live by the PO on 2026-10-02 ("tested both all good", as the PM reports): the existing
install was unchanged, and a fresh install asked for the setup token, refused a wrong one,
accepted the right one and deleted the token file. The code and docs are committed as
`d77f690a` and `c70dca4e` and were pushed to `main` on 2026-10-03. Every requirement is delivered. Making `url_base` work
stays deferred to the backlog (#119), as the PO decided; this spec only stopped the docs claiming
it works. The spec now describes what was built ([as built](#7-as-built-corrections-and-limits)).

**What the build corrected.** The silent config check had a second cause the spec missed: it read
the whole file as one TOML value, so it never ran at all. Three token rules were tightened in code
review: the token is used up the moment the account is claimed, a token file is reused only if it
holds exactly the 32 characters, and setup is refused on non-Unix systems
([corrections](#corrections-found-during-the-build)).

**Recorded limits.** The non-Unix code was never compiled on this host; Livrarr ships Linux images
only ([limits](#limits)). After the push, the PM checked that the Help page's AI prompt reaches the
corrected docs: GitHub serves the pushed `docs/llm-context.md` byte for byte
([acceptance](#acceptance-as-built)).

Three pre-release fixes, approved by the PO on 2026-10-01, with the open choices settled the same
day ([Revision](#revision)).

- **Part 1: correct the user docs.** 22 wrong claims: 15 of the scan's 16 (D10 is correct and
  dropped), plus 7 more in `docs/llm-context.md` that the PO added, including a Docker example that
  cannot start. The most important file is `docs/llm-context.md`. The "Get AI Help" prompt
  sends the user's AI to read it from GitHub `main`, so the corrected AI help reaches users only
  after a push. `docs/ARCHITECTURE.md` becomes the redirect that was planned but never reached git.
  Root `ARCHITECTURE.md` gets back the one provider sentence the PO approved on 2026-09-18
  ([REQ-101](#2-requirements) to [REQ-107](#2-requirements)).
- **Part 2: config settings.** The `[auth]` proxy-login keys, which never did anything, are
  **removed**; anyone who still has them gets a warning and a normal start. `url_base` is
  **deferred** (#119 is back on the backlog): its code stays, but the docs stop saying it works.
  **No config warning reaches the user today**, because the check runs before logging starts
  (tested, [ST-205](#0b-system-truths)). After the fix, warnings show at the default level (and any level that
  shows warnings), and the working `[server] trusted_proxies` and `[metadata_cache]` are not flagged ([REQ-201](#2-requirements) to
  [REQ-204](#2-requirements)).
- **Part 3: one-time setup token.** Today, on a fresh install, whoever sends the first setup
  request becomes admin. After the fix, a fresh install prints a token to its console and saves it
  as `setup-token` in the data folder (`/config/setup-token` in Docker). The setup page asks for
  it; a wrong token is refused with a plain message. The token file is rewritten privately on every
  start; if that fails, setup is not served. The token lasts across restarts and is deleted once
  setup succeeds. Finished installs see no change. There is no environment variable
  and no schema change. About one day ([REQ-301](#2-requirements) to [REQ-306](#2-requirements)).

**Tested vs read.** Only the dropped warnings and the placeholder admin were checked by running the
binary on scratch folders. Everything else was read at `37dd4426`. External precedent was not
checked.

Sections: [system truths](#0b-system-truths), [prior art](#0c-prior-art),
[problems](#1-problem-statement), [requirements](#2-requirements),
[traceability](#2a-claim-traceability-d1d23), [files](#2b-files-per-part),
[decisions](#5-open-questions), [acceptance](#6-acceptance-criteria),
[as built](#7-as-built-corrections-and-limits).

## Revision

v1, 2026-10-01: written from source at `37dd4426`, the scan report
`build/reports/docs-ops-security-scan-2026-09-26.md`, the task list in `build/plans/` and GitHub
#119, plus one probe of `target/debug/livrarr`. IDs are numeric because the gate requires them:
REQ-1xx = REQ-D (docs), REQ-2xx = REQ-S (settings), REQ-3xx = REQ-B (bootstrap). v1 sha256
`581ebda61deb8f2d9686d6e63ebf06879b96f34fd2f9dc0bf313eec7b0aa3567`.

v2, 2026-10-01: PO answers folded, verbatim via the PM (`FOLD-PACKET-v2.md`).
1. DS1 `url_base`: "skip it for now - re add to back log". It now has no code change, and the docs
   say it is not supported (REQ-201, REQ-105).
2. DS2 `[auth]` keys: "Remove them (Recommended)" (REQ-202, REQ-204).
3. Root `ARCHITECTURE.md`: "Add the one sentence (Recommended)". The base is the committed file,
   and only the 2026-09-18 provider sentence comes back (REQ-106). This one Part 1 principle edit is
   PO-approved, an override of the packet rule.

Also changed: ST-108 corrected (the sentence is in parked commit `57189130`, not in r4); REQ-103
no longer waits on a base or borrows r4 wording; the make-it-work designs are one line each in §5.

v3, 2026-10-01: PO scope addition, verbatim via the PM (`FOLD-PACKET-v3.md`): "Add all 7
(Recommended)". The seven extra `docs/llm-context.md` items are D17–D23 (§2a, REQ-107,
ST-117–ST-122). All seven held on re-check, so none were dropped. v2 sha256
`45ad79742d112ec43d2199748d1fc2582897b43d62da5a2cacb8d278a30e3e7e`.

v4, 2026-10-01: review r1 (GPT-6 Astra, FAIL) folded per `FOLD-PACKET-v4.md`.
- R1 (accept): every refusal uses the fixed message, and error and success bodies are checked for
  the token (REQ-302, AC-301, AC-303).
- R2 (accept): `[metadata_cache]` children are checked (REQ-203, AC-201).
- R3 (partly accepted, per the PM): no second output path; the promise is narrowed to levels that
  show warnings, the default-level AC-202 is added, and §4 records the design.
- R4 (accept): the token file is rewritten privately each start by create-new plus rename, never
  through a link, failing closed (REQ-301, AC-304).
- R5 (accept): the database flag is the authority; cleanup is best effort (REQ-301/302, AC-305).
- R6 (accept): AC-307 uses `installApiStub`.
- R7 (accept): login is a burst of 5 plus one every 12 s (ST-119, D20).
- R8 (accept): CWA hardlink, copy across filesystems (D6).
- R9 (accept): `root-architecture-r1.md` is the predecessor; no r5.
- R10 (accept): ST-204, ST-309 (12 sites, listed) and ST-121 corrected.
- Coverage guards added: AC-302 (no-token service, concurrent claims) and AC-306 (session and API
  key survive restart).
No deviation beyond R3, which follows the PM disposition. v3 sha256
`2ad7f7617cdc21f54c08bfebac93e210022aabb6ad1dd8824db16118041c8ce8`.

v5, 2026-10-01: review r2 (GPT-6 Astra; nine of ten r1 items closed) folded per
`FOLD-PACKET-v5.md`. R5 continued (accept): AC-305 requires the cleanup warning on the restart
with the directory present, and a recovery restart over a regular stale file that must exist
before launch and be removed by startup. N1 (accept): AC-302(a) and AC-305's cleanup and warning
assertions are labelled red; concurrency and login invariants stay guards. v4 sha256
`812680529126a6229d101d42c9541c33f9d20756a8a872869af3703f6f6c13be`.

v6, 2026-10-02: brought in line with the built change, per `packet-7-as-built/PACKET.md`. Sources:
commits `d77f690a` (code and tests) and `c70dca4e` (docs); the code hand-back evidence in
`packet-3a-code/` (including `evidence-toml-value-parse.log`); `packet-3b-docs/TRACE.md`; reviews
`packet-4-code-review/REVIEW-astra-code-r1.md` (FAIL, R1–R5) and `-r2.md` (PASS, N1 folded as a
test-only change in `packet-6-fold-n1/`); `packet-5-code-fix-r1/PACKET.md` and
`pm-delta-fix-r1.patch`; the PM session log entries of 2026-10-02; and
`deploy-20261002T022337Z/DEPLOYMENT.json` and `LIVE-CHECK.md`. Each requirement is marked
Delivered; §7 records what the spec got wrong or left out and the limits. No decision changed.
Status `delivered`. v5 is kept at
`build/reviews/prerelease-trust-pass/packet-0-spec/spec-prerelease-trust-pass-v5.md` (sha256
`5053ced20f2a73a5e425f447c397acbd1fa583071ad89c7c7b1deb24b84d5cb6`).

v6 update, 2026-10-03 (factual, no version bump; the committed v6 at `dff80a06` is the predecessor),
per `packet-8-ac101/PACKET.md`: after the push `59b7e9db..dff80a06`, the PM fetched the ST-101 raw
URL (HTTP 200, sha256 `8128fe822937337a8f0a241234d25aa5302ec07db3d9b71d4c3c9b5c592594b3`, equal to
the local and `origin/main` `docs/llm-context.md`; copy in `post-push-llm-context.md`), so AC-101 is
met.

## 0a. Design Principles

- Fix where the wrong thing is made, with one definition per rule: one known-key list, and one
  token check in the auth service that every setup request passes through.
- Docs describe delivered behaviour. A key that never did anything never stops startup.
- Tests drive the real door (the built binary, the real router with real `SqliteDb`, the real
  `App` mount) and pin only the named observable.

## 0b. System Truths

Read at `37dd4426` unless marked T (tested).

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-101 | The Help page builds a copyable prompt that tells the AI to read `docs/llm-context.md` from GitHub `main` (raw URL) and links the GitHub view. The page does not fetch it. | `frontend/src/pages/help/HelpPage.tsx:16-17`, `:38-40`, `:235` | Treating a local doc edit as live AI help. | read |
| ST-102 | The workspace has 17 members, including `livrarr-cli` and `livrarr-behavioral`. | `Cargo.toml:3-21` | "13 crates". | read |
| ST-103 | Build stages: `node:20-alpine` (frontend), `rust:1.94-alpine` (builder), `alpine:3.21` (runtime). The runtime image creates a fixed `livrarr` user 1000:1000. | `Dockerfile:4`, `:19`, `:78`, `:83-84` | "bookworm". | read |
| ST-104 | The entrypoint creates no user. Run as non-root, it execs the app as that user. Run as root, it fixes `/config` ownership and drops to `PUID:PGID` (default 1000:1000) with `su-exec`. App output passes straight to the container log. | `docker/entrypoint.sh:22-23`, `:30-31`, `:37`, `:50`; `Dockerfile:108` | "PUID/PGID user creation in entrypoint". | read |
| ST-105 | SQLite uses one pool of up to 4 connections, in WAL mode. | `crates/livrarr-db/src/pool.rs:20`, `:27-29` | "single write connection, multiple readers". | read |
| ST-106 | Download clients are qBittorrent, SABnzbd and Transmission, all selectable in Settings. The setup wizard's client step configures qBittorrent only. | `crates/livrarr-domain/src/infra_config.rs:13-21`; `frontend/src/pages/settings/download-clients/DownloadClientsPage.tsx:550`; `CHANGELOG.md:345` (alpha5, `:321`); `frontend/src/pages/setup/SetupPage.tsx:64`, `:528` | Listing two clients. | read |
| ST-107 | Provider clients live in `livrarr-external-data`: Goodreads, Audible, Audnexus, Google Books, Hardcover and OpenLibrary. | `crates/livrarr-external-data/src/` (`goodreads/`, `audible.rs`, `audnexus.rs`, `google_books.rs`, `hardcover.rs`, `openlibrary.rs`) | "four providers"; providers in `livrarr-metadata`. | read |
| ST-108 | Committed root `ARCHITECTURE.md` (last change `90c9554e`, 2026-08-19) says at `:66` "Adding a new provider means implementing the trait contract — nothing else changes." The PO-approved sentence of 2026-09-18 is in git only at `57189130:ARCHITECTURE.md:90`, a parked WIP commit that is not an ancestor of `37dd4426`. `docs/design-history/root-architecture-r4.md` is that pass's *input* and lacks the sentence. The committed file is already preserved byte-for-byte as the tracked `docs/design-history/root-architecture-r1.md` (both sha256 `1b0b596f…`, 20,985 bytes). | `ARCHITECTURE.md:62-66`; `57189130:ARCHITECTURE.md:86-94`; `git merge-base --is-ancestor` (false); `rg "Keep the details" docs/design-history` (none); `provider-rule-2026-09-18/MANIFEST.json`; `sha256sum` of both files | Restoring any other parked text. | read |
| ST-109 | M4B and MP3 tag writing returns `Unsupported`; only EPUB is written. Import writes tags automatically when enrichment data is present. | `crates/livrarr-tagwrite/src/lib.rs:168-179`; `crates/livrarr-server/src/import_service.rs:265-284` | "M4B/MP3 tag writing"; "never automatic". | read |
| ST-110 | `livrarr-domain` has 14 external dependencies and no internal ones. | `crates/livrarr-domain/Cargo.toml:7-21` | "zero external deps beyond serde/chrono". | read |
| ST-111 | Root `ARCHITECTURE.md`'s dependency table is headed "May depend on" (a permission). Its rows match the canonical seams. | `ARCHITECTURE.md:174-190`; `docs/canonical-model.yaml:63-64`, `:82`, `:86`, `:88` | Treating D10 as an error. | read |
| ST-112 | Third-party credentials are plain `TEXT`: the download-client password, indexer, Prowlarr and LLM keys, the Hardcover token, the SMTP password and the Google Books key. The download-client response returns neither password nor API key. | `crates/livrarr-db/migrations/001_initial_schema.sql:144`, `:151`, `:154`, `:175`; `002_add_indexers.sql:10`; `011_add_email_config.sql:9`; `047_add_google_books_api_key.sql:1`; `crates/livrarr-server/src/api_secondary_impl.rs:760-777` | "the one exception is download-client passwords". | read |
| ST-113 | Import paths are `{root}/{user_id}/{Author}/{Title}.{ext}` (ebook) and `{root}/{user_id}/{Author}/{Title}/{files}` (audiobook). There is one definition, re-exported by the server. | `crates/livrarr-library/src/import_workflow.rs:1781`, `:1794`; `crates/livrarr-server/src/infra/import_pipeline.rs:5` | Paths without `{user_id}`. | read |
| ST-114 | The README Quick Start compose has no `cap_add:` block; only the repository `docker-compose.yml` has one, and its comment says to delete it for `user:` mode. | `README.md:37-49`, `:136`; `docker-compose.yml:18-24`, `:38-39` | Telling Quick Start users to delete it. | read |
| ST-115 | `livrarr-http` has no `tower` dependency. | `crates/livrarr-http/Cargo.toml` `[dependencies]` | "via tower middleware". | read |
| ST-116 | `docs/ARCHITECTURE.md` is byte-identical to `docs/design-history/docs-architecture-r1.md`. The design-history index says the old path "redirects"; at HEAD it does not. | `diff` (identical); `docs/design-history/README.md`; `git log -- docs/ARCHITECTURE.md` (last `f1c6486e`) | A second snapshot of the same bytes. | read |
| ST-117 | The file log is `{data}/logs/livrarr.log.<YYYY-MM-DD>`, a daily rolling file. | `crates/livrarr-server/src/main.rs:1258`, `:1262`, `:1266`. **T:** the scratch probe wrote `logs/livrarr.log.2026-10-01` | "`livrarr.txt`". | read; T |
| ST-118 | Command-line options are `--data`, `--ui-dir`, the `identity-cutover` subcommand, and clap's `--help` and `--version`. The container reads `PUID`, `PGID` (and the compose file sets `TZ`), and `RUST_LOG` replaces the log filter. None of these override `config.toml` keys. | `main.rs:38-49`, `:52-59`, `:1237-1243`; `docker/entrypoint.sh:22-23`; `docker-compose.yml:5-8` | "No CLI flags beyond `--data`". | read |
| ST-119 | Per-IP limits exist: login allows a burst of 5, then one more attempt every 12 s (so up to 9 in the first minute); setup allows 1 per 12 s with no burst; every API route allows 100 per second sustained with a burst of 50. Separately, a username locks for 15 minutes after 5 failures. | `crates/livrarr-server/src/router.rs:31-37`, `:43-48`, `:50-56`, `:674`; `crates/livrarr-server/src/auth_service.rs:110-113` | "No per-IP global rate limit". | read |
| ST-120 | API calls authenticate with `Authorization: Bearer <session token>` (from login) or `X-Api-Key`. No cookie is read. OPDS uses Basic Auth per handler. | `crates/livrarr-server/src/middleware.rs:22-23`, `:58-59`, `:85`, `:88-94`; `router.rs:676` | "session cookie". | read |
| ST-121 | The `docs/llm-context.md:208-225` example drops all capabilities with no `cap_add` and no `user:`. The image has no `USER` line, so the container starts as root and takes the PUID/PGID path. It cannot start, but the first failure depends on ownership. If anything under `/config` is not owned by `PUID:PGID` (for example a fresh bind mount owned by root), the chown at `:37` needs CHOWN and exits with "cannot chown /config". Otherwise the preflight `su-exec "$PUID:$PGID" true` at `:44` needs SETUID and SETGID and exits with "cannot drop privileges". The repository `docker-compose.yml` adds exactly the four capabilities that path needs. | `docker/entrypoint.sh:30-32`, `:37-40`, `:44-47`; `Dockerfile` (no `USER`; `ENTRYPOINT` `:108`); `docker-compose.yml:16-24`; `README.md:138` (same warning) | Shipping that example. | read (no image on this host, so not run) |
| ST-122 | The README Discord invite was changed on purpose on 2026-05-27 ("docs: update Discord invite link"). `docs/llm-context.md:412` still has the old invite. The README and `docker-compose.yml` pin image `0.1.0-alpha6`. | `git log -S` on both invites (`ef0cebcd`); `README.md:40`, `:180`; `docker-compose.yml:3` | Inventing a tag or link. | read |
| ST-201 | `[server] url_base` has a default, a slash check and a known-key entry. Nothing else reads it. All other `url_base` hits are the download-client field. | `crates/livrarr-server/src/config.rs:52-53`, `:64`, `:321-339`, `:419`; `crates/livrarr-domain/src/infra_config.rs:75`; `crates/livrarr-server/src/infra/release_helpers.rs:5-14` | Confusing the two. | read |
| ST-202 | Routes are fixed: API at `/api/v1`, OPDS at `/opds`, and the web app as the root fallback. The frontend and the server build root-absolute URLs. | `crates/livrarr-server/src/router.rs:699`, `:702-706`; `frontend/src/api/client.ts:155`; `crates/livrarr-handlers/src/opds.rs:130-131`; `crates/livrarr-server/src/cover_service.rs:98`, `:103` | Docs saying sub-path hosting works. | read |
| ST-203 | `[auth] external_header` is read by nothing. `[auth] trusted_proxies` is read only by a CIDR check that stops startup on a bad entry. `parse_cidr` has no other caller. `AuthType::ExternalAuth` is never constructed. The middleware accepts only a Bearer session token or `X-Api-Key`. | `config.rs:21-22`, `:86-95`, `:341-349`, `:390-408`, `:418`, `:420`, `:445-451`; `crates/livrarr-domain/src/entities.rs:232-236`; `crates/livrarr-server/src/middleware.rs:15-86` | Claiming proxy login works. | read |
| ST-204 | `[server] trusted_proxies` (default empty) feeds the rate-limit key: forwarded-for headers are trusted only from a listed peer. Entries that fail `IpNet::parse` are silently skipped. Over-long prefixes (an IPv4 `/33`, an IPv6 `/129`) are accepted and match one exact address. | `config.rs:55-56`, `:70-72`; `router.rs:21-29`; `crates/livrarr-server/src/rate_limit.rs:22-41`, `:44-54`, `:85-113` | Removing it. | read |
| ST-205 | The unknown-key check runs inside `load_config`, before the log subscriber exists, so every warning is dropped. Its lists also omit `[server] trusted_proxies` and the root `[metadata_cache]` section (children `ttl_days`, `max_rows`), all of which the loader reads. The filter built from `[log] level` (or `RUST_LOG`) covers every output, so a level above WARN hides any warning. **As built (v6): incomplete.** A second cause hid the warnings even with logging in place: the check read the file with `raw.parse::<toml::Value>()` (`37dd4426:crates/livrarr-server/src/main.rs:1203`), which in toml 1.1 parses one value, not a document, so on a real config it failed and `if let Ok` skipped the check ([§7](#corrections-found-during-the-build)). | `crates/livrarr-server/src/main.rs:721`, `:730`, `:1202-1205`; `config.rs:30-31`, `:214-237`, `:418-420`; `main.rs:1229-1245`, `:1287-1292`. **T:** the binary on a scratch folder with `trusted_proxies`, a bogus key and section, and `[metadata_cache]` started with no "Unknown config key" line in any output | Treating the false warning as the user-visible bug. | read; T |
| ST-206 | TOML parsing ignores unknown keys (no `deny_unknown_fields`). Config errors stop startup through `eprintln!` and exit 1. | `config.rs`; `main.rs:721-727`, `:1207`, `:1213` | — | read; T (bad port: "Configuration error: …", exit 1) |
| ST-207 | `tests/implementation/test_impl_config_v21.rs` (tracked) is compiled by nothing: no manifest lists it, and it imports the old name `librarr`. It asserts the `url_base` and `auth.trusted_proxies` checks. | `test_impl_config_v21.rs:1`, `:23-147`; `rg test_impl_ --glob Cargo.toml` (none) | Counting it as coverage. | read |
| ST-208 | The tracked test `crates/livrarr-server/tests/test_fresh_author_index.rs` starts the Cargo-built binary on a temp data folder with its own `config.toml`, captures its output and completes setup over HTTP. New files under any `tests/` directory are gitignored. | `test_fresh_author_index.rs:57-94`, `:189-200`; `crates/livrarr-server/Cargo.toml:69-71`; `.gitignore:3` | A hand-built startup sequence. | read |
| ST-301 | Migration 001 inserts a placeholder admin (id 1, `admin`, empty hashes, `setup_pending = 1`). No other writer sets `setup_pending = 1`. | `crates/livrarr-db/migrations/001_initial_schema.sql:28-32`; `crates/livrarr-db/src/sqlite_user.rs:98-99`. **T:** scratch DB row `1\|admin\|admin\|1\|0` | — | read; T |
| ST-302 | Setup is complete when no `setup_pending = 1` row exists. Public `GET /api/v1/setup/status` returns only `{"setupRequired": bool}`. | `sqlite_user.rs:201-210`; `crates/livrarr-server/src/auth_service.rs:492-495`; `crates/livrarr-handlers/src/setup.rs:8-19`; `crates/livrarr-handlers/src/types/auth.rs:128-132`. **T:** `{"setupRequired":true}` | Exposing more there. | read; T |
| ST-303 | `POST /api/v1/setup` is public and limited to one attempt per 12 s per IP. The handler passes the body to `complete_setup`, which validates, returns `SetupCompleted` if setup is done, hashes, then claims the first pending row conditionally. The first caller wins. | `router.rs:43-48`, `:59-68`; `setup.rs:21-27`; `auth_service.rs:211-279`; `sqlite_user.rs:212-250` | — (the defect) | read |
| ST-304 | That is the only door that creates the first admin. Admin user creation needs an authenticated admin, and `UserDb::complete_setup` has no other production caller. | `auth_service.rs:243`, `:373`; `middleware.rs:96-119`; `rg "\.complete_setup\("` | A second token check. | read |
| ST-305 | `SetupCompleted` maps to 409. An HTTP 401 makes the frontend clear its session and say "Session expired". On any other error code, a JSON `message` reaches the setup form as text. | `crates/livrarr-handlers/src/types/api_error.rs:620-622`; `frontend/src/api/client.ts:60-75`, `:99-111`; `SetupPage.tsx:172-174`, `:317` | 401 for a wrong token. | read |
| ST-306 | The setup page sends `{username, password}` and saves only step, username and root folders to `sessionStorage`. | `frontend/src/types/api.ts:85-88`; `frontend/src/stores/auth.ts:107-108`; `SetupPage.tsx:78-106`, `:131-139` | Saving the token in browser storage. | read |
| ST-307 | Startup order: data folder, config, logging, database (with migrations), auth service, HTTP server. Log output goes to stdout, to `{data}/logs/livrarr.log.<date>` (when file logging initialises; otherwise console and buffer only) and to an in-memory buffer that admins can read. In Docker, stdout is `docker logs`. | `main.rs:134-139`, `:1247-1292`, `:1263-1284`; ST-104 | Printing the token through the log macros. | read; T |
| ST-308 | `livrarr-server` already depends on `getrandom`, `data-encoding` and `subtle`. Existing tokens are `getrandom` bytes, hex-encoded. | `crates/livrarr-server/Cargo.toml:38`, `:41`, `:58`; `crates/livrarr-server/src/auth_crypto.rs:73-77` | A new dependency. | read |
| ST-309 | `ServerAuthService::new` has 12 call sites: `main.rs:962`; `router.rs:765`; `auth_service.rs:553`, `:581`, `:646`; and `tests/behavioral/` `test_irf_u2_import_defer.rs:489`, `test_author_link_doors.rs:63`, `test_irf_u1_refusal.rs:128`, `test_irf_u7_card_hygiene.rs:87`, `test_irf_u5_durable_dismissal.rs:131`, `test_ilr_contracts.rs:6205`, `test_subtitle_matching_preview_door.rs:271`. Only the tests at `auth_service.rs:551` and `:574`, `router.rs:1133` and `test_fresh_author_index.rs:189` drive setup. | unbounded `rg -n "ServerAuthService::new" crates tests`; `rg "/setup"` over `crates`, `tests` | Breaking harnesses that never set up. | read |
| ST-310 | The project rule is "No env var config overrides. TOML only." The one existing exception is `RUST_LOG`. | `/mnt/opt/livrarr/CLAUDE.md:198`; `main.rs:1237-1243` | `LIVRARR_SETUP_TOKEN`. | read |

## 0c. Prior Art

Searched `docs/` (with `design-history/`), `wiki/`, `build/foundation/`, `build/design/`,
`build/plans/`, `build/ops/document-steward/`, and GitHub #119.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `build/foundation/security-model-policy.md:23`, `:31`, `:171`, `:191` | Original token design: 128+ bits, stdout, a file `/config/setup-token` or env `LIVRARR_SETUP_TOKEN`, single use; deferred 2026-07-22 (PO). Kept without the env var (REQ-301/302). `:21` says 403 for a completed setup; code returns 409, unchanged. |
| PA-002 | `build/design/spec-librarr-v2.md:125` (AUTH-010), `:457` | Placeholder admin, atomic claim, 409 once complete; kept (REQ-303). |
| PA-003 | `build/design/spec-librarr-v2.1.md:98`; GitHub #119 | `url_base` intent and the user request; deferred by the PO. |
| PA-004 | `build/design/spec-librarr-v2.md:124` (AUTH-009) | Proxy-login intent, never built; removed by the PO. |
| PA-005 | `docs/architecture-review-2026-07-04.md:92-98` (AR-03); `build/ops/document-steward/ARCHIVE-003.md`, `predecessors/003/SCOPE.diff:1-10` | Retire `docs/ARCHITECTURE.md`. The redirect was written but never reached git (ST-116); REQ-101 lands it. |
| PA-006 | `docs/design-history/root-architecture-r2.md`–`-r4.md`; `provider-rule-2026-09-18/RECEIPT.md`, `MANIFEST.json`; `57189130:ARCHITECTURE.md` | Where the approved sentence survives (ST-108). Root `ARCHITECTURE.md` is snapshotted before each revision; REQ-106 follows. |
| PA-007 | `docs/architecture-review-2026-07-04.md:68-86` (AR-02) | Root `:270` "never automatic" contradicts code; REQ-103 fixes. |
| PA-008 | `build/reports/docs-ops-security-scan-2026-09-26.md:59-80`, `:121`, `:148` | Source list. Citations moved: migrations are under `crates/livrarr-db/`; README lines after 69 are +19; `CHANGELOG.md:317` is now `:345`. |
| PA-009 | `config.rs:208-213`, `:244-251`; `/mnt/opt/livrarr/CLAUDE.md` § Lessons (responsiveness retro) | Config is TOML only. A new test file must be registered and force-added in one change. |

## 1. Problem Statement

1. **Wrong docs.** A user reading the README, or an AI via "Get AI Help", gets false statements.
   Examples: Transmission is never listed, an AudioBookShelf "push" is promised, and dead proxy
   settings are documented ([§2a](#2a-claim-traceability-d1d23)).
2. **Settings that do nothing.** An owner who sets `[auth]` keys or `url_base` as
   `docs/llm-context.md:186` and `:192-194` show gets no effect and no notice (ST-201, ST-203,
   ST-205).
3. **Silent config check.** When a `config.toml` has a mistyped key, starting Livrarr gives no
   warning at all (tested, ST-205).
4. **First-run takeover.** On a fresh install's first start, before the owner opens the setup page,
   anyone who can reach the port can send `POST /api/v1/setup`. The first request wins and gets a
   session and an API key (ST-303).

## 2. Requirements

### Part 1: docs (REQ-D)

No runtime door. The reviewer checks each corrected line against its cited code line. After the
push, the PM checks that the ST-101 raw URL serves the new file.

- **REQ-101** **Delivered**. (D1–D5, D7–D9, D16 in `docs/ARCHITECTURE.md`): replace it with packet 003's
  redirect (PA-005), minus its link into gitignored `build/ops/`. The redirect is titled
  "Architecture — current reference" and links root `ARCHITECTURE.md`, `wiki/index.md` and r1
  (marked superseded). No new snapshot: r1 already holds these bytes (ST-116).
- **REQ-102** **Delivered**. (`README.md`; D5, D6, D7, D15):
  - `:16` → "Grab via qBittorrent, Transmission or SABnzbd"; `:67` and `:111` name all three (ST-106).
  - `:18` → "Enrich metadata from Hardcover, OpenLibrary, Google Books, Goodreads, Audible and
    Audnexus" (ST-107).
  - `:20` → "Hand off ebooks to Calibre-Web Automated (hardlinked into its ingest folder, or copied across filesystems);
    AudioBookShelf and Kavita read Livrarr's library folders directly"
    (`crates/livrarr-server/src/infra/import_pipeline.rs:216-226`; `docs/llm-context.md:172-176`).
  - `:136` → "to never run as root, add `user: "1000:1000"` (pre-`chown` `./config` first). If you
    copied the repository's hardened `docker-compose.yml`, also delete its `cap_add:` block" (ST-114).
- **REQ-103** **Delivered**. (root `ARCHITECTURE.md`; D2, D3, D5, D8, D9, D11). The base is the committed file at
  `37dd4426`, preserved as `root-architecture-r1.md` (REQ-106). Line numbers are at HEAD.
  - `:355` → "Multi-stage build: `node:20-alpine` frontend, `rust:1.94-alpine` builder,
    `alpine:3.21` runtime. The image has a fixed `livrarr` user (1000:1000). Started as root, the
    entrypoint fixes `/config` ownership and drops to `PUID:PGID`. Started as a non-root user, it
    runs as that user." (ST-103, ST-104)
  - `:144`, `:230` → download crate "(qBittorrent, Transmission, SABnzbd, Torznab)" (ST-106).
  - `:144`, `:236` → tagwrite "EPUB tag writing; the M4B and MP3 writers are disabled" (ST-109).
  - `:270` → "Tag writing happens only inside a user-authorized workflow, including import of a
    book the user added; only EPUB files are written" (ST-109). This is new wording; no parked text
    is reused.
  - `:89` (Part 1) stays: it is a principle and claims no M4B writing.
  - `:176` → "Nothing internal"; `:201` → "No internal dependencies; its external crates are listed
    in its manifest" (ST-110).
  - `:117` (Part 1, the factual sentence only) → "Third-party credentials are stored in plain text:
    download-client passwords and API keys, indexer and Prowlarr API keys, the LLM, Hardcover and
    Google Books keys, and the SMTP password. Download-client credentials are not returned by the
    API. Encryption at rest is tracked in #118." (ST-112)
  - D10 is dropped (ST-111).
- **REQ-104** **Delivered**. (`docs/llm-context.md`; D1, D5, D13, D14):
  - `:403` "13 Rust crates" → 17, adding `livrarr-external-data` (provider clients),
    `livrarr-identity`, `livrarr-enrichment` and `livrarr-materialize`; `livrarr-metadata` is
    orchestration (ST-102, ST-107).
  - `:13`, `:24`, `:43`, `:279` → include Transmission (ST-106).
  - `:58-59`, `:237-238` → paths with `{user_id}` (ST-113).
  - `:61` → "the container's `PUID:PGID` user (default 1000:1000), or the `user:` you set" (ST-104).
- **REQ-105** **Delivered**. (D12, `docs/llm-context.md:180-203`; follows Part 2):
  - Remove the `[auth]` block (`:192-194`).
  - Replace the `url_base` line (`:186`) with a note that serving Livrarr under a sub-path is not
    supported yet: the key is accepted and has no effect (ST-201, ST-202). No doc describes another
    proxy route (`rg -i "subdomain|reverse proxy"` over `README.md`, `docs/llm-context.md` and
    `wiki/deployment` finds only `:186`, `:193`), so no instructions are added.
  - Add `[server] trusted_proxies` with one line on its effect (ST-204).
  - `:201` lists the real sections (`[server]`, `[log]`, `[convergence]`, `[metadata_cache]`,
    `[author_link]`; `config.rs:17-35`) instead of "[server], [log] and [auth] … nothing else".
  - `:52` changes with REQ-305.
- **REQ-106** **Delivered**. (approved provider sentence; PO, 2026-10-01). In root `ARCHITECTURE.md` § "Providers
  Are Interchangeable", the sentence at `:66` "Adding a new provider means implementing the trait
  contract — nothing else changes." is replaced by: "Keep the details of talking to each
  book-information service in its own integration. Adding a service may also require telling
  Livrarr what it provides and when to use it." (`MANIFEST.json` `approved_wording`;
  `57189130:ARCHITECTURE.md:90`). The rest of `:64` and `:66` is unchanged, and nothing else from
  `57189130` comes back (ST-108). No new snapshot: the tracked `docs/design-history/root-architecture-r1.md` is
  byte-identical to the committed file (ST-108). In the same change, `docs/design-history/README.md`
  gains a line naming r1 as the predecessor of this edit and of REQ-103.

- **REQ-107** **Delivered**. (D17–D23, `docs/llm-context.md`, added by the PO):
  - D17 `:208-225` → the service block from `docker-compose.yml:1-32`: its environment
    (`PUID`, `PGID`, `TZ`), `cap_add` (CHOWN, SETUID, SETGID, DAC_OVERRIDE), limits and health
    check, with that file's image tag (ST-121, ST-122).
  - D18 `:231`, `:333` → `{data_dir}/logs/livrarr.log.<YYYY-MM-DD>`, a new file each day (ST-117).
  - D19 `:203` → "There is no config.yaml and no environment-variable override of `config.toml`.
    The container reads `PUID` and `PGID`, and `RUST_LOG` replaces the log filter. Command-line
    options: `--data`, `--ui-dir` and the `identity-cutover` subcommand." (ST-118)
  - D20 `:399` → "Login is rate limited per IP: a burst of 5 attempts, then one more every 12
    seconds. Every API route also has a per-IP limit. A username locks for 15 minutes after 5
    failed logins." (ST-119)
  - D21 `:360` → "Authenticate with an `X-Api-Key: <key>` header, or with `Authorization: Bearer
    <token>` using the session token from login." (ST-120)
  - D22 `:211` → the README's tag, `0.1.0-alpha6` (ST-122). This is the same edit as D17.
  - D23 `:412` → the README's invite, `https://discord.gg/PJDsgjEvCV` (ST-122).

### Part 2: settings (REQ-S)

- **REQ-201** **Delivered**: no code change, as specified. Making `url_base` work is **deferred** to the backlog (#119; PO, 2026-10-01: "skip it for now - re add to back log"). (`url_base`, deferred by the PO): no code change. Its field, slash check and
  known-key entry stay (`config.rs:52-53`, `:321-339`, `:419`), so a user who has it set sees no
  change and no warning. The docs change per REQ-105.
- **REQ-202** **Delivered**. (`[auth]` keys, removed by the PO): delete `AuthConfig` and `AppConfig.auth`
  (`config.rs:21-22`, `:86-95`), the CIDR check (`:341-349`), `parse_cidr` (`:390-408`, no other
  caller) and `"auth"`/`KNOWN_AUTH` from the key lists (`:418`, `:420`, `:445-451`).
  `AuthType::ExternalAuth` is left alone. The docs change per REQ-105.
- **REQ-203** **Delivered**, after a second cause the spec missed ([§7](#corrections-found-during-the-build)). (warnings reach the log): `load_config` returns the unknown keys, and they are logged
  at WARN after `init_tracing` (`main.rs:721-734`). `warn_unknown_keys` (only caller
  `main.rs:1204`) becomes a function that returns the list. The known-key lists name every key the
  loader reads. Add `trusted_proxies` to the server list. Add `metadata_cache` to the root list,
  with a child check for `ttl_days` and `max_rows` like the other sections (`config.rs:214-237`).
  Observable: one `Unknown config key: <key>` line per unknown key whenever the configured level
  shows warnings, including the default (`info`). No such line for a key the loader reads. Door:
  the built binary (ST-208). A level that hides warnings hides these too, by design (§4).
- **REQ-204** **Delivered**. (upgrade with removed keys): a `config.toml` that still has `[auth]` starts normally
  and logs `Unknown config key: auth`. A bad `[auth] trusted_proxies` entry no longer stops
  startup, as it does today (`config.rs:341-349`). Nothing else changes, because the keys never did
  anything (ST-203). Door: the built binary.

### Part 3: first-run setup token (REQ-B)

The database's `setup_pending` flag is the only authority on whether setup is open (ST-302). The
token file and the in-memory token are means of proof, never of state.

- **REQ-301** **Delivered**, with two rules made exact in review ([§7](#corrections-found-during-the-build)). (token creation): at startup, after the database is ready and before the HTTP server
  binds (`main.rs:136-139`), if setup is pending:
  1. Look at `{data}/setup-token` without following links (`symlink_metadata`). If it is a regular
     file holding 32 lowercase hex characters, reuse that token. In every other case (missing,
     malformed, a symbolic link, a directory, another file type), make a new one: 16 bytes from
     `getrandom`, hex-encoded. Nothing is ever read or written through a link.
  2. **Write the active token privately on every start.** Create a new temporary file in the data
     folder with create-new and owner-only permissions (0600 on Unix), write the token, then rename
     it onto `{data}/setup-token`. The rename replaces a link or an old file's directory entry and
     never writes into its target. So the file is private after every start, whatever was there
     before. The contents of a file never inspected for mode or owner are reused only after this
     rewrite.
  3. **Fail closed.** If the private write or the rename fails (for example the path is a
     directory, or the folder is not writable), Livrarr does not serve setup: it prints
     `Cannot write the setup token file <path>: <error>. Fix the data folder and restart.` (no
     token) to stderr and exits 1 before binding. The owner sees that message in `docker logs`
     and a container that stops.
  4. Print a banner to **stdout directly, not through the log macros**: first-run setup needs this
     token, then the token, then the file path. Log one INFO line naming the file without the
     token. Give the token to the auth service, which holds it in memory.
  If setup is complete, there is no token and no banner. A leftover `{data}/setup-token` is removed
  as a **best effort**: a failure logs a WARN without the token, and startup continues. A restart
  before setup reuses the token. Residual: a token that sat in a readable file before an earlier
  start stays valid; whoever could read it could also read the data folder's database.
- **REQ-302** **Delivered**, through a new claim method ([§7](#corrections-found-during-the-build)). (the check): `SetupRequest` gains `setupToken` (camelCase,
  `crates/livrarr-handlers/src/types/auth.rs:95-110`), redacted in `Debug`.
  `ServerAuthService::complete_setup` checks it after the "setup complete" check and before
  hashing (`auth_service.rs:220-224`), trims surrounding whitespace, and compares in constant time
  (`subtle`).
  - Missing, empty, whitespace-only or wrong → a new error that maps to **403** with the fixed
    message "The setup token is missing or wrong. Find it in Livrarr's startup output, or in the
    file setup-token in its data folder (/config/setup-token in Docker)." The body never contains
    the token. Setup stays pending.
  - Right → the existing conditional database claim runs (`sqlite_user.rs:231-250`). As soon as it
    commits, the service drops its in-memory token, before session creation or the response. It
    then removes `{data}/setup-token` as a best effort: a failure logs a WARN without the token and
    never fails the request or reopens setup. The 200 body holds only the existing `token`
    (session) and `apiKey` (`types/auth.rs:112-117`), never the setup token. If a step after the
    claim fails, the account still exists and the owner signs in normally. A repeat request gets
    the existing 409.
  - A service built without a token refuses setup with 403 (fail closed).
  - The per-IP limit stays. This is the only door (ST-304), and two simultaneous right-token
    requests still produce one winner through the conditional claim.
- **REQ-303** **Delivered**. (existing installs): an install that already finished setup gets no banner and no
  token. `setup/status` is `{"setupRequired":false}`; `POST /setup` with a valid body is 409 as
  before. Sessions and API keys keep working across the upgrade restart. A stale-file cleanup
  failure never stops it starting.
- **REQ-304** **Delivered**. (where the token appears): only in the stdout banner and in `{data}/setup-token`.
  Never in an API response (status, success or error), in `{data}/logs/livrarr.log.*` when file
  logging is active (ST-307), or in the in-memory log read by the Logs and Help pages.
- **REQ-305** **Delivered**. (setup page and docs): the account step (`SetupPage.tsx:280-316`) gets a required
  "Setup token" field with the text "Livrarr printed a one-time setup token when it started. Run
  `docker logs livrarr`, or open the file `setup-token` in your config folder." The token is sent
  in the body (`stores/auth.ts:107-108`, `types/api.ts:85-88`), never in `sessionStorage`,
  `localStorage` or the URL. Server messages show in the error line (`SetupPage.tsx:317`).
  `README.md:61` and `docs/llm-context.md:52` say the same in place of "no pre-seeding required".
- **REQ-306** **Delivered**. (no environment variable): no environment variable carries or sets the token.
  Scripted installs read the file (ST-310, PA-009).

## 2a. Claim traceability (D1–D23)

"Still wrong" means re-checked at `37dd4426`.

| # | Doc line(s) at HEAD | Still wrong? | Code proof | Requirement |
|---|---|---|---|---|
| D1 | `docs/ARCHITECTURE.md:3`; also `docs/llm-context.md:403` | Yes | ST-102 | REQ-101, REQ-104 |
| D2 | `ARCHITECTURE.md:355`; `docs/ARCHITECTURE.md:72` | Yes | ST-103 | REQ-103, REQ-101 |
| D3 | same lines | Yes | ST-104 | REQ-103, REQ-101 |
| D4 | `docs/ARCHITECTURE.md:68` (root `:349` correct) | Yes | ST-105 | REQ-101 |
| D5 | `README.md:16`, `:67`, `:111` (scan `:92`); `ARCHITECTURE.md:144`, `:230`; `docs/ARCHITECTURE.md:45`; `docs/llm-context.md:24`, `:43`, also `:13`, `:279` | Yes | ST-106 | REQ-102, REQ-103, REQ-101, REQ-104 |
| D6 | `README.md:20` | Yes | `rg -il audiobookshelf` over `crates`, `frontend/src`: only `crates/livrarr-matching/src/m2_path.rs` (comment) and `frontend/src/pages/system/about/AboutPage.tsx` | REQ-102 |
| D7 | `README.md:18`; `docs/ARCHITECTURE.md:44` | Yes | ST-107 | REQ-102, REQ-101 |
| D8 | `ARCHITECTURE.md:144`, `:236`, `:270`; `docs/ARCHITECTURE.md:48`, `:63`; root `:89` kept | Yes (not `:89`) | ST-109 | REQ-103, REQ-101 |
| D9 | `ARCHITECTURE.md:176`, `:201`; `docs/ARCHITECTURE.md:41` | Yes | ST-110 | REQ-103, REQ-101 |
| D10 | `ARCHITECTURE.md:186`, `:189`, `:190` | **No, dropped** | ST-111 | — |
| D11 | `ARCHITECTURE.md:117` | Yes | ST-112 | REQ-103 |
| D12 | `docs/llm-context.md:186`, `:192-194`, and `:201` | Yes | ST-201–ST-204 | REQ-105 |
| D13 | `docs/llm-context.md:58-59`, also `:237-238` | Yes | ST-113 | REQ-104 |
| D14 | `docs/llm-context.md:61` | Yes | ST-104 | REQ-104 |
| D15 | `README.md:136` (scan `:117`) | Yes | ST-114 | REQ-102 |
| D16 | `docs/ARCHITECTURE.md:43` | Yes | ST-115 | REQ-101 |
| D17 | `docs/llm-context.md:208-225` (Docker example) | Yes | ST-121 | REQ-107 |
| D18 | `docs/llm-context.md:231`, `:333` | Yes | ST-117 | REQ-107 |
| D19 | `docs/llm-context.md:203` | Yes | ST-118 | REQ-107 |
| D20 | `docs/llm-context.md:399` | Yes | ST-119 | REQ-107 |
| D21 | `docs/llm-context.md:360` | Yes | ST-120 | REQ-107 |
| D22 | `docs/llm-context.md:211` | Yes | ST-122 | REQ-107 |
| D23 | `docs/llm-context.md:412` | Yes | ST-122 | REQ-107 |

## 2b. Files per part

- **Part 1:** `README.md`, `ARCHITECTURE.md`, `docs/ARCHITECTURE.md`, `docs/llm-context.md`, plus
  an index line in `docs/design-history/README.md` naming the existing r1 snapshot.
- **Part 2:** `crates/livrarr-server/src/config.rs` and `main.rs`; `docs/llm-context.md` content;
  tests in `crates/livrarr-server/tests/test_fresh_author_index.rs`.
- **Part 3:** `crates/livrarr-server/src/auth_service.rs` (and its tests `:551`, `:574`) and
  `main.rs`; `crates/livrarr-handlers/src/types/auth.rs` and `types/api_error.rs`; the `router.rs`
  tests (`:1133` and new cases); `test_fresh_author_index.rs` (harness `:189`, new cases);
  `frontend/src/pages/setup/SetupPage.tsx`, `stores/auth.ts`, `types/api.ts` and a frontend test;
  `README.md:61` and `docs/llm-context.md:52`.
- **Shared:** `README.md` (Parts 1 and 3), `docs/llm-context.md` (1, 2, 3), `main.rs` (2, 3) and
  `test_fresh_author_index.rs` (2, 3). Suggested order: the Part 2 and 3 code first, then one
  Part 1 doc pass.

## 3. UI/Interface Design

The setup page's account step gets a required "Setup token" field (REQ-305). The console shows a
banner while setup is pending. Nothing else changes.

## 4. Non-Requirements

No `url_base` code change. No `[server] trusted_proxies` validation change. No second output
path for config warnings: a `[log] level` or `RUST_LOG` that hides warnings hides these too, by
design. No change to
`ExternalAuth`, password rules, login or the 409. No setup time window, no loopback-only setup,
no token in the URL. No doc checker, no new user docs, no #118 or #76 work. Nothing from
`57189130` except the REQ-106 sentence.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | DS1 `url_base`: make it work or remove it. | closed — PO 2026-10-01: "skip it for now - re add to back log" | Deferred. Making it work was sized at 2–4 days. Docs say "not supported yet" (REQ-201, REQ-105). |
| Q-002 | DS2 `[auth]` keys: make them work or remove them. | closed — PO 2026-10-01: "Remove them (Recommended)" | Removed (REQ-202). Rejected option: 1–2 days plus a security review, and it needed cross-site request protection. |
| Q-003 | DS3: what a user with a removed key sees on upgrade. | closed — spec | A warning; startup continues (REQ-204). |
| Q-004 | DD1: which claims are still wrong. | closed — spec; PO 2026-10-01 added D17–D23 | 15 of 16 (D10 dropped), plus D17–D23 (§2a). |
| Q-005 | DD2: the supersede rule for the two `docs/` files. | closed — spec | `docs/ARCHITECTURE.md`: met by r1 (ST-116). `docs/llm-context.md`: user help, not a design record. Root file: already preserved as `root-architecture-r1.md` (REQ-106). |
| Q-006 | DB1: the token design. | closed — spec | Console plus file, kept across restarts, deleted on success, 128 bits (PA-001; Jenkins uses the same pattern, not re-checked). A time window (Portainer-style, not verified) was rejected because the first visitor in the window still wins. Loopback-only was rejected because it fails in Docker. |
| Q-007 | DB2: how the token travels, and the message. | closed — spec | JSON body `setupToken`; 403 with the REQ-302 message (not 401, ST-305). |
| Q-008 | DB3: the env var against the TOML-only rule. | closed — spec | Dropped; the file covers scripts (REQ-306). |
| Q-009 | The base for root `ARCHITECTURE.md`. | closed — PO 2026-10-01: "Add the one sentence (Recommended)" | Committed file plus one sentence (REQ-106). |

## 6. Acceptance Criteria

Red means the case must fail on `37dd4426` before the fix. Guard means it passes before and after.

As built, every criterion is met; the PM ran AC-101's post-push check on 2026-10-03. The test
for each criterion, and where one moved, is in [§7](#acceptance-as-built).

- [x] **AC-101** (REQ-101–107): the reviewer checks each §2a row. Each doc line agrees with its code
  line, D10 is unchanged, and `docs/ARCHITECTURE.md` is the redirect with working tracked links.
  The design-history index names `root-architecture-r1.md` (sha256 `1b0b596f…`, equal to
  `37dd4426:ARCHITECTURE.md`) as the predecessor. In § "Providers Are Interchangeable", only the
  `:66` sentence changed. The D17 example matches `docker-compose.yml:1-32` key for key. D20 matches
  `burst_size(5)` and the 12-second period (`router.rs:31-37`) and promises no five-per-minute cap.
  D6 says hardlink, with a copy across filesystems. After the push, the ST-101 raw URL serves the
  new `docs/llm-context.md`.
- [x] **AC-201** (REQ-203, red): in `crates/livrarr-server/tests/test_fresh_author_index.rs` (or a
  new file registered and force-added in the same change), start the built binary at the harness's
  level with a config holding every valid section and key, including `[server] trusted_proxies`
  and `[metadata_cache] ttl_days` and `max_rows`, plus `[server] not_a_key`,
  `[metadata_cache] ttl_day` and an unknown `[no_such_section]`. The output names exactly those three
  (`server.not_a_key`, `metadata_cache.ttl_day`, `no_such_section`), each once (red: today none),
  and nothing for a valid key.
- [x] **AC-202** (REQ-203, red): the same misspelled key with **no `[log]` section** (default
  `info`). The warning appears once. This is the default-level case.
- [x] **AC-203** (REQ-202/204, red): `[auth]` holding `external_header = "X-Remote-User"` and
  `trusted_proxies = ["not-a-cidr"]`. The server becomes ready (red: today it exits on the bad
  CIDR) and names `auth` as unknown once. `[server] url_base = "/livrarr"` produces no unknown-key
  line (guard, REQ-201).
- [x] **AC-301** (REQ-302, red): the router harness in the `router.rs` tests (real `build_router`,
  real `SqliteDb`, service built with a known token as `main` wires it), with a distinct peer IP per
  request. Missing, empty, whitespace-only and wrong tokens each → 403 with the exact REQ-302
  message, and a serialized body that does not contain the token (red: 200 today). After them,
  `setup/status` is still `true`, and the attacker's username and password cannot log in. The right
  token → 200 whose body has only `token` and `apiKey`, both usable (`/auth/me` with each), and no
  setup token. A second right-token request → 409. The existing test at `:1133` sends the right
  token and still sees 429.
- [x] **AC-302** (REQ-302): (a) **red**: a service built without a token refuses a valid first
  setup through the real router with 403 (today it returns 200). (b) **guard**: two simultaneous
  right-token requests from distinct IPs give exactly one 200 and one 409.
- [x] **AC-303** (REQ-301, REQ-304, red): the binary harness on a fresh data folder. `setup-token`
  holds 32 hex characters (mode 0600 on Unix), and the token appears in stdout but not in
  `logs/livrarr.log.*`. Restart: the same token. Wrong and missing tokens → 403 without the token
  in the body; vary the client address via `[server] trusted_proxies = ["127.0.0.1"]` and
  `X-Real-IP` to respect the per-IP limit. Then the right token → 200 and the file is gone. The
  token is not in `GET /setup/status` or `GET /system/logs/tail`. Restart: no banner, no file.
- [x] **AC-304** (REQ-301, red): with setup pending, before start: (a) a valid token in a
  world-readable file → the server becomes ready, the file is 0600, and the token is the same;
  (b) an invalid token in a world-readable file → a new token in a 0600 file; (c) `setup-token` a
  symbolic link to another file → after start the path is a regular 0600 file, and the link's
  target is unchanged; (d) `setup-token` a non-empty directory → the process exits 1 before
  readiness with the REQ-301 message and no token in its output.
- [x] **AC-305** (REQ-301–303), the binary harness. After start with setup pending, replace
  `setup-token` with a non-empty directory, then set up with the right token.
  - **guard:** 200, the account is claimed, a second attempt → 409, and on restart normal login
    works with no banner.
  - **red:** the setup logs a cleanup WARN without the token. The restart with the directory still
    present logs a new cleanup WARN without the token and becomes ready.
  - **red** (recovery): stop, replace the directory with a regular stale token file, and assert that
    the file exists just before launch. After start, the file is gone, there is no banner,
    `setupRequired` is `false`, and normal login works. Cleanup never deletes a directory
    recursively.
- [x] **AC-306** (REQ-303, guard): the binary harness. Setup completed, then restart: no file, no
  banner, `{"setupRequired":false}`, and `POST /setup` with a valid body and no token → 409. The
  session token and API key from setup still pass `/auth/me`.
- [x] **AC-307** (REQ-305, red): a frontend test mounting the real `App` through
  `installApiStub` (`frontend/src/test-support/apiStub.tsx:47`, which replaces `globalThis.fetch`;
  `@/api`, the auth store and the route guards stay real). The stub answers `setup/status` with
  `setupRequired: true`. Submitting the form sends `POST /api/v1/setup` with `setupToken` in the
  JSON body. A 403 JSON reply's message is shown with no "Session expired" transition. The token is
  never in `sessionStorage`, `localStorage` or the URL, after either failure or success.
- [x] **AC-308**: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` with zero
  warnings, the `livrarr-server` tests and the touched test binaries, plus the frontend tests for
  the setup page and the typecheck.

## 7. As built: corrections and limits

The feature was built by two coders in one checkout (Parts 2 and 3 code, Part 1 docs), reviewed
twice by GPT-6 Astra (r1 FAIL on three P2 and two P3 findings, r2 PASS with one P3 folded as a
test-only change), deployed by hand on 2026-10-02 and checked live by the PO the same day. The PM
reports the PO's verdict, "tested both all good": the existing install on port 8789 was unchanged,
and the fresh-install token flow on port 8799 worked. The PM then confirmed that the scratch
install's `setup-token` file was gone and its setup was complete
(`build/reviews/prerelease-trust-pass/deploy-20261002T022337Z/LIVE-CHECK.md`, `DEPLOYMENT.json`).
Committed locally as `d77f690a` (code and tests) and `c70dca4e` (docs).

### Corrections found during the build

What the spec got wrong or left out, as the builders and the reviewer found it. None of these
changed a decision.

- **ST-205 missed a second cause (found by the coder).** The unknown-key check parsed the file
  with `raw.parse::<toml::Value>()` (`37dd4426:crates/livrarr-server/src/main.rs:1203-1205`). In
  toml 1.1, `Value`'s `FromStr` reads one TOML value, not a document (`toml-1.1.2`
  `src/value.rs:395-400`, which calls `ValueDeserializer::parse`, "Parse a TOML value",
  `src/de/deserializer/value.rs:48-49`). A real `config.toml` therefore failed to parse there, and
  `if let Ok` skipped the check silently, so moving the warnings after logging alone would still
  have shown nothing. The check now parses a `toml::Table` (`crates/livrarr-server/src/main.rs:1266-1268`;
  `toml-1.1.2` `src/table.rs:53-58`, which is `toml::from_str`), and `unknown_keys` takes that table
  (`crates/livrarr-server/src/config.rs:389`). Evidence:
  `build/reviews/prerelease-trust-pass/packet-3a-code/evidence-toml-value-parse.log`, a mid-build
  run with the old parse still in place, in which the four config tests fail with no "Unknown
  config key" line. They pass in `packet-3a-code/green-workspace.log`. The PM confirmed the crate
  behaviour at source.
- **REQ-302's "as soon as it commits" needed a new database method (review r1, R1).** The old
  `UserDb::complete_setup` ran the conditional claim and then read the user back. If that read
  failed, the account was claimed but the service still held the token. The new
  `UserDb::claim_setup` returns the claimed id once the update commits
  (`crates/livrarr-db/src/api/user.rs:45-52`; `crates/livrarr-db/src/sqlite_user.rs:212-251`); the
  service drops its token straight after it (`crates/livrarr-server/src/auth_service.rs:301-313`)
  and builds the session from the id. `complete_setup` is kept as claim plus read-back
  (`sqlite_user.rs:253-256`) for other callers. Test:
  `setup_token_is_consumed_when_a_step_after_the_claim_fails` (`auth_service.rs:653`), which after
  the N1 fold also pins that the owner then signs in normally.
- **REQ-301(1) is an exact 32-byte file (review r1, R2).** A file is reused only when its whole
  contents are exactly 32 lowercase hex bytes. Trailing whitespace, a newline or anything else
  means a new token (`crates/livrarr-server/src/setup_token.rs:104-124`). This differs on purpose
  from the request check, which trims the offered token (REQ-302). Test:
  `setup_token_file_with_anything_after_the_token_is_replaced`
  (`crates/livrarr-server/tests/test_fresh_author_index.rs:1571`).
- **Non-Unix targets refuse setup (review r1, R3; PM call within REQ-301(3)).** Without a
  no-follow open there is no file reuse, and without owner-only creation the private write fails,
  so startup exits before binding with the REQ-301 message (`setup_token.rs:135-139`,
  `:188-201`). Livrarr ships Linux images only (`.github/workflows/release.yml:26-29`); macOS is
  Unix and keeps the protections.
- **AC-301 moved to the binary harness (tests review r1, R1).** The router harness could not hold a
  token without a production change, so the refusal and success matrix runs against the built
  binary and its real `setup-token` file. AC-302(a) stays in the router harness on an explicitly
  tokenless builder.
- **Files beyond §2b.** `crates/livrarr-server/src/setup_token.rs` (new; token issue, private
  write and removal) and its `lib.rs` entry; `crates/livrarr-db/src/api/user.rs` and
  `sqlite_user.rs` (`claim_setup`); `frontend/src/pages/setup/SetupPage.token.test.tsx` (new,
  tracked).
- **Review r1 R4 and R5.** R4 (test-edit boundary) needed no change. R5: the docs trace citations
  were refreshed after the code fix (`packet-3b-docs/TRACE.md`).

### Acceptance as built

B is `crates/livrarr-server/tests/test_fresh_author_index.rs`, R is
`crates/livrarr-server/src/router.rs`, A is `crates/livrarr-server/src/auth_service.rs`, F is
`frontend/src/pages/setup/SetupPage.token.test.tsx`.

| AC | Tests | State |
|----|-------|-------|
| AC-101 | Astra checked each §2a row in code review r1 and r2; trace in `packet-3b-docs/TRACE.md`. After the 2026-10-03 push the PM fetched the ST-101 raw URL: HTTP 200, sha256 `8128fe82…` equal to the local and `origin/main` file (`post-push-llm-context.md`) | Met |
| AC-201 | B:909 `config_warnings_name_each_unknown_key_once_and_no_valid_key`; B:980 `config_warning_shows_at_the_warn_log_level` | Met |
| AC-202 | B:939 `config_warning_shows_at_the_default_log_level` | Met |
| AC-203 | B:959 `removed_auth_section_warns_and_starts_while_url_base_stays_silent`; guard B:1001 `url_base_alone_starts_and_names_no_unknown_key` | Met |
| AC-301 | B:1248 `setup_refuses_missing_empty_blank_and_wrong_tokens_then_accepts_a_padded_right_one_once`; limiter guard R:1161 `setup_route_burst_of_one_blocks_a_second_immediate_request` | Met (moved to B, above) |
| AC-302 | R:1266 `setup_is_refused_when_the_auth_service_holds_no_token`; R:1292 `two_simultaneous_right_token_setups_have_one_winner` | Met |
| AC-303 | B:1342 `fresh_install_writes_private_setup_token_reused_on_restart_and_single_use`; B:1453 `two_fresh_installs_get_different_setup_tokens`; B:1467 `setup_token_appears_only_in_the_stdout_banner_with_verbose_logging`; A:612 `setup_request_debug_never_shows_the_password_or_setup_token` | Met |
| AC-304 | B:1544, B:1558, B:1571, B:1610, B:1624, B:1645 (valid and invalid world-readable files, trailing data, private file still replaced, symbolic link, directory) | Met |
| AC-305 | B:1723 `setup_completes_when_the_token_file_cannot_be_removed`; B:1759 `setup_warns_without_the_token_when_the_token_file_cannot_be_removed`; B:1787 `restart_warns_when_a_stale_token_directory_cannot_be_removed`; B:1818 `restart_after_setup_removes_a_stale_token_file` | Met |
| AC-306 | B:1853 `finished_install_keeps_session_and_api_key_and_serves_no_token_after_restart` | Met |
| AC-307 | F:190 | Met |
| AC-308 | PM re-runs: fmt and clippy clean, workspace 2438 passed and 0 failed (`packet-5-code-fix-r1/pm-green-workspace.log`), then `verify.py code` PASS after the N1 fold (`pm-verify-code.log`) | Met |

### Limits

Recorded and accepted; none is a known defect.

- **The non-Unix branches were never compiled on this host** (only `x86_64-unknown-linux-gnu` is
  installed). Review r2 closed R3 by reading the source only.
- **The corrected AI help lives on GitHub `main`.** The Help page sends the user's AI to
  `docs/llm-context.md` there (ST-101), so it reaches users only once pushed. The PM confirmed
  after the 2026-10-03 push that the raw URL serves the corrected file (AC-101).
- **Residual from REQ-301 stands:** a token that sat in a readable file before an earlier start
  stays valid.
- **`url_base` still does nothing.** Its key is accepted without a warning; making it work is on
  the backlog (#119).
