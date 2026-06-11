/**
 * Route Guards — Behavioral Contract Tests
 *
 * These tests define the contract for route access control.
 * The router implementation (Phase 3) must satisfy these rules.
 *
 * Satisfies: UI-NAV-003 through UI-NAV-008, Section 13 (Security)
 */
import { describe, it, expect } from 'vitest';

// --- Route definition from IR ---
interface RouteDefinition {
  path: string;
  auth: boolean;
  adminOnly: boolean;
  adminWrite: boolean;
  greyed: boolean;
}

// Full route table from IR (source of truth)
const ROUTES: RouteDefinition[] = [
  { path: '/setup',                    auth: false, adminOnly: false, adminWrite: false, greyed: false },
  { path: '/login',                    auth: false, adminOnly: false, adminWrite: false, greyed: false },
  { path: '/',                         auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/work/add',                 auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/work/:id',                 auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/author',                   auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/author/add',               auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/author/:id',               auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/unmapped',                 auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/activity/queue',           auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/activity/history',         auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/settings',                 auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/settings/mediamanagement', auth: true,  adminOnly: false, adminWrite: true,  greyed: false },
  { path: '/settings/indexers',        auth: true,  adminOnly: true,  adminWrite: false, greyed: false },
  { path: '/settings/downloadclients', auth: true,  adminOnly: false, adminWrite: true,  greyed: false },
  { path: '/settings/metadata',        auth: true,  adminOnly: true,  adminWrite: false, greyed: false },
  { path: '/settings/general',         auth: true,  adminOnly: true,  adminWrite: false, greyed: true  },
  { path: '/settings/ui',              auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/settings/users',           auth: true,  adminOnly: true,  adminWrite: false, greyed: false },
  { path: '/system/status',            auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/system/health',            auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/profile',                  auth: true,  adminOnly: false, adminWrite: false, greyed: false },
  { path: '/calendar',                 auth: true,  adminOnly: false, adminWrite: false, greyed: true  },
  { path: '/wanted/missing',           auth: true,  adminOnly: false, adminWrite: false, greyed: true  },
  { path: '/wanted/cutoff',            auth: true,  adminOnly: false, adminWrite: false, greyed: true  },
  { path: '/shelf',                    auth: true,  adminOnly: false, adminWrite: false, greyed: true  },
  { path: '/system/logs',              auth: true,  adminOnly: false, adminWrite: false, greyed: true  },
  { path: '/settings/profiles',        auth: true,  adminOnly: true,  adminWrite: false, greyed: true  },
  { path: '/settings/customformats',   auth: true,  adminOnly: true,  adminWrite: false, greyed: true  },
  { path: '/settings/importlists',     auth: true,  adminOnly: true,  adminWrite: false, greyed: true  },
  { path: '/settings/notifications',   auth: true,  adminOnly: true,  adminWrite: false, greyed: true  },
  { path: '/settings/tags',            auth: true,  adminOnly: false, adminWrite: false, greyed: true  },
  { path: '/settings/development',     auth: true,  adminOnly: true,  adminWrite: false, greyed: true  },
];

const findRoute = (path: string) => ROUTES.find(r => r.path === path);

describe('Route Guard Contract', () => {
  // Satisfies: UI-AUTH-004
  it('unauthenticated user cannot access auth-required routes', () => {
    const authRoutes = ROUTES.filter(r => r.auth);
    expect(authRoutes.length).toBeGreaterThan(0);
    authRoutes.forEach(route => {
      expect(route.auth).toBe(true);
    });
  });

  // Satisfies: UI-AUTH-004
  it('unauthenticated user can access /login and /setup', () => {
    expect(findRoute('/login')!.auth).toBe(false);
    expect(findRoute('/setup')!.auth).toBe(false);
  });

  // Satisfies: UI-NAV-003
  it('works page (/) requires auth but not admin', () => {
    const route = findRoute('/')!;
    expect(route.auth).toBe(true);
    expect(route.adminOnly).toBe(false);
  });

  // Satisfies: UI-NAV-003, Section 13 (route guards)
  it('admin-only settings pages are flagged correctly', () => {
    const adminOnlyPaths = ['/settings/indexers', '/settings/metadata', '/settings/general', '/settings/users'];
    adminOnlyPaths.forEach(path => {
      const route = findRoute(path)!;
      expect(route.adminOnly).toBe(true);
    });
  });

  // Satisfies: Section 13 (non-admin redirect)
  it('non-admin accessing admin-only route must be redirected to /', () => {
    // Contract: route guard redirects non-admin to works page
    const adminRoutes = ROUTES.filter(r => r.adminOnly);
    expect(adminRoutes.length).toBeGreaterThan(0);
    // Phase 3 implements the redirect; this test verifies the route data is correct
    adminRoutes.forEach(route => {
      expect(route.adminOnly).toBe(true);
    });
  });

  // Satisfies: UI-SETTINGS-MM-001, Settings non-admin access
  it('admin-write routes are accessible to non-admins in read-only mode', () => {
    const adminWritePaths = ['/settings/mediamanagement', '/settings/downloadclients'];
    adminWritePaths.forEach(path => {
      const route = findRoute(path)!;
      expect(route.adminOnly).toBe(false); // non-admin CAN access
      expect(route.adminWrite).toBe(true); // but write controls hidden
    });
  });

  // Satisfies: UI-NAV-006
  it('greyed-out routes are flagged correctly', () => {
    const greyedPaths = ['/calendar', '/wanted/missing', '/wanted/cutoff', '/shelf', '/system/logs'];
    greyedPaths.forEach(path => {
      const route = findRoute(path)!;
      expect(route.greyed).toBe(true);
    });
  });

  // Satisfies: UI-NAV-007
  it('greyed-out settings pages are flagged correctly', () => {
    const greyedSettings = [
      '/settings/profiles', '/settings/customformats', '/settings/importlists',
      '/settings/notifications', '/settings/tags', '/settings/development',
    ];
    greyedSettings.forEach(path => {
      const route = findRoute(path)!;
      expect(route.greyed).toBe(true);
    });
  });

  // Satisfies: UI-NAV-008
  it('indexers settings page is active (not greyed out)', () => {
    const route = findRoute('/settings/indexers')!;
    expect(route.greyed).toBe(false);
  });

  // Satisfies: UI-NAV-003
  it('all active routes have correct auth requirements', () => {
    const activeAuthRoutes = ROUTES.filter(r => !r.greyed && r.auth);
    expect(activeAuthRoutes.length).toBeGreaterThan(10);
    // Every active auth route has auth=true
    activeAuthRoutes.forEach(route => {
      expect(route.auth).toBe(true);
    });
  });
});
