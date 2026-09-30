import { toast } from "sonner";
import { updatePlaybackProgress } from "@/api";

const SAVE_FAILED_TOAST_ID = "save-position-failed";

/**
 * How long a dismissed warning stays mounted while it leaves the screen.
 * Sonner unmounts a toast 200 ms after it starts leaving and reports no
 * "removed" event; until then a toast issued under the same id is merged
 * into the leaving one and removed with it. The margin lets that removal
 * render before the warning is shown again.
 */
const WARNING_EXIT_MS = 300;

/** True from a failed position save until the next successful one. */
let saveFailing = false;
/** The warning is on screen and has not started to leave. */
let warningShown = false;
/** The warning has been dismissed and has not yet left the screen. */
let warningLeaving = false;
/** A failure run began while the warning was leaving; show it once it has gone. */
let showWhenGone = false;
let exitTimer: ReturnType<typeof setTimeout> | undefined;

function showWarning() {
  warningShown = true;
  toast.error("Could not save your place.", {
    id: SAVE_FAILED_TOAST_ID,
    duration: Infinity,
    onDismiss: warningStartedLeaving,
  });
}

/** Sonner calls this when the warning starts to leave, dismissed or closed. */
function warningStartedLeaving() {
  warningShown = false;
  warningLeaving = true;
  clearTimeout(exitTimer);
  exitTimer = setTimeout(() => {
    warningLeaving = false;
    if (showWhenGone) {
      showWhenGone = false;
      showWarning();
    }
  }, WARNING_EXIT_MS);
}

/**
 * Saves the reading or listening position. A failed save shows one error
 * toast that stays until a later save succeeds; further failures in the same
 * run add nothing. A failure run that begins while the previous warning is
 * still leaving shows its warning once that one has gone. A failed save is
 * not retried.
 */
export function savePosition(
  ...args: Parameters<typeof updatePlaybackProgress>
): void {
  updatePlaybackProgress(...args).then(
    () => {
      if (!saveFailing) return;
      saveFailing = false;
      showWhenGone = false;
      if (warningShown) {
        warningShown = false;
        warningLeaving = true;
        toast.dismiss(SAVE_FAILED_TOAST_ID);
      }
    },
    () => {
      if (saveFailing) return;
      saveFailing = true;
      if (warningLeaving) {
        showWhenGone = true;
      } else {
        showWarning();
      }
    },
  );
}
