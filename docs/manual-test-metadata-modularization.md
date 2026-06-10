# Manual Test Script — metadata-modularization + WCC

**Branch:** `wcc-stage5-green` @ `2cf112d` (merged, **not pushed**)
**Goal:** Validate the merged work end-to-end before pushing. Work top to bottom; tick each box.

## How to read this
- **Force** = you can stage this deterministically.
- **Observe** = an edge state that may not reproduce on demand — verify *if* you hit it; don't burn time forcing it.
- Each item has **Do → Expect (pass) → Red flag (fail)**. A red flag = stop and note it.

## What changed (what you're actually testing)
1. The pipeline was split into crates (`external-data` / `identity` / `enrichment`) — **behavior-preserving**, so §1 is "nothing regressed."
2. **Two separate status badges** replaced the old single status: an **Identity** badge ("which book is this?") and a **Details** badge ("what do we know?").
3. **Add-by-search** now fans out across 4 sources at once and trusts your pick (instant, no re-lookup).
4. **#97 Manual Import auto-match** — per-file suggested matches, lands books Confirmed + with a cover.

---

## 0. Setup
- [ ] Server is running the **merged** build (ask Claude to run `scripts/dev-restart.sh`, or run it yourself).
- [ ] App loads; you're logged in; existing library is visible.
- [ ] Open the browser devtools **Console** — keep it visible. Any red error during testing = a red flag worth noting.
- [ ] **Test-data prep:** have a folder with **3–5 book files not yet in the library** ready for §4 — mix of `.epub` and (if possible) `.m4b`, ideally a couple from authors you already have, and at least one whose filename has a clear `Author - Title`.

---

## 1. Smoke / regression (the crate split must not have broken anything)
- [ ] Open 2–3 **existing** works → detail page renders (cover, author, files, all 4 tabs).
- [ ] Open an existing ebook in the **reader**; open an existing audiobook in the **player** → both load and play.
- [ ] On a work, **Search** tab → "Search Releases" → results (or a clean "no releases") with no error.
- [ ] Authors page + a Series page load.
- **Red flag:** any page that previously worked now errors, or a console exception on load.

---

## 2. Add a book via Search — discovery fan-out + trust-the-pick
*(Delivers: 4-way search, Goodreads cards, instant zero-relookup add)*

- [ ] **Force:** Search a well-known English book (e.g. *Project Hail Mary* — Andy Weir, or *The Name of the Wind* — Patrick Rothfuss).
- [ ] Under **"Add to Your Library"**, results show covers + author.
- [ ] **Multi-source proof:** results carry **source tags** and you see **more than one distinct source** across the list (OpenLibrary, Hardcover, Goodreads, Google Books). At least one result shows a **rating (★)** — that's the Goodreads card.
  - **Red flag:** only ever one source tag → the fan-out isn't running (it regressed to first-hit).
- [ ] A **"Filtered N / Raw N"** toggle appears; clicking **Raw** shows more results.
- [ ] Click **Select** → cover picker opens → pick a cover (or Skip) → **Add to Library**.
- [ ] The add is **fast** (no long spinner) and lands you on the new work's detail page with a success toast.
- [ ] Open **Book Information** tab → **Identity = Confirmed** (green). *This is "trust-the-pick" working — your selection became the identity without a re-lookup.*
  - **Red flag:** a clean pick of a popular book lands **Pending** or **Provisional**.
- [ ] Watch the **Details** badge: it may start **Pending** (amber) and flip to **Enriched** (green) within a few seconds (the page auto-polls every ~3s). Cover may fill in slightly after — that's expected.

---

## 3. Book Information tab — Identity + Details badges
*(Delivers: the two-state split + the new "Unverified" state)*

Open Book Information on any enriched work. Shortcut URL: `/work/<id>?tab=metadata`.

- [ ] There are **two stacked sections**, each with its **own** badge under its header:
  - **Identity** ("Which book this is") → IDs: Open Library / Hardcover / Goodreads / ISBN-13 / ASIN.
  - **Details** ("What we know about it") → Year / Series / Genres / Publisher / Language / etc.
- [ ] Hovering a badge's **(?)** shows a tooltip explaining that state.
- [ ] **Confirmed / Enriched** on the popular book from §2.

Now provoke the other states (some are Observe-only):
- [ ] **Force — Details "Pending"→"Enriched":** add a fresh book and watch the Details badge flip from amber **Pending** to green **Enriched** on the auto-poll.
- [ ] **Force — Identity "Pending":** search a **vague/partial** title with no strong match (e.g. a generic phrase) and add a weak result → Identity should sit at amber **Pending** (fuzzy guess, no key) and **not** aggressively enrich.
- [ ] **Force — "Provisional" (blue):** search a **bare ISBN-13** of an edition OpenLibrary/Hardcover likely lack (a self-published or non-English edition Google Books has). If it adds with only an ISBN and no master record → Identity = blue **Provisional**, and it **still enriches**.
- [ ] **Observe — "Sparse" (zinc, Details):** an obscure book the catalog knows by title but has almost no info for → Details = **Sparse** (a *settled* "we found nothing," not "still loading").
- [ ] **Observe — "Conflict" (red, Identity):** sources disagree on the match → red **Conflict**, asking for your review.
- [ ] **Observe — "Unverified" (amber, Identity):** every provider was rejected (needs LLM enrichment on) → amber **Unverified**. Then click **Refresh** in the Identity section → it should re-attempt and recover (not stay stuck).
- **Red flag:** a single status conflating identity + details; a badge with no matching label/color; "Unverified" permanently stuck after Refresh.

---

## 4. Manual Import auto-match — #97 (the headline)
*(Delivers: per-file suggested matches, ISBN-beats-title, Confirmed + cover on import)*

- [ ] Open the **Manual Import** page → enter/Browse to your test folder → **Scan**.
- [ ] Files list with parsed **Author — Title** under each filename.
- [ ] A **"Matching against OpenLibrary…"** progress bar runs, then the **Match** column fills in:
  - A confident auto-match shows as a **blue link** (title — author).
  - A filename-only guess shows greyed with **"(parsed)"**.
- [ ] **Auto-match quality:** the suggested matches are the **right books** (author grouping works). A file with an embedded ISBN matches the **correct edition** even if the title is messy (ISBN-beats-title).
  - **Red flag:** matches to a *different author's* book, or obviously wrong editions.
- [ ] **Correct a match:** click a match → inline **"Search Open Library…"** dropdown → type → pick a different result → the row updates to your pick.
- [ ] A file whose book+media-type is already in the library shows a yellow **"duplicate"** badge (and a **Delete existing release(s)** checkbox when selected).
- [ ] Tick a few files → **Import Selected** → **"Importing and enriching…"** bar → rows end at **Imported** (green, links to the work) or a clear failure.
- [ ] Open an **imported** work → Book Information → it should be **Confirmed + Enriched with a cover** (the #97 win — not stuck ISBN-only/Pending).
  - **Red flag:** imported works land Pending/Provisional with no cover despite a confident match.
- [ ] Re-import the **same** file (or scan again) → it's recognized as already imported / duplicate (no silent double-add).

---

## 5. Foreign-language add (Google Books path)
*(Requires >1 language enabled in Settings → Metadata)*

- [ ] In Settings, confirm a non-English language is enabled (e.g. Korean / French / German).
- [ ] On Search, the **language selector** appears → pick the non-English language → search a native-language title.
- [ ] Results come back **in that language** (Google Books).
- [ ] Add one → it enriches (Details → Enriched) with native-language metadata.
- **Red flag:** foreign search returns English-only or empty; foreign add never enriches.

---

## 6. Refresh & recovery
- [ ] On any work, top **Refresh** button → success toast → metadata re-fetches; cover re-checks.
- [ ] Identity-section **Refresh** on a Pending/Unverified work → it re-attempts identity (recovers if a source now matches).

---

## Known-disabled — NOT bugs (don't flag these)
- **Audiobook file-tag writing is OFF** (alpha5, memory-OOM on large files). Audiobook **import + matching + metadata still work**; the audio file itself just won't get tags written. EPUB tag-writing is on.
- **`main` is not updated** and the branch is **not pushed** — by design, pending this test pass.
- The 4th planned crate (`materialize`) and the full Goodreads anti-bot ladder are **deferred** features, not part of this build.

---

## Result
- [ ] **All ✔, no red flags → cleared to push** `wcc-stage5-green`.
- [ ] Any red flag → note it under the failing item and stop; bring it back for a fix before push.
