import { useState, useRef } from "react";
import { useNavigate } from "react-router";
import { useQuery } from "@tanstack/react-query";
import {
  Copy,
  Check,
  Compass,
  Bot,
  ExternalLink,
  BookOpen,
  Info,
  ChevronDown,
  ChevronRight,
} from "lucide-react";
import { getSystemStatus, getLogTail } from "@/api";
import type { SystemStatus } from "@/types/api";

const REPO_URL = "https://github.com/kkodecs/livrarr";
const CONTEXT_FILE_URL = `${REPO_URL}/blob/main/docs/llm-context.md`;
const CONTEXT_RAW_URL = `https://raw.githubusercontent.com/kkodecs/livrarr/main/docs/llm-context.md`;

const QUESTION_PLACEHOLDER = "[Click here to describe your issue or question.]";

function buildPrompt(
  status: SystemStatus | undefined,
  logs: string[],
): string {
  const version = status?.version ?? "unknown";
  const os = status?.osInfo ?? "unknown";
  const uptime = status?.startupTime
    ? `since ${new Date(status.startupTime).toISOString()}`
    : "unknown";

  let prompt = `I need help with Livrarr, a self-hosted book management application (like Sonarr/Radarr but for books).

## My Instance
- Version: ${version}
- OS: ${os}
- Running: ${uptime}

## Context
Before answering, read the Livrarr context file for architecture, configuration, and troubleshooting details:
${CONTEXT_RAW_URL}

`;

  if (logs.length > 0) {
    prompt += `
## Recent Logs (last ${logs.length} lines)
\`\`\`
${logs.join("\n")}
\`\`\`
`;
  }

  prompt += `
## My Question
${QUESTION_PLACEHOLDER}
`;

  return prompt;
}

function CopyButton({ getText }: { getText: () => string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(getText());
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <button
      onClick={handleCopy}
      className="flex items-center gap-2 rounded border border-border bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-300 hover:bg-zinc-700 transition-colors"
    >
      {copied ? (
        <>
          <Check size={14} />
          Copied!
        </>
      ) : (
        <>
          <Copy size={14} />
          Copy to clipboard
        </>
      )}
    </button>
  );
}

export default function HelpPage() {
  const navigate = useNavigate();
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const { data: status } = useQuery({
    queryKey: ["system-status"],
    queryFn: getSystemStatus,
  });
  const { data: logs } = useQuery({
    queryKey: ["log-tail"],
    queryFn: () => getLogTail(20),
  });

  const [promptText, setPromptText] = useState<string | null>(null);
  const [aboutOpen, setAboutOpen] = useState(false);

  // Build prompt once data loads, but only if user hasn't edited yet
  const defaultPrompt = buildPrompt(status, logs ?? []);
  const displayText = promptText ?? defaultPrompt;

  // When the user clicks the textarea for the first time, select the placeholder
  const handleFocus = () => {
    if (promptText === null) {
      setPromptText(defaultPrompt);
      // Select the placeholder text so the user can type over it
      requestAnimationFrame(() => {
        const ta = textareaRef.current;
        if (!ta) return;
        const idx = ta.value.indexOf(QUESTION_PLACEHOLDER);
        if (idx >= 0) {
          ta.setSelectionRange(idx, idx + QUESTION_PLACEHOLDER.length);
        }
      });
    }
  };

  return (
    <div className="mx-auto max-w-2xl p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-zinc-100">Help</h1>
        <p className="mt-1 text-sm text-zinc-400">
          Setup guide, AI-assisted troubleshooting, and documentation.
        </p>
      </div>

      {/* Setup Guide */}
      <section className="rounded-lg border border-border bg-surface p-5">
        <div className="flex items-start gap-3">
          <Compass size={20} className="mt-0.5 text-brand shrink-0" />
          <div>
            <h2 className="text-lg font-semibold text-zinc-100">
              Setup Guide
            </h2>
            <p className="mt-1 text-sm text-zinc-400">
              Walk through initial configuration — metadata providers, download
              clients, indexers, and root folders.
            </p>
            <button
              onClick={() => {
                navigate("/");
                window.dispatchEvent(new CustomEvent("livrarr:start-tour"));
              }}
              className="mt-3 flex items-center gap-2 rounded border border-border bg-zinc-800 px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-700 transition-colors"
            >
              <Compass size={14} />
              Start setup guide
            </button>
          </div>
        </div>
      </section>

      {/* AI Help */}
      <section className="rounded-lg border border-border bg-surface p-5">
        <div className="flex items-start gap-3">
          <Bot size={20} className="mt-0.5 text-brand shrink-0" />
          <div className="flex-1 min-w-0">
            <h2 className="text-lg font-semibold text-zinc-100">
              Get AI Help
            </h2>
            <p className="mt-1 text-sm text-zinc-400">
              A ready-made prompt with your instance details, docker config, and
              recent logs. Edit your question below, then copy and paste into any
              AI assistant.
            </p>

            {/* Editable prompt */}
            <div className="mt-4">
              <div className="flex items-center justify-between mb-1">
                <label className="text-xs font-medium text-zinc-400">
                  Prompt
                </label>
                <CopyButton getText={() => textareaRef.current?.value ?? displayText} />
              </div>
              <textarea
                ref={textareaRef}
                value={displayText}
                onChange={(e) => setPromptText(e.target.value)}
                onFocus={handleFocus}
                rows={16}
                className="w-full rounded border border-zinc-700 bg-zinc-900 px-3 py-2 text-xs text-zinc-300 font-mono focus:outline-none focus:border-brand resize-y"
              />
            </div>

            <div className="mt-2">
              <a
                href={CONTEXT_FILE_URL}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-300 transition-colors"
              >
                <ExternalLink size={12} />
                View context file on GitHub
              </a>
            </div>
          </div>
        </div>
      </section>

      {/* Documentation */}
      <section className="rounded-lg border border-border bg-surface p-5">
        <div className="flex items-start gap-3">
          <BookOpen size={20} className="mt-0.5 text-brand shrink-0" />
          <div>
            <h2 className="text-lg font-semibold text-zinc-100">
              Documentation
            </h2>
            <p className="mt-1 text-sm text-zinc-400">
              Source code, issues, contact information, and documentation on GitHub.
            </p>
            <a
              href={REPO_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-3 inline-flex items-center gap-1.5 text-sm text-brand hover:text-brand/80 transition-colors"
            >
              <ExternalLink size={14} />
              github.com/kkodecs/livrarr
            </a>
          </div>
        </div>
      </section>

      {/* About */}
      <section className="rounded-lg border border-border bg-surface">
        <button
          onClick={() => setAboutOpen((o) => !o)}
          className="flex w-full items-center gap-3 p-5 text-left"
        >
          <Info size={20} className="text-brand shrink-0" />
          <h2 className="flex-1 text-lg font-semibold text-zinc-100">
            About Livrarr
          </h2>
          {aboutOpen ? (
            <ChevronDown size={16} className="text-zinc-500" />
          ) : (
            <ChevronRight size={16} className="text-zinc-500" />
          )}
        </button>
        {aboutOpen && (
          <div className="space-y-3 text-sm text-zinc-400 leading-relaxed px-5 pb-5 pl-12">
            <p>
              Livrarr (from <em>livre</em>, French for "book") is a self-hosted
              ebook and audiobook library manager built for the Servarr
              ecosystem. It automates the entire workflow from
              searching for books to organizing tagged files in your library,
              working alongside tools like Prowlarr, qBittorrent, SABnzbd,
              Calibre-Web Automated, and Audiobookshelf.
            </p>
            <p>
              The project manages both ebooks and audiobooks in a single
              application — users search for works, not formats, and grab
              releases in whichever media type they want. It was born from
              studying why earlier efforts in this space struggled. A detailed
              post-mortem of Readarr's architecture informed every design
              decision — from the works-first data model (books, not authors,
              are the primary entity) to the multi-user isolation that's built
              in from day one rather than retrofitted.
            </p>
            <p>
              Livrarr is built entirely with AI-assisted development. Not a
              single line of code was written by hand. The backend is
              approximately 25,000 lines of Rust across 10 crates; the frontend
              is React/TypeScript. The entire codebase was generated through a
              rigorous pipeline: detailed specification, cross-family adversarial
              review (Claude, Gemini, and GPT each reviewing each other's blind
              spots), intermediate representation (typed Rust signatures that
              constrain the generation space), behavioral tests written before
              implementation, and Rust's own type system as a final reviewer.
            </p>
            <p>
              When code generation is cheap, the specification becomes the
              critical input — not the hand-written code. Rust was chosen for the
              same reason: runtime performance and compile-time safety matter
              more when the cost of writing code approaches zero.
            </p>
            <p>
              Our sincere thanks to the user community for your support and
              feedback. You make this project better.
            </p>
            <p className="italic">
              — The Livrarr Dev Team, April 11, 2026
            </p>
          </div>
        )}
      </section>
    </div>
  );
}
