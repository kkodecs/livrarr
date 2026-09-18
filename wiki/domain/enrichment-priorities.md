# Enrichment provider priorities

## Executive summary

The metadata branch now reads provider priorities from the database once when
the server starts, and keeps that copy in server memory. For ordinary English
metadata, Google Books sits between Hardcover and Goodreads. See
[the orders](#provider-orders) and [when changes take effect](#database-and-startup).

This implementation passed tests and review on `fix/add-metadata-preservation`;
it has not been deployed. The separate fix to save details supplied when Add
creates a new Work is still pending. See [scope](#scope) and
[verification](#verification).

## Provider orders

| Information | English or unknown book language | Other book languages |
|---|---|---|
| Ordinary metadata, including descriptions | Hardcover → Google Books → Goodreads → Readarr → OpenLibrary → Audible | Google Books → Goodreads → Readarr → Audible |
| Audio details, such as narrator and duration | Audible → Audnexus → Hardcover → Goodreads → OpenLibrary → Google Books | Audible → Audnexus → Goodreads → Google Books |

Google Books was already first for other languages. Hardcover and OpenLibrary
remain excluded there. An explicitly stored language policy takes precedence
over the generic list, subject to the existing provider and language safeguards.
Readarr contributes retained import data; it is not a network enrichment client.

These lists choose among usable values for each field. They do not control the
order in which requests are sent: eligible providers still run concurrently.
Google Books now participates in English enrichment through its existing client,
with its existing key and identity requirements.

## Database and startup

The existing `provider_policy` table holds the ordered lists. Its `ebook` kind
means ordinary metadata; `audiobook` means audio details. These historical names
do not mean that a Work gets only one list based on the file formats it has.

After migrations, startup loads and validates one immutable snapshot. Fresh
provider responses and reusable cached responses consult the same snapshot.
A later database edit takes effect on the next server restart. There is no new
settings page and no priority lookup for each field or Work.

The migration supplies missing defaults and expands the exact old seeded generic
lists. Nonempty custom lists are preserved. Invalid provider names, list kinds,
out-of-range ranks or incomplete required lists stop startup with an error rather
than silently substitute another order.

## Scope

Cover selection retains its separate ranking and safeguards. Editing the audio
metadata list therefore does not reorder audiobook covers. Existing protection
for personal edits, identity and book language also remains.

The [Add field mapping](search-result-metadata.md) is still research for the next
change. Saving the selected result's metadata and comparing a replacement
provider with the provider of an already stored value are separate pending work.
This priority change does not refresh the library automatically.

## Verification

The tests exercise real SQLite and the production startup, enrichment and merge
paths. External responses are controlled fixtures. They cover fresh/cached
ordering, restart behavior, English Google Books dispatch, language routing,
cover independence, migration upgrades and invalid data.

See the [implementation record](../../build/reviews/provider-priority-cache/RESULT.md)
for the final check and deployment status, and the
[specification](../../spec-provider-priority-cache.md) for exact scope.
