import { create } from "zustand";
import { persist } from "zustand/middleware";

type ViewMode = "table" | "poster" | "overview";
type Theme = "dark" | "light";

interface UIState {
  sidebarCollapsed: boolean;
  mobileSidebarOpen: boolean;
  worksView: ViewMode;
  authorsView: ViewMode;
  worksSort: string;
  worksSortDir: "asc" | "desc";
  authorsSort: string;
  authorsSortDir: "asc" | "desc";
  worksFilter: string;
  worksMediaFilter: string;
  posterZoom: number;
  relativeDates: boolean;
  dateFormat: string;
  theme: Theme;
  tourRunning: boolean;
  rpmHighlight: boolean;
  checkForUpdates: boolean;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setMobileSidebarOpen: (open: boolean) => void;
  setWorksView: (view: ViewMode) => void;
  setAuthorsView: (view: ViewMode) => void;
  setWorksSort: (field: string, dir: "asc" | "desc") => void;
  setAuthorsSort: (field: string, dir: "asc" | "desc") => void;
  setWorksFilter: (filter: string) => void;
  setWorksMediaFilter: (filter: string) => void;
  setPosterZoom: (zoom: number) => void;
  setRelativeDates: (value: boolean) => void;
  setDateFormat: (fmt: string) => void;
  setTheme: (theme: Theme) => void;
  setTourRunning: (running: boolean) => void;
  setRpmHighlight: (highlight: boolean) => void;
  setCheckForUpdates: (value: boolean) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      mobileSidebarOpen: false,
      worksView: "table",
      authorsView: "table",
      worksSort: "title",
      worksSortDir: "asc",
      authorsSort: "name",
      authorsSortDir: "asc",
      worksFilter: "",
      worksMediaFilter: "",
      posterZoom: 5,
      relativeDates: true,
      dateFormat: "MMM d, yyyy",
      theme: "dark",
      tourRunning: false,
      rpmHighlight: false,
      checkForUpdates: true,
      toggleSidebar: () =>
        set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
      setMobileSidebarOpen: (open) => set({ mobileSidebarOpen: open }),
      setWorksView: (view) => set({ worksView: view }),
      setAuthorsView: (view) => set({ authorsView: view }),
      setWorksSort: (field, dir) =>
        set({ worksSort: field, worksSortDir: dir }),
      setAuthorsSort: (field, dir) =>
        set({ authorsSort: field, authorsSortDir: dir }),
      setWorksFilter: (filter) => set({ worksFilter: filter }),
      setWorksMediaFilter: (filter) => set({ worksMediaFilter: filter }),
      setPosterZoom: (zoom) => set({ posterZoom: zoom }),
      setRelativeDates: (value) => set({ relativeDates: value }),
      setDateFormat: (fmt) => set({ dateFormat: fmt }),
      setTheme: (theme) => set({ theme }),
      setTourRunning: (running) => set({ tourRunning: running }),
      setRpmHighlight: (highlight) => set({ rpmHighlight: highlight }),
      setCheckForUpdates: (value) => set({ checkForUpdates: value }),
    }),
    {
      name: "livrarr_ui",
    },
  ),
);
