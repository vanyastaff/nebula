import { create } from "zustand";
import { commands } from "../../bindings";

export type ConnectionMode = "local" | "remote";

export interface ConnectionConfig {
  mode: ConnectionMode;
  localBaseUrl: string;
  remoteBaseUrl: string;
}

interface ConnectionState {
  config: ConnectionConfig;
  activeBaseUrl: string;
  initialized: boolean;
}

interface ConnectionActions {
  initialize: () => Promise<void>;
  setMode: (mode: ConnectionMode) => Promise<void>;
  setLocalBaseUrl: (url: string) => Promise<void>;
  setRemoteBaseUrl: (url: string) => Promise<void>;
}

const DEFAULT: ConnectionConfig = {
  mode: "local",
  localBaseUrl: "http://localhost:5678",
  remoteBaseUrl: "",
};

function active(cfg: ConnectionConfig): string {
  return cfg.mode === "local" ? cfg.localBaseUrl : cfg.remoteBaseUrl;
}

function normalize(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

export const useConnectionStore = create<ConnectionState & ConnectionActions>(
  (set, get) => ({
    config: DEFAULT,
    activeBaseUrl: DEFAULT.localBaseUrl,
    initialized: false,

    initialize: async () => {
      const raw = await commands.getConnection();
      const config: ConnectionConfig = {
        mode: raw.mode as ConnectionMode,
        localBaseUrl: raw.localBaseUrl,
        remoteBaseUrl: raw.remoteBaseUrl,
      };
      set({ config, activeBaseUrl: active(config), initialized: true });
    },

    setMode: async (mode) => {
      const config = { ...get().config, mode };
      await commands.setConnection({
        mode,
        localBaseUrl: config.localBaseUrl,
        remoteBaseUrl: config.remoteBaseUrl,
      });
      set({ config, activeBaseUrl: active(config) });
    },

    setLocalBaseUrl: async (url) => {
      const config = { ...get().config, localBaseUrl: normalize(url) };
      await commands.setConnection({
        mode: config.mode,
        localBaseUrl: config.localBaseUrl,
        remoteBaseUrl: config.remoteBaseUrl,
      });
      set({ config, activeBaseUrl: active(config) });
    },

    setRemoteBaseUrl: async (url) => {
      const config = { ...get().config, remoteBaseUrl: normalize(url) };
      await commands.setConnection({
        mode: config.mode,
        localBaseUrl: config.localBaseUrl,
        remoteBaseUrl: config.remoteBaseUrl,
      });
      set({ config, activeBaseUrl: active(config) });
    },
  })
);
