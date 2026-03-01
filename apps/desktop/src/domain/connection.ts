export type Profile = "local" | "selfhosted" | "saas";

export type ConnectionMode = "local" | "remote";

export interface ConnectionDefaults {
  profile: Profile;
  initialMode: ConnectionMode;
  localBaseUrl: string;
  remoteBaseUrl: string;
}

export interface ConnectionSnapshot {
  mode: ConnectionMode;
  localBaseUrl: string;
  remoteBaseUrl: string;
  activeBaseUrl: string;
}
