import { act } from "react";
import { Toaster, toast } from "sonner";

/**
 * The app's notification container, with the props `App` gives it. Pages
 * mounted on their own have no container, so a test that observes a toast
 * mounts this beside the page, as `App` does at the root.
 */
export function AppToaster() {
  return (
    <Toaster
      theme="dark"
      position="bottom-right"
      visibleToasts={5}
      gap={8}
      expand
      closeButton
    />
  );
}

/** Every toast on screen, whatever its text or type; leaving toasts excluded. */
export function liveToasts(): Element[] {
  return Array.from(
    document.querySelectorAll('[data-sonner-toast]:not([data-removed="true"])'),
  );
}

/**
 * The whole toast container as text and Sonner type ("error", "warning",
 * "success", "default"), for asserting exactly one toast of a given kind.
 */
export function toastSummary(): Array<{ type: string | null; text: string }> {
  return liveToasts().map((el) => ({
    type: el.getAttribute("data-type"),
    text: el.textContent ?? "",
  }));
}

/** Remove every toast so the next test starts with an empty container. */
export async function clearToasts() {
  await act(async () => {
    toast.dismiss();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

/**
 * Every toast element added to the page from now on, including any that has
 * already left, so a toast that came and went between two looks is counted.
 */
export function recordAddedToasts(): {
  added: () => Element[];
  stop: () => void;
} {
  const seen = new Set<Element>();
  const collect = (node: Node) => {
    if (!(node instanceof Element)) return;
    if (node.matches("[data-sonner-toast]")) seen.add(node);
    for (const el of Array.from(node.querySelectorAll("[data-sonner-toast]")))
      seen.add(el);
  };
  const observer = new MutationObserver((records) => {
    for (const record of records) record.addedNodes.forEach(collect);
  });
  observer.observe(document.body, { childList: true, subtree: true });
  return {
    added: () => {
      for (const record of observer.takeRecords())
        record.addedNodes.forEach(collect);
      return Array.from(seen);
    },
    stop: () => observer.disconnect(),
  };
}
