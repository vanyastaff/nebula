import { Github, Globe } from "lucide-react"; // Globe used in URL input
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate } from "react-router";
import { Button } from "../../components/ui/Button";
import { useConnectionStore } from "../connection/store";
import { useAuthStore } from "./store";

/** Google icon – Lucide doesn't ship one, so we inline a small SVG. */
function GoogleIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" />
      <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
      <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" />
      <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" />
    </svg>
  );
}

type Tab = "local" | "remote";

const TAB_LABELS: Record<Tab, string> = {
  local: "Local",
  remote: "Remote",
};

export function LoginPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { status, error, login } = useAuthStore();
  const { config, setMode, setLocalBaseUrl, setRemoteBaseUrl } = useConnectionStore();

  const [activeTab, setActiveTab] = useState<Tab>(config.mode);
  const [localUrl, setLocalUrl] = useState(config.localBaseUrl);
  const [remoteUrl, setRemoteUrl] = useState(config.remoteBaseUrl);

  const isLoading = status === "authorizing";
  const from = (location.state as { from?: string })?.from ?? "/";

  useEffect(() => {
    if (status === "signed_in") {
      navigate(from, { replace: true });
    }
  }, [status, navigate, from]);

  const handleTabChange = async (tab: Tab) => {
    setActiveTab(tab);
    await setMode(tab);
  };

  const handleUrlBlur = async () => {
    if (activeTab === "local") {
      await setLocalBaseUrl(localUrl);
    } else {
      await setRemoteBaseUrl(remoteUrl);
    }
  };

  const currentUrl = activeTab === "local" ? localUrl : remoteUrl;
  const setCurrentUrl = activeTab === "local" ? setLocalUrl : setRemoteUrl;

  return (
    <div className="flex h-screen items-center justify-center bg-[var(--bg-primary)]">
      <div className="w-full max-w-[440px]">

        {/* Card */}
        <div className="overflow-hidden rounded-xl border border-[var(--border-primary)] bg-[var(--bg-elevated)] shadow-[var(--shadow-lg)]">

          {/* Header */}
          <div className="px-8 pt-8 pb-6">
            {/* Logo */}
            <div className="mb-6 flex items-center gap-2">
              <div className="flex h-6 w-6 items-center justify-center rounded bg-[var(--accent)] text-xs font-bold text-[var(--accent-text)]">
                N
              </div>
              <span className="text-base font-semibold text-[var(--text-primary)]">Nebula</span>
              <span className="rounded-md border border-[var(--border-primary)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--text-tertiary)]">
                alpha
              </span>
            </div>

            {/* Title */}
            <h1 className="text-xl font-semibold text-[var(--text-primary)]">
              {t("auth.signInTitle", "Sign in to workspace")}
            </h1>
            <p className="mt-1.5 text-sm text-[var(--text-secondary)]">
              {t("auth.signInSubtitle", "Connect to your Nebula instance to get started")}
            </p>
          </div>

          {/* Divider */}
          <div className="h-px bg-[var(--border-primary)]" />

          {/* Body */}
          <div className="space-y-4 px-8 py-6">

            {/* Error */}
            {error && (
              <div className="rounded-lg border border-[var(--error)] border-opacity-30 bg-[var(--error-subtle)] px-4 py-3 text-sm text-[var(--error)]">
                {error}
              </div>
            )}

            {/* Connection type */}
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-[var(--text-tertiary)]">
                {t("auth.connectionType", "Connection type")}
              </label>
              <div className="flex gap-1 rounded-lg border border-[var(--border-primary)] bg-[var(--bg-secondary)] p-1">
                {(["local", "remote"] as const).map((tab) => (
                  <button
                    key={tab}
                    type="button"
                    onClick={() => handleTabChange(tab)}
                    className={`flex-1 rounded-md py-1.5 text-sm font-medium transition-all ${
                      activeTab === tab
                        ? "bg-[var(--bg-elevated)] text-[var(--text-primary)] shadow-[var(--shadow-sm)]"
                        : "text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]"
                    }`}
                  >
                    {TAB_LABELS[tab]}
                  </button>
                ))}
              </div>
            </div>

            {/* Server URL */}
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-[var(--text-tertiary)]">
                {t("auth.serverUrl", "Server URL")}
              </label>
              <div className="flex h-9 items-center overflow-hidden rounded-lg border border-[var(--border-primary)] bg-[var(--bg-secondary)] transition-colors focus-within:border-[var(--border-focus)] focus-within:ring-2 focus-within:ring-[var(--border-focus)] focus-within:ring-opacity-20">
                <Globe className="ml-3 h-3.5 w-3.5 shrink-0 text-[var(--text-tertiary)]" />
                <input
                  type="text"
                  value={currentUrl}
                  onChange={(e) => setCurrentUrl(e.target.value)}
                  onBlur={handleUrlBlur}
                  className="h-full flex-1 bg-transparent px-2.5 text-sm text-[var(--text-primary)] placeholder:text-[var(--text-tertiary)] outline-none"
                  placeholder={
                    activeTab === "local"
                      ? "http://localhost:5678"
                      : "https://nebula.example.com"
                  }
                  spellCheck={false}
                />
              </div>
            </div>
          </div>

          {/* Divider */}
          <div className="h-px bg-[var(--border-primary)]" />

          {/* Footer */}
          <div className="space-y-4 px-8 py-6">

            {/* OAuth label */}
            <p className="text-center text-xs text-[var(--text-tertiary)]">
              {t("auth.continueWith", "Sign in with")}
            </p>

            {/* OAuth buttons */}
            <div className="grid grid-cols-2 gap-2">
              <Button
                variant="secondary"
                size="md"
                className="w-full"
                icon={<Github className="h-4 w-4" />}
                loading={isLoading}
                disabled={isLoading}
                onClick={() => login("github")}
              >
                GitHub
              </Button>
              <Button
                variant="secondary"
                size="md"
                className="w-full"
                icon={<GoogleIcon className="h-4 w-4" />}
                loading={isLoading}
                disabled={isLoading}
                onClick={() => login("google")}
              >
                Google
              </Button>
            </div>

            {/* Links */}
            <div className="flex items-center justify-center gap-3">
              <a
                href="https://docs.nebula.dev"
                target="_blank"
                rel="noreferrer"
                className="text-xs text-[var(--text-tertiary)] transition-colors hover:text-[var(--text-primary)]"
              >
                {t("auth.footerDocs", "Docs")}
              </a>
              <span className="text-[var(--border-secondary)]">·</span>
              <a
                href="https://github.com/nebula-dev/nebula"
                target="_blank"
                rel="noreferrer"
                className="text-xs text-[var(--text-tertiary)] transition-colors hover:text-[var(--text-primary)]"
              >
                {t("auth.footerGitHub", "GitHub")}
              </a>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
