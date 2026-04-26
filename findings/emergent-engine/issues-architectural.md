# Issues — emergent-engine

Total issues: ~37 (13 open + 24 closed). Under the 100-closed-issue threshold for mandatory citation, but cited here for completeness.

## Architecturally Significant Open Issues

| # | Title | Architectural Significance |
|---|-------|---------------------------|
| [#25](https://github.com/govcraft/emergent/issues/25) | feat(engine): OS-level sandboxing for primitive processes | Confirms there is currently NO sandbox for primitives — any process can do anything |
| [#24](https://github.com/govcraft/emergent/issues/24) | feat(engine): authenticate primitive connections by spawned PID | No auth on IPC today — any process that knows the socket path can connect |
| [#23](https://github.com/govcraft/emergent/issues/23) | feat(engine): enforce pub/sub declarations at the broker level | Currently declarations in TOML are informational only, not enforced at runtime |
| [#28](https://github.com/govcraft/emergent/issues/28) | feat(engine): standardized tool request/response message protocol | Protocol for tool/LLM calls via pub-sub being designed |
| [#27](https://github.com/govcraft/emergent/issues/27) | feat(engine): support streaming/chunked message delivery | Streaming LLM output not yet in engine; pull-based stream added in SDK but not engine |
| [#26](https://github.com/govcraft/emergent/issues/26) | feat(engine): support stdin/stdout IPC transport for primitives | Current transport is Unix sockets only; stdin/stdout would allow simpler tooling |

## Architecturally Significant Closed Issues

| # | Title | Significance |
|---|-------|-------------|
| [#33](https://github.com/govcraft/emergent/issues/33) | bug(ipc): fast-interval exec-sources silently stop publishing after ~2 minutes | IPC back-pressure gap — not resolved by backpressure, resolved by a heartbeat fix |
| [#13](https://github.com/govcraft/emergent/issues/13) | Sink exits with status 143 on graceful shutdown | Three-phase shutdown had races; resolved |
| [#8](https://github.com/govcraft/emergent/issues/8) | fix(engine): stale socket file blocks startup after unclean shutdown | Single-node hard restart limitation |
| [#35](https://github.com/govcraft/emergent/issues/35) | Game of Life example — works handler doesn't seem to receive the seed | Pub-sub subscription timing race condition in rapid-start scenarios |

## Summary of Themes

1. **No security model yet**: Issues #24, #25 explicitly call out missing PID-based auth and OS-level sandboxing for primitives. Any process that discovers the socket path can connect. This is a known gap being tracked.
2. **Pub/sub timing races**: Multiple issues (#35, #13) trace back to subscription readiness before producers start — the 50ms sleep workaround in process_manager.rs:200 is a band-aid acknowledged by the maintainer.
3. **Streaming / LLM support**: Issues #27, #28 indicate the maintainer is building toward a first-class tool-call / streaming protocol, signaling awareness that exec-handler-via-shell-pipe is not the end state.
4. **No credential management tracked in issues**: No issues mention secret/credential management — confirms it is intentionally outside scope (env vars are the mechanism).
