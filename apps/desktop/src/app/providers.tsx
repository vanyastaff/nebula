import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { useAuthStore } from "../features/auth/store";
import { useConnectionStore } from "../features/connection/store";
import { useCredentialStore } from "../features/credentials/store";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
    },
  },
});

export function Providers({ children }: { children: React.ReactNode }) {
  const initAuth = useAuthStore((s) => s.initialize);
  const initConnection = useConnectionStore((s) => s.initialize);
  const initCredentials = useCredentialStore((s) => s.initialize);

  useEffect(() => {
    // Initialize connection first — auth start_oauth needs activeBaseUrl
    void initConnection()
      .then(() => initAuth())
      .then(() => initCredentials());
  }, [initAuth, initConnection, initCredentials]);

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}
