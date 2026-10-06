# Health checks, config warnings and log files

## Executive summary

Livrarr has two health replies. The public `GET /api/v1/health` is for Docker and uptime
monitors: it reads the database, gives up after 2 seconds, and answers 200 or a fixed 503 with no
detail. The admin-only `GET /api/v1/system/health` feeds System → Status: the same database check
with its failure detail, then one amber row per config warning. The check proves the pool can hand
out a connection and SQLite can run a read; it does not prove the file is intact or writable
([health endpoints](#health-endpoints)). The Docker image carries its own health check on
`127.0.0.1:8789` ([Docker](#docker-health-check)).

Config warnings are built once, from the config loaded at startup, by one function that both the
log and the page use. They name an ignored `trusted_proxies` entry or an unknown key, never a value,
and change only after a restart ([config warnings](#config-warnings)). The proxy parser is shared
by the rate limiter and the warnings ([trusted proxies](#trusted-proxies)). Log files are capped at
today's plus 30 earlier, counted by file, not by calendar day ([log retention](#log-retention)).

When a page gains a new web call, add it to the whole-app test stub or that file times out
([testing a change](#testing-a-change)).

## Health endpoints

| | Public `/api/v1/health` | Admin `/api/v1/system/health` |
|---|---|---|
| Who | Anyone; mounted without login (`crates/livrarr-server/src/router.rs:70`) | Admins only, `RequireAdmin` (`crates/livrarr-handlers/src/system.rs:80-83`; route `router.rs:458-459`) |
| Healthy | 200, one item `database` / `ok` / "database is reachable" | 200, the database item, then config warnings |
| Check fails | 503, one fixed item `database` / `error` / "database check failed", nothing else (`system.rs:61-76`) | Still 200; database item `error` with "database check failed: {detail}", cleansed; warnings still follow (`system.rs:84-95`) |
| HEAD | Same status, empty body (axum's `get` route) | — |

Both call one function, `system::database_check` (`system.rs:33`). It runs the domain
`check_database` under a 2-second `tokio` timeout (`system.rs:28`); a timeout's detail is "did not
answer within 2 seconds" (`system.rs:48`). Every failure is logged at WARN with its detail
(`system.rs:50`). The public handler's only capability is `HasDatabaseHealth`, so it cannot reach
the config warnings; that is how detail stays off the public reply.

`check_database` on `SqliteDb` runs `SELECT count(*) FROM sqlite_schema` through the live pool
(`crates/livrarr-db/src/sqlite_database_health.rs:7-14`). On SQLite this means:

- **Detected:** a pool that cannot hand out a connection within 2 seconds (every connection held,
  or the pool closed), and any error SQLite raises on the read.
- **Not detected:** a deleted, moved or damaged file while connections stay open (open connections
  keep reading), a full disk (reads still work), or a database that refuses writes. A WAL reader does
  not wait for a writer, so a long write transaction does not fail the check.

These limits were probed against the real pool (`spec-operations-hygiene.md`, ST-005). When the
database cannot answer at all, an admin does not see a red row: the login check reads the database
first and fails, so System → Status shows its error page with Retry (ST-003). A red row appears
only when login gets through and the check then fails.

Tests make the check fail with `set_schema_reads_refused` (`crates/livrarr-db/src/lib.rs:141`, in
`test_helpers`). It installs a SQLite authorizer that refuses reads of the schema table, so the
real statement fails in the real engine while logins still work.

## Docker health check

The runtime image declares (`Dockerfile:111-112`):

`HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["wget","-q","-T","4","--spider","http://127.0.0.1:8789/api/v1/health"]`

The time limits nest: server check 2 s, `wget` 4 s, Docker 5 s. busybox `wget --spider` sends GET
and exits 1 on a 503, so a failed database check makes the container unhealthy. The 60-second start
period covers migrations before the listener opens. The URL is fixed: an admin who changes
`[server] port`, or sets `bind_address` to one specific address, inside the container must set
their own health check (`README.md:104`; `docs/llm-context.md:246`). `url_base` has no effect and
does not matter here.

`docker-compose.yml:27-32` keeps its own `wget` check, because the example selects the alpha6
image, which has no built-in check. A compose `healthcheck:` overrides the image's. The image check
reaches Unraid and plain `docker run` users only with the first release image built after it.

## Config warnings

`config::config_warnings` (`crates/livrarr-server/src/config.rs:456-472`) builds every message from
the loaded `AppConfig`: first each rejected `trusted_proxies` entry in file order, then each
unknown key in the loader's sorted order. Each passes through `cleanse_log_line`, so the page and
the log show the same text.

- **Startup** logs each one at WARN once, after logging starts (`crates/livrarr-server/src/main.rs:733-735`).
- **The page** gets them through `HasConfigWarnings`, which `AppState` implements by calling the
  same function on `AppState.config` (`crates/livrarr-server/src/state.rs:787-791`). System →
  Status reads `/api/v1/system/health` under the query key `system-health`
  (`frontend/src/pages/system/status/StatusPage.tsx:186-189`) and shows each `config` item as an
  amber `warning` row.
- **Only after a restart.** The file is read once at startup by `config::load_config`
  (`config.rs:422`), which also stores the unknown keys in `AppConfig.unknown_keys`, a field serde
  skips (`config.rs:32-36`, set at `:444`). Fixing `config.toml` clears a row only when Livrarr
  restarts; the page does not say so.
- **Names, never values.** An unknown key is reported as `key`, `section.key` or a table name, as
  "Unknown config key: {key}"; the loader's `unknown_keys` returns names only and
  `config_warnings` never sees the table. A secret-shaped key name is masked by the cleanser.
- Warnings never stop startup.

`load_config` also still returns the unknown keys as the second part of its tuple; startup ignores
that copy (`main.rs:721`).

## Trusted proxies

`[server] trusted_proxies` decides whose proxy headers the per-address rate limits believe. One
parser serves both the router and the warnings: `rate_limit::parse_trusted_proxies`
(`crates/livrarr-server/src/rate_limit.rs:73-83`) splits entries into accepted ranges and rejected
entries; the router keeps the accepted ones (`router.rs:20-24`) and `config_warnings` reports the
rejected ones.

`IpNet::parse` rules (`rate_limit.rs:53-69`):

- Surrounding spaces are trimmed; `" 10.0.0.5 "` is accepted.
- An address (`10.0.0.1`, `::1`) or a range (`10.0.0.0/8`, `fd00::/8`) is accepted.
- Rejected: an empty string, a host name, a port (`10.0.0.1:443`, `[::1]:443`), a missing or
  non-numeric prefix, and a prefix above 32 (IPv4) or 128 (IPv6). A rejected entry trusts nothing.
- A prefix of 0 is accepted: `0.0.0.0/0` matches every IPv4 address and `::/0` every IPv6 address,
  never the other kind (`rate_limit.rs:25-27`, `:35-37`).
- Duplicates and the default empty list give no warning.

The key for a request (`rate_limit.rs:113-141`): an untrusted peer is keyed by its own address. A
trusted peer's valid `X-Real-IP` is the key. Without it, `X-Forwarded-For` is walked from the right
and the first untrusted address is the key; if every address in the chain is trusted, the key falls
back to the peer (`rate_limit.rs:96-107`). Under a `/0` entry every address of that kind is
trusted, so a proxy that sends only `X-Forwarded-For` gives all its clients the proxy's bucket,
while one that sends `X-Real-IP` gets per-client buckets. That walk is left as it is.

## Log retention

`log_surface::build_file_appender` (`crates/livrarr-server/src/log_surface.rs:22-28`) builds the
daily appender: prefix `livrarr.log`, files named `livrarr.log.YYYY-MM-DD` by UTC date,
`max_log_files(31)` (`log_surface.rs:18`). Startup calls it (`main.rs:1309-1321`).

- **What is kept.** `tracing-appender` prunes when the appender is built (startup) and when it
  opens a new day's file, deleting until 30 remain before it opens the new one. After a change of
  day: today's file and the 30 most recent earlier files. After a restart later the same day:
  today's file and 29 earlier.
- **Counted by file, ordered by creation time.** Days Livrarr did not run leave no file and use none
  of the 30, so the oldest kept file can be older than 30 days. Order is file creation (birth)
  time, not the date in the name; a copied folder whose newest dates were created first loses
  those first.
- **Undated files count.** Every regular file whose name starts with `livrarr.log` counts and can
  be deleted, including an old undated `livrarr.log` or a `livrarr.log.bak`. Other names and
  directories are left alone.
- **Build failure.** If the appender cannot create its first file (a read-only folder, or a
  directory named as today's file), startup prints the error to stderr, records it in
  `logInitError` of `GET /api/v1/system/status`, and runs without the file layer. System → Status
  does not show that field.

Pruning at a change of day is read from the library source, not driven in a test; startup pruning
is driven through the real binary (`spec-operations-hygiene.md`, AC-317).

## Testing a change

- **Health failures:** use `set_schema_reads_refused` through the real `build_router`; closing the
  pool also blocks login, so an admin request cannot reach the handler on a closed pool.
- **Config warnings:** write `config.toml` to a temporary folder and load it with the real
  `load_config`; the binary tests in `crates/livrarr-server/tests/test_fresh_author_index.rs` cover
  startup's own loader.
- **New web calls:** see [lesson 112](../insights/tests-and-fixtures.md#lesson-112): every
  whole-app test stub that renders the page must answer the new call.

## Source

Read at `25ca4f30`. Requirements, probes and the as-built record:
`spec-operations-hygiene.md` (v4), System Truths ST-005, ST-011, ST-020 to ST-022, ST-027, and §8.
