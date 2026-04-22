/**
 * UI Store — Implementation Tests
 *
 * Tests the real Zustand UI store with persist middleware.
 * Uses jsdom's built-in localStorage for persistence verification.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { useUIStore } from "@/stores/ui";

const STORAGE_KEY = "livrarr_ui";

describe("UI Store Implementation", () => {
  beforeEach(() => {
    localStorage.clear();
    // Reset store to defaults
    useUIStore.setState({
      sidebarCollapsed: false,
      worksView: "table",
      authorsView: "table",
      worksSort: "title",
      worksSortDir: "asc",
      authorsSort: "name",
      authorsSortDir: "asc",
      worksFilter: "",
      relativeDates: true,
    });
  });

  describe("Initial state", () => {
    it("has correct defaults", () => {
      const state = useUIStore.getState();
      expect(state.sidebarCollapsed).toBe(false);
      expect(state.worksView).toBe("table");
      expect(state.authorsView).toBe("table");
      expect(state.worksSort).toBe("title");
      expect(state.worksSortDir).toBe("asc");
      expect(state.authorsSort).toBe("name");
      expect(state.authorsSortDir).toBe("asc");
      expect(state.worksFilter).toBe("");
      expect(state.relativeDates).toBe(true);
    });
  });

  describe("toggleSidebar", () => {
    it("flips sidebarCollapsed", () => {
      expect(useUIStore.getState().sidebarCollapsed).toBe(false);

      useUIStore.getState().toggleSidebar();
      expect(useUIStore.getState().sidebarCollapsed).toBe(true);

      useUIStore.getState().toggleSidebar();
      expect(useUIStore.getState().sidebarCollapsed).toBe(false);
    });
  });

  describe("setWorksView", () => {
    it("updates view mode", () => {
      useUIStore.getState().setWorksView("poster");
      expect(useUIStore.getState().worksView).toBe("poster");

      useUIStore.getState().setWorksView("overview");
      expect(useUIStore.getState().worksView).toBe("overview");

      useUIStore.getState().setWorksView("table");
      expect(useUIStore.getState().worksView).toBe("table");
    });
  });

  describe("setWorksSort", () => {
    it("updates both field and direction", () => {
      useUIStore.getState().setWorksSort("authorName", "desc");

      const state = useUIStore.getState();
      expect(state.worksSort).toBe("authorName");
      expect(state.worksSortDir).toBe("desc");
    });
  });

  describe("setWorksFilter", () => {
    it("updates filter", () => {
      useUIStore.getState().setWorksFilter("dune");
      expect(useUIStore.getState().worksFilter).toBe("dune");

      useUIStore.getState().setWorksFilter("");
      expect(useUIStore.getState().worksFilter).toBe("");
    });
  });

  describe("setRelativeDates", () => {
    it("updates value", () => {
      expect(useUIStore.getState().relativeDates).toBe(true);

      useUIStore.getState().setRelativeDates(false);
      expect(useUIStore.getState().relativeDates).toBe(false);

      useUIStore.getState().setRelativeDates(true);
      expect(useUIStore.getState().relativeDates).toBe(true);
    });
  });

  describe("Persistence", () => {
    it("state persists to localStorage under key 'livrarr_ui'", () => {
      useUIStore.getState().toggleSidebar();
      useUIStore.getState().setWorksView("poster");

      // Zustand persist writes to localStorage with the storage key
      const stored = localStorage.getItem(STORAGE_KEY);
      expect(stored).not.toBeNull();

      const parsed = JSON.parse(stored!);
      // Zustand persist wraps state in { state: {...}, version: 0 }
      const persisted = parsed.state ?? parsed;
      expect(persisted.sidebarCollapsed).toBe(true);
      expect(persisted.worksView).toBe("poster");
    });

    it("state restores from localStorage on store creation", () => {
      // Pre-populate localStorage with persisted state
      const persistedData = {
        state: {
          sidebarCollapsed: true,
          worksView: "poster",
          authorsView: "overview",
          worksSort: "year",
          worksSortDir: "desc",
          authorsSort: "name",
          authorsSortDir: "asc",
          worksFilter: "sci-fi",
          relativeDates: false,
        },
        version: 0,
      };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(persistedData));

      // Trigger rehydration by calling persist's rehydrate
      useUIStore.persist.rehydrate();

      const state = useUIStore.getState();
      expect(state.sidebarCollapsed).toBe(true);
      expect(state.worksView).toBe("poster");
      expect(state.authorsView).toBe("overview");
      expect(state.worksSort).toBe("year");
      expect(state.worksSortDir).toBe("desc");
      expect(state.worksFilter).toBe("sci-fi");
      expect(state.relativeDates).toBe(false);
    });
  });
});
