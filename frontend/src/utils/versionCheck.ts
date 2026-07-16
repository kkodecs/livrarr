// Pure helpers for the sidebar "update available" notifier.
//
// The notifier fetches the GitHub Releases list for the repo and shows a banner
// when the newest published release differs from the running build's version.
// The parsing + comparison live here (not inline in the hook) so they are unit
// testable without a browser: the hook in `Sidebar.tsx` calls these directly.

export interface GitHubRelease {
  tag_name?: string;
  html_url?: string;
}

export interface LatestRelease {
  /** Release tag with any leading "v" stripped, e.g. "0.1.0-alpha6". */
  version: string;
  /** The release's GitHub page, or null when the response omits it. */
  url: string | null;
}

/**
 * Pick the newest version-like release from a GitHub `/releases` response.
 * GitHub returns releases newest-first, so we take the first entry whose
 * `tag_name` looks like a version (`^v?\d`) — this skips non-version releases
 * such as the "toolchain" tag. Returns null for any malformed/empty input.
 */
export function parseLatestRelease(data: unknown): LatestRelease | null {
  if (!Array.isArray(data)) return null;
  const latest = (data as GitHubRelease[]).find(
    (r) => typeof r?.tag_name === "string" && /^v?\d/.test(r.tag_name),
  );
  if (!latest?.tag_name) return null;
  return {
    version: latest.tag_name.replace(/^v/, ""),
    url: typeof latest.html_url === "string" ? latest.html_url : null,
  };
}

/**
 * True when a different published version is available. This is a plain string
 * inequality (not a semver comparison) — it preserves the notifier's existing
 * behavior: any newest-release tag that differs from the running version counts
 * as an update.
 */
export function isUpdateAvailable(
  currentVersion: string | null | undefined,
  latestVersion: string | null | undefined,
): boolean {
  return Boolean(
    currentVersion && latestVersion && latestVersion !== currentVersion,
  );
}
