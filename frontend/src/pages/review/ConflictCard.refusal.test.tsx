import { afterEach, describe, expect, it, vi } from "vitest";
import { Toaster, toast } from "sonner";
import ReviewPage from "@/pages/review/ReviewPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";

type ConflictFixture = "legacy" | "typed-compatibility";
type Control =
  | { label: "Keep Existing"; action: "keep_existing" }
  | { label: "Treat as Separate"; action: "accept_separate" }
  | { label: "Use New Match"; action: "replace_anchor" }
  | { label: "Combine Both"; action: "merge" }
  | { label: "Dismiss"; action: null };

const CONTROLS: Control[] = [
  { label: "Keep Existing", action: "keep_existing" },
  { label: "Treat as Separate", action: "accept_separate" },
  { label: "Use New Match", action: "replace_anchor" },
  { label: "Combine Both", action: "merge" },
  { label: "Dismiss", action: null },
];

const serverMessage =
  "continuation unavailable for IdentityConflict; handled by wave B";

afterEach(() => {
  toast.dismiss();
  vi.restoreAllMocks();
});

function conflictSummary(id: number) {
  return {
    id,
    existingWorkId: 42,
    kind: "incoming_different_ol_key",
    incomingTitle: "The Incoming Match",
    incomingAuthor: "The Incoming Author",
    incomingOlKey: "OL-U1-INCOMING-W",
    raisedAt: "2026-08-23T00:00:00Z",
    raisedBy: "manual_add",
    status: "open",
  };
}

function isQueryKey(call: unknown[], key: string): boolean {
  const filters = call[0];
  if (!filters || typeof filters !== "object" || !("queryKey" in filters)) {
    return false;
  }
  const queryKey = (filters as { queryKey?: unknown }).queryKey;
  return Array.isArray(queryKey) && queryKey[0] === key;
}

async function assertConflictCardRefusal(
  fixture: ConflictFixture,
  control: Control,
) {
  // AC-001 compatibility justification: the typed variant has no production
  // mint door today. Both fixtures deliberately feed the same production
  // ConflictCard DTO because the component must handle the server refusal
  // identically regardless of which backend store proved the id exists.
  const conflictId = fixture === "legacy" ? 701 : 801;
  const api = installApiStub((call: ApiCall) => {
    if (call.method === "GET" && call.path === "/identity-review") {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path === "/identity-review-card") {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path === "/author-link-review") {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path === "/identity-conflict") {
      return { status: 200, body: [conflictSummary(conflictId)] };
    }
    if (call.method === "GET" && call.path === "/work/42") {
      return { status: 200, body: { id: 42, title: "The Existing Book" } };
    }
    if (
      control.action !== null &&
      call.method === "POST" &&
      call.path === `/identity-conflict/${conflictId}/resolve`
    ) {
      expect(call.body).toEqual({ action: control.action });
      if (control.action === "accept_separate") {
        expect(call.body).not.toHaveProperty("winningWorkId");
      }
      return {
        status: 409,
        body: { status: 409, error: "conflict", message: serverMessage },
      };
    }
    if (
      control.action === null &&
      call.method === "POST" &&
      call.path === `/identity-conflict/${conflictId}/dismiss`
    ) {
      expect(call.body).toBeUndefined();
      return {
        status: 409,
        body: { status: 409, error: "conflict", message: serverMessage },
      };
    }
    throw new Error(`unexpected call ${call.method} ${call.path}`);
  });

  const client = newTestClient();
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const mounted = mountWith(
    client,
    <>
      <ReviewPage />
      <Toaster richColors position="bottom-center" />
    </>,
  );
  try {
    await vi.waitFor(
      () => expect(mounted.container.textContent).toContain("The Incoming Match"),
      { timeout: 5000 },
    );
    await clickButton(mounted.container, control.label);

    // The production ApiError preserves the 409 message; ConflictCard must
    // render that message rather than replacing it with a fixed fallback.
    await vi.waitFor(
      () => expect(document.body.textContent).toContain(serverMessage),
      { timeout: 5000 },
    );
    expect(document.body.textContent).toContain("IdentityConflict");

    // The row stays open/actionable after the rejected mutation.
    await vi.waitFor(() => {
      for (const candidate of CONTROLS) {
        const button = Array.from(
          mounted.container.querySelectorAll<HTMLButtonElement>("button"),
        ).find((item) => item.textContent?.includes(candidate.label));
        expect(button, candidate.label).toBeDefined();
        expect(button?.disabled, candidate.label).toBe(false);
      }
    });

    expect(document.body.textContent).not.toContain("Conflict resolved");
    expect(document.body.textContent).not.toContain("Conflict dismissed");
    expect(invalidate.mock.calls.some((call) => isQueryKey(call, "works"))).toBe(
      false,
    );
    expect(
      invalidate.mock.calls.some((call) => isQueryKey(call, "identity-conflicts")),
    ).toBe(false);

    const writes = api.calls.filter((call) => call.method !== "GET");
    expect(writes).toHaveLength(1);
  } finally {
    mounted.cleanup();
    api.restore();
  }
}

describe("ConflictCard — REQ-001 continuation refusal", () => {
  // RED-UNTIL-U1: today the legacy Keep Existing failure hides the server message behind a fixed toast.
  it("shows the legacy Keep Existing IdentityConflict refusal", async () => {
    await assertConflictCardRefusal("legacy", CONTROLS[0]!);
  });

  // RED-UNTIL-U1: today legacy Treat as Separate sends no winningWorkId and its server failure is hidden.
  it("shows the legacy Treat as Separate IdentityConflict refusal", async () => {
    await assertConflictCardRefusal("legacy", CONTROLS[1]!);
  });

  // RED-UNTIL-U1: today the legacy Use New Match failure hides the server message behind a fixed toast.
  it("shows the legacy Use New Match IdentityConflict refusal", async () => {
    await assertConflictCardRefusal("legacy", CONTROLS[2]!);
  });

  // RED-UNTIL-U1: today the legacy Combine Both failure hides the server message behind a fixed toast.
  it("shows the legacy Combine Both IdentityConflict refusal", async () => {
    await assertConflictCardRefusal("legacy", CONTROLS[3]!);
  });

  // RED-UNTIL-U1: today the legacy Dismiss failure hides the server message behind a fixed toast.
  it("shows the legacy Dismiss IdentityConflict refusal", async () => {
    await assertConflictCardRefusal("legacy", CONTROLS[4]!);
  });

  // RED-UNTIL-U1: today typed Keep Existing fabricates success and the component has no refusal presentation.
  it("shows the typed compatibility Keep Existing refusal", async () => {
    await assertConflictCardRefusal("typed-compatibility", CONTROLS[0]!);
  });

  // RED-UNTIL-U1: today typed Treat as Separate fails before the shared guard and the component hides its message.
  it("shows the typed compatibility Treat as Separate refusal", async () => {
    await assertConflictCardRefusal("typed-compatibility", CONTROLS[1]!);
  });

  // RED-UNTIL-U1: today typed Use New Match fabricates success and the component has no refusal presentation.
  it("shows the typed compatibility Use New Match refusal", async () => {
    await assertConflictCardRefusal("typed-compatibility", CONTROLS[2]!);
  });

  // RED-UNTIL-U1: today typed Combine Both fabricates success and the component has no refusal presentation.
  it("shows the typed compatibility Combine Both refusal", async () => {
    await assertConflictCardRefusal("typed-compatibility", CONTROLS[3]!);
  });

  // RED-UNTIL-U1: today typed Dismiss fabricates success and the component has no refusal presentation.
  it("shows the typed compatibility Dismiss refusal", async () => {
    await assertConflictCardRefusal("typed-compatibility", CONTROLS[4]!);
  });
});
