import { afterEach, describe, expect, it, vi } from "vitest";
import ReviewPage from "@/pages/review/ReviewPage";
import cardList from "@/pages/review/fixtures/groupIdentityCardList.json";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

afterEach(() => vi.restoreAllMocks());

// The proposal exactly as the typed card list serializes it.
const proposal = {
  title: {
    main: "Beta",
    subtitle: "Proposed",
    volume: "7",
    normalized_main: "beta",
    normalized_subtitle: "proposed",
    normalized_volume: "7",
    provenance: "User",
  },
  primary_author_id: 9,
  routes: [],
};

function groupCard(workIds: number[], proposed: unknown) {
  return {
    id: 17,
    userId: 1,
    workId: 71,
    workTitle: "Alpha",
    workAuthor: "Ann",
    kind: "GroupIdentity",
    generation: 3,
    payload: {
      GroupIdentity: { work_ids: workIds, proposed_identity: proposed, merge_choices: [] },
    },
  };
}

const bob: StubReply = { status: 200, body: { author: { id: 9, name: "Bob" }, works: [] } };

function installReviewApi(card: unknown, authorReply: StubReply = bob) {
  return installApiStub((call: ApiCall) => {
    if (call.method === "GET" && call.path === "/identity-review-card") {
      return { status: 200, body: [card] };
    }
    if (call.method === "GET" && ["/identity-review", "/author-link-review"].includes(call.path)) {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path === "/author/9") {
      return authorReply;
    }
    if (call.method === "POST" && call.path === "/identity-review-card/17/resolve") {
      return { status: 200, body: { workId: 71 } };
    }
    throw new Error(`Unexpected request: ${call.method} ${call.path}`);
  });
}

function buttonLabels(container: HTMLElement) {
  return Array.from(container.querySelectorAll("button")).map((button) => button.textContent?.trim());
}

describe("GroupIdentity review card", () => {
  it.each([[[71]], [[71, 72]]])("offers Different book and Dismiss only: %j", async (workIds) => {
    const api = installReviewApi(groupCard(workIds, proposal));
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() => expect(mounted.container.textContent).toContain("Alpha"));
      expect(buttonLabels(mounted.container)).toEqual(["Dismiss", "Different book"]);
      const text = mounted.container.textContent ?? "";
      expect(text).not.toContain("Merging is currently unavailable");
      expect(text).not.toMatch(/books need review/);
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("shows what the book is being compared with", async () => {
    const api = installReviewApi(groupCard([71], proposal));
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() => expect(mounted.container.textContent).toContain("Bob"));
      const text = mounted.container.textContent ?? "";
      expect(text).toContain("Compared with");
      expect(text).toContain("Beta");
      expect(text).toContain("Ann");
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("keeps the proposed title when the proposed author no longer exists", async () => {
    const api = installReviewApi(groupCard([71], proposal), {
      status: 404,
      body: { error: "not_found", message: "not found", status: 404 },
    });
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() => expect(mounted.container.textContent).toContain("Compared with"));
      await vi.waitFor(() =>
        expect(api.calls.some((call) => call.path === "/author/9")).toBe(true),
      );
      expect(mounted.container.textContent).toContain("Beta");
      expect(mounted.container.textContent).not.toContain("Bob");
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("shows no comparison when the card carries no proposal", async () => {
    const api = installReviewApi(groupCard([71, 72], null));
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() => expect(mounted.container.textContent).toContain("Alpha"));
      expect(mounted.container.textContent).not.toContain("Compared with");
      expect(api.calls.some((call) => call.path.startsWith("/author/"))).toBe(false);
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("sends the different-book answer with the listed generation", async () => {
    const api = installReviewApi(groupCard([71], proposal));
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() => expect(buttonLabels(mounted.container)).toContain("Different book"));
      await clickButton(mounted.container, "Different book");
      await vi.waitFor(() =>
        expect(api.calls.some((call) => call.method === "POST")).toBe(true),
      );
      expect(api.calls.filter((call) => call.method === "POST")).toEqual([
        {
          method: "POST",
          path: "/identity-review-card/17/resolve",
          body: {
            command: {
              GroupIdentity: { card_id: 17, expected_generation: 3, action: "DifferentFromAll" },
            },
          },
        },
      ]);
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});

// AC-021: the list below is what the real card-list route serializes (a
// backend test keeps the fixture equal to the route's output). Card 17 compares
// with "Beta: Proposed" volume 7 by Bob; card 18's proposed author was deleted;
// card 19 carries no proposal.
describe("GroupIdentity comparison from the real card list", () => {
  function cardText(container: HTMLElement, workId: number) {
    const card = container.querySelector(`a[href="/work/${workId}"]`)?.closest("div.rounded-lg");
    return card?.textContent ?? "";
  }

  it("shows the complete proposal and author, the title alone without the author, and nothing without a proposal", async () => {
    const api = installApiStub((call: ApiCall) => {
      if (call.method === "GET" && call.path === "/identity-review-card") {
        return { status: 200, body: cardList };
      }
      if (call.method === "GET" && ["/identity-review", "/author-link-review"].includes(call.path)) {
        return { status: 200, body: [] };
      }
      if (call.method === "GET" && call.path === "/author/9") {
        return { status: 200, body: { author: { id: 9, name: "Bob" }, works: [] } };
      }
      if (call.method === "GET" && call.path === "/author/10") {
        return { status: 404, body: { error: "not_found", message: "not found", status: 404 } };
      }
      throw new Error(`Unexpected request: ${call.method} ${call.path}`);
    });
    const mounted = mountWith(newTestClient(), <ReviewPage />);
    try {
      await vi.waitFor(() =>
        expect(cardText(mounted.container, 71)).toContain("Compared with: Beta: Proposed (7) by Bob"),
      );
      await vi.waitFor(() =>
        expect(api.calls.some((call) => call.path === "/author/10")).toBe(true),
      );
      expect(cardText(mounted.container, 71)).toContain("Alpha");
      expect(cardText(mounted.container, 71)).toContain("by Ann");
      const orphaned = cardText(mounted.container, 72);
      expect(orphaned).toContain("Gamma");
      expect(orphaned).toContain("Compared with: Delta");
      expect(orphaned).not.toContain("Delta by");
      const bare = cardText(mounted.container, 73);
      expect(bare).toContain("Epsilon");
      expect(bare).not.toContain("Compared with");
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});
