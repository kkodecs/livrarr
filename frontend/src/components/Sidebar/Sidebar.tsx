import { NavLink, useLocation } from "react-router";
import {
  BookOpen,
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
  Heart,
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
} from "lucide-react";
import { cn } from "@/utils/cn";
import { useAuthStore } from "@/stores/auth";
import { useUIStore } from "@/stores/ui";
import { useState, type ReactNode } from "react";

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
      { label: "Authors", path: "/author", icon: <Users size={18} /> },
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
        label: "Import Lists",
        path: "/settings/importlists",
        icon: <Import size={18} />,
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
      { label: "Health", path: "/system/health", icon: <Heart size={18} /> },
      {
        label: "Logs",
        path: "/system/logs",
        icon: <ScrollText size={18} />,
        greyed: true,
      },
    ],
  },
];

function SidebarGroup({ group }: { group: NavGroup }) {
  const location = useLocation();
  const isAdmin = useAuthStore((s) => s.isAdmin);
  const collapsed = useUIStore((s) => s.sidebarCollapsed);

  const isActive = group.children.some(
    (item) =>
      location.pathname === item.path ||
      (item.path !== "/" && location.pathname.startsWith(item.path)),
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
          <SidebarItem key={item.path} item={item} collapsed />
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
          <SidebarItem key={item.path} item={item} />
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
          <SidebarItem key={item.path} item={item} />
        ))}
    </div>
  );
}

function SidebarItem({
  item,
  collapsed,
}: {
  item: NavItem;
  collapsed?: boolean;
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
      end={item.path === "/"}
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

export function Sidebar() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);

  return (
    <aside
      className={cn(
        "fixed left-0 top-12 bottom-0 z-30 flex flex-col overflow-y-auto bg-sidebar border-r border-border transition-all",
        collapsed ? "w-14" : "w-56",
      )}
    >
      <nav className="flex-1 space-y-2 p-2">
        {navGroups.map((group) => (
          <SidebarGroup key={group.label} group={group} />
        ))}
      </nav>
    </aside>
  );
}
