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
  | "pending"
  | "partial"
  | "enriched"
  | "failed"
  | "exhausted"
  | "skipped";
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
  | "pathNotFound";
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
  | "fileDeleted";
export type DownloadClientImplementation = "qBittorrent" | "sabnzbd";
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
  seriesName: string | null;
  seriesPosition: number | null;
  genres: string[] | null;
  language: string | null;
  pageCount: number | null;
  durationSeconds: number | null;
  publisher: string | null;
  publishDate: string | null;
  olKey: string | null;
  hardcoverId: string | null;
  isbn13: string | null;
  asin: string | null;
  narrator: string[] | null;
  narrationType: NarrationType | null;
  abridged: boolean;
  rating: number | null;
  ratingCount: number | null;
  enrichmentStatus: EnrichmentStatus;
  enrichedAt: string | null;
  enrichmentSource: string | null;
  coverManual: boolean;
  monitored: boolean;
  addedAt: string;
  libraryItems: LibraryItemResponse[];
  metadataSource?: string | null;
  detailUrl?: string | null;
}

export interface LibraryItemResponse {
  id: number;
  path: string;
  mediaType: MediaType;
  fileSize: number;
  importedAt: string;
}

export interface DeleteWorkResponse {
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

export interface UpdateAuthorRequest {
  monitored?: boolean | null;
  monitorNewItems?: boolean | null;
}

export interface AuthorResponse {
  id: number;
  name: string;
  sortName: string | null;
  olKey: string | null;
  monitored: boolean;
  monitorNewItems: boolean;
  addedAt: string;
}

export interface AuthorDetailResponse {
  author: AuthorResponse;
  works: WorkDetailResponse[];
}

// Author Bibliography
export interface BibliographyEntry {
  olKey: string;
  title: string;
  year: number | null;
  seriesName?: string | null;
  seriesPosition?: number | null;
}

export interface AuthorBibliography {
  authorId: number;
  entries: BibliographyEntry[];
  fetchedAt: string;
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
  { code: "en", englishName: "English", providerName: "OpenLibrary + Hardcover", providerType: "api", requiresLlm: false, flag: "\u{1F1EC}\u{1F1E7}" },
  { code: "nl", englishName: "Dutch", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1F3}\u{1F1F1}" },
  { code: "fr", englishName: "French", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1EB}\u{1F1F7}" },
  { code: "de", englishName: "German", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1E9}\u{1F1EA}" },
  { code: "it", englishName: "Italian", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1EE}\u{1F1F9}" },
  { code: "ja", englishName: "Japanese", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1EF}\u{1F1F5}" },
  { code: "ko", englishName: "Korean", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1F0}\u{1F1F7}" },
  { code: "pl", englishName: "Polish", providerName: "lubimyczytac.pl", providerType: "llm", requiresLlm: true, flag: "\u{1F1F5}\u{1F1F1}" },
  { code: "es", englishName: "Spanish", providerName: "Web Search", providerType: "llm", requiresLlm: true, flag: "\u{1F1EA}\u{1F1F8}" },
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
  files: ScannedFile[];
  warnings: string[];
}

export interface ScannedFile {
  path: string;
  filename: string;
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
}

export interface ManualImportItem {
  path: string;
  olKey: string;
  title: string;
  author: string;
  deleteExisting: boolean;
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
