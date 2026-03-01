import {
  ConnectionDefaults,
  ConnectionMode,
  ConnectionSnapshot,
} from "../../domain/connection";

const STORAGE_KEY = "nebula.desktop.connection.v1";

type Listener = () => void;

interface PersistedConnectionState {
  mode: ConnectionMode;
  localBaseUrl: string;
  remoteBaseUrl: string;
}

function normalizeBaseUrl(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

function toSnapshot(state: PersistedConnectionState): ConnectionSnapshot {
  const activeBaseUrl =
    state.mode === "local" ? state.localBaseUrl : state.remoteBaseUrl;

  return {
    ...state,
    activeBaseUrl,
  };
}

export class ConnectionManager {
  private state: PersistedConnectionState;
  private snapshot: ConnectionSnapshot;
  private listeners = new Set<Listener>();

  constructor(defaults: ConnectionDefaults) {
    const initialState = this.load(defaults);
    this.state = {
      ...initialState,
      localBaseUrl: normalizeBaseUrl(initialState.localBaseUrl),
      remoteBaseUrl: normalizeBaseUrl(initialState.remoteBaseUrl),
    };
    this.snapshot = toSnapshot(this.state);
  }

  getSnapshot(): ConnectionSnapshot {
    return this.snapshot;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  setMode(mode: ConnectionMode): void {
    this.state = {
      ...this.state,
      mode,
    };
    this.persistAndNotify();
  }

  setLocalBaseUrl(url: string): void {
    this.state = {
      ...this.state,
      localBaseUrl: normalizeBaseUrl(url),
    };
    this.persistAndNotify();
  }

  setRemoteBaseUrl(url: string): void {
    this.state = {
      ...this.state,
      remoteBaseUrl: normalizeBaseUrl(url),
    };
    this.persistAndNotify();
  }

  private persistAndNotify(): void {
    this.snapshot = toSnapshot(this.state);
    this.persist();
    for (const listener of this.listeners) {
      listener();
    }
  }

  private persist(): void {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(this.state));
  }

  private load(defaults: ConnectionDefaults): PersistedConnectionState {
    const fallback: PersistedConnectionState = {
      mode: defaults.initialMode,
      localBaseUrl: defaults.localBaseUrl,
      remoteBaseUrl: defaults.remoteBaseUrl,
    };

    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return fallback;
    }

    try {
      const parsed = JSON.parse(raw) as Partial<PersistedConnectionState>;
      if (
        (parsed.mode === "local" || parsed.mode === "remote") &&
        typeof parsed.localBaseUrl === "string" &&
        typeof parsed.remoteBaseUrl === "string"
      ) {
        return {
          mode: parsed.mode,
          localBaseUrl: parsed.localBaseUrl,
          remoteBaseUrl: parsed.remoteBaseUrl,
        };
      }
    } catch {
      // Ignore malformed persisted settings and use defaults.
    }

    return fallback;
  }
}
