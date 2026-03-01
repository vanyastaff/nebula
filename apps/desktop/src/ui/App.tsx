import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { handleDeepLinkUrls } from "../application/oauthCallback";
import { useAuthSnapshot, useConnectionSnapshot } from "../application/hooks";
import { authManager } from "../application/services";
import { AuthProvider } from "../domain/auth";

export function App() {
  const connection = useConnectionSnapshot();
  const auth = useAuthSnapshot();
  const [version, setVersion] = useState<string>("...");

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    async function setupDeepLinkHandlers() {
      const current = await getCurrent();
      if (current && current.length > 0) {
        await handleDeepLinkUrls(current);
      }

      unlisten = await onOpenUrl((urls) => {
        void handleDeepLinkUrls(urls);
      });
    }

    setupDeepLinkHandlers().catch((error) => {
      const message =
        error instanceof Error ? error.message : "failed to initialize deep-link handlers";
      authManager.setAuthError(message);
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    getVersion()
      .then((value) => setVersion(value))
      .catch(() => setVersion("0.1.0"));
  }, []);

  async function signIn(provider: AuthProvider) {
    await authManager.signInWithOAuth(provider, connection.activeBaseUrl);
  }

  return (
    <main
      style={{
        width: "100%",
        minHeight: "100dvh",
        margin: 0,
        padding: 0,
        boxSizing: "border-box",
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        alignItems: "center",
        position: "relative",
        overflow: "hidden",
        fontFamily: "'Segoe UI', 'Inter', sans-serif",
        background:
          "radial-gradient(1200px 520px at 20% -10%, #1c2b4f 0%, transparent 55%), radial-gradient(1000px 540px at 95% 110%, #0f3b2f 0%, transparent 60%), #0a0f1d",
        color: "#edf2ff",
      }}
    >
      <section
        style={{
          width: "min(460px, calc(100% - 24px))",
          margin: "12px",
          background: "rgba(14, 20, 38, 0.82)",
          border: "1px solid rgba(151, 165, 198, 0.2)",
          borderRadius: 14,
          padding: "clamp(16px, 3.2vw, 28px)",
          boxShadow: "0 12px 40px rgba(0, 0, 0, 0.35)",
        }}
      >
        <h1 style={{ margin: 0, fontSize: 26, letterSpacing: 0.3 }}>Nebula</h1>
        <p style={{ marginTop: 8, marginBottom: 20, color: "#b8c5e6", fontSize: 14 }}>
          Sign in to continue.
        </p>

        <button
          onClick={() => signIn("github")}
          disabled={auth.status === "authorizing"}
          style={{
            width: "100%",
            padding: "11px 14px",
            borderRadius: 10,
            border: "1px solid rgba(184, 197, 230, 0.35)",
            background: "transparent",
            color: "#edf2ff",
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          Continue with GitHub
        </button>

        {auth.status === "authorizing" ? (
          <p style={{ marginTop: 14, marginBottom: 0, color: "#b8c5e6", fontSize: 13 }}>
            Waiting for OAuth callback...
          </p>
        ) : null}
        {auth.error ? (
          <p style={{ marginTop: 14, marginBottom: 0, color: "#ffb7b7", fontSize: 13 }}>
            {auth.error}
          </p>
        ) : null}
      </section>

      <footer
        style={{
          position: "absolute",
          bottom: 10,
          left: 0,
          right: 0,
          textAlign: "center",
          fontSize: 12,
          color: "#8ea0cf",
          opacity: 0.9,
        }}
      >
        v{version}
      </footer>
    </main>
  );
}
