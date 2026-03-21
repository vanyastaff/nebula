import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback } from "react";

interface UseTauriWindowReturn {
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<void>;
  close: () => Promise<void>;
  startDrag: () => Promise<void>;
}

export function useTauriWindow(): UseTauriWindowReturn {
  const minimize = useCallback(async () => {
    await getCurrentWindow().minimize();
  }, []);

  const toggleMaximize = useCallback(async () => {
    await getCurrentWindow().toggleMaximize();
  }, []);

  const close = useCallback(async () => {
    await getCurrentWindow().close();
  }, []);

  const startDrag = useCallback(async () => {
    await getCurrentWindow().startDragging();
  }, []);

  return { minimize, toggleMaximize, close, startDrag };
}
