# Desktop App — Integration

How the desktop app integrates with the Nebula backend API and OS platform.

---

## Backend API

### Connection modes

| Mode | URL source | Used for |
|------|-----------|---------|
| `local` | `localBaseUrl` (default `http://localhost:5678`) | Local development, embedded future mode |
| `remote` | `remoteBaseUrl` | Self-hosted or SaaS deployment |

Active URL is always `useConnectionStore.getState().activeBaseUrl`.
Switching mode is instant — no restart required.

### Authentication

All API requests include a Bearer token obtained from Rust:

```
Authorization: Bearer <access_token>
```

The token is stored securely in `nebula-auth.json` via `tauri-plugin-store`.
`shared/api/client.ts` injects it automatically — feature code never touches the token directly.

---

## OAuth Flow

### Sequence diagram

```
User clicks "Sign in with GitHub"
        │
        ▼
React calls commands.startOAuth("github", activeBaseUrl)
        │
        ▼
Rust POSTs /auth/oauth/start { provider, redirectUri: "nebula://auth/callback" }
        │
        ▼ backend returns { authUrl: "https://github.com/login/oauth/authorize?..." }
        │
        ▼
Rust opens authUrl in system browser (tauri-plugin-opener)
Rust sets auth state → "authorizing", emits auth_state_changed
        │
        ▼ user authenticates in browser
        │
        ▼ backend redirects → nebula://auth/callback?code=XXX&provider=github
        │
        ▼
OS delivers deep-link to Tauri (tauri-plugin-deep-link)
        │
        ▼
Rust deep_link::handle() parses URL
        │
        ▼
Rust POSTs /auth/oauth/callback { provider, code, redirectUri }
        │
        ▼ backend returns { accessToken, user }
        │
        ▼
Rust saves token + user to nebula-auth.json
Rust emits auth_state_changed { status: "signed_in", accessToken, user }
        │
        ▼
React useAuthStore listener receives event → updates UI
```

### Deep-link registration

On Windows and Linux, Tauri registers `nebula://` scheme via `register_all()` at startup.
On macOS, the scheme is declared in `tauri.conf.json` and registered by the OS at install time.

```json
// src-tauri/tauri.conf.json
{
  "plugins": {
    "deep-link": {
      "desktop": {
        "schemes": ["nebula"]
      }
    }
  }
}
```

### Auth state lifecycle

```
app start → get_auth_state (never returns "authorizing")
         → listen("auth_state_changed")

signed_in → API requests work
         → sign_out → auth cleared

authorizing → browser opened, waiting for deep-link
           → deep-link arrives → signed_in or error
           → app restart → reset to signed_out
```

---

## API Endpoints

Consumed by the desktop app. All relative to `activeBaseUrl`.

### Auth

| Method | Path | Used by |
|--------|------|---------|
| POST | `/auth/oauth/start` | `commands/auth.rs::start_oauth` |
| POST | `/auth/oauth/callback` | `commands/auth.rs::exchange_code` |

### Workflows *(planned — Phase 2)*

| Method | Path | Used by |
|--------|------|---------|
| GET | `/workflows` | `features/workflows/queries.ts::useWorkflows` |
| GET | `/workflows/:id` | `features/workflows/queries.ts::useWorkflow` |
| POST | `/workflows` | `useCreateWorkflow` |
| PATCH | `/workflows/:id` | `useUpdateWorkflow` |
| DELETE | `/workflows/:id` | `useDeleteWorkflow` |
| POST | `/workflows/:id/activate` | `useActivateWorkflow` |

### Runs *(planned — Phase 3)*

| Method | Path | Used by |
|--------|------|---------|
| GET | `/runs` | `features/runs/queries.ts::useRuns` |
| GET | `/runs/:id` | `useRun` |
| GET | `/runs/:id/logs` | `useRunLogs` |
| POST | `/workflows/:id/execute` | `useExecuteWorkflow` |

### Credentials *(planned — Phase 4)*

| Method | Path | Used by |
|--------|------|---------|
| GET | `/credentials` | `features/credentials/queries.ts::useCredentials` |
| POST | `/credentials` | `useCreateCredential` |
| DELETE | `/credentials/:id` | `useDeleteCredential` |

### Nodes *(planned — Phase 4)*

| Method | Path | Used by |
|--------|------|---------|
| GET | `/nodes` | `features/nodes/queries.ts::useNodes` |
| GET | `/nodes/:type` | `useNodeDefinition` |

---

## Error Handling

### API errors

`apiFetch` throws on non-2xx. Feature query hooks catch and expose via TanStack Query:

```typescript
const { data, error, isError } = useWorkflows();
// error.message contains the server message
```

Convention for server error shape:
```json
{ "error": "not_found", "message": "Workflow not found" }
```

### Auth errors

If a request returns 401:
- The feature query hook calls `useAuthStore.getState().signOut()`
- The user is redirected to the login screen

```typescript
// shared/api/client.ts
if (response.status === 401) {
  await useAuthStore.getState().signOut();
}
```

### Rust command errors

All fallible Rust commands return `Result<T, String>`.
tauri-specta converts them to TypeScript `Promise<T>` that throws on error.

```typescript
try {
  await commands.startOAuth(provider, apiBaseUrl);
} catch (e) {
  // e is string from Rust
}
```

Feature stores catch and surface these as `error` state fields.

---

## Platform-specific Notes

### Windows

- Deep-link: registered in Windows Registry via `register_all()` at startup.
- Secure store: data written to `%APPDATA%\nebula-desktop\`.
- WebView: Edge WebView2 (ships with Windows 11, auto-installed on Windows 10).

### macOS

- Deep-link: registered via `Info.plist` URL scheme at install. `register_all()` is skipped.
- Secure store: `~/Library/Application Support/nebula-desktop/`.
- WebView: WKWebView (system, no install needed).

### Linux

- Deep-link: registered via `.desktop` file + `xdg-mime`. Requires `register_all()`.
- Secure store: `~/.local/share/nebula-desktop/`.
- WebView: WebKitGTK (must be installed separately — listed as system dep in `.deb`/`.rpm`).
