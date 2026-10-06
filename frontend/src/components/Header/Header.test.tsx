import { act } from "react";
import { useLocation } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Header } from "@/components/Header/Header";
import { useAuthStore } from "@/stores/auth";
import { SUPPORTED_LANGUAGES, type UserRole } from "@/types/api";
import {
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

type LanguageReply = StubReply | "network-error";

const languages = (codes: string[]): StubReply => ({
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
 * The header's reads. `/config/languages` answers with `languageReply`;
 * `/config/metadata` answers as the server does for `role`: the saved
 * settings for an admin, 403 for a normal user.
 */
function headerApi(
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
    if (call.method === "GET" && call.path.startsWith("/notification?")) {
      return {
        status: 200,
        body: { items: [], total: 0, page: 1, pageSize: 200 },
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

/** The address the router is at, so a navigation from the header is visible. */
function LocationProbe() {
  const location = useLocation();
  return (
    <output data-testid="location">{location.pathname + location.search}</output>
  );
}

function mountHeader(api: ReturnType<typeof installApiStub>) {
  const mounted = mountWith(
    newTestClient(),
    <>
      <Header />
      <LocationProbe />
      <AppToaster />
    </>,
  );
  return {
    ...mounted,
    header: () => mounted.container.querySelector("header")!,
    location: () =>
      mounted.container.querySelector('[data-testid="location"]')!.textContent,
    cleanup: () => {
      mounted.cleanup();
      api.restore();
    },
  };
}

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

/** The desktop search form, the one shown at full width. */
const searchForm = (header: Element) => header.querySelector("form")!;

/** The language picker's trigger: the form's only non-submit button while closed. */
const pickerTrigger = (header: Element) =>
  searchForm(header).querySelector<HTMLButtonElement>('button[type="button"]');

/** Opens the picker and returns the language names it offers, or null with no picker. */
async function offeredLanguages(header: Element): Promise<string[] | null> {
  const trigger = pickerTrigger(header);
  if (!trigger) return null;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  const optionTexts = Array.from(
    searchForm(header).querySelectorAll('button[type="button"]'),
  )
    .filter((b) => b !== trigger)
    .map((b) => b.textContent ?? "");
  const offered = SUPPORTED_LANGUAGES.filter((l) =>
    optionTexts.some((t) => t.includes(l.englishName)),
  ).map((l) => l.englishName);
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return offered;
}

async function typeInto(input: HTMLInputElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    setValue.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function searchFromHeader(header: Element, term: string) {
  const form = searchForm(header);
  await typeInto(form.querySelector<HTMLInputElement>('input[type="text"]')!, term);
  await act(async () => {
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

/** The header's text once its language read has settled, with only English saved and readable. */
async function englishOnlyHeaderText(role: UserRole): Promise<string> {
  signIn(role);
  const api = headerApi(role, () => languages(["en"]), ["en"]);
  const mounted = mountHeader(api);
  try {
    await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
    await settle();
    return mounted.header().textContent ?? "";
  } finally {
    mounted.cleanup();
  }
}

let restore: () => void = () => {};
afterEach(async () => {
  restore();
  restore = () => {};
  await clearToasts();
  useAuthStore.setState({
    status: "loading",
    user: null,
    token: null,
    isAdmin: false,
  });
});

const languageRouteReads = (calls: ApiCall[]) =>
  calls.filter((c) => c.method === "GET" && c.path === "/config/languages")
    .length;

const roleName = (role: UserRole) => (role === "admin" ? "an admin" : "a normal user");
const username = (role: UserRole) => (role === "admin" ? "owner" : "reader");

describe("Header search: a saved list of several languages", () => {
  for (const role of ["admin", "user"] as const) {
    it(`offers ${roleName(role)} English, French and German from the language route and never reads the admin settings`, async () => {
      signIn(role);
      const api = headerApi(role, () => languages(["en", "fr", "de"]), [
        "en",
        "fr",
        "de",
      ]);
      const mounted = mountHeader(api);
      restore = mounted.cleanup;

      // Controls: the header rendered for the signed-in user and asked for its language list.
      await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
      await settle();
      expect(mounted.header().textContent).toContain(username(role));

      expect({
        offered: await offeredLanguages(mounted.header()),
        readsLanguageRoute: languageRouteReads(api.calls) > 0,
        metadataReads: metadataReads(api.calls),
      }).toEqual({
        offered: ["English", "French", "German"],
        readsLanguageRoute: true,
        metadataReads: 0,
      });
    });

    it(`with French saved first, a fresh header preselects French for ${roleName(role)} and searches in French`, async () => {
      signIn(role);
      const api = headerApi(role, () => languages(["fr", "en"]), ["fr", "en"]);
      const mounted = mountHeader(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
      await settle();
      expect(mounted.header().textContent).toContain(username(role));

      // The picker is read, never opened or clicked: the choice must come from the saved order.
      const trigger = pickerTrigger(mounted.header());
      const shownChoice = trigger
        ? SUPPORTED_LANGUAGES.filter((l) =>
            (trigger.textContent ?? "").includes(l.flag),
          ).map((l) => l.englishName)
        : null;
      await searchFromHeader(mounted.header(), "Dune");
      await settle();

      expect({
        shownChoice,
        address: mounted.location(),
        readsLanguageRoute: languageRouteReads(api.calls) > 0,
        metadataReads: metadataReads(api.calls),
      }).toEqual({
        shownChoice: ["French"],
        address: "/search?q=Dune&lang=fr",
        readsLanguageRoute: true,
        metadataReads: 0,
      });
    });
  }
});

describe("Header search: one saved language", () => {
  for (const role of ["admin", "user"] as const) {
    it(`shows no picker for ${role === "admin" ? "an admin" : "a normal user"}`, async () => {
      signIn(role);
      const api = headerApi(role, () => languages(["en"]), ["en"]);
      const mounted = mountHeader(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageReads(api.calls)).toBeGreaterThan(0));
      await settle();
      expect(mounted.header().textContent).toContain(
        role === "admin" ? "owner" : "reader",
      );

      expect(await offeredLanguages(mounted.header())).toBeNull();
    });
  }
});

describe("Header search: the language list cannot be read", () => {
  const failures: Array<[string, () => LanguageReply]> = [
    ["answered 500", () => languageReadFailure],
    ["rejected as a network failure", () => "network-error"],
  ];
  for (const role of ["admin", "user"] as const) {
    for (const [label, reply] of failures) {
      it(`${role === "admin" ? "an admin" : "a normal user"}, language read ${label}: English only, no picker, no message, no settings read`, async () => {
        const englishOnly = await englishOnlyHeaderText(role);

        signIn(role);
        // The saved list puts French first, so a fallback read of the admin
        // settings would show a picker and search in French.
        const api = headerApi(role, reply, ["fr", "en"]);
        const toasts = recordAddedToasts();
        const mounted = mountHeader(api);
        restore = () => {
          toasts.stop();
          mounted.cleanup();
        };

        // Controls: the header rendered for the signed-in user and asked for its language list.
        await vi.waitFor(() =>
          expect(languageReads(api.calls)).toBeGreaterThan(0),
        );
        await settle();
        expect(mounted.header().textContent).toContain(
          role === "admin" ? "owner" : "reader",
        );

        const headerText = mounted.header().textContent ?? "";
        const offered = await offeredLanguages(mounted.header());
        await searchFromHeader(mounted.header(), "Dune");
        await settle();

        expect({
          address: mounted.location(),
          offered,
          headerText,
          newToasts: toasts.added().length,
          metadataReads: metadataReads(api.calls),
        }).toEqual({
          address: "/search?q=Dune",
          offered: null,
          headerText: englishOnly,
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
async function chooseLanguage(header: Element, name: string) {
  const trigger = pickerTrigger(header)!;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  const option = Array.from(
    searchForm(header).querySelectorAll<HTMLButtonElement>('button[type="button"]'),
  ).find((b) => b !== trigger && (b.textContent ?? "").includes(name))!;
  await act(async () => {
    option.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("Header search: the saved language list changes after a language was chosen", () => {
  for (const role of ["admin", "user"] as const) {
    it(`${roleName(role)} chose French, then a re-read list drops French: no picker and the search is in English`, async () => {
      signIn(role);
      let saved = ["en", "fr"];
      const api = headerApi(role, () => languages(saved), saved);
      const mounted = mountHeader(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(1));
      await settle();
      await chooseLanguage(mounted.header(), "French");
      // Control: French is the shown choice before the list changes.
      expect(pickerTrigger(mounted.header())?.textContent).toContain(
        SUPPORTED_LANGUAGES.find((l) => l.code === "fr")!.flag,
      );

      saved = ["en"];
      await returnToTab();
      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(2));
      await settle();
      const offered = await offeredLanguages(mounted.header());
      await searchFromHeader(mounted.header(), "Dune");
      await settle();

      expect({ offered, address: mounted.location() }).toEqual({
        offered: null,
        address: "/search?q=Dune",
      });
    });

    it(`${roleName(role)} chose French, then a re-read list is unchanged: French stays chosen`, async () => {
      signIn(role);
      const api = headerApi(role, () => languages(["en", "fr"]), ["en", "fr"]);
      const mounted = mountHeader(api);
      restore = mounted.cleanup;

      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(1));
      await settle();
      await chooseLanguage(mounted.header(), "French");

      await returnToTab();
      await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(2));
      await settle();
      await searchFromHeader(mounted.header(), "Dune");
      await settle();

      expect(mounted.location()).toBe("/search?q=Dune&lang=fr");
    });
  }
});

/** The phone search overlay: the header's direct child that holds a search form. */
const mobileOverlay = (header: Element) =>
  Array.from(header.children).find(
    (el) => el.tagName === "DIV" && el.querySelector("form") !== null,
  ) ?? null;

/** Language names offered in the phone overlay's language list (outside its form). */
function mobileListLanguages(header: Element): string[] {
  const overlay = mobileOverlay(header);
  if (!overlay) return [];
  const form = overlay.querySelector("form")!;
  const texts = Array.from(overlay.querySelectorAll("button"))
    .filter((b) => !form.contains(b))
    .map((b) => b.textContent ?? "");
  return SUPPORTED_LANGUAGES.filter((l) =>
    texts.some((t) => t.includes(l.englishName)),
  ).map((l) => l.englishName);
}

/** A tap on a phone: `mousedown`, then `click`, on the same element. */
async function tap(el: Element) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  });
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

async function click(el: Element) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("Header search on a phone: the language list after closing and reopening search", () => {
  it("open search, open the language list, close with X, tap the search button: the language list is closed", async () => {
    signIn("user");
    const api = headerApi("user", () => languages(["en", "fr"]), ["en", "fr"]);
    const mounted = mountHeader(api);
    restore = mounted.cleanup;

    await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(1));
    await settle();
    const header = mounted.header();
    const searchButton = header.querySelector<HTMLButtonElement>(
      'button[aria-label="Search"]',
    )!;

    // Open search.
    await tap(searchButton);
    const overlay = mobileOverlay(header);
    expect(overlay).not.toBeNull();

    // Open the language list from the overlay's picker.
    const overlayPicker = overlay!
      .querySelector("form")!
      .querySelector<HTMLButtonElement>('button[type="button"]')!;
    await click(overlayPicker);
    // Control: the overlay shows the language list.
    expect(mobileListLanguages(header)).toEqual(["English", "French"]);

    // Close search with X: the overlay form's last button.
    const formButtons = overlay!
      .querySelector("form")!
      .querySelectorAll<HTMLButtonElement>('button[type="button"]');
    await click(formButtons[formButtons.length - 1]!);
    // Control: the overlay is gone.
    expect(mobileOverlay(header)).toBeNull();

    // Reopen search with a tap on the search button.
    await tap(searchButton);
    // Control: the overlay is back.
    expect(mobileOverlay(header)).not.toBeNull();

    expect(mobileListLanguages(header)).toEqual([]);
  });
});

describe("Header search on the desktop: click outside the language list", () => {
  it("an open language list closes on a mousedown outside it", async () => {
    signIn("user");
    const api = headerApi("user", () => languages(["en", "fr"]), ["en", "fr"]);
    const mounted = mountHeader(api);
    restore = mounted.cleanup;

    await vi.waitFor(() => expect(languageRouteReads(api.calls)).toBe(1));
    await settle();
    const header = mounted.header();
    const trigger = pickerTrigger(header)!;
    const options = () =>
      Array.from(
        searchForm(header).querySelectorAll<HTMLButtonElement>('button[type="button"]'),
      ).filter((b) => b !== trigger).length;

    await click(trigger);
    // Control: the list is open with one option per language.
    expect(options()).toBe(2);

    await act(async () => {
      document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });

    expect(options()).toBe(0);
  });
});
