---
feature: security-before-release
stage: spec
status: delivered
version: 5
type: bugfix
req_ids: [REQ-101, REQ-102, REQ-104, REQ-105, REQ-106, REQ-107, REQ-201, REQ-202, REQ-203, REQ-301, REQ-302]
---

# Bug Spec: security-before-release

## Executive summary

**Delivered.** All three parts were built, passed code review on the third round, were deployed
to the local app and checked by the PO on 2026-10-05, and were committed locally as `497b391e`
(not yet pushed). Every requirement is delivered and no decision changed. Code review found four
log-filter shapes this spec had not named; they are fixed and pinned as new criteria
([AC-109 to AC-112](#added-in-code-review)). Three more shapes the review found are accepted
limits by PO decision ([§4](#4-non-requirements)). The [as-built section](#7-as-built-corrections-and-limits)
records what the spec missed and the limits of the evidence: the new masking cases are pinned by
unit tests on the cleanser, and the live check showed that routine running leaks no saved
secret, not that each masking path works live.

Three security fixes before the release, approved by the PO on 2026-10-05. Each part is under
a day of work, with no schema change, no new dependency and no function or trait signature
change. The PO settled how far the log filter goes on 2026-10-05 ("1 only"); the other open
choices are settled in [§5](#5-open-questions) with a recommendation each.

- **Part 1: secrets kept out of every log (GitHub #76).** Today a secret reaches the logs
  when a call site prints it. Tested: calling the admin route that tests the AI (LLM) connection
  (`POST /api/v1/config/metadata/test/llm`; the web app has no button for it), when the
  provider refuses the key and echoes it back, writes the key to the console (Docker logs),
  the log file and the in-app Logs page, in both text and JSON log formats, whatever the key's
  length ([ST-103](#0b-system-truths), [ST-113](#0b-system-truths)). A secret-shaped
  unknown key in `config.toml` reaches the same three places at startup
  ([ST-111](#0b-system-truths)). After the fix, every log line passes through one cleanser
  before it is written, as Sonarr does. It masks secret-shaped text (URL keys, passwords,
  `Authorization` headers, JSON fields). It works on logs only: the warnings and errors
  Livrarr returns to the web app keep today's text. The AI connection test also blanks the key
  it just sent out of the provider's reply before logging the reply, however the reply's JSON
  spells it, so the admin keeps the rest of the reply. **Accepted limit (PO):** a reply that quotes a password back in plain
  prose at some other place is not caught, because patterns cannot recognise it; Sonarr has
  the same limit ([§4](#4-non-requirements)). The first-run setup token keeps its one
  designed place, the console banner ([REQ-101](#part-1-secrets-kept-out-of-every-log-req-l)
  to [REQ-107](#part-1-secrets-kept-out-of-every-log-req-l)).
- **Part 2: Readarr import undo deletes only inside the library.** Pressing "Undo" on a
  finished Readarr import deletes each imported file by joining the stored path to the
  import's root folder, with no check. Tested: a stored path with `..`, an absolute path
  outside the library, or a path through a linked folder deletes a file outside the library;
  when the root folder cannot be read, the bare path is deleted relative to the server's
  working folder ([ST-203](#0b-system-truths), [ST-205](#0b-system-truths)). After the fix,
  undo uses the same removal rule as book delete, with each file's own library folder. Files
  it may not remove are left, counted as skipped, and the Readarr import page warns how many
  were left ([REQ-201](#part-2-readarr-import-undo-deletes-only-inside-the-library-req-u) to
  [REQ-203](#part-2-readarr-import-undo-deletes-only-inside-the-library-req-u)).
- **Part 3: manual import Retry reaches the download client.** Pressing "Retry" on a failed
  import whose grab has no saved content path, with qBittorrent configured as
  `http://qbittorrent:8080`, fails, because this lookup uses the web client built for outside
  addresses, which refuses any host name that resolves to a private address. Tested with
  `localhost` ([ST-302](#0b-system-truths)). The background poller makes the same lookup
  through the trusted client and works. After the fix, Retry uses the trusted client too
  ([REQ-301](#part-3-manual-import-retry-uses-the-trusted-client-req-c),
  [REQ-302](#part-3-manual-import-retry-uses-the-trusted-client-req-c)).

**Tested vs read.** Fifteen truths were checked by scratch probes that drove the real code or,
for ST-115, the real JSON library (`build/reviews/security-before-release/packet-0-spec/probes.log`, `probes-v2.log` and
`probes-v3.log`); the rest were read at `0e06a205`. Non-Unix builds were not compiled ([§4](#4-non-requirements)).
**Found while probing, not in scope:** both web clients forward the full SABnzbd address,
API key included, to wherever a SABnzbd reply redirects them (in the `Referer` header). The
poller has the same exposure today. It is listed for a separate decision
([ST-304](#0b-system-truths), [§4](#4-non-requirements)).

Sections: [revision](#revision), [truths](#0b-system-truths), [prior art](#0c-prior-art),
[problems](#1-problem-statement), [requirements](#2-requirements),
[input shapes](#2a-input-shapes), [files](#2b-files-per-part),
[non-requirements](#4-non-requirements), [decisions](#5-open-questions),
[acceptance](#6-acceptance-criteria), [as built](#7-as-built-corrections-and-limits).

## Revision

v5, 2026-10-05: as built, per
`build/reviews/security-before-release/packet-10-as-built/PACKET.md`. v4 is kept at
`build/reviews/security-before-release/packet-0-spec/spec-security-before-release-v4.md`
(sha256 `ee3468e56de1934f9958efec8492b28cf2a13fd0b84d9c598e3244f8e4d2fc07`). Status
`delivered`. Sources: the commit `497b391e`; the hand-backs `packet-4-code/HANDBACK-4.md`,
`packet-6-code-fix-r1/HANDBACK-6.md` and `packet-8-code-fix-r2/HANDBACK-8a.md`; GPT-6 Astra's
code reviews `packet-5-code-review/REVIEW-astra-code-r1.md` (FAIL: C1, C2, C3),
`packet-7-code-review-r2/REVIEW-astra-code-r2.md` (FAIL: D1, D2, D3, D4) and
`packet-9-code-review-r3/REVIEW-astra-code-r3.md` (PASS); `deploy-20261005T175224Z/`
(`DEPLOYMENT.json`, `LIVE-CHECK.md`, `secret-scan.txt`); and the PO decisions in
`build/state/security-before-release.yaml`. No requirement, criterion or decision changes.

- Each requirement is marked Delivered.
- AC-109 to AC-112 are new ([added in code review](#added-in-code-review)): the shapes code
  review fixed (C1, C2, C3, D4), each with its test.
- §4 gains one accepted-limit shape from review round 3 (a literal escape spelling of `/`, `?`
  or `#` inside a text-format URL password) and states the URL rule that a `/` before `@` means
  there is no userinfo.
- New [§7](#7-as-built-corrections-and-limits): what the spec missed and the limits of the
  evidence.

v4, 2026-10-05: folded per
`build/reviews/security-before-release/packet-8-code-fix-r2/PACKET-b-spec.md`. v3 is kept at
`build/reviews/security-before-release/packet-0-spec/spec-security-before-release-v3.md`.

- **PO decision of record (2026-10-05, "3").** Code review round 2
  (`packet-7-code-review-r2/REVIEW-astra-code-r2.md`) found three secret shapes the log
  cleanser misses, D1 to D3. The PO chose to ship the current cleanser with only D4 fixed and
  to accept D1 to D3 as known limits. §4 gains an **Accepted limits** entry naming each, with
  the review's example and the log format it affects (checked against the review's formatter
  probe: D1 and D2 leak in text format only, D3 in both). §4 also records the deferred
  type-based design as not approved.
- **Pointers the fold forces** (no other change to any requirement, criterion or shape):
  REQ-102's JSON-member bullet (D3), its structured-field bullet (D1) and its "masked at least
  up to its first …" sentence (D1, D2) each end "except the accepted limits in §4". AC-102
  says the same after "every pattern shape of REQ-102 and §2a". In §2a, the `user:pass@` row
  (D2), the JSON-body row (D3) and the structured-field row (D1) say the same in their After
  column.

v3, 2026-10-05: folded per `build/reviews/security-before-release/packet-0-spec/FOLD-PACKET-v3.md`
(Astra round 2, `REVIEW-astra-spec-r2.md`). v2 is kept at
`build/reviews/security-before-release/packet-0-spec/spec-security-before-release-v2.md`.
New probe in `probes-v3.log` (P3-L1). The PO decision of record stands: no saved-secret
registry, no API change.

- **N1 (P2), two literal forms missed alternate JSON escapes.** Folded per the suggested fix.
  REQ-106 now has two paths. A reply that is valid JSON is decoded; the sent key is blanked in
  every decoded string (member names and values, any depth); the result is re-encoded and
  logged, keeping the rest of the diagnostic text. A reply that is not valid JSON gets v2's
  literal blanking (the key as sent and its JSON-escaped spelling). New ST-115 (probed). AC-107
  keeps (i) and (ii) and adds (iii) `probe&key-1234` echoed as `probe\u0026key-1234` and (iv)
  `probe/key-1234` echoed as `probe\/key-1234`. Correction the fold forces: the decoded path
  also runs the literal pass over the re-encoded text, because a key written as a JSON number
  survives decoding (P3-L1) and v2's literal rule caught it; new AC-107 (v) and a §2a row. §4
  and Q-003 state the re-encoding's cost (members sorted, whitespace and escapes normalised).
  The recoverability check also decodes the `body=` value in text-format sinks, so the pin
  holds in both log formats (in text format the body is the event's last field, P2-L2).
- **N2 (P3), local fakes behind the refusing proxy.** Folded. Door B now names the harness
  option: `NO_PROXY=localhost,127.0.0.1` for the binary under test, so requests to local fakes
  go direct while every other outbound request still reaches the refusing proxy; existing
  harness callers keep `NO_PROXY=""`. Each fake records its requests, and a criterion that
  relies on a fake checks that record first. AC-106 and AC-107 cite it; ST-110 records why it
  is needed. Correction the fold forces: AC-101, AC-301, AC-302 and AC-303 also use local
  fakes through the trusted client, so they cite the same option (AC-301's own `NO_PROXY` note
  now points to it). Test setup only; no production change.
- **N3 (P3), successful login logs.** Folded. ST-112 now lists successful login as a logging
  flow: INFO `login successful` with username and user id (`auth_service.rs:256`), which the
  existing setup-token test requires in the file log and the tail. The v2 F4 entry below said
  two flows log; it is three. AC-101 (f) now names that event as its control, beside the
  200 replies for the authenticated requests (which still write no event).

v2, 2026-10-05: folded per `build/reviews/security-before-release/packet-0-spec/FOLD-PACKET-v2.md`.
v1 is kept at `build/reviews/security-before-release/packet-0-spec/spec-security-before-release-v1.md`.
New probes are in `probes-v2.log` (P2-L1, P2-L2).

- **PO decision of record (2026-10-05, "1 only").** The registry of saved secret values is
  removed: v1's REQ-103, its eight registration sites in `livrarr-db`, the 8-character floor,
  the JSON-escaped registry forms, v1's AC-103 and Q-001 option (b). The pattern cleanser
  (REQ-101, REQ-102) and the `FetchRequest` `Debug` (REQ-104) stay. New **REQ-106**: the LLM
  test blanks the key it sent out of the reply, any length or characters, before logging it
  (Q-003 is now "keep, with the sent key blanked, then cleansed"). The accepted tradeoff is
  stated in §4. `livrarr-db` leaves the file plan.
- **F1 (P2), widened redactor changed API replies.** Folded. `redact_secrets` keeps today's
  rules and output for its five callers, which build returned warnings and errors (ST-105).
  The widened rules live in a separate log-only cleanser used by the sinks and the
  `FetchRequest` `Debug` (REQ-101, REQ-102, REQ-104, new **REQ-107**). New AC-106 pins the
  release-search warning unchanged. No log copy of that warning exists today (ST-105), so the
  pin's "log copy is cleansed" half has nothing to observe; said in AC-106.
- **F2 (P2), short keys left the LLM leak open.** Folded by the PO decision: REQ-106 has no
  length floor. New AC-107 (i): a 7-character key echoed in prose, through the real LLM test
  route, both formats (ST-113, probed).
- **F3 (P2), nested JSON escaping.** Folded. REQ-106 blanks the key in the raw reply, as sent
  and in its JSON-string-escaped form, before any formatter sees it. AC-107 (ii) uses a key
  with a quote and a backslash in a JSON error body. The observable is "the key cannot be
  recovered": JSON events and the logs-tail reply are parsed, and a body string that is JSON
  is parsed again. REQ-102 now requires every JSON log line to stay valid after cleansing.
  Re-check of the escaped JSON-member form: still needed, because in the JSON sinks a logged
  body's members appear with escaped quotes (ST-103, P-L5 in `probes.log`), while the text
  sinks carry them plain.
- **F4 (P2), controls required log lines that do not exist.** Folded. ST-112 lists which
  AC-101 flows log today. AC-101 names the real event for the two flows that log (the LLM
  warning, the Retry's `internal error:` event; corrected in v3: successful login logs too). For the silent flows the control is the HTTP
  reply plus the fake server's recorded request. One established event, the startup line
  `Livrarr starting — data directory:`, proves capture is on in every sink.
- **F5 (P2), the red case did not tell a sink cleanser from an LLM-only patch.** Folded with
  the reviewer's door: AC-108 is a red binary case at an independent emitter with no
  call-site redaction, a secret-shaped unknown key in `config.toml` logged at startup
  (ST-111, probed). The registration half of F5 is moot: there is no registry in v2.
- **F6 (P2), undo root per item.** Folded. New AC-205: the import's root differs from one
  item's root, same-relative-path decoys sit under the other root, and counts are per item.
  ST-114 records why the fixture is seeded state (today's import gives every item the
  import's root) and that it uses the real `SqliteDb` writers.
- **F7 (P3), missed harness.** Folded after re-count: eleven `LiveImportService::new` calls,
  eight behavioural harnesses; `tests/behavioral/test_ilr_contracts.rs:6294` added to ST-306,
  REQ-301 and the file plan.
- **F8 (P3), absolute path inside the root.** Folded. The shape row now says "absolute path
  outside the root"; a new row and AC-201 case say an absolute path to a regular file inside
  the item's root is deleted, as the shared helper does (ST-204).

v1, 2026-10-05: written from source at `0e06a205`, GitHub #76, the scan report
`build/reports/docs-ops-security-scan-2026-09-26.md` (§3, lines 150–215), the task list in `build/plans/`
and Sonarr's `CleanseLogMessage.cs` (read with `gh api`, branch `develop`), per
`build/reviews/security-before-release/packet-0-spec/PACKET.md`. Premises taken from that
packet are re-opened and cited to code below. IDs are numeric because the gate requires them:
REQ-1xx = REQ-L (logs), REQ-2xx = REQ-U (undo), REQ-3xx = REQ-C (client); see `BLOCKED-spec.md`
in the packet folder.

## 0a. Design Principles

- One definition per rule. One log cleanser (new, in `livrarr-domain`, beside
  `redact_secrets`) runs at every log sink; `redact_secrets` keeps serving returned text
  unchanged. One removal rule (`library_path::remove_library_file`) serves every door that
  deletes a library file. One trust class (the trusted client) serves every lookup sent to an
  admin-configured download client.
- A secret that only one call site knows is removed at that call site (the LLM test knows the
  key it just sent); everything else relies on the shared patterns.
- Trust follows where an address came from (`wiki/insights/process.md:64-68`, lesson 37).
- Tests drive the real door (the built binary with its real log sinks, the real `SqliteDb`, a
  real temporary filesystem, real local HTTP servers) and pin only the named observable.

## 0b. System Truths

Read at `0e06a205` unless marked T (driven by a scratch probe; command and output in
`probes.log` or `probes-v2.log`, entry named in brackets).

| ID | Truth | Source | Forbids | How verified |
|----|-------|--------|---------|--------------|
| ST-101 | Tracing has three sinks, all built in `init_tracing`: a console layer (text or JSON, ANSI colour codes on), a daily file `{data}/logs/livrarr.log.<date>` (text or JSON, no colour), and an in-memory buffer of the last 200 lines that formats its own line through `MessageVisitor`. The buffer is read only by admin `GET /api/v1/system/logs/tail` (Logs page), which replies with a JSON array of line strings. There is no log download route and no request tracing layer. `eprintln!` writes five messages to stderr outside tracing: a data-folder error, a config error, a setup-token file error, a log-folder warning (paths and config text only), and the `identity-cutover` subcommand's error. | `crates/livrarr-server/src/main.rs:1280`, `:1312-1316`, `:1319`, `:1327-1347`, `:1350-1355`, `:1367-1412`; `crates/livrarr-server/src/infra/log_buffer.rs:7`, `:28-34`; `crates/livrarr-handlers/src/system.rs:70-77`; `crates/livrarr-server/src/router.rs:449-452`; `frontend/src/api/index.ts:582`; `main.rs:127`, `:702`, `:724`, `:1004`; `crates/livrarr-server/src/log_surface.rs:49`; `rg TraceLayer` (none) | A cleanser on one sink only. | read; T [P-L4] (ANSI codes in stdout), [P2-L2] (tail reply is JSON) |
| ST-102 | The console and file layers format each event into one buffer and write it with one `write_all`, so a writer wrapper sees whole events. | `~/.cargo/registry/src/*/tracing-subscriber-0.3.23/src/fmt/fmt_layer.rs:1049-1050`; `Cargo.lock` (0.3.23) | Cleansing fragments. | read |
| ST-103 | Calling `POST /api/v1/config/metadata/test/llm` (admin; no frontend caller) when the provider answers non-2xx logs the whole reply body as a structured field `body=` (`config.rs:416`) after sending the key as `Authorization: Bearer <key>` (`:406`). A body echoing the key puts the key in stdout, the log file and the logs tail, in text and in JSON format; not in stderr or the API reply. In console text format the field is written `body` + ANSI codes + `=`. In the JSON sinks the body is a JSON string, so a JSON body's own members appear with escaped quotes (`{\"error\":…}`). | `crates/livrarr-handlers/src/config.rs:381-421`; `router.rs:240-241`; `frontend/src/api/index.ts:551`, `:553` (the only `config/metadata` calls). **T:** [P-L4], [P-L5] | Treating structured fields as safe. | read; T |
| ST-104 | `redact_secrets` masks `[?&]` + `apikey`, `api_key`, `token`, `passkey`, `password` (any case) and `user:pass@`, with `[REDACTED]`. It misses `access_token=`, `key=`, a Debug tuple `("X-Api-Key", "…")`, `Authorization: Bearer …`, JSON `"apikey":"…"` (plain or backslash-escaped) and a bare value in prose. It handles mixed case, URL-encoded values and multi-line input. | `crates/livrarr-domain/src/redact.rs:14`, `:19-21`, `:25-26`, `:30-38`. **T:** [P-L3] | Reusing it unchanged as the log cleanser. | read; T |
| ST-105 | `redact_secrets` has five production callers, and all five build returned text, not log lines: three build release-search warnings (`release_service.rs:266`, `:272`, `:277`) that reach the API reply (`release_service.rs:321-326`; `crates/livrarr-handlers/src/release.rs:91-106`); two build returned `FetchError::Connection` text (`fetcher.rs:333-337`, `:414-418`). Neither the release service nor the release handler logs the warnings (no tracing call in either file). Most HTTP error texts already drop the URL (`without_url`). A reqwest *request* error's text includes the full URL and its query (seen with `apikey=`); a reqwest *decode* error's text does not. | `crates/livrarr-http/src/fetcher.rs:335`, `:416`; `crates/livrarr-download/src/release_service.rs:266`, `:272`, `:277`; `rg "warn!\|info!\|debug!\|error!"` over `release_service.rs` and `handlers/src/release.rs` (none). **T:** [P-L1] (decode text has no URL), [P-C2] (request error text carries `apikey=` with the probe key) | Changing `redact_secrets` output (it would change API replies, §4). | read; T |
| ST-106 | `FetchRequest` derives `Debug`, which prints every header value and the body. No current log call prints a `FetchRequest` (scan, `docs-ops-security-scan-2026-09-26.md:177-178`). | `crates/livrarr-domain/src/services/http.rs:79-91`. **T:** [P-L2] printed `("X-Goog-Api-Key", "PROBEGBKEY")` and `("Authorization", "Bearer PROBEHCTOKEN")` | — | read; T |
| ST-107 | Secrets Livrarr holds. Stored in settings: download-client password and API key, indexer API key, Prowlarr API key, Hardcover token, LLM key, Google Books key, SMTP password. Request-only: the Readarr import API key (sent as `X-Api-Key`), login and user passwords, values typed into a "Test" form before saving. Livrarr's own session tokens and user API keys are stored hashed and travel in headers; stream tokens travel as `?token=`; the HMAC key is generated in memory each start; qBittorrent's `SID` cookie is held per request. | `crates/livrarr-db/src/sqlite_download_client.rs:37`, `:49`; `sqlite_indexer.rs:37`; `sqlite_config.rs:117`, `:167`, `:177`, `:187`, `:293`; `crates/livrarr-domain/src/readarr.rs:11`, `:27`; `crates/livrarr-server/src/readarr_client.rs:140`; `crates/livrarr-domain/src/entities.rs:348-374`; `crates/livrarr-handlers/src/stream_token.rs:3-5`; `crates/livrarr-server/src/main.rs:360`; `crates/livrarr-server/src/infra/release_helpers.rs:38-53` | Missing a secret from the test. | read |
| ST-108 | The setup token is printed by `println!` straight to stdout (the banner) and never through tracing; the following `info!` names only the file. The prior spec makes the banner and `{data}/setup-token` its only places, and a tracked test pins that. A cleanser on tracing sinks cannot touch the banner. | `main.rs:1012-1026`; `spec-prerelease-trust-pass.md:378-380` (REQ-304); `crates/livrarr-server/tests/test_fresh_author_index.rs:1467-1533`. **T:** [P-L4] token in stdout only | Masking or moving the banner. | read; T |
| ST-109 | Audnexus and Goodreads log calls print whole URLs, which hold only public ids and search terms, never a key: neither provider takes one. Google Books and Hardcover send their keys in headers. | `crates/livrarr-external-data/src/audnexus.rs:202`, `:210`; `goodreads/client.rs:206-221`, `:230`; `google_books.rs:104`, `:168`; `hardcover.rs:87` | Treating those three as leaks. | read |
| ST-110 | The binary harness starts the Cargo-built binary on a temp data folder with its own `config.toml`, captures stdout and stderr, reads `logs/livrarr.log.*` and the logs tail, and routes provider traffic through a local proxy that refuses everything (`HTTP_PROXY`, `NO_PROXY=""`). The trusted client honours those variables; the safe client ignores them. With `NO_PROXY` empty, a trusted-client request to a local fake (an indexer, a download client, the LLM endpoint) also goes to the refusing proxy and never reaches the fake: both trusted clients keep proxy support (the `HttpClient` built without `ssrf_safe`, and the fetcher's client behind `fetch`, which release search uses). reqwest's `NO_PROXY` takes host names and IP addresses, so `localhost,127.0.0.1` sends only loopback requests direct; P2-L2 reached a `localhost` fake that way while other traffic stayed on the proxy. New files under any `tests/` folder are gitignored. | `test_fresh_author_index.rs:30-37`, `:57-103`, `:87`, `:1152-1165`; `crates/livrarr-http/src/lib.rs:64-87`; `crates/livrarr-http/src/fetcher.rs:147-160`, `:671-672`; `crates/livrarr-download/src/release_service.rs:189`; `~/.cargo/registry/src/*/reqwest-0.12.28/src/proxy.rs:476-505`; `.gitignore:3`. **T:** [P2-L2] (`NO_PROXY=localhost,127.0.0.1`) | A hand-built subscriber as the test's sink; a local-fake case run with `NO_PROXY=""`. | read; T |
| ST-111 | An independent emitter with no call-site redaction: each unknown `config.toml` key is kept as written (a quoted TOML key may hold any text) and logged at startup as `WARN Unknown config key: <key>`, right after the startup event `Livrarr starting — data directory: …`. Keys `"x?apikey=<v1>"` and `"y access_token=<v2>"` reach stdout, the file and the logs tail verbatim, in text and JSON. | `crates/livrarr-server/src/config.rs:389-406`; `main.rs:1266-1267`, `:732-735`. **T:** [P2-L1] | An LLM-only fix passing as the sink fix. | read; T |
| ST-112 | Which AC-101 flows write a log event today. Logging: the LLM test (`config.rs:416`) and a failed Retry, which logs `internal error: …` at ERROR with the import error's text, for SABnzbd "SABnzbd history request failed: …" (`queue.rs:117-132`; `api_error.rs:520-521`; `import_pipeline.rs:60-65`); and a successful login, which logs INFO `login successful` with the username and user id, never the password or session token (`crates/livrarr-server/src/auth_service.rs:256`; the existing setup-token test requires it in the file log and the tail, `test_fresh_author_index.rs:1513`, `:1525`). Silent: indexer Test and Prowlarr import (`indexer.rs:81-104`, `:436-462`; no tracing call in the file), qBittorrent and SABnzbd Test (`download_client.rs:370-386`, `:475-480`), Hardcover Test (`config.rs:323-334`; the fetcher has no tracing call), authenticated requests after login (no request tracing layer, ST-101), e-mail Test (`config.rs:597-607`; `email_service.rs:29-37`; `infra/email.rs:100-108`). `BadRequest` and `BadGateway` replies are rendered without logging (`api_error.rs:497-498`). The default filter enables only `livrarr` and `tower_http` targets (`main.rs:1303-1306`). | as cited | Requiring a log line that no flow writes. | read |
| ST-113 | The settings API accepts any non-empty LLM key. A 7-character key (`hunter2`) echoed in prose reaches stdout, the file and the tail in both formats. A key with a quote and a backslash (`probe"quote\back-1234`) is accepted and sent in the header; the provider's JSON reply carries it JSON-escaped once; the text sinks carry that once-escaped form; the JSON sinks carry it escaped twice, so neither the key nor its once-escaped form is a substring there, yet parsing the event and then its body recovers the key. The tail reply is JSON, so a raw-text substring check on it misses the key too. | `config.rs:185-191`, `:406`, `:415-416`. **T:** [P2-L2] | A raw-substring-only observable; a length floor. | read; T |
| ST-114 | Root folders are unique per media type (one ebook root, one audiobook root). Each library item carries its own `root_folder_id`. Today's Readarr import writes the import's `target_root_folder_id` and every item's `root_folder_id` from the same request field, so they coincide unless a row changes later; undo uses the import's root for every item. | `crates/livrarr-db/migrations/001_initial_schema.sql:107`, `:113`; `readarr_import_workflow.rs:997`, `:2976`, `:1174-1194` | A fixture where the two roots coincide as the only per-item case. | read |
| ST-115 | The LLM test reads the provider's reply as unparsed text (`resp.text()`). JSON allows one character several spellings, so a reply can echo the key in a form that is neither the key nor its usual escaped spelling: `probe\u0026key-1234` for `probe&key-1234`, `probe\/key-1234` for `probe/key-1234`. Replacing the key and its serde_json spelling leaves both recoverable after decoding. Parsing the whole reply as `serde_json::Value`, replacing the key in every decoded string (member names and values) and re-encoding with `serde_json::to_string` blanks both, keeps "Incorrect API key provided", and gives text that parses, also as the body of a JSON event. Re-encoding sorts object members by name (no `preserve_order` feature), drops whitespace and rewrites escapes canonically. A key written as a JSON number survives decoding; a literal pass over the re-encoded text blanks it. A body with a lone surrogate escape (`\ud800`) is not valid JSON to serde_json. | `config.rs:415-416`; `Cargo.lock:2864-2865` (serde_json 1.0.149); `Cargo.toml:26`; `crates/livrarr-handlers/Cargo.toml:14`; reviewer check `reviewer-scratch/r2-json-escape-check.txt`. **T:** [P3-L1] (serde_json only, not the binary) | Two literal forms as the whole rule for a JSON reply. | read; T |
| ST-201 | `undo_import` reads the import's target root folder, turns any read error into "no root" (`.ok()`), joins root and stored path (or uses the stored path alone), and calls `std::fs::remove_file`. Removed and not-found both count as `files_deleted`; other errors count as `files_skipped`. Rows, orphan works, covers and authors are then deleted whatever happened to the files. | `crates/livrarr-server/src/readarr_import_workflow.rs:1137`, `:1174-1182`, `:1184-1194`, `:1196-1217`, `:1219-1300` | — (the defect) | read |
| ST-202 | The import writes each stored path relative to the Livrarr root as `{user_id}/{Author}/{Title}.{ext}`; components are sanitised (`/` replaced, `.`/`..` replaced), and the only writer of `target_root_folder_id` sets it. So today's writer never stores an escaping path; the risk is a row changed later or a root that cannot be read. Each library item has its own `root_folder_id` (NOT NULL, `ON DELETE RESTRICT`). | `readarr_import_workflow.rs:997`, `:1404-1421`, `:2964-2981`; `crates/livrarr-domain/src/util.rs:14-37`; `crates/livrarr-db/migrations/001_initial_schema.sql:113` | — | read |
| ST-203 | Today's undo, per shape: inside the root, deleted; `../outside/x`, deleted outside; absolute path outside, deleted; entry that is a link, the link is deleted and its target kept; parent folder that is a link to outside, the outside file is deleted; entry that is a folder, skipped; file already gone, counted deleted. Reply `files_deleted: 6, files_skipped: 1`. | **T:** [P-U1] | Calling undo safe today. | T |
| ST-204 | `remove_library_file` on the same shapes: inside, `Removed`; `..`, absolute outside, and parent link, `OutsideRoot`; entry link, `NotRegularFile(Link)`; folder, `NotRegularFile(Folder)`; gone (file or parent), `Absent`; root missing on disk, `Root(NotFound)`. Every outside file survived. An absolute path to a regular file inside the root is `Removed`: `root.join` of an absolute path is that path, and its parent resolves inside the root. It is used by book delete and work delete, each with the item's own root. | `crates/livrarr-domain/src/library_path.rs:1-11`, `:108-149` (absolute inside: `:111`, `:116-125`, `:144`); `crates/livrarr-library/src/file_service.rs:77-96`; `crates/livrarr-metadata/src/work_service.rs:3554-3573`. **T:** [P-U3] (absolute inside: read only) | A second rule for undo. | read; T |
| ST-205 | When the root lookup yields none, undo deletes the bare stored path relative to the server process's working folder. | `readarr_import_workflow.rs:1189-1191`. **T:** [P-U2] a file at `{cwd}/zz_probe_cwd_victim.epub` was deleted; reply `files_deleted: 1` | Any bare-path fallback. | read; T |
| ST-206 | A root folder cannot be deleted from settings while an import or a library item references it: foreign keys are on and the delete fails. | `crates/livrarr-db/src/pool.rs:22`; `001_initial_schema.sql:113`; `026_cascade_user_fk.sql:36`; `crates/livrarr-db/src/sqlite_root_folder.rs:62-75`. **T:** [P-U1] `FOREIGN KEY constraint failed` | — | read; T |
| ST-207 | Undo is `DELETE /api/v1/import/readarr/{import_id}`, reply `{filesDeleted, filesSkipped, worksDeleted, authorsDeleted}`. The page's API call is typed `void`, ignores the reply, and always shows "Import undone". | `crates/livrarr-server/src/router.rs:508-511`; `crates/livrarr-handlers/src/readarr_import.rs:76-87`; `crates/livrarr-domain/src/readarr.rs:145-152`; `frontend/src/api/index.ts:733-734`; `frontend/src/pages/import/ReadarrImportPage.tsx:233-244` | — | read |
| ST-208 | The tracked test `wh_readarr_import_undo_marks_file_and_orphan_work_deletions_as_undo` seeds an import with no target root and items under roots that do not exist on disk, and asserts `files_deleted == 2`. That count comes from the bare-path fallback (ST-205) finding nothing. Under REQ-201 the same fixture gives 0 deleted and 2 skipped (inferred from ST-204: those roots do not exist, so `Root(NotFound)`). | `tests/behavioral/test_wh_deletion.rs:62`, `:283-343`; `crates/livrarr-behavioral/Cargo.toml:563-565` | Keeping that assertion unchanged. | read |
| ST-301 | Retry is the Queue page's "Retry" button on an `importFailed` row, `POST /api/v1/grab/{id}/retry`. When the grab has no saved content path but has a download id, the import service asks the client for it through `http_client_safe` (SABnzbd branch if the client type is `sabnzbd`, else the qBittorrent branch) and saves it. `main.rs` passes the safe client. | `frontend/src/pages/activity/queue/QueuePage.tsx:61-68`, `:129-137`; `router.rs:419-420`; `crates/livrarr-handlers/src/queue.rs:102-132`; `crates/livrarr-server/src/import_service.rs:27`, `:36`, `:215-243`; `crates/livrarr-server/src/main.rs:214-220` | — (the defect) | read |
| ST-302 | Through the safe client, the qBittorrent lookup to `localhost` fails ("qBittorrent login failed: error sending request for url (http://localhost:…/api/v2/auth/login)"); to literal `127.0.0.1` it succeeds. Through the trusted client both succeed. An unreachable client gives the same "login failed" text. On failure the user sees the toast "Something went wrong" (500) and the row keeps the error text as the button's tooltip; the server logs `internal error: …`. | `crates/livrarr-server/src/infra/release_helpers.rs:18-36`; `infra/import_pipeline.rs:8-24`; `crates/livrarr-http/src/ssrf.rs:141-180`; `queue.rs:117-132`; `crates/livrarr-handlers/src/types/api_error.rs:520-527`. **T:** [P-C1] | — | read; T |
| ST-303 | The poller makes the same lookups through the trusted client: qBittorrent at `download_poller.rs:198-203`, SABnzbd queue and history at `:313-321` and `:369-374`. Its qBittorrent list call asks by category, not by hash. | `crates/livrarr-server/src/jobs/download_poller.rs:98-121`, `:198-203`, `:313-321`, `:369-374` | Two trust classes for one client. | read |
| ST-304 | Both clients follow up to 10 redirects and set `Referer` to the previous URL with its query (only user, password and fragment removed). A SABnzbd reply that redirects sent the full `…/api?mode=history&apikey=…` address with the probe key to the landing server: from the trusted client to `localhost` and `127.0.0.1`, from the safe client to `127.0.0.1` (the safe client blocked only the host name). Headers `Authorization` and `Cookie` are removed on a cross-host redirect. | `~/.cargo/registry/src/*/reqwest-0.12.28/src/redirect.rs:161-164`, `:239-251`, `:293-303`; `async_impl/client.rs:312`; `crates/livrarr-http/src/lib.rs:64-93`. **T:** [P-C2] | Claiming the safe client protects the key. | read; T |
| ST-305 | Apart from this lookup, no admin-configured service uses the safe client or `fetch_ssrf_safe`: the other safe-fetch callers are covers, the cover proxy and Goodreads, all runtime-derived URLs. `AppState::http_client_safe` has no caller. | `rg "\.fetch_ssrf_safe\(|http_client_safe"` over `crates`; `crates/livrarr-metadata/src/cover.rs:42`, `:598`; `crates/livrarr-materialize/src/lib.rs:107-109`; `crates/livrarr-handlers/src/context.rs:271` | — | read |
| ST-306 | `LiveImportService::new` is called in eleven places: `main.rs:214`, the router test mirror `router.rs:848`, its own unit test `import_service.rs:1092`, and eight behavioural harnesses. All take an `HttpClient`; the type does not change, so none fails to compile. `test_ilr_contracts.rs` passes `http_client_safe` (`:6299`), built with `.ssrf_safe(true)` (`:6215-6220`). | `git grep -n "LiveImportService::new" -- crates tests`; `tests/behavioral/test_irf_u2_import_defer.rs:571`, `test_wh_tags.rs:230`, `test_author_link_doors.rs:133`, `test_irf_u1_refusal.rs:194`, `test_subtitle_matching_preview_door.rs:340`, `test_irf_u7_card_hygiene.rs:165`, `test_irf_u5_durable_dismissal.rs:236`, `test_ilr_contracts.rs:6294` | — | read |
| ST-307 | No function or trait signature changes in this spec (packet rule 3). New items only: a log-only cleanser function in `livrarr-domain` (`redact_secrets` keeps its signature and output), a writer wrapper in `main.rs`, a hand-written `Debug`, and the key blanking inside `test_llm`. | this spec, §2 | — | read |

## 0c. Prior Art

Searched `build/foundation/security-model-policy.md`, `build/reviews/security-hardening/`,
`build/reviews/security-model-policy/`, `spec-errors-and-delete-pass.md`,
`crates/livrarr-domain/src/library_path.rs`, `wiki/insights/process.md`, and
`git grep -il` over `docs/` and `wiki/` for "redact", "secret" and "undo".

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `build/foundation/security-model-policy.md:79-81`, `:89-93`, `:173` | Policy: hand-written redacting `Debug`; "do not log full outbound URLs with query strings. Redact `Authorization` and `X-Api-Key` headers. Sanitize all upstream error payloads." Listed as Phase 1 item 8, not deferred; the scan rates it Partial. REQ-101 to REQ-106 complete it for logs. |
| PA-002 | `build/reviews/security-hardening/security-fixes-review-cycle1-streamB-round1.md:318-341` | The earlier redaction fix was "call-site-by-call-site rather than centralized"; the reviewer flagged that coverage risk. REQ-101 is the centralised layer. Same review flagged redirects as an SSRF bypass (`:35-38`). |
| PA-003 | `build/reviews/security-model-policy/v1-gpt.md:11`, `:229-230` | "Redact secrets in structured logs." ST-103 shows the gap is real. |
| PA-004 | Sonarr `src/NzbDrone.Common/Instrumentation/CleanseLogMessage.cs` (`develop`, read 2026-10-05 via `gh api`) | PO-chosen model: one `Cleanse(message)` with regex rules for query keys (`apikey`, `token`, `passkey`, `auth`, `passwd`, `*apikey`, `username`, `password`), JSON fields (`"…api_?key…":"…"`), and client-specific shapes; mask `(removed)`. Patterns only, no known values, which is the PO's choice here too. It also masks home paths, e-mail addresses and remote IPs, which the PO has not approved here (§4). |
| PA-005 | `crates/livrarr-domain/src/library_path.rs:1-11`; `spec-errors-and-delete-pass.md:61`, `:95`, `:199`, `:392`; `wiki/domain/library-item.md:22-35` | The removal rule exists and serves both delete doors. That pass left Readarr undo "unchanged" on purpose (R8) and listed it at `:199` (ST-028). REQ-201 brings undo onto the rule. |
| PA-006 | `wiki/insights/process.md:56-60` (lesson 35) | Path prefixes must match whole components. `remove_library_file` uses `Path::starts_with`, which compares components (`library_path.rs:125`). |
| PA-007 | `wiki/insights/process.md:64-68` (lesson 37) | Download clients use the trusted class; the safe class is for outside addresses. Approved Readarr origins use no-redirect requests "so API keys cannot be forwarded" (`readarr_client.rs:151-159`). Rule of record for REQ-301; precedent for the redirect follow-up in §4. |
| PA-008 | `wiki/decisions/key-decisions.md:48`; `wiki/insights/coding-patterns.md:101`; `wiki/deployment/first-run-setup.md:43` | Download-client passwords stored in plain text, redacted in API replies; credential reads kept separate from settings reads; the setup request's `Debug` redacts the token. All kept. |
| PA-009 | `build/reports/docs-ops-security-scan-2026-09-26.md:157-181`, `:189-215`; the task list in `build/plans/` (line 138) | Source lists for items 1 and 3; source entry for item 2. The scan's "~60% confident, not tested" on item 3 is now tested (ST-302). |

## 1. Problem Statement

1. **A secret in a log.** An admin (or a script using the API) calls the AI-connection test
   route `POST /api/v1/config/metadata/test/llm` with a key the provider refuses. Expected:
   the logs say the provider refused. Observed: the provider's reply, which repeats the key, is
   written to the console, the log file and the Logs page, whatever the key's length; a key
   with a quote in it is written in an escaped form that still reveals it (ST-103, ST-113).
   The same happens to any secret any call site prints in a shape the current masking misses,
   for example a secret-shaped unknown key in `config.toml` at startup (ST-104, ST-111).
2. **Undo deletes outside the library.** A user presses "Undo" on a finished Readarr import.
   Expected: only that import's files inside each file's library folder are deleted.
   Observed: a stored path that leads outside, through `..`, an absolute path outside the
   library or a linked folder, is deleted outside; and if the root folder cannot be read, the
   stored path is deleted relative to the server's working folder (ST-203, ST-205).
3. **Retry cannot reach the download client.** A user presses "Retry" on the Queue page for a
   failed import whose grab has no saved content path, with qBittorrent configured as
   `http://qbittorrent:8080` (a Docker service name). Expected: Livrarr asks qBittorrent where
   the files are, as the background poller does. Observed: the request is refused before it
   is sent, the toast says "Something went wrong", and the row's tooltip says "qBittorrent
   login failed" (ST-302).

## 2. Requirements

### Part 1: secrets kept out of every log (REQ-L)

- **REQ-101** (one cleanser at every sink). **Delivered** (cleanser `crates/livrarr-domain/src/redact.rs:193-202`; console and file writers wrapped at `crates/livrarr-server/src/main.rs:1317`, `:1323`, `:1345`, `:1352`, wrapper `:1375-1410`; Logs-page buffer `:1431`). Every line written by tracing passes through one
  new log-only cleanser in `livrarr-domain` (for example `cleanse_log_line`) before it
  reaches a sink: the console layer and the file layer through a writer wrapper around their
  writers (`main.rs:1312-1347`), in text and JSON format; the in-memory buffer in
  `LogBufferLayer::on_event` before `push` (`main.rs:1385`). The mask is `[REDACTED]`.
  Observable: no secret of ST-107 appears in stdout, the log file or the logs tail for the
  flows of AC-101, and the secret-shaped unknown key of AC-108 is masked in all three. Door:
  the built binary (ST-110).
- **REQ-102** (pattern rules). **Delivered**, with the shapes code review added (AC-109 to AC-112) and the accepted limits of §4 (rules in order at `redact.rs:128-181`). The log cleanser applies today's two `redact_secrets` rules
  and also masks, any case:
  - a value after `?`, `&`, a space or the start of the text for `apikey`, `api_key`,
    `*_apikey`, `access_token`, `*_token`, `token`, `passkey`, `password`, `passwd`,
    `authkey`, `auth` and `nzb_key`. Bare `key=` is left alone: no Livrarr request sends a
    secret that way (ST-109), and masking it would hide ordinary ids;
  - a JSON member whose name contains `apikey`, `api_key`, `token`, `password`, `passkey`,
    `secret` or `nzb_key`, with plain quotes (`"x":"v"`, text sinks) and backslash-escaped
    quotes (`\"x\":\"v\"`, a logged JSON body inside a JSON sink event, ST-103), except the
    accepted limits in §4;
  - `Authorization`, `X-Api-Key`, `X-Goog-Api-Key` and `Cookie` header values in `name: value`
    and Debug-tuple `("name", "value")` form, and any `Bearer <token>` / `Basic <token>`;
  - a structured field named like the query keys above, with or without ANSI colour codes
    between the name and `=` (ST-103), except the accepted limits in §4.

  A value is masked at least up to its first `&`, `#`, whitespace, quote or backslash, except
  the accepted limits in §4.
  Masking never removes a quote and never leaves a lone backslash, so every JSON-format log
  line still parses as JSON after cleansing. An ordinary URL and ordinary text stay
  byte-identical.
- **REQ-104** (`FetchRequest` `Debug`). **Delivered** (`crates/livrarr-domain/src/services/http.rs:94-129`). Replace the derive with a hand-written `Debug` that
  prints the URL through the log cleanser, method, header names with every value as
  `[REDACTED]`, the body as its byte length, and the other fields as today.
- **REQ-105** (setup token unchanged). **Delivered**: no change to the banner; its test passes unchanged (`test_fresh_author_index.rs:1490`). The token keeps its banner on stdout and its file, and
  never reaches a tracing sink. The cleanser does not alter the banner (ST-108). The existing
  test at `test_fresh_author_index.rs:1467` stays green unchanged.
- **REQ-106** (the LLM test blanks the key it sent). **Delivered** (`crates/livrarr-handlers/src/config.rs:415`, `blank_sent_key` `:424-462`). When the provider answers non-2xx,
  `test_llm` blanks every occurrence of the key it just sent (`config.rs:389-391`, `:406`) in
  the reply with `[REDACTED]` before logging it, in one of two ways (ST-115):
  - **The reply is valid JSON** (the whole text parses). Match against the decoded contents:
    replace the key in every decoded string, object member names and string values, at any
    depth, so any valid JSON spelling of it is caught (`\u0026`, `\/`, `\"`, and so on).
    Re-encode the result as JSON with the project's serialiser, then apply the literal pass
    below to the re-encoded text (this catches a key written as a JSON number). Unrelated
    diagnostic text is kept; member order, whitespace and escape spelling may change.
  - **The reply is not valid JSON.** Apply the literal pass to the reply text: replace the key
    as sent and the key as it is written inside a JSON string (its JSON-escaped spelling).

  This holds for any key the settings API accepts, of any length and any characters (ST-113).
  The result is logged as today, as the `body` field of the same WARN event, and then passes
  through the sink cleanser like every line. The API reply is unchanged.
- **REQ-107** (logs only; returned text unchanged). **Delivered**: `redact_secrets` is unchanged (`redact.rs:30-38`); the log rules are separate. `redact_secrets` keeps its current rules
  and output, so its five callers (ST-105) return the same warnings and errors as today. The
  widened rules of REQ-102 apply only to the log sinks (REQ-101) and the `FetchRequest`
  `Debug` (REQ-104). Lower-level pattern code may be shared.

### Part 2: Readarr import undo deletes only inside the library (REQ-U)

- **REQ-201** (one removal rule). **Delivered** (`crates/livrarr-server/src/readarr_import_workflow.rs:1174-1227`). `undo_import` removes each item's file only with
  `remove_library_file(<that item's root folder path>, <stored path>)`, reading the root from
  that item's own `root_folder_id` as book delete does (`file_service.rs:77-90`), item by item.
  It never joins paths itself and never removes a bare stored path. `Removed` and `Absent`
  count as `files_deleted` (today's meaning). Any `RemoveError` counts as `files_skipped` and
  logs one WARN line with the stored path and the reason's `Display`
  (`library_path.rs:83-101`). Rows, orphan works, covers, authors and history are handled as
  today.
- **REQ-202** (no usable root). **Delivered** (root read per item at `readarr_import_workflow.rs:1176-1187`; skipped with a WARN at `:1222-1225`). If an item's root folder cannot be read from the database,
  or `remove_library_file` reports `Root(_)`, that item's file is skipped and counted, with
  a WARN line naming the reason. The import's `target_root_folder_id` is no longer used
  for paths.
- **REQ-203** (the page tells the user). **Delivered** (`frontend/src/pages/import/ReadarrImportPage.tsx:236-242`; `frontend/src/api/index.ts:734-735`; `frontend/src/types/api.ts:1586-1591`). The page's undo call is typed with the reply. When
  `filesSkipped > 0`, the page shows a warning toast instead of the success toast: "Import
  undone. N file(s) were left on disk because Livrarr could not safely delete them; the
  server log lists each one." Otherwise "Import undone" as today.

### Part 3: manual import Retry uses the trusted client (REQ-C)

- **REQ-301** (trusted client). **Delivered** (`crates/livrarr-server/src/import_service.rs:27`, `:36`, `:224`, `:232`; `main.rs:219`; comments `crates/livrarr-server/src/infra/import_pipeline.rs:17`, `:58-59`). The import service's content-path lookups (both branches,
  `import_service.rs:222-237`) use the trusted client. Rename the field and constructor
  parameter `http_client_safe` to `http_client` (`:27`, `:36`, `:43`); `main.rs:219` passes
  `http_client.clone()`. The router test mirror (`router.rs:848-853`) and the eight
  behavioural harnesses (ST-306, including `test_ilr_contracts.rs:6294`) pass their trusted
  client, so harnesses match `main.rs`. Correct the comments at `import_pipeline.rs:17` and
  `:58-59` to say the trusted client is used because the client is admin-configured.
- **REQ-302** (redirects: no worse than the poller). **Delivered**: Retry uses the same trusted client as the poller; no redirect change. Retry gets exactly the poller's client and
  therefore its redirect behaviour (ST-303, ST-304). No redirect change in this feature
  (§4).

## 2a. Input shapes

Every shape is an acceptance case (rule 1 of the packet).

| Part | Shape | Today | After | AC |
|---|---|---|---|---|
| L | query value per key name, mixed case | masked for 5 keys | masked for REQ-102 keys | AC-102 |
| L | URL-encoded value | masked | masked | AC-102 |
| L | `user:pass@` | masked | masked, except the accepted limits in §4 | AC-102 |
| L | `Authorization` / `X-Api-Key` in a `Debug` | printed (ST-106) | masked | AC-102, AC-104 |
| L | JSON body `"apikey":"…"` (a secret-named member) | printed | masked by pattern, plain in text sinks, escaped in JSON sinks; JSON stays valid; except the accepted limits in §4 | AC-102 |
| L | SABnzbd or qBittorrent Test reply echoing a credential | not logged (ST-112) | not logged | AC-101(c) |
| L | provider error body echoing the LLM key | printed (ST-103) | blanked at source (REQ-106) | AC-101(a), AC-107 |
| L | LLM key of 7 characters echoed in prose | printed (ST-113) | blanked at source | AC-107(i) |
| L | LLM key with a quote and a backslash echoed in a JSON body | printed, escaped (ST-113) | blanked at source; JSON lines valid | AC-107(ii) |
| L | LLM key echoed in a JSON body with another valid escape (`\u0026` for `&`, `\/` for `/`) | printed (ST-115) | blanked in the decoded contents | AC-107(iii), (iv) |
| L | LLM key written as a JSON number in a JSON body | printed | blanked by the literal pass after re-encoding (ST-115) | AC-107(v) |
| L | LLM reply that is not valid JSON | printed | literal pass (REQ-106) | AC-107(i) |
| L | HTTP client error text with the URL | printed when not `without_url` (ST-105) | masked | AC-101(d), AC-102 |
| L | secret in a structured field | printed (ST-103) | masked, text and JSON, with ANSI, except the accepted limits in §4 | AC-101(a), AC-102 |
| L | secret-shaped text at an emitter with no call-site redaction | printed (ST-111) | masked | AC-108 |
| L | multi-line message | masked per pattern | masked | AC-102 |
| L | bare secret no pattern fits, outside the LLM test | printed | printed | not applicable: accepted limit (PO, 2026-10-05; §4) |
| L | secret-shaped text in a returned release-search warning | returned as today | returned as today (REQ-107) | AC-106 |
| U | relative path inside the root | deleted | deleted | AC-201 |
| U | `..` that escapes | deleted outside | kept, skipped | AC-201 |
| U | absolute path outside the root | deleted outside | kept, skipped | AC-201 |
| U | absolute path to a regular file inside the item's root | deleted | deleted (shared helper, ST-204) | AC-201 |
| U | entry is a link to outside | link deleted | link kept, skipped | AC-201 |
| U | parent folder is a link to outside | deleted outside | kept, skipped | AC-201 |
| U | entry is a folder | skipped | skipped | AC-201 |
| U | file already gone | counted deleted | counted deleted | AC-201 |
| U | item's root differs from the import's root | import's root used for every item | each item's own root | AC-205 |
| U | import with no root folder | bare path from working folder | item's own root used | AC-202 |
| U | root folder deleted from settings since import | not reachable: the delete is refused (ST-206) | same; root missing on disk is skipped | AC-202 |
| U | non-Unix builds | not built | not built | not applicable: no non-Unix target is compiled on this host and Livrarr ships Linux images (§4) |
| C | host name resolving to a private address | refused | reached | AC-301 |
| C | literal private address | reached | reached | AC-301 |
| C | public name | reached | reached | not applicable: no public fake can be served from a test, and the trusted client adds no check |
| C | reply that redirects elsewhere | safe client follows unless host name is private | follows, as the poller (REQ-302) | AC-302 |
| C | client unreachable | "login failed" toast and tooltip | same | AC-303 |

## 2b. Files per part

| Part | Production files | Test files |
|---|---|---|
| L | `crates/livrarr-domain/src/redact.rs` (new log cleanser beside `redact_secrets`), `crates/livrarr-domain/src/lib.rs`, `crates/livrarr-domain/src/services/http.rs`, `crates/livrarr-server/src/main.rs` (sink wrappers, `LogBufferLayer`), `crates/livrarr-handlers/src/config.rs` (`test_llm`) | `crates/livrarr-server/tests/test_fresh_author_index.rs` (tracked; adds the B-local harness option, §6); unit tests in `redact.rs` and `services/http.rs` |
| U | `crates/livrarr-server/src/readarr_import_workflow.rs`, `frontend/src/api/index.ts`, `frontend/src/types/api.ts`, `frontend/src/pages/import/ReadarrImportPage.tsx` | `tests/behavioral/test_wh_deletion.rs` (tracked; ST-208 assertion changes; AC-201, AC-202, AC-205 cases), `frontend/src/pages/import/ReadarrImportPage.test.tsx` |
| C | `crates/livrarr-server/src/import_service.rs`, `crates/livrarr-server/src/main.rs:219`, `crates/livrarr-server/src/infra/import_pipeline.rs` (comments), `crates/livrarr-server/src/router.rs` (test mirror) | `crates/livrarr-server/tests/test_fresh_author_index.rs`; the eight harnesses of ST-306 (argument only), including `tests/behavioral/test_ilr_contracts.rs` |

**Overlap:** `main.rs` (L and C, different functions) and `test_fresh_author_index.rs` (tests
for L and C). Build L and C in one change or sequence them. No new test file is needed; a new
file under `tests/` would need `git add -f` (ST-110).

## 3. UI/Interface Design

Only REQ-203 changes the UI: one warning toast on the Readarr import page. No API shape
or content changes; the undo reply already carries `filesSkipped`, and returned warnings keep
today's text (REQ-107).

## 4. Non-Requirements

- Not approved, not done: encrypting stored credentials (#118); making the safe client the
  default; an HTTPS warning; the delete-with-files folder-swap race; masking paths, usernames,
  e-mail addresses or IPs (Sonarr does, PA-004); any change to what the database stores or the
  API returns, including the grab's `download_url`, the grab error text and release-search
  warnings.
- **Accepted tradeoff (PO, 2026-10-05).** Livrarr does not keep a list of its saved secret
  values to mask. A reply that quotes a password or key back in plain prose, at any call site
  other than the LLM test, is not caught by patterns alone. Sonarr has the same limit.
- Pattern limits. A value is masked up to its first `&`, `#`, whitespace, quote or backslash;
  a secret containing one of those characters may leave its remainder after that character
  where only a pattern protects it. REQ-106 blanks the LLM key in full. JSON escapes are decoded only when the whole reply
  is valid JSON, and only one level deep: a JSON document carried inside one of the reply's
  strings is not decoded again. Other encodings of the LLM key in a reply (URL-encoded, HTML
  entities, partial echoes) are not blanked.
- **Accepted limits (PO, 2026-10-05).** Code review round 2 found three secret shapes the
  log cleanser misses (`build/reviews/security-before-release/packet-7-code-review-r2/REVIEW-astra-code-r2.md`,
  D1 to D3). The PO accepted them to ship. In D1 and D3 the secret holds none of the
  delimiter characters above. In D2 the password holds one, yet the part before it is also
  left visible, which the pattern limit above does not allow.
  - **D1, a quoted field value that starts with `:`, `,`, `}` or `]`.** Example: the
    structured field `password=":QUOTEDSECRET"` is written unchanged. This affects text-format
    logs, with or without colour codes, for string and Debug fields. JSON-format logs mask it.
  - **D2, a quote or backslash directly before the `@` of a URL's user and password.**
    Example: `https://user:PREFIX"@host/path` and `https://user:PREFIX\@host/path` are written
    unchanged, password prefix included. A quote followed by `,`, `:`, `}`, `]` or another
    quote behaves the same. This affects text-format logs. JSON-format logs mask the prefix.
  - **D3, a number under a JSON member whose name only contains a secret word.** Example:
    `{"client_secret":123456789,"count":7}` keeps `123456789`. The same name with a string
    value is masked. This affects both formats: a logged JSON body in text-format logs, and JSON members in
    JSON-format logs.
  - **A literal escape spelling of `/`, `?` or `#` inside a URL password.** Code review round 3
    (`packet-9-code-review-r3/REVIEW-astra-code-r3.md`) listed this under the same class. In a
    text-format log, a password that literally contains the text `\u002f` (or another escape
    spelling of `/`, `?` or `#`, such as `\/` or `\u003f`) is read as a URL path boundary, so
    `https://user:PREFIX\u002fsuffix@host/path` is written unchanged. The cleanser cannot tell
    that text from a JSON escape.
- **URL rule (not a limit).** A `/` before the `@`, raw or JSON-escaped (`\/`, `\u002f`), means
  there is no userinfo: the span is a host, port and path, so it is left alone, the same as the
  raw form (`redact.rs:92-115`). For example `{"url":"http://host:9696\/dir@host/path"}` and
  `{"url":"http://user:p\/ss@host/path"}` are both written unchanged, as
  `http://user:p/ss@host/path` already was. The same holds for an escaped `?` or `#`. A
  percent-encoded `%2F` is not a boundary: that userinfo is masked.
- **Deferred alternative, not approved.** The code would declare each secret value as a type
  that prints `[REDACTED]` (for example the `secrecy` crate), with key-bearing URLs as a
  redacting type, and the pattern cleanser would stay as a backstop. The PO marked it as an
  unapproved to-do (2026-10-05).
- Redirect hardening. ST-304 shows the SABnzbd key can be forwarded in `Referer` by both
  clients, so by the poller today. Turning off redirects (or `Referer`) for download-client
  calls, as the approved Readarr path does (PA-007), would touch the poller and is a separate,
  unapproved item.
- Non-Unix. `remove_library_file` uses only std calls, but no non-Unix target is compiled on
  this host and Livrarr ships Linux images; behaviour there is unverified.
- Text printed outside tracing (stderr `eprintln!`, panics) is not cleansed. ST-101 lists the
  five `eprintln!` messages; none prints a stored secret.

## 5. Open Questions

Q-001 and Q-003 were decided by the PO on 2026-10-05; the rest are routine and settled here.

| ID | Question | Options | Decision and why | Status |
|----|----------|---------|------------------|--------|
| Q-001 | DL1: cleanser rules | (a) patterns only, as Sonarr; (b) patterns plus known stored values; (c) per-call-site fixes only | **(a), PO decision 2026-10-05**, plus the LLM test's own blanking (REQ-106) for the one known bare echo. Mask stays `[REDACTED]` so existing tests and habits hold. Cost: the limit in §4. | settled |
| Q-002 | DL2: which secrets; setup-token exception | — | ST-107's list; the setup token keeps its stdout banner and file only (REQ-105). | settled |
| Q-003 | DL3: LLM-test error body | (a) drop it; (b) keep it, cleansed; (c) keep it, with the sent key blanked, then cleansed | **(c), PO decision 2026-10-05.** The reply tells the admin to read the logs for details (`config.rs:418`); the body is the only detail. Cost: for a very short key, ordinary words equal to it in the reply are blanked too; a JSON reply is logged re-encoded, with members sorted and whitespace dropped (ST-115). | settled |
| Q-004 | DL4: `FetchRequest` `Debug` | (a) mask listed headers; (b) mask every header value | **(b).** No list to keep current; header names still show. | settled |
| Q-005 | DU1: no root folder, or root gone | (a) skip the item's file and count it; (b) refuse the whole undo | **(a).** Uses the item's own root, which always exists in the database (ST-202, ST-206), so the case narrows to a root missing on disk. (b) would need a new error mapping in the handler (`readarr_import.rs:86`, today a 500). | settled |
| Q-006 | DU2: how skipped files are counted and shown | (a) count and log only; (b) also warn on the page | **(b).** Today the page ignores the reply (ST-207), so a kept file is invisible. Small, in the item's own page. | settled |
| Q-007 | DC1: the fix, and redirects | (a) trusted client; (b) trusted client without redirects for this lookup; (c) also change the poller | **(a).** The packet's bar is "no worse than the poller"; (b) makes Retry differ from the poller, and (c) is outside scope (§4). | settled |

## 6. Acceptance Criteria

Doors: **B** = the Cargo-built binary on a temp data folder (ST-110), with real local HTTP
fakes and, where a grab, work or indexer row is needed, rows written with the real `SqliteDb`
writers while the binary is stopped, then restarted. **B-local** = door B with the harness
option that starts the binary with `NO_PROXY=localhost,127.0.0.1` instead of `""`
(`test_fresh_author_index.rs:87`), so requests to local fakes on `localhost` or `127.0.0.1`
go direct while every other outbound request (Hardcover and the other public providers)
still reaches the refusing proxy (ST-110). Existing callers of the harness keep
`NO_PROXY=""`. Each fake records the requests it receives, and a criterion that relies on a
fake asserts that record before its main observable. Test setup only; no production HTTP
change. **R** = `undo_import` over a real
`SqliteDb` and a real temp filesystem (the door of `test_wh_deletion.rs:283`). **F** = the real
`ReadarrImportPage` with the project's API stub. Red = fails on `0e06a205`; guard = passes
today and must keep passing.

**Recoverability check** (used by AC-101, AC-107, AC-108): a secret "appears" in a sink if
the secret or its JSON-escaped form is a substring of a text sink (stdout, stderr,
text-format file), or of any string obtained by parsing each JSON-format line and the
logs-tail reply as JSON and, where a string field itself parses as JSON, parsing that once
more (ST-113). In a text-format sink or tail line, the value after `body=` of the LLM WARN
event (the last field on its line, after removing ANSI codes) is also parsed as JSON where it
parses, and its strings searched, so an escape other than the serialiser's own is decoded in
both formats (ST-115). Every JSON-format line must parse.

| ID | Criterion | Door | Kind |
|----|-----------|------|------|
| AC-101 | With `[log] level = 'debug'`, run once in text and once in JSON format, with settings holding a distinct 12+ character value for every stored secret of ST-107. Flows, each with its reachability control: (a) `POST /config/metadata/test/llm`, fake answers 401 echoing the key; control: the WARN event "LLM test endpoint returned non-success" is present in every sink with `[REDACTED]` in its body. (b) indexer `POST /indexer/test` and `POST /indexer/import/prowlarr` against fakes that answer 401/500 echoing the key; control: reply 502 and the fake recorded the request carrying the key (`apikey=` query, `X-Api-Key` header). (c) qBittorrent and SABnzbd `POST /downloadclient/test` against fakes that answer `Fails.` / `{"error":"API Key Incorrect: <key>"}`; control: reply 502 and the fake recorded the login form or `apikey=` request. (d) the Retry of AC-301 with SABnzbd whose port refuses connection; control: the ERROR event `internal error:` containing "SABnzbd history request failed" (ST-112). (e) Hardcover `POST /config/metadata/test/hardcover` through the refusing proxy; control: reply 502 "Hardcover connection failed". (f) login, then requests with the session token and with `X-Api-Key`; control: the INFO event `login successful` is present in stdout, the file and the tail (`auth_service.rs:256`, ST-112), and the authenticated requests reply 200. (g) `POST /config/email/test` to a closed SMTP port; control: reply 400. Capture control for every run: the startup event `Livrarr starting — data directory:` is present in stdout, the file and the tail. Observable: by the recoverability check, no secret value, session token or API key appears in stdout, stderr, any `logs/livrarr.log.*`, or `GET /system/logs/tail?lines=200` read after each flow. (a) is red; (b) to (g) are guards (ST-112: they write no secret today). | B-local | red (a) |
| AC-102 | The log cleanser masks every pattern shape of REQ-102 and §2a (except the accepted limits in §4), including the ANSI-decorated field and the escaped JSON member; a JSON event line holding an escaped secret member still parses after cleansing; the existing ordinary-URL cases (`redact.rs:87-97`) stay byte-identical through it. | the function | red |
| AC-103 | Removed in v2 (registry removed by the PO decision; see Revision). | — | — |
| AC-104 | `format!("{:?}", FetchRequest{…})` with `X-Goog-Api-Key` and `Authorization` shows both header names and neither value, and shows the body length, not its bytes. | the type | red |
| AC-105 | The setup token appears in the stdout banner and `setup-token` only, with the cleanser active: `setup_token_appears_only_in_the_stdout_banner_with_verbose_logging` passes unchanged. | B | guard |
| AC-106 | Door B-local. Two local fake Torznab indexers (rows written while stopped; origins trusted at startup, `main.rs:222-234`): one answers one item, the other `<error code="100" description="access_token=abc"/>`. `GET /api/v1/release?workId=<id>`. Control, checked first: each fake recorded one search request. Observable: the failing indexer's warning `error` field equals `error 100: access_token=abc`, as today. Only that field is asserted. No log copy of the warning exists (ST-105), so none is checked. Run with `NO_PROXY=""`, both requests reach the refusing proxy and the warning reads `All indexers failed` (`release_service.rs:291`; `crates/livrarr-handlers/src/release.rs:54-59`), so the case would fail before reaching the redactor. | B-local | guard |
| AC-107 | Door B-local. Through the real LLM test route, in text and JSON format, with the LLM endpoint on a local fake: (i) key `hunter2` (7 characters), fake answers 401 with "Incorrect API key provided: hunter2. Check your settings." (not JSON); (ii) key `probe"quote\back-1234`, fake answers 401 with the JSON body `{"error":{"message":"Incorrect API key provided: <key>"}}` built by a JSON serialiser; (iii) key `probe&key-1234`, fake answers 401 with the literal body `{"error":{"message":"Incorrect API key provided: probe\u0026key-1234"}}`; (iv) key `probe/key-1234`, fake answers 401 with the literal body `{"error":{"message":"Incorrect API key provided: probe\/key-1234"}}`; (v) key `12345678901234`, fake answers 401 with `{"error":{"code":12345678901234,"message":"Incorrect API key provided"}}`. Control, checked first: the fake recorded `Authorization: Bearer <key>`. Observable: the WARN event "LLM test endpoint returned non-success" is present in stdout, the file and the tail, its body still contains "Incorrect API key provided" and `[REDACTED]`; by the recoverability check the key appears in none of them; every JSON-format line parses. A blanking limited to the raw key and its serialiser spelling fails (iii) and (iv) (ST-115); a JSON-decoding blanking without the literal pass fails (v). | B-local | red |
| AC-108 | `config.toml` holds the quoted root keys `"x?apikey=<v1>"` and `"y access_token=<v2>"` (distinct 12+ character values). Start the binary, set up and log in, read the tail. In text and JSON format: both `Unknown config key:` WARN events are present in stdout, the file and the tail, each with `[REDACTED]`; by the recoverability check neither value appears. An LLM-only fix fails this case (ST-111). | B | red |
| AC-201 | One import, ebook root `R` (the import's root and each item's root), with these stored paths and files: inside; an absolute path to a regular file inside `R`; `../outside/x`; an absolute path outside; an entry that is a link to an outside file; `1/linkdir/x` where `linkdir` links outside; an entry that is a folder; a missing file. Observable: after undo every outside file and the link still exist, the folder exists, both inside files are gone; reply `filesDeleted == 3`, `filesSkipped == 5`; one WARN line per skipped path names its reason. | R | red |
| AC-202 | An import with `target_root_folder_id` none whose items' root folder exists on disk: the inside file is removed, and a file at the same relative path under the process working folder still exists. An item whose root folder path is missing on disk: its file counts as skipped. | R | red |
| AC-203 | Undo reply `filesSkipped: 2` shows the warning toast with "2"; `filesSkipped: 0` shows "Import undone". | F | red |
| AC-204 | The existing undo test's history assertions still hold; its `files_deleted == 2` (ST-208) is updated to the new counts or its fixture given real roots. Listed as an allowed test edit. | R | — |
| AC-205 | Two roots: ebook root `A` (the import's `target_root_folder_id`) and audiobook root `B`. Item E (root `A`, stored path `p1`) and item M (root `B`, stored path `p2`), both on the import. Files: `A/p1`, `B/p2`, and decoys `A/p2` and `B/p1`. Observable: after undo `A/p1` and `B/p2` are gone, both decoys still exist; reply `filesDeleted == 2`, `filesSkipped == 0`. Both "use the import's root" and "use the first item's root" fail. Justification for seeded state: today's import gives every item the import's root (ST-114), so the rows are written with the real `SqliteDb` writers to pin REQ-201's per-item root, which a same-root fixture cannot. | R | red |
| AC-301 | A qBittorrent client configured with host `localhost` and the port of a local fake (fake answers `torrents/info` with a content path only when the query has `hashes=<download id>`); a grab in `importFailed` with that download id and no content path. Pressing Retry (`POST /api/v1/grab/{id}/retry`). Observable: after the binary stops, `get_grab` shows `content_path` equal to the fake's path. Repeat with host `127.0.0.1` (guard). | B-local | red |
| AC-302 | A SABnzbd client configured as `127.0.0.1:<port>` whose history call answers 302 to `http://localhost:<port2>/…`, where a second fake answers the history JSON with the grab's `nzo_id` and a `storage` path. Retry as in AC-301. Observable: the grab's `content_path` equals that `storage` path (the trusted client followed the redirect, as the poller's would; the safe client refuses the `localhost` hop). Pins parity with the poller, not safety; revise it with the redirect follow-up of §4. | B-local | red |
| AC-303 | Same as AC-301 with the port closed: reply 500, the grab's status is `importFailed`, and its error names the login failure. | B-local | guard |

### Added in code review

Code review found shapes REQ-102 covers that the criteria above did not name. Each is pinned by
a unit test in `crates/livrarr-domain/src/redact.rs` that calls `cleanse_log_line` directly
(door: the function, as AC-102). Line numbers are the test functions at `497b391e`.

| ID | Criterion | Test | Kind |
|----|-----------|------|------|
| AC-109 | (Review r1 C1.) A structured field the text formatter quotes (string and Debug values) is masked and keeps its quotes: `password="QUOTEDSECRET"` becomes `password="[REDACTED]"`, also with ANSI colour codes around `=`, where the colour reset stays and is never masked in place of the value. Inside a JSON-format event, a Debug string field and a quoted field-shaped fragment in a message are masked and the event still parses. Guard: `{"fields":{"message":"set token=","level":"x"}}` stays byte-identical, so the end of a JSON string is never taken for an opening quote. | `log_cleanser_masks_a_quoted_structured_field_and_keeps_its_quotes` (`redact.rs:356`), `log_cleanser_masks_quoted_fields_inside_a_json_event` (`:382`); guard `log_cleanser_keeps_a_json_string_ending_in_a_field_name_intact` (`:409`) | red; guard |
| AC-110 | (Review r1 C2.) In a JSON-format event, every structured-field name of REQ-102 is masked, any case (`auth`, `Passwd`, `AUTHKEY`, `access_token`), and a numeric value under such a name (`password`, `token`) becomes the string `"[REDACTED]"`, also inside a JSON body carried in a field. Ordinary neighbours (`id`, `count`, `code`) keep their values and every event parses. | `log_cleanser_masks_every_structured_field_name_in_a_json_event` (`redact.rs:415`), `log_cleanser_masks_numeric_structured_fields_in_a_json_event` (`:443`) | red |
| AC-111 | (Review r1 C3.) An apostrophe, double quote or backslash inside `user:pass@` no longer cancels the match: for `https://user:prefix'suffix@host/path` and its `"` and `\` variants, the text before the delimiter is masked, `[REDACTED]` appears and `@host/path` stays, in a plain line and in a JSON event that still parses. Guard: host-and-port URLs beside quoted values and e-mail addresses stay byte-identical. | `log_cleanser_masks_userinfo_before_a_quote_or_backslash` (`redact.rs:475`); guard `log_cleanser_keeps_host_and_port_urls_beside_quotes_byte_identical` (`:500`) | red; guard |
| AC-112 | (Review r2 D4.) An escaped `/`, `?` or `#` ends userinfo, so an ordinary URL with `@` in its path is left alone: `{"url":"http://host:9696\/dir@host/path","id":7}`, the `\u002f`, `\u002F`, `\u003f` and `\u0023` spellings, and the text forms `\?` and `\#` are all byte-identical. | `log_cleanser_keeps_a_url_with_an_escaped_path_boundary_byte_identical` (`redact.rs:511`) | red |

Checks before hand-back: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`
with zero warnings, the targeted tests above, the `livrarr-server` and `livrarr-behavioral`
test binaries they live in, and for REQ-203 the page's unit tests plus the frontend
typecheck.

## 7. As built: corrections and limits

The code was built by `fix` subagents on Claude Opus 5.5 (high): packet 4 built all three parts,
packet 6 fixed C1 to C3, packet 8a fixed D4, and packet 8b folded spec v4. The tests were written
by Opus 5.5 with the PO's approval and reviewed by GPT-6 Astra (r1 FAIL, r2 PASS). Astra (xhigh,
sole reviewer) reviewed the code three times: r1 FAIL (C1, C2, C3), r2 FAIL (D1 to D4), r3 PASS.
The server binary and the web app were deployed to the local app at 2026-10-05T17:53Z, with no
migration and no database change (`build/reviews/security-before-release/deploy-20261005T175224Z/DEPLOYMENT.json`).
The PO ran the live check in `LIVE-CHECK.md` and said "done". The change is committed locally as
`497b391e`; the push waits for the end of the feature close (PO).

### Corrections found during the build

What the spec missed, as the reviewer found it. None of these changed a decision.

- **The text formatter quotes string and Debug field values (review r1 C1).** REQ-102 and ST-103
  described a structured field as `name=value`. The text formatter writes a string or Debug
  value as `name="value"`, so the field rule, which required an unquoted value right after `=`,
  left `password="QUOTEDSECRET"` whole. With colour on, it could mask the ANSI reset instead and
  leave the secret beside `[REDACTED]`. As built, a quoted value is masked and its quotes stay
  (`redact.rs:154-156`, `QUOTED_VALUE` `:49`), and a value never runs into an ANSI code (`VALUE`
  `:44`). AC-109.
- **JSON-format events carry the same field names and numeric values (review r1 C2).** The JSON
  member rule used only the substring words of REQ-102's JSON bullet, so `auth`, `passwd` and
  `authkey` fields were left in JSON-format logs, and no rule matched a number:
  `"password":123456789` stayed in JSON while text format masked it. As built, the text-field
  names (`SECRET_KEYS`, `redact.rs:60-61`) also apply to JSON members (`:130`), and a number
  under such a name becomes the string `"[REDACTED]"` (`:169-174`, `mask_number` `:87-90`).
  AC-110.
- **A delimiter inside userinfo cancelled the whole match (review r1 C3).** The log cleanser's
  copy of the `user:pass@` rule forbade quotes and backslashes in the password but still required
  the `@`, so `https://user:prefix'suffix@host/path` was left whole, where `redact_secrets` masks
  it. As built, the span accepts escape sequences, apostrophes and a double quote not followed by
  JSON structure, and every run of ordinary characters in it is masked while quotes and escapes
  stay (`redact.rs:69`, `:73-74`, `:109-125`). AC-111.
- **The C3 fix let an escaped slash cross a URL path boundary (review r2 D4, a regression caught
  and fixed in review).** Accepting any escape inside userinfo let the rule run through `\/` to a
  later `@`, so `{"url":"http://host:9696\/dir@host/path","id":7}` lost its host and path. As
  built, a span holding an escape of `/`, `?` or `#` is returned unchanged
  (`escapes_url_boundary`, `redact.rs:92-102`, used at `:110-115`). The check sits in the mask
  function rather than the pattern because the regex crate has no lookahead. AC-112; the URL
  rule is stated in §4.
- **Three shapes left as accepted limits (review r2 D1 to D3).** PO decision "3": ship with D4
  fixed and D1 to D3 recorded in §4. Round 3 added one more shape of the same class (§4).

Two build choices, accepted in review r1: a secret-named field is also caught right after `"`
and at the start of a line, not only after `?`, `&` or a space; and a masked value stops at
`&`, `#`, whitespace, a quote or a backslash, where `redact_secrets` also stops at `<` and `>`,
so masking never breaks a JSON line. `Bearer` and `Basic` match in any case, so ordinary text such as "basic setup"
is logged as "basic [REDACTED]"; the reviewer judged this follows REQ-102's rule, and the PO was
told at the live check.

### Limits of the evidence

- **Masking cases are pinned on the function, not the binary.** AC-102 and AC-109 to AC-112 call
  `cleanse_log_line` directly. That the sinks run it is covered by the binary tests in
  `crates/livrarr-server/tests/test_fresh_author_index.rs`: unknown config keys
  (`:2436`, `:2441`), the REQ-102 shapes through every sink (`:2731`, `:2736`), the LLM test
  (`:2934`, `:2939`) and the AC-101 flows (`:3764`, `:3769`), each in text and JSON format. The
  coders and the reviewer drove the real tracing formatter through the cleanser in scratch
  probes for C1 to C3 and D1 to D4 before their fixes; the D4 fix itself was not re-run
  through the formatter or the binary.
- **The live check shows routine running leaks nothing, not that each path works live.** After
  the deploy the PM searched the 53 log lines written since the start for the 7 saved keys and
  passwords read from the database: 0 found, and 0 `[REDACTED]`
  (`deploy-20261005T175224Z/secret-scan.txt`). The PO's connection tests wrote no log lines,
  because a successful test does not log, so no masking path was exercised live.
- Not tested: output that is not valid UTF-8 passes through the writer uncleansed (the reviewer
  found it unreachable from the formatter's string buffer); if the background removal task in
  undo panics, every file counts as skipped with one import-level WARN rather than one per file
  (`readarr_import_workflow.rs:1229-1232`); non-Unix builds (§4).
