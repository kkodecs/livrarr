import { afterEach, describe, expect, it, vi } from "vitest";
import ReviewPage from "@/pages/review/ReviewPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";

const refusedCopy =
  "not yet actionable — EditionEvidence; handled by its post-wave-B continuation feature";
const durableDismissCopy =
  "Dismiss prevents this evidence question from returning";

afterEach(() => {
  vi.restoreAllMocks();
});

function editionEvidenceCard() {
  return {
    id: 707,
    userId: 7,
    workId: 1707,
    workTitle: "U7 Evidence Book",
    workAuthor: "U7 Evidence Author",
    kind: "EditionEvidence",
    generation: 11,
    payload: {
      EditionEvidence: {
        edition_id: 2707,
        evidence_ids: [],
      },
    },
  };
}

function installReviewPageApi() {
  return installApiStub((call: ApiCall) => {
    if (call.method === "GET" && call.path === "/identity-review") {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path === "/identity-review-card") {
      return { status: 200, body: [editionEvidenceCard()] };
    }
    if (call.method === "GET" && call.path === "/author-link-review") {
      return { status: 200, body: [] };
    }
    if (
      call.method === "POST" &&
      call.path === "/identity-review-card/707/dismiss"
    ) {
      expect(call.body).toBeUndefined();
      return { status: 204, body: undefined };
    }
    throw new Error(`unexpected call ${call.method} ${call.path}`);
  });
}

describe("ReviewPage — REQ-007 refused card copy", () => {
  // RED-UNTIL-U7: today EditionEvidence falls through to the generic “identity needs your review” sentence; Dismiss is present and Resolve is already absent, but the owner and durable-Dismiss copy do not render.
  it("renders exact refused EditionEvidence ownership and durable Dismiss copy", async () => {
    const api = installReviewPageApi();
    const client = newTestClient();
    const mounted = mountWith(client, <ReviewPage />);
    try {
      await vi.waitFor(
        () => expect(mounted.container.textContent).toContain(refusedCopy),
        { timeout: 5000 },
      );
      expect(mounted.container.textContent).toContain(durableDismissCopy);
      expect(mounted.container.textContent).toContain("U7 Evidence Book");

      const buttons = Array.from(
        mounted.container.querySelectorAll<HTMLButtonElement>("button"),
      );
      expect(buttons.map((button) => button.textContent?.trim())).toEqual([
        "Dismiss",
      ]);
      expect(mounted.container.textContent).not.toContain("Confirm Merge");
      expect(mounted.container.textContent).not.toContain("Link it");
      expect(mounted.container.textContent).not.toContain("Resolve");

      await clickButton(mounted.container, "Dismiss");
      await vi.waitFor(() => {
        expect(
          api.calls.filter(
            (call) =>
              call.method === "POST" &&
              call.path === "/identity-review-card/707/dismiss",
          ),
        ).toHaveLength(1);
      });
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});
