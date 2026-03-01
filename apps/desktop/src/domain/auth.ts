export type AuthProvider = "google" | "github";

export type AuthStatus = "signed_out" | "authorizing" | "signed_in";

export interface AuthUserProfile {
  id: string;
  login: string;
  name?: string;
  email?: string;
  avatarUrl?: string;
}

export interface AuthSnapshot {
  status: AuthStatus;
  provider?: AuthProvider;
  accessToken: string;
  user?: AuthUserProfile;
  error?: string;
}
