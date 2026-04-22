/**
 * Auth Store — Implementation Tests
 *
 * Tests the real Zustand auth store with mocked API and token functions.
 * Covers state machine transitions that behavioral tests only checked
 * structurally.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import type { AuthMeResponse, UserResponse } from "@/types/api";

// Mock the API module
vi.mock("@/api", () => ({
  getMe: vi.fn(),
  getSetupStatus: vi.fn(),
  login: vi.fn(),
  setup: vi.fn(),
  logout: vi.fn(),
}));

// Mock the token functions from client
vi.mock("@/api/client", () => ({
  getToken: vi.fn(() => null),
  setToken: vi.fn(),
  clearToken: vi.fn(),
  registerAuthClearedListener: vi.fn(),
  apiFetch: vi.fn(),
  ApiError: class ApiError extends Error {
    status: number;
    error: string;
    constructor(r: { status: number; error: string; message: string }) {
      super(r.message);
      this.name = "ApiError";
      this.status = r.status;
      this.error = r.error;
    }
  },
}));

import * as api from "@/api";
import { getToken, setToken, clearToken } from "@/api/client";
import { useAuthStore } from "@/stores/auth";

const mockApi = api as {
  getMe: ReturnType<typeof vi.fn>;
  getSetupStatus: ReturnType<typeof vi.fn>;
  login: ReturnType<typeof vi.fn>;
  setup: ReturnType<typeof vi.fn>;
  logout: ReturnType<typeof vi.fn>;
};

const mockGetToken = getToken as ReturnType<typeof vi.fn>;
const mockSetToken = setToken as ReturnType<typeof vi.fn>;
const mockClearToken = clearToken as ReturnType<typeof vi.fn>;

const adminUser: UserResponse = {
  id: 1,
  username: "pete",
  role: "admin",
  createdAt: "2026-03-31T00:00:00Z",
  updatedAt: "2026-03-31T00:00:00Z",
};

const regularUser: UserResponse = {
  id: 2,
  username: "reader",
  role: "user",
  createdAt: "2026-03-31T00:00:00Z",
  updatedAt: "2026-03-31T00:00:00Z",
};

function adminMeResponse(): AuthMeResponse {
  return { user: adminUser, authType: "session" };
}

function regularMeResponse(): AuthMeResponse {
  return { user: regularUser, authType: "session" };
}

describe("Auth Store Implementation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetToken.mockReturnValue(null);
    // Reset the store to initial state
    useAuthStore.setState({
      status: "loading",
      user: null,
      token: null,
      isAdmin: false,
    });
  });

  describe("initialize", () => {
    it("with valid token, calls getMe and transitions to authenticated", async () => {
      mockGetToken.mockReturnValue("valid-token");
      mockApi.getMe.mockResolvedValue(adminMeResponse());

      await useAuthStore.getState().initialize();

      const state = useAuthStore.getState();
      expect(state.status).toBe("authenticated");
      expect(state.user).toEqual(adminUser);
      expect(state.token).toBe("valid-token");
      expect(state.isAdmin).toBe(true);
      expect(mockApi.getMe).toHaveBeenCalledOnce();
    });

    it("with valid token but getMe fails (401), clears token, checks setup status", async () => {
      mockGetToken.mockReturnValue("expired-token");
      const { ApiError: MockApiError } = await import("@/api/client");
      mockApi.getMe.mockRejectedValue(
        new MockApiError({ status: 401, error: "unauthorized", message: "Session expired" }),
      );
      mockApi.getSetupStatus.mockResolvedValue({ setupRequired: false });

      await useAuthStore.getState().initialize();

      expect(mockClearToken).toHaveBeenCalled();
      expect(mockApi.getSetupStatus).toHaveBeenCalledOnce();
      const state = useAuthStore.getState();
      expect(state.status).toBe("unauthenticated");
      expect(state.user).toBeNull();
      expect(state.token).toBeNull();
    });

    it("no token, setupRequired=true -> setup_required state", async () => {
      mockGetToken.mockReturnValue(null);
      mockApi.getSetupStatus.mockResolvedValue({ setupRequired: true });

      await useAuthStore.getState().initialize();

      const state = useAuthStore.getState();
      expect(state.status).toBe("setup_required");
      expect(state.user).toBeNull();
    });

    it("no token, setupRequired=false -> unauthenticated state", async () => {
      mockGetToken.mockReturnValue(null);
      mockApi.getSetupStatus.mockResolvedValue({ setupRequired: false });

      await useAuthStore.getState().initialize();

      const state = useAuthStore.getState();
      expect(state.status).toBe("unauthenticated");
    });

    it("no token, getSetupStatus fails -> unauthenticated state", async () => {
      mockGetToken.mockReturnValue(null);
      mockApi.getSetupStatus.mockRejectedValue(new Error("Network error"));

      await useAuthStore.getState().initialize();

      const state = useAuthStore.getState();
      expect(state.status).toBe("unauthenticated");
      expect(state.user).toBeNull();
      expect(state.token).toBeNull();
      expect(state.isAdmin).toBe(false);
    });
  });

  describe("loginAction", () => {
    it("stores token, calls getMe, sets user and isAdmin=true for admin", async () => {
      mockApi.login.mockResolvedValue({ token: "new-token" });
      mockApi.getMe.mockResolvedValue(adminMeResponse());

      await useAuthStore.getState().loginAction("pete", "password", true);

      expect(mockSetToken).toHaveBeenCalledWith("new-token");
      expect(mockApi.getMe).toHaveBeenCalledOnce();
      const state = useAuthStore.getState();
      expect(state.status).toBe("authenticated");
      expect(state.user).toEqual(adminUser);
      expect(state.isAdmin).toBe(true);
      expect(state.token).toBe("new-token");
    });

    it("sets isAdmin=false for regular user", async () => {
      mockApi.login.mockResolvedValue({ token: "user-token" });
      mockApi.getMe.mockResolvedValue(regularMeResponse());

      await useAuthStore.getState().loginAction("reader", "password", false);

      const state = useAuthStore.getState();
      expect(state.status).toBe("authenticated");
      expect(state.user).toEqual(regularUser);
      expect(state.isAdmin).toBe(false);
    });
  });

  describe("setupAction", () => {
    it("returns apiKey string, stores token, sets user", async () => {
      mockApi.setup.mockResolvedValue({
        token: "setup-token",
        apiKey: "generated-api-key",
      });
      mockApi.getMe.mockResolvedValue(adminMeResponse());

      const apiKey = await useAuthStore
        .getState()
        .setupAction("pete", "password");

      expect(apiKey).toBe("generated-api-key");
      expect(mockSetToken).toHaveBeenCalledWith("setup-token");
      const state = useAuthStore.getState();
      expect(state.status).toBe("authenticated");
      expect(state.user).toEqual(adminUser);
      expect(state.token).toBe("setup-token");
    });
  });

  describe("logoutAction", () => {
    it("calls api.logout, clears token, resets state", async () => {
      // Set up authenticated state first
      useAuthStore.setState({
        status: "authenticated",
        user: adminUser,
        token: "active-token",
        isAdmin: true,
      });
      mockApi.logout.mockResolvedValue(undefined);

      await useAuthStore.getState().logoutAction();

      expect(mockApi.logout).toHaveBeenCalledOnce();
      expect(mockClearToken).toHaveBeenCalled();
      const state = useAuthStore.getState();
      expect(state.status).toBe("unauthenticated");
      expect(state.user).toBeNull();
      expect(state.token).toBeNull();
      expect(state.isAdmin).toBe(false);
    });

    it("clears state even if api.logout throws", async () => {
      useAuthStore.setState({
        status: "authenticated",
        user: adminUser,
        token: "active-token",
        isAdmin: true,
      });
      mockApi.logout.mockRejectedValue(new Error("Network failure"));

      await useAuthStore.getState().logoutAction();

      expect(mockClearToken).toHaveBeenCalled();
      const state = useAuthStore.getState();
      expect(state.status).toBe("unauthenticated");
      expect(state.user).toBeNull();
    });
  });

  describe("clearAuth", () => {
    it("resets all state fields", () => {
      useAuthStore.setState({
        status: "authenticated",
        user: adminUser,
        token: "some-token",
        isAdmin: true,
      });

      useAuthStore.getState().clearAuth();

      expect(mockClearToken).toHaveBeenCalled();
      const state = useAuthStore.getState();
      expect(state.status).toBe("unauthenticated");
      expect(state.user).toBeNull();
      expect(state.token).toBeNull();
      expect(state.isAdmin).toBe(false);
    });
  });

  describe("refreshUser", () => {
    it("updates user and isAdmin", async () => {
      useAuthStore.setState({
        status: "authenticated",
        user: adminUser,
        token: "token",
        isAdmin: true,
      });
      mockApi.getMe.mockResolvedValue(regularMeResponse());

      await useAuthStore.getState().refreshUser();

      const state = useAuthStore.getState();
      expect(state.user).toEqual(regularUser);
      expect(state.isAdmin).toBe(false);
    });
  });
});
