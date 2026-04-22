import { create } from "zustand";
import type { UserResponse } from "@/types/api";
import {
  ApiError,
  getToken,
  setToken,
  clearToken,
  registerAuthClearedListener,
} from "@/api/client";
import * as api from "@/api";

export type AuthStatus =
  | "loading"
  | "unauthenticated"
  | "setup_required"
  | "authenticated"
  | "auth_unknown";

interface AuthState {
  status: AuthStatus;
  user: UserResponse | null;
  token: string | null;
  isAdmin: boolean;
  initialize: () => Promise<void>;
  loginAction: (
    username: string,
    password: string,
    rememberMe: boolean,
  ) => Promise<void>;
  setupAction: (username: string, password: string) => Promise<string>;
  logoutAction: () => Promise<void>;
  clearAuth: () => void;
  refreshUser: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set, get) => ({
  status: "loading",
  user: null,
  token: getToken(),
  isAdmin: false,

  initialize: async () => {
    const token = getToken();
    if (token) {
      try {
        const { user } = await api.getMe();
        set({
          status: "authenticated",
          user,
          token,
          isAdmin: user.role === "admin",
        });
        return;
      } catch (err) {
        // Only clear the stored token on an explicit 401 — the server has
        // rejected the credential.  Network failures (TypeError from fetch,
        // status 0, or any non-API error) should preserve the token so a
        // page refresh can retry once connectivity is restored.
        if (err instanceof ApiError && err.status === 401) {
          clearToken();
        } else {
          // Network or unexpected error — keep token, show retry state
          // instead of bouncing to login.
          set({
            status: "auth_unknown",
            user: null,
            token,
            isAdmin: false,
          });
          return;
        }
      }
    }
    // No valid token — check setup status
    try {
      const { setupRequired } = await api.getSetupStatus();
      set({
        status: setupRequired ? "setup_required" : "unauthenticated",
        user: null,
        token: null,
        isAdmin: false,
      });
    } catch {
      set({
        status: "unauthenticated",
        user: null,
        token: null,
        isAdmin: false,
      });
    }
  },

  loginAction: async (username, password, rememberMe) => {
    const { token } = await api.login({ username, password, rememberMe });
    setToken(token);
    const { user } = await api.getMe();
    set({
      status: "authenticated",
      user,
      token,
      isAdmin: user.role === "admin",
    });
  },

  // Setup lives in the auth store because it creates the first user and
  // returns an auth token in a single atomic operation.
  setupAction: async (username, password) => {
    const { token, apiKey } = await api.setup({ username, password });
    setToken(token);
    // Clear tour-completed flag so the guided tour starts for new setups.
    localStorage.removeItem("livrarr-tour-completed");
    const { user } = await api.getMe();
    set({
      status: "authenticated",
      user,
      token,
      isAdmin: user.role === "admin",
    });
    return apiKey;
  },

  logoutAction: async () => {
    try {
      await api.logout();
    } catch {
      // Ignore logout errors
    }
    get().clearAuth();
  },

  clearAuth: () => {
    clearToken();
    set({ status: "unauthenticated", user: null, token: null, isAdmin: false });
  },

  refreshUser: async () => {
    const { user } = await api.getMe();
    set({ user, isAdmin: user.role === "admin" });
  },
}));

// When apiFetch/apiUpload receive a 401 and call clearAuth() in client.ts,
// this listener keeps the Zustand state in sync (sets status, clears user).
registerAuthClearedListener(() => {
  const { status } = useAuthStore.getState();
  // Avoid re-entrancy if clearAuth was called from the store itself.
  if (status !== "unauthenticated") {
    useAuthStore.setState({
      status: "unauthenticated",
      user: null,
      token: null,
      isAdmin: false,
    });
  }
});
