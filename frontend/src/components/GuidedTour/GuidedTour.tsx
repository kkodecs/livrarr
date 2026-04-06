import { useState, useCallback } from "react";
import { Joyride, ACTIONS, EVENTS, STATUS } from "react-joyride";
import type { EventData, Controls } from "react-joyride";
import { useNavigate } from "react-router";
import { TOUR_STEPS } from "./tourSteps";

export default function GuidedTour({
  running,
  onStop,
}: {
  running: boolean;
  onStop: () => void;
}) {
  const navigate = useNavigate();
  const [stepIndex, setStepIndex] = useState(0);

  const handleEvent = useCallback(
    (data: EventData, controls: Controls) => {
      const { action, index, status, type } = data;
      console.log("[tour]", { type, action, status, index });

      // Tour finished or skipped
      if (status === STATUS.FINISHED || status === STATUS.SKIPPED) {
        onStop();
        setStepIndex(0);
        return;
      }

      // Close button
      if (action === ACTIONS.CLOSE) {
        controls.skip();
        onStop();
        setStepIndex(0);
        return;
      }

      if (type === EVENTS.STEP_AFTER) {
        const nextIndex =
          action === ACTIONS.PREV ? index - 1 : index + 1;

        // Past the last step — tour is done
        if (nextIndex >= TOUR_STEPS.length) {
          onStop();
          setStepIndex(0);
          return;
        }

        if (nextIndex >= 0 && nextIndex < TOUR_STEPS.length) {
          const nextStep = TOUR_STEPS[nextIndex];
          const nextRoute = (nextStep?.data as { route?: string })?.route;
          const currentRoute = (TOUR_STEPS[index]?.data as { route?: string })
            ?.route;

          if (nextRoute && nextRoute !== currentRoute) {
            navigate(nextRoute);
            // Small delay to let the page render before Joyride looks for targets
            setTimeout(() => setStepIndex(nextIndex), 300);
          } else {
            setStepIndex(nextIndex);
          }
        }
      }
    },
    [navigate, onStop],
  );

  if (!running) return null;

  return (
    <Joyride
      steps={TOUR_STEPS}
      stepIndex={stepIndex}
      run={running}
      continuous
      debug
      onEvent={handleEvent}
      locale={{
        back: "Back",
        close: "Close",
        last: "Finish",
        next: "Next",
        skip: "Skip Tour",
      }}
      options={{
        buttons: ["back", "skip", "primary"],
        showProgress: true,
        primaryColor: "#6366f1",
        overlayColor: "rgba(0, 0, 0, 0.6)",
        overlayClickAction: false,
      }}
      styles={{
        tooltip: {
          borderRadius: 8,
          padding: 16,
          backgroundColor: "#27272a",
          color: "#e4e4e7",
        },
        tooltipTitle: {
          color: "#e4e4e7",
        },
        tooltipContent: {
          color: "#a1a1aa",
        },
      }}
    />
  );
}
