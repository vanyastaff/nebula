# Tasks: Desktop App (Tauri)

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `DSK`

---

## Phase 1: Foundation 🔄

**Goal**: Typed IPC, stores, API client, feature-first structure. Backend Phase 1 ✅ complete.

- [x] DSK-T001 Tauri 2 project scaffold with Vite + React + TypeScript
- [x] DSK-T002 GitHub OAuth via deep-link (`nebula://auth/callback`)
- [x] DSK-T003 Connection profiles (local/remote) persisted via localStorage
- [ ] DSK-T004 Migrate to `tauri-plugin-store` — replace localStorage with OS-native store
- [ ] DSK-T005 [P] Set up `tauri-specta` — generate `bindings.ts` with typed `commands.*` and `events.*`
- [ ] DSK-T006 [P] Implement `AppError` type in Rust and expose via tauri-specta
- [ ] DSK-T007 Set up Zustand auth store in `src/features/auth/store.ts`
- [ ] DSK-T008 [P] Set up Zustand connection store in `src/features/connection/store.ts`
- [ ] DSK-T009 Set up TanStack Query in `src/shared/query-client.ts`
- [ ] DSK-T010 [P] Implement shared API client with auth headers in `src/shared/api/client.ts`
- [ ] DSK-T011 Move deep-link handler from React to Rust backend (`src-tauri/src/deep_link.rs`)
- [ ] DSK-T012 Implement `AuthService` in Rust — commands are thin, logic in service
- [ ] DSK-T013 [P] Implement `ConnectionService` in Rust
- [ ] DSK-T014 Migrate to feature-first directory structure per `src/ARCHITECTURE.md`
- [ ] DSK-T015 Verify zero TypeScript and Rust warnings in CI
- [ ] DSK-T016 [P] Verify OAuth flow end-to-end on Windows; [P] verify on macOS
- [ ] DSK-T017 Verify token survives app restart (persisted in plugin-store)

**Checkpoint**: Zero warnings; OAuth works end-to-end; token persists; all IPC uses typed bindings.

---

## Phase 2: Workflow Management + Multi-Connection ⬜

**Goal**: Workflow CRUD and canvas; multi-connection profiles. Requires Backend Phase 2.

- [ ] DSK-T018 Implement workflow list screen with TanStack Query + pagination in `src/features/workflows/`
- [ ] DSK-T019 [P] Implement create/rename/delete workflow mutations with optimistic updates
- [ ] DSK-T020 [P] Implement activate/deactivate workflow toggle with instant feedback
- [ ] DSK-T021 Set up React Flow canvas in `src/features/canvas/`
- [ ] DSK-T022 [P] Implement node palette — drag nodes onto canvas
- [ ] DSK-T023 [P] Implement edge connections between nodes
- [ ] DSK-T024 Implement node parameter editor sidebar using `nebula-parameter` schema
- [ ] DSK-T025 [P] Implement canvas zoom + pan
- [ ] DSK-T026 [P] Implement undo/redo for canvas operations
- [ ] DSK-T027 Define `ConnectionProfile` model: `{ id, name, url, auth }[]` in connection store
- [ ] DSK-T028 [P] Implement add/edit/remove connection profile UI in settings panel
- [ ] DSK-T029 [P] Implement connection switcher UI (sidebar pill/dropdown)
- [ ] DSK-T030 Wire all API calls to read from active connection profile

**Checkpoint**: Create 3-node workflow end-to-end; canvas handles 50+ nodes; multiple connections work.

---

## Phase 3: Monitor and Registry ⬜

**Goal**: Real-time execution monitoring; node catalog; credentials. Requires Backend Phases 2 + 3.

- [ ] DSK-T031 Implement run list screen with filters (workflow, status, date) in `src/features/monitor/`
- [ ] DSK-T032 [P] Implement run detail — node-by-node execution trace view
- [ ] DSK-T033 Implement live log streaming via WebSocket (`tauri-plugin-websocket`)
- [ ] DSK-T034 [P] Implement manual trigger (test run) from run detail screen
- [ ] DSK-T035 [P] Implement cancel running execution
- [ ] DSK-T036 Implement system tray with run status indicator (active/idle/error)
- [ ] DSK-T037 [P] Implement native notifications on execution failure (`tauri-plugin-notification`)
- [ ] DSK-T038 [P] Implement node catalog browser with search in `src/features/registry/`
- [ ] DSK-T039 [P] Implement node documentation panel (inline help)
- [ ] DSK-T040 Implement credential list in `src/features/credentials/`
- [ ] DSK-T041 [P] Implement create/edit credential form (type-specific: API key, OAuth, etc.)
- [ ] DSK-T042 [P] Implement credential picker inline from canvas parameter editor

**Checkpoint**: Live logs within 1 second; system tray reflects run status; GitHub credential attaches from canvas.

---

## Phase 4: Resources and Workspaces ⬜

**Goal**: Resource lifecycle viewer; workspace management; local mode. Requires Backend Phase 3.

- [ ] DSK-T043 Implement resource list screen (active/idle/failed) in `src/features/resources/`
- [ ] DSK-T044 [P] Implement resource detail with lifecycle trace
- [ ] DSK-T045 [P] Implement force release/restart resource actions
- [ ] DSK-T046 Implement workspace switcher in `src/features/workspaces/`
- [ ] DSK-T047 [P] Implement workspace settings (members, billing)
- [ ] DSK-T048 [P] Implement project list within workspace
- [ ] DSK-T049 [P] Implement invite member flow
- [ ] DSK-T050 Implement local mode: connect to local Nebula binary (SQLite + queue-memory)
- [ ] DSK-T051 [P] Add "Local" badge in connection UI for local mode indicator
- [ ] DSK-T052 [P] Implement health check / connection status with ping and latency display

**Checkpoint**: Free user runs Nebula locally without Docker; Pro user switches local ↔ remote seamlessly.

---

## Phase 5: Polish and Distribution ⬜

**Goal**: App-store-quality release with auto-update, code signing, and full platform support.

- [ ] DSK-T053 Implement auto-update via `tauri-plugin-updater`
- [ ] DSK-T054 [P] Implement keyboard shortcuts for canvas and global actions
- [ ] DSK-T055 [P] Implement dark/light theme following system preference
- [ ] DSK-T056 [P] Implement onboarding flow (first launch wizard)
- [ ] DSK-T057 [P] Add Google OAuth provider (in addition to GitHub)
- [ ] DSK-T058 Set up Windows code-signing (EV cert or Azure Trusted Signing)
- [ ] DSK-T059 [P] Set up macOS notarization (Apple Developer ID)
- [ ] DSK-T060 [P] Build Linux packages: `.deb`, `.rpm`, `.AppImage`
- [ ] DSK-T061 Implement plan upgrade prompt when user hits gated feature
- [ ] DSK-T062 [P] Wire all feature gates to `/me` response from backend

**Checkpoint**: App builds and signs on all three platforms; auto-update works; onboarding complete.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 (requires Backend Phase 2 execution engine)
- Phase 2 → Phase 3 (requires Backend Phase 3 plugin system)
- Phase 3 → Phase 4 (requires Backend Phase 3 resource/workspace APIs)
- Phase 4 → Phase 5 (polish after all features are working)
- [P] tasks within phases can run in parallel between frontend/backend developers

## Verification (after all phases)

- [ ] `pnpm run build` with zero TypeScript errors
- [ ] `cargo check -p nebula-desktop`
- [ ] `pnpm run test` — Vitest tests pass
- [ ] `cargo test -p nebula-desktop`
- [ ] Zero warnings in CI for both TypeScript and Rust
