import { Moon, Sun, Calendar, Clock } from "lucide-react";
import { useUIStore } from "@/stores/ui";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";

const dateFormats = [
  { value: "MMM d, yyyy", label: "Mar 31, 2026" },
  { value: "yyyy-MM-dd", label: "2026-03-31" },
  { value: "dd/MM/yyyy", label: "31/03/2026" },
  { value: "MM/dd/yyyy", label: "03/31/2026" },
];

export default function UISettingsPage() {
  const relativeDates = useUIStore((s) => s.relativeDates);
  const setRelativeDates = useUIStore((s) => s.setRelativeDates);
  const dateFormat = useUIStore((s) => s.dateFormat);
  const setDateFormat = useUIStore((s) => s.setDateFormat);

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">UI Settings</h1>
      </PageToolbar>

      <PageContent className="max-w-xl space-y-8">
        {/* ── Theme ── */}
        <section>
          <div className="flex items-center gap-2 mb-4">
            <Moon size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">Theme</h2>
          </div>
          <div className="flex gap-3">
            <button className="flex items-center gap-2 rounded border-2 border-brand bg-zinc-800 px-4 py-3 text-sm font-medium text-zinc-100">
              <Moon size={16} /> Dark
            </button>
            <button
              disabled
              title="Coming Soon"
              className="flex items-center gap-2 rounded border border-border bg-zinc-800/50 px-4 py-3 text-sm text-zinc-500 cursor-not-allowed"
            >
              <Sun size={16} /> Light
            </button>
          </div>
        </section>

        {/* ── Date Format ── */}
        <section>
          <div className="flex items-center gap-2 mb-4">
            <Calendar size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">
              Date Format
            </h2>
          </div>
          <select
            value={dateFormat}
            onChange={(e) => setDateFormat(e.target.value)}
            className="rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          >
            {dateFormats.map((f) => (
              <option key={f.value} value={f.value}>
                {f.label} ({f.value})
              </option>
            ))}
          </select>
        </section>

        {/* ── Relative Dates ── */}
        <section>
          <div className="flex items-center gap-2 mb-4">
            <Clock size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">
              Relative Dates
            </h2>
          </div>
          <label className="flex items-center gap-3 text-sm text-zinc-200 cursor-pointer">
            <input
              type="checkbox"
              checked={relativeDates}
              onChange={(e) => setRelativeDates(e.target.checked)}
              className="rounded border-border"
            />
            Show relative dates (e.g. "2 hours ago") instead of absolute dates
          </label>
        </section>
      </PageContent>
    </>
  );
}
