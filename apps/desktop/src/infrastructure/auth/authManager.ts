import { AuthProvider, AuthSnapshot } from "../../domain/auth";

const STORAGE_KEY = "nebula.desktop.auth.v1";

interface PersistedAuthState {
  status: AuthSnapshot["status"];
  provider?: AuthProvider;
  accessToken: string;
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
      };

      if (payload.authUrl) {
        window.open(payload.authUrl, "_blank", "noopener,noreferrer");
      }

      if (payload.accessToken) {
        this.completeSignIn(payload.accessToken, provider);
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
        throw new Error(`oauth callback failed: ${response.status}`);
      }

      const payload = (await response.json()) as {
        accessToken?: string;
        access_token?: string;
      };
      const token = payload.accessToken ?? payload.access_token ?? "";
      if (!token) {
        throw new Error("oauth callback returned no access token");
      }

      this.completeSignIn(token, provider);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "unknown oauth callback error";
      this.setAuthError(message);
    }
  }

  completeSignIn(token: string, provider?: AuthProvider): void {
    const normalized = token.trim();
    this.state = {
      ...this.state,
      status: normalized ? "signed_in" : "signed_out",
      provider: provider ?? this.state.provider,
      accessToken: normalized,
      error: undefined,
    };
    this.persistAndNotify();
  }

  signOut(): void {
    this.state = {
      status: "signed_out",
      provider: undefined,
      accessToken: "",
      error: undefined,
    };
    this.persistAndNotify();
  }

  setAuthError(message: string): void {
    this.state = {
      ...this.state,
      status: "signed_out",
      accessToken: "",
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
        return {
          status: parsed.status,
          provider: parsed.provider,
          accessToken: parsed.accessToken,
          error: parsed.error,
        };
      }
    } catch {
      // Ignore malformed persisted auth settings.
    }

    return fallback;
  }
}
