import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";
import MetadataPage from "./MetadataPage";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import { AppToaster, recordAddedToasts } from "@/test-support/toasts";
import {
  SUPPORTED_LANGUAGES,
  type MetadataConfigResponse,
  type UpdateMetadataConfigRequest,
} from "@/types/api";

// Settings -> Metadata mounted for real with the real API wrappers and
// `apiFetch`; only `fetch` is stubbed. The stub keeps the saved settings and
// applies a successful save to them, as the server does, so every reload of
// the page reads what was last saved. Pop-ups are read from the app's Toaster
// mounted beside the page.

vi.setConfig({ testTimeout: 30_000 });

const SAVE_FIRST = "Save your changes first. Test checks the saved settings.";
const GROQ_ENDPOINT = "https://api.groq.com/openai/v1";
const OPENAI_ENDPOINT = "https://api.openai.com/v1";
const GROQ_FIRST_MODEL = "llama-3.3-70b-versatile";
const GROQ_SECOND_MODEL = "llama-3.1-8b-instant";
const SAVED_AUDNEXUS = "https://api.audnex.us";
const ENTERED_AUDNEXUS = "https://audnexus.example.test";
const ATTEMPTED_AUDNEXUS = "https://attempt.example.test";

const SERVICES = [
  {
    heading: "Hardcover",
    route: "/config/metadata/test/hardcover",
    ok: "Hardcover connection successful",
    badGateway: "Hardcover returned 401 — check API token",
  },
  {
    heading: "Audnexus",
    route: "/config/metadata/test/audnexus",
    ok: "Audnexus connection successful",
    badGateway: "Audnexus returned 503",
  },
  {
    heading: "LLM Enrichment",
    route: "/config/metadata/test/llm",
    ok: "AI connection successful",
    badGateway: "LLM returned 500 Internal Server Error (see server logs for details)",
  },
] as const;

type Service = (typeof SERVICES)[number];

const OTHER_LANGUAGES = SUPPORTED_LANGUAGES.filter((l) => l.code !== "en");

// ── Stubbed server ──

interface Saved {
  config: MetadataConfigResponse;
  defaultLanguage: string;
}

type Outcome = "ok" | "fail";
type TestReply = StubReply | "network-error";

/** Replies the test can reassign or hold while the page is mounted. */
interface Script {
  metadataPut: () => Outcome | Promise<Outcome>;
  defaultLanguagePut: () => Outcome | Promise<Outcome>;
  test: (route: string) => TestReply | Promise<TestReply>;
}

const SERVER_FAILURE: StubReply = {
  status: 500,
  body: { status: 500, error: "internal", message: "Something went wrong" },
};

function savedSettings(
  config: Partial<MetadataConfigResponse> = {},
  defaultLanguage = "en",
): Saved {
  return {
    config: {
      hardcoverEnabled: true,
      hardcoverApiTokenSet: true,
      llmEnabled: true,
      llmProvider: "groq",
      llmEndpoint: GROQ_ENDPOINT,
      llmApiKeySet: true,
      llmModel: GROQ_FIRST_MODEL,
      audnexusUrl: SAVED_AUDNEXUS,
      languages: ["en"],
      googleBooksApiKeySet: true,
      ...config,
    },
    defaultLanguage,
  };
}

/** A successful metadata save as the server stores it: secrets kept as "is set" flags. */
function applyMetadataSave(c: MetadataConfigResponse, req: UpdateMetadataConfigRequest) {
  if (req.hardcoverEnabled !== undefined) c.hardcoverEnabled = req.hardcoverEnabled;
  if (req.llmEnabled !== undefined) c.llmEnabled = req.llmEnabled;
  if (req.hardcoverApiToken) c.hardcoverApiTokenSet = true;
  if (req.googleBooksApiKey) c.googleBooksApiKeySet = true;
  if (req.llmApiKey) c.llmApiKeySet = true;
  if (req.audnexusUrl) c.audnexusUrl = req.audnexusUrl;
  if (req.llmProvider !== undefined) c.llmProvider = req.llmProvider;
  if (req.llmEndpoint !== undefined) c.llmEndpoint = req.llmEndpoint;
  if (req.llmModel !== undefined) c.llmModel = req.llmModel;
  if (Array.isArray(req.languages)) c.languages = [...req.languages];
}

const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

function installMetadataApi(saved: Saved, overrides: Partial<Script> = {}) {
  const script: Script = {
    metadataPut: () => "ok",
    defaultLanguagePut: () => "ok",
    test: () => ({ status: 200 }),
    ...overrides,
  };
  const stub = installApiStub(async (call: ApiCall): Promise<StubReply> => {
    if (call.method === "GET" && call.path === "/config/metadata") {
      return { status: 200, body: copy(saved.config) };
    }
    if (call.method === "GET" && call.path === "/config/default-language") {
      return { status: 200, body: { defaultLanguage: saved.defaultLanguage } };
    }
    if (call.method === "PUT" && call.path === "/config/metadata") {
      if ((await script.metadataPut()) === "fail") return SERVER_FAILURE;
      applyMetadataSave(saved.config, call.body as UpdateMetadataConfigRequest);
      return { status: 200, body: copy(saved.config) };
    }
    if (call.method === "PUT" && call.path === "/config/default-language") {
      if ((await script.defaultLanguagePut()) === "fail") return SERVER_FAILURE;
      saved.defaultLanguage = (call.body as { defaultLanguage: string }).defaultLanguage;
      return { status: 200, body: { defaultLanguage: saved.defaultLanguage } };
    }
    if (call.method === "POST" && call.path.startsWith("/config/metadata/test/")) {
      const reply = await script.test(call.path);
      if (reply === "network-error") throw new TypeError("Failed to fetch");
      return reply;
    }
    return {
      status: 404,
      body: { status: 404, error: "not_found", message: `unstubbed ${call.method} ${call.path}` },
    };
  });
  cleanups.push(stub.restore);

  // The index of every call whose reply body the page has finished reading.
  const handled = new Set<number>();
  const stubbedFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const index = stub.calls.length;
    const response = await stubbedFetch(input, init);
    const read = <T,>(body: () => Promise<T>) => async () => {
      try {
        return await body();
      } finally {
        handled.add(index);
      }
    };
    response.text = read(response.text.bind(response));
    response.json = read(response.json.bind(response));
    return response;
  }) as typeof globalThis.fetch;

  return { calls: stub.calls, script, handled };
}

type Api = ReturnType<typeof installMetadataApi>;

function errorReply(status: number, error: string, message: string): StubReply {
  return { status, body: { status, error, message } };
}

function hold<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/** Every request other than a read, as "METHOD /path", from `from` on. */
function writesSince(api: Api, from: number): string[] {
  return api.calls
    .slice(from)
    .filter((c) => c.method !== "GET")
    .map((c) => `${c.method} ${c.path}`);
}

// ── Page and controls ──

let cleanups: Array<() => void> = [];

afterEach(() => {
  act(() => {
    toast.dismiss();
  });
  for (const cleanup of cleanups.reverse()) cleanup();
  cleanups = [];
});

async function pause(ms: number) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function mountPage(saved: Saved): Promise<HTMLElement> {
  const mounted = mountWith(
    newTestClient(),
    <>
      <MetadataPage />
      <AppToaster />
    </>,
  );
  cleanups.push(mounted.cleanup);
  const root = mounted.container;
  await vi.waitFor(() => {
    expect(field(root, "audnexusUrl").value).toBe(saved.config.audnexusUrl);
    expect(defaultLanguageSelect(root).value).toBe(saved.defaultLanguage);
    expect(defaultLanguageSelect(root).disabled).toBe(false);
  });
  return root;
}

function field(root: HTMLElement, name: string): HTMLInputElement {
  const input = root.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  if (!input) throw new Error(`no input named ${name}`);
  return input;
}

function selectWithOption(root: HTMLElement, value: string): HTMLSelectElement {
  const select = root.querySelector(`select option[value="${value}"]`)?.closest("select");
  if (!select) throw new Error(`no select offering "${value}"`);
  return select;
}

const providerSelect = (root: HTMLElement) => selectWithOption(root, "groq");
const modelSelect = (root: HTMLElement) => selectWithOption(root, GROQ_FIRST_MODEL);
const defaultLanguageSelect = (root: HTMLElement) => selectWithOption(root, "fr");

function buttonByText(root: HTMLElement, text: string): HTMLButtonElement {
  const button = Array.from(root.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === text,
  );
  if (!button) throw new Error(`no button "${text}"`);
  return button;
}

function saveButton(root: HTMLElement): HTMLButtonElement {
  const button = root.querySelector<HTMLButtonElement>('button[type="submit"]');
  if (!button) throw new Error("no Save button");
  return button;
}

const TEST_LABEL = /^(Test|Testing\.\.\.)$/;

/** The Test button of the section headed `heading`: the nearest one around that heading. */
function testButton(root: HTMLElement, heading: string): HTMLButtonElement {
  const title = Array.from(root.querySelectorAll("h1, h2, h3")).find(
    (h) => h.textContent?.trim() === heading,
  );
  if (!title) throw new Error(`no "${heading}" heading`);
  for (let el = title.parentElement; el && el !== root; el = el.parentElement) {
    const found = Array.from(el.querySelectorAll("button")).filter((b) =>
      TEST_LABEL.test(b.textContent?.trim() ?? ""),
    );
    if (found.length === 1) return found[0]!;
    if (found.length > 1) {
      throw new Error(`the ${heading} heading shares its section with ${found.length} Test buttons`);
    }
  }
  throw new Error(`no Test button in the ${heading} section`);
}

/** The switch in the language row named `name` (a row holds one language name and one switch). */
function languageToggle(root: HTMLElement, name: string): HTMLButtonElement {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeValue?.trim() !== name) continue;
    const start = node.parentElement;
    if (!start || start.closest("select")) continue;
    for (let el: HTMLElement | null = start; el && el !== root; el = el.parentElement) {
      const row: HTMLElement = el;
      const switches = row.querySelectorAll<HTMLButtonElement>("button, [role='switch']");
      if (switches.length === 0) continue;
      const otherNames = SUPPORTED_LANGUAGES.filter(
        (l) => l.englishName !== name && row.textContent?.includes(l.englishName),
      );
      if (otherNames.length > 0 || switches.length !== 1) break;
      return switches[0]!;
    }
  }
  throw new Error(`no language switch for ${name}`);
}

async function typeInto(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  if (!setter) throw new Error("HTMLInputElement value setter missing");
  await act(async () => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

async function choose(select: HTMLSelectElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
  if (!setter) throw new Error("HTMLSelectElement value setter missing");
  await act(async () => {
    setter.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

/** A click as a user makes it: `click()` does nothing on a disabled control. */
async function press(el: HTMLElement) {
  await act(async () => {
    el.click();
  });
}

const isLocked = (el: HTMLElement) =>
  el.matches(":disabled") || (el instanceof HTMLInputElement && el.readOnly);

/** Typing as a user can: nothing reaches a disabled or read-only field. */
async function attemptType(input: HTMLInputElement, value: string) {
  if (isLocked(input)) return;
  await typeInto(input, value);
}

async function attemptChoose(select: HTMLSelectElement, value: string) {
  if (isLocked(select)) return;
  await choose(select, value);
}

const MODEL_MODE_BUTTONS = ["Use a different model...", "Back to preset models..."];

/** Every settings writer on the page, labelled: fields, model-mode buttons, language switches. */
function writers(root: HTMLElement): Array<{ label: string; el: HTMLElement }> {
  const fields = Array.from(
    root.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>(
      "input, select, textarea",
    ),
  ).map((el) => ({
    label:
      el instanceof HTMLSelectElement
        ? `select offering ${el.options[1]?.value ?? el.options[0]?.value}`
        : `input ${el.name || el.type}`,
    el: el as HTMLElement,
  }));
  const modeButtons = Array.from(root.querySelectorAll("button"))
    .filter((b) => MODEL_MODE_BUTTONS.includes(b.textContent?.trim() ?? ""))
    .map((b) => ({ label: `button ${b.textContent?.trim()}`, el: b as HTMLElement }));
  const switches = OTHER_LANGUAGES.map((l) => ({
    label: `${l.englishName} switch`,
    el: languageToggle(root, l.englishName) as HTMLElement,
  }));
  return [...fields, ...modeButtons, ...switches];
}

/** The writers a user can still use. */
function usableWriters(root: HTMLElement): string[] {
  return writers(root)
    .filter((w) => !isLocked(w.el))
    .map((w) => w.label);
}

function lockedWriters(root: HTMLElement): string[] {
  return writers(root)
    .filter((w) => isLocked(w.el))
    .map((w) => w.label);
}

// ── Pop-ups ──

/** Dismiss every pop-up and wait until each has left the page, so a re-issued one shows anew. */
async function emptyToasts() {
  await act(async () => {
    toast.dismiss();
  });
  await vi.waitFor(
    () => expect(document.querySelectorAll("[data-sonner-toast]")).toHaveLength(0),
    { timeout: 3000 },
  );
  await pause(100);
}

interface Pop {
  type: string | null;
  text: string;
}

function readToast(el: Element): Pop {
  return {
    type: el.getAttribute("data-type"),
    text: (el.querySelector("[data-title]") ?? el).textContent?.trim() ?? "",
  };
}

interface TestPress {
  writes: string[];
  toasts: Pop[];
}

/**
 * Press the Test button of one section with an empty pop-up area, and collect
 * what the press sent and every pop-up it added.
 */
async function pressTest(
  root: HTMLElement,
  api: Api,
  heading: string,
  options: { expectPopUp?: boolean } = {},
): Promise<TestPress> {
  const expectPopUp = options.expectPopUp ?? true;
  await emptyToasts();
  const recorder = recordAddedToasts();
  const from = api.calls.length;
  await press(testButton(root, heading));
  if (expectPopUp) {
    await vi
      .waitFor(() => expect(recorder.added().length).toBeGreaterThan(0), { timeout: 2000 })
      .catch(() => undefined);
  }
  await pause(expectPopUp ? 200 : 400);
  const toasts = recorder.added().map(readToast);
  recorder.stop();
  return { writes: writesSince(api, from), toasts };
}

async function expectSaveFirstEverywhere(root: HTMLElement, api: Api) {
  for (const service of SERVICES) {
    const result = await pressTest(root, api, service.heading);
    expect({
      service: service.heading,
      writes: result.writes,
      toasts: result.toasts.map((t) => t.text),
    }).toEqual({ service: service.heading, writes: [], toasts: [SAVE_FIRST] });
  }
}

async function expectOnePostEverywhere(root: HTMLElement, api: Api) {
  for (const service of SERVICES) {
    const result = await pressTest(root, api, service.heading);
    expect({ service: service.heading, writes: result.writes }).toEqual({
      service: service.heading,
      writes: [`POST ${service.route}`],
    });
    expect(result.toasts.map((t) => t.text)).not.toContain(SAVE_FIRST);
  }
}

const METADATA_PUT = "PUT /config/metadata";
const LANGUAGE_PUT = "PUT /config/default-language";

/** Writes sent from `from` on whose replies the page has not yet read. */
function unreadWritesSince(api: Api, from: number): string[] {
  return api.calls
    .map((c, index) => ({ c, index }))
    .slice(from)
    .filter(({ c, index }) => c.method !== "GET" && !api.handled.has(index))
    .map(({ c }) => `${c.method} ${c.path}`)
    .sort();
}

/** Let React finish the work queued by replies the page has read. */
async function flushReact() {
  for (let turn = 0; turn < 3; turn++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

/**
 * Wait until the page has read every reply sent from `from` on except the
 * `stillHeld` ones, then let React finish handling them.
 */
async function repliesHandled(api: Api, from: number, stillHeld: string[]) {
  await vi.waitFor(() => expect(unreadWritesSince(api, from)).toEqual([...stillHeld].sort()), {
    timeout: 3000,
  });
  await flushReact();
}

/** A save reply held until the test answers it. */
function saveGate(path: string) {
  return { ...hold<Outcome>(), path };
}

async function pressSave(root: HTMLElement, api: Api): Promise<number> {
  const from = api.calls.length;
  await press(saveButton(root));
  await vi.waitFor(() => expect(writesSince(api, from)).toContain("PUT /config/metadata"));
  return from;
}

const BOTH_SAVES = ["PUT /config/default-language", "PUT /config/metadata"].sort();

const sortedWritesSince = (api: Api, from: number) => [...writesSince(api, from)].sort();

/** Save a two-part change and wait until both requests are submitted, in either order. */
async function saveBothParts(root: HTMLElement, api: Api): Promise<number> {
  const from = api.calls.length;
  await press(saveButton(root));
  await vi.waitFor(() => expect(sortedWritesSince(api, from)).toEqual(BOTH_SAVES));
  return from;
}

/** Answer a held save request and wait until the page has handled that reply. */
async function release(
  api: Api,
  from: number,
  gate: { resolve: (o: Outcome) => void },
  outcome: Outcome,
  stillHeld: string[],
) {
  await act(async () => {
    gate.resolve(outcome);
  });
  await repliesHandled(api, from, stillHeld);
}

const secretFields = (root: HTMLElement) => ({
  hardcoverApiToken: field(root, "hardcoverApiToken").value,
  googleBooksApiKey: field(root, "googleBooksApiKey").value,
  llmApiKey: field(root, "llmApiKey").value,
});

const BLANK_SECRETS = { hardcoverApiToken: "", googleBooksApiKey: "", llmApiKey: "" };

// ── Tests ──

describe("Settings -> Metadata Test buttons", () => {
  describe("AC-511: with nothing changed, Test sends one POST and a 200 shows one success pop-up", () => {
    it.each(SERVICES)("$heading", async (service: Service) => {
      const saved = savedSettings();
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      const result = await pressTest(root, api, service.heading);

      expect(result.writes).toEqual([`POST ${service.route}`]);
      expect(result.toasts).toEqual([{ type: "success", text: service.ok }]);
    });
  });

  describe("AC-511: a saved, configured but disabled service is still tested", () => {
    it.each([
      { name: "Hardcover disabled", enabledBox: "hardcoverEnabled", service: SERVICES[0] },
      { name: "AI disabled", enabledBox: "llmEnabled", service: SERVICES[2] },
    ] as const)("$name", async ({ enabledBox, service }) => {
      const saved = savedSettings({ [enabledBox]: false });
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);
      expect(field(root, enabledBox).checked).toBe(false);

      const result = await pressTest(root, api, service.heading);

      expect(result.writes).toEqual([`POST ${service.route}`]);
      expect(result.toasts).toEqual([{ type: "success", text: service.ok }]);
    });
  });

  describe("AC-512: a 400 'not configured' shows one error pop-up with the server's message", () => {
    it("Hardcover with no saved token", async () => {
      const saved = savedSettings({ hardcoverApiTokenSet: false });
      const api = installMetadataApi(saved, {
        test: () => errorReply(400, "bad_request", "Hardcover API token not configured"),
      });
      const root = await mountPage(saved);

      const result = await pressTest(root, api, "Hardcover");

      expect(result.writes).toEqual(["POST /config/metadata/test/hardcover"]);
      expect(result.toasts).toEqual([
        { type: "error", text: "Hardcover API token not configured" },
      ]);
    });

    it("AI connection with no saved model", async () => {
      const saved = savedSettings({
        llmProvider: "custom",
        llmEndpoint: "https://llm.example.test/v1",
        llmModel: null,
      });
      const api = installMetadataApi(saved, {
        test: () => errorReply(400, "bad_request", "LLM model not configured"),
      });
      const root = await mountPage(saved);

      const result = await pressTest(root, api, "LLM Enrichment");

      expect(result.writes).toEqual(["POST /config/metadata/test/llm"]);
      expect(result.toasts).toEqual([{ type: "error", text: "LLM model not configured" }]);
    });
  });

  describe("AC-513: a 502 and a network failure each show one error pop-up", () => {
    const cases = SERVICES.flatMap((service) => [
      {
        name: `${service.heading}, 502`,
        service,
        reply: errorReply(502, "bad_gateway", service.badGateway) as TestReply,
        message: service.badGateway as string,
      },
      {
        name: `${service.heading}, network failure`,
        service,
        reply: "network-error" as TestReply,
        message: "Unable to reach Livrarr",
      },
    ]);

    it.each(cases)("$name", async ({ service, reply, message }) => {
      const saved = savedSettings();
      const api = installMetadataApi(saved, { test: () => reply });
      const root = await mountPage(saved);

      const result = await pressTest(root, api, service.heading);

      expect(result.writes).toEqual([`POST ${service.route}`]);
      expect(result.toasts).toEqual([{ type: "error", text: message }]);
    });
  });

  describe("AC-514: after any change, every Test sends nothing and asks to save first", () => {
    const changes: Array<{
      name: string;
      change: (root: HTMLElement) => Promise<void>;
      took: (root: HTMLElement) => void;
    }> = [
      {
        name: "typing the Audnexus URL",
        change: (r) => typeInto(field(r, "audnexusUrl"), ENTERED_AUDNEXUS),
        took: (r) => expect(field(r, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS),
      },
      {
        name: "typing a Hardcover token",
        change: (r) => typeInto(field(r, "hardcoverApiToken"), "typed-token"),
        took: (r) => expect(field(r, "hardcoverApiToken").value).toBe("typed-token"),
      },
      {
        name: "typing a Google Books key",
        change: (r) => typeInto(field(r, "googleBooksApiKey"), "typed-key"),
        took: (r) => expect(field(r, "googleBooksApiKey").value).toBe("typed-key"),
      },
      {
        name: "typing the AI endpoint",
        change: (r) => typeInto(field(r, "llmEndpoint"), "https://llm.example.test/v1"),
        took: (r) => expect(field(r, "llmEndpoint").value).toBe("https://llm.example.test/v1"),
      },
      {
        name: "typing an AI key",
        change: (r) => typeInto(field(r, "llmApiKey"), "typed-ai-key"),
        took: (r) => expect(field(r, "llmApiKey").value).toBe("typed-ai-key"),
      },
      {
        name: "unticking Hardcover Enabled",
        change: (r) => press(field(r, "hardcoverEnabled")),
        took: (r) => expect(field(r, "hardcoverEnabled").checked).toBe(false),
      },
      {
        name: "unticking AI Enabled",
        change: (r) => press(field(r, "llmEnabled")),
        took: (r) => expect(field(r, "llmEnabled").checked).toBe(false),
      },
      {
        name: "choosing another preset model",
        change: (r) => choose(modelSelect(r), GROQ_SECOND_MODEL),
        took: (r) => expect(modelSelect(r).value).toBe(GROQ_SECOND_MODEL),
      },
      {
        name: "adding a language",
        change: (r) => press(languageToggle(r, "French")),
        took: () => undefined,
      },
      {
        name: "changing the default language",
        change: (r) => choose(defaultLanguageSelect(r), "fr"),
        took: (r) => expect(defaultLanguageSelect(r).value).toBe("fr"),
      },
    ];

    it.each(changes)("$name", async ({ change, took }) => {
      const saved = savedSettings();
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      await change(root);
      took(root);

      await expectSaveFirstEverywhere(root, api);
    });
  });

  describe("AC-515: a change returned to the saved value lets Test send its POST", () => {
    const roundTrips: Array<{ name: string; there: (r: HTMLElement) => Promise<void>; back: (r: HTMLElement) => Promise<void> }> = [
      {
        name: "Audnexus URL typed and typed back",
        there: (r) => typeInto(field(r, "audnexusUrl"), ENTERED_AUDNEXUS),
        back: (r) => typeInto(field(r, "audnexusUrl"), SAVED_AUDNEXUS),
      },
      {
        name: "Hardcover token typed and cleared",
        there: (r) => typeInto(field(r, "hardcoverApiToken"), "typed-token"),
        back: (r) => typeInto(field(r, "hardcoverApiToken"), ""),
      },
      {
        name: "Hardcover Enabled unticked and ticked",
        there: (r) => press(field(r, "hardcoverEnabled")),
        back: (r) => press(field(r, "hardcoverEnabled")),
      },
    ];

    it.each(roundTrips)("$name", async ({ there, back }) => {
      const saved = savedSettings();
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      await there(root);
      await back(root);
      expect(field(root, "audnexusUrl").value).toBe(SAVED_AUDNEXUS);
      expect(field(root, "hardcoverApiToken").value).toBe("");
      expect(field(root, "hardcoverEnabled").checked).toBe(true);

      await expectOnePostEverywhere(root, api);
    });
  });

  describe("AC-516: after a successful save the page is clean", () => {
    it("a typed Hardcover token is blank afterwards and Test sends one POST", async () => {
      const saved = savedSettings({ hardcoverApiTokenSet: false });
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      await typeInto(field(root, "hardcoverApiToken"), "new-token");
      const from = await pressSave(root, api);
      await repliesHandled(api, from, []);
      const put = api.calls.slice(from).find((c) => c.method === "PUT");
      expect((put?.body as UpdateMetadataConfigRequest).hardcoverApiToken).toBe("new-token");

      await vi
        .waitFor(() => expect(field(root, "hardcoverApiToken").value).toBe(""), { timeout: 1500 })
        .catch(() => undefined);
      expect(field(root, "hardcoverApiToken").value).toBe("");

      await expectOnePostEverywhere(root, api);
    });

    it("a saved language change: Test sends one POST", async () => {
      const saved = savedSettings();
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      await press(languageToggle(root, "French"));
      const from = await pressSave(root, api);
      await repliesHandled(api, from, []);
      const put = api.calls.slice(from).find((c) => c.method === "PUT");
      expect((put?.body as UpdateMetadataConfigRequest).languages).toEqual(["en", "fr"]);

      await expectOnePostEverywhere(root, api);
    });
  });

  describe("AC-516: an already-set secret replaced and saved is blank afterwards", () => {
    it.each([
      { name: "hardcoverApiToken" as const },
      { name: "googleBooksApiKey" as const },
      { name: "llmApiKey" as const },
    ])("$name", async ({ name }) => {
      const saved = savedSettings();
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      const replacement = `replacement-${name}`;
      await typeInto(field(root, name), replacement);
      const from = await pressSave(root, api);
      await repliesHandled(api, from, []);
      const put = api.calls.slice(from).find((c) => c.method === "PUT");
      expect((put?.body as UpdateMetadataConfigRequest)[name]).toBe(replacement);

      await vi
        .waitFor(() => expect(field(root, name).value).toBe(""), { timeout: 1500 })
        .catch(() => undefined);
      expect(field(root, name).value).toBe("");

      await expectOnePostEverywhere(root, api);
    });
  });

  it("AC-517: a save whose service part fails keeps the entered values and Test asks to save first", async () => {
    const saved = savedSettings();
    const api = installMetadataApi(saved, { metadataPut: () => "fail" });
    const root = await mountPage(saved);

    await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
    await typeInto(field(root, "hardcoverApiToken"), "typed-token");
    const from = await pressSave(root, api);
    await repliesHandled(api, from, []);
    expect(writesSince(api, from)).toEqual(["PUT /config/metadata"]);

    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
    expect(field(root, "hardcoverApiToken").value).toBe("typed-token");

    await expectSaveFirstEverywhere(root, api);
  });

  describe("AC-518: while a test is pending its button reads Testing... and is disabled", () => {
    it.each(SERVICES)("$heading", async (service: Service) => {
      const saved = savedSettings();
      const reply = hold<TestReply>();
      const api = installMetadataApi(saved, { test: () => reply.promise });
      const root = await mountPage(saved);

      const from = api.calls.length;
      await press(testButton(root, service.heading));
      await vi.waitFor(() => expect(writesSince(api, from)).toEqual([`POST ${service.route}`]));
      await vi
        .waitFor(() => expect(testButton(root, service.heading).textContent?.trim()).toBe("Testing..."))
        .catch(() => undefined);

      const pending = testButton(root, service.heading);
      expect(pending.textContent?.trim()).toBe("Testing...");
      expect(pending.disabled).toBe(true);
      await press(pending);
      await pause(50);
      expect(writesSince(api, from)).toEqual([`POST ${service.route}`]);

      await act(async () => {
        reply.resolve({ status: 200 });
      });
      await vi.waitFor(() => {
        const after = testButton(root, service.heading);
        expect(after.textContent?.trim()).toBe("Test");
        expect(after.disabled).toBe(false);
      });
      expect(writesSince(api, from)).toEqual([`POST ${service.route}`]);
    });
  });

  it("AC-519: changing only the AI Provider makes every Test ask to save first", async () => {
    const saved = savedSettings();
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    await choose(providerSelect(root), "openai");
    expect(providerSelect(root).value).toBe("openai");
    expect(field(root, "llmEndpoint").value).toBe(OPENAI_ENDPOINT);

    await expectSaveFirstEverywhere(root, api);
  });

  it("AC-520: Back to preset models from a saved custom model makes every Test ask to save first", async () => {
    const saved = savedSettings({ llmModel: "my-own-model" });
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    expect(field(root, "llmModel").value).toBe("my-own-model");
    await press(buttonByText(root, "Back to preset models..."));
    expect(modelSelect(root).value).toBe(GROQ_FIRST_MODEL);

    await expectSaveFirstEverywhere(root, api);
  });

  it("AC-514/AC-515: the custom-model switch alone changes nothing; typing a model does until typed back", async () => {
    const saved = savedSettings();
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    await press(buttonByText(root, "Use a different model..."));
    expect(field(root, "llmModel").value).toBe(GROQ_FIRST_MODEL);
    const switched = await pressTest(root, api, "LLM Enrichment");
    expect(switched.writes).toEqual(["POST /config/metadata/test/llm"]);
    expect(switched.toasts.map((t) => t.text)).not.toContain(SAVE_FIRST);

    await typeInto(field(root, "llmModel"), "my-own-model");
    expect(field(root, "llmModel").value).toBe("my-own-model");
    const typed = await pressTest(root, api, "LLM Enrichment");
    expect(typed.writes).toEqual([]);
    expect(typed.toasts.map((t) => t.text)).toEqual([SAVE_FIRST]);

    await typeInto(field(root, "llmModel"), GROQ_FIRST_MODEL);
    const restored = await pressTest(root, api, "LLM Enrichment");
    expect(restored.writes).toEqual(["POST /config/metadata/test/llm"]);
    expect(restored.toasts.map((t) => t.text)).not.toContain(SAVE_FIRST);
  });

  it("AC-521: provider, endpoint and model returned to their saved values let Test send its POST", async () => {
    const saved = savedSettings({ llmModel: GROQ_SECOND_MODEL });
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    await choose(providerSelect(root), "openai");
    const changed = await pressTest(root, api, "LLM Enrichment");
    expect(changed.writes).toEqual([]);
    expect(changed.toasts.map((t) => t.text)).toEqual([SAVE_FIRST]);

    await choose(providerSelect(root), "groq");
    await choose(modelSelect(root), GROQ_SECOND_MODEL);
    expect(providerSelect(root).value).toBe("groq");
    expect(field(root, "llmEndpoint").value).toBe(GROQ_ENDPOINT);
    expect(modelSelect(root).value).toBe(GROQ_SECOND_MODEL);

    await expectOnePostEverywhere(root, api);
  });

  describe("AC-522 and AC-523: a two-part save is saved part by part", () => {
    it("(a) service part saved, default language held then failed; the retry saves it", async () => {
      const saved = savedSettings();
      const languageGate = saveGate(LANGUAGE_PUT);
      const api = installMetadataApi(saved, { defaultLanguagePut: () => languageGate.promise });
      const root = await mountPage(saved);

      await typeInto(field(root, "hardcoverApiToken"), "typed-token");
      await choose(defaultLanguageSelect(root), "fr");
      const from = await saveBothParts(root, api);
      await repliesHandled(api, from, [LANGUAGE_PUT]);

      // Default language held: the service part shows its saved values.
      expect(field(root, "hardcoverApiToken").value).toBe("");
      expect(defaultLanguageSelect(root).value).toBe("fr");
      const whileHeld = await pressTest(root, api, "Hardcover", { expectPopUp: false });
      expect(whileHeld.writes).toEqual([]);

      await release(api, from, languageGate, "fail", []);
      expect(sortedWritesSince(api, from)).toEqual(BOTH_SAVES);
      expect(defaultLanguageSelect(root).value).toBe("fr");
      expect(field(root, "hardcoverApiToken").value).toBe("");
      await expectSaveFirstEverywhere(root, api);

      // AC-523: Save again with the default language answered 200.
      api.script.defaultLanguagePut = () => "ok";
      const retry = await pressSave(root, api);
      await vi.waitFor(() =>
        expect(writesSince(api, retry)).toContain("PUT /config/default-language"),
      );
      await repliesHandled(api, retry, []);
      expect(saved.defaultLanguage).toBe("fr");
      expect(defaultLanguageSelect(root).value).toBe("fr");
      expect(secretFields(root)).toEqual(BLANK_SECRETS);
      await expectOnePostEverywhere(root, api);
    });

    it("(b) default language saved, service part held then failed; the retry saves it", async () => {
      const saved = savedSettings({ hardcoverApiTokenSet: false });
      const metadataGate = saveGate(METADATA_PUT);
      const api = installMetadataApi(saved, { metadataPut: () => metadataGate.promise });
      const root = await mountPage(saved);

      await typeInto(field(root, "hardcoverApiToken"), "typed-token");
      await choose(defaultLanguageSelect(root), "fr");
      const from = await saveBothParts(root, api);
      await repliesHandled(api, from, [METADATA_PUT]);
      expect(saved.defaultLanguage).toBe("fr");

      // Service part held: its entered values stay; the default language shows its saved value.
      expect(field(root, "hardcoverApiToken").value).toBe("typed-token");
      expect(defaultLanguageSelect(root).value).toBe("fr");
      const whileHeld = await pressTest(root, api, "Hardcover", { expectPopUp: false });
      expect(whileHeld.writes).toEqual([]);

      await release(api, from, metadataGate, "fail", []);
      expect(sortedWritesSince(api, from)).toEqual(BOTH_SAVES);
      expect(field(root, "hardcoverApiToken").value).toBe("typed-token");
      expect(defaultLanguageSelect(root).value).toBe("fr");
      await expectSaveFirstEverywhere(root, api);

      // AC-523: Save again with the service part answered 200.
      api.script.metadataPut = () => "ok";
      const retry = await pressSave(root, api);
      await repliesHandled(api, retry, []);
      expect(saved.config.hardcoverApiTokenSet).toBe(true);
      await vi
        .waitFor(() => expect(secretFields(root)).toEqual(BLANK_SECRETS), { timeout: 1500 })
        .catch(() => undefined);
      expect(secretFields(root)).toEqual(BLANK_SECRETS);
      expect(defaultLanguageSelect(root).value).toBe("fr");
      await expectOnePostEverywhere(root, api);
    });
  });

  it("AC-522 (b): the saved default language is clean on its own once the failed service edit is undone", async () => {
    const saved = savedSettings({ hardcoverApiTokenSet: false });
    const metadataGate = saveGate(METADATA_PUT);
    const api = installMetadataApi(saved, { metadataPut: () => metadataGate.promise });
    const root = await mountPage(saved);

    await typeInto(field(root, "hardcoverApiToken"), "typed-token");
    await choose(defaultLanguageSelect(root), "fr");
    const from = await saveBothParts(root, api);
    await repliesHandled(api, from, [METADATA_PUT]);
    await release(api, from, metadataGate, "fail", []);
    expect(sortedWritesSince(api, from)).toEqual(BOTH_SAVES);
    expect(saved.defaultLanguage).toBe("fr");
    expect(field(root, "hardcoverApiToken").value).toBe("typed-token");

    await typeInto(field(root, "hardcoverApiToken"), "");
    expect(field(root, "hardcoverApiToken").value).toBe("");
    expect(defaultLanguageSelect(root).value).toBe("fr");
    const clean = await pressTest(root, api, "Hardcover");
    expect(clean.writes).toEqual(["POST /config/metadata/test/hardcover"]);
    expect(clean.toasts.map((t) => t.text)).not.toContain(SAVE_FIRST);

    await choose(defaultLanguageSelect(root), "en");
    expect(defaultLanguageSelect(root).value).toBe("en");
    const changed = await pressTest(root, api, "Hardcover");
    expect(changed.writes).toEqual([]);
    expect(changed.toasts.map((t) => t.text)).toEqual([SAVE_FIRST]);
  });

  describe("AC-524: every writer is disabled while both writes of a save are held", () => {
    it.each([
      { name: "(a) both answered 200", languageOutcome: "ok" as Outcome },
      { name: "(b) metadata 200, default language 500", languageOutcome: "fail" as Outcome },
    ])("$name", async ({ languageOutcome }) => {
      const saved = savedSettings();
      const metadataGate = saveGate(METADATA_PUT);
      const languageGate = saveGate(LANGUAGE_PUT);
      const api = installMetadataApi(saved, {
        metadataPut: () => metadataGate.promise,
        defaultLanguagePut: () => languageGate.promise,
      });
      const root = await mountPage(saved);
      expect(lockedWriters(root)).toEqual([]);

      await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
      await choose(defaultLanguageSelect(root), "fr");
      const from = await saveBothParts(root, api);
      await repliesHandled(api, from, BOTH_SAVES);

      expect(usableWriters(root)).toEqual([]);
      await attemptType(field(root, "audnexusUrl"), ATTEMPTED_AUDNEXUS);
      await press(languageToggle(root, "German"));
      await attemptChoose(defaultLanguageSelect(root), "de");
      expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
      expect(defaultLanguageSelect(root).value).toBe("fr");

      await release(api, from, metadataGate, "ok", [LANGUAGE_PUT]);
      await release(api, from, languageGate, languageOutcome, []);
      await vi
        .waitFor(() => expect(lockedWriters(root)).toEqual([]), { timeout: 1500 })
        .catch(() => undefined);
      expect(lockedWriters(root)).toEqual([]);
    });
  });

  describe("AC-527: the save lock holds while any submitted write is pending", () => {
    async function attemptEdits(root: HTMLElement, chosenDefault: string) {
      expect(usableWriters(root)).toEqual([]);
      await attemptType(field(root, "audnexusUrl"), ATTEMPTED_AUDNEXUS);
      await press(languageToggle(root, "German"));
      await attemptChoose(defaultLanguageSelect(root), "de");
      expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
      expect(defaultLanguageSelect(root).value).toBe(chosenDefault);
    }

    async function expectUnlocked(root: HTMLElement) {
      await vi
        .waitFor(() => expect(lockedWriters(root)).toEqual([]), { timeout: 1500 })
        .catch(() => undefined);
      expect(lockedWriters(root)).toEqual([]);
      expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
    }

    it("(a) a service-only save with its one PUT held", async () => {
      const saved = savedSettings({ llmModel: "my-own-model" });
      const metadataGate = saveGate(METADATA_PUT);
      const api = installMetadataApi(saved, { metadataPut: () => metadataGate.promise });
      const root = await mountPage(saved);
      expect(buttonByText(root, "Back to preset models...")).toBeTruthy();
      expect(lockedWriters(root)).toEqual([]);

      await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
      const from = await pressSave(root, api);
      await repliesHandled(api, from, [METADATA_PUT]);
      expect(writesSince(api, from)).toEqual(["PUT /config/metadata"]);

      await attemptEdits(root, "en");

      await release(api, from, metadataGate, "ok", []);
      await expectUnlocked(root);
    });

    it.each([
      { name: "(b) default language answered 200 first, metadata held", first: "language", outcome: "ok" },
      { name: "(b) default language answered 500 first, metadata held", first: "language", outcome: "fail" },
      { name: "(c) metadata answered 200 first, default language held", first: "metadata", outcome: "ok" },
      { name: "(c) metadata answered 500 first, default language held", first: "metadata", outcome: "fail" },
    ] as Array<{ name: string; first: "language" | "metadata"; outcome: Outcome }>)(
      "$name",
      async ({ first, outcome }) => {
        const saved = savedSettings({ llmModel: "my-own-model" });
        const metadataGate = saveGate(METADATA_PUT);
        const languageGate = saveGate(LANGUAGE_PUT);
        const api = installMetadataApi(saved, {
          metadataPut: () => metadataGate.promise,
          defaultLanguagePut: () => languageGate.promise,
        });
        const root = await mountPage(saved);
        expect(lockedWriters(root)).toEqual([]);

        await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
        await choose(defaultLanguageSelect(root), "fr");
        const from = await saveBothParts(root, api);
        await repliesHandled(api, from, BOTH_SAVES);

        const [answered, last] =
          first === "language" ? [languageGate, metadataGate] : [metadataGate, languageGate];
        await release(api, from, answered, outcome, [last.path]);

        await attemptEdits(root, "fr");

        await release(api, from, last, "ok", []);
        expect(sortedWritesSince(api, from)).toEqual(BOTH_SAVES);
        await expectUnlocked(root);
      },
    );
  });

  describe("AC-525: the language list counts as changed only when it differs from the saved list", () => {
    it("enabling French and disabling it again leaves no unsaved change", async () => {
      const saved = savedSettings({ languages: ["en"] });
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      await press(languageToggle(root, "French"));
      const changed = await pressTest(root, api, "Hardcover");
      expect(changed.writes).toEqual([]);
      expect(changed.toasts.map((t) => t.text)).toEqual([SAVE_FIRST]);

      await press(languageToggle(root, "French"));
      const restored = await pressTest(root, api, "Hardcover");
      expect(restored.writes).toEqual(["POST /config/metadata/test/hardcover"]);
      expect(restored.toasts.map((t) => t.text)).not.toContain(SAVE_FIRST);
    });

    it("the same languages in a new order still count as a change", async () => {
      const saved = savedSettings({ languages: ["en", "fr", "de"] });
      const api = installMetadataApi(saved);
      const root = await mountPage(saved);

      await press(languageToggle(root, "French"));
      await press(languageToggle(root, "French"));
      const reordered = await pressTest(root, api, "Hardcover");
      expect(reordered.writes).toEqual([]);
      expect(reordered.toasts.map((t) => t.text)).toEqual([SAVE_FIRST]);
    });
  });

  it("AC-526: the default language changed and changed back leaves no unsaved change", async () => {
    const saved = savedSettings();
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    await choose(defaultLanguageSelect(root), "fr");
    expect(defaultLanguageSelect(root).value).toBe("fr");
    const changed = await pressTest(root, api, "Hardcover");
    expect(changed.writes).toEqual([]);
    expect(changed.toasts.map((t) => t.text)).toEqual([SAVE_FIRST]);

    await choose(defaultLanguageSelect(root), "en");
    expect(defaultLanguageSelect(root).value).toBe("en");
    const restored = await pressTest(root, api, "Hardcover");
    expect(restored.writes).toEqual(["POST /config/metadata/test/hardcover"]);
    expect(restored.toasts.map((t) => t.text)).not.toContain(SAVE_FIRST);
  });
});

// ── A settings read that started before a save's reply ──

/**
 * Hold the reply to the next GET of `path`. The stub builds the reply body
 * when the request is sent, so the held reply carries the value saved at
 * that moment.
 */
function holdNextRead(path: string) {
  const gate = hold<void>();
  let issued = false;
  const inner = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    const method = (init?.method ?? "GET").toUpperCase();
    const matches = !issued && method === "GET" && url === `/api/v1${path}`;
    if (matches) issued = true;
    const response = await inner(input, init);
    if (matches) await gate.promise;
    return response;
  }) as typeof globalThis.fetch;
  cleanups.push(() => {
    gate.resolve();
    globalThis.fetch = inner;
  });
  return {
    issued: () => issued,
    release: async () => {
      await act(async () => {
        gate.resolve();
      });
      await flushReact();
      await pause(50);
    },
  };
}

/** The browser tab regains focus, which refetches the page's settings reads. */
async function windowRefocused() {
  await act(async () => {
    window.dispatchEvent(new Event("visibilitychange"));
  });
}

/** Start a background read of `path` and hold its reply, which carries the pre-save value. */
async function olderReadInFlight(path: string) {
  const read = holdNextRead(path);
  await windowRefocused();
  await vi.waitFor(() => expect(read.issued()).toBe(true));
  return read;
}

describe("A settings read sent before a save's reply never replaces that reply", () => {
  it("service settings: the older read answered after the save keeps the saved Audnexus URL", async () => {
    const saved = savedSettings();
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
    const read = await olderReadInFlight("/config/metadata");
    const from = await pressSave(root, api);
    await repliesHandled(api, from, []);
    expect(saved.config.audnexusUrl).toBe(ENTERED_AUDNEXUS);
    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);

    await read.release();

    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
    await expectOnePostEverywhere(root, api);
  });

  it("service settings: the older read answered before the save's reply leaves the saved Audnexus URL", async () => {
    const saved = savedSettings();
    const metadataGate = saveGate(METADATA_PUT);
    const api = installMetadataApi(saved, { metadataPut: () => metadataGate.promise });
    const root = await mountPage(saved);

    await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
    const read = await olderReadInFlight("/config/metadata");
    const from = await pressSave(root, api);
    await repliesHandled(api, from, [METADATA_PUT]);

    await read.release();
    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);

    await release(api, from, metadataGate, "ok", []);
    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
    await expectOnePostEverywhere(root, api);
  });

  it("service settings saved, default language held then failed: the older read keeps both", async () => {
    const saved = savedSettings();
    const languageGate = saveGate(LANGUAGE_PUT);
    const api = installMetadataApi(saved, { defaultLanguagePut: () => languageGate.promise });
    const root = await mountPage(saved);

    await typeInto(field(root, "audnexusUrl"), ENTERED_AUDNEXUS);
    await choose(defaultLanguageSelect(root), "fr");
    const read = await olderReadInFlight("/config/metadata");
    const from = await saveBothParts(root, api);
    await repliesHandled(api, from, [LANGUAGE_PUT]);

    await read.release();
    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
    expect(defaultLanguageSelect(root).value).toBe("fr");
    const whileHeld = await pressTest(root, api, "Hardcover", { expectPopUp: false });
    expect(whileHeld.writes).toEqual([]);

    await release(api, from, languageGate, "fail", []);
    expect(field(root, "audnexusUrl").value).toBe(ENTERED_AUDNEXUS);
    expect(defaultLanguageSelect(root).value).toBe("fr");
    await expectSaveFirstEverywhere(root, api);

    await choose(defaultLanguageSelect(root), "en");
    await expectOnePostEverywhere(root, api);
  });

  it("default language: the older read answered after the save keeps the saved default", async () => {
    const saved = savedSettings();
    const api = installMetadataApi(saved);
    const root = await mountPage(saved);

    await choose(defaultLanguageSelect(root), "fr");
    const read = await olderReadInFlight("/config/default-language");
    const from = await saveBothParts(root, api);
    await repliesHandled(api, from, []);
    expect(saved.defaultLanguage).toBe("fr");
    expect(defaultLanguageSelect(root).value).toBe("fr");

    await read.release();

    expect(defaultLanguageSelect(root).value).toBe("fr");
    await expectOnePostEverywhere(root, api);
  });

  it("default language: the older read answered before the save's reply leaves the saved default", async () => {
    const saved = savedSettings();
    const languageGate = saveGate(LANGUAGE_PUT);
    const api = installMetadataApi(saved, { defaultLanguagePut: () => languageGate.promise });
    const root = await mountPage(saved);

    await choose(defaultLanguageSelect(root), "fr");
    const read = await olderReadInFlight("/config/default-language");
    const from = await saveBothParts(root, api);
    await repliesHandled(api, from, [LANGUAGE_PUT]);

    await read.release();
    expect(defaultLanguageSelect(root).value).toBe("fr");

    await release(api, from, languageGate, "ok", []);
    expect(defaultLanguageSelect(root).value).toBe("fr");
    await expectOnePostEverywhere(root, api);
  });

  it("default language saved, service settings held then failed: the older read keeps both", async () => {
    const saved = savedSettings({ hardcoverApiTokenSet: false });
    const metadataGate = saveGate(METADATA_PUT);
    const api = installMetadataApi(saved, { metadataPut: () => metadataGate.promise });
    const root = await mountPage(saved);

    await typeInto(field(root, "hardcoverApiToken"), "typed-token");
    await choose(defaultLanguageSelect(root), "fr");
    const read = await olderReadInFlight("/config/default-language");
    const from = await saveBothParts(root, api);
    await repliesHandled(api, from, [METADATA_PUT]);
    expect(saved.defaultLanguage).toBe("fr");

    await read.release();
    expect(defaultLanguageSelect(root).value).toBe("fr");
    expect(field(root, "hardcoverApiToken").value).toBe("typed-token");
    const whileHeld = await pressTest(root, api, "Hardcover", { expectPopUp: false });
    expect(whileHeld.writes).toEqual([]);

    await release(api, from, metadataGate, "fail", []);
    expect(defaultLanguageSelect(root).value).toBe("fr");
    expect(field(root, "hardcoverApiToken").value).toBe("typed-token");
    await expectSaveFirstEverywhere(root, api);

    await typeInto(field(root, "hardcoverApiToken"), "");
    await expectOnePostEverywhere(root, api);
  });
});
