import { Link, useNavigate } from "react-router";
import { Menu, Search, User, LogOut, HelpCircle } from "lucide-react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { useUIStore } from "@/stores/ui";
import { useAuthStore } from "@/stores/auth";
import { NotificationBell } from "@/components/Header/NotificationBell";
import { useState } from "react";

export function Header({ onStartTour }: { onStartTour?: () => void }) {
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const user = useAuthStore((s) => s.user);
  const logoutAction = useAuthStore((s) => s.logoutAction);
  const navigate = useNavigate();
  const [searchTerm, setSearchTerm] = useState("");

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    if (searchTerm.trim()) {
      navigate(`/search?q=${encodeURIComponent(searchTerm.trim())}`);
      setSearchTerm("");
    }
  };

  return (
    <header className="fixed top-0 left-0 right-0 z-40 flex h-12 items-center justify-between border-b border-border bg-header px-4">
      <div className="flex items-center gap-3">
        <button
          onClick={toggleSidebar}
          className="rounded p-1.5 text-zinc-400 hover:bg-surface-hover hover:text-zinc-100"
          aria-label="Toggle sidebar"
        >
          <Menu size={20} />
        </button>
        <Link to="/" className="text-lg font-bold text-zinc-100">
          Livrarr
        </Link>
      </div>

      <form onSubmit={handleSearch} className="hidden md:flex items-center">
        <div className="relative">
          <Search
            size={16}
            className="absolute left-3 top-1/2 -translate-y-1/2 text-muted"
          />
          <input
            type="text"
            placeholder="Search..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-64 rounded border border-border bg-zinc-900 py-1.5 pl-9 pr-3 text-sm text-zinc-100 placeholder-muted focus:border-brand focus:outline-none"
          />
        </div>
      </form>

      <div className="flex items-center gap-2">
        {onStartTour && (
          <button
            onClick={() => {
              navigate("/");
              setTimeout(onStartTour, 100);
            }}
            className="rounded p-1.5 text-zinc-400 hover:bg-surface-hover hover:text-zinc-100"
            title="Setup Guide"
          >
            <HelpCircle size={18} />
          </button>
        )}
        <NotificationBell />
        <DropdownMenu.Root>
          <DropdownMenu.Trigger className="flex items-center gap-2 rounded px-2 py-1.5 text-sm text-zinc-400 hover:bg-surface-hover hover:text-zinc-100">
            <User size={18} />
            <span className="hidden sm:inline">{user?.username}</span>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              align="end"
              className="z-50 min-w-[160px] rounded border border-border bg-zinc-800 p-1 shadow-xl"
            >
              <DropdownMenu.Item
                className="flex cursor-pointer items-center gap-2 rounded px-3 py-2 text-sm text-zinc-300 outline-none hover:bg-surface-hover"
                onSelect={() => navigate("/profile")}
              >
                <User size={14} />
                Profile
              </DropdownMenu.Item>
              <DropdownMenu.Separator className="my-1 h-px bg-border" />
              <DropdownMenu.Item
                className="flex cursor-pointer items-center gap-2 rounded px-3 py-2 text-sm text-zinc-300 outline-none hover:bg-surface-hover"
                onSelect={() => logoutAction()}
              >
                <LogOut size={14} />
                Logout
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>
    </header>
  );
}
