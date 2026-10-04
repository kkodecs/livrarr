import { toast } from "sonner";

const PLAYBACK_FAILED_TOAST_ID = "playback-failed";

/**
 * Tells the user the audiobook could not play. Every door that sees a
 * playback failure reports through this one fixed id, so one failure seen by
 * several doors shows one pop-up: a report while it is on screen replaces it.
 */
export function reportPlaybackFailure(): void {
  toast.error("Could not play this audiobook. Try reloading the page.", {
    id: PLAYBACK_FAILED_TOAST_ID,
  });
}
