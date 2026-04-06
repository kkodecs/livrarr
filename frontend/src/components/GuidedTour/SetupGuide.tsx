import { useCallback, useMemo } from "react";
import { useNavigate, useLocation } from "react-router";
import { ChevronRight, ChevronLeft, X, Sparkles } from "lucide-react";
import { useTourState } from "./useTourState";

interface GuideStep {
  route: string;
  title: string;
  description: string;
}

const STEPS: GuideStep[] = [
  {
    route: "/settings/metadata",
    title: "Step 1: Metadata Providers",
    description:
      "Configure where Livrarr gets book metadata. Hardcover is free and recommended — grab an API token at hardcover.app → Settings → API. Optionally, connect an LLM (Groq and Gemini both have free tiers) to help disambiguate search results. Only publicly available information (titles, author names) is ever sent.",
  },
  {
    route: "/settings/indexers",
    title: "Step 2: Indexers",
    description:
      "Add the search engines Livrarr uses to find releases. You'll need a Torznab indexer for torrents (e.g. MyAnonamouse) or a Newznab indexer for Usenet (e.g. NZBGeek). Each requires a URL and API key from your indexer account.",
  },
  {
    route: "/settings/downloadclients",
    title: "Step 3: Download Clients",
    description:
      "Connect a download client so Livrarr can grab releases. Add qBittorrent for torrents or SABnzbd for Usenet. You'll need the host, port, and credentials.",
  },
  {
    route: "/settings/mediamanagement",
    title: "Step 4: Media Management",
    description:
      "Set up where your books live. Add at least one root folder (e.g. /books). If your download client runs on a different machine, configure remote path mappings. If you use Calibre-Web Automated, point Livrarr at the CWA ingest folder to auto-import.",
  },
];

export function SetupGuide() {
  const { running, stop } = useTourState();
  const navigate = useNavigate();
  const location = useLocation();

  const currentIndex = useMemo(
    () => STEPS.findIndex((s) => location.pathname === s.route),
    [location.pathname],
  );

  const goTo = useCallback(
    (index: number) => {
      const step = STEPS[index];
      if (step) navigate(step.route);
    },
    [navigate],
  );

  // Don't render if tour isn't active or we're not on a guide page
  if (!running || currentIndex === -1) return null;

  const step = STEPS[currentIndex]!;
  const isFirst = currentIndex === 0;
  const isLast = currentIndex === STEPS.length - 1;

  return (
    <div className="mx-auto max-w-4xl px-4 pt-4">
      <div className="rounded-lg border border-brand/30 bg-brand/5 p-4">
        {/* Header row */}
        <div className="flex items-start justify-between gap-3 mb-2">
          <div className="flex items-center gap-2">
            <Sparkles size={16} className="text-brand shrink-0" />
            <h3 className="text-sm font-semibold text-zinc-100">
              {step.title}
            </h3>
            <span className="text-xs text-zinc-500">
              ({currentIndex + 1} of {STEPS.length})
            </span>
          </div>
          <button
            onClick={stop}
            className="shrink-0 rounded p-1 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-700/50"
            title="Dismiss setup guide"
          >
            <X size={14} />
          </button>
        </div>

        {/* Description */}
        <p className="text-sm text-zinc-400 leading-relaxed mb-3 ml-6">
          {step.description}
        </p>

        {/* Navigation */}
        <div className="flex items-center gap-2 ml-6">
          {!isFirst && (
            <button
              onClick={() => goTo(currentIndex - 1)}
              className="inline-flex items-center gap-1 rounded border border-border px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-700/50"
            >
              <ChevronLeft size={12} /> Back
            </button>
          )}
          {!isLast ? (
            <button
              onClick={() => goTo(currentIndex + 1)}
              className="inline-flex items-center gap-1 rounded bg-brand px-3 py-1.5 text-xs font-medium text-white hover:bg-brand-hover"
            >
              Next <ChevronRight size={12} />
            </button>
          ) : (
            <button
              onClick={stop}
              className="inline-flex items-center gap-1 rounded bg-brand px-3 py-1.5 text-xs font-medium text-white hover:bg-brand-hover"
            >
              Finish Setup Guide
            </button>
          )}
          <span className="flex-1" />
          {/* Progress dots */}
          <div className="flex gap-1.5">
            {STEPS.map((_, i) => (
              <button
                key={i}
                onClick={() => goTo(i)}
                className={`h-1.5 rounded-full transition-all ${
                  i === currentIndex
                    ? "w-4 bg-brand"
                    : i < currentIndex
                      ? "w-1.5 bg-brand/50"
                      : "w-1.5 bg-zinc-600"
                }`}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
