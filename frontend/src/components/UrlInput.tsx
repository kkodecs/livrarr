import { useState, useEffect } from "react";

function splitUrl(url: string): { protocol: string; host: string } {
  if (url.startsWith("https://")) return { protocol: "https://", host: url.slice(8) };
  if (url.startsWith("http://")) return { protocol: "http://", host: url.slice(7) };
  return { protocol: "http://", host: url };
}

export function UrlInput({
  value,
  onChange,
  placeholder,
  disabled,
  className,
}: {
  value: string;
  onChange: (url: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
}) {
  const [protocol, setProtocol] = useState("http://");
  const [host, setHost] = useState("");

  useEffect(() => {
    const parts = splitUrl(value);
    setProtocol(parts.protocol);
    setHost(parts.host);
  }, [value]);

  const emit = (p: string, h: string) => {
    onChange(h ? p + h : "");
  };

  return (
    <div className={`flex ${className ?? ""}`}>
      <select
        value={protocol}
        onChange={(e) => {
          setProtocol(e.target.value);
          emit(e.target.value, host);
        }}
        disabled={disabled}
        className="rounded-l border border-r-0 border-border bg-zinc-800 px-2 text-sm text-zinc-300 focus:border-brand focus:outline-none disabled:opacity-50"
      >
        <option value="http://">http://</option>
        <option value="https://">https://</option>
      </select>
      <input
        value={host}
        onChange={(e) => {
          setHost(e.target.value);
          emit(protocol, e.target.value);
        }}
        placeholder={placeholder}
        disabled={disabled}
        className="w-full rounded-r border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-600 focus:border-brand focus:outline-none disabled:opacity-50"
      />
    </div>
  );
}
