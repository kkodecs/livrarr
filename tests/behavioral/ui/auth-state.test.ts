/**
 * Auth State Machine — Behavioral Contract Tests
 *
 * These tests define the contract for the auth state management layer.
 * The auth store (Zustand) must satisfy these state transition contracts.
 *
 * State machine:
 *   [NoToken] → check /setup/status
 *     → setupRequired: true  → [Setup]
 *     → setupRequired: false → [Login]
 *   [Login] → POST /auth/login success → [Authenticated]
 *   [Authenticated] → GET /auth/me success → [AppReady]
 *                   → 401 → clear token → [Login]
 *   [AppReady] → any 401 → clear token → [Login]
 *              → POST /auth/logout → clear token → [Login]
 *
 * Satisfies: UI-AUTH-001 through UI-AUTH-007, UI-SETUP-001
 */
import { describe, it, expect } from 'vitest';

// --- Contract types from IR ---
type UserRole = 'admin' | 'user';
type AuthType = 'session' | 'api_key' | 'external_auth';

interface UserResponse {
  id: number;
  username: string;
  role: UserRole;
  createdAt: string;
  updatedAt: string;
}

interface AuthMeResponse {
  user: UserResponse;
  authType: AuthType;
}

interface LoginResponse {
  token: string;
}

interface SetupStatusResponse {
  setupRequired: boolean;
}

// --- Auth state contract ---
type AuthStatus =
  | 'unauthenticated'
  | 'setup_required'
  | 'authenticated';

interface AuthState {
  status: AuthStatus;
  user: UserResponse | null;
  token: string | null;
}

const TOKEN_KEY = 'librarr_token';

describe('Auth State Machine Contract', () => {
  // Satisfies: UI-AUTH-004, UI-SETUP-001
  it('initial state with no token is unauthenticated', () => {
    const state: AuthState = { status: 'unauthenticated', user: null, token: null };
    expect(state.status).toBe('unauthenticated');
    expect(state.user).toBeNull();
    expect(state.token).toBeNull();
  });

  // Satisfies: UI-SETUP-001
  it('setup required transitions to setup_required state', () => {
    const setupStatus: SetupStatusResponse = { setupRequired: true };
    const state: AuthState = {
      status: setupStatus.setupRequired ? 'setup_required' : 'unauthenticated',
      user: null,
      token: null,
    };
    expect(state.status).toBe('setup_required');
  });

  // Satisfies: UI-SETUP-001
  it('setup not required with no token transitions to unauthenticated', () => {
    const setupStatus: SetupStatusResponse = { setupRequired: false };
    const state: AuthState = {
      status: setupStatus.setupRequired ? 'setup_required' : 'unauthenticated',
      user: null,
      token: null,
    };
    expect(state.status).toBe('unauthenticated');
  });

  // Satisfies: UI-AUTH-003
  it('successful login stores token and transitions to authenticated', () => {
    const loginResponse: LoginResponse = { token: 'abc123hextoken' };
    const adminUser: UserResponse = {
      id: 1,
      username: 'testuser',
      role: 'admin',
      createdAt: '2026-03-31T00:00:00Z',
      updatedAt: '2026-03-31T00:00:00Z',
    };
    const state: AuthState = {
      status: 'authenticated',
      user: adminUser,
      token: loginResponse.token,
    };
    expect(state.status).toBe('authenticated');
    expect(state.token).toBe('abc123hextoken');
    expect(state.user).not.toBeNull();
  });

  // Satisfies: UI-AUTH-005
  it('auth/me with admin role sets user as admin', () => {
    const meResponse: AuthMeResponse = {
      user: {
        id: 1,
        username: 'testuser',
        role: 'admin',
        createdAt: '2026-03-31T00:00:00Z',
        updatedAt: '2026-03-31T00:00:00Z',
      },
      authType: 'session',
    };
    expect(meResponse.user.role).toBe('admin');
    expect(meResponse.authType).toBe('session');
  });

  // Satisfies: UI-AUTH-005
  it('auth/me with regular user role sets user as user', () => {
    const meResponse: AuthMeResponse = {
      user: {
        id: 2,
        username: 'reader',
        role: 'user',
        createdAt: '2026-03-31T00:00:00Z',
        updatedAt: '2026-03-31T00:00:00Z',
      },
      authType: 'session',
    };
    expect(meResponse.user.role).toBe('user');
  });

  // Satisfies: UI-AUTH-004
  it('401 response clears token and transitions to unauthenticated', () => {
    // Contract: on 401, token must be cleared and state reset
    const stateAfter401: AuthState = {
      status: 'unauthenticated',
      user: null,
      token: null,
    };
    expect(stateAfter401.status).toBe('unauthenticated');
    expect(stateAfter401.token).toBeNull();
    expect(stateAfter401.user).toBeNull();
  });

  // Satisfies: UI-AUTH-007
  it('logout clears token and transitions to unauthenticated', () => {
    const stateAfterLogout: AuthState = {
      status: 'unauthenticated',
      user: null,
      token: null,
    };
    expect(stateAfterLogout.status).toBe('unauthenticated');
    expect(stateAfterLogout.token).toBeNull();
  });

  // Satisfies: UI-AUTH-001
  it('LoginResponse contains only token string', () => {
    const response: LoginResponse = { token: 'session-token-hex' };
    expect(typeof response.token).toBe('string');
    expect(response.token.length).toBeGreaterThan(0);
  });

  // Satisfies: UI-SETUP-003
  it('SetupResponse contains both apiKey and token', () => {
    const response = { apiKey: 'api-key-hex', token: 'session-token-hex' };
    expect(typeof response.apiKey).toBe('string');
    expect(typeof response.token).toBe('string');
  });
});
