# Active Work
Updated: 2026-04-13

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`.
- **Sandbox Phase 1** slice 1a landed (duplex + SDK + echo + 12 tests). Slice 1b next (rewrite `ProcessSandbox` to v2). Roadmap: `docs/plans/2026-04-13-sandbox-roadmap.md`.

## Blocked
- **engine**: credential DI + Postgres storage.
- **auth**: RFC phase.
- **poll cursor persistence**: runtime storage only `MemoryStorage`. See `docs/plans/2026-04-13-poll-api-v2.md`.

## Next Up
- Sandbox slice 1b.
- Credential bugs B1-B9 (B6 CRITICAL).
- CredentialPhase + OwnerId.
- Wire CredentialResolver into ActionContext.

Recent work in `git log`.
