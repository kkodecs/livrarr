import { useState, type ReactNode } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";

export function ConfirmModal({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  variant = "danger",
  onConfirm,
  children,
  typeConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: "danger" | "default";
  onConfirm: () => void | Promise<void>;
  children?: ReactNode;
  typeConfirm?: string;
}) {
  const [confirming, setConfirming] = useState(false);
  const [typedValue, setTypedValue] = useState("");

  const canConfirm = typeConfirm ? typedValue === typeConfirm : true;

  const handleConfirm = async () => {
    setConfirming(true);
    try {
      await onConfirm();
      onOpenChange(false);
    } finally {
      setConfirming(false);
      setTypedValue("");
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-black/60 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <Dialog.Content className="fixed left-1/2 top-1/2 w-full max-w-md -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-zinc-800 p-6 shadow-xl">
          <div className="flex items-start justify-between">
            <Dialog.Title className="text-lg font-semibold text-zinc-100">
              {title}
            </Dialog.Title>
            <Dialog.Close className="rounded p-1 text-muted hover:text-zinc-100">
              <X size={18} />
            </Dialog.Close>
          </div>
          {description && (
            <Dialog.Description className="mt-2 text-sm text-muted">
              {description}
            </Dialog.Description>
          )}
          {children}
          {typeConfirm && (
            <div className="mt-4">
              <label className="text-sm text-muted">
                Type{" "}
                <span className="font-mono font-medium text-zinc-200">
                  {typeConfirm}
                </span>{" "}
                to confirm
              </label>
              <input
                type="text"
                value={typedValue}
                onChange={(e) => setTypedValue(e.target.value)}
                className="mt-1 w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
                autoFocus
              />
            </div>
          )}
          <div className="mt-6 flex justify-end gap-3">
            <Dialog.Close className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100">
              {cancelLabel}
            </Dialog.Close>
            <button
              onClick={handleConfirm}
              disabled={confirming || !canConfirm}
              className={`rounded px-4 py-2 text-sm font-medium text-white disabled:opacity-50 ${
                variant === "danger"
                  ? "bg-red-600 hover:bg-red-700"
                  : "bg-brand hover:bg-brand-hover"
              }`}
            >
              {confirming ? "Processing..." : confirmLabel}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
