import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { commands, type AuthState as RawAuthState, type UserProfile } from "../../bindings";

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
  startOAuth: (provider: string, apiBaseUrl: string) => Promise<void>;
  signOut: () => Promise<void>;
}

export const useAuthStore = create<AuthState & AuthActions>((set) => ({
  status: "signed_out",
  accessToken: "",
  initialized: false,

  initialize: async () => {
    const raw = await commands.getAuthState();
    set({ ...normalize(raw), initialized: true });

    // Rust emits this event whenever auth state changes
    await listen<typeof raw>("auth_state_changed", (event) => {
      set(normalize(event.payload));
    });
  },

  startOAuth: async (provider, apiBaseUrl) => {
    await commands.startOAuth(provider, apiBaseUrl);
  },

  signOut: async () => {
    await commands.signOut();
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
