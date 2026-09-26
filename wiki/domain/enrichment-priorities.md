# Enrichment provider priorities

## Executive summary

The server reads provider priorities from the database once when it starts, and
keeps that copy in server memory. For ordinary English metadata, Google Books sits
between Hardcover and Goodreads. See [the orders](#provider-orders) and
[when changes take effect](#database-and-startup).

This implementation passed tests and review on `fix/add-metadata-preservation`
and is deployed. The follow-on fix that saves the selected Add details and protects
saved values from lower-priority replacements is also done: it went live on
2026-09-21, the current build went live on 2026-09-23, and the branch was merged to
main and pushed the same day
([status record](../../build/state/STATUS-add-metadata-preservation.md)). The dated
test evidence that found the overwrite defect is kept in
[saved-value protection](#saved-value-protection); see also [verification](#verification).

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

The [Add field mapping](search-result-metadata.md) records the research behind the
separate save fix. Saving the selected result's metadata at Add time and the agreed
provider-overwrite rules shipped with that fix
([status record](../../build/state/STATUS-add-metadata-preservation.md),
[specification](../../spec-add-metadata-preservation.md)). Neither change refreshes
the library automatically.

## Saved-value protection

<a id="saved-value-protection-is-still-pending"></a>

On 20 September, isolated tests against branch base `dbb821b0` confirmed that a later
lower-priority offer could replace a saved higher-priority description. Both fresh
and cached merge paths did this. A pass with no usable offer could keep the description
but erase the record of which provider supplied it. These are separate from
choosing the best offer among providers responding in the current pass.

The tests use the actual merge engine and SQLite writer with controlled inputs;
they do not modify the live library. Five behavioral cases failed for these
reasons at the time. Two additional import fixtures were rejected during setup and
investigated separately; those are not evidence of another live defect. See the
[execution record](../../build/reviews/add-metadata-preservation/tests/J4/PM-RED-OBSERVATION.json).
The [accepted save and overwrite decisions](../../build/reviews/add-metadata-preservation/PM-DECISIONS.md)
defined the intended fix. The fix was implemented, reviewed, deployed (2026-09-21;
current build 2026-09-23) and merged to main on 2026-09-23; the
[status record](../../build/state/STATUS-add-metadata-preservation.md) names the
deployment receipts, the database backups and the full test run (2,332 passed, 0 failed).

## Verification

The tests exercise real SQLite and the production startup, enrichment and merge
paths. External responses are controlled fixtures. They cover fresh/cached
ordering, restart behavior, English Google Books dispatch, language routing,
cover independence, migration upgrades and invalid data.

See the [implementation record](../../build/reviews/provider-priority-cache/RESULT.md)
for the code checks, the [live deployment receipt](../../build/reviews/provider-priority-cache/deployment-20260918T235514Z/RECEIPT.md)
for runtime verification, and the
[specification](../../spec-provider-priority-cache.md) for exact scope.

The [live trace of two user additions](../../build/reviews/provider-priority-cache/live-add-trace-20260919T000839Z/RESULT.md)
confirmed Google Books supplying English metadata and Hardcover winning competing
fields. No usable Goodreads response was available for a direct GB/GR comparison.

That trace did not verify identifiers in Book Information. A subsequent user report
found an apparent gap: Jekyll has a saved ISBN, placed under Details by the page,
but no other saved catalog IDs and no Google Books ID row. See the
[identifier follow-up](../../build/reviews/provider-priority-cache/live-add-trace-20260919T000839Z/IDENTIFIER-FOLLOWUP.md)
for confirmed storage, display limitations and the remaining browser check.

## Source and history

The version of this page main carried until 2026-09-26 (which still said the save fix
was pending) is preserved under
`build/reviews/steward-upkeep-2026-09-26/hand-merge/predecessors/`.
