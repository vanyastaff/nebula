import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { ThemeProvider } from "../lib/theme";
import { AppRouter } from "../lib/router";
import { SplashScreen } from "../features/splash/SplashScreen";
import { useSettingsStore } from "../stores/settingsStore";
import { useAuthStore } from "../features/auth/store";
import { useConnectionStore } from "../features/connection/store";
import { useAppStore } from "../stores/appStore";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
    },
  },
});

function AppInitializer({ children }: { children: React.ReactNode }) {
  const [ready, setReady] = useState(false);

  const initSettings = useSettingsStore((s) => s.initialize);
  const initAuth = useAuthStore((s) => s.initialize);
  const initConnection = useConnectionStore((s) => s.initialize);
  const setInitialized = useAppStore((s) => s.setInitialized);
  const setSplashVisible = useAppStore((s) => s.setSplashVisible);

  useEffect(() => {
    async function init() {
      try {
        await initSettings();
        // Connection must init before auth — auth needs activeBaseUrl
        await initConnection();
        await initAuth();
      } catch (err) {
        console.error("App initialization failed:", err);
      } finally {
        setInitialized(true);
        setReady(true);
      }
    }

    void init();
  }, [initSettings, initConnection, initAuth, setInitialized]);

  const handleSplashReady = useCallback(() => {
    setSplashVisible(false);
  }, [setSplashVisible]);

  return (
    <>
      {!ready && <SplashScreen onReady={handleSplashReady} />}
      {ready && children}
    </>
  );
}

export function AppProvider() {
  return (
    <ErrorBoundary>
      <ThemeProvider>
        <QueryClientProvider client={queryClient}>
          <AppInitializer>
            <AppRouter />
          </AppInitializer>
        </QueryClientProvider>
      </ThemeProvider>
    </ErrorBoundary>
  );
}
