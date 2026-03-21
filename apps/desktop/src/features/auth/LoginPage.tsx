import { useTranslation } from "react-i18next";
import { Github } from "lucide-react";
import { useNavigate, useLocation } from "react-router";
import { useEffect } from "react";
import { Card } from "../../components/ui/Card";
import { Button } from "../../components/ui/Button";
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

export function LoginPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const { status, error, login } = useAuthStore();

  const isLoading = status === "authorizing";
  const from = (location.state as { from?: string })?.from ?? "/";

  // Redirect when signed in
  useEffect(() => {
    if (status === "signed_in") {
      navigate(from, { replace: true });
    }
  }, [status, navigate, from]);

  const handleLogin = (provider: string) => {
    login(provider);
  };

  return (
    <div className="flex h-screen items-center justify-center bg-[var(--bg-primary)]">
      <Card className="w-full max-w-sm">
        {/* Branding */}
        <div className="mb-6 text-center">
          <h1 className="text-2xl font-bold text-[var(--text-primary)]">Nebula</h1>
          <p className="mt-1 text-sm text-[var(--text-secondary)]">
            {t("dashboard.welcome")}
          </p>
        </div>

        {/* Error */}
        {error && (
          <div className="mb-4 rounded-md bg-[var(--error-bg,rgba(239,68,68,0.1))] px-3 py-2 text-sm text-[var(--error)]">
            {error}
          </div>
        )}

        {/* OAuth buttons */}
        <div className="flex flex-col gap-3">
          <Button
            variant="secondary"
            size="lg"
            className="w-full"
            icon={<GoogleIcon className="h-5 w-5" />}
            loading={isLoading}
            disabled={isLoading}
            onClick={() => handleLogin("google")}
          >
            {t("auth.signInGoogle", "Sign in with Google")}
          </Button>

          <Button
            variant="secondary"
            size="lg"
            className="w-full"
            icon={<Github className="h-5 w-5" />}
            loading={isLoading}
            disabled={isLoading}
            onClick={() => handleLogin("github")}
          >
            {t("auth.signInGithub", "Sign in with GitHub")}
          </Button>
        </div>
      </Card>
    </div>
  );
}
