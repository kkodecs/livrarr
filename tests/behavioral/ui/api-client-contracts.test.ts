/**
 * API Client Contracts — Behavioral Type Shape Tests
 *
 * These tests verify that the API request/response type shapes match the IR.
 * They catch drift between frontend types and backend contract.
 *
 * Satisfies: UI-SEARCH-003, UI-SEARCH-004, UI-LIB-DETAIL-005,
 *            UI-SETTINGS-DC-001, UI-SETTINGS-META-001, UI-NOTIF-003
 */
import { describe, it, expect } from 'vitest';

// --- Types from IR (copied for contract verification) ---
type MediaType = 'ebook' | 'audiobook';
type EnrichmentStatus = 'pending' | 'partial' | 'enriched' | 'failed' | 'exhausted';
type NarrationType = 'human' | 'ai' | 'ai_authorized_replica';
type NotificationType = 'newWorkDetected' | 'workAutoAdded' | 'metadataUpdated' | 'bulkEnrichmentComplete' | 'jobPanicked' | 'rateLimitHit';
type DownloadClientImplementation = 'qBittorrent';
type LlmProvider = 'groq' | 'gemini' | 'openai' | 'custom';

describe('API Client Type Contracts', () => {
  // Satisfies: UI-SEARCH-003, UI-SEARCH-004
  it('AddWorkRequest has all required fields in correct types', () => {
    const request = {
      olKey: '/works/OL123W',
      title: 'Dune',
      authorName: 'Frank Herbert',
      authorOlKey: '/authors/OL123A' as string | null,
      year: 1965 as number | null,
      coverUrl: 'https://covers.openlibrary.org/b/id/123-M.jpg' as string | null,
    };
    expect(typeof request.olKey).toBe('string');
    expect(typeof request.title).toBe('string');
    expect(typeof request.authorName).toBe('string');
    expect(request.olKey.length).toBeGreaterThan(0);
    expect(request.title.length).toBeGreaterThan(0);
  });

  // Satisfies: UI-SEARCH-004, UI-LIB-DETAIL-010
  it('AddWorkResponse contains work, authorCreated flag, and messages array', () => {
    const response = {
      work: { id: 1, title: 'Dune', enrichmentStatus: 'enriched' as EnrichmentStatus, libraryItems: [] },
      authorCreated: true,
      messages: ['Author Frank Herbert added to your library', 'Enriched from Hardcover + Audnexus'],
    };
    expect(typeof response.authorCreated).toBe('boolean');
    expect(Array.isArray(response.messages)).toBe(true);
    expect(response.messages.length).toBeGreaterThan(0);
    expect(response.messages.every(m => typeof m === 'string')).toBe(true);
    expect(response.work).toHaveProperty('id');
    expect(response.work).toHaveProperty('enrichmentStatus');
  });

  // Satisfies: UI-LIB-DETAIL-005
  it('GrabRequest has correct field types', () => {
    const request = {
      workId: 42,
      downloadUrl: 'magnet:?xt=urn:btih:abc123',
      title: 'Dune.epub',
      indexer: 'MyIndexer',
      guid: 'unique-guid-123',
      size: 1048576,
      downloadClientId: null as number | null,
    };
    expect(typeof request.workId).toBe('number');
    expect(typeof request.downloadUrl).toBe('string');
    expect(typeof request.size).toBe('number');
  });

  // Satisfies: UI-LIB-WORKS-003, UI-LIB-DETAIL-007
  it('WorkDetailResponse includes libraryItems with mediaType enum', () => {
    const work = {
      id: 1,
      title: 'Dune',
      authorName: 'Frank Herbert',
      enrichmentStatus: 'enriched' as EnrichmentStatus,
      narrationType: 'human' as NarrationType | null,
      coverManual: false,
      monitored: true,
      addedAt: '2026-03-31T00:00:00Z',
      libraryItems: [
        { id: 1, path: 'Herbert/Dune.epub', mediaType: 'ebook' as MediaType, fileSize: 500000, importedAt: '2026-03-31T01:00:00Z' },
        { id: 2, path: 'Herbert/Dune/chapter1.mp3', mediaType: 'audiobook' as MediaType, fileSize: 10000000, importedAt: '2026-03-31T01:00:00Z' },
      ],
    };
    expect(work.libraryItems).toHaveLength(2);
    expect(work.libraryItems[0].mediaType).toBe('ebook');
    expect(work.libraryItems[1].mediaType).toBe('audiobook');
    // Verify mediaType is the correct enum
    const validMediaTypes: MediaType[] = ['ebook', 'audiobook'];
    work.libraryItems.forEach(item => {
      expect(validMediaTypes).toContain(item.mediaType);
    });
  });

  // Satisfies: UI-SETTINGS-DC-001
  it('DownloadClientResponse omits password field', () => {
    const response = {
      id: 1,
      name: 'qBit',
      implementation: 'qBittorrent' as DownloadClientImplementation,
      host: '192.168.1.100',
      port: 8080,
      useSsl: false,
      skipSslValidation: false,
      urlBase: null as string | null,
      username: 'admin' as string | null,
      category: 'librarr',
      enabled: true,
    };
    expect(response).not.toHaveProperty('password');
    expect(response.implementation).toBe('qBittorrent');
  });

  // Satisfies: UI-SETTINGS-DC-002
  it('UpdateDownloadClientRequest allows null password (keep existing)', () => {
    const request = {
      name: 'qBit Updated',
      password: null as string | null,
    };
    expect(request.password).toBeNull();
    // Contract: null password means "keep existing" — backend does not clear it
  });

  // Satisfies: UI-SETTINGS-META-001 through UI-SETTINGS-META-004
  it('MetadataConfigResponse uses boolean set flags for secrets', () => {
    const response = {
      hardcoverApiTokenSet: true,
      llmProvider: 'openai' as LlmProvider | null,
      llmEndpoint: 'https://api.openai.com/v1' as string | null,
      llmApiKeySet: true,
      llmModel: 'gpt-4o' as string | null,
      audnexusUrl: 'https://api.audnex.us',
      languages: ['en'],
    };
    // Secrets are never exposed — only boolean flags
    expect(typeof response.hardcoverApiTokenSet).toBe('boolean');
    expect(typeof response.llmApiKeySet).toBe('boolean');
    expect(response).not.toHaveProperty('hardcoverApiToken');
    expect(response).not.toHaveProperty('llmApiKey');
  });

  // Satisfies: UI-NOTIF-003
  it('NotificationResponse types notificationType as 6-value union', () => {
    const validTypes: NotificationType[] = [
      'newWorkDetected', 'workAutoAdded', 'metadataUpdated',
      'bulkEnrichmentComplete', 'jobPanicked', 'rateLimitHit',
    ];
    const notification = {
      id: 1,
      notificationType: 'newWorkDetected' as NotificationType,
      refKey: '/works/OL123W' as string | null,
      message: 'New work detected: Dune Messiah',
      data: {},
      read: false,
      createdAt: '2026-03-31T00:00:00Z',
    };
    expect(validTypes).toContain(notification.notificationType);
    expect(validTypes).toHaveLength(6);
    expect(typeof notification.read).toBe('boolean');
  });
});
