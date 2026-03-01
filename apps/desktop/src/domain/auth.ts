export type AuthProvider = "google" | "github";

export type AuthStatus = "signed_out" | "authorizing" | "signed_in";

export interface AuthSnapshot {
  status: AuthStatus;
  provider?: AuthProvider;
  accessToken: string;
  error?: string;
}
