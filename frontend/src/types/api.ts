// API types derived from build/ir-livrarr-ui.ts
// All response fields are camelCase (Servarr convention).
// All dates are ISO 8601 strings. All IDs are numbers.

// Shared Enums
export type MediaType = "ebook" | "audiobook";
export type UserRole = "admin" | "user";
export type GrabStatus =
  | "sent"
  | "confirmed"
  | "importing"
  | "imported"
  | "importFailed"
  | "removed"
  | "failed";
export type EnrichmentStatus =
  | "unenriched"
  | "enriched"
  | "thin"
  | "failed";
export type IdentityStatus =
  | "pending"
  | "confirmed"
  | "provisional"
  | "conflict"
  | "needs_review"
  | "not_found";
export type QueueStatus =
  | "downloading"
  | "queued"
  | "paused"
  | "completed"
  | "warning"
  | "error";
export type NotificationType =
  | "newWorkDetected"
  | "workAutoAdded"
  | "metadataUpdated"
  | "bulkEnrichmentComplete"
  | "jobPanicked"
  | "rateLimitHit"
  | "pathNotFound"
  | "rssGrabbed"
  | "rssGrabFailed";
export type NarrationType = "human" | "ai" | "ai_authorized_replica";
export type AuthType = "session" | "api_key" | "external_auth";
export type HealthCheckType = "ok" | "warning" | "error";
export type EventType =
  | "grabbed"
  | "downloadCompleted"
  | "downloadFailed"
  | "imported"
  | "importFailed"
  | "enriched"
  | "enrichmentFailed"
  | "tagWritten"
  | "tagWriteFailed"
  | "fileDeleted"
  | "added"
  | "workDeleted"
  | "worksMerged"
  | "identityResolved";
export type DownloadClientImplementation = "qBittorrent" | "sabnzbd" | "transmission";
export type LlmProvider = "groq" | "gemini" | "openai" | "custom";

// Paginated response wrapper
export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  pageSize: number;
}

// Auth & Setup
export interface LoginRequest {
  username: string;
  password: string;
  rememberMe: boolean;
}

export interface LoginResponse {
  token: string;
}

export interface SetupRequest {
  username: string;
  password: string;
}

export interface SetupResponse {
  apiKey: string;
  token: string;
}

export interface SetupStatusResponse {
  setupRequired: boolean;
}

export interface UpdateProfileRequest {
  username?: string | null;
  password?: string | null;
}

export interface ApiKeyResponse {
  apiKey: string;
}

export interface AuthMeResponse {
  user: UserResponse;
  authType: AuthType;
}

// Users
export interface UserResponse {
  id: number;
  username: string;
  role: UserRole;
  createdAt: string;
  updatedAt: string;
}

export interface AdminCreateUserRequest {
  username: string;
  password: string;
  role: UserRole;
}

export interface AdminUpdateUserRequest {
  username?: string | null;
  password?: string | null;
  role?: UserRole | null;
}

// Works
export interface LookupResponse {
  results: WorkSearchResult[];
  filteredCount: number;
  rawCount: number;
  rawAvailable: boolean;
}

export interface WorkSearchResult {
  olKey: string | null;
  title: string;
  authorName: string;
  authorOlKey: string | null;
  year: number | null;
  coverUrl: string | null;
  description: string | null;
  seriesName?: string | null;
  seriesPosition?: number | null;
  source?: string | null;
  sourceType?: string | null;
  language?: string | null;
  detailUrl?: string | null;
  rating?: string | null;
  isbn13?: string | null;
  /** Opaque handle to the per-provider payloads cached during this search; echo
   * it back in the add request so the server reuses them network-free. */
  candidateId?: string | null;
  /** Federated work anchors carried from discovery so the add path trusts the
   * pick (no re-resolve). */
  hcKey?: string | null;
  grKey?: string | null;
  asin?: string | null;
}

export interface PreaddCoverCandidate {
  proxyUrl: string;
  source: string;
  title: string;
  authorName: string;
}

export interface AddWorkRequest {
  olKey?: string | null;
  title: string;
  authorName: string;
  authorOlKey?: string | null;
  year?: number | null;
  coverUrl?: string | null;
  metadataSource?: string | null;
  language?: string | null;
  detailUrl?: string | null;
  coverManual?: boolean;
  isbn13?: string | null;
  /** Echoed from the selected search result so the server reuses the cached
   * provider payloads instead of re-querying. */
  candidateId?: string | null;
  hcKey?: string | null;
  grKey?: string | null;
  asin?: string | null;
}

export interface AddWorkResponse {
  work: WorkDetailResponse;
  authorCreated: boolean;
  messages: string[];
}

export interface RefreshWorkResponse {
  work: WorkDetailResponse;
  messages: string[];
}

export interface UpdateWorkRequest {
  title?: string | null;
  authorName?: string | null;
  seriesName?: string | null;
  seriesPosition?: number | null;
  monitorEbook?: boolean | null;
  monitorAudiobook?: boolean | null;
}

export interface WorkDetailResponse {
  id: number;
  title: string;
  sortTitle: string | null;
  subtitle: string | null;
  originalTitle: string | null;
  authorName: string;
  authorId: number | null;
  description: string | null;
  year: number | null;
  seriesId: number | null;
  seriesName: string | null;
  seriesPosition: number | null;
  genres: string[] | null;
  language: string | null;
  pageCount: number | null;
  durationSeconds: number | null;
  publisher: string | null;
  publishDate: string | null;
  olKey: string | null;
  hcKey: string | null;
  grKey: string | null;
  isbn13: string | null;
  asin: string | null;
  narrator: string[] | null;
  narrationType: NarrationType | null;
  abridged: boolean;
  rating: number | null;
  ratingCount: number | null;
  enrichmentStatus: EnrichmentStatus;
  enriching: boolean;
  identityStatus: IdentityStatus;
  /** True while open identity conflicts pause re-matching and enrichment. */
  parkedByConflicts: boolean;
  enrichedAt: string | null;
  enrichmentSource: string | null;
  coverManual: boolean;
  coverSource: string | null;
  coverTrust: string;
  coverWidth: number;
  coverHeight: number;
  audiobookCoverUrl: string | null;
  audiobookCoverSource: string | null;
  audiobookCoverTrust: string;
  audiobookCoverWidth: number;
  audiobookCoverHeight: number;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  addedAt: string;
  libraryItems: LibraryItemResponse[];
  metadataSource?: string | null;
  detailUrl?: string | null;
  coverMtime?: number | null;
  audiobookCoverMtime?: number | null;
}

export interface LibraryItemResponse {
  id: number;
  path: string;
  mediaType: MediaType;
  fileSize: number;
  importedAt: string;
  progressPct: number | null;
  durationSeconds: number | null;
  finishedAt: string | null;
}

export interface ChapterResponse {
  id: number;
  chapterIndex: number;
  title: string;
  startTimeSecs: number;
  endTimeSecs: number;
}

export interface BookmarkResponse {
  id: number;
  libraryItemId: number;
  mediaType: string;
  position: string;
  sortKey: number;
  name: string;
  chapterTitle: string | null;
  pairedBookmarkId: number | null;
  createdAt: string;
}

export interface CreateBookmarkRequest {
  position: string;
  sortKey: number;
  name: string;
  chapterTitle?: string | null;
}

export interface RenameBookmarkRequest {
  name: string;
}

export interface DeleteWorkResponse {
  warnings: string[];
}

// Merge duplicates
export type MergeableField = "series_name" | "series_position";
export type MergeFieldChoice = "keep_survivor" | "take_loser";

export interface MergeConflict {
  field: MergeableField;
  survivorValue: string;
  loserValue: string;
}

export interface MergePreviewResponse {
  survivorId: number;
  loserId: number;
  libraryItemsMoving: number;
  grabsMoving: number;
  monitorEbookResult: boolean;
  monitorAudiobookResult: boolean;
  /** Fields needing an explicit choice before the merge can execute. */
  conflicts: MergeConflict[];
}

export interface MergeChoiceEntry {
  field: MergeableField;
  choice: MergeFieldChoice;
}

export interface MergeWorksRequest {
  choices: MergeChoiceEntry[];
}

export interface MergeWorksResponse {
  survivor: WorkDetailResponse;
  libraryItemsMoved: number;
  grabsMoved: number;
  warnings: string[];
}

// Authors
export interface AuthorSearchResult {
  olKey: string;
  name: string;
  sortName: string | null;
}

export interface AddAuthorRequest {
  name: string;
  sortName: string | null;
  olKey: string;
}

/** Monitoring and language only. Provider keys are NOT settable here: routes
 * are changed through the dedicated author-route endpoints, never through a
 * generic author update. */
export interface UpdateAuthorRequest {
  monitored?: boolean | null;
  monitorNewItems?: boolean | null;
  monitorLanguage?: string | null;
}

export interface AuthorResponse {
  id: number;
  name: string;
  sortName: string | null;
  /** Compatibility projection of the route ledger, not a stored column. */
  olKey: string | null;
  grKey: string | null;
  hcKey: string | null;
  /** Active routes first, then the removal history. */
  routes: AuthorRouteResponse[];
  nameVariants: AuthorNameVariantResponse[];
  linkState: AuthorLinkState;
  /** True only with an active Open Library route — a Goodreads or Hardcover
   * route makes an author linked, never monitorable. */
  monitorable: boolean;
  monitored: boolean;
  monitorNewItems: boolean;
  monitorLanguage: string | null;
  addedAt: string;
}

export interface AuthorDetailResponse {
  author: AuthorResponse;
  works: WorkDetailResponse[];
}

// --- Author provider linking ---
// The route/name/candidate surface. Response casing is NOT uniform here: the
// author, route and name-variant DTOs are camelCase like the rest of the API,
// while the candidate rows and the sweep counters are the domain types
// serialized as-is, i.e. snake_case. These declarations mirror the wire.

export type AuthorProvider = "open_library" | "goodreads" | "hardcover";

export type AuthorRouteState = "active" | "removed";

export type AuthorRouteProvenance =
  | "legacy_unguarded"
  | "tier1_inherited"
  | "readarr_guarded"
  | "user_picked"
  | "merge_coalesced";

export type AuthorLinkState = "linked" | "needs_review" | "unlinked";

export type AuthorNameSource =
  | "user"
  | "goodreads"
  | "hardcover"
  | "google_books"
  | "open_library"
  | "readarr"
  | "import"
  | "legacy";

/** The shared author-name authority's verdict. Serialized unrenamed. */
export type AuthorVerdict = "Agree" | "Grey" | "Disagree" | "Abstain";

export type AuthorCandidateCatalogState =
  | "pending"
  | "partial"
  | "retrying"
  | "complete"
  | "unavailable";

export type AuthorLinkCandidateReason =
  | "tier2_name_search"
  | "name_guard_failed"
  | "readarr_name_guard_failed"
  | "tombstoned"
  | "legacy_contradiction"
  | "ownership_collision"
  | "invalid_legacy_route";

export type AuthorLinkCandidateStatus =
  | "pending"
  | "dismissed"
  | "picked"
  | "superseded";

/** Externally-tagged provider key, exactly as the domain enum serializes. */
export type AuthorRouteKey =
  | { open_library: string }
  | { goodreads: number }
  | { hardcover: number };

export interface AuthorRouteResponse {
  id: number;
  provider: AuthorProvider;
  value: string;
  state: AuthorRouteState;
  provenance: AuthorRouteProvenance;
  /** Set on a removed route: the tombstone no automatic process may undo. */
  removedAt: string | null;
}

export interface AuthorNameVariantResponse {
  id: number;
  name: string;
  source: AuthorNameSource;
  /** The user's own choice, not whatever ranking happens to be showing. */
  selected: boolean;
}

export interface AuthorCandidateAlternateNameEvidence {
  name: string;
  verdict: AuthorVerdict;
}

/** A parked route the user is being asked about. snake_case on the wire. */
export interface AuthorLinkCandidate {
  id: number;
  author_id: number;
  key: AuthorRouteKey;
  candidate_name: string;
  reason: AuthorLinkCandidateReason;
  name_verdict: AuthorVerdict;
  primary_name_verdict: AuthorVerdict;
  alternate_name_evidence: AuthorCandidateAlternateNameEvidence[];
  /** A fetch-order hint the provider volunteered. Never evidence, never counted. */
  top_work_preview: string | null;
  catalog_evidence_state: AuthorCandidateCatalogState;
  corroborated_title_count: number;
  settled_work_count: number;
  previously_removed: boolean;
  status: AuthorLinkCandidateStatus;
  evidence_generation: number;
  observed_at: string;
}

export interface AuthorLinkReview {
  author: AuthorResponse;
  candidates: AuthorLinkCandidate[];
}

/** Persisted sweep counters. snake_case on the wire. */
export interface AuthorSweepProgress {
  total: number;
  completed: number;
  queued: number;
  running: number;
  parked: number;
  needs_review: number;
  retryable_failures: number;
  key_retryable: number;
  key_skipped: number;
  key_layout_drift: number;
  would_have_linked_at_090: number;
  oldest_due_at: string | null;
}

export interface AuthorMergeReport {
  worksMoved: number;
  seriesMoved: number;
  seriesFolded: number;
}

// Author Bibliography
export interface BibliographyEntry {
  olKey: string | null;
  title: string;
  year: number | null;
  seriesName?: string | null;
  seriesPosition?: number | null;
  alreadyInLibrary?: boolean;
  /** ISO 639-1 code if a real edition's language was confirmed; `null` means
   * unknown — shown by default, not treated as foreign. */
  language?: string | null;
}

export interface AuthorBibliography {
  authorId: number;
  entries: BibliographyEntry[];
  llmFiltered: boolean;
  rawAvailable: boolean;
  filteredCount: number;
  rawCount: number;
  fetchedAt: string;
}

// Series
export interface SeriesResponse {
  id: number | null;
  name: string;
  grKey: string;
  bookCount: number;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  worksInLibrary: number;
  /** ISO 639-1 code if a confident Google Books match was found; `null`
   * means unknown — shown by default, not treated as foreign. */
  language?: string | null;
}

export interface SeriesListResponse {
  series: SeriesResponse[];
  fetchedAt: string | null;
  rawAvailable: boolean;
  filteredCount: number;
  rawCount: number;
}

export interface MonitorSeriesRequest {
  grKey: string;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  language?: string | null;
}

export interface UpdateSeriesRequest {
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  language?: string | null;
}

export interface GrAuthorCandidate {
  grKey: string;
  name: string;
  profileUrl: string;
}

export interface PromoteSeriesRequest {
  grKey?: string | null;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  language?: string | null;
}

export interface PromoteSeriesResponse {
  status: "monitoring" | "needsAuthorResolution" | "needsPicker";
  authorId: number;
  series?: SeriesResponse;
  candidates?: SeriesResponse[];
}

export interface SeriesWithAuthorResponse {
  id: number;
  name: string;
  grKey: string;
  bookCount: number;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  monitorLanguage: string | null;
  suggestedLanguage: string | null;
  worksInLibrary: number;
  authorId: number;
  authorName: string;
  firstWorkId: number | null;
}

export interface SeriesDetailResponse {
  id: number;
  name: string;
  grKey: string;
  bookCount: number;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
  authorId: number;
  authorName: string;
  works: WorkDetailResponse[];
}

/** The wire still carries `autoLinked`, but it is always false — the
 * name-similarity auto-link is gone. Leaving it out of the type is what stops
 * a caller branching on it again. */
export interface ResolveGrResponse {
  candidates: GrAuthorCandidate[];
}

export interface SeriesBookRowResponse {
  position: number | null;
  inLibrary: boolean;
  title: string;
  year?: number | null;
  work?: WorkDetailResponse;
}

export interface SeriesBooksResponse {
  rosterAvailable: boolean;
  rows: SeriesBookRowResponse[];
}

// Notifications
export interface NotificationResponse {
  id: number;
  notificationType: NotificationType;
  refKey: string | null;
  message: string;
  data: Record<string, unknown>;
  read: boolean;
  createdAt: string;
}

// Identity review (AC-013 grey-park surface): a work parked "needs review"
// with its persisted, real-scored candidates.
export interface IdentityReviewCandidate {
  candidateId: string;
  title: string;
  authorName: string;
  language: string | null;
  olKey: string | null;
  grKey: string | null;
  hcKey: string | null;
  isbn13: string | null;
  asin: string | null;
  coverUrl: string | null;
  sources: string[];
  titleJaccard: number;
  authorOverlap: number;
  existingWorkId: number | null;
}

export interface IdentityReviewPark {
  workId: number;
  title: string;
  authorName: string;
  coverUrl: string | null;
  candidates: IdentityReviewCandidate[];
}

export interface ResolveIdentityReviewRequest {
  candidateId: string;
}

// Identity conflicts (AC-021 work-key contradictions). Field names are
// camelCase like every other endpoint. Enum VALUES (action/status/kind/source)
// are snake_case strings, and the nested `incoming` payload keeps snake_case
// keys — it serializes the domain type IncomingConflictPayload, whose JSON
// shape is also persisted inside DB conflict rows.
export type ConflictResolutionAction =
  | "keep_existing"
  | "accept_separate"
  | "replace_anchor"
  | "merge";
export type ConflictStatus = "open" | "resolved" | "dismissed";
export type ConflictSource =
  | "manual_add"
  | "manual_import"
  | "list_import"
  | "readarr_import"
  | "series_monitor"
  | "author_monitor"
  | "refresh"
  | "manual_retry"
  | "convergence";
export type IdentityConflictKind =
  | "incoming_different_ol_key"
  | "incoming_different_gr_key"
  | "incoming_different_hc_key"
  | "ol_redirect_collision"
  | "quorum_tie";

export interface IdentityConflictSummary {
  id: number;
  existingWorkId: number;
  kind: string;
  incomingTitle: string;
  incomingAuthor: string;
  incomingOlKey: string | null;
  raisedAt: string;
  raisedBy: string;
  status: string;
}

export interface IdentityConflictDetail {
  id: number;
  existingWorkId: number;
  kind: IdentityConflictKind;
  // Domain-type payload — snake_case keys (see section comment above).
  incoming: {
    ol_key: string | null;
    gr_key: string | null;
    hc_key: string | null;
    isbn_13: string | null;
    asin: string | null;
    title: string;
    author_name: string;
    year: number | null;
    cover_url: string | null;
    top_candidates: unknown[];
  };
  raisedAt: string;
  raisedBy: ConflictSource;
  raisedSourcePath: string | null;
  status: ConflictStatus;
  resolvedAt: string | null;
  resolutionAction: ConflictResolutionAction | null;
  resolutionNotes: string | null;
}

export interface ResolveIdentityConflictRequest {
  action: ConflictResolutionAction;
  notes?: string;
}

// Queue
export interface QueueProgress {
  percent: number;
  eta: number | null;
  downloadStatus: string;
}

export interface QueueItemResponse {
  id: number;
  title: string;
  status: GrabStatus;
  size: number | null;
  mediaType: MediaType | null;
  indexer: string;
  downloadClient: string;
  workId: number;
  protocol: string;
  error: string | null;
  grabbedAt: string;
  progress: QueueProgress | null;
}

export interface QueueListResponse {
  items: QueueItemResponse[];
  total: number;
  page: number;
  perPage: number;
}

// Releases
export interface ReleaseSearchResponse {
  results: ReleaseResponse[];
  warnings?: SearchWarning[];
  cacheAgeSeconds?: number;
  searchQuery: string;
}

export interface SearchWarning {
  indexer: string;
  error: string;
}

export interface ReleaseResponse {
  title: string;
  indexer: string;
  size: number;
  guid: string;
  downloadUrl: string;
  seeders: number | null;
  leechers: number | null;
  publishDate: string | null;
  protocol: "torrent" | "usenet";
  categories: number[];
  format: string | null;
}

export interface GrabRequest {
  workId: number;
  downloadUrl: string;
  title: string;
  indexer: string;
  guid: string;
  size: number;
  downloadClientId?: number | null;
  protocol?: "torrent" | "usenet" | null;
  categories?: number[];
}

// History
export interface HistoryResponse {
  id: number;
  workId: number | null;
  eventType: EventType;
  data: Record<string, unknown>;
  date: string;
}

// Root Folders
export interface RootFolderResponse {
  id: number;
  path: string;
  mediaType: MediaType;
  freeSpace: number | null;
  totalSpace: number | null;
}

// Download Clients
export interface DownloadClientResponse {
  id: number;
  name: string;
  implementation: DownloadClientImplementation;
  host: string;
  port: number;
  useSsl: boolean;
  skipSslValidation: boolean;
  urlBase: string | null;
  username: string | null;
  category: string;
  downloadDir: string | null;
  enabled: boolean;
  clientType: string;
  apiKeySet: boolean;
  isDefaultForProtocol: boolean;
}

export interface CreateDownloadClientRequest {
  name: string;
  implementation: DownloadClientImplementation;
  host: string;
  port: number;
  useSsl: boolean;
  skipSslValidation: boolean;
  urlBase: string | null;
  username: string | null;
  password: string | null;
  category: string;
  downloadDir?: string | null;
  enabled: boolean;
  apiKey?: string | null;
  isDefaultForProtocol?: boolean;
}

export interface UpdateDownloadClientRequest {
  name?: string | null;
  host?: string | null;
  port?: number | null;
  useSsl?: boolean | null;
  skipSslValidation?: boolean | null;
  urlBase?: string | null;
  username?: string | null;
  password?: string | null;
  category?: string | null;
  downloadDir?: string | null;
  enabled?: boolean | null;
  apiKey?: string | null;
  isDefaultForProtocol?: boolean | null;
}

// Remote Path Mappings
export interface RemotePathMappingResponse {
  id: number;
  host: string;
  remotePath: string;
  localPath: string;
}

export interface CreateRemotePathMappingRequest {
  host: string;
  remotePath: string;
  localPath: string;
}

export interface UpdateRemotePathMappingRequest {
  host?: string | null;
  remotePath?: string | null;
  localPath?: string | null;
}

// Config
export interface NamingConfigResponse {
  authorFolderFormat: string;
  bookFolderFormat: string;
  renameFiles: boolean;
  replaceIllegalChars: boolean;
}

export interface MediaManagementConfigResponse {
  cwaIngestPath: string | null;
  preferredEbookFormats: string[];
  preferredAudiobookFormats: string[];
}

export interface UpdateMediaManagementConfigRequest {
  cwaIngestPath: string | null;
  preferredEbookFormats: string[];
  preferredAudiobookFormats: string[];
}

// Indexers
export interface IndexerResponse {
  id: number;
  name: string;
  protocol: "torrent" | "usenet";
  url: string;
  apiPath: string;
  apiKeySet: boolean;
  categories: number[];
  priority: number;
  enableAutomaticSearch: boolean;
  enableInteractiveSearch: boolean;
  supportsBookSearch: boolean;
  enableRss: boolean;
  enabled: boolean;
  addedAt: string;
}

export interface CreateIndexerRequest {
  name: string;
  protocol?: "torrent" | "usenet";
  url: string;
  apiPath?: string;
  apiKey?: string | null;
  categories?: number[];
  priority?: number;
  enableAutomaticSearch?: boolean;
  enableInteractiveSearch?: boolean;
  enableRss?: boolean;
  enabled?: boolean;
}

export interface UpdateIndexerRequest {
  name?: string | null;
  url?: string | null;
  apiPath?: string | null;
  apiKey?: string | null;
  categories?: number[] | null;
  priority?: number | null;
  enableAutomaticSearch?: boolean | null;
  enableInteractiveSearch?: boolean | null;
  enableRss?: boolean | null;
  enabled?: boolean | null;
}

export interface TestIndexerRequest {
  url: string;
  apiPath: string;
  apiKey?: string | null;
}

export interface TestIndexerResponse {
  ok: boolean;
  supportsBookSearch: boolean;
  warnings?: string[];
  error?: string | null;
}

export interface IndexerConfigResponse {
  rssSyncIntervalMinutes: number;
  rssMatchThreshold: number;
  rssGrabFailureLimit: number;
}

export interface UpdateIndexerConfigRequest {
  rssSyncIntervalMinutes?: number;
  rssMatchThreshold?: number;
  rssGrabFailureLimit?: number;
}

export interface ProwlarrConfigResponse {
  url: string | null;
  apiKeySet: boolean;
  enabled: boolean;
}

export interface ProwlarrImportRequest {
  url: string;
  apiKey: string;
}

export interface ProwlarrImportResponse {
  imported: number;
  skipped: number;
  errors: string[];
}

export interface EmailConfigResponse {
  enabled: boolean;
  smtpHost: string;
  smtpPort: number;
  encryption: string;
  username: string | null;
  passwordSet: boolean;
  fromAddress: string | null;
  recipientEmail: string | null;
  sendOnImport: boolean;
}

export interface UpdateEmailConfigRequest {
  enabled?: boolean;
  smtpHost?: string;
  smtpPort?: number;
  encryption?: string;
  username?: string | null;
  password?: string | null;
  fromAddress?: string | null;
  recipientEmail?: string | null;
  sendOnImport?: boolean;
}

export interface MetadataConfigResponse {
  hardcoverEnabled: boolean;
  hardcoverApiTokenSet: boolean;
  llmEnabled: boolean;
  llmProvider: LlmProvider | null;
  llmEndpoint: string | null;
  llmApiKeySet: boolean;
  llmModel: string | null;
  audnexusUrl: string;
  languages: string[];
  googleBooksApiKeySet: boolean;
  providerStatus?: Record<string, string>;
}

export interface LanguageInfo {
  code: string;
  englishName: string;
  providerName: string;
  providerType: "api" | "llm";
  requiresLlm: boolean;
  flag: string;
}

/** All supported languages with their metadata providers. */
export const SUPPORTED_LANGUAGES: LanguageInfo[] = [
  { code: "en", englishName: "English", providerName: "OpenLibrary + Hardcover", providerType: "api", requiresLlm: false, flag: "\u{1F1FA}\u{1F1F8}" },
  { code: "nl", englishName: "Dutch", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1F3}\u{1F1F1}" },
  { code: "fr", englishName: "French", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1EB}\u{1F1F7}" },
  { code: "de", englishName: "German", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1E9}\u{1F1EA}" },
  { code: "it", englishName: "Italian", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1EE}\u{1F1F9}" },
  { code: "ja", englishName: "Japanese", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1EF}\u{1F1F5}" },
  { code: "ko", englishName: "Korean", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1F0}\u{1F1F7}" },
  { code: "pl", englishName: "Polish", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1F5}\u{1F1F1}" },
  { code: "es", englishName: "Spanish", providerName: "Google Books", providerType: "api", requiresLlm: false, flag: "\u{1F1EA}\u{1F1F8}" },
];

export interface UpdateMetadataConfigRequest {
  hardcoverEnabled?: boolean;
  hardcoverApiToken?: string | null;
  llmEnabled?: boolean;
  llmProvider?: LlmProvider | null;
  llmEndpoint?: string | null;
  llmApiKey?: string | null;
  llmModel?: string | null;
  audnexusUrl?: string | null;
  languages?: string[] | null;
  googleBooksApiKey?: string | null;
}

export interface DefaultLanguageResponse {
  defaultLanguage: string;
}

export interface UpdateDefaultLanguageRequest {
  defaultLanguage: string;
}

// System
export interface HealthCheckResult {
  source: string;
  checkType: HealthCheckType;
  message: string;
}

export interface SystemStatus {
  version: string;
  osInfo: string;
  dataDirectory: string;
  logFile: string;
  startupTime: string;
  logLevel: string;
  /** Current process resident memory in bytes; null when unavailable. */
  rssBytes?: number | null;
}

export interface HealthSummaryResponse {
  llm: LlmStatusInfo;
  indexers: InfraItemStatus[];
  downloadClients: InfraItemStatus[];
  rssSync: RssSyncStatus;
  metadataProviders: ProviderStatusInfo[];
  library: LibraryStats;
}

export interface LlmStatusInfo {
  configured: boolean;
  enabled: boolean;
  provider: string | null;
  model: string | null;
}

export interface InfraItemStatus {
  id: number;
  name: string;
  implementation: string;
  enabled: boolean;
}

export interface RssSyncStatus {
  running: boolean;
  lastRunAt: string | null;
}

export interface ProviderStatusInfo {
  name: string;
  status: "ok" | "error";
  lastError: string | null;
}

export interface LibraryStats {
  workCount: number;
  libraryItemCount: number;
  totalSizeBytes: number;
}

// Unmapped Files
export interface ScanResult {
  matched: number;
  unmatched: ScanUnmatchedFile[];
  errors: ScanError[];
}

export interface ScanUnmatchedFile {
  path: string;
  mediaType: MediaType;
}

export interface ScanError {
  path: string;
  message: string;
}

// API Errors
export interface ApiErrorResponse {
  status: number;
  error: string;
  message: string;
  fieldErrors?: FieldError[];
  /** Structured machine-readable detail (identity-edit door contract):
   * stable `code` plus the collision owner when applicable. */
  details?: ApiErrorDetails;
}

export interface ApiErrorDetails {
  code:
    | "preview_required"
    | "anchor_collision"
    | "preview_capacity"
    | "pending_anchor_stale"
    | "identity_review_stale"
    | "identity_conflict_stale"
    | (string & {});
  owningWorkId?: number;
  owningWorkTitle?: string;
}

// --- Identity edit (preview-confirm; design identity-edit r4) ---

export type IdentitySlot = "gr_work" | "ol_work" | "hc_work" | "isbn_13" | "asin";

export interface IdentityPreviewRequest {
  input: string;
  slot: IdentitySlot | null;
}

export interface ResolvedPreviewRecord {
  title: string | null;
  author: string | null;
  year: number | null;
  language: string | null;
  coverUrl: string | null;
  slot: IdentitySlot;
  canonicalValue: string;
}

export interface SiblingAssessment {
  slot: IdentitySlot;
  action: "keep" | "drop";
  cause?: string;
}

export interface BridgeWarning {
  slot: IdentitySlot;
  message: string;
}

export interface IdentityCollision {
  owningWorkId: number;
  owningWorkTitle: string;
}

export interface IdentityPreviewResponse {
  resolved: ResolvedPreviewRecord | null;
  previewId?: string;
  siblings: SiblingAssessment[];
  bridgeWarnings: BridgeWarning[];
  collision?: IdentityCollision;
  conflictWarning: boolean;
  reason?: string;
}


export interface FieldError {
  field: string;
  message: string;
}

// Manual Import
export interface BrowseResponse {
  parent: string | null;
  directories: { name: string; path: string }[];
}

export interface ScanRequest {
  path: string;
}

export interface ScanResponse {
  scanId: string;
  files: ScannedFile[];
  warnings: string[];
  olTotal: number;
  olCompleted: number;
}

export interface ScanProgressResponse {
  files: ScannedFile[];
  warnings: string[];
  olTotal: number;
  olCompleted: number;
}

export interface ScannedFile {
  path: string;
  filename: string;
  /** Path relative to the scan root (e.g. `Author/Book/file.epub`), for folder context. */
  relPath: string;
  mediaType: MediaType;
  size: number;
  parsed: ParsedFile | null;
  match: OlMatch | null;
  existingWorkId: number | null;
  hasExistingMediaType: boolean;
  routable: boolean;
  /** Multi-file audiobook: all file paths in the group. */
  groupedPaths?: string[];
}

export interface ParsedFile {
  author: string;
  title: string;
  series: string | null;
  seriesPosition: number | null;
  language?: string;
}

export interface OlMatch {
  olKey: string;
  title: string;
  author: string;
  coverUrl: string | null;
  existingWorkId: number | null;
  // #97: federated anchors + reuse handle carried from discovery, so import can
  // land the work Confirmed (and reuse the cached payload) instead of ISBN-only.
  candidateId?: string | null;
  hcKey?: string | null;
  grKey?: string | null;
  asin?: string | null;
  isbn13?: string | null;
  year?: number | null;
  source?: string | null;
  language?: string | null;
}

export interface ManualImportItem {
  path: string;
  olKey: string;
  title: string;
  author: string;
  deleteExisting: boolean;
  language?: string;
  // #97: forward the picked match's anchors + cover so the work is created
  // Confirmed (it enriches directly, skipping the flaky ISBN convergence).
  candidateId?: string | null;
  hcKey?: string | null;
  grKey?: string | null;
  asin?: string | null;
  coverUrl?: string | null;
  isbn?: string | null;
  year?: number | null;
}

export interface ManualImportResponse {
  results: ManualImportResult[];
}

export interface ManualImportResult {
  path: string;
  status: "imported" | "skipped" | "failed";
  workId: number | null;
  error: string | null;
}

export interface ManualSearchRequest {
  query: string;
  author?: string;
}

export interface ManualSearchResponse {
  results: OlMatch[];
}

// Readarr Import types
export interface ReadarrRootFolder {
  id: number;
  name: string | null;
  path: string;
  accessible: boolean | null;
  freeSpace: number | null;
  totalSpace: number | null;
}

export interface ImportPreviewResponse {
  authorsToCreate: number;
  authorsExisting: number;
  worksToCreate: number;
  worksExisting: number;
  filesToImport: number;
  filesToSkip: number;
  skippedItems: ImportSkippedItem[];
  importFiles: ImportPreviewFileItem[];
}

export interface ImportPreviewFileItem {
  title: string;
  author: string;
  path: string;
  mediaType: string;
  workStatus: "new" | "existing";
}

export interface ImportSkippedItem {
  title: string;
  author: string;
  reason: string;
}

export interface ImportProgressResponse {
  running: boolean;
  importId: string | null;
  phase: string;
  authorsProcessed: number;
  authorsTotal: number;
  worksProcessed: number;
  worksTotal: number;
  filesProcessed: number;
  filesTotal: number;
  filesSkipped: number;
  errors: string[];
}

export interface ImportHistoryItem {
  id: string;
  source: string;
  status: string;
  startedAt: string;
  completedAt: string | null;
  authorsCreated: number;
  worksCreated: number;
  filesImported: number;
  filesSkipped: number;
  sourceUrl: string | null;
}

// List Imports (CSV: Goodreads, Hardcover)
export interface ListImportPreviewRow {
  rowIndex: number;
  title: string;
  author: string;
  isbn13: string | null;
  isbn10: string | null;
  year: number | null;
  sourceStatus: string | null;
  sourceRating: number | null;
  previewStatus: "new" | "already_exists" | "parse_error";
}

export interface ListImportPreviewResponse {
  previewId: string;
  source: string;
  totalRows: number;
  rows: ListImportPreviewRow[];
}

export interface ListImportConfirmRequest {
  previewId: string;
  rowIndices: number[];
  importId?: string;
  language?: string | null;
}

export interface ListImportConfirmRowResult {
  rowIndex: number;
  status: "added" | "already_exists" | "add_failed" | "lookup_error";
  message: string | null;
}

export interface ListImportConfirmResponse {
  importId: string;
  results: ListImportConfirmRowResult[];
}

export interface ListImportSummary {
  id: string;
  source: string;
  status: string;
  startedAt: string;
  completedAt: string | null;
  worksCreated: number;
}

export interface ListImportUndoResponse {
  worksRemoved: number;
}

// Cross-format resume
export interface ResumePromptDTO {
  format: "ebook" | "audiobook";
  position: string;
  label: string;
}

export interface AnchorDTO {
  cfi: string;
  ts: number;
}

/** A fuzzy-matched identifier guess held pending until the user confirms it. */
export interface PendingAnchorDTO {
  anchorType: string;
  value: string;
  setter: string;
}
