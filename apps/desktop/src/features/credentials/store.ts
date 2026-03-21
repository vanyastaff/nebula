import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import {
  type CreateCredentialRequest,
  type Credential as RawCredential,
  type UpdateCredentialRequest,
  commands,
} from "../../bindings";
import {
  type Credential,
  type CredentialListItem,
  normalizeCredential,
  toListItem,
} from "./domain/types";

interface CredentialState {
  credentials: CredentialListItem[];
  selectedCredentialId: string | null;
  initialized: boolean;
  error?: string;
}

interface CredentialActions {
  initialize: () => Promise<void>;
  list: () => Promise<void>;
  get: (id: string) => Promise<Credential | undefined>;
  create: (request: CreateCredentialRequest) => Promise<Credential>;
  update: (id: string, request: UpdateCredentialRequest) => Promise<Credential>;
  delete: (id: string) => Promise<void>;
  select: (id: string | null) => void;
}

export const useCredentialStore = create<CredentialState & CredentialActions>((set, _get) => ({
  credentials: [],
  selectedCredentialId: null,
  initialized: false,

  initialize: async () => {
    try {
      const rawCredentials = await commands.listCredentials();
      const credentials = rawCredentials.map((raw) => toListItem(normalizeCredential(raw)));
      set({ credentials, initialized: true, error: undefined });

      // Rust emits individual events for each CRUD operation
      await listen<RawCredential>("credential_created", (event) => {
        const newCredential = toListItem(normalizeCredential(event.payload));
        set((state) => ({
          credentials: [...state.credentials, newCredential],
          error: undefined,
        }));
      });

      await listen<RawCredential>("credential_updated", (event) => {
        const updatedCredential = toListItem(normalizeCredential(event.payload));
        set((state) => ({
          credentials: state.credentials.map((c) =>
            c.id === updatedCredential.id ? updatedCredential : c,
          ),
          error: undefined,
        }));
      });

      await listen<string>("credential_deleted", (event) => {
        const deletedId = event.payload;
        set((state) => ({
          credentials: state.credentials.filter((c) => c.id !== deletedId),
          selectedCredentialId:
            state.selectedCredentialId === deletedId ? null : state.selectedCredentialId,
          error: undefined,
        }));
      });

      await listen<RawCredential>("credential_rotated", (event) => {
        const rotatedCredential = toListItem(normalizeCredential(event.payload));
        set((state) => ({
          credentials: state.credentials.map((c) =>
            c.id === rotatedCredential.id ? rotatedCredential : c,
          ),
          error: undefined,
        }));
      });
    } catch (error) {
      set({ error: String(error), initialized: true });
    }
  },

  list: async () => {
    try {
      const rawCredentials = await commands.listCredentials();
      const credentials = rawCredentials.map((raw) => toListItem(normalizeCredential(raw)));
      set({ credentials, error: undefined });
    } catch (error) {
      set({ error: String(error) });
    }
  },

  get: async (id: string) => {
    try {
      const result = await commands.getCredential(id);
      if (result.status === "ok") {
        const credential = normalizeCredential(result.data);
        set({ error: undefined });
        return credential;
      }
      set({ error: result.error });
      return undefined;
    } catch (error) {
      set({ error: String(error) });
      return undefined;
    }
  },

  create: async (request: CreateCredentialRequest) => {
    try {
      const result = await commands.createCredential(request);
      if (result.status === "ok") {
        const credential = normalizeCredential(result.data);
        set({ error: undefined });
        // Event listener will update the list automatically
        return credential;
      }
      set({ error: result.error });
      throw new Error(result.error);
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  update: async (id: string, request: UpdateCredentialRequest) => {
    try {
      const result = await commands.updateCredential(id, request);
      if (result.status === "ok") {
        const credential = normalizeCredential(result.data);
        set({ error: undefined });
        // Event listener will update the list automatically
        return credential;
      }
      set({ error: result.error });
      throw new Error(result.error);
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  delete: async (id: string) => {
    try {
      await commands.deleteCredential(id);
      set({ error: undefined, selectedCredentialId: null });
      // Event listener will update the list automatically
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },

  select: (id: string | null) => {
    set({ selectedCredentialId: id });
  },
}));
