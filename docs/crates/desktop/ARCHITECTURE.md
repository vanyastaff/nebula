# Desktop App — Architecture

## Overview

The Nebula desktop app is a **full client** to the Nebula backend API, built with Tauri 2.
It is not a thin shell — it manages auth state, connection profiles, and native OS integration
on the Rust side, while the React layer handles all UI and API data fetching.

---

## Key Decisions

### Decision 1: Tauri over pure-Rust UI (gpui / egui / iced)

**Chosen:** Tauri 2 (WebView + Rust backend)

**Rationale:**
Platform integration features required by Nebula desktop — OAuth deep-link callback,
secure OS credential storage, system tray, auto-update — each take 20–50 hours to
implement from scratch in a pure-Rust UI framework. Tauri provides all of them as
first-party plugins. The WebView is a *native* system component (WKWebView on macOS,
WebView2 on Windows, WebKitGTK on Linux) — not a bundled browser like Electron.

**Rejected alternatives:**
- **gpui** — GPU-accelerated, production-proven in Zed, but macOS-first, unstable API,
  no platform integration primitives. Revisit when ecosystem matures.
- **egui / iced** — good for tool UIs, insufficient widget ecosystem for workflow canvas.

---

### Decision 2: Hybrid IPC (not full Rust proxy)

**Chosen:** React calls HTTP API directly; Rust owns only auth + connection + native features.

```
React ──── HTTP API (workflows, runs, credentials)
  │
  └── invoke() ──── Rust (auth token, connection config, tray, notifications)
```

**Rationale:**
Making Rust a proxy for every API call adds 200+ commands of boilerplate with no benefit —
the HTTP API is already typed via OpenAPI. Rust handles what only Rust can do: secure
storage, deep-link, system tray, OAuth browser open. React handles what React is good at:
data fetching, caching, UI reactivity.

**Rejected alternative:** Full Rust proxy (Axis pattern) — valid when Rust IS the backend
(like a Git CLI wrapper), not when a separate HTTP service already exists.

---

### Decision 3: Feature-first structure with pragmatic internal layers

**Chosen:** Feature-first top level, internal layers only where complexity warrants.

**Rationale:**
Flat structure (components/ hooks/ services/) does not scale beyond 3–4 features —
cross-feature imports become implicit and hard to trace. Full DDD (domain/ application/
infrastructure/ presentation/ per feature) adds ceremony for features that are just
a store + a screen. The right level is: feature boundary is always respected,
internal structure grows with complexity.

**Rule:**
> A simple feature is a `store.ts` and a `ui/` folder.
> A complex feature adds `types.ts`, `queries.ts`, and sub-folders for screens.
> Never add a layer until you feel the pain of not having it.

---

### Decision 4: Multi-connection model (ConnectionProfile[])

**Chosen:** Array of connection profiles, one active at a time — like Slack workspaces.

**Rationale:**
A developer runs a local Nebula instance and also connects to a self-hosted team instance.
The simple `{ mode, localBaseUrl, remoteBaseUrl }` model cannot express multiple remote
connections. Each profile has its own URL and its own auth state (different accounts
per server). The active profile determines which URL and token `apiFetch` uses.

```
ConnectionProfile[] stored in nebula-connections.json
  ├── id: "local-default" | uuid
  ├── name: "Local" | "Work Server" | ...
  ├── url: "http://localhost:5678" | "https://nebula.mycompany.com"
  └── auth: AuthState (per-connection token + user)
```

Free plan: 1 connection (local only).
Pro plan: unlimited connections (local + remote).

---

### Decision 5: Workspace = Tenant

**Chosen:** In the UI, "Workspace" maps to `Tenant` in the backend DB schema.

**Rationale:**
The backend migrations define `tenant` as the top-level isolation boundary. Calling it
"workspace" in the UI is the standard SaaS convention (Slack, Notion, Linear). The term
"organization" is avoided as it implies user directory management not present in v1.

```
Tenant (backend)  =  Workspace (UI)
Project (backend) =  Project (UI, Pro feature)
User              =  Member
```

Workspace switcher is a **Pro feature** — Free plan sees only one workspace (their own).

---

## Directory Structure

```
apps/desktop/
├── src/
│   ├── app/
│   │   └── providers.tsx          # QueryClient + store init
│   ├── features/
│   │   │
│   │   │   ── Orchestration ──────────────────────────────────
│   │   ├── workflows/             # COMPLEX — grows with canvas
│   │   │   ├── types.ts           # Workflow domain types
│   │   │   ├── queries.ts         # TanStack Query hooks
│   │   │   ├── canvas/            # Node graph editor
│   │   │   └── list/              # Workflow list + CRUD UI
│   │   ├── monitor/               # Live execution streaming
│   │   │   ├── types.ts
│   │   │   ├── queries.ts         # useRuns, useRun, useRunLogs
│   │   │   └── ui/                # Run list, trace view, log panel
│   │   │
│   │   │   ── Infrastructure ─────────────────────────────────
│   │   ├── registry/              # Node catalog (browse + search)
│   │   │   ├── types.ts
│   │   │   ├── queries.ts         # useNodes, useNodeDefinition
│   │   │   └── ui/
│   │   ├── resources/             # Resource lifecycle viewer
│   │   │   ├── types.ts
│   │   │   ├── queries.ts
│   │   │   └── ui/
│   │   ├── credentials/           # Credential management
│   │   │   ├── types.ts
│   │   │   ├── queries.ts
│   │   │   └── ui/
│   │   │
│   │   │   ── Platform ──────────────────────────────────────
│   │   ├── auth/
│   │   │   ├── store.ts           # Zustand auth store (per active connection)
│   │   │   └── ui/                # LoginScreen, UserBadge
│   │   ├── connection/
│   │   │   ├── store.ts           # ConnectionProfile[] store (backed by Rust)
│   │   │   └── ui/                # ConnectionSettings, switcher
│   │   ├── workspaces/            # Tenant switcher (Pro feature)
│   │   │   ├── store.ts
│   │   │   ├── queries.ts
│   │   │   └── ui/                # WorkspaceSwitcher, WorkspaceSettings
│   │   └── shell/                 # App chrome: sidebar, statusbar
│   │       └── ui/                # Sidebar nav, StatusBar, ConnectionPill
│   │
│   ├── shared/
│   │   ├── api/
│   │   │   └── client.ts          # apiFetch with auth headers
│   │   ├── ui/                    # Design system: Button, Input, etc.
│   │   └── hooks/                 # Generic hooks (useDebounce, etc.)
│   ├── bindings.ts                # Auto-generated by tauri-specta (do not edit)
│   ├── ui/
│   │   └── App.tsx                # Root shell + routing
│   ├── main.tsx
│   └── styles.css
└── src-tauri/
    ├── src/
    │   ├── commands/
    │   │   ├── mod.rs
    │   │   ├── auth.rs            # get_auth_state, start_oauth, sign_out
    │   │   └── connection.rs      # list_connections, set_active_connection, upsert_connection
    │   ├── services/
    │   │   ├── auth.rs            # AuthService (all OAuth logic)
    │   │   └── connection.rs      # ConnectionService (profile management)
    │   ├── models/
    │   │   ├── auth.rs            # AuthState, UserProfile, AuthStatus
    │   │   └── connection.rs      # ConnectionProfile
    │   ├── events/
    │   │   └── auth.rs            # AuthStateChanged (typed tauri-specta event)
    │   ├── error.rs               # AppError (Serialize + Type for TS)
    │   ├── state.rs               # AppState { auth, connection }
    │   ├── deep_link.rs           # nebula://auth/callback handler
    │   └── lib.rs                 # Tauri builder + specta wiring
    └── Cargo.toml
```

---

## State Architecture

```
┌────────────────────────────────────────────────────────┐
│                     React UI                           │
│                                                        │
│  useAuthStore()        useConnectionStore()            │
│       │                      │                         │
│       └────── Zustand ────────┘                        │
│                  ↕ invoke / listen                     │
│  useWorkflowsQuery()   useRunsQuery()                  │
│       │                      │                         │
│       └──── TanStack Query ───┘                        │
│                  ↕ apiFetch → activeProfile.url        │
│              HTTP API  (active connection)             │
└─────────────────┬──────────────────────────────────────┘
                  │ tauri-specta commands + events
┌─────────────────▼──────────────────────────────────────┐
│                  Rust (src-tauri)                       │
│                                                        │
│  tauri-plugin-store   ←→  ConnectionProfile[]          │
│  (nebula-connections.json) id, name, url, auth         │
│                            activeProfileId             │
│                                                        │
│  tauri-plugin-tray         system tray                 │
│  tauri-plugin-updater      auto-update                 │
│  tauri-plugin-deep-link    nebula:// scheme            │
└────────────────────────────────────────────────────────┘
```

### State ownership

| State | Owner | Why |
|-------|-------|-----|
| Auth token (per connection) | Rust (plugin-store) | Secure, persists across restarts, OS-protected |
| Connection profiles | Rust (plugin-store) | Needs to survive reinstall, not browser-scoped |
| Active profile ID | Rust (plugin-store) | Survives restart |
| Workflow list | TanStack Query | Server-owned, needs cache + refetch |
| Run status | TanStack Query | Real-time via polling or WebSocket |
| Canvas layout | React local state | Ephemeral, per-session |
| Modal / drawer open | React local state | Pure UI, no persistence needed |

---

## IPC Architecture

### Commands (React → Rust)

Typed via **tauri-specta**. TypeScript types are auto-generated from Rust signatures.
Never use raw `invoke('string')` — always use the generated `commands.*` object from `bindings.ts`.

```typescript
// ✅ correct
import { commands } from "../bindings";
const state = await commands.getAuthState();

// ❌ wrong
const state = await invoke<AuthState>("get_auth_state");
```

### Events (Rust → React)

Rust emits typed events; React listens via the generated `events.*` object from `bindings.ts`.
Events are used for **push notifications** — state changes initiated by Rust
(OAuth callback, deep-link arrival, tray action).

```typescript
// ✅ correct — typed, from bindings
await events.authStateChanged.listen((event) => {
  useAuthStore.setState(normalize(event.payload));
});

// ❌ wrong — raw string, not type-checked
await listen("auth_state_changed", ...);
```

### When to use commands vs events

| Use | Direction | Example |
|-----|-----------|---------|
| Commands | React → Rust | `start_oauth`, `set_active_connection` |
| Events | Rust → React | `auth_state_changed`, `update_available` |

---

## Dependency Rules

These rules are enforced by convention (no tooling yet).

```
features/X/ui        → features/X/queries  ✓
features/X/ui        → features/X/store    ✓
features/X/queries   → shared/api          ✓
features/X/store     → bindings.ts         ✓
features/X           → features/Y          ✗  never cross-feature
features/X           → shared/*            ✓
shared/*             → features/*          ✗  never upward
app/providers        → features/*/store    ✓  init only
shell/               → features/*/store    ✓  read-only for nav state
```

---

## Progressive Feature Disclosure

The same backend data model exists at all tiers. The UI gates **access** to features by plan,
not the backend. This keeps the backend simple and lets UI-level gating be changed without
a backend deploy.

| Feature | Free | Pro | Enterprise |
|---------|------|-----|------------|
| Connections | 1 (local) | Unlimited | Unlimited |
| Connection switcher UI | Hidden | Visible | Visible |
| Workspace (Tenant) switcher | Hidden | Visible | Visible |
| Projects within workspace | Hidden | Visible | Visible |
| Monitor history (days) | 7 | 90 | Unlimited |
| Credentials | 3 | Unlimited | Unlimited |
| Node registry (community) | Hidden | Visible | Visible |
| SSO / SAML | — | — | Available |
| Audit log | — | — | Available |

**Implementation:** Plan tier is returned in the auth token claims or a `/me` endpoint.
The `useAuthStore` exposes `plan: "free" | "pro" | "enterprise"`.
Feature components read it to conditionally render or redirect to upgrade prompt.

```typescript
// Example: gate connection switcher
const { plan } = useAuthStore();
if (plan === "free") return <SingleConnectionPill />;
return <ConnectionSwitcher />;
```

**Rule:** Never hide data — hide controls. A free-plan user who manually crafts an API
request should not be blocked by the UI layer alone. Backend enforces limits.

---

## Local Mode (Zero Docker)

When `ConnectionProfile.url` points to a local instance, Nebula backend can run without
any external infrastructure:

| Component | Normal (Docker) | Local mode |
|-----------|----------------|------------|
| Database | PostgreSQL | SQLite (sqlx feature flag) |
| Queue | Redis | nebula-runtime MemoryQueue (in-process) |
| Binary | Separate process | Same binary, different feature flags |

Local mode is the default experience for Free plan users. Setup = download binary, run it,
connect the desktop app to `http://localhost:5678`. No Docker, no config files.

---

## Technology Stack

| Layer | Library | Version |
|-------|---------|---------|
| Desktop framework | Tauri | 2.x |
| IPC type safety | tauri-specta | 0.20 |
| Persistence | tauri-plugin-store | 2.x |
| Deep-link | tauri-plugin-deep-link | 2.x |
| Log streaming | tauri-plugin-websocket | 2.x |
| HTTP (Rust) | reqwest | 0.12 |
| UI framework | React | 18.x |
| Client state | Zustand | 5.x |
| Server state | TanStack Query | 5.x |
| Build tool | Vite | 6.x |
| Language | TypeScript | 5.x |
