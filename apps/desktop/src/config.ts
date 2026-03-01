type Profile = "local" | "selfhosted" | "saas";

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

export const profile = normalizeProfile(import.meta.env.VITE_NEBULA_PROFILE);
export const apiBaseUrl =
  import.meta.env.VITE_NEBULA_API_URL ?? defaultApiBaseUrl(profile);
