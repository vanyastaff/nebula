# nebula-plugin-sdk
Sole crate for plugin authors + wire protocol + transport.

## Invariants
- Plugin authors depend only here. Host imports `nebula_plugin_sdk::protocol::*` (envelope types) and `transport::dial` (host-side connect).
- `protocol` module: tagged envelope types + `DUPLEX_PROTOCOL_VERSION = 2`. Flat variants, line-delimited JSON. No gRPC/TLS/protobuf.
- `transport` module (slice 1c): UDS (0700 dir + 0600 sock) on Unix, Named Pipe (`\\.\pipe\LOCAL\...`) on Windows. Tokio `net` feature, zero new deps.
- `PluginHandler` async trait: `metadata()` + `execute(ctx, action_key, input)`.
- `run_duplex(handler)` binds listener → prints handshake to stdout → accepts one connection → runs event loop over stream. Exits on EOF or `Shutdown`.
- `PluginCtx` is the sole capability surface. 1a: placeholder. 1d+: `.network()`, `.credentials()`, `.log()` → `RpcCall`.

## Key Decisions
- Thin façade hides envelope types. Plugin authors see `PluginHandler` only.
- Sequential dispatch through slice 1c. Concurrent multiplexed dispatch in 1d.
- Auth via OS ACL (UDS mode bits, pipe namespace + DACL) — no TLS.
- One crate instead of `plugin-protocol` + `plugin-sdk` split — removed in slice 1b cleanup.

## Traps
- `PluginCtx::new()` private.
- `Cancel` / `RpcResponseOk` / `RpcResponseError` silently ignored through 1c. Slice 1d routes to pending-call tables.
- `Shutdown` → immediate return. In-flight NOT awaited. Slice 1d adds drain.
- Sequential dispatch = head-of-line blocking until 1d.
- Unix `CleanupGuard` removes `/tmp/nebula-plugin-{pid}/` on drop. Host side has `dir: None` (doesn't own the dir).
- Handshake line must be printed before `accept()` — host reads, dials, plugin accepts. Ordering matters.

## Relations
- Deps: `async-trait`, `serde`, `serde_json`, `tokio` (io-std/io-util/sync/net), `thiserror`, `tracing`.
- Consumers: community plugin binaries + `nebula-sandbox`.

<!-- reviewed: 2026-04-13 — slice 1c: UDS/Named-Pipe transport -->
