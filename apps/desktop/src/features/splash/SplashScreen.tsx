import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Spinner } from "../../components/ui/Spinner";

interface SplashScreenProps {
  /** Resolves when the app is ready to show the main window. */
  onReady?: () => void;
}

export function SplashScreen({ onReady }: SplashScreenProps) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    let mounted = true;

    async function init() {
      try {
        // Give the app a moment to initialize, then close splash
        await invoke("close_splashscreen");
      } catch {
        // Command may not exist yet – silently ignore
      }

      if (!mounted) return;

      // Start fade-out
      setVisible(false);

      // Wait for transition to complete before notifying parent
      setTimeout(() => {
        if (mounted) onReady?.();
      }, 300);
    }

    init();

    return () => {
      mounted = false;
    };
  }, [onReady]);

  return (
    <div
      className={`fixed inset-0 z-50 flex flex-col items-center justify-center bg-[var(--bg-primary)] transition-opacity duration-300 ${
        visible ? "opacity-100" : "opacity-0 pointer-events-none"
      }`}
    >
      <h1 className="mb-4 text-3xl font-bold text-[var(--text-primary)]">Nebula</h1>
      <Spinner size="lg" />
      <p className="mt-3 text-sm text-[var(--text-secondary)]">{t("common.loading")}</p>
    </div>
  );
}
