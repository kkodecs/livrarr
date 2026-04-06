import { Outlet } from "react-router";
import { Header } from "@/components/Header/Header";
import { Sidebar } from "@/components/Sidebar/Sidebar";
import { useUIStore } from "@/stores/ui";
import { cn } from "@/utils/cn";
import { Toaster } from "sonner";

export function AppLayout() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);

  return (
    <div className="min-h-screen bg-zinc-900">
      <Header />
      <Sidebar />
      <main
        className={cn("pt-12 transition-all", collapsed ? "ml-14" : "ml-56")}
      >
        <Outlet />
      </main>
      <Toaster
        theme="dark"
        position="bottom-right"
        toastOptions={{
          className: "bg-zinc-800 border-border text-zinc-100",
        }}
      />
    </div>
  );
}
