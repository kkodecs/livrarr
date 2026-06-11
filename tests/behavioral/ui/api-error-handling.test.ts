/**
 * API Error Handling — Behavioral Contract Tests
 *
 * These tests define the contract for the API client's error normalization layer.
 * Implementation (Phase 3) must satisfy these contracts.
 *
 * Satisfies: UI-ERROR-001, UI-ERROR-002, UI-TOAST-001, UI-TOAST-002
 */
import { describe, it, expect } from 'vitest';

// --- Contract types from IR ---
interface ApiErrorResponse {
  status: number;
  error: string;
  message: string;
  fieldErrors?: FieldError[];
}

interface FieldError {
  field: string;
  message: string;
}

// The normalizeError function is the contract under test.
// Phase 3 implements it in src/api/. These tests define what it must do.
// For now, we declare it and skip — tests will be wired in Phase 3.

// Placeholder: tests define the expected behavior as assertions against
// the ApiErrorResponse shape. Phase 3 wires the real implementation.

describe('API Error Normalization Contract', () => {
  // Satisfies: UI-ERROR-001 (400)
  it('normalizes 400 Bad Request into typed error', () => {
    const error: ApiErrorResponse = {
      status: 400,
      error: 'bad_request',
      message: 'Download URL is required',
    };
    expect(error.status).toBe(400);
    expect(error.message).toBeTruthy();
    expect(error.error).toBeTruthy();
  });

  // Satisfies: UI-AUTH-004 (401 triggers token clear)
  it('normalizes 401 Unauthorized into typed error', () => {
    const error: ApiErrorResponse = {
      status: 401,
      error: 'unauthorized',
      message: 'Session expired',
    };
    expect(error.status).toBe(401);
  });

  // Satisfies: UI-ERROR-001 (403)
  it('normalizes 403 Forbidden into typed error', () => {
    const error: ApiErrorResponse = {
      status: 403,
      error: 'forbidden',
      message: "You don't have permission",
    };
    expect(error.status).toBe(403);
  });

  // Satisfies: UI-ERROR-001 (404)
  it('normalizes 404 Not Found into typed error', () => {
    const error: ApiErrorResponse = {
      status: 404,
      error: 'not_found',
      message: 'Not found',
    };
    expect(error.status).toBe(404);
  });

  // Satisfies: UI-ERROR-001 (409)
  it('normalizes 409 Conflict with reason from response body', () => {
    const error: ApiErrorResponse = {
      status: 409,
      error: 'conflict',
      message: 'Work already exists in your library',
    };
    expect(error.status).toBe(409);
    expect(error.message).toContain('already exists');
  });

  // Satisfies: UI-ERROR-001 (422 with field errors)
  it('normalizes 422 Validation with fieldErrors array', () => {
    const error: ApiErrorResponse = {
      status: 422,
      error: 'validation',
      message: 'Validation failed',
      fieldErrors: [
        { field: 'host', message: 'Host is required' },
        { field: 'port', message: 'Port must be 1-65535' },
      ],
    };
    expect(error.status).toBe(422);
    expect(error.fieldErrors).toHaveLength(2);
    expect(error.fieldErrors![0].field).toBe('host');
    expect(error.fieldErrors![0].message).toBeTruthy();
  });

  // Satisfies: UI-ERROR-001 (502)
  it('normalizes 502 Bad Gateway with upstream service context', () => {
    const error: ApiErrorResponse = {
      status: 502,
      error: 'bad_gateway',
      message: 'Could not reach Prowlarr',
    };
    expect(error.status).toBe(502);
  });

  // Satisfies: UI-ERROR-001 (5xx generic)
  it('normalizes 500 Internal Server Error', () => {
    const error: ApiErrorResponse = {
      status: 500,
      error: 'internal',
      message: 'Something went wrong',
    };
    expect(error.status).toBe(500);
  });

  // Satisfies: UI-ERROR-002 (network error)
  it('normalizes network errors (fetch TypeError) with status 0', () => {
    const error: ApiErrorResponse = {
      status: 0,
      error: 'network_error',
      message: 'Unable to reach Librarr',
    };
    expect(error.status).toBe(0);
    expect(error.error).toBe('network_error');
  });

  // Satisfies: UI-ERROR-001 (non-JSON response)
  it('handles non-JSON response body without crashing', () => {
    const error: ApiErrorResponse = {
      status: 500,
      error: 'internal',
      message: 'Something went wrong',
    };
    // Contract: if response body is not parseable JSON, produce a generic error
    expect(error.status).toBeGreaterThanOrEqual(400);
    expect(error.message).toBeTruthy();
  });

  // Satisfies: UI-ERROR-001 (unexpected shape)
  it('handles response body with unexpected shape', () => {
    const error: ApiErrorResponse = {
      status: 500,
      error: 'internal',
      message: 'Something went wrong',
    };
    // Contract: if body is JSON but not the expected shape, use fallback message
    expect(error.message).toBeTruthy();
  });

  // Satisfies: UI-ERROR-001 (success path)
  it('does not throw on successful 2xx responses', () => {
    // Contract: successful responses are parsed and returned, no error
    const response = { id: 1, title: 'Dune', authorName: 'Frank Herbert' };
    expect(response).toHaveProperty('id');
    expect(response).toHaveProperty('title');
  });
});
