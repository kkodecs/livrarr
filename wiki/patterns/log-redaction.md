# Log redaction

## Executive summary

Every log line Livrarr writes through tracing passes through one function,
`cleanse_log_line`, before it reaches the console, the log file or the in-app Logs page. It
masks secret-shaped text with `[REDACTED]` by pattern, in the Sonarr style: Livrarr keeps no
list of its saved secret values. Text the app returns to users goes through a different, narrower
function, `redact_secrets`, which this filter does not change. Some secret shapes are known to slip
through; they are accepted limits listed in the spec. The PO prefers, in principle, a later design
where code declares a value secret through its type and patterns become a backstop; it is not
approved yet.

[Where it runs](#where-it-runs) · [Two functions, two jobs](#two-functions-two-jobs) ·
[The rules, in order](#the-rules-in-order) · [The LLM test](#the-llm-connection-test) ·
[Limits](#accepted-limits) · [Deferred alternative](#deferred-alternative) ·
[Testing](#testing-a-change)

## Where it runs

Source baseline: `497b391e` (security-before-release, 2026-10-05).

- **Console and log file.** Both fmt layers write through `CleansingWriter`, in text and JSON
  format (`crates/livrarr-server/src/main.rs:1317`, `:1323`, `:1345`, `:1352`). The wrapper
  cleanses each write (`:1375-1410`); the fmt layers write each event with one `write_all`, so
  a write holds whole events. Bytes that are not valid UTF-8 pass through unchanged (`:1402`).
- **Logs page buffer.** `LogBufferLayer` formats its own line and cleanses it before storing it
  (`main.rs:1431`). The buffer feeds `GET /api/v1/system/logs/tail`.
- **`FetchRequest` `Debug`.** The hand-written `Debug` prints the URL through the cleanser, every
  header value as `[REDACTED]` and the body as its byte length
  (`crates/livrarr-domain/src/services/http.rs:94-129`).
- **Not covered.** Text printed outside tracing: the setup-token banner (`println!`, by design),
  `eprintln!` messages and panics.

## Two functions, two jobs

Both live in `crates/livrarr-domain/src/redact.rs` and are exported at
`crates/livrarr-domain/src/lib.rs:30`.

- `cleanse_log_line` (`redact.rs:193-202`) is for log sinks only.
- `redact_secrets` (`redact.rs:30-38`) builds text returned to users: release-search warnings and
  fetcher connection errors. It masks only five query keys and `user:pass@`. It stays narrower on
  purpose: widening it would change what the API returns.

Do not route returned text through `cleanse_log_line`, and do not widen `redact_secrets` to match
it.

## The rules, in order

`LOG_RULES` (`redact.rs:128-181`) runs these in order; each rule keeps its prefix and puts
`[REDACTED]` where the value was:

1. A Debug tuple `("Authorization", "…")` for `Authorization`, `X-Api-Key`, `X-Goog-Api-Key` and
   `Cookie`, with plain or escaped quotes.
2. A header line `Authorization: Bearer …`; the scheme word stays.
3. `Bearer …` or `Basic …` anywhere, any case. Ordinary text such as "basic setup" is logged as
   "basic [REDACTED]".
4. A query value or structured field (`apikey`, `*_token`, `password`, `passwd`, `auth`,
   `nzb_key` and the rest of `SECRET_KEYS`, `redact.rs:60-61`), with ANSI colour codes allowed
   around `=`.
5. The same field when the formatter quotes its value (`password="…"`); the quotes stay.
6. A JSON member whose name contains a secret word (`SECRET_MEMBER_WORDS`, `redact.rs:64`) or is a
   `SECRET_KEYS` name, with a string value.
7. The same member with backslash-escaped quotes, as a JSON body carried inside a JSON-format
   line.
8. A `SECRET_KEYS`-named JSON member with a number value; the number becomes the string
   `"[REDACTED]"`.
9. `user:pass@` in a URL. Quotes and escape sequences inside it stay, so JSON stays valid. A `/`,
   `?` or `#` before the `@`, raw or JSON-escaped, means there is no userinfo, and the span is
   left alone (`redact.rs:92-125`).

A masked value never runs past `&`, `#`, whitespace, a quote, a backslash or an ANSI code, so a
JSON-format line still parses after cleansing.

## The LLM connection test

The AI connection test knows the key it just sent, so it blanks that key out of the provider's
error reply before logging it (`blank_sent_key`, `crates/livrarr-handlers/src/config.rs:415`,
`:424-462`). A JSON reply is decoded, the key is replaced in every string, and the result is
re-encoded; then a literal pass replaces the raw and JSON-escaped spellings. The logged line then
goes through the sink cleanser like any other.

## Accepted limits

The PO accepted these on 2026-10-05; the full list with examples is in
`spec-security-before-release.md` §4 (Non-Requirements):

- a secret that no pattern recognises, such as a password quoted back in plain prose;
- a value holding `&`, `#`, whitespace, a quote or a backslash may leave its remainder visible;
- in text format, a quoted field value starting with `:`, `,`, `}` or `]`, and a quote or
  backslash right before the `@` of `user:pass@`;
- a number under a JSON member whose name only contains a secret word (`client_secret`);
- in text format, a URL password that literally contains `\u002f` (or another escape spelling of
  `/`, `?`, `#`) is read as a path boundary and left unchanged.

## Deferred alternative

The PO's stated principle: sensitivity should come from the code declaring it, not from inspecting
spelling. The deferred design would wrap each secret value in a type that prints `[REDACTED]` (for
example the `secrecy` crate), make key-bearing URLs a redacting type, and keep this cleanser as a
backstop. It is recorded in the project to-do list as unapproved.

## Testing a change

- The pattern cases are unit tests in `redact.rs` that call `cleanse_log_line` directly.
- Sink wiring is covered by binary tests that start the built server and read stdout, the log
  file and the logs tail in text and JSON format
  (`crates/livrarr-server/tests/test_fresh_author_index.rs:2436`, `:2731`, `:2934`, `:3764`).
- A clean live log is weak evidence: see lessons [108](../insights/tests-and-fixtures.md#lesson-108)
  and [109](../insights/tests-and-fixtures.md#lesson-109).
