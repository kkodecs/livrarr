import { lazy, Suspense, useEffect } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Toaster } from "sonner";
import { useAuthStore } from "@/stores/auth";
import { AppLayout } from "@/components/Page/AppLayout";
import { AuthGuard, AdminGuard, GuestGuard } from "@/components/Page/AuthGuard";
import { FullPageLoading, PageLoading } from "@/components/Page/LoadingSpinner";
import { ComingSoonPage } from "@/components/Page/ComingSoonPage";
import { NotFoundPage } from "@/components/Page/NotFoundPage";

// Eagerly loaded (initial bundle)
import { LoginPage } from "@/pages/login/LoginPage";
import { SetupPage } from "@/pages/setup/SetupPage";
import { WorksPage } from "@/pages/works/WorksPage";

// Lazy loaded
const WorkDetailPage = lazy(() => import("@/pages/work-detail/WorkDetailPage"));
const AuthorsPage = lazy(() => import("@/pages/authors/AuthorsPage"));
const SeriesPage = lazy(() => import("@/pages/series/SeriesPage"));
const SeriesDetailPage = lazy(
  () => import("@/pages/series/SeriesDetailPage"),
);
const AuthorDetailPage = lazy(
  () => import("@/pages/author-detail/AuthorDetailPage"),
);
const SearchPage = lazy(() => import("@/pages/search/SearchPage"));
const AuthorSearchPage = lazy(() => import("@/pages/search/AuthorSearchPage"));
const QueuePage = lazy(() => import("@/pages/activity/queue/QueuePage"));
const HistoryPage = lazy(() => import("@/pages/activity/history/HistoryPage"));
const ProfilePage = lazy(() => import("@/pages/profile/ProfilePage"));
const UnmappedPage = lazy(() => import("@/pages/unmapped/UnmappedPage"));
const ReviewPage = lazy(() => import("@/pages/review/ReviewPage"));
const ManualImportPage = lazy(
  () => import("@/pages/manual-import/ManualImportPage"),
);
const MissingPage = lazy(() => import("@/pages/wanted/MissingPage"));
const ReadarrImportPage = lazy(
  () => import("@/pages/import/ReadarrImportPage"),
);
const ListImportPage = lazy(
  () => import("@/pages/lists/ListImportPage"),
);

// Readers (lazy, full-page — no AppLayout)
const ReaderPage = lazy(() => import("@/pages/reader/ReaderPage"));
const ListenPage = lazy(() => import("@/pages/reader/ListenPage"));

// Settings (lazy)
const MediaManagementPage = lazy(
  () => import("@/pages/settings/media-management/MediaManagementPage"),
);
const IndexersPage = lazy(
  () => import("@/pages/settings/indexers/IndexersPage"),
);
const DownloadClientsPage = lazy(
  () => import("@/pages/settings/download-clients/DownloadClientsPage"),
);
const MetadataPage = lazy(
  () => import("@/pages/settings/metadata/MetadataPage"),
);
const UISettingsPage = lazy(() => import("@/pages/settings/ui/UISettingsPage"));
const UsersPage = lazy(() => import("@/pages/settings/users/UsersPage"));

// System (lazy)
const StatusPage = lazy(() => import("@/pages/system/status/StatusPage"));
const AboutPage = lazy(() => import("@/pages/system/about/AboutPage"));
const LogsPage = lazy(() => import("@/pages/system/logs/LogsPage"));
const HelpPage = lazy(() => import("@/pages/help/HelpPage"));

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      gcTime: 5 * 60_000,
      retry: 1,
      refetchOnWindowFocus: true,
    },
  },
});

function AuthInitializer({ children }: { children: React.ReactNode }) {
  const status = useAuthStore((s) => s.status);
  const initialize = useAuthStore((s) => s.initialize);

  useEffect(() => {
    initialize();
  }, [initialize]);

  if (status === "loading") {
    return <FullPageLoading />;
  }

  return <>{children}</>;
}

function LazyPage({ children }: { children: React.ReactNode }) {
  return <Suspense fallback={<PageLoading />}>{children}</Suspense>;
}

/** Settings with no sub-page: Media Management for an admin, UI for everyone else. */
function SettingsIndex() {
  const isAdmin = useAuthStore((s) => s.isAdmin);
  if (!isAdmin) {
    return <Navigate to="/settings/ui" replace />;
  }
  return (
    <LazyPage>
      <MediaManagementPage />
    </LazyPage>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <AuthInitializer>
          <Routes>
            {/* Public routes */}
            <Route
              path="/setup"
              element={
                <GuestGuard>
                  <SetupPage />
                </GuestGuard>
              }
            />
            <Route
              path="/login"
              element={
                <GuestGuard>
                  <LoginPage />
                </GuestGuard>
              }
            />

            {/* Full-page readers — authenticated but no AppLayout */}
            <Route
              path="/read/:id"
              element={
                <AuthGuard>
                  <Suspense fallback={<FullPageLoading />}>
                    <ReaderPage />
                  </Suspense>
                </AuthGuard>
              }
            />
            <Route
              path="/listen/:id"
              element={
                <AuthGuard>
                  <Suspense fallback={<FullPageLoading />}>
                    <ListenPage />
                  </Suspense>
                </AuthGuard>
              }
            />

            {/* Authenticated routes */}
            <Route
              element={
                <AuthGuard>
                  <AppLayout />
                </AuthGuard>
              }
            >
              <Route index element={<WorksPage />} />
              <Route
                path="search"
                element={
                  <LazyPage>
                    <SearchPage />
                  </LazyPage>
                }
              />
              <Route
                path="work/add"
                element={
                  <LazyPage>
                    <SearchPage />
                  </LazyPage>
                }
              />
              <Route
                path="work/:id"
                element={
                  <LazyPage>
                    <WorkDetailPage />
                  </LazyPage>
                }
              />
              <Route
                path="series"
                element={
                  <LazyPage>
                    <SeriesPage />
                  </LazyPage>
                }
              />
              <Route
                path="series/:id"
                element={
                  <LazyPage>
                    <SeriesDetailPage />
                  </LazyPage>
                }
              />
              <Route
                path="author"
                element={
                  <LazyPage>
                    <AuthorsPage />
                  </LazyPage>
                }
              />
              <Route
                path="author/add"
                element={
                  <LazyPage>
                    <AuthorSearchPage />
                  </LazyPage>
                }
              />
              <Route
                path="author/:id"
                element={
                  <LazyPage>
                    <AuthorDetailPage />
                  </LazyPage>
                }
              />
              <Route
                path="import"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <ManualImportPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="import/readarr"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <ReadarrImportPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="lists"
                element={
                  <LazyPage>
                    <ListImportPage />
                  </LazyPage>
                }
              />
              <Route
                path="unmapped"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <UnmappedPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="review"
                element={
                  <LazyPage>
                    <ReviewPage />
                  </LazyPage>
                }
              />
              <Route
                path="activity/queue"
                element={
                  <LazyPage>
                    <QueuePage />
                  </LazyPage>
                }
              />
              <Route
                path="activity/history"
                element={
                  <LazyPage>
                    <HistoryPage />
                  </LazyPage>
                }
              />
              <Route
                path="profile"
                element={
                  <LazyPage>
                    <ProfilePage />
                  </LazyPage>
                }
              />

              {/* Settings */}
              <Route path="settings" element={<SettingsIndex />} />
              <Route
                path="settings/mediamanagement"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <MediaManagementPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="settings/indexers"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <IndexersPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="settings/downloadclients"
                element={
                  <LazyPage>
                    <DownloadClientsPage />
                  </LazyPage>
                }
              />
              <Route
                path="settings/metadata"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <MetadataPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="settings/general"
                element={
                  <AdminGuard>
                    <ComingSoonPage title="General Settings" />
                  </AdminGuard>
                }
              />
              <Route
                path="settings/ui"
                element={
                  <LazyPage>
                    <UISettingsPage />
                  </LazyPage>
                }
              />
              <Route
                path="settings/users"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <UsersPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              {/* Import Lists moved to /lists (main nav) */}
              <Route
                path="settings/notifications"
                element={<ComingSoonPage title="Notifications" />}
              />
              <Route
                path="settings/tags"
                element={<ComingSoonPage title="Tags" />}
              />

              {/* System */}
              <Route
                path="system/status"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <StatusPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="system/logs"
                element={
                  <AdminGuard>
                    <LazyPage>
                      <LogsPage />
                    </LazyPage>
                  </AdminGuard>
                }
              />
              <Route
                path="system/about"
                element={
                  <LazyPage>
                    <AboutPage />
                  </LazyPage>
                }
              />

              {/* Help */}
              <Route
                path="help"
                element={
                  <LazyPage>
                    <HelpPage />
                  </LazyPage>
                }
              />

              <Route
                path="wanted/missing"
                element={
                  <LazyPage>
                    <MissingPage />
                  </LazyPage>
                }
              />

              {/* Fallback */}
              <Route path="*" element={<NotFoundPage />} />
            </Route>
          </Routes>
        </AuthInitializer>
      </BrowserRouter>
      <Toaster
        theme="dark"
        position="bottom-right"
        visibleToasts={5}
        gap={8}
        expand
        closeButton
        toastOptions={{
          className: "bg-zinc-800 border-border text-zinc-100",
        }}
      />
    </QueryClientProvider>
  );
}
