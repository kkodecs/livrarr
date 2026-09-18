# Search-result metadata: provider mapping

## Executive summary

When Add creates a new catalogue Work from a search result, useful information is
lost at several hand-offs. All four providers can supply a cover address, and
several return descriptions, publication details, series and ratings that Livrarr
discards. See [the provider comparison](#provider-comparison) and
[the proposed field mapping](#proposed-field-mapping).

Two meanings need care: search language is not proof of a book’s language, and an
edition’s publication date is not necessarily the Work’s original publication
date. Hardcover subtitles are intentionally excluded because they can be wrong.
See [concrete examples](#concrete-examples).

This completes mapping research on `fix/add-metadata-preservation`. No metadata
mapping or save behavior has been changed. The next step is to agree the bounded
save implementation described in [the suggested scope](#suggested-scope).

## Provider comparison

These are the four ordinary text-search providers. Fields vary by individual
result; a missing value should remain missing. The observations reuse the three
live search samples captured on 18 September at 21:09 UTC, before the result cap
changed. Their source mappings are unchanged on the new branch.

| Provider | Useful fields returned by the provider | What currently survives discovery | What is lost before or during Add |
|---|---|---|---|
| Google Books | Title, optional subtitle, authors, actual language, volume date, description, publisher, pages, categories, ISBNs, cover; some results have ratings/counts | Title, first author, volume year, language, cover, one normalized ISBN | Other descriptions and publication details are omitted; rating/count are not even parsed. Add then drops the supplied language, year and cover address. |
| OpenLibrary | Work title/key, author names/keys, first publication year, cover, ISBN and Amazon identifiers | Title, first author/key, Work key, year, cover, first ISBN/Amazon identifier | Additional authors/identifiers are reduced. Language is assigned from the search setting rather than read from the payload. Add drops language/year/cover. |
| Hardcover | Title, authors, description, series/position, genres, release date/year, pages, ISBNs, cover, rating/count when present | Essentially title, first author, Work identifier and cover | Nearly all descriptive fields are discarded during discovery; the cover address is lost during Add. Search does not supply a reliable language here. |
| Goodreads | Bare and decorated titles, author, Book and Work identifiers, cover, rating/count, pages and a possibly shortened description; series is parsed from title decoration | Decorated title, author, Book identifier/link, cover, parsed series and rating | Bare title and Work identifier are parsed but omitted from the result. Description/pages/count are unmodeled. Series/rating reach results but are not sent through Add. No language/year is supplied by this search parser. |

Title, primary author and valid identity routes are saved through the current
creation path. The failures above concern the other selected information. An image
visible in search is not proof that its URL was saved on the new Work.

## Proposed field mapping

These are recommendations for the implementation, not adopted overwrite policy.
The input column names fields in the provider response; it does not imply the
current parser or Add request already carries them. Google rating/count and
Goodreads description/pages/count require extending the parsers. Goodreads Work
ID is already parsed, then discarded by discovery before the result reaches Add;
its Book ID follows the existing separate route. Those are different loss points.

| Field | Provider input | Recommended handling |
|---|---|---|
| Title and subtitle | Google `title`/`subtitle`; OpenLibrary `title`; Hardcover `title`; Goodreads `bookTitleBare` and decorated `title` | Keep the title separately from series decoration. Preserve a trustworthy supplied subtitle. Retain the existing exclusion of Hardcover’s unreliable subtitle from automatic catalogue fields. |
| Authors | Google `authors`; OpenLibrary `author_name`/`author_key`; Hardcover `author_names` and contributions; Goodreads `author` | Preserve supplied names and identifiers in the selected result. Keep existing identity/author-linking rules in control; do not automatically treat every contributor as a primary author. |
| Language | Google `language`; no trustworthy mapped language from the other three search paths | Save the provider’s actual language. Remove the OpenLibrary search-language stamp and the default-language substitution as claims about the book. Unknown stays unknown; a default may still guide searches. |
| Publication details | Google `publishedDate` and `publisher`; OpenLibrary `first_publish_year`; Hardcover `release_date`/`release_year` | Preserve date precision, provider and whether the date is edition-specific or first publication. Decide explicitly how these feed the existing single Work year/date fields; do not silently equate their meanings. |
| Description | Google `description`; Hardcover `description`; Goodreads `description.html` plus `truncated` | Carry readable, safely normalized text and its source. Preserve the shortened/full distinction for Goodreads. Do not treat an excerpt as a complete description. |
| Series | Hardcover `featured_series.series.name`/`position`; Goodreads title decoration | Carry name and position together. A list of all series names is not a substitute for one identified series/position pair. |
| Genres/categories | Google `categories`; Hardcover `genres` | Preserve the provider’s category labels and source through the shared representation. |
| Pages | Google `pageCount`; Hardcover `pages`; Goodreads `numPages` | Preserve as provider-reported publication information, without claiming it describes the exact file the user owns. |
| Ratings | Google `averageRating`/`ratingsCount`; Hardcover `rating`/`ratings_count`; Goodreads `avgRating`/`ratingsCount` | Parse numbers consistently and keep rating, count and provider together. Missing/unrated should not become a misleading score of zero. |
| Identifiers | Google volume id and typed ISBNs; OpenLibrary Work/author keys and ISBNs; Hardcover Work id and ISBNs; Goodreads Book id and Work id | Retain their namespace and Work/edition distinction. Send supported identifiers through existing identity settlement; preserve other provider references without pretending they are equivalent IDs. Google’s existing ISBN-10-to-13 conversion can be reused. |
| Cover | Google image links; OpenLibrary cover id; Hardcover image URL; Goodreads image URL | Save the external source address, provider and any explicit user-choice intent durably. Download the image afterward. A failed download must not lose its address or falsely mark the image as saved. |
| Source and unknown values | Provider label and actual field presence | Preserve field origin. The frontend currently sends a provider label that the Add request ignores. Do not mark copied provider values as personal edits, and do not manufacture values for missing fields. |

## Concrete examples

The first Dune Google Books result reported `2005-08-02`; Hardcover reported
`1965-06-01`. That does not establish a contradiction: a particular edition’s date
and a Work-level release date answer different questions. The same samples
reported 548, 896 and 658 pages across Google Books, Hardcover and Goodreads.
Those numbers should retain their source and publication context.

The Hardcover Dune result also supplied the subtitle “Teacher’s Book”. Existing
Hardcover enrichment explicitly omits subtitles because of known data quality.
The proposed mapping should retain that protection rather than treating every
nonempty field as a trustworthy catalogue value.

The Goodreads Dune description was marked shortened. Its title was decorated with
series information, while `bookTitleBare` separately supplied the plain title.
Those distinctions are available before Add and should survive normalization.

## Suggested scope

1. Establish one selected-result representation carrying the supported values,
   their source, date meaning and any incomplete-description marker.
2. Map each provider’s already-fetched response into it. Add should not need another
   lookup just to recover data already received.
3. Extend the browser/server hand-off and initial save to retain those values when
   a new Work is created. Keep cover intent separate from saved-image state.
4. Keep duplicate-Work behavior, identity matching, additional-author adoption,
   cover replacement policy and enrichment overwrite policy as separate concerns.

A Work/Edition schema redesign is not proposed here. Facts that cannot honestly
fit an existing field need preservation with their provider context rather than
an invented mapping. The publication-date meaning is the main decision to settle
before implementation.

## Evidence and limits

Code inspected on branch `fix/add-metadata-preservation`, based on committed
runtime source `42b3027e` plus the unchanged test-fixture commit `1b52de87`.
All 917 deployed-source files match the baseline. Of the thirteen files in the
prior detailed mapping, only the already-deployed result-cap constant/comment
changed. No fresh provider requests, application writes or deployments were needed
for this mapping pass.

- [Detailed original field flow](../../build/reviews/add-search-research-2026-09-18/field-map/FIELD-MAP.md)
  and [source hashes](../../build/reviews/add-search-research-2026-09-18/field-map/mapping.json).
- [Observed field presence](../../build/reviews/add-search-research-2026-09-18/LIVE-FIELD-PRESENCE.json):
  60 Google Books results, 32 Hardcover results and 10 Goodreads results across
  the three samples. These are pre-cap response samples, not current visible counts.
- [Commit and branch preparation](../../build/reviews/metadata-save-preparation-2026-09-18/RESULT.md).
- Source anchors: `livrarr-external-data/src/google_books.rs` (`GbVolumeInfo`,
  `extract_isbn13`); `hardcover.rs` (`query_hardcover`);
  `goodreads/parsers.rs` (`AutocompleteEntry`, `parse_autocomplete_json_checked`);
  `livrarr-metadata/src/discovery_service.rs` (four lookup mappings);
  `livrarr-handlers/src/types/work.rs` (`AddWorkRequest`, `WorkSearchResult`).

This is source-and-response mapping research. It does not claim that the proposed
normalization or persistence fix has been implemented or tested.
