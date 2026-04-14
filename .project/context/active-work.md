# Active Work
Updated: 2026-04-14

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`.
- **Sandbox Phase 1** slices 1a + 1b + 1c all landed. Duplex v2 is the only protocol. `ProcessSandbox` speaks v2 long-lived handle over UDS (Unix) / Named Pipes (Windows) with JSON framing — gRPC/tonic/protobuf stack was dropped in favour of UDS+JSON (`b607ac39`). Roadmap: `docs/plans/2026-04-13-sandbox-roadmap.md`. Next: Phase 1 slice 1d (supervisor + broker) per `docs/plans/2026-04-13-sandbox-phase1-broker.md`.

## Blocked
- **engine**: credential DI + Postgres storage.
- **auth**: RFC phase.
- **poll cursor persistence**: runtime storage only `MemoryStorage`. See `docs/plans/2026-04-13-poll-api-v2.md`.

## Next Up
- Sandbox slice 1d (supervisor + broker).
- Credential bugs **B3, B4, B5, B13** remaining. Shipped: B1, B2, B6 (was CRITICAL), B7, B8, B9, B10, B11, B12, B14 — visible in `git log --grep '\[B[0-9]'`.
- CredentialPhase + OwnerId.
- Wire CredentialResolver into ActionContext.
- **ControlAction Phase 2**: 7 concrete control nodes (If/Switch/Router/Filter/NoOp/Stop/Fail) in a downstream crate — blocked on crate-placement decision. Trait + adapter + Drop/Terminate prerequisites landed on `main` as `d77a3a1f` (#247). Plan: `docs/plans/2026-04-13-control-action-plan.md`.
- **ControlAction Phase 3**: full engine wiring for `Terminate` (parallel-branch cancellation, `ExecutionResult::termination_reason` propagation, `determine_final_status` handling). v1 gates downstream edges locally only.

## Recently shipped (this session, 2026-04-13)
- **#247 ControlAction DX trait** — public trait + adapter pattern over `StatelessHandler`, plus correctness fixes `ActionResult::Drop`/`Terminate`, `TerminationCode` newtype, `ActionCategory` metadata, `ExecutionTerminationReason`. 56 new tests, 7 demo fixtures, 5 commits squashed.

## Archived branches
- `archive/refactor-credential-beta` (tag at `c93b51d5`) — parallel attempt at credential v2 rewrite; v2 won, beta preserved as tag. Three salvageable ideas documented (CredentialContext encapsulation, CredentialDescription::builder required key, CacheLayer::exists hit counter) — revive as separate small PRs if wanted.

Recent work in `git log`.
