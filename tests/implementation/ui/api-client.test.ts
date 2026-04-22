/**
 * API Client — Implementation Tests
 *
 * Tests the real apiFetch, ApiError, token management, and error normalization
 * with mocked fetch and localStorage. Covers edge cases not in behavioral tests.
 */
import { describe, it, expect, beforeEach, vi, type Mock } from "vitest";

// Mock localStorage before importing the module
const localStorageMock = (() => {
  let store: Record<string, string> = {};
  return {
    getItem: vi.fn((key: string) => store[key] ?? null),
    setItem: vi.fn((key: string, value: string) => {
      store[key] = value;
    }),
    removeItem: vi.fn((key: string) => {
      delete store[key];
    }),
    clear: vi.fn(() => {
      store = {};
    }),
    get length() {
      return Object.keys(store).length;
    },
    key: vi.fn((i: number) => Object.keys(store)[i] ?? null),
  };
})();
Object.defineProperty(global, "localStorage", { value: localStorageMock });

import {
  apiFetch,
  ApiError,
  getToken,
  setToken,
  clearToken,
} from "@/api/client";

function mockFetchResponse(
  status: number,
  body?: unknown,
  headers?: Record<string, string>,
): Response {
  const responseHeaders = new Headers(headers);
  if (body !== undefined && !responseHeaders.has("content-length")) {
    // Don't auto-set content-length; let individual tests control it
  }
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: statusTextFor(status),
    headers: responseHeaders,
    json: vi.fn().mockResolvedValue(body),
    text: vi.fn().mockResolvedValue(typeof body === "string" ? body : JSON.stringify(body)),
  } as unknown as Response;
}

function statusTextFor(status: number): string {
  const map: Record<number, string> = {
    200: "OK",
    204: "No Content",
    400: "Bad Request",
    401: "Unauthorized",
    403: "Forbidden",
    404: "Not Found",
    409: "Conflict",
    422: "Unprocessable Entity",
    500: "Internal Server Error",
    502: "Bad Gateway",
    418: "I'm a Teapot",
  };
  return map[status] ?? "Unknown";
}

describe("API Client Implementation", () => {
  beforeEach(() => {
    localStorageMock.clear();
    vi.restoreAllMocks();
    global.fetch = vi.fn();
  });

  describe("Authorization header", () => {
    it("adds Authorization header when token exists in localStorage", async () => {
      localStorageMock.setItem("livrarr_token", "test-token-123");
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(200, { ok: true }),
      );

      await apiFetch("/test");

      const call = (global.fetch as Mock).mock.calls[0];
      const headers = call[1].headers as Headers;
      expect(headers.get("Authorization")).toBe("Bearer test-token-123");
    });

    it("does NOT add Authorization header when no token", async () => {
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(200, { ok: true }),
      );

      await apiFetch("/test");

      const call = (global.fetch as Mock).mock.calls[0];
      const headers = call[1].headers as Headers;
      expect(headers.has("Authorization")).toBe(false);
    });
  });

  describe("Content-Type header", () => {
    it("sets Content-Type: application/json for string body", async () => {
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(200, { ok: true }),
      );

      await apiFetch("/test", {
        method: "POST",
        body: JSON.stringify({ name: "test" }),
      });

      const call = (global.fetch as Mock).mock.calls[0];
      const headers = call[1].headers as Headers;
      expect(headers.get("Content-Type")).toBe("application/json");
    });

    it("does NOT set Content-Type for non-string body (FormData)", async () => {
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(200, { ok: true }),
      );
      const formData = new FormData();
      formData.append("file", "data");

      await apiFetch("/test", {
        method: "POST",
        body: formData,
      });

      const call = (global.fetch as Mock).mock.calls[0];
      const headers = call[1].headers as Headers;
      expect(headers.has("Content-Type")).toBe(false);
    });
  });

  describe("Network errors", () => {
    it("throws ApiError with status 0 and error 'network_error' when fetch throws TypeError", async () => {
      (global.fetch as Mock).mockRejectedValue(new TypeError("Failed to fetch"));

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        expect(e).toBeInstanceOf(ApiError);
        const err = e as ApiError;
        expect(err.status).toBe(0);
        expect(err.error).toBe("network_error");
        expect(err.message).toBe("Unable to reach Livrarr");
      }
    });
  });

  describe("401 handling", () => {
    it("clears token on 401 and throws ApiError", async () => {
      localStorageMock.setItem("livrarr_token", "old-token");
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(401, { message: "Session expired" }),
      );

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        expect(e).toBeInstanceOf(ApiError);
        const err = e as ApiError;
        expect(err.status).toBe(401);
        expect(err.error).toBe("unauthorized");
      }

      expect(localStorageMock.getItem("livrarr_token")).toBeNull();
    });
  });

  describe("Empty response handling", () => {
    it("handles 204 No Content (returns undefined)", async () => {
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(204, undefined),
      );

      const result = await apiFetch("/test");
      expect(result).toBeUndefined();
    });

    it("handles response with content-length: 0", async () => {
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(200, undefined, { "content-length": "0" }),
      );

      const result = await apiFetch("/test");
      expect(result).toBeUndefined();
    });
  });

  describe("Error normalization", () => {
    it("normalizes JSON error body with message field", async () => {
      const errorBody = {
        message: "Username already taken",
        error: "conflict",
      };
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(409, errorBody),
      );

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        const err = e as ApiError;
        expect(err.status).toBe(409);
        expect(err.error).toBe("conflict");
        expect(err.message).toBe("Username already taken");
      }
    });

    it("normalizes JSON error body WITHOUT message field (uses fallback)", async () => {
      // Body is JSON but has no message property
      const errorBody = { detail: "some internal detail" };
      const res = mockFetchResponse(403, undefined);
      // Override json() to return body without message
      (res.json as Mock).mockResolvedValue(errorBody);
      (global.fetch as Mock).mockResolvedValue(res);

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        const err = e as ApiError;
        expect(err.status).toBe(403);
        expect(err.error).toBe("forbidden");
        expect(err.message).toBe("You don't have permission");
      }
    });

    it("normalizes non-JSON error body (uses status-based fallback)", async () => {
      const res = mockFetchResponse(500, undefined);
      // Override json() to throw (simulating non-JSON body)
      (res.json as Mock).mockRejectedValue(new SyntaxError("Unexpected token"));
      (global.fetch as Mock).mockResolvedValue(res);

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        const err = e as ApiError;
        expect(err.status).toBe(500);
        expect(err.error).toBe("internal");
        expect(err.message).toBe("Something went wrong");
      }
    });

    it("normalizes unknown status code (falls back to 'internal'/'Something went wrong')", async () => {
      const res = mockFetchResponse(418, undefined);
      (res.json as Mock).mockRejectedValue(new SyntaxError("not json"));
      (global.fetch as Mock).mockResolvedValue(res);

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        const err = e as ApiError;
        expect(err.status).toBe(418);
        expect(err.error).toBe("internal");
        expect(err.message).toBe("Something went wrong");
      }
    });

    it("normalizes 422 with fieldErrors array", async () => {
      const errorBody = {
        message: "Validation failed",
        error: "validation",
        fieldErrors: [
          { field: "host", message: "Host is required" },
          { field: "port", message: "Port must be 1-65535" },
        ],
      };
      (global.fetch as Mock).mockResolvedValue(
        mockFetchResponse(422, errorBody),
      );

      try {
        await apiFetch("/test");
        expect.unreachable("Should have thrown");
      } catch (e) {
        const err = e as ApiError;
        expect(err.status).toBe(422);
        expect(err.error).toBe("validation");
        expect(err.fieldErrors).toHaveLength(2);
        expect(err.fieldErrors![0].field).toBe("host");
        expect(err.fieldErrors![1].field).toBe("port");
      }
    });
  });

  describe("Token management", () => {
    it("setToken/getToken/clearToken round-trip through localStorage", () => {
      expect(getToken()).toBeNull();

      setToken("my-token");
      expect(getToken()).toBe("my-token");

      clearToken();
      expect(getToken()).toBeNull();
    });
  });

  describe("ApiError class", () => {
    it("has correct name, status, error, message properties", () => {
      const err = new ApiError({
        status: 404,
        error: "not_found",
        message: "Not found",
      });
      expect(err).toBeInstanceOf(Error);
      expect(err.name).toBe("ApiError");
      expect(err.status).toBe(404);
      expect(err.error).toBe("not_found");
      expect(err.message).toBe("Not found");
      expect(err.fieldErrors).toBeUndefined();
    });
  });
});
