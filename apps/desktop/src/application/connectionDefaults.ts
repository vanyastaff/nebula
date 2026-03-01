import { ConnectionDefaults, Profile } from "../domain/connection";

const DEFAULT_PROFILE: Profile = "local";

function normalizeProfile(value: string | undefined): Profile {
  switch ((value ?? "").toLowerCase()) {
    case "local":
      return "local";
    case "selfhosted":
    case "self-hosted":
      return "selfhosted";
    case "saas":
      return "saas";
    default:
      return DEFAULT_PROFILE;
  }
}

function defaultApiBaseUrl(profile: Profile): string {
  switch (profile) {
    case "local":
      return "http://localhost:5678";
    case "selfhosted":
      return "http://localhost:5678";
    case "saas":
      return "https://api.nebula.example.com";
  }
}

function defaultRemoteBaseUrl(profile: Profile): string {
  if (profile === "local") {
    return "https://api.nebula.example.com";
  }
  return defaultApiBaseUrl(profile);
}

export function getConnectionDefaults(): ConnectionDefaults {
  const profile = normalizeProfile(import.meta.env.VITE_NEBULA_PROFILE);
  const configuredApiUrl = import.meta.env.VITE_NEBULA_API_URL;

  return {
    profile,
    initialMode: profile === "local" ? "local" : "remote",
    localBaseUrl: defaultApiBaseUrl("local"),
    remoteBaseUrl: configuredApiUrl ?? defaultRemoteBaseUrl(profile),
  };
}
