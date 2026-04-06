import type { Step } from "react-joyride";

export const TOUR_STEPS: Step[] = [
  // ── Metadata: Welcome ──
  {
    target: "body",
    content:
      "Welcome to Livrarr! (Livre is the French word for book.) Let's walk through the key settings to get you up and running. You can exit this tour at any time.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/" },
  },
  {
    target: "[data-tour='hardcover-section']",
    content:
      "Hardcover is a free metadata source for books. Go to https://hardcover.app, create an account, and obtain an API token at Settings → API. (Do not delete \"Bearer\" from the token.)",
    placement: "right",
    skipBeacon: true,
    data: { route: "/settings/metadata" },
  },
  {
    target: "[data-tour='llm-section']",
    content:
      "Optional but recommended: connect an LLM to disambiguate search results and clean bibliographies. Both Groq and Google Gemini offer free tiers that work well — just pick a provider from the dropdown, grab a free API key, and paste it in. Livrarr only sends publicly available information (book titles and author names) — no file names, paths, or personal data. Model names change frequently; if a preset stops working, check the provider's docs.",
    placement: "right",
    skipBeacon: true,
    data: { route: "/settings/metadata" },
  },

  // ── Indexers ──
  {
    target: "body",
    content:
      "Next up: indexers. These are the search engines Livrarr uses to find ebook and audiobook releases.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/settings/indexers" },
  },
  {
    target: "[data-tour='add-indexer-form']",
    content:
      "Add a Torznab indexer (e.g. MyAnonamouse, TorrentLeech) or a Newznab indexer (e.g. DrunkenSlug, NZBGeek). You'll need the indexer URL and API key. Fill out the form below and click Add Indexer.",
    placement: "top",
    skipBeacon: true,
    data: { route: "/settings/indexers" },
  },

  // ── Download Clients ──
  {
    target: "body",
    content:
      "Now let's set up a download client so Livrarr can actually grab releases.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/settings/downloadclients" },
  },
  {
    target: "[data-tour='add-client-form']",
    content:
      "Add qBittorrent for torrents or SABnzbd for Usenet. You'll need the host, port, and credentials for your client. Fill out the form below and click Add Client.",
    placement: "top",
    skipBeacon: true,
    data: { route: "/settings/downloadclients" },
  },

  // ── Media Management ──
  {
    target: "body",
    content:
      "Finally, media management — where your books live and how they're organized.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/settings/mediamanagement" },
  },
  {
    target: "[data-tour='root-folders-section']",
    content:
      "Root folders are where Livrarr stores your library. Add at least one — e.g. /books or /media/ebooks.",
    placement: "top",
    skipBeacon: true,
    data: { route: "/settings/mediamanagement" },
  },
  {
    target: "[data-tour='remote-path-section']",
    content:
      "If your download client runs on a different machine (e.g. a seedbox), set up remote path mappings so Livrarr can find the downloaded files locally.",
    placement: "top",
    skipBeacon: true,
    data: { route: "/settings/mediamanagement" },
  },
  {
    target: "[data-tour='cwa-section']",
    content:
      "Calibre-Web Automated integration: if you use CWA, Livrarr can hardlink imported books into your CWA ingest folder automatically.",
    placement: "top",
    skipBeacon: true,
    data: { route: "/settings/mediamanagement" },
  },
  {
    target: "body",
    content:
      "You're all set! Configure each section at your own pace. You can always revisit these settings from the sidebar. Happy reading!",
    placement: "center",
    skipBeacon: true,
    data: { route: "/settings/mediamanagement" },
  },
];
