import { create } from "zustand";

interface AppState {
  initialized: boolean;
  splashVisible: boolean;
}

interface AppActions {
  setInitialized: (initialized: boolean) => void;
  setSplashVisible: (visible: boolean) => void;
}

export const useAppStore = create<AppState & AppActions>((set) => ({
  initialized: false,
  splashVisible: true,

  setInitialized: (initialized) => set({ initialized }),
  setSplashVisible: (splashVisible) => set({ splashVisible }),
}));
