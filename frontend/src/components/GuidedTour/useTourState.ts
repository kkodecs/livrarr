import { useCallback, useMemo } from "react";
import { useUIStore } from "@/stores/ui";

const TOUR_COMPLETED_KEY = "livrarr-tour-completed";

export function useTourState() {
  const running = useUIStore((s) => s.tourRunning);
  const setTourRunning = useUIStore((s) => s.setTourRunning);

  const start = useCallback(() => setTourRunning(true), [setTourRunning]);
  const stop = useCallback(() => {
    setTourRunning(false);
    localStorage.setItem(TOUR_COMPLETED_KEY, "true");
  }, [setTourRunning]);
  const hasCompleted = localStorage.getItem(TOUR_COMPLETED_KEY) === "true";

  return useMemo(
    () => ({ running, start, stop, hasCompleted }),
    [running, start, stop, hasCompleted],
  );
}
