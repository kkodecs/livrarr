# First-run setup token

## Executive summary

A fresh install will not create its first account without a one-time setup token. This stops
whoever reaches the port first from becoming admin. The token is made at startup while setup is
pending. It is printed to the console (`docker logs livrarr`) and saved as `setup-token` in the
data folder (`/config/setup-token` in Docker). The setup page asks for it; a missing or wrong token
is refused with 403. The token is used up the moment the account is claimed, and its file is then
deleted. An install that finished setup gets no token, no banner and no change in behaviour.

- [When the token is made](#when-the-token-is-made) and [where it appears](#where-it-appears).
- [File rules](#file-rules): reuse, private rewrite, fail closed.
- [The check and single use](#the-check-and-single-use).
- [Completed installs](#completed-installs).
- [Limits](#limits).

Source: main at `d77f690a`, delivered by the prerelease-trust-pass feature
(`spec-prerelease-trust-pass.md` v6). Line numbers are at that commit.

## When the token is made

At startup the order is data folder, config and logging, database, then the auth service, then the
HTTP server (`crates/livrarr-server/src/main.rs:134-139`). While the auth service is built,
`prepare_setup_token` asks the database whether setup is pending
(`main.rs:981-999`). The database's pending-setup flag is the only authority; the token and its
file are proof, never state (`crates/livrarr-server/src/setup_token.rs:3-4`).

If setup is pending, `issue_setup_token` reuses a valid token file or makes a new token: 16 bytes
from `getrandom`, lowercase hex, 32 characters (`setup_token.rs:66-74`, `:90-95`). The auth
service then holds it in memory (`main.rs:971-974`; `crates/livrarr-server/src/auth_service.rs:58-63`).
A restart before setup reuses the same token.

## Where it appears

Only in two places:

- **A banner printed straight to stdout,** not through the log macros, so it never reaches the log
  file or the in-memory log that the Logs and Help pages read (`main.rs:1012-1022`). One INFO line
  names the file without the token (`main.rs:1023-1026`).
- **The file `{data}/setup-token`** (`setup_token.rs:17`, `:59-61`).

It is never in an API response. `SetupRequest`'s `Debug` output redacts it
(`crates/livrarr-handlers/src/types/auth.rs:106-117`), and so does `SetupToken`'s
(`setup_token.rs:49-56`).

## File rules

- **Reuse only an exact file.** The path is examined without following links. It is reused only
  if it is a regular file whose whole contents are exactly 32 lowercase hex bytes. A newline,
  whitespace, extra text, a link, a directory or any other file type means a new token
  (`setup_token.rs:104-124`).
- **Rewritten privately on every start.** The token goes into a new temporary file in the data
  folder, created with create-new and mode 0600, then renamed onto `setup-token`. The rename
  replaces whatever entry was there and never writes through a link (`setup_token.rs:152-186`).
- **Fail closed.** If the write or rename fails (for example the path is a directory), Livrarr
  prints `Cannot write the setup token file <path>: <error>. Fix the data folder and restart.` to
  stderr and exits 1 before the HTTP server binds (`main.rs:1001-1011`).
- **Non-Unix refuses.** Without a no-follow open and owner-only creation, no file is reused and the
  write fails, so startup stops (`setup_token.rs:135-139`, `:188-201`). Livrarr ships Linux images
  only (`.github/workflows/release.yml:26-29`); macOS is Unix and keeps the protections.

## The check and single use

The setup page has a required "Setup token" field with a hint pointing at `docker logs livrarr` and
the `setup-token` file (`frontend/src/pages/setup/SetupPage.tsx:318-332`). The token travels in the
JSON body as `setupToken` (`frontend/src/stores/auth.ts:107-108`) and is never stored in the browser.

`ServerAuthService::complete_setup` checks it after "setup already complete" and before any
password hashing (`auth_service.rs:278-282`). The offered token is trimmed of surrounding whitespace
and compared in constant time (`auth_service.rs:65-83`). Missing, blank or wrong gives **403** with
the fixed message "The setup token is missing or wrong. Find it in Livrarr's startup output, or in
the file setup-token in its data folder (/config/setup-token in Docker)."
(`crates/livrarr-handlers/src/types/auth.rs:235-239`;
`crates/livrarr-handlers/src/types/api_error.rs:620`). A service built without a token refuses
every setup (`auth_service.rs:47-55`, `:75`).

With the right token, `UserDb::claim_setup` writes the account in one conditional update and
returns the id once it commits (`crates/livrarr-db/src/sqlite_user.rs:212-251`). The service drops
its token straight away, before the session is made, then deletes the file as a best effort
(`auth_service.rs:301-313`, `:85-98`). A later request gets the existing 409. Two simultaneous
right-token requests still give one winner through the conditional update. The per-IP setup limit
of one attempt per 12 seconds stays (`crates/livrarr-server/src/router.rs:43-48`).

## Completed installs

When setup is not pending there is no token and no banner. A leftover `setup-token` is removed as a
best effort: a file or link is unlinked, a directory is never removed, and a failure only logs a
WARN without the token (`main.rs:996-999`; `setup_token.rs:76-88`). Sessions and API keys keep
working across the upgrade restart.

## Limits

- The non-Unix branches have never been compiled on the development host; they were reviewed from
  source only.
- A token that sat in a readable file before an earlier start stays valid; anyone who could read
  that file could also read the database in the same folder.
- There is no environment variable for the token, by the TOML-only config rule. Scripted installs
  read the file.
