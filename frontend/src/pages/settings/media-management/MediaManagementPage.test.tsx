import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MediaManagementPage from "@/pages/settings/media-management/MediaManagementPage";
import { useAuthStore } from "@/stores/auth";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

const FILE_LOCATIONS =
  "When Livrarr imports a book it files each ebook as Author/Title.ext and keeps each audiobook's own file names inside an Author/Title folder, under a folder for your user number in the root folder for that type; Readarr Import instead puts every file, audiobooks included, in the root folder you choose there, as Author/Title.ext.";
const ROOT_FOLDERS_TIP =
  "Where your library files are stored. Add one folder for ebooks and one for audiobooks. Inside each, Livrarr files books by user number and author; File locations below gives the full layout.";

// Every call the page sends today is answered, including the naming row, so
// the page draws in full on today's code.
const replies: Record<string, StubReply> = {
  "/rootfolder": { status: 200, body: [] },
  "/remotepathmapping": { status: 200, body: [] },
  "/config/mediamanagement": {
    status: 200,
    body: {
      cwaIngestPath: null,
      preferredEbookFormats: ["epub"],
      preferredAudiobookFormats: ["m4b"],
    },
  },
  "/config/naming": {
    status: 200,
    body: {
      authorFolderFormat: "{Author Name}",
      bookFolderFormat: "{Book Title}",
      renameFiles: false,
      replaceIllegalChars: true,
    },
  },
  "/config/email": {
    status: 200,
    body: {
      enabled: false,
      smtpHost: "",
      smtpPort: 587,
      encryption: "starttls",
      username: null,
      passwordSet: false,
      fromAddress: null,
      recipientEmail: null,
      sendOnImport: false,
    },
  },
};

let cleanup: () => void = () => {};

beforeEach(() => {
  useAuthStore.setState({
    status: "authenticated",
    isAdmin: true,
    user: {
      id: 1,
      username: "admin",
      role: "admin",
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
    },
  });
});

afterEach(() => {
  cleanup();
  useAuthStore.setState({ status: "loading", isAdmin: false, user: null });
});

async function openPage() {
  const api = installApiStub((call: ApiCall) => {
    const reply = replies[call.path.split("?")[0]!];
    return (
      reply ?? {
        status: 404,
        body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
      }
    );
  });
  const mounted = mountWith(newTestClient(), <MediaManagementPage />);
  cleanup = () => {
    mounted.cleanup();
    api.restore();
  };
  // Control: the page drew its sections.
  await vi.waitFor(
    () => expect(sectionTitles(mounted.container)).toContain("Root Folders"),
    { timeout: 5000 },
  );
  return { container: mounted.container, calls: api.calls };
}

function sectionTitles(scope: HTMLElement): string[] {
  return Array.from(scope.querySelectorAll("h2")).map(
    (h) => h.textContent?.trim() ?? "",
  );
}

/** The section whose heading reads `title`. */
function section(scope: HTMLElement, title: string): Element | null {
  const heading = Array.from(scope.querySelectorAll("h2")).find(
    (h) => h.textContent?.trim() === title,
  );
  return heading?.closest("section") ?? null;
}

describe("Media Management says where imports put files", () => {
  it("shows the File locations sentence, drops the Naming and File Management boxes, and does not read the naming row", async () => {
    const { container, calls } = await openPage();
    const text = container.textContent ?? "";
    expect({
      fileLocations: section(container, "File locations")?.textContent?.includes(
        FILE_LOCATIONS,
      ) ?? false,
      oldTexts: [
        "Naming",
        "Rename Files",
        "Author Folder Format",
        "File Management",
        "Create empty author folders",
      ].filter((t) => text.includes(t)),
      namingReads: calls.filter(
        (c) => c.method === "GET" && c.path === "/config/naming",
      ).length,
    }).toEqual({ fileLocations: true, oldTexts: [], namingReads: 0 });
  });

  it("the Root Folders help tip points to File locations", async () => {
    const { container } = await openPage();
    const header = section(container, "Root Folders")!.querySelector("h2")!
      .parentElement!;
    const tip = header.querySelector("span");
    expect(tip).not.toBeNull();
    await act(async () => {
      tip!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    // Control: hovering opened the tip.
    const tipText = Array.from(document.body.children)
      .filter((el) => !el.contains(container))
      .map((el) => el.textContent ?? "")
      .join("");
    expect(tipText).not.toBe("");
    expect({
      tip: tipText,
      oldWording: tipText.includes("Author/Title subfolders"),
    }).toEqual({ tip: ROOT_FOLDERS_TIP, oldWording: false });
  });
});
