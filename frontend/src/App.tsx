import { lazy, Suspense, useEffect } from "react";
import { BrowserRouter, Routes, Route } from "react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth";
import { AppLayout } from "@/components/Page/AppLayout";
import { AuthGuard, AdminGuard, GuestGuard } from "@/components/Page/AuthGuard";
import { FullPageLoading, PageLoading } from "@/components/Page/LoadingSpinner";
import { ComingSoonPage } from "@/components/Page/ComingSoonPage";

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
                  <LazyPage>
                    <ManualImportPage />
                  </LazyPage>
                }
              />
              <Route
                path="import/readarr"
                element={
                  <LazyPage>
                    <ReadarrImportPage />
                  </LazyPage>
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
                  <LazyPage>
                    <UnmappedPage />
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
              <Route
                path="settings"
                element={
                  <LazyPage>
                    <MediaManagementPage />
                  </LazyPage>
                }
              />
              <Route
                path="settings/mediamanagement"
                element={
                  <LazyPage>
                    <MediaManagementPage />
                  </LazyPage>
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
              <Route
                path="settings/profiles"
                element={<ComingSoonPage title="Profiles" />}
              />
              <Route
                path="settings/customformats"
                element={<ComingSoonPage title="Custom Formats" />}
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
              <Route
                path="settings/development"
                element={<ComingSoonPage title="Development" />}
              />

              {/* System */}
              <Route
                path="system/status"
                element={
                  <LazyPage>
                    <StatusPage />
                  </LazyPage>
                }
              />
              <Route
                path="system/logs"
                element={
                  <LazyPage>
                    <LogsPage />
                  </LazyPage>
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

              {/* Greyed-out placeholders */}
              <Route
                path="calendar"
                element={<ComingSoonPage title="Calendar" />}
              />
              <Route
                path="wanted/missing"
                element={
                  <LazyPage>
                    <MissingPage />
                  </LazyPage>
                }
              />
              <Route
                path="wanted/cutoff"
                element={<ComingSoonPage title="Cutoff Unmet" />}
              />
              <Route
                path="shelf"
                element={<ComingSoonPage title="Bookshelf" />}
              />

              {/* Fallback */}
              <Route
                path="*"
                element={<ComingSoonPage title="Page Not Found" />}
              />
            </Route>
          </Routes>
        </AuthInitializer>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
