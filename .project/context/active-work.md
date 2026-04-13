# Active Work
Updated: 2026-04-13

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`.
- **Sandbox Phase 1** slices 1a+1b landed. Duplex v2 is the only protocol. `ProcessSandbox` speaks v2 one-shot. Legacy v1 deleted. Slice 1c next (protobuf framing). Roadmap: `docs/plans/2026-04-13-sandbox-roadmap.md`.

## Blocked
- **engine**: credential DI + Postgres storage.
- **auth**: RFC phase.
- **poll cursor persistence**: runtime storage only `MemoryStorage`. See `docs/plans/2026-04-13-poll-api-v2.md`.

## Next Up
- Sandbox slice 1c (protobuf framing) or slice 1d (gRPC + supervisor).
- Credential bugs B1-B9 (B6 CRITICAL).
- CredentialPhase + OwnerId.
- Wire CredentialResolver into ActionContext.

Recent work in `git log`.
