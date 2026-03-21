import { Minus, Square, X } from "lucide-react";
import { useTauriWindow } from "../../hooks/useTauriWindow";

export function Titlebar() {
  const { minimize, toggleMaximize, close } = useTauriWindow();

  return (
    <header
      className="flex h-10 shrink-0 items-center justify-between border-b border-neutral-200 bg-white dark:border-neutral-700 dark:bg-neutral-900"
      data-tauri-drag-region
    >
      <div className="flex items-center gap-2 pl-3" data-tauri-drag-region>
        <img src="/nebula-32x32.png" alt="Nebula" className="h-5 w-5" />
        <span className="text-sm font-semibold select-none text-neutral-800 dark:text-neutral-200">
          Nebula
        </span>
      </div>

      <div className="flex h-full">
        <button
          type="button"
          onClick={minimize}
          className="flex h-full w-12 items-center justify-center text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Minimize"
        >
          <Minus size={16} />
        </button>
        <button
          type="button"
          onClick={toggleMaximize}
          className="flex h-full w-12 items-center justify-center text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Maximize"
        >
          <Square size={14} />
        </button>
        <button
          type="button"
          onClick={close}
          className="flex h-full w-12 items-center justify-center text-neutral-500 hover:bg-red-500 hover:text-white dark:text-neutral-400"
          aria-label="Close"
        >
          <X size={16} />
        </button>
      </div>
    </header>
  );
}
