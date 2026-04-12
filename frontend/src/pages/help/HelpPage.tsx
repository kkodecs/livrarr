import { useState, useRef, useEffect } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import {
  Copy,
  Check,
  Compass,
  Bot,
  ExternalLink,
  BookOpen,
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

Please include a Summary and then a more detailed answer.
`;

  return prompt;
}

function CopyButton({ getText }: { getText: () => string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    const text = getText();
    // Clipboard API is unavailable in non-secure contexts (HTTP on LAN IP).
    // Check explicitly before calling, then fall back to execCommand.
    if (navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
        return;
      } catch {
        // Permission denied or other error — fall through to fallback
      }
    }
    // Fallback: temporary textarea + execCommand
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.left = "-9999px";
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    try {
      document.execCommand("copy");
    } catch {
      // Silently fail — nothing more we can do
    }
    document.body.removeChild(ta);
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
  const [searchParams] = useSearchParams();
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

  // If ?question= is in the URL, pre-fill the prompt with that question.
  const prefillQuestion = searchParams.get("question");
  useEffect(() => {
    if (prefillQuestion && status) {
      const prompt = buildPrompt(status, logs ?? []).replace(
        QUESTION_PLACEHOLDER,
        prefillQuestion,
      );
      setPromptText(prompt);
    }
  }, [prefillQuestion, status, logs]);

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
    <div className="mx-auto max-w-2xl px-3 sm:px-6 py-4 sm:py-6 space-y-4 sm:space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-zinc-100">Help</h1>
        <p className="mt-1 text-sm text-zinc-400">
          Setup guide, AI-assisted troubleshooting, and documentation.
        </p>
      </div>

      {/* Setup Guide */}
      <section className="rounded-lg border border-border bg-surface p-3 sm:p-5">
        <div className="flex items-start gap-3">
          <Compass size={20} className="mt-0.5 text-brand shrink-0 hidden sm:block" />
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
      <section className="rounded-lg border border-border bg-surface p-3 sm:p-5">
        <div className="flex items-start gap-3">
          <Bot size={20} className="mt-0.5 text-brand shrink-0 hidden sm:block" />
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
              <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-1 mb-1">
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
                wrap="off"
                className="w-full rounded border border-zinc-700 bg-zinc-900 px-3 py-2 text-xs text-zinc-300 font-mono focus:outline-none focus:border-brand resize-y overflow-x-auto"
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
      <section className="rounded-lg border border-border bg-surface p-3 sm:p-5">
        <div className="flex items-start gap-3">
          <BookOpen size={20} className="mt-0.5 text-brand shrink-0 hidden sm:block" />
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

    </div>
  );
}
