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

  // Auto-start tour on first visit (after setup, regardless of entry path)
  useEffect(() => {
    if (!tour.hasCompleted) {
      tour.start();
      navigate("/settings/metadata");
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleStartTour = () => {
    tour.start();
    navigate("/settings/metadata");
  };

  return (
    <div className="min-h-screen bg-zinc-900">
      <Header onStartTour={handleStartTour} />
      <Sidebar />
      <main
        className={cn("pt-12 transition-all", collapsed ? "ml-14" : "ml-56")}
      >
        <Outlet />
      </main>
      <Toaster
        theme="dark"
        position="bottom-right"
        visibleToasts={5}
        gap={8}
        expand
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
