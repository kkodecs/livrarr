import { useState, useCallback } from "react";

const TOUR_COMPLETED_KEY = "livrarr-tour-completed";

export function useTourState() {
  const [running, setRunning] = useState(false);

  const start = useCallback(() => setRunning(true), []);
  const stop = useCallback(() => {
    setRunning(false);
    localStorage.setItem(TOUR_COMPLETED_KEY, "true");
  }, []);
  const hasCompleted = localStorage.getItem(TOUR_COMPLETED_KEY) === "true";

  return { running, start, stop, hasCompleted };
}
