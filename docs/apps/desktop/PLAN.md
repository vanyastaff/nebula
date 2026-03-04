# Implementation Plan: Desktop App (Tauri)

**App**: `nebula-desktop` | **Path**: `apps/desktop` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The desktop app is a Tauri 2 application (React + TypeScript frontend, Rust backend) that provides the visual workflow editor and execution monitor for Nebula. It replaces the old egui `nebula-app`. Backend phases unlock desktop phases: Phase 1 (Core) unblocks Foundation, Phase 2 (Execution Engine) unblocks workflow canvas and monitor.

## Technical Context

**Stack**: Tauri 2, React, TypeScript, Vite, Zustand, TanStack Query
**IPC**: `tauri-specta` for typed commands/events — no raw `invoke` strings
**Persistence**: `tauri-plugin-store` (replaces localStorage)
**Build**: `pnpm` in `apps/desktop/`
**Testing**: Vitest (frontend), `cargo test -p nebula-desktop` (backend)

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Foundation | 🔄 In Progress | Tauri scaffold done, GitHub OAuth done; typed IPC, Zustand stores, TanStack Query planned |
| Phase 2: Workflow Management + Multi-Connection | ⬜ Planned | Workflow list/canvas, multi-connection profiles |
| Phase 3: Monitor and Registry | ⬜ Planned | Real-time execution monitoring, node catalog, credentials |
| Phase 4: Resources and Workspaces | ⬜ Planned | Resource lifecycle viewer, workspace/tenant management |
| Phase 5: Polish and Distribution | ⬜ Planned | Auto-update, code signing, app store distribution |

## Phase Details

### Phase 1: Foundation 🔄

**Goal**: Typed IPC, auth/connection stores, API client, feature-first structure.

**Depends on**: Backend Phase 1 (Core) — complete.

**Deliverables**:
- tauri-specta typed IPC — `commands.*` and `events.*` from `bindings.ts`
- tauri-plugin-store replacing localStorage
- Zustand auth + connection stores
- TanStack Query setup for all API data
- Deep-link handler moved to Rust
- Shared API client with auth headers
- Feature-first directory structure
- AppError + typed events via tauri-specta
- Service layer: AuthService, ConnectionService (thin commands, logic in services)

**Exit Criteria**:
- Zero TypeScript and Rust warnings
- OAuth flow end-to-end on Windows and macOS
- Token survives app restart
- All IPC uses typed `commands.*` / `events.*`

### Phase 2: Workflow Management + Multi-Connection

**Goal**: Create/edit/activate/delete workflows; visual canvas; multiple connection profiles.

**Depends on**: Backend Phase 2 (Execution Engine).

**Deliverables**:
- Workflow list screen (TanStack Query, pagination)
- Workflow CRUD: create/rename/delete/activate/deactivate
- Workflow canvas with React Flow
- Node palette, edge connections, node parameter editor
- Canvas zoom/pan, undo/redo
- ConnectionProfile model replacing simple mode/urls
- Connection switcher UI

**Recommendation**: Start with React Flow; replace with custom if >200 nodes cause perf issues.

**Exit Criteria**:
- Create a 3-node workflow end-to-end
- Canvas handles 50+ nodes without lag
- Parameter editor covers all core types
- User can add second connection and switch between them

### Phase 3: Monitor and Registry

**Goal**: Real-time execution monitoring; node catalog; credentials in canvas.

**Depends on**: Backend Phases 2 + 3 (Plugin System).

**Deliverables**:
- Run list with filters; run detail with node-by-node trace
- Live log streaming (WebSocket via `tauri-plugin-websocket`)
- Manual trigger and cancel
- System tray with run status; native notifications on failure
- Node catalog browser with search
- Credential create/edit/picker in node params

**Exit Criteria**:
- Live log updates within 1 second
- GitHub credential creates + attaches to node from canvas

### Phase 4: Resources and Workspaces

**Goal**: Resource lifecycle viewer; workspace (tenant) management; local mode polish.

**Depends on**: Backend Phase 3 (Resource & Workspace APIs).

**Deliverables**:
- Resource list (active/idle/failed) with force release/restart
- Workspace switcher; workspace settings; member invite
- Local mode: connect to local binary (SQLite + queue-memory), zero Docker

**Exit Criteria**:
- Free user runs Nebula locally without Docker
- Pro user switches local ↔ remote without re-logging in

### Phase 5: Polish and Distribution

**Goal**: App-store-quality release with auto-update and code signing.

**Deliverables**:
- Auto-update via `tauri-plugin-updater`
- Keyboard shortcuts; dark/light theme following system
- Onboarding flow (first launch wizard)
- Google OAuth provider
- Windows code-signing (EV cert or Azure Trusted Signing)
- macOS notarization (Apple Developer ID)
- Linux packages: `.deb`, `.rpm`, `.AppImage`
- Plan upgrade prompt; feature gating wired to `/me` response

## Inter-Crate Dependencies

- **Depends on**: `nebula-api` (REST + WebSocket), `nebula-execution`, `nebula-action`, `nebula-credential`, `nebula-plugin`
- **Backend unlock sequence**: Phase 1 ✅ → Phase 2 🔄 → Phase 3 → Phase 4

## Verification

- [ ] `pnpm run build` — zero TypeScript errors
- [ ] `cargo check -p nebula-desktop`
- [ ] `pnpm run test` — Vitest frontend tests pass
- [ ] `cargo test -p nebula-desktop`
- [ ] Zero TypeScript and Rust warnings in CI
