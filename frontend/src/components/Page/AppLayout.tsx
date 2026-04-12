import { lazy, Suspense, useEffect } from "react";
import { Outlet, useNavigate } from "react-router";
import { Header } from "@/components/Header/Header";
import { Sidebar } from "@/components/Sidebar/Sidebar";
import { useUIStore } from "@/stores/ui";
import { cn } from "@/utils/cn";
import { Toaster } from "sonner";
import { useTourState } from "@/components/GuidedTour/useTourState";

const GuidedTour = lazy(() => import("@/components/GuidedTour/GuidedTour"));

export function AppLayout() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const tour = useTourState();
  const navigate = useNavigate();

  const handleStartTour = () => {
    tour.start();
    navigate("/settings/metadata");
  };

  // Auto-start tour on first visit after setup
  useEffect(() => {
    if (!tour.hasCompleted && !tour.running) {
      setTimeout(handleStartTour, 500);
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Listen for tour start from help page
  useEffect(() => {
    const handler = () => {
      setTimeout(handleStartTour, 100);
    };
    window.addEventListener("livrarr:start-tour", handler);
    return () => window.removeEventListener("livrarr:start-tour", handler);
  }); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="min-h-screen bg-zinc-900">
      <Header />
      <Sidebar />
      <main
        className={cn(
          "pt-12 transition-all",
          // Desktop: margin for fixed sidebar
          // Mobile: no margin (sidebar is overlay)
          "ml-0 md:ml-14",
          !collapsed && "md:ml-56",
        )}
      >
        <Outlet />
      </main>
      <Toaster
        theme="dark"
        position="bottom-right"
        visibleToasts={5}
        gap={8}
        expand
        closeButton
        toastOptions={{
          className: "bg-zinc-800 border-border text-zinc-100",
        }}
      />
      {tour.running && (
        <Suspense fallback={null}>
          <GuidedTour running={tour.running} onStop={tour.stop} />
        </Suspense>
      )}
    </div>
  );
}
