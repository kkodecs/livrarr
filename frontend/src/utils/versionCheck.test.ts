import { describe, it, expect } from "vitest";
import { parseLatestRelease, isUpdateAvailable } from "./versionCheck";

describe("parseLatestRelease", () => {
  it("picks the newest published release and strips the leading v", () => {
    const data = [
      {
        tag_name: "v0.1.0-alpha6",
        html_url:
          "https://github.com/kkodecs/livrarr/releases/tag/v0.1.0-alpha6",
      },
      {
        tag_name: "v0.1.0-alpha5",
        html_url:
          "https://github.com/kkodecs/livrarr/releases/tag/v0.1.0-alpha5",
      },
    ];
    expect(parseLatestRelease(data)).toEqual({
      version: "0.1.0-alpha6",
      url: "https://github.com/kkodecs/livrarr/releases/tag/v0.1.0-alpha6",
    });
  });

  it("skips a leading non-version release (e.g. the toolchain tag)", () => {
    const data = [
      { tag_name: "toolchain", html_url: "https://example/toolchain" },
      { tag_name: "v0.1.0-alpha6", html_url: "https://example/alpha6" },
    ];
    expect(parseLatestRelease(data)?.version).toBe("0.1.0-alpha6");
  });

  it("accepts a tag with no leading v", () => {
    expect(parseLatestRelease([{ tag_name: "0.1.0-alpha6" }])?.version).toBe(
      "0.1.0-alpha6",
    );
  });

  it("returns a null url when html_url is absent", () => {
    expect(parseLatestRelease([{ tag_name: "v1.0.0" }])).toEqual({
      version: "1.0.0",
      url: null,
    });
  });

  it("returns null for an empty release list", () => {
    expect(parseLatestRelease([])).toBeNull();
  });

  it("returns null when no release has a version-like tag", () => {
    expect(
      parseLatestRelease([{ tag_name: "toolchain" }, { tag_name: "nightly" }]),
    ).toBeNull();
  });

  it("returns null for non-array input (rate-limit object / failed fetch)", () => {
    expect(parseLatestRelease(null)).toBeNull();
    expect(parseLatestRelease(undefined)).toBeNull();
    expect(parseLatestRelease({ message: "API rate limit exceeded" })).toBeNull();
  });
});

describe("isUpdateAvailable", () => {
  it("flags an update when an alpha5 build sees alpha6", () => {
    expect(isUpdateAvailable("0.1.0-alpha5", "0.1.0-alpha6")).toBe(true);
  });

  it("reports no update when the versions match", () => {
    expect(isUpdateAvailable("0.1.0-alpha6", "0.1.0-alpha6")).toBe(false);
  });

  it("reports no update before the latest release has loaded", () => {
    expect(isUpdateAvailable("0.1.0-alpha5", null)).toBe(false);
  });

  it("reports no update when the current version is unknown", () => {
    expect(isUpdateAvailable(null, "0.1.0-alpha6")).toBe(false);
  });
});
