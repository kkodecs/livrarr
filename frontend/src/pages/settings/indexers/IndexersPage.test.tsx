import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import IndexersPage from "@/pages/settings/indexers/IndexersPage";
import { useAuthStore } from "@/stores/auth";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import type { IndexerResponse } from "@/types/api";

function indexer(
  id: number,
  name: string,
  protocol: "torrent" | "usenet",
  interactive: boolean,
): IndexerResponse {
  return {
    id,
    name,
    protocol,
    url: `http://${name.toLowerCase().replace(/ /g, "-")}.invalid`,
    apiPath: "/api",
    apiKeySet: true,
    categories: [7020],
    priority: 1,
    enableAutomaticSearch: true,
    enableInteractiveSearch: interactive,
    supportsBookSearch: true,
    enableRss: true,
    enabled: true,
    addedAt: "2026-01-01T00:00:00Z",
  };
}

// Every stored indexer has Automatic Search on, as the server defaults it.
const TORRENT_ON = indexer(1, "Torrent On", "torrent", true);
const TORRENT_OFF = indexer(2, "Torrent Off", "torrent", false);
const USENET_ON = indexer(3, "Usenet On", "usenet", true);
const USENET_OFF = indexer(4, "Usenet Off", "usenet", false);
const STORED = [TORRENT_ON, TORRENT_OFF, USENET_ON, USENET_OFF];

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
  const api = installApiStub((call: ApiCall): StubReply => {
    const path = call.path.split("?")[0]!;
    if (call.method === "GET" && path === "/indexer") {
      return { status: 200, body: STORED };
    }
    if (call.method === "POST" && path === "/indexer") {
      return { status: 201, body: { ...TORRENT_ON, id: 99 } };
    }
    const update = path.match(/^\/indexer\/(\d+)$/);
    if (call.method === "PUT" && update) {
      const stored = STORED.find((i) => i.id === Number(update[1]))!;
      return { status: 200, body: stored };
    }
    if (call.method === "GET" && path === "/config/indexer") {
      return {
        status: 200,
        body: {
          rssSyncIntervalMinutes: 15,
          rssMatchThreshold: 0.8,
          rssGrabFailureLimit: 3,
        },
      };
    }
    if (call.method === "GET" && path === "/config/prowlarr") {
      return { status: 200, body: { url: null, apiKeySet: false } };
    }
    return {
      status: 404,
      body: { status: 404, error: "not_found", message: `unstubbed ${path}` },
    };
  });
  const mounted = mountWith(newTestClient(), <IndexersPage />);
  cleanup = () => {
    mounted.cleanup();
    api.restore();
  };
  // Control: the list drew.
  await vi.waitFor(
    () => expect(mounted.container.textContent).toContain("Usenet Off"),
    { timeout: 5000 },
  );
  return { container: mounted.container, calls: api.calls };
}

function setValue(el: HTMLInputElement | HTMLSelectElement, value: string) {
  const proto =
    el instanceof HTMLSelectElement
      ? HTMLSelectElement.prototype
      : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

/** The checkbox inside the label that reads `label`, if the form shows one. */
function checkbox(form: Element, label: string): HTMLInputElement | null {
  const owner = Array.from(form.querySelectorAll("label")).find(
    (l) => l.textContent?.trim() === label,
  );
  return owner?.querySelector<HTMLInputElement>('input[type="checkbox"]') ?? null;
}

async function click(el: Element) {
  await act(async () => {
    (el as HTMLElement).click();
  });
}

async function submit(form: Element, label: string) {
  const button = Array.from(form.querySelectorAll("button")).find(
    (b) => b.getAttribute("type") === "submit" && b.textContent?.trim() === label,
  );
  if (!button) throw new Error(`no submit button "${label}"`);
  await click(button);
}

const inlineForm = (container: HTMLElement) =>
  container.querySelector('section[data-tour="add-indexer-form"] form')!;

const creates = (calls: ApiCall[]) =>
  calls.filter((c) => c.method === "POST" && c.path === "/indexer");
const updates = (calls: ApiCall[], id: number) =>
  calls.filter((c) => c.method === "PUT" && c.path === `/indexer/${id}`);

type Body = Record<string, unknown>;

/**
 * Add an indexer through the inline add form, with the Interactive Search box
 * left at its default or clicked once. Returns whether the box was enabled and
 * the request body.
 */
async function addInline(
  protocol: "torrent" | "usenet",
  toggleInteractive: boolean,
) {
  const { container, calls } = await openPage();
  const form = inlineForm(container);
  const name = form.querySelector<HTMLInputElement>('input[placeholder="My Indexer"]')!;
  setValue(name, "New Indexer");
  setValue(form.querySelector<HTMLSelectElement>("select[name=protocol]")!, protocol);
  setValue(
    form.querySelector<HTMLInputElement>('input[placeholder="indexer.example.com"]')!,
    "new.invalid",
  );
  const box = checkbox(form, "Interactive Search");
  expect(box).not.toBeNull();
  const boxEnabled = !box!.disabled;
  if (toggleInteractive) await click(box!);
  await submit(form, "Add Indexer");
  await vi.waitFor(() => expect(creates(calls)).toHaveLength(1), {
    timeout: 5000,
  });
  const body = creates(calls)[0]!.body as Body;
  // Control: the request is the one this form built.
  expect(body.name).toBe("New Indexer");
  expect(body.protocol).toBe(protocol);
  return { boxEnabled, body, form };
}

/** The open edit dialog's form. */
function dialogForm(): Element {
  const form = document.body.querySelector('[role="dialog"] form');
  if (!form) throw new Error("no edit dialog open");
  return form;
}

/**
 * Edit a stored indexer through the edit dialog: rename it, or click the
 * Interactive Search box once. Returns whether the box was enabled and the
 * request body.
 */
async function editIndexer(stored: IndexerResponse, change: "rename" | "toggle") {
  const { container, calls } = await openPage();
  const row = Array.from(container.querySelectorAll("tr")).find((tr) =>
    tr.textContent?.includes(stored.name),
  )!;
  const actions = row.querySelectorAll("td:last-child button");
  // Test, edit, delete: the edit button is the second.
  await click(actions[1]!);
  const form = dialogForm();
  const box = checkbox(form, "Interactive Search");
  expect(box).not.toBeNull();
  const boxEnabled = !box!.disabled;
  if (change === "rename") {
    setValue(
      form.querySelector<HTMLInputElement>('input[placeholder="My Indexer"]')!,
      `${stored.name} Renamed`,
    );
  } else {
    await click(box!);
  }
  await submit(form, "Save");
  await vi.waitFor(() => expect(updates(calls, stored.id)).toHaveLength(1), {
    timeout: 5000,
  });
  const body = updates(calls, stored.id)[0]!.body as Body;
  // Control: the request is the one this dialog built.
  expect(body.name).toBe(
    change === "rename" ? `${stored.name} Renamed` : stored.name,
  );
  return { boxEnabled, body, form };
}

describe("Interactive Search is a working checkbox", () => {
  it("inline add form: the box is enabled; unchecking it and saving sends enableInteractiveSearch false", async () => {
    const { boxEnabled, body } = await addInline("torrent", true);
    expect({ boxEnabled, sent: body.enableInteractiveSearch }).toEqual({
      boxEnabled: true,
      sent: false,
    });
  });

  for (const protocol of ["torrent", "usenet"] as const) {
    it(`inline add form, ${protocol}: leaving the box checked sends true`, async () => {
      const { body } = await addInline(protocol, false);
      expect(body.enableInteractiveSearch).toBe(true);
    });

    it(`inline add form, ${protocol}: unchecking the box sends false`, async () => {
      const { body } = await addInline(protocol, true);
      expect(body.enableInteractiveSearch).toBe(false);
    });
  }

  it("edit an indexer stored on: unchecking sends false", async () => {
    const { boxEnabled, body } = await editIndexer(TORRENT_ON, "toggle");
    expect({ boxEnabled, sent: body.enableInteractiveSearch }).toEqual({
      boxEnabled: true,
      sent: false,
    });
  });

  it("edit an indexer stored off: checking sends true", async () => {
    const { boxEnabled, body } = await editIndexer(USENET_OFF, "toggle");
    expect({ boxEnabled, sent: body.enableInteractiveSearch }).toEqual({
      boxEnabled: true,
      sent: true,
    });
  });

  for (const stored of [TORRENT_ON, TORRENT_OFF]) {
    it(`edit only the name of an indexer stored ${stored.enableInteractiveSearch ? "on" : "off"}: sends its stored value`, async () => {
      const { body } = await editIndexer(stored, "rename");
      expect(body.enableInteractiveSearch).toBe(stored.enableInteractiveSearch);
    });
  }
});

describe("Automatic Search is gone from the forms and requests", () => {
  it("the inline add form does not show Automatic Search, and neither a create nor an update carries enableAutomaticSearch", async () => {
    const created = await addInline("torrent", false);
    const inlineFormShows = (created.form.textContent ?? "").includes(
      "Automatic Search",
    );
    cleanup();
    cleanup = () => {};
    const edited = await editIndexer(TORRENT_ON, "rename");
    expect({
      inlineFormShows,
      createCarries: "enableAutomaticSearch" in created.body,
      updateCarries: "enableAutomaticSearch" in edited.body,
    }).toEqual({
      inlineFormShows: false,
      createCarries: false,
      updateCarries: false,
    });
  });

  it("the edit dialog does not show Automatic Search", async () => {
    const { container } = await openPage();
    const row = Array.from(container.querySelectorAll("tr")).find((tr) =>
      tr.textContent?.includes(TORRENT_ON.name),
    )!;
    await click(row.querySelectorAll("td:last-child button")[1]!);
    const form = dialogForm();
    // Control: the dialog is the indexer form.
    expect(checkbox(form, "RSS Sync")).not.toBeNull();
    expect(form.textContent).not.toContain("Automatic Search");
  });

  const scenarios: Array<[string, () => Promise<{ body: Body }>]> = [
    ["inline add, torrent, box left checked", () => addInline("torrent", false)],
    ["inline add, torrent, box unchecked", () => addInline("torrent", true)],
    ["inline add, usenet, box left checked", () => addInline("usenet", false)],
    ["inline add, usenet, box unchecked", () => addInline("usenet", true)],
    ["edit only the name, stored on", () => editIndexer(TORRENT_ON, "rename")],
    ["edit only the name, stored off", () => editIndexer(TORRENT_OFF, "rename")],
  ];
  for (const [name, run] of scenarios) {
    it(`${name}: the request omits enableAutomaticSearch`, async () => {
      const { body } = await run();
      expect(Object.keys(body)).not.toContain("enableAutomaticSearch");
    });
  }
});

/** The capability badges shown in a list row. */
function badges(container: HTMLElement, name: string): string[] {
  const row = Array.from(container.querySelectorAll("tr")).find(
    (tr) => tr.querySelector("td")?.textContent?.trim() === name,
  )!;
  return Array.from(row.querySelectorAll("td span"))
    .map((s) => s.textContent?.trim() ?? "")
    .filter((t) => ["Book", "Interactive", "Auto", "RSS"].includes(t));
}

describe("List badges", () => {
  it("no Auto badge in either list for indexers stored with Automatic Search on", async () => {
    const { container } = await openPage();
    expect(
      STORED.filter((i) => badges(container, i.name).includes("Auto")).map(
        (i) => i.name,
      ),
    ).toEqual([]);
  });

  it("the Interactive badge follows the stored value in both lists", async () => {
    const { container } = await openPage();
    expect(
      STORED.map((i) => [i.name, badges(container, i.name).includes("Interactive")]),
    ).toEqual([
      ["Torrent On", true],
      ["Torrent Off", false],
      ["Usenet On", true],
      ["Usenet Off", false],
    ]);
  });
});
