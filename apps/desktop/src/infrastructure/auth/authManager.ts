import { AuthProvider, AuthSnapshot, AuthUserProfile } from "../../domain/auth";
import { openUrl } from "@tauri-apps/plugin-opener";

const STORAGE_KEY = "nebula.desktop.auth.v1";

interface PersistedAuthState {
  status: AuthSnapshot["status"];
  provider?: AuthProvider;
  accessToken: string;
  user?: AuthUserProfile;
  error?: string;
}

type Listener = () => void;

export class AuthManager {
  private state: PersistedAuthState;
  private snapshot: AuthSnapshot;
  private listeners = new Set<Listener>();

  constructor() {
    this.state = this.load();
    this.snapshot = { ...this.state };
  }

  getSnapshot(): AuthSnapshot {
    return this.snapshot;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async signInWithOAuth(provider: AuthProvider, apiBaseUrl: string): Promise<void> {
    this.state = {
      ...this.state,
      status: "authorizing",
      provider,
      error: undefined,
    };
    this.persistAndNotify();

    // TODO: replace with backend callback handling via deep-link.
    // Current scaffold supports mocked token from /auth/oauth/start.
    try {
      const response = await fetch(`${apiBaseUrl}/auth/oauth/start`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          provider,
          redirectUri: "nebula://auth/callback",
        }),
      });

      if (!response.ok) {
        throw new Error(`oauth start failed: ${response.status}`);
      }

      const payload = (await response.json()) as {
        authUrl?: string;
        accessToken?: string;
        user?: AuthUserProfile;
      };

      if (payload.authUrl) {
        let opened = false;
        try {
          await openUrl(payload.authUrl);
          opened = true;
        } catch {
          // Fallback for environments where opener invoke is unavailable.
        }

        if (!opened) {
          const popup = window.open(payload.authUrl, "_blank", "noopener,noreferrer");
          if (popup) {
            opened = true;
          }
        }

        if (!opened) {
          window.location.assign(payload.authUrl);
        }
      }

      if (payload.accessToken) {
        this.completeSignIn(payload.accessToken, provider, payload.user);
        return;
      }

      this.state = {
        ...this.state,
        status: "authorizing",
        accessToken: "",
        error: undefined,
      };
      this.persistAndNotify();
    } catch (error) {
      const message = error instanceof Error ? error.message : "unknown oauth error";
      this.state = {
        status: "signed_out",
        provider,
        accessToken: "",
        error: message,
      };
      this.persistAndNotify();
    }
  }

  async exchangeOAuthCode(
    code: string,
    provider: AuthProvider,
    apiBaseUrl: string
  ): Promise<void> {
    this.state = {
      ...this.state,
      status: "authorizing",
      provider,
      error: undefined,
    };
    this.persistAndNotify();

    try {
      const response = await fetch(`${apiBaseUrl}/auth/oauth/callback`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          provider,
          code,
          redirectUri: "nebula://auth/callback",
        }),
      });

      if (!response.ok) {
        let details = "";
        try {
          const body = (await response.json()) as { error?: string; message?: string };
          details = body.message ?? body.error ?? "";
        } catch {
          // ignore non-json response body
        }
        throw new Error(
          details
            ? `oauth callback failed: ${response.status} (${details})`
            : `oauth callback failed: ${response.status}`
        );
      }

      const payload = (await response.json()) as {
        accessToken?: string;
        access_token?: string;
        user?: AuthUserProfile;
      };
      const token = payload.accessToken ?? payload.access_token ?? "";
      if (!token) {
        throw new Error("oauth callback returned no access token");
      }

      this.completeSignIn(token, provider, payload.user);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "unknown oauth callback error";
      this.setAuthError(message);
    }
  }

  completeSignIn(token: string, provider?: AuthProvider, user?: AuthUserProfile): void {
    const normalized = token.trim();
    this.state = {
      ...this.state,
      status: normalized ? "signed_in" : "signed_out",
      provider: provider ?? this.state.provider,
      accessToken: normalized,
      user,
      error: undefined,
    };
    this.persistAndNotify();
  }

  signOut(): void {
    this.state = {
      status: "signed_out",
      provider: undefined,
      accessToken: "",
      user: undefined,
      error: undefined,
    };
    this.persistAndNotify();
  }

  setAuthError(message: string): void {
    this.state = {
      ...this.state,
      status: "signed_out",
      accessToken: "",
      user: undefined,
      error: message,
    };
    this.persistAndNotify();
  }

  buildAuthHeaders(): HeadersInit {
    if (this.state.status !== "signed_in" || !this.state.accessToken) {
      return {};
    }
    return {
      Authorization: `Bearer ${this.state.accessToken}`,
    };
  }

  private persistAndNotify(): void {
    this.snapshot = { ...this.state };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(this.state));
    for (const listener of this.listeners) {
      listener();
    }
  }

  private load(): PersistedAuthState {
    const fallback: PersistedAuthState = {
      status: "signed_out",
      accessToken: "",
      provider: undefined,
      user: undefined,
      error: undefined,
    };

    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return fallback;
    }

    try {
      const parsed = JSON.parse(raw) as Partial<PersistedAuthState>;
      if (
        (parsed.status === "signed_out" ||
          parsed.status === "authorizing" ||
          parsed.status === "signed_in") &&
        typeof parsed.accessToken === "string"
      ) {
        // Do not restore transient "authorizing" across app restarts.
        const status = parsed.status === "authorizing" ? "signed_out" : parsed.status;
        return {
          status,
          provider: parsed.provider,
          accessToken: parsed.accessToken,
          user: parsed.user,
          error: parsed.error,
        };
      }
    } catch {
      // Ignore malformed persisted auth settings.
    }

    return fallback;
  }
}
