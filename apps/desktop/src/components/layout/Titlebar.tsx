import { Minus, Square, X } from "lucide-react";
import { useTauriWindow } from "../../hooks/useTauriWindow";

export function Titlebar() {
  const { minimize, toggleMaximize, close } = useTauriWindow();

  return (
    <header
      className="flex h-10 shrink-0 items-center justify-between border-b border-[var(--border-primary)] bg-[var(--bg-secondary)]"
      data-tauri-drag-region
    >
      <div className="flex items-center gap-2 pl-3" data-tauri-drag-region>
        <img src="/nebula-32x32.png" alt="Nebula" className="h-5 w-5" />
        <span className="text-sm font-semibold select-none text-[var(--text-primary)]">
          Nebula
        </span>
      </div>

      <div className="flex h-full">
        <button
          type="button"
          onClick={minimize}
          className="flex h-full w-12 items-center justify-center text-[var(--text-tertiary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)] transition-colors"
          aria-label="Minimize"
        >
          <Minus size={16} />
        </button>
        <button
          type="button"
          onClick={toggleMaximize}
          className="flex h-full w-12 items-center justify-center text-[var(--text-tertiary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)] transition-colors"
          aria-label="Maximize"
        >
          <Square size={14} />
        </button>
        <button
          type="button"
          onClick={close}
          className="flex h-full w-12 items-center justify-center text-[var(--text-tertiary)] hover:bg-[var(--error)] hover:text-white transition-colors"
          aria-label="Close"
        >
          <X size={16} />
        </button>
      </div>
    </header>
  );
}
