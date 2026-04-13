import { apiFetch, apiUpload } from "./client";
import type {
  SetupStatusResponse,
  SetupRequest,
  SetupResponse,
  LoginRequest,
  LoginResponse,
  AuthMeResponse,
  UpdateProfileRequest,
  ApiKeyResponse,
  UserResponse,
  AdminCreateUserRequest,
  AdminUpdateUserRequest,
  WorkSearchResult,
  AddWorkRequest,
  AddWorkResponse,
  WorkDetailResponse,
  UpdateWorkRequest,
  RefreshWorkResponse,
  DeleteWorkResponse,
  AuthorSearchResult,
  AddAuthorRequest,
  AuthorResponse,
  AuthorDetailResponse,
  UpdateAuthorRequest,
  AuthorBibliography,
  NotificationResponse,
  QueueListResponse,
  ReleaseSearchResponse,
  GrabRequest,
  HistoryResponse,
  EventType,
  RootFolderResponse,
  MediaType,
  DownloadClientResponse,
  CreateDownloadClientRequest,
  UpdateDownloadClientRequest,
  RemotePathMappingResponse,
  CreateRemotePathMappingRequest,
  UpdateRemotePathMappingRequest,
  NamingConfigResponse,
  MediaManagementConfigResponse,
  UpdateMediaManagementConfigRequest,
  IndexerResponse,
  CreateIndexerRequest,
  UpdateIndexerRequest,
  TestIndexerRequest,
  TestIndexerResponse,
  ProwlarrConfigResponse,
  ProwlarrImportRequest,
  ProwlarrImportResponse,
  EmailConfigResponse,
  UpdateEmailConfigRequest,
  MetadataConfigResponse,
  UpdateMetadataConfigRequest,
  IndexerConfigResponse,
  UpdateIndexerConfigRequest,
  HealthCheckResult,
  SystemStatus,
  LibraryItemResponse,
  ScanResult,
  BrowseResponse,
  ScanResponse,
  ManualSearchResponse,
  ManualImportItem,
  ManualImportResponse,
  PaginatedResponse,
  ReadarrRootFolder,
  ImportPreviewResponse,
  ImportProgressResponse,
  ImportHistoryItem,
} from "@/types/api";

// Setup
export const getSetupStatus = () =>
  apiFetch<SetupStatusResponse>("/setup/status");
export const setup = (req: SetupRequest) =>
  apiFetch<SetupResponse>("/setup", {
    method: "POST",
    body: JSON.stringify(req),
  });

// Auth
export const login = (req: LoginRequest) =>
  apiFetch<LoginResponse>("/auth/login", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const logout = () => apiFetch<void>("/auth/logout", { method: "POST" });
export const getMe = () => apiFetch<AuthMeResponse>("/auth/me");
export const updateProfile = (req: UpdateProfileRequest) =>
  apiFetch<void>("/auth/profile", { method: "PUT", body: JSON.stringify(req) });
export const regenerateApiKey = () =>
  apiFetch<ApiKeyResponse>("/auth/apikey", { method: "POST" });

// Users (admin)
export const listUsers = () => apiFetch<UserResponse[]>("/user");
export const createUser = (req: AdminCreateUserRequest) =>
  apiFetch<UserResponse>("/user", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const getUser = (id: number) => apiFetch<UserResponse>(`/user/${id}`);
export const updateUser = (id: number, req: AdminUpdateUserRequest) =>
  apiFetch<UserResponse>(`/user/${id}`, {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const deleteUser = (id: number) =>
  apiFetch<void>(`/user/${id}`, { method: "DELETE" });
export const regenerateUserApiKey = (id: number) =>
  apiFetch<ApiKeyResponse>(`/user/${id}/apikey`, { method: "POST" });

// Works
export const lookupWorks = (term: string, lang?: string) =>
  apiFetch<WorkSearchResult[]>(
    `/work/lookup?term=${encodeURIComponent(term)}${lang ? `&lang=${encodeURIComponent(lang)}` : ""}`,
  );
export const addWork = (req: AddWorkRequest) =>
  apiFetch<AddWorkResponse>("/work", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const listWorks = () =>
  apiFetch<PaginatedResponse<WorkDetailResponse>>("/work?page_size=500");
export const getWork = (id: number) =>
  apiFetch<WorkDetailResponse>(`/work/${id}`);
export const updateWork = (id: number, req: UpdateWorkRequest) =>
  apiFetch<WorkDetailResponse>(`/work/${id}`, {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const uploadWorkCover = (id: number, imageData: Blob) =>
  apiUpload<void>(`/work/${id}/cover`, imageData);
export const deleteWork = (id: number, deleteFiles: boolean) =>
  apiFetch<DeleteWorkResponse>(`/work/${id}?deleteFiles=${deleteFiles}`, {
    method: "DELETE",
  });
export const refreshWork = (id: number) =>
  apiFetch<RefreshWorkResponse>(`/work/${id}/refresh`, { method: "POST" });
export const refreshAllWorks = () =>
  apiFetch<void>("/work/refresh", { method: "POST" });

// Authors
export const lookupAuthors = (term: string) =>
  apiFetch<AuthorSearchResult[]>(
    `/author/lookup?term=${encodeURIComponent(term)}`,
  );
export const addAuthor = (req: AddAuthorRequest) =>
  apiFetch<AuthorResponse>("/author", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const listAuthors = () => apiFetch<AuthorResponse[]>("/author");
export const getAuthor = (id: number) =>
  apiFetch<AuthorDetailResponse>(`/author/${id}`);
export const updateAuthor = (id: number, req: UpdateAuthorRequest) =>
  apiFetch<AuthorResponse>(`/author/${id}`, {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const deleteAuthor = (id: number) =>
  apiFetch<void>(`/author/${id}`, { method: "DELETE" });
export const searchAuthors = () =>
  apiFetch<void>("/author/search", { method: "POST" });
export const getAuthorBibliography = (id: number) =>
  apiFetch<AuthorBibliography>(`/author/${id}/bibliography`);
export const refreshAuthorBibliography = (id: number) =>
  apiFetch<AuthorBibliography>(`/author/${id}/bibliography/refresh`, {
    method: "POST",
  });

// Notifications
export const listNotifications = (unreadOnly?: boolean) =>
  apiFetch<PaginatedResponse<NotificationResponse>>(
    `/notification?page_size=200${unreadOnly ? "&unreadOnly=true" : ""}`,
  );
export const markNotificationRead = (id: number) =>
  apiFetch<void>(`/notification/${id}`, { method: "PUT" });
export const dismissNotification = (id: number) =>
  apiFetch<void>(`/notification/${id}`, { method: "DELETE" });
export const dismissAllNotifications = () =>
  apiFetch<void>("/notification", { method: "DELETE" });

// Queue
export const getQueue = (page = 1) =>
  apiFetch<QueueListResponse>(`/queue?page=${page}`);
export const removeQueueItem = (id: number) =>
  apiFetch<void>(`/queue/${id}`, { method: "DELETE" });
export const retryImport = (grabId: number) =>
  apiFetch<void>(`/grab/${grabId}/retry`, { method: "POST" });

// Releases
// The backend returns 502 with a valid ReleaseSearchResponse body when all
// indexers fail. We parse the body as warnings in that case instead of throwing.
export const searchReleases = async (
  workId: number,
  opts?: { refresh?: boolean; cacheOnly?: boolean },
): Promise<ReleaseSearchResponse> => {
  const token = (await import("./client")).getToken();
  const headers: Record<string, string> = {};
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const params = new URLSearchParams({ workId: String(workId) });
  if (opts?.refresh) params.set("refresh", "true");
  if (opts?.cacheOnly) params.set("cacheOnly", "true");
  const res = await fetch(`/api/v1/release?${params}`, { headers });
  if (res.status === 502) {
    try {
      return (await res.json()) as ReleaseSearchResponse;
    } catch {
      return { results: [], warnings: [{ indexer: "unknown", error: "All indexers failed" }] };
    }
  }
  if (!res.ok) {
    const { ApiError } = await import("./client");
    if (res.status === 401) {
      // Dynamically import to avoid circular dep; clearAuth handles both
      // localStorage removal and Zustand state reset.
      const { useAuthStore } = await import("@/stores/auth");
      useAuthStore.getState().clearAuth();
      throw new ApiError({ status: 401, error: "unauthorized", message: "Session expired" });
    }
    throw new ApiError({ status: res.status, error: "error", message: "Search failed" });
  }
  return res.json() as Promise<ReleaseSearchResponse>;
};
export const grabRelease = (req: GrabRequest) =>
  apiFetch<void>("/release/grab", {
    method: "POST",
    body: JSON.stringify(req),
  });

// History
export const getHistory = (params?: {
  eventType?: EventType;
  workId?: number;
  startDate?: string;
  endDate?: string;
}) => {
  const searchParams = new URLSearchParams();
  if (params?.eventType) searchParams.set("eventType", params.eventType);
  if (params?.workId) searchParams.set("workId", String(params.workId));
  if (params?.startDate) searchParams.set("startDate", params.startDate);
  if (params?.endDate) searchParams.set("endDate", params.endDate);
  searchParams.set("page_size", "200");
  const qs = searchParams.toString();
  return apiFetch<PaginatedResponse<HistoryResponse>>(`/history?${qs}`);
};

// Root Folders
export const listRootFolders = () =>
  apiFetch<RootFolderResponse[]>("/rootfolder");
export const createRootFolder = (path: string, mediaType: MediaType) =>
  apiFetch<RootFolderResponse>("/rootfolder", {
    method: "POST",
    body: JSON.stringify({ path, mediaType }),
  });
export const deleteRootFolder = (id: number) =>
  apiFetch<void>(`/rootfolder/${id}`, { method: "DELETE" });

// Download Clients
export const listDownloadClients = () =>
  apiFetch<DownloadClientResponse[]>("/downloadclient");
export const createDownloadClient = (req: CreateDownloadClientRequest) =>
  apiFetch<DownloadClientResponse>("/downloadclient", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const getDownloadClient = (id: number) =>
  apiFetch<DownloadClientResponse>(`/downloadclient/${id}`);
export const updateDownloadClient = (
  id: number,
  req: UpdateDownloadClientRequest,
) =>
  apiFetch<DownloadClientResponse>(`/downloadclient/${id}`, {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const deleteDownloadClient = (id: number) =>
  apiFetch<void>(`/downloadclient/${id}`, { method: "DELETE" });
export const testDownloadClient = (req: CreateDownloadClientRequest) =>
  apiFetch<void>("/downloadclient/test", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const testSavedDownloadClient = (id: number) =>
  apiFetch<void>(`/downloadclient/${id}/test`, { method: "POST" });

// Remote Path Mappings
export const listRemotePathMappings = () =>
  apiFetch<RemotePathMappingResponse[]>("/remotepathmapping");
export const createRemotePathMapping = (req: CreateRemotePathMappingRequest) =>
  apiFetch<RemotePathMappingResponse>("/remotepathmapping", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const updateRemotePathMapping = (
  id: number,
  req: UpdateRemotePathMappingRequest,
) =>
  apiFetch<RemotePathMappingResponse>(`/remotepathmapping/${id}`, {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const deleteRemotePathMapping = (id: number) =>
  apiFetch<void>(`/remotepathmapping/${id}`, { method: "DELETE" });

// Config
export const getNamingConfig = () =>
  apiFetch<NamingConfigResponse>("/config/naming");
export const getMediaManagementConfig = () =>
  apiFetch<MediaManagementConfigResponse>("/config/mediamanagement");
export const updateMediaManagementConfig = (
  req: UpdateMediaManagementConfigRequest,
) =>
  apiFetch<MediaManagementConfigResponse>("/config/mediamanagement", {
    method: "PUT",
    body: JSON.stringify(req),
  });
// Indexers
export const listIndexers = () =>
  apiFetch<IndexerResponse[]>("/indexer");
export const getIndexer = (id: number) =>
  apiFetch<IndexerResponse>(`/indexer/${id}`);
export const createIndexer = (req: CreateIndexerRequest) =>
  apiFetch<IndexerResponse>("/indexer", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const updateIndexer = (id: number, req: UpdateIndexerRequest) =>
  apiFetch<IndexerResponse>(`/indexer/${id}`, {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const deleteIndexer = (id: number) =>
  apiFetch<void>(`/indexer/${id}`, { method: "DELETE" });
export const testIndexer = (req: TestIndexerRequest) =>
  apiFetch<TestIndexerResponse>("/indexer/test", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const testSavedIndexer = (id: number) =>
  apiFetch<TestIndexerResponse>(`/indexer/${id}/test`, { method: "POST" });
export const getProwlarrConfig = () =>
  apiFetch<ProwlarrConfigResponse>("/config/prowlarr");
export const importIndexersFromProwlarr = (req: ProwlarrImportRequest) =>
  apiFetch<ProwlarrImportResponse>("/indexer/import/prowlarr", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const importDownloadClientsFromProwlarr = (req: ProwlarrImportRequest) =>
  apiFetch<ProwlarrImportResponse>("/downloadclient/import/prowlarr", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const getEmailConfig = () =>
  apiFetch<EmailConfigResponse>("/config/email");
export const updateEmailConfig = (req: UpdateEmailConfigRequest) =>
  apiFetch<EmailConfigResponse>("/config/email", {
    method: "PUT",
    body: JSON.stringify(req),
  });
export const testEmailConfig = () =>
  apiFetch<{ success: boolean }>("/config/email/test", { method: "POST" });
export const sendFileEmail = (fileId: number) =>
  apiFetch<{ success: boolean }>(`/workfile/${fileId}/send-email`, {
    method: "POST",
  });
export const getMetadataConfig = () =>
  apiFetch<MetadataConfigResponse>("/config/metadata");
export const updateMetadataConfig = (req: UpdateMetadataConfigRequest) =>
  apiFetch<MetadataConfigResponse>("/config/metadata", {
    method: "PUT",
    body: JSON.stringify(req),
  });
// Indexer Config (RSS sync settings)
export const getIndexerConfig = () =>
  apiFetch<IndexerConfigResponse>("/config/indexer");
export const updateIndexerConfig = (req: UpdateIndexerConfigRequest) =>
  apiFetch<IndexerConfigResponse>("/config/indexer", {
    method: "PUT",
    body: JSON.stringify(req),
  });
// RSS Sync
export const triggerRssSync = () =>
  apiFetch<void>("/command/rss-sync", { method: "POST" });

// System
export const getHealth = () => apiFetch<HealthCheckResult[]>("/health");
export const getSystemStatus = () => apiFetch<SystemStatus>("/system/status");
export const getLogTail = (lines = 30) =>
  apiFetch<string[]>(`/system/logs/tail?lines=${lines}`);
export const setLogLevel = (level: string) =>
  apiFetch<{ level: string }>("/system/logs/level", {
    method: "PUT",
    body: JSON.stringify({ level }),
  });

// Library Files
export const listLibraryFiles = () =>
  apiFetch<PaginatedResponse<LibraryItemResponse>>("/workfile?page_size=500");
export const getLibraryFile = (id: number) =>
  apiFetch<LibraryItemResponse>(`/workfile/${id}`);
export const deleteLibraryFile = (id: number) =>
  apiFetch<void>(`/workfile/${id}`, { method: "DELETE" });

// Unmapped Files
export const scanRootFolder = (id: number) =>
  apiFetch<ScanResult>(`/rootfolder/${id}/scan`, { method: "POST" });

// Unmapped scan (arbitrary path)
export const scanUnmappedPath = (path: string) =>
  apiFetch<ScanResult>("/unmapped/scan", {
    method: "POST",
    body: JSON.stringify({ path }),
  });

// Manual Import
export const browseFilesystem = (path?: string) =>
  apiFetch<BrowseResponse>(
    `/filesystem${path ? `?path=${encodeURIComponent(path)}` : ""}`,
  );
export const scanManualImport = (path: string) =>
  apiFetch<ScanResponse>("/manualimport/scan", {
    method: "POST",
    body: JSON.stringify({ path }),
  });
export const searchManualImport = (query: string, author?: string) =>
  apiFetch<ManualSearchResponse>("/manualimport/search", {
    method: "POST",
    body: JSON.stringify({ query, author }),
  });
export const executeManualImport = (items: ManualImportItem[]) =>
  apiFetch<ManualImportResponse>("/manualimport/import", {
    method: "POST",
    body: JSON.stringify({ items }),
  });

// Readarr Import
export const readarrConnect = (url: string, apiKey: string) =>
  apiFetch<ReadarrRootFolder[]>("/import/readarr/connect", {
    method: "POST",
    body: JSON.stringify({ url, apiKey }),
  });
export const readarrPreview = (req: {
  url: string;
  apiKey: string;
  readarrRootFolderId: number;
  livrarrRootFolderId: number;
}) =>
  apiFetch<ImportPreviewResponse>("/import/readarr/preview", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const readarrStartImport = (req: {
  url: string;
  apiKey: string;
  readarrRootFolderId: number;
  livrarrRootFolderId: number;
}) =>
  apiFetch<{ importId: string }>("/import/readarr/start", {
    method: "POST",
    body: JSON.stringify(req),
  });
export const readarrProgress = () =>
  apiFetch<ImportProgressResponse>("/import/readarr/progress");
export const readarrHistory = () =>
  apiFetch<ImportHistoryItem[]>("/import/readarr/history");
export const readarrUndo = (importId: string) =>
  apiFetch<void>(`/import/readarr/${importId}`, { method: "DELETE" });
