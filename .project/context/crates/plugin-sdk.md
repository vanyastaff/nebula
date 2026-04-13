# nebula-plugin-sdk
Sole crate for plugin authors and wire protocol. `nebula-plugin-protocol` merged in (slice 1b cleanup, 2026-04-13).

## Invariants
- Plugin authors depend only here. Host (`nebula-sandbox`) imports `nebula_plugin_sdk::protocol::*` for envelope (de)serialization.
- `protocol` submodule: tagged envelope types + `DUPLEX_PROTOCOL_VERSION = 2`. Flat variants, line-delimited JSON. No gRPC/TLS/protobuf.
- `PluginHandler` async trait: `metadata()` + `execute(ctx, action_key, input)`.
- `run_duplex(handler)` owns the event loop. Stdio in 1a/1b; UDS/Named-Pipe in slice 1c. Exits on EOF or `Shutdown`.
- `PluginCtx` is the sole capability surface. 1a: placeholder. 1d+: `.network()`, `.credentials()`, `.log()` → `RpcCall`.

## Key Decisions
- Thin façade hides envelope types. Transport migrations transparent to plugin code.
- Sequential dispatch in 1a/1b. Concurrent multiplexed dispatch lands 1c.
- Malformed input non-fatal — skip and continue.
- Test fixture `src/bin/echo_fixture.rs` in-crate for `env!("CARGO_BIN_EXE_...")` reach.
- One crate instead of `plugin-protocol` + `plugin-sdk` split — artificial split added ceremony for no benefit.

## Traps
- `PluginCtx::new()` private — prevents tests constructing unwired contexts.
- `Cancel` / `RpcResponseOk` / `RpcResponseError` silently ignored in 1a/1b. Slice 1c routes them to pending-call tables.
- `Shutdown` → immediate return. In-flight NOT awaited. Slice 1c adds drain window.
- Sequential dispatch = head-of-line blocking until 1c.
- `write_line` flushes every envelope — correctness requirement.

## Relations
- Deps: `async-trait`, `serde`, `serde_json`, `tokio`, `thiserror`, `tracing`. Zero new transitive crates.
- Consumers: community plugin binaries + `nebula-sandbox` (`protocol` module).

<!-- reviewed: 2026-04-13 — plugin-protocol merged in -->
