import type { Step } from "react-joyride";

export const TOUR_STEPS: Step[] = [
  // ── Metadata: Welcome ──
  {
    target: "body",
    content:
      "Welcome to Livrarr! (Livre is the French word for book.) Let's walk through the key settings to get you up and running. You can exit this tour at any time. Click Next to continue.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/" },
  },
  {
    target: "[data-tour='hardcover-section']",
    content:
      "Hardcover is a free metadata source for books. Go to https://hardcover.app, create an account, and obtain an API token at Settings → API. Enter the API Token (do not delete \"Bearer\"), click Save Changes, and click Next.",
    placement: "right",
    skipBeacon: true,
    data: { route: "/settings/metadata" },
  },
  {
    target: "[data-tour='llm-section']",
    content:
      "Optional and Recommended: Add an LLM to assist with search results and metadata. Livrarr only sends publicly available information to LLMs; file names, system information, and personal data are not sent. Google Gemini offers a free tier that is recommended and has been tested. Please follow the instructions to obtain an API key, click Save Changes, and click Next to continue.",
    placement: "right",
    skipBeacon: true,
    data: { route: "/settings/metadata" },
  },

  {
    target: "[data-tour='languages-section']",
    content:
      "Livrarr has experimental support for some Foreign Languages. This functionality requires LLM configuration. You may select Foreign Languages here, then click Next to continue.",
    placement: "bottom" as const,
    skipBeacon: true,
    data: { route: "/settings/metadata" },
  },

  // ── Indexers ──
  {
    target: "body",
    content:
      "Next up: indexers. These are the search engines Livrarr uses to find ebook and audiobook releases. Click Next to continue.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/settings/indexers" },
  },
  {
    target: "[data-tour='prowlarr-import']",
    content:
      "If you use Prowlarr, you can import all your indexers in one click. Just enter your Prowlarr URL and API key. If you don't use Prowlarr, skip this and add indexers manually below.",
    placement: "bottom",
    skipBeacon: true,
    data: { route: "/settings/indexers" },
  },
  {
    target: "[data-tour='add-indexer-form']",
    content:
      "Or add indexers manually — enter a Torznab URL (e.g. MyAnonamouse, TorrentLeech) or Newznab URL (e.g. DrunkenSlug, NZBGeek) with its API key.",
    placement: "top",
    skipBeacon: true,
    data: { route: "/settings/indexers" },
  },

  // ── Download Clients ──
  {
    target: "body",
    content:
      "Now let's set up a download client so Livrarr can actually grab releases. Click Next to continue.",
    placement: "center",
    skipBeacon: true,
    data: { route: "/settings/downloadclients" },
  },
  {
    target: "[data-tour='prowlarr-import-dc']",
    content:
      "If you imported indexers from Prowlarr, you can also import your download clients here — your Prowlarr credentials are already saved. Only qBittorrent and SABnzbd are supported.",
    placement: "bottom",
    skipBeacon: true,
    data: { route: "/settings/downloadclients" },
  },
  {
    target: "[data-tour='add-client-form']",
    content:
      "Or add download clients manually — qBittorrent for torrents or SABnzbd for Usenet. You'll need the host, port, and credentials.",
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
    target: "[data-tour='email-kindle-section']",
    content:
      "Send books directly to your Kindle or eReader via email. Pick a provider preset (Gmail, Outlook) or enter custom SMTP settings. You'll need your Kindle email address (find it at amazon.com/myk → Devices) and must add the From address to your Approved Personal Document Email List (amazon.com/myk → Preferences → Personal Document Settings). Enable 'Send on import' to automatically deliver new books.",
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
