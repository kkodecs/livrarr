import { Link, useNavigate } from "react-router";
import { Menu, Search, User, LogOut, HelpCircle, ChevronDown } from "lucide-react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { useUIStore } from "@/stores/ui";
import { useAuthStore } from "@/stores/auth";
import { NotificationBell } from "@/components/Header/NotificationBell";
import { useState, useEffect, useRef, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { getMetadataConfig } from "@/api";
import { SUPPORTED_LANGUAGES } from "@/types/api";

export function Header({ onStartTour }: { onStartTour?: () => void }) {
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const user = useAuthStore((s) => s.user);
  const logoutAction = useAuthStore((s) => s.logoutAction);
  const navigate = useNavigate();
  const [searchTerm, setSearchTerm] = useState("");
  const [selectedLang, setSelectedLang] = useState("en");
  const [langOpen, setLangOpen] = useState(false);
  const langRef = useRef<HTMLDivElement>(null);

  const { data: metaConfig } = useQuery({
    queryKey: ["metadata-config"],
    queryFn: getMetadataConfig,
  });

  const enabledLanguages = useMemo(() => {
    const codes = metaConfig?.languages ?? ["en"];
    return SUPPORTED_LANGUAGES.filter((l) => codes.includes(l.code));
  }, [metaConfig]);

  // Sync to primary language from config
  useEffect(() => {
    if (metaConfig) {
      setSelectedLang(metaConfig.languages[0] ?? "en");
    }
  }, [metaConfig]);

  // Click-outside
  useEffect(() => {
    if (!langOpen) return;
    const handler = (e: MouseEvent) => {
      if (langRef.current && !langRef.current.contains(e.target as Node)) {
        setLangOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [langOpen]);

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    if (searchTerm.trim()) {
      const params = new URLSearchParams({ q: searchTerm.trim() });
      if (selectedLang !== "en") params.set("lang", selectedLang);
      navigate(`/search?${params.toString()}`);
      setSearchTerm("");
    }
  };

  const currentLang = SUPPORTED_LANGUAGES.find((l) => l.code === selectedLang);
  const showLangSelector = enabledLanguages.length > 1;

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

      <form onSubmit={handleSearch} className="hidden md:flex items-center gap-1.5">
        {showLangSelector && (
          <div className="relative" ref={langRef}>
            <button
              type="button"
              onClick={() => setLangOpen(!langOpen)}
              className="flex items-center gap-1 rounded border border-border bg-zinc-900 px-2 py-1.5 text-xs text-zinc-400 hover:border-zinc-500"
            >
              <span>{currentLang?.flag}</span>
              <ChevronDown size={10} className="text-zinc-500" />
            </button>
            {langOpen && (
              <div className="absolute top-full left-0 mt-1 z-50 min-w-[180px] rounded-lg border border-border bg-zinc-800 py-1 shadow-xl">
                {enabledLanguages.map((lang) => (
                  <button
                    key={lang.code}
                    type="button"
                    onClick={() => {
                      setSelectedLang(lang.code);
                      setLangOpen(false);
                    }}
                    className={`flex items-center gap-2 w-full px-3 py-1.5 text-xs text-left hover:bg-blue-500/10 ${
                      selectedLang === lang.code ? "bg-blue-500/10" : ""
                    }`}
                  >
                    <span>{lang.flag}</span>
                    <span className="text-zinc-100">{lang.englishName}</span>
                    {selectedLang === lang.code && (
                      <span className="ml-auto text-brand">&#10003;</span>
                    )}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
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
