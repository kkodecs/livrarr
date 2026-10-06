import { afterEach, describe, expect, it } from "vitest";
import UISettingsPage from "@/pages/settings/ui/UISettingsPage";
import { useAuthStore } from "@/stores/auth";
import {
  installApiStub,
  mountWith,
  newTestClient,
} from "@/test-support/apiStub";

const headings = (scope: HTMLElement) =>
  Array.from(scope.querySelectorAll("h1, h2")).map(
    (h) => h.textContent?.trim() ?? "",
  );
const buttonLabels = (scope: HTMLElement) =>
  Array.from(scope.querySelectorAll("button")).map(
    (b) => b.textContent?.trim() ?? "",
  );

let cleanup: () => void = () => {};
afterEach(() => {
  cleanup();
  useAuthStore.setState({ isAdmin: false, user: null });
});

describe("Settings → UI has no Theme section", () => {
  for (const role of ["admin", "user"] as const) {
    it(`${role}: no Theme heading and no Light or Dark button; Date Format and Relative Dates still show`, () => {
      useAuthStore.setState({
        isAdmin: role === "admin",
        user: {
          id: 1,
          username: role,
          role,
          createdAt: "2026-01-01T00:00:00Z",
          updatedAt: "2026-01-01T00:00:00Z",
        },
      });
      const api = installApiStub((call) => ({
        status: 404,
        body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
      }));
      const mounted = mountWith(newTestClient(), <UISettingsPage />);
      cleanup = () => {
        mounted.cleanup();
        api.restore();
      };
      // Control: the page drew.
      expect(headings(mounted.container)).toContain("UI Settings");
      expect(headings(mounted.container)).toContain("Date Format");
      expect(headings(mounted.container)).toContain("Relative Dates");

      expect({
        theme: headings(mounted.container).includes("Theme"),
        light: buttonLabels(mounted.container).includes("Light"),
        dark: buttonLabels(mounted.container).includes("Dark"),
      }).toEqual({ theme: false, light: false, dark: false });
    });
  }
});
