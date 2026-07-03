# How other apps solve "same book/album/movie?" — precedent research (2026-07-02)

Phase-5 input. Three research agents opened the actual sources (GitHub files, official
docs) and quoted them; the orchestrator's synthesis is first, the three full reports
follow verbatim. Companion: `docs/matching-inventory-2026-07-02.md` (our own code).

## Synthesis (orchestrator)

**Bottom line: the industry solved this with two proven models; nobody has six
accidental variants, and nobody cuts titles at colons.**

**Model A — clean hard, then exact equality** (Radarr/Sonarr, Calibre, Kavita):
strip articles + punctuation, lowercase, de-accent → strings must match exactly.
Ambiguity goes to a human (Calibre duplicates dialog, Kavita "Unmatched" + user remap
rules, Sonarr's hand-curated alias table).

**Model B — weighted distance with confidence bands** (beets → Lidarr → Readarr):
one scoring formula; every field contributes penalty × weight; typo-tolerant
Levenshtein on aggressively cleaned strings. Readarr book weights: ISBN/ASIN 10,
author 3, title 3, year 1; import accept ≈0.80 effective. beets adds four confidence
bands: ~0.96+ auto-accept / middle band pre-selects + one-key human confirm / low /
none → full candidate list.

**Universal agreements:**
1. IDs first, always — hard override (Radarr/Sonarr, ABS-ASIN) or
   heaviest-weight-but-still-scored (beets/Lidarr/Readarr import). Ours is the second
   flavor already.
2. A FEW NAMED purpose-built cleaners (2-3), never one-per-callsite. Readarr: one
   fuzzy import scorer + one exact-equality search path = the "two brains" shape.
3. Nobody truncates at the colon. Subtitle stripping is opt-in per comparison
   (Calibre), conditional (ABS keeps subtitles when the query has one), or handled by
   forgiveness-weights (beets patterns: parentheticals/"(EP)"/feat. clauses cost
   little instead of being cut).
4. Auto-accept bars are STRICTER than our 0.75-on-sloppy-cleaning: beets ~0.96;
   Readarr ~0.80 multi-field incl. ID weights; Radarr/Sonarr exact-only.
5. Middle band goes to a human; nobody LLM-matches. Escape hatches = IDs, curated
   alias tables, fingerprints (ABS: audio DURATION weighted 0.7 vs title 0.2), human
   confirmation. (Hardcover-style LLM tie-break: no precedent, harmless as last tier.)
6. Identity matching vs release-quality ranking are separate systems everywhere
   (ours too).

**Mapping to our four design questions:** (1) two sanctioned tiers — identity-grade
+ typo-tolerant — exactly Readarr; our canonical cleaner and m4 scorer are already
those two; (2) per-purpose bars on one shared model underneath; identity auto-bar
should be stricter with an ask/park band; (3) keep subtitles; one-side-has-subtitle
rule + forgiveness scoring for variants; (4) deterministic matching everywhere; GR
unlock without LLM is what everyone else would do.

**Chaptarr:** real but closed-source (Docker image active, 353k pulls, repo private;
community alleges GPLv3 breach of Readarr). Nothing to learn from it.

---

## Report 1 — Servarr family (Readarr, Chaptarr, Radarr/Sonarr)

Everything below was fetched and read directly this session (raw GitHub source files
via raw.githubusercontent.com, or the GitHub API for directory listings) unless
explicitly marked "relayed / not independently opened."

### READARR — book/edition identification (github.com/Readarr/Readarr, develop)

Pipeline (import path): (1) TrackGroupingService.GroupTracks clusters loose files
into candidate releases; (2) CandidateService.GetDbCandidatesFromTags looks for
local-DB candidates first; (3) if none, GetRemoteCandidates queries Goodreads —
ISBN first, then ASIN, then Goodreads ID; if any ID search returns a hit it stops
and never falls back to fuzzy text search; else author+title, title-only,
author-only; (4) IdentificationService.GetBestRelease scores EVERY candidate through
DistanceCalculator.BookDistance, keeping lowest distance; early exit only on 0.0;
(5) if best > 0.15 and remote untried, retry with remote candidates;
(6) CloseBookMatchSpecification is the final accept gate, threshold 0.20.

Weights (Distance.cs:11-39, "// from beets default config"): isbn 10.0,
isbn_missing 0.1, asin 10.0, asin_missing 0.1, recording_id 10.0, author 3.0,
book 3.0, wrong_format 5.0, language 5.0, book_id 5.0, source 2.0, year 1.0,
publisher 0.5, catalog_number 0.5, country 0.5.

Metric (Distance.cs:123-150 + StringExtensions.cs:211-214): Clean() = lowercase +
RemoveAccent + keep-letters/digits-only; then normalized Levenshtein
(1 - dist/maxlen). Thresholds: import accept 0.20 (CloseBookMatchSpecification —
file still named CloseAlbumMatchSpecification.cs, a Lidarr-fork leftover);
escalate-to-remote 0.15 (IdentificationService.cs:171).

ID short-circuit nuance: IDs short-circuit candidate DISCOVERY
(CandidateService.cs:274-281 "If we got an id result... stop") but ID-derived
candidates are NOT auto-accepted — still scored; a mismatched ISBN drives distance
up. Different from Radarr/Sonarr's absolute ID bypass.

Three different clean functions, not interchangeable: Distance.Clean() (alnum-only,
fuzzy scorer); Parser.CleanAuthorName() (strips a/an/the/and/or/of + non-word,
lowercase, de-accent — ALSO reused for exact book-title equality in
ParsingService.GetBooks:123); Parser.CleanBookTitle() (strips only edition cruft
like "(Deluxe Edition)"/"(Unabridged)", KEEPS articles/punctuation).

Release-title parsing (separate mechanism): ~20 hand-written regexes
(ReportBookTitleRegex, Parser.cs:41-133); Bitap approximate-substring fallback
(FuzzyContains.cs, ported from google diff-match-patch); token-based FuzzyMatch
last resort (StringExtensions.cs:167-209). Parser.cs:715 admits: "// Coppied from
Radarr" (sic).

Verdict: several matchers BY PURPOSE sharing one core module — one Distance scorer
for import identification (three consumption points, multiple thresholds), a
separate exact-clean-equality mechanism for search/grab matching, and release
RANKING (DownloadDecisionComparer: quality/format/priority) fully separate from
identity.

Sources: github.com/Readarr/Readarr blob develop —
src/NzbDrone.Core/MediaFiles/BookImport/Identification/{DistanceCalculator,Distance,
CandidateService,IdentificationService}.cs ·
src/NzbDrone.Core/MediaFiles/BookImport/Specifications/CloseAlbumMatchSpecification.cs ·
src/NzbDrone.Core/DecisionEngine/DownloadDecisionComparer.cs ·
src/NzbDrone.Core/Parser/{Parser,ParsingService}.cs ·
src/NzbDrone.Common/Extensions/{StringExtensions,FuzzyContains,BerghelRoach}.cs

### CHAPTARR — verified real but closed-source

api.github.com/repos/robertlordhood/Chaptarr → HTTP 404 (both casings). Docker Hub
API confirms the image: public, 353,766 pulls, updated 2026-06-30, "source":null;
description: "a fork of the now retired Readarr that handles audiobooks and ebooks
in one instance." linuxserver discussion #99: repo "currently set to private,"
commenters allege GPLv3 breach. Cannot analyze matching logic — nothing public.
(GPL-breach claims = community assertion, not verified fact.)

### RADARR / SONARR — deterministic equality, not distance

Normalization (Radarr Parser.cs:109-110, 430-444; Sonarr Parser.cs:840-857
CleanSeriesTitle — identical body): NormalizeRegex strips articles
(a/an/the/and/or/of) + all non-word chars; German umlaut folding; lowercase;
RemoveAccent. Matching: **zero hits for Levenshtein/FuzzyMatch in either app's
Parser/ParsingService/MovieService/SeriesService** — lookup is exact Contains() on
the cleaned-title column, year as secondary filter
(Radarr MovieService.cs:126-130), with Roman/Arabic numeral interchange the only
tolerance (:159-186). Sonarr's FindByTitleInexact (SeriesService.cs:108-141) is
substring containment (leftmost, longest), not edit distance. Sonarr adds a
human-curated Scene Mapping alias table + TheXEM for anime.

ID short-circuit is a HARD override: TryGetMovieByImDbId/TmdbId return on ID + year
sanity only, no title comparison (ParsingService.cs:161-185); Sonarr logs a Sentry
warning when ID matched but parsed title mismatched (alias gap, not rejection).
Wiki confirms (Sonarr faq.md:396-398: "The text-based search must match exactly";
Radarr faq.md:661, 407-409: ID hit + name mismatch ⇒ UI asks the user to validate).

Sources: github.com/{Radarr/Radarr,Sonarr/Sonarr} blob develop —
src/NzbDrone.Core/Parser/{Parser,ParsingService}.cs ·
Radarr src/NzbDrone.Core/Movies/MovieService.cs ·
Sonarr src/NzbDrone.Core/Tv/SeriesService.cs ·
github.com/Servarr/Wiki blob master {radarr,sonarr}/faq.md

Unverified: Prowlarr matching docs (WebSearch summary only — apparently forwards
search params, no own matching); Sonarr's FindByTitleInexact SQL literal;
Lidarr-lineage of Readarr's Distance (music-vestige fields imply it; not opened in
this report — confirmed independently in Report 2).

## Report 2 — the music lineage (beets, Lidarr, Picard, MusicBrainz)

Branches: beets master, Lidarr develop, Picard master; fetched 2026-07-02.

### beets — the canonical precedent

Pipeline: majority-vote tags → ID-first (`match_by_id`; if the MBID candidate scores
strong, return immediately) → text search → per-candidate optimal track assignment
(Jonker-Volgenant LAP) → one Distance object over all fields → normalized
weighted-sum distance in [0,1] → `_recommendation()` bands → UI: strong auto-applies;
medium/low pre-selects + one-key confirm; none → full candidate list.

Normalization (beets/autotag/distance.py:31-119): `unidecode` → lowercase → strip
non-[a-z0-9] → Levenshtein/maxlen; then forgiveness patterns — leading "the " 90%
forgiven, "[({]?(ep|single)[)}]?" 100% free, "feat./ft. …" 90%, parentheticals/
brackets 70%, "pt./part …" 80%; "&"→"and"; ", The" comma-swap.

Weights (config_default.yaml:173-193): artist 3.0, album 3.0, album_id 5.0,
track_id 5.0, tracks 2.0, data_source 2.0, track_title 3.0, track_artist 2.0,
track_length 2.0, media/mediums/year/track_index 1.0, missing_tracks 0.9,
unmatched_tracks 0.6, country/label/catalognum/albumdisambig 0.5.

Thresholds (config_default.yaml:167-172): strong_rec_thresh 0.04 (auto),
medium_rec_thresh 0.25, rec_gap_thresh 0.25, max_rec caps (missing_tracks →
medium). ID = sourcing shortcut, not override: the MBID candidate still rides the
same distance formula (album_id just the heaviest weight). Optional hard vetoes
(match.required / match.ignored) ship EMPTY.

### Lidarr — beets model, unattended flavor

Distance.cs:10 "// from beets default config". Kept: distance math shape, field
set, helper names. Changed: missing/unmatched track weights swapped (0.6/0.9);
recording_id doubled to 10.0; string cleaning simplified (lowercase + accent-strip
+ alnum-only + plain LevenshteinCoefficient — beets' forgiveness regexes NOT
ported; comment: "musicbrainz never has 'featuring' in the track title" — trusts
source-side editorial style); no human band — instead ShouldFingerprint(): best
distance > 0.15 or worst track > 0.40 → AcoustID fingerprint escalation and rerun;
GetBestRelease = deterministic argmin. Honors merged-away old MBIDs
(OldForeignReleaseIds) — ID redirects still count as matches.

### Picard — tiered, identifiers decisive

astrcmp = normalized Levenshtein similarity; similarity2 = greedy token-sorted
per-word best-match (leftover words only 40%-penalized). Tiers
(matching.py:136-194): id_score ≥ 0.9 → 0.85 + sim×0.1 + pref×0.05 (cliff);
id_score ≤ 0.1 → capped at 0.3; else id×0.4 + sim×0.5 + pref×0.1. Cluster→release
weights (cluster.py:83-99): barcode 28, catno 22, album 17, releasetype 10,
albumartist 6, totalalbumtracks 5, date 4, format/country 2. User-facing config:
match_min_similarity 0.25, match_min_margin 0.02 (margin → flagged 'ambiguous'),
track_matching_threshold 0.4. File MBIDs tried first; conflicting recording/track
IDs disambiguated by duration score. Near-veto via extreme weight (release-type
preference 0 → (0,9999) "only picked if there are no others at all").

### MusicBrainz server-side

No formula — human editorial merge queue; "merge rather than delete" because old
MBIDs forward (exactly what Lidarr/Picard code tolerates). Client-side fuzzy
matchers exist because the canonical human-curated DB can't be asked at import time.

**The beets model in five bullets:** (1) one weighted multi-field distance formula
for the whole app; (2) transliterate→lowercase→alnum-only→Levenshtein, with
forgiveness-weights instead of hard strips; (3) FOUR confidence bands with a
human-confirm middle; (4) IDs shortcut sourcing, never scoring; (5) penalties, not
vetoes — hard-exclusion config exists but ships empty.

Sources: github.com/beetbox/beets blob master beets/autotag/{match,distance}.py ·
beets/config_default.yaml · beets/ui/commands/import_/session.py ·
github.com/Lidarr/Lidarr blob develop src/NzbDrone.Core/MediaFiles/TrackImport/
Identification/{Distance,DistanceCalculator,IdentificationService,CandidateService}.cs ·
src/NzbDrone.Common/Extensions/StringExtensions.cs ·
github.com/metabrainz/picard blob master picard/{similarity,matching,cluster,album,
options}.py picard/util/astrcmp.py picard/ui/options/matching.py ·
musicbrainz.org/doc/{How_to_Merge_Releases,Merge_Rather_Than_Delete}
Flagged unverified: Lidarr manual-review UI existence; Picard matching.py rewrite age.

## Report 3 — book apps (Calibre, Audiobookshelf, Kavita, OpenLibrary)

### Calibre — rank, don't threshold; three purpose-built cleaners

get_title_tokens (sources/base.py; builds outbound provider queries): optional
strip_subtitle param (brackets OR anything after /:\\), strips edition/format noise
(omnibus/hardcover/audiobook/audio cd/paperback/mass market/edition), splits,
drops joiners (a/and/the/&). get_author_tokens: "Last, First"→"First Last", drops
≤2-char tokens + von/van. cleanup_title (rank bonus): lowercase, strip leading
the/a/an/of/and, strip trailing parenthetical → EXACT equality. fuzzy_title
(db/utils.py; library dedup): despite the name = aggressive normalization (strip
brackets/quotes/punct, language-aware leading-article pattern, -._→space, collapse)
→ EXACT equality. **No edit distance anywhere in core Calibre.**

Download ranking (InternalMetadataCompareKeyGen): sort-key tuple — same_identifier
FIRST, then has_cover, all_fields, language, exact_title, comments-length (>10%),
source relevance LAST. Cross-source merge (ISBNMerge): pool by shared ISBN first,
fall back to lowercase title+authors equality. ISBN privileged three separate ways
(per-plugin discard of ISBN-less results, first sort-key field, pool grouping).
Library dedup (find_identical_books): authors-first exact intersection (ALL
incoming authors must match), then fuzzy_title equality, then language filter.
Ambiguous adds → DuplicatesQuestion dialog (human).

### Audiobookshelf — Levenshtein + DURATION-dominant

cleanTitleForCompares: conditional subtitle keep (keepSubtitle decided by whether
the QUERY has one), strip parentheticals, strip apostrophes, de-accent, lowercase.
calculateMatchConfidence: isTitleAsin → instant 1.0; else weighted blend
W_DURATION 0.7 / W_TITLE 0.2 / W_AUTHOR 0.1 — duration diff ≤1min → 1.0, no
duration → floor 0.1 (a perfect title+author match without duration caps at 0.37).
Levenshtein similarity on cleaned strings; author = max across comma-split parts.
Config: maxTitleDistance 4, maxAuthorDistance 4, maxFuzzySearches 5 (progressive
stripped-variant retry cascade). Audio runtime is a physical fingerprint — the
text-only apps have no analog.

### Kavita — tiered exact ladder (docs-verified; source 404'd)

CBL import: Tier 0 user-defined remap rules (override everything) → Tier 2 exact
name (lowercase, strip punct) → Tier 4 article-stripped retry → Tier 5
reprint/edition-stripped retry ("Deluxe Edition"/"Omnibus"/"TPB") → Tier 6
alternate-name fields → else "Unmatched." Equality-after-normalization at every
tier; no edit distance.

### OpenLibrary — humans, not code

FRBR-style Work definition; dedup = librarian merge queue (openlibrary.org/merges)
+ style guidelines; no public algorithmic spec found. The "same book?" question is
answered editorially.

**Book-app patterns:** rank-don't-threshold (Calibre) or discrete tiers (Kavita) vs
ABS's true score; IDs always short-circuit, in multiple mechanisms at once;
multiple NAMED normalizers per app, purpose-built; text-only apps use
equality-after-normalization, not edit distance — ABS reaches for Levenshtein and
has duration to lean on; subtitle stripping explicit and conditional everywhere;
human-in-the-loop fallback everywhere except ABS.

Sources: github.com/kovidgoyal/calibre blob master src/calibre/ebooks/metadata/
sources/{base,identify}.py · src/calibre/ebooks/metadata/__init__.py ·
src/calibre/db/utils.py · src/calibre/gui2/add.py ·
github.com/advplyr/audiobookshelf blob master server/finders/BookFinder.js ·
wiki.kavitareader.com/guides/features/cbl-import/ ·
openlibrary.org/{about/lib,librarians}
Flagged unverified: Kavita Parser.cs source (404 ×2 — docs only); ABS fuzzy-cascade
consuming-loop verbatim (constants confirmed, loop paraphrased); calibre third-party
"Find Duplicates" plugin (not core, not opened); ABS manual-override UI existence.
