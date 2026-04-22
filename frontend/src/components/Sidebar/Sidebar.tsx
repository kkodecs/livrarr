import { NavLink, useLocation } from "react-router";
import {
  BookOpen,
  Library,
  Users,
  PlusCircle,
  BookMarked,
  ListOrdered,
  History,
  HardDrive,
  Search,
  Download,
  Database,
  Shield,
  Palette,
  UserCog,
  Activity,
  Calendar,
  AlertCircle,
  LayoutList,
  Tag,
  Bell,
  Import,
  Code,
  ScrollText,
  ChevronDown,
  ChevronRight,
  Bookmark,
  ArrowUpCircle,
  Info,
  X,
} from "lucide-react";
import { cn } from "@/utils/cn";
import { useAuthStore } from "@/stores/auth";
import { useUIStore } from "@/stores/ui";
import { useState, useEffect, type ReactNode } from "react";
import { getSystemStatus } from "@/api";

interface NavItem {
  label: string;
  path: string;
  icon: ReactNode;
  adminOnly?: boolean;
  greyed?: boolean;
}

interface NavGroup {
  label: string;
  children: NavItem[];
  /** When true, group is always expanded with no toggle */
  pinned?: boolean;
}

const navGroups: NavGroup[] = [
  {
    label: "Library",
    pinned: true,
    children: [
      { label: "Works", path: "/", icon: <BookOpen size={18} /> },
      { label: "Series", path: "/series", icon: <Library size={18} /> },
      { label: "Authors", path: "/author", icon: <Users size={18} /> },
      { label: "Lists", path: "/lists", icon: <Import size={18} /> },
      { label: "Add New", path: "/search", icon: <PlusCircle size={18} /> },
      {
        label: "Missing",
        path: "/wanted/missing",
        icon: <AlertCircle size={18} />,
      },
      {
        label: "Bookshelf",
        path: "/shelf",
        icon: <Bookmark size={18} />,
        greyed: true,
      },
    ],
  },
  {
    label: "Activity",
    pinned: true,
    children: [
      {
        label: "Queue",
        path: "/activity/queue",
        icon: <ListOrdered size={18} />,
      },
      {
        label: "History",
        path: "/activity/history",
        icon: <History size={18} />,
      },
      {
        label: "Manual Import",
        path: "/import",
        icon: <Import size={18} />,
      },
      {
        label: "Readarr Import",
        path: "/import/readarr",
        icon: <Download size={18} />,
      },
    ],
  },
  {
    label: "More",
    children: [
      {
        label: "Calendar",
        path: "/calendar",
        icon: <Calendar size={18} />,
        greyed: true,
      },
      {
        label: "Cutoff Unmet",
        path: "/wanted/cutoff",
        icon: <LayoutList size={18} />,
        greyed: true,
      },
    ],
  },
  {
    label: "Settings",
    children: [
      {
        label: "Media Management",
        path: "/settings/mediamanagement",
        icon: <HardDrive size={18} />,
      },
      {
        label: "Indexers",
        path: "/settings/indexers",
        icon: <Search size={18} />,
        adminOnly: true,
      },
      {
        label: "Download Clients",
        path: "/settings/downloadclients",
        icon: <Download size={18} />,
      },
      {
        label: "Metadata",
        path: "/settings/metadata",
        icon: <Database size={18} />,
        adminOnly: true,
      },
      {
        label: "General",
        path: "/settings/general",
        icon: <Shield size={18} />,
        adminOnly: true,
        greyed: true,
      },
      { label: "UI", path: "/settings/ui", icon: <Palette size={18} /> },
      {
        label: "User Management",
        path: "/settings/users",
        icon: <UserCog size={18} />,
        adminOnly: true,
      },
      {
        label: "Profiles",
        path: "/settings/profiles",
        icon: <BookMarked size={18} />,
        adminOnly: true,
        greyed: true,
      },
      {
        label: "Custom Formats",
        path: "/settings/customformats",
        icon: <Tag size={18} />,
        adminOnly: true,
        greyed: true,
      },
      {
        label: "Notifications",
        path: "/settings/notifications",
        icon: <Bell size={18} />,
        adminOnly: true,
        greyed: true,
      },
      {
        label: "Tags",
        path: "/settings/tags",
        icon: <Tag size={18} />,
        greyed: true,
      },
      {
        label: "Development",
        path: "/settings/development",
        icon: <Code size={18} />,
        adminOnly: true,
        greyed: true,
      },
    ],
  },
  {
    label: "System",
    children: [
      { label: "Status", path: "/system/status", icon: <Activity size={18} /> },
      {
        label: "Logs",
        path: "/system/logs",
        icon: <ScrollText size={18} />,
      },
      {
        label: "About Livrarr",
        path: "/system/about",
        icon: <Info size={18} />,
      },
    ],
  },
];

function SidebarGroup({
  group,
  onNavigate,
}: {
  group: NavGroup;
  onNavigate?: () => void;
}) {
  const location = useLocation();
  const isAdmin = useAuthStore((s) => s.isAdmin);
  const collapsed = useUIStore((s) => s.sidebarCollapsed);

  const isActive = group.children.some(
    (item) => location.pathname === item.path,
  );
  const [open, setOpen] = useState(isActive);

  const visibleChildren = group.children.filter(
    (item) => !item.adminOnly || isAdmin,
  );

  if (visibleChildren.length === 0) return null;

  if (collapsed) {
    return (
      <div className="space-y-0.5">
        {visibleChildren.map((item) => (
          <SidebarItem
            key={item.path}
            item={item}
            collapsed
            onNavigate={onNavigate}
          />
        ))}
      </div>
    );
  }

  if (group.pinned) {
    return (
      <div className="space-y-0.5">
        <div className="px-3 py-1.5 text-xs font-semibold uppercase tracking-wider text-muted">
          {group.label}
        </div>
        {visibleChildren.map((item) => (
          <SidebarItem
            key={item.path}
            item={item}
            onNavigate={onNavigate}
          />
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-0.5">
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-xs font-semibold uppercase tracking-wider text-muted hover:text-zinc-300"
      >
        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        {group.label}
      </button>
      {open &&
        visibleChildren.map((item) => (
          <SidebarItem
            key={item.path}
            item={item}
            onNavigate={onNavigate}
          />
        ))}
    </div>
  );
}

function SidebarItem({
  item,
  collapsed,
  onNavigate,
}: {
  item: NavItem;
  collapsed?: boolean;
  onNavigate?: () => void;
}) {
  if (item.greyed) {
    return (
      <span
        className={cn(
          "flex items-center gap-3 rounded px-3 py-2 text-sm text-zinc-600 cursor-default",
          collapsed && "justify-center px-0",
        )}
        title="Coming Soon"
      >
        {item.icon}
        {!collapsed && <span>{item.label}</span>}
      </span>
    );
  }

  return (
    <NavLink
      to={item.path}
      end
      onClick={onNavigate}
      className={({ isActive }) =>
        cn(
          "flex items-center gap-3 rounded px-3 py-2 text-sm transition-colors",
          isActive
            ? "bg-brand/10 text-brand font-medium"
            : "text-zinc-400 hover:bg-surface-hover hover:text-zinc-100",
          collapsed && "justify-center px-0",
        )
      }
    >
      {item.icon}
      {!collapsed && <span>{item.label}</span>}
    </NavLink>
  );
}

const REPO_URL = "https://github.com/kkodecs/livrarr";
const RELEASES_API = "https://api.github.com/repos/kkodecs/livrarr/releases/latest";

function useVersionCheck() {
  const checkForUpdates = useUIStore((s) => s.checkForUpdates);
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [latestVersion, setLatestVersion] = useState<string | null>(null);
  const [latestUrl, setLatestUrl] = useState<string | null>(null);

  useEffect(() => {
    getSystemStatus().then((s) => setCurrentVersion(s.version)).catch(() => {});
  }, []);

  useEffect(() => {
    if (!checkForUpdates) return;
    fetch(RELEASES_API)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (data?.tag_name) {
          // Strip leading "v" if present (e.g. "v0.1.0-alpha3" → "0.1.0-alpha3")
          setLatestVersion(data.tag_name.replace(/^v/, ""));
          setLatestUrl(data.html_url);
        }
      })
      .catch(() => {});
  }, [checkForUpdates]);

  const hasUpdate =
    checkForUpdates && currentVersion && latestVersion && latestVersion !== currentVersion;

  return { currentVersion, latestVersion, latestUrl, hasUpdate };
}

function VersionFooter({ collapsed }: { collapsed: boolean }) {
  const { currentVersion, latestVersion, latestUrl, hasUpdate } =
    useVersionCheck();

  return (
    <div
      className={cn(
        "border-t border-border p-2",
        collapsed && "flex flex-col items-center",
      )}
    >
      {hasUpdate && !collapsed && (
        <a
          href={latestUrl ?? `${REPO_URL}/releases`}
          target="_blank"
          rel="noopener noreferrer"
          className="flex items-center gap-2 rounded px-2 py-1.5 mb-1 text-xs font-medium bg-brand/15 text-brand hover:bg-brand/25 transition-colors"
        >
          <ArrowUpCircle size={14} className="shrink-0" />
          <span>Update: livrarr:{latestVersion}</span>
        </a>
      )}
      {hasUpdate && collapsed && (
        <a
          href={latestUrl ?? `${REPO_URL}/releases`}
          target="_blank"
          rel="noopener noreferrer"
          title={`Update available: livrarr:${latestVersion}`}
          className="flex items-center justify-center rounded p-1.5 mb-1 text-brand hover:bg-brand/25 transition-colors"
        >
          <ArrowUpCircle size={16} />
        </a>
      )}
      <a
        href={REPO_URL}
        target="_blank"
        rel="noopener noreferrer"
        className={cn(
          "text-brand hover:text-brand/80 transition-colors text-center",
          collapsed ? "text-[9px]" : "block py-1 text-[11px]",
        )}
        title={`livrarr:${currentVersion ?? "..."}`}
      >
        {collapsed
          ? "v" + (currentVersion ?? "…")
          : `livrarr:${currentVersion ?? "..."}`}
      </a>
    </div>
  );
}

export function Sidebar() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const mobileSidebarOpen = useUIStore((s) => s.mobileSidebarOpen);
  const setMobileSidebarOpen = useUIStore((s) => s.setMobileSidebarOpen);

  // Close mobile sidebar on navigation (handled via onNavigate callback)
  const closeMobile = () => setMobileSidebarOpen(false);

  // Close mobile sidebar on escape key
  useEffect(() => {
    if (!mobileSidebarOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeMobile();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [mobileSidebarOpen]); // eslint-disable-line react-hooks/exhaustive-deps

  // Prevent body scroll when mobile sidebar is open
  useEffect(() => {
    if (mobileSidebarOpen) {
      document.body.style.overflow = "hidden";
    } else {
      document.body.style.overflow = "";
    }
    return () => {
      document.body.style.overflow = "";
    };
  }, [mobileSidebarOpen]);

  return (
    <>
      {/* Mobile backdrop */}
      {mobileSidebarOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/60 md:hidden"
          onClick={closeMobile}
        />
      )}

      {/* Mobile sidebar drawer */}
      <aside
        className={cn(
          "fixed top-0 left-0 bottom-0 z-50 flex w-64 flex-col overflow-y-auto bg-sidebar border-r border-border transition-transform duration-200 md:hidden",
          mobileSidebarOpen ? "translate-x-0" : "-translate-x-full",
        )}
      >
        {/* Mobile sidebar header */}
        <div className="flex h-12 items-center justify-between border-b border-border px-4">
          <span className="text-lg font-bold text-zinc-100">Livrarr</span>
          <button
            onClick={closeMobile}
            className="rounded p-1.5 text-zinc-400 hover:bg-surface-hover hover:text-zinc-100"
          >
            <X size={20} />
          </button>
        </div>
        <nav className="flex-1 space-y-2 p-2">
          {navGroups.map((group) => (
            <SidebarGroup
              key={group.label}
              group={group}
              onNavigate={closeMobile}
            />
          ))}
        </nav>
        <VersionFooter collapsed={false} />
      </aside>

      {/* Desktop sidebar (hidden on mobile) */}
      <aside
        className={cn(
          "fixed left-0 top-12 bottom-0 z-30 hidden md:flex flex-col overflow-y-auto bg-sidebar border-r border-border transition-all",
          collapsed ? "w-14" : "w-56",
        )}
      >
        <nav className="flex-1 space-y-2 p-2">
          {navGroups.map((group) => (
            <SidebarGroup key={group.label} group={group} />
          ))}
        </nav>
        <VersionFooter collapsed={collapsed} />
      </aside>
    </>
  );
}
