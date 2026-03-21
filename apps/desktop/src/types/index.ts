export type { Theme, Locale } from "@/stores/settingsStore";

export type {
  AuthStatus,
  UserProfile,
  AuthState,
  ConnectionMode,
  ConnectionConfig,
} from "@/bindings";

export interface NavItem {
  id: string;
  labelKey: string;
  icon: string;
  path: string;
  requiresAuth?: boolean;
}
