import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { type AuthState as RawAuthState, type UserProfile, commands } from "../../bindings";

// Mirror the Rust AuthState shape
export type AuthStatus = "signed_out" | "authorizing" | "signed_in";
export type { UserProfile };

interface AuthState {
  status: AuthStatus;
  provider?: string;
  accessToken: string;
  user?: UserProfile;
  error?: string;
  initialized: boolean;
}

interface AuthActions {
  initialize: () => Promise<void>;
  login: (provider: string) => Promise<void>;
  logout: () => Promise<void>;
  checkAuth: (provider: string) => Promise<UserProfile | null>;
  refreshToken: (provider: string) => Promise<void>;
}

export const useAuthStore = create<AuthState & AuthActions>((set) => ({
  status: "signed_out",
  accessToken: "",
  initialized: false,

  initialize: async () => {
    const raw = await commands.getAuthState();
    set({ ...normalize(raw), initialized: true });

    // Rust emits this event whenever auth state changes
    await listen<RawAuthState>("auth_state_changed", (event) => {
      set(normalize(event.payload));
    });
  },

  login: async (provider) => {
    set({ status: "authorizing", error: undefined });
    try {
      const result = await commands.authLogin(provider);
      if (result.status === "ok") {
        set(normalize(result.data));
      } else {
        set({ status: "signed_out", error: String(result.error) });
      }
    } catch (e) {
      set({ status: "signed_out", error: String(e) });
    }
  },

  logout: async () => {
    try {
      await commands.authLogout();
      set({
        status: "signed_out",
        accessToken: "",
        user: undefined,
        provider: undefined,
        error: undefined,
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  checkAuth: async (provider) => {
    try {
      const result = await commands.authGetUser(provider);
      if (result.status === "ok") {
        set({ user: result.data });
        return result.data;
      }
      return null;
    } catch {
      return null;
    }
  },

  refreshToken: async (provider) => {
    try {
      const result = await commands.authRefreshToken(provider);
      if (result.status === "ok") {
        set(normalize(result.data));
      } else {
        set({ error: String(result.error) });
      }
    } catch (e) {
      set({ error: String(e) });
    }
  },
}));

function normalize(raw: RawAuthState): Omit<AuthState, "initialized"> {
  return {
    status: raw.status as AuthStatus,
    provider: raw.provider ?? undefined,
    accessToken: raw.accessToken,
    user: raw.user ?? undefined,
    error: raw.error ?? undefined,
  };
}
