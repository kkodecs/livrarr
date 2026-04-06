import type { ApiErrorResponse } from "@/types/api";

const TOKEN_KEY = "librarr_token";

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token);
}

export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY);
}

export class ApiError extends Error {
  status: number;
  error: string;
  fieldErrors?: ApiErrorResponse["fieldErrors"];

  constructor(response: ApiErrorResponse) {
    super(response.message);
    this.name = "ApiError";
    this.status = response.status;
    this.error = response.error;
    this.fieldErrors = response.fieldErrors;
  }
}

async function normalizeError(res: Response): Promise<ApiErrorResponse> {
  try {
    const body: unknown = await res.json();
    if (
      body &&
      typeof body === "object" &&
      "message" in body &&
      typeof (body as Record<string, unknown>).message === "string"
    ) {
      const b = body as Record<string, unknown>;
      return {
        status: res.status,
        error:
          typeof b.error === "string"
            ? b.error
            : res.statusText.toLowerCase().replace(/\s+/g, "_"),
        message: b.message as string,
        fieldErrors: Array.isArray(b.fieldErrors)
          ? (b.fieldErrors as ApiErrorResponse["fieldErrors"])
          : undefined,
      };
    }
  } catch {
    // non-JSON body
  }

  const fallbacks: Record<number, { error: string; message: string }> = {
    400: { error: "bad_request", message: "Bad request" },
    401: { error: "unauthorized", message: "Session expired" },
    403: { error: "forbidden", message: "You don't have permission" },
    404: { error: "not_found", message: "Not found" },
    409: { error: "conflict", message: "Conflict" },
    422: { error: "validation", message: "Validation failed" },
    502: { error: "bad_gateway", message: "Could not reach upstream service" },
  };

  const fallback = fallbacks[res.status] ?? {
    error: "internal",
    message: "Something went wrong",
  };

  return { status: res.status, ...fallback };
}

export async function apiFetch<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const token = getToken();
  const headers = new Headers(options.headers);
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  if (
    options.body &&
    typeof options.body === "string" &&
    !headers.has("Content-Type")
  ) {
    headers.set("Content-Type", "application/json");
  }

  let res: Response;
  try {
    res = await fetch(`/api/v1${path}`, { ...options, headers });
  } catch {
    throw new ApiError({
      status: 0,
      error: "network_error",
      message: "Unable to reach Librarr",
    });
  }

  if (res.status === 401) {
    clearToken();
    // Redirect handled by auth store listener
    throw new ApiError({
      status: 401,
      error: "unauthorized",
      message: "Session expired",
    });
  }

  if (!res.ok) {
    throw new ApiError(await normalizeError(res));
  }

  if (res.status === 204 || res.headers.get("content-length") === "0") {
    return undefined as T;
  }

  return res.json() as Promise<T>;
}

export async function apiUpload<T>(path: string, file: Blob): Promise<T> {
  const token = getToken();
  const headers = new Headers();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const formData = new FormData();
  formData.append("image_data", file);

  let res: Response;
  try {
    res = await fetch(`/api/v1${path}`, {
      method: "POST",
      headers,
      body: formData,
    });
  } catch {
    throw new ApiError({
      status: 0,
      error: "network_error",
      message: "Unable to reach Librarr",
    });
  }

  if (res.status === 401) {
    clearToken();
    throw new ApiError({
      status: 401,
      error: "unauthorized",
      message: "Session expired",
    });
  }

  if (!res.ok) {
    throw new ApiError(await normalizeError(res));
  }

  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}
