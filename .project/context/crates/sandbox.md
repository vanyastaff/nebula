# nebula-sandbox
Plugin isolation — `SandboxRunner` trait + implementations.

## Invariants
- `SandboxRunner` is the common interface for all action execution.
- `InProcessSandbox` — trusted, in-process. Built-in actions.
- `ProcessSandbox` — child process, legacy stdio JSON (v1). Rewrite to duplex v2 + `PluginSupervisor` lands Phase 1 slice 1b.
- Phase 0 (2026-04-13): `execute_stateless` routes `CapabilityGated`/`Isolated` through `self.sandbox.execute()`. Stateful still fail-closes — needs Phase 1 broker loop.
- **Permission manifest deferred** (roadmap §D4). `plugin.toml` final form = 9 lines (identity + signing). Defense = process isolation + broker RPC + anti-SSRF + audit log + OS jail + signed manifest.

## Key Decisions
- **Process isolation over WASM** — tokio/reqwest/teloxide don't compile to WASM.
- Phase 1 target transport: gRPC over UDS/TCP-loopback + AutoMTLS + Reattach (go-plugin pattern). Slice 1a ships duplex line-delimited JSON as stepping stone — zero new deps.
- Response shape: `ActionResultOk { output }` or `ActionResultError { code, message, retryable }` (duplex v2). Flat variants, no untagged flattening.
- OS enforcement (seccomp/landlock/cgroups on Linux, sandbox-exec+disclaim on macOS, AppContainer+WFP on Windows) — Phase 2/3.

## Traps
- `ProcessSandbox` spawns a new process per call — no pooling. Phase 1 `PluginSupervisor` adds long-lived per `(ActionKey, credential_scope)` + Reattach.
- Legacy v1 `PluginResponse` uses `#[serde(tag)]` — variant order matters.
- Permissions are advisory only — OS enforcement lands Phase 2+.

## Relations
- Depends on `nebula-action`. Used by `nebula-runtime` (re-export), `nebula-engine` (via runtime).
- Slice 1b wires `PluginSupervisor` using `nebula-plugin-protocol::duplex`.

Roadmap: `docs/plans/2026-04-13-sandbox-roadmap.md`. Research: `.project/context/research/sandbox-prior-art.md`.

<!-- reviewed: 2026-04-13 — Phase 1 slice 1a: duplex + plugin-sdk landed -->
