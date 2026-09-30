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
