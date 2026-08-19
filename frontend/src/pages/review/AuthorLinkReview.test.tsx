import { describe, it, expect, afterEach, vi } from "vitest";
import { act } from "react";
import ReviewPage from "./ReviewPage";
import AuthorDetailPage from "@/pages/author-detail/AuthorDetailPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import {
  EMPTY_BIBLIOGRAPHY,
  makeAuthor,
  makeCandidate,
  makeRoute,
  makeSweep,
  makeVariant,
} from "@/test-support/authorFixtures";

let restore: (() => void) | null = null;

afterEach(() => {
  restore?.();
  restore = null;
});

function stub(handler: (call: ApiCall) => StubReply | Promise<StubReply>) {
  const s = installApiStub(handler);
  restore = s.restore;
  return s;
}

/** Text of every rendered author-review card, joined. */
function authorSectionText(container: HTMLElement): string {
  return container.textContent ?? "";
}

describe("listAuthorLinkReview — the Review page's Authors section", () => {
  // IR tdd_directive 1. The two halves of the review page answer different
  // questions from different endpoints. Neither may hide or wait on the other,
  // and every label must come from the server's own evidence fields.
  it("renders author evidence when the book queries fail, and vice versa", async () => {
    const candidates = [
      makeCandidate({
        id: 41,
        candidate_name: "Ursula Le Guin",
        primary_name_verdict: "Grey",
        alternate_name_evidence: [
          { name: "U. K. Le Guin", verdict: "Agree" },
          { name: "Ursula Kroeber", verdict: "Disagree" },
        ],
        top_work_preview: "A Wizard of Earthsea",
        catalog_evidence_state: "complete",
        corroborated_title_count: 4,
      }),
      // A read that is still going and has seen nothing yet: "no count" is the
      // honest answer, never a count of zero.
      makeCandidate({
        id: 42,
        key: { open_library: "OL222A" },
        candidate_name: "Ursula K LeGuin",
        catalog_evidence_state: "retrying",
        corroborated_title_count: 0,
      }),
      // A read that failed outright.
      makeCandidate({
        id: 43,
        key: { open_library: "OL333A" },
        candidate_name: "U Le Guin",
        catalog_evidence_state: "unavailable",
        corroborated_title_count: 0,
      }),
      // Not a Tier-2 name search, so no catalogue read is ever scheduled for
      // it — a "Pending" state here must not be shown as one that is coming.
      makeCandidate({
        id: 44,
        key: { goodreads: 874602 },
        candidate_name: "Ursula K. Le Guin",
        reason: "readarr_name_guard_failed",
        catalog_evidence_state: "pending",
        corroborated_title_count: 0,
        previously_removed: true,
      }),
    ];

    // Books half is down; authors half is healthy.
    stub((call) => {
      if (
        call.path === "/identity-review" ||
        call.path === "/identity-review-card" ||
        call.path === "/identity-conflict"
      ) {
        return { status: 500, body: { status: 500, error: "internal", message: "boom" } };
      }
      if (call.path === "/author-link-review") {
        return {
          status: 200,
          body: [{ author: makeAuthor({ linkState: "needs_review" }), candidates }],
        };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const client = newTestClient();
    const { container, cleanup } = mountWith(client, <ReviewPage />);
    try {
      await vi.waitFor(
        () => expect(authorSectionText(container)).toContain("Ursula Le Guin"),
        { timeout: 5000 },
      );

      const text = authorSectionText(container);
      // The books half failed and says so, on its own, without taking the
      // authors half down with it.
      expect(text).toContain("Retry");
      expect(text).not.toContain("Nothing needs review right now");

      // Primary and alias verdicts, straight from the candidate's fields.
      expect(text).toContain("Main name: name partly matches");
      expect(text).toContain('Also known as "U. K. Le Guin": name matches');
      expect(text).toContain('Also known as "Ursula Kroeber": name does not match');
      // A preview title is a hint, never a count.
      expect(text).toContain('Best known for "A Wizard of Earthsea"');

      // Catalogue counts: final when complete, "at least"/unknown while a read
      // is unfinished, and never rendered as zero for a read that failed.
      expect(text).toContain("4 of your books by this author are in their catalogue");
      expect(text).toContain("Still reading their catalogue — no count yet");
      expect(text).toContain("Their catalogue could not be read — no count available");
      expect(text).not.toContain("0 of your books");
      // The non-Tier-2 candidate gets no catalogue line at all.
      expect(text).not.toContain("Their catalogue has not been read yet");
      // And a previously-removed suggestion says so.
      expect(text).toContain("previously removed");
      // Four candidate rows, one per server row — nothing invented, nothing dropped.
      const pickButtons = Array.from(container.querySelectorAll("button")).filter(
        (b) => b.textContent === "Use this",
      );
      expect(pickButtons).toHaveLength(4);
    } finally {
      cleanup();
    }
  });

  // U8 D8-4. A question raised by a credit on one of the user's own books can
  // name that book. Without one — a deleted book, or a question that never came
  // from a book — the card says what kind of question it is instead of
  // inventing a title.
  it("names the book a credit was read on, and falls back when there is none", async () => {
    const candidates = [
      makeCandidate({
        id: 51,
        key: { hardcover: 4102 },
        candidate_name: "Kobayashi Chiaki",
        reason: "name_guard_failed",
        primary_name_verdict: "Disagree",
        catalog_evidence_state: "pending",
        evidence_work_id: 88,
        evidence_work_title: "The Left Hand of Darkness",
      }),
      // Same kind of question, but its book is gone.
      makeCandidate({
        id: 52,
        key: { hardcover: 4103 },
        candidate_name: "Someone Else",
        reason: "name_guard_failed",
        primary_name_verdict: "Disagree",
        catalog_evidence_state: "pending",
        evidence_work_id: null,
        evidence_work_title: null,
      }),
    ];

    stub((call) => {
      if (
        call.path === "/identity-review" ||
        call.path === "/identity-review-card" ||
        call.path === "/identity-conflict"
      ) {
        return { status: 200, body: [] };
      }
      if (call.path === "/author-link-review") {
        return {
          status: 200,
          body: [{ author: makeAuthor({ linkState: "needs_review" }), candidates }],
        };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const client = newTestClient();
    const { container, cleanup } = mountWith(client, <ReviewPage />);
    try {
      await vi.waitFor(
        () => expect(authorSectionText(container)).toContain("Kobayashi Chiaki"),
        { timeout: 5000 },
      );

      const text = authorSectionText(container);
      expect(text).toContain(
        'Hardcover credits "Kobayashi Chiaki" as an author of ' +
          '"The Left Hand of Darkness". It doesn\'t match any name you have for ' +
          "Ursula K. Le Guin.",
      );
      // The book-less question keeps the category wording.
      expect(text).toContain(
        "The provider's name for this author did not match",
      );
    } finally {
      cleanup();
    }
  });

  it("renders the book queries when the author query fails", async () => {
    stub((call) => {
      if (call.path === "/identity-review") {
        return {
          status: 200,
          body: [
            {
              workId: 3,
              title: "The Dispossessed",
              authorName: "Ursula K. Le Guin",
              candidates: [],
            },
          ],
        };
      }
      if (call.path === "/identity-conflict") return { status: 200, body: [] };
      if (call.path === "/identity-review-card") return { status: 200, body: [] };
      if (call.path === "/author-link-review") {
        return {
          status: 503,
          body: { status: 503, error: "service_unavailable", message: "authors unavailable" },
        };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const client = newTestClient();
    const { container, cleanup } = mountWith(client, <ReviewPage />);
    try {
      await vi.waitFor(
        () => expect(container.textContent).toContain("The Dispossessed"),
        { timeout: 5000 },
      );
      // The authors half reports its own failure, next to a working books half.
      expect(container.textContent).toContain("authors unavailable");
      expect(container.textContent).toContain("Needs Your Pick (1)");
      expect(container.textContent).not.toContain("Nothing needs review right now");
    } finally {
      cleanup();
    }
  });

  // Bug reproduction: identity-layer-rewrite round 18 — conflict cards
  // exposed a bare internal Work id while the title query loaded or failed.
  it("uses neutral conflict labels while the existing book loads or fails", async () => {
    let finishWork!: (reply: StubReply) => void;
    const workReply = new Promise<StubReply>((resolve) => {
      finishWork = resolve;
    });
    stub(async (call) => {
      if (call.path === "/identity-review") return { status: 200, body: [] };
      if (call.path === "/identity-review-card") return { status: 200, body: [] };
      if (call.path === "/identity-conflict") {
        return {
          status: 200,
          body: [
            {
              id: 18,
              existingWorkId: 73,
              kind: "incoming_different_ol_key",
              incomingTitle: "A Candidate Book",
              incomingAuthor: "Candidate Author",
              incomingOlKey: "OL18W",
              raisedAt: "2026-08-18T00:00:00Z",
              raisedBy: "convergence",
              status: "open",
            },
          ],
        };
      }
      if (call.path === "/author-link-review") return { status: 200, body: [] };
      if (call.path === "/work/73") return workReply;
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const client = newTestClient();
    const review = mountWith(client, <ReviewPage />);
    try {
      await vi.waitFor(
        () => expect(review.container.textContent).toContain("A Candidate Book"),
        { timeout: 5000 },
      );
      expect(review.container.textContent).toContain("Loading…");
      expect(review.container.textContent).not.toContain("Work #73");

      await act(async () => {
        finishWork({
          status: 500,
          body: { status: 500, error: "internal", message: "book unavailable" },
        });
      });
      await vi.waitFor(
        () => expect(review.container.textContent).toContain("this book"),
        { timeout: 5000 },
      );
      expect(review.container.textContent).not.toContain("Work #73");
    } finally {
      review.cleanup();
    }
  });

  it("finishes a merge from the typed GroupIdentity card", async () => {
    let resolved = false;
    const { calls } = stub((call) => {
      if (call.path === "/identity-review" || call.path === "/identity-conflict") {
        return { status: 200, body: [] };
      }
      if (call.path === "/identity-review-card") {
        return {
          status: 200,
          body: resolved
            ? []
            : [
                {
                  id: 219,
                  userId: 1,
                  workId: 216,
                  workTitle: "Merge Survivor",
                  workAuthor: "Merge Author",
                  kind: "GroupIdentity",
                  // The collection returns the anchor's current generation,
                  // which may be newer than the immutable mint generation.
                  generation: 9,
                  payload: {
                    GroupIdentity: {
                      work_ids: [216, 215],
                      proposed_identity: null,
                      merge_choices: [],
                    },
                  },
                },
              ],
        };
      }
      if (call.path === "/identity-review-card/219/resolve" && call.method === "POST") {
        resolved = true;
        return { status: 200, body: { workId: 216 } };
      }
      if (call.path === "/author-link-review") return { status: 200, body: [] };
      if (call.path === "/work/216") {
        return { status: 200, body: { id: 216, title: "Merge Survivor" } };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const client = newTestClient();
    const review = mountWith(client, <ReviewPage />);
    try {
      await vi.waitFor(
        () => expect(review.container.textContent).toContain("Confirm Merge"),
        { timeout: 5000 },
      );
      await clickButton(review.container, "Confirm Merge");
      await vi.waitFor(() => {
        const resolveCall = calls.find(
          (call) =>
            call.path === "/identity-review-card/219/resolve" && call.method === "POST",
        );
        expect(resolveCall?.body).toEqual({
          command: {
            GroupIdentity: {
              card_id: 219,
              expected_generation: 9,
              action: { AttachOrMerge: { anchor: 216 } },
            },
          },
        });
      });
    } finally {
      review.cleanup();
    }
  });
});

describe("pickAuthorLinkCandidate — picking a candidate from review", () => {
  // IR tdd_directive 2. The pick writes the route through the real endpoint,
  // and everything downstream of an author's identity must catch up: the
  // author detail panel's route and its provenance, whether the author can be
  // monitored, and the name spellings the pick brought with it.
  it("writes the route and refreshes every surface that depends on it", async () => {
    const parked = makeAuthor({ linkState: "needs_review" });
    const linked = makeAuthor({
      linkState: "linked",
      monitorable: true,
      olKey: "OL111A",
      routes: [makeRoute({ id: 9, provenance: "user_picked", value: "OL111A" })],
      nameVariants: [
        makeVariant({ id: 1, name: "Ursula K. Le Guin", selected: true }),
        makeVariant({ id: 2, name: "U. K. Le Guin" }),
      ],
    });

    let picked = false;
    const { calls } = stub((call) => {
      if (
        call.path === "/identity-review" ||
        call.path === "/identity-review-card" ||
        call.path === "/identity-conflict"
      ) {
        return { status: 200, body: [] };
      }
      if (call.path === "/author-link-review") {
        return {
          status: 200,
          body: picked
            ? []
            : [{ author: parked, candidates: [makeCandidate({ id: 41 })] }],
        };
      }
      if (call.path === "/author-link-review/41/pick" && call.method === "POST") {
        picked = true;
        return {
          status: 200,
          body: {
            id: 9,
            provider: "open_library",
            value: "OL111A",
            state: "active",
            provenance: "user_picked",
            removedAt: null,
          },
        };
      }
      if (call.path === "/author/7") {
        return {
          status: 200,
          body: { author: picked ? linked : parked, works: [] },
        };
      }
      if (call.path === "/author/7/bibliography") {
        return { status: 200, body: EMPTY_BIBLIOGRAPHY };
      }
      if (call.path === "/author-link-sweep/progress") {
        return { status: 200, body: makeSweep({ total: 1, completed: 1 }) };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    // One cache, two surfaces: the review page picks, the author page is what
    // has to change as a result.
    const client = newTestClient();
    const review = mountWith(client, <ReviewPage />);
    const detail = mountWith(client, <AuthorDetailPage />, {
      path: "/author/7",
      route: "/author/:id",
    });

    try {
      await vi.waitFor(
        () => expect(detail.container.textContent).toContain("Cannot be monitored"),
        { timeout: 5000 },
      );
      await vi.waitFor(
        () => expect(review.container.textContent).toContain("Use this"),
        { timeout: 5000 },
      );

      await clickButton(review.container, "Use this");

      // The real endpoint, by the real path and method.
      const pickIndex = calls.findIndex(
        (c) => c.path === "/author-link-review/41/pick",
      );
      expect(pickIndex).toBeGreaterThanOrEqual(0);
      expect(calls[pickIndex]?.method).toBe("POST");

      // The picked route and the provenance it was written with.
      await vi.waitFor(
        () => expect(detail.container.textContent).toContain("You picked it"),
        { timeout: 5000 },
      );
      expect(detail.container.textContent).toContain("OL111A");
      expect(detail.container.textContent).toContain("Open Library");
      // An Open Library route is what makes an author monitorable.
      expect(detail.container.textContent).toContain("Can be monitored");
      // Both spellings the candidate carried are still offered.
      expect(detail.container.textContent).toContain("U. K. Le Guin");

      // Invalidation reached every surface: the review list, the author,
      // the author list, and the sweep counters.
      const paths = calls.slice(pickIndex + 1).map((c) => c.path);
      expect(paths).toContain("/author-link-review");
      expect(paths).toContain("/author/7");
      expect(paths).toContain("/author-link-sweep/progress");
      // The author left the review list rather than being hidden locally.
      await vi.waitFor(
        () => expect(review.container.textContent).not.toContain("Use this"),
        { timeout: 5000 },
      );
    } finally {
      detail.cleanup();
      review.cleanup();
    }
  });
});
