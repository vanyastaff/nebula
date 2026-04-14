# nebula-sandbox
Plugin isolation — `SandboxRunner` trait + implementations.

## Invariants
- `SandboxRunner` is the common interface.
- `InProcessSandbox` — trusted in-process. Built-in actions.
- `ProcessSandbox` — long-lived plugin over UDS/Named-Pipe (slice 1c). First call spawns + dials socket the plugin announces via stdout handshake; subsequent calls reuse `PluginHandle` via `Mutex<Option<_>>`. Connection error → clear + retry once. Supervisor + concurrent dispatch + reattach slice 1d.
- Phase 0: `execute_stateless` routes `CapabilityGated`/`Isolated` through `self.sandbox`. Stateful still fail-closes.
- **Permission manifest deferred** (roadmap §D4). `plugin.toml` = 9 lines. Defense = process isolation + broker + anti-SSRF + audit + OS jail + signed manifest.

## Key Decisions
- Process isolation over WASM — tokio/reqwest don't compile to WASM.
- Phase 1 transport: **UDS (Unix) / Named Pipe (Windows) + line-delimited JSON**. No gRPC, no TLS. Rust-only plugin constraint means no cross-language interop; tonic+prost+rustls+rcgen rejected (~65 transitive crates). Prior art: LSP/DAP.
- `call_action` for execution, `get_metadata` for discovery.
- OS enforcement — Phase 2/3.

## Traps
- `ProcessSandbox` spawns per call — no pooling. Slice 1d adds long-lived per `(ActionKey, credential_scope)` + Reattach.
- Only `ActionResultOk`/`ActionResultError` valid as response to `ActionInvoke`; other envelope kinds → fatal. `Log`/`RpcCall` from plugin discarded in one-shot read.
- `DUPLEX_PROTOCOL_VERSION` match is compile-time only; runtime handshake lands slice 1d.
- Permissions advisory only until Phase 2.

## Relations
- Depends on `nebula-action`, `nebula-plugin-protocol::duplex`. Used by `nebula-runtime` (re-export), `nebula-engine`.

Roadmap: `docs/plans/2026-04-13-sandbox-roadmap.md`. Research: `.project/context/research/sandbox-prior-art.md`.

<!-- reviewed: 2026-04-14 — slice 1c landed: UDS/Named-Pipe long-lived handle + JSON framing; process.rs docstring cleanup for rustdoc -->

<!-- reviewed: 2026-04-14 -->
