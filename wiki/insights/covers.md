# Covers lessons

## Executive summary

Cover selection can keep a small image even when other providers offer URLs: the
save gate compares the selected candidate with the saved image, not every provider.
See [selection and saving](#lesson-63) and [Open Library identifiers](#lesson-44).
Read implementation claims against the named source revision; accepted design is
not proof of runtime behavior.

<a id="44-ol-covers"></a>
<a id="lesson-44"></a>
## 44. OpenLibrary cover identifiers

The recorded OL policy distinguishes identifier lookups (ISBN/OCLC/LCCN: 100 per IP per five minutes) from Cover ID/OLID fetches. Resolve a cover identifier once, preserve the stable Cover ID and cache the bytes. These are dated external-policy observations; verify policy before changing traffic. See [OpenLibrary](../integrations/openlibrary.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/covers.md#44-ol-covers).

<a id="63-covers-have-one-rank-one-save-gate-one-layout-n2"></a>
<a id="lesson-63"></a>
## 63. Cover ownership and recovery

Use one per-slot rank/comparison policy and one recoverable write protocol. The documented automatic comparator uses measured pixels: a 400×600 floor, good over bad, rank among good candidates, and area among undersized candidates. User choices bypass automatic ranking.

Candidate selection and image comparison are separate. At deployed revision `98d35a37`, selection offers the first eligible nonempty URL in provider order. The save gate compares that image with the current file; an existing file with the same URL returns `AlreadyCurrent`. The size floor is therefore not a minimum accepted size and does not trigger a search through other providers. A live Add on 2026-09-19 saved a 128×205 Hardcover ebook image and retained it on the next pass; the separate audiobook image was 2400×2400. Another provider supplied a URL, but its image was not fetched or measured during the review. See the [read-only review](../../build/reviews/catalog-work-lookup/live-add-107-20260919/REVIEW.md#cover-quality).

Serialize by user, Work and slot. The protocol writes candidate bytes and a metadata sidecar, commits database metadata, renames atomically, then cleans the sidecar. Recovery must preserve candidates across transient database errors; only a proved missing Work permits discard. A user-owned slot remains protected while final bytes or the pending recoverable commit exist. A user flag with neither is damaged, not permanently locked.

Generic field merge must not make cover columns describe a file never saved. Persist measured dimensions for both slots and carry accepted bytes into any authorized retag. Layout is covers/{user_id}/{work_id}.jpg and _audio.jpg; orphan legacy root files are retained/logged and never served across users. Startup layout adoption, recovery and provenance repair are sequenced.

Goodreads cover candidates remain excluded under the documented containment policy, even though historical rank tables list them. Wrong covers were caused by Work/Book ID confusion, not the earlier claimed parser drift. Re-enabling requires the separate provider work; see [Goodreads](../integrations/goodreads.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/covers.md#63-covers-have-one-rank-one-save-gate-one-layout-n2).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/covers.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="cover-handling-insights"></a>
