# Desktop App — Reference

All public contracts: Rust commands, Tauri events, Zustand stores, shared utilities.

---

## Rust Commands

Defined in `src-tauri/src/commands/`.
TypeScript types auto-generated to `src/bindings.ts` by tauri-specta on debug build.

### Auth commands

| Command | Input | Output | Notes |
|---------|-------|--------|-------|
| `get_auth_state` | — | `AuthState` | Never returns `authorizing` — clears it on load |
| `start_oauth` | `provider: string`, `apiBaseUrl: string` | `Result<void>` | Opens browser, emits `auth_state_changed` |
| `sign_out` | — | `Result<void>` | Clears store, emits `auth_state_changed` |

> `complete_oauth` is internal (called by the deep-link handler, not from React).

### Connection commands

| Command | Input | Output | Notes |
|---------|-------|--------|-------|
| `list_connections` | — | `ConnectionProfile[]` | Reads from `nebula-connections.json` |
| `get_active_connection` | — | `ConnectionProfile \| null` | Currently active profile |
| `upsert_connection` | `ConnectionProfile` | `Result<void>` | Create or update by id |
| `delete_connection` | `id: string` | `Result<void>` | Removes profile; clears auth for it |
| `set_active_connection` | `id: string` | `Result<void>` | Switches active profile, emits event |

### System commands

| Command | Input | Output | Notes |
|---------|-------|--------|-------|
| `get_api_profile` | — | `string` | Returns `NEBULA_API_PROFILE` env var or `"local"` |

---

## Rust Types (IPC)

Defined in `src-tauri/src/types.rs`. Mirrored to TypeScript via tauri-specta.

```rust
pub struct AuthState {
    pub status: AuthStatus,        // "signed_out" | "authorizing" | "signed_in"
    pub provider: Option<String>,  // "github" | "google"
    pub access_token: String,
    pub user: Option<UserProfile>,
    pub error: Option<String>,
}

pub struct UserProfile {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

pub struct ConnectionProfile {
    pub id: String,                   // uuid or "local-default"
    pub name: String,                 // "Local" | "Work Server" | ...
    pub url: String,                  // "http://localhost:5678" | "https://..."
    pub auth: AuthState,              // per-connection token + user
}
```

---

## Tauri Events

Events emitted by Rust, consumed by React via `listen()`.

| Event | Payload | Emitted when |
|-------|---------|--------------|
| `auth_state_changed` | `AuthState` | OAuth starts, completes, fails, sign-out |
| `active_connection_changed` | `ConnectionProfile \| null` | Active profile switches |
| `update_available` | `{ version: string }` | Auto-updater finds new version *(planned)* |
| `tray_action` | `{ action: string }` | User clicks tray menu item *(planned)* |

---

## Zustand Stores

### `useAuthStore` — `src/features/auth/store.ts`

```typescript
interface AuthStore {
  // State
  status: "signed_out" | "authorizing" | "signed_in";
  provider?: string;
  accessToken: string;
  user?: AuthUser;
  error?: string;
  initialized: boolean;

  // Actions
  initialize(): Promise<void>;       // call once in Providers
  startOAuth(provider: string, apiBaseUrl: string): Promise<void>;
  signOut(): Promise<void>;
}
```

**Usage:**
```typescript
const { status, user, startOAuth } = useAuthStore();
// or for imperative access outside React:
const token = useAuthStore.getState().accessToken;
```

**Notes:**
- `initialize()` must be called once at app startup (done in `app/providers.tsx`).
- Subscribes to `auth_state_changed` Tauri event — state updates automatically
  when OAuth callback arrives.
- Do not call `initialize()` more than once.

---

### `useConnectionStore` — `src/features/connection/store.ts`

```typescript
interface ConnectionProfile {
  id: string;        // uuid or "local-default"
  name: string;      // display name
  url: string;       // base URL for this connection
  auth: AuthState;   // per-connection token + user
}

interface ConnectionStore {
  // State
  profiles: ConnectionProfile[];
  activeProfile: ConnectionProfile | null;
  activeBaseUrl: string;   // derived: activeProfile?.url ?? ""
  initialized: boolean;

  // Actions
  initialize(): Promise<void>;
  setActiveConnection(id: string): Promise<void>;
  upsertConnection(profile: ConnectionProfile): Promise<void>;
  deleteConnection(id: string): Promise<void>;
}
```

**Usage:**
```typescript
const { profiles, activeProfile, setActiveConnection } = useConnectionStore();
// In apiFetch:
const { activeBaseUrl } = useConnectionStore.getState();
```

**Notes:**
- `activeBaseUrl` is derived from `activeProfile.url` — used by `shared/api/client.ts`.
- Each action persists immediately to Rust store — no explicit save needed.
- Free plan users always have exactly one profile (`local-default`); the switcher UI is hidden.
- Pro plan users can add remote connections; the switcher renders as a sidebar dropdown.

---

## TanStack Query Hooks (planned)

Conventions for all feature query hooks.

### Naming

```typescript
// List
export function useWorkflows(params?: WorkflowListParams) { ... }
// Single
export function useWorkflow(id: string) { ... }
// Mutation
export function useCreateWorkflow() { ... }
export function useUpdateWorkflow() { ... }
export function useDeleteWorkflow() { ... }
```

### Query key factory pattern

Each feature exports a `keys` object to keep query keys consistent:

```typescript
// features/workflows/queries.ts
export const workflowKeys = {
  all: ["workflows"] as const,
  list: (params?: WorkflowListParams) => [...workflowKeys.all, "list", params] as const,
  detail: (id: string) => [...workflowKeys.all, "detail", id] as const,
};
```

### Standard query options

```typescript
{
  staleTime: 30_000,   // 30 seconds — don't refetch on every focus
  retry: 1,
}
```

For real-time data (run status):
```typescript
{
  refetchInterval: 3_000,  // poll every 3 seconds while run is active
}
```

---

## Shared API Client

`src/shared/api/client.ts`

```typescript
async function apiFetch(path: string, init?: RequestInit): Promise<Response>
```

- Reads `activeBaseUrl` from `useConnectionStore.getState()`.
- Adds `Authorization: Bearer <token>` when signed in.
- Throws on non-2xx by default — feature query hooks handle errors.

**Do not use `fetch()` directly in feature code. Always use `apiFetch`.**

---

## Persistence Store Paths

Rust stores data via `tauri-plugin-store` in the app's data directory.

| File | Contents | Owner |
|------|----------|-------|
| `nebula-connections.json` | `ConnectionProfile[]` + `activeProfileId` | `commands/connection.rs` |

Location on disk (resolved by Tauri):
- Windows: `%APPDATA%\nebula-desktop\`
- macOS: `~/Library/Application Support/nebula-desktop/`
- Linux: `~/.local/share/nebula-desktop/`
