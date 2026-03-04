# Desktop App — Roadmap

Aligned with the backend roadmap in `docs/ROADMAP.md`.
Desktop phases unlock as backend phases complete.

---

## Phase 1 — Foundation ✅ / 🔄

**Goal:** Auth, connection management, typed IPC, native OS integration.
**Depends on:** Backend Phase 1 (Core) — done.

| Feature | Status | Notes |
|---------|--------|-------|
| Tauri 2 project scaffold | Done | Vite + React + TypeScript |
| GitHub OAuth via deep-link | Done | `nebula://auth/callback` |
| Connection profiles (local / remote) | Done | Persisted via localStorage |
| tauri-specta typed IPC | Planned | See implementation plan |
| tauri-plugin-store (replace localStorage) | Planned | Secure OS-native persistence |
| Zustand auth + connection stores | Planned | Replace Manager classes |
| TanStack Query setup | Planned | Foundation for all API data |
| Deep-link handler in Rust | Planned | Move from React to Rust |
| Shared API client with auth headers | Planned | `shared/api/client.ts` |
| Feature-first structure migration | Planned | Per `src/ARCHITECTURE.md` |
| AppError + typed events (tauri-specta) | Planned | No raw `invoke` strings anywhere |
| Service layer (AuthService, ConnectionService) | Planned | Commands are thin, logic in services |

**Exit criteria:**
- [ ] App builds with zero TypeScript and Rust warnings
- [ ] OAuth flow works end-to-end on Windows and macOS
- [ ] Token survives app restart
- [ ] Connection URL change takes effect immediately without restart
- [ ] All IPC uses typed `commands.*` and `events.*` from `bindings.ts`

---

## Phase 2 — Workflow Management + Multi-Connection 🔄

**Goal:** Create, edit, activate, delete workflows. Visual canvas. Multiple connections.
**Depends on:** Backend Phase 2 (Execution Engine).

### Workflows

| Feature | Status | Notes |
|---------|--------|-------|
| Workflow list screen | Planned | TanStack Query, pagination |
| Create / rename / delete workflow | Planned | Mutations + optimistic update |
| Activate / deactivate workflow | Planned | Toggle with instant feedback |
| Workflow canvas — node graph | Planned | React Flow (replace with custom if >200 nodes) |
| Node palette — drag onto canvas | Planned | |
| Edge connections between nodes | Planned | |
| Node parameter editor (sidebar) | Planned | Uses `nebula-parameter` schema |
| Canvas zoom + pan | Planned | |
| Undo / redo | Planned | |

### Multi-Connection (Slack-like)

| Feature | Status | Notes |
|---------|--------|-------|
| ConnectionProfile model (replace simple mode/urls) | Planned | `{ id, name, url, auth }[]` |
| Add / edit / remove connection profiles | Planned | Settings panel |
| Connection switcher UI (Pro feature) | Planned | Sidebar pill, dropdown |
| Per-connection auth state | Planned | Each profile has its own token |
| Active connection drives all API calls | Planned | `apiFetch` reads active profile |

**Canvas implementation decision:**

| Option | Pros | Cons |
|--------|------|------|
| [React Flow / XY Flow](https://reactflow.dev) | Production-ready, large community | Dependency, opinionated |
| Custom WebGL canvas | Full control, max performance | 2–4 months of work |
| Custom SVG/Canvas | Simpler than WebGL | Performance ceiling with many nodes |

**Recommendation:** Start with React Flow. Replace with custom if performance becomes an issue at 200+ nodes.

**Exit criteria:**
- [ ] Can create a 3-node workflow end-to-end
- [ ] Canvas handles 50+ nodes without lag
- [ ] Parameter editor covers all core types (string, number, boolean, select, json)
- [ ] User can add a second connection (e.g., remote server) and switch between them

---

## Phase 3 — Monitor & Registry 🔄

**Goal:** Real-time execution monitoring. Node catalog. Credentials.
**Depends on:** Backend Phase 2 (Execution Engine) + Backend Phase 3 (Plugin System).

### Monitor (Execution Observability)

| Feature | Status | Notes |
|---------|--------|-------|
| Run list with filters | Planned | By workflow, status, date |
| Run detail — node-by-node execution trace | Planned | Matches UI mockup |
| Live log streaming | Planned | WebSocket (`tauri-plugin-websocket`) |
| Manual trigger (test run) | Planned | |
| Cancel running execution | Planned | |
| System tray — run status indicator | Planned | Active / idle / error |
| Native notifications on failure | Planned | `tauri-plugin-notification` |

### Registry (Node Catalog)

| Feature | Status | Notes |
|---------|--------|-------|
| Node catalog browser | Planned | Search by name, category |
| Node documentation panel | Planned | Inline help |
| Plugin install / update | Planned | Via backend plugin system |
| Community nodes (Pro) | Planned | Gated by plan |

### Credentials

| Feature | Status | Notes |
|---------|--------|-------|
| Credential list | Planned | |
| Create / edit credential | Planned | Type-specific form (API key, OAuth, etc.) |
| Credential picker in node params | Planned | Inline from canvas |

**Exit criteria:**
- [ ] Live log updates within 1 second of backend event
- [ ] System tray shows correct run status at all times
- [ ] GitHub credential creates + attaches to a GitHub node without leaving canvas
- [ ] All built-in nodes are browseable and searchable

---

## Phase 4 — Resources & Workspaces 🔄

**Goal:** Resource lifecycle viewer. Workspace (Tenant) management. Local mode polish.
**Depends on:** Backend Phase 3 (Resource & Workspace APIs).

### Resources

| Feature | Status | Notes |
|---------|--------|-------|
| Resource list (active / idle / failed) | Planned | |
| Resource detail — lifecycle trace | Planned | |
| Force release / restart resource | Planned | |

### Workspaces (Pro)

| Feature | Status | Notes |
|---------|--------|-------|
| Workspace switcher | Planned | Maps to Tenant in backend |
| Workspace settings | Planned | Members, billing |
| Project list within workspace | Planned | Project = grouping of workflows |
| Invite member flow | Planned | |

### Local Mode

| Feature | Status | Notes |
|---------|--------|-------|
| Connect to local binary (SQLite + runtime MemoryQueue) | Planned | Zero Docker |
| Local mode indicator in connection UI | Planned | "Local" badge |
| Health check / connection status | Planned | Ping, show latency |

**Exit criteria:**
- [ ] Free user runs Nebula locally without Docker installed
- [ ] Pro user can switch between local and remote workspace without re-logging in
- [ ] Resource list correctly reflects backend resource lifecycle

---

## Phase 5 — Polish & Distribution

**Goal:** App-store-quality release.

| Feature | Status | Notes |
|---------|--------|-------|
| Auto-update | Planned | `tauri-plugin-updater` |
| Keyboard shortcuts | Planned | Canvas, global |
| Dark / light theme | Planned | System-follows |
| Onboarding flow | Planned | First launch wizard |
| Google OAuth provider | Planned | Currently GitHub only |
| Windows code-signing | Planned | EV cert or Azure Trusted Signing |
| macOS notarization | Planned | Apple Developer ID |
| Linux packages | Planned | `.deb`, `.rpm`, `.AppImage` |
| Plan upgrade prompt | Planned | When user hits a gated feature |
| Feature gating rollout | Planned | All plan gates wired to `/me` response |

---

## Not Planned

These are explicitly out of scope for the desktop app (v1–v5):

| Feature | Reason |
|---------|--------|
| Embedded backend (run Nebula engine in-process) | Valid future idea, not Phase 1–5 |
| Mobile app | Tauri supports mobile — evaluate after desktop v1 |
| Plugin development IDE | Out of scope, separate tooling |
| SSO / SAML provider configuration | Enterprise backend prerequisite, Phase 5+ |
