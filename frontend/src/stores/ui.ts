import { create } from "zustand";
import { persist } from "zustand/middleware";

type ViewMode = "table" | "poster" | "overview";
type Theme = "dark" | "light";

interface UIState {
  sidebarCollapsed: boolean;
  worksView: ViewMode;
  authorsView: ViewMode;
  worksSort: string;
  worksSortDir: "asc" | "desc";
  authorsSort: string;
  authorsSortDir: "asc" | "desc";
  worksFilter: string;
  relativeDates: boolean;
  dateFormat: string;
  theme: Theme;
  tourRunning: boolean;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setWorksView: (view: ViewMode) => void;
  setAuthorsView: (view: ViewMode) => void;
  setWorksSort: (field: string, dir: "asc" | "desc") => void;
  setAuthorsSort: (field: string, dir: "asc" | "desc") => void;
  setWorksFilter: (filter: string) => void;
  setRelativeDates: (value: boolean) => void;
  setDateFormat: (fmt: string) => void;
  setTheme: (theme: Theme) => void;
  setTourRunning: (running: boolean) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      worksView: "table",
      authorsView: "table",
      worksSort: "title",
      worksSortDir: "asc",
      authorsSort: "name",
      authorsSortDir: "asc",
      worksFilter: "",
      relativeDates: true,
      dateFormat: "MMM d, yyyy",
      theme: "dark",
      tourRunning: false,
      toggleSidebar: () =>
        set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
      setWorksView: (view) => set({ worksView: view }),
      setAuthorsView: (view) => set({ authorsView: view }),
      setWorksSort: (field, dir) =>
        set({ worksSort: field, worksSortDir: dir }),
      setAuthorsSort: (field, dir) =>
        set({ authorsSort: field, authorsSortDir: dir }),
      setWorksFilter: (filter) => set({ worksFilter: filter }),
      setRelativeDates: (value) => set({ relativeDates: value }),
      setDateFormat: (fmt) => set({ dateFormat: fmt }),
      setTheme: (theme) => set({ theme }),
      setTourRunning: (running) => set({ tourRunning: running }),
    }),
    {
      name: "livrarr_ui",
    },
  ),
);
