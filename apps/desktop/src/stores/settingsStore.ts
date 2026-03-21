import { create } from "zustand";
import { LazyStore } from "@tauri-apps/plugin-store";

export type Theme = "light" | "dark" | "system";
export type Locale = "en" | "ru";

interface SettingsState {
  theme: Theme;
  locale: Locale;
  sidebarCollapsed: boolean;
  initialized: boolean;
}

interface SettingsActions {
  initialize: () => Promise<void>;
  setTheme: (theme: Theme) => Promise<void>;
  setLocale: (locale: Locale) => Promise<void>;
  toggleSidebar: () => Promise<void>;
}

const store = new LazyStore("nebula-settings.json");

const DEFAULTS: Omit<SettingsState, "initialized"> = {
  theme: "system",
  locale: "en",
  sidebarCollapsed: false,
};

export const useSettingsStore = create<SettingsState & SettingsActions>(
  (set, get) => ({
    ...DEFAULTS,
    initialized: false,

    initialize: async () => {
      const theme = ((await store.get<Theme>("theme")) ?? DEFAULTS.theme) as Theme;
      const locale = ((await store.get<Locale>("locale")) ?? DEFAULTS.locale) as Locale;
      const sidebarCollapsed =
        (await store.get<boolean>("sidebarCollapsed")) ?? DEFAULTS.sidebarCollapsed;

      set({ theme, locale, sidebarCollapsed, initialized: true });
    },

    setTheme: async (theme) => {
      await store.set("theme", theme);
      await store.save();
      set({ theme });
    },

    setLocale: async (locale) => {
      await store.set("locale", locale);
      await store.save();
      set({ locale });
    },

    toggleSidebar: async () => {
      const sidebarCollapsed = !get().sidebarCollapsed;
      await store.set("sidebarCollapsed", sidebarCollapsed);
      await store.save();
      set({ sidebarCollapsed });
    },
  })
);
