import { afterEach, describe, expect, it, vi } from "vitest";
import ReviewPage from "@/pages/review/ReviewPage";
import { installApiStub, mountWith, newTestClient } from "@/test-support/apiStub";

afterEach(() => vi.restoreAllMocks());

describe("temporary merge containment", () => {
  it.each([[71], [71, 72]])("keeps the unresolved group visible without a confirmation action: %j", async (...workIds) => {
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/identity-review-card") {
        return { status: 200, body: [{
          id: 17, userId: 1, workId: 71, workTitle: "Preserved book",
          workAuthor: "Preserved author", kind: "GroupIdentity", generation: 1,
          payload: { GroupIdentity: { work_ids: workIds, proposed_identity: null, merge_choices: [] } },
        }] };
      }
      if (call.method === "GET" && ["/identity-review", "/author-link-review"].includes(call.path)) {
        return { status: 200, body: [] };
      }
      throw new Error(`Unexpected request: ${call.method} ${call.path}`);
    });
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() => expect(mounted.container.textContent).toContain("Preserved book"));
      expect(mounted.container.textContent).toContain("Merging is currently unavailable");
      const buttons = Array.from(mounted.container.querySelectorAll("button")).map((button) => button.textContent?.trim());
      expect(buttons).toEqual(["Dismiss"]);
      expect(api.calls.every((call) => call.method === "GET")).toBe(true);
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});
