import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import SearchPage from "@/pages/search/SearchPage";
import { useAuthStore } from "@/stores/auth";
import { SUPPORTED_LANGUAGES, type UserRole } from "@/types/api";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import {
  AppToaster,
  clearToasts,
  recordAddedToasts,
} from "@/test-support/toasts";

const libraryWork = {
  id: 11,
  title: "Dune",
  authorName: "Frank Herbert",
  olKey: "OL1W",
  seriesName: null,
  seriesPosition: null,
  year: 1965,
  coverUrl: null,
};

const found = {
  title: "Dune Messiah",
  authorName: "Frank Herbert",
  olKey: "OL2W",
  authorOlKey: null,
  year: 1969,
  coverUrl: null,
  description: null,
  source: "openlibrary",
};

type LookupReply = StubReply | "network-error";

// A network rejection reaches the page as apiFetch's "Unable to reach Livrarr".
const NETWORK_MESSAGE = "Unable to reach Livrarr";

/** The page's reads; `lookup` answers each /work/lookup request in turn. */
function searchApi(
  lookup: (call: ApiCall) => LookupReply,
  library: unknown[] = [libraryWork],
) {
  return installApiStub((call) => {
    if (call.method === "GET" && call.path === "/config/metadata") {
      return { status: 200, body: { languages: ["en", "fr"] } };
    }
    if (call.method === "GET" && /^\/work\?/.test(call.path)) {
      return {
        status: 200,
        body: { items: library, total: library.length, page: 1, pageSize: 1000 },
      };
    }
    if (call.method === "GET" && call.path.startsWith("/work/lookup?")) {
      const reply = lookup(call);
      if (reply === "network-error") throw new TypeError("Failed to fetch");
      return reply;
    }
    return {
      status: 404,
      body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
    };
  });
}

const lookups = (calls: ApiCall[]) =>
  calls.filter((c) => c.path.startsWith("/work/lookup?")).map((c) => c.path);

function results(rawAvailable: boolean) {
  return {
    status: 200,
    body: { results: [found], filteredCount: 1, rawCount: 3, rawAvailable },
  };
}

const serverFailure: StubReply = {
  status: 500,
  body: { status: 500, error: "internal", message: "Search is unavailable right now" },
};

const buttonLabels = (scope: HTMLElement) =>
  Array.from(scope.querySelectorAll("button")).map(
    (b) => b.textContent?.trim() ?? "",
  );

let restore: () => void = () => {};
afterEach(() => restore());

describe("Book search: the search request fails", () => {
  it("a 500 shows the server's message and Retry, keeps library matches, and Retry repeats the same search", async () => {
    let lookupReply: LookupReply = results(true);
    const api = searchApi(() => lookupReply);
    const mounted = mountWith(newTestClient(), <SearchPage />, {
      path: "/search?q=Dune&lang=fr",
    });
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );

    lookupReply = serverFailure;
    await clickButton(mounted.container, "Raw 3");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    const failedSearch = lookups(api.calls)[1];
    expect(failedSearch).toBe("/work/lookup?term=Dune&lang=fr&raw=true");

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(
        "Search is unavailable right now",
      ),
    );
    const text = mounted.container.textContent ?? "";
    expect(text).not.toContain("No results");
    expect(text).toContain("In Your Library");
    expect(text).toContain("Dune");

    lookupReply = results(true);
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(3));
    expect(lookups(api.calls)[2]).toBe(failedSearch);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );
    expect(mounted.container.textContent).not.toContain(
      "Search is unavailable right now",
    );
  });

  it("a network rejection shows the error and Retry, not No results, and Retry repeats the same search", async () => {
    let lookupReply: LookupReply = "network-error";
    const api = searchApi(() => lookupReply);
    const mounted = mountWith(newTestClient(), <SearchPage />, {
      path: "/search?q=Dune&lang=fr",
    });
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
    const failedSearch = lookups(api.calls)[0];
    expect(failedSearch).toBe("/work/lookup?term=Dune&lang=fr");

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(NETWORK_MESSAGE),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    const text = mounted.container.textContent ?? "";
    expect(text).not.toContain("No results");
    expect(text).toContain("In Your Library");

    lookupReply = results(false);
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe(failedSearch);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );
    expect(mounted.container.textContent).not.toContain(NETWORK_MESSAGE);
    expect(buttonLabels(mounted.container)).not.toContain("Retry");
  });

  it("with no library match, a 500 shows the error and Retry, not No results", async () => {
    let lookupReply: LookupReply = serverFailure;
    const api = searchApi(() => lookupReply, []);
    const mounted = mountWith(newTestClient(), <SearchPage />, {
      path: "/search?q=Dune",
    });
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
    const failedSearch = lookups(api.calls)[0];

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(
        "Search is unavailable right now",
      ),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    expect(mounted.container.textContent).not.toContain("No results");
    expect(mounted.container.textContent).not.toContain("In Your Library");

    lookupReply = results(false);
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe(failedSearch);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );
    expect(mounted.container.textContent).not.toContain(
      "Search is unavailable right now",
    );
  });
});

type LanguageReply = StubReply | "network-error";

const languageList = (codes: string[]): StubReply => ({
  status: 200,
  body: { languages: codes },
});

const languageReadFailure: StubReply = {
  status: 500,
  body: { status: 500, error: "internal", message: "language list unavailable" },
};

/** The admin-only settings reply, as the server sends it to an admin. */
function adminMetadata(codes: string[]): StubReply {
  return {
    status: 200,
    body: {
      hardcoverEnabled: true,
      hardcoverApiTokenSet: true,
      llmEnabled: true,
      llmProvider: "openai",
      llmEndpoint: "https://llm.example.com/v1",
      llmApiKeySet: true,
      llmModel: "gpt-test",
      audnexusUrl: "https://api.audnex.us",
      languages: codes,
      googleBooksApiKeySet: true,
      providerStatus: {},
    },
  };
}

const forbidden: StubReply = {
  status: 403,
  body: { status: 403, error: "forbidden", message: "forbidden" },
};

function signIn(role: UserRole) {
  useAuthStore.setState({
    status: "authenticated",
    token: "test-session",
    isAdmin: role === "admin",
    user: {
      id: role === "admin" ? 1 : 2,
      username: role === "admin" ? "owner" : "reader",
      role,
      createdAt: "2026-10-01T00:00:00Z",
      updatedAt: "2026-10-01T00:00:00Z",
    },
  });
}

/**
 * Add New's reads for a signed-in `role`. `/config/languages` answers with
 * `languageReply`; `/config/metadata` answers as the server does: the saved
 * settings for an admin, 403 for a normal user.
 */
function addNewApi(
  role: UserRole,
  languageReply: () => LanguageReply,
  savedLanguages: string[],
) {
  return installApiStub((call) => {
    if (call.method === "GET" && call.path === "/config/languages") {
      const reply = languageReply();
      if (reply === "network-error") throw new TypeError("Failed to fetch");
      return reply;
    }
    if (call.method === "GET" && call.path === "/config/metadata") {
      return role === "admin" ? adminMetadata(savedLanguages) : forbidden;
    }
    if (call.method === "GET" && /^\/work\?/.test(call.path)) {
      return {
        status: 200,
        body: { items: [], total: 0, page: 1, pageSize: 1000 },
      };
    }
    if (call.method === "GET" && call.path.startsWith("/work/lookup?")) {
      return {
        status: 200,
        body: { results: [], filteredCount: 0, rawCount: 0, rawAvailable: false },
      };
    }
    return {
      status: 404,
      body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
    };
  });
}

const languageReads = (calls: ApiCall[]) =>
  calls.filter(
    (c) =>
      c.method === "GET" &&
      (c.path === "/config/languages" || c.path === "/config/metadata"),
  ).length;

const metadataReads = (calls: ApiCall[]) =>
  calls.filter((c) => c.method === "GET" && c.path === "/config/metadata")
    .length;

/** The language each search was sent in; a lookup without `lang` searches in English. */
const lookupLanguages = (calls: ApiCall[]) =>
  lookups(calls).map(
    (path) => new URLSearchParams(path.split("?")[1]).get("lang") ?? "en",
  );

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

function mountAddNew(api: ReturnType<typeof installApiStub>) {
  const mounted = mountWith(
    newTestClient(),
    <>
      <SearchPage />
      <AppToaster />
    </>,
    { path: "/search" },
  );
  return {
    ...mounted,
    cleanup: () => {
      mounted.cleanup();
      api.restore();
    },
  };
}

const searchForm = (scope: HTMLElement) => scope.querySelector("form")!;

/** The language picker's trigger: the form's only non-submit button while closed. */
const pickerTrigger = (scope: HTMLElement) =>
  searchForm(scope).querySelector<HTMLButtonElement>('button[type="button"]');

const languageNamesIn = (texts: string[]) =>
  SUPPORTED_LANGUAGES.filter((l) => texts.some((t) => t.includes(l.englishName))).map(
    (l) => l.englishName,
  );

/** Opens the picker and returns the language names it offers, or null with no picker. */
async function offeredLanguages(scope: HTMLElement): Promise<string[] | null> {
  const trigger = pickerTrigger(scope);
  if (!trigger) return null;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  const offered = languageNamesIn(
    Array.from(searchForm(scope).querySelectorAll('button[type="button"]'))
      .filter((b) => b !== trigger)
      .map((b) => b.textContent ?? ""),
  );
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return offered;
}

async function searchFor(scope: HTMLElement, term: string) {
  const form = searchForm(scope);
  const input = form.querySelector<HTMLInputElement>('input[type="text"]')!;
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    setValue.call(input, term);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

/** Add New's text with only English saved and readable, before and after one search. */
async function englishOnlyAddNewText(
  role: UserRole,
): Promise<{ beforeSearch: string; afterSearch: string }> {
  signIn(role);
  const api = addNewApi(role, () => languageList(["en"]), ["en"]);
  const mounted = mountAddNew(api);
  try {
    await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
    await settle();
    const beforeSearch = mounted.container.textContent ?? "";
    await searchFor(mounted.container, "Dune");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
    await settle();
    return { beforeSearch, afterSearch: mounted.container.textContent ?? "" };
  } finally {
    mounted.cleanup();
  }
}

const roleName = (role: UserRole) => (role === "admin" ? "an admin" : "a normal user");

const languageRouteReads = (calls: ApiCall[]) =>
  calls.filter((c) => c.method === "GET" && c.path === "/config/languages")
    .length;

describe("Add New: a saved list of several languages", () => {
  afterEach(async () => {
    await clearToasts();
    useAuthStore.setState({ status: "loading", user: null, token: null, isAdmin: false });
  });

  for (const role of ["admin", "user"] as const) {
    it(`offers ${roleName(role)} English, French and German from the language route and never reads the admin settings`, async () => {
      signIn(role);
      const api = addNewApi(role, () => languageList(["en", "fr", "de"]), [
        "en",
        "fr",
        "de",
      ]);
      const mounted = mountAddNew(api);
      restore = mounted.cleanup;

      // Controls: the page rendered and asked for its language list.
      await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
      await settle();
      expect(mounted.container.querySelector("h1")?.textContent).toBe("Search");

      expect({
        offered: await offeredLanguages(mounted.container),
        readsLanguageRoute: languageRouteReads(api.calls) > 0,
        metadataReads: metadataReads(api.calls),
      }).toEqual({
        offered: ["English", "French", "German"],
        readsLanguageRoute: true,
        metadataReads: 0,
      });
    });

    it(`with French saved first, preselects French for ${roleName(role)} and searches in French`, async () => {
      signIn(role);
      const api = addNewApi(role, () => languageList(["fr", "en"]), ["fr", "en"]);
      const mounted = mountAddNew(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
      await settle();
      expect(mounted.container.querySelector("h1")?.textContent).toBe("Search");

      const trigger = pickerTrigger(mounted.container);
      const shownChoice = trigger ? languageNamesIn([trigger.textContent ?? ""]) : null;
      await searchFor(mounted.container, "Dune");
      await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));

      expect({
        shownChoice,
        searchLanguages: lookupLanguages(api.calls),
        readsLanguageRoute: languageRouteReads(api.calls) > 0,
        metadataReads: metadataReads(api.calls),
      }).toEqual({
        shownChoice: ["French"],
        searchLanguages: ["fr"],
        readsLanguageRoute: true,
        metadataReads: 0,
      });
    });
  }
});

describe("Add New: one saved language", () => {
  afterEach(async () => {
    await clearToasts();
    useAuthStore.setState({ status: "loading", user: null, token: null, isAdmin: false });
  });

  for (const role of ["admin", "user"] as const) {
    it(`shows no picker for ${roleName(role)}`, async () => {
      signIn(role);
      const api = addNewApi(role, () => languageList(["en"]), ["en"]);
      const mounted = mountAddNew(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
      await settle();
      expect(mounted.container.querySelector("h1")?.textContent).toBe("Search");

      expect(await offeredLanguages(mounted.container)).toBeNull();
    });
  }
});

describe("Add New: the language list cannot be read", () => {
  afterEach(async () => {
    await clearToasts();
    useAuthStore.setState({ status: "loading", user: null, token: null, isAdmin: false });
  });

  const failures: Array<[string, () => LanguageReply]> = [
    ["answered 500", () => languageReadFailure],
    ["rejected as a network failure", () => "network-error"],
  ];
  for (const role of ["admin", "user"] as const) {
    for (const [label, reply] of failures) {
      it(`${roleName(role)}, language read ${label}: searches in English, no picker, no message, no settings read`, async () => {
        const englishOnly = await englishOnlyAddNewText(role);

        signIn(role);
        // The saved list puts French first, so a fallback read of the admin
        // settings would show a picker and search in French.
        const api = addNewApi(role, reply, ["fr", "en"]);
        const toasts = recordAddedToasts();
        const mounted = mountAddNew(api);
        restore = () => {
          toasts.stop();
          mounted.cleanup();
        };

        // Controls: the page rendered at /search with no language and asked for its language list.
        await vi.waitFor(() =>
          expect(languageReads(api.calls)).toBeGreaterThan(0),
        );
        await settle();
        expect(mounted.container.querySelector("h1")?.textContent).toBe("Search");

        const beforeSearch = mounted.container.textContent ?? "";
        const offered = await offeredLanguages(mounted.container);
        await searchFor(mounted.container, "Dune");
        await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
        await settle();

        expect({
          searchLanguages: lookupLanguages(api.calls),
          offered,
          beforeSearch,
          afterSearch: mounted.container.textContent ?? "",
          newToasts: toasts.added().length,
          metadataReads: metadataReads(api.calls),
        }).toEqual({
          searchLanguages: ["en"],
          offered: null,
          beforeSearch: englishOnly.beforeSearch,
          afterSearch: englishOnly.afterSearch,
          newToasts: 0,
          metadataReads: 0,
        });
      });
    }
  }
});

/** The browser's tab-return event, which makes the app re-read stale queries. */
async function returnToTab() {
  await act(async () => {
    window.dispatchEvent(new Event("visibilitychange"));
  });
}

/** Opens the picker and chooses `name` from it. */
async function chooseLanguage(scope: HTMLElement, name: string) {
  const trigger = pickerTrigger(scope)!;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  const option = Array.from(
    searchForm(scope).querySelectorAll<HTMLButtonElement>('button[type="button"]'),
  ).find((b) => b !== trigger && (b.textContent ?? "").includes(name))!;
  await act(async () => {
    option.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("Add New: the saved language list changes after a language was chosen", () => {
  afterEach(async () => {
    await clearToasts();
    useAuthStore.setState({ status: "loading", user: null, token: null, isAdmin: false });
  });

  for (const role of ["admin", "user"] as const) {
    it(`${roleName(role)} chose French, then a re-read list drops French: no picker and the search is in English`, async () => {
      signIn(role);
      let saved = ["en", "fr"];
      const api = addNewApi(role, () => languageList(saved), saved);
      const mounted = mountAddNew(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(1));
      await settle();
      await chooseLanguage(mounted.container, "French");
      // Control: French is the shown choice before the list changes.
      expect(languageNamesIn([pickerTrigger(mounted.container)?.textContent ?? ""])).toEqual([
        "French",
      ]);

      saved = ["en"];
      await returnToTab();
      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(2));
      await settle();
      const offered = await offeredLanguages(mounted.container);
      await searchFor(mounted.container, "Dune");
      await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));

      expect({ offered, searchLanguages: lookupLanguages(api.calls) }).toEqual({
        offered: null,
        searchLanguages: ["en"],
      });
    });

    it(`${roleName(role)} chose French, then a re-read list is unchanged: French stays chosen`, async () => {
      signIn(role);
      const api = addNewApi(role, () => languageList(["en", "fr"]), ["en", "fr"]);
      const mounted = mountAddNew(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(1));
      await settle();
      await chooseLanguage(mounted.container, "French");

      await returnToTab();
      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(2));
      await settle();
      await searchFor(mounted.container, "Dune");
      await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));

      expect(lookupLanguages(api.calls)).toEqual(["fr"]);
    });
  }
});
