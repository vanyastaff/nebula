# nebula-plugin-sdk
Author-facing SDK for community plugins. Thin wrapper around `nebula-plugin-protocol::duplex`.

## Invariants
- Plugin authors depend only on this crate. Never see `tonic`/`prost`/`rustls`.
- `PluginHandler` async trait: `metadata()` + `execute(ctx, action_key, input)`.
- `run_duplex(handler)` owns the event loop: stdin → dispatch → stdout. Exits on EOF or `Shutdown`.
- `PluginCtx` is the **sole** capability surface. Slice 1a: placeholder. Slice 1d+: `.network()`, `.credentials()`, `.log()` → `RpcCall` envelopes.
- Plugin metadata declared via `PluginMeta::new(key, version).with_action(...)`. Slice 1e wires derive-macros to generate automatically.

## Key Decisions
- Thin façade hides envelope types. Migrations to concurrency / protobuf / gRPC are transparent to plugin code.
- **Sequential dispatch** in slice 1a — one action at a time. Multiplexed concurrent dispatch lands slice 1b.
- Malformed input non-fatal — skip and continue. Resilient across migrations.
- Test fixture (`src/bin/echo_fixture.rs`) lives inside the crate for `env!("CARGO_BIN_EXE_...")` reach from integration tests. Not an `examples/` example.

## Traps
- `PluginCtx::new()` private — prevents tests from constructing unwired contexts meaningless once slice 1d lands.
- `Cancel` / `RpcResponseOk` / `RpcResponseError` silently ignored in slice 1a. Tests lock this. Slice 1b must route them to pending-call tables.
- `Shutdown` → immediate `return Ok(())`. In-flight actions NOT awaited. Slice 1b needs drain window.
- Sequential dispatch = head-of-line blocking. Acceptable for slice 1a.
- `write_line` flushes every envelope — correctness requirement, I/O-bound for high-freq logs.

## Relations
- Depends on `nebula-plugin-protocol` + pre-existing workspace deps. Zero new transitive crates.
- Primary consumer: community plugin binaries. Host consumer (`PluginSupervisor`) lands slice 1b.

<!-- reviewed: 2026-04-13 — created for Phase 1 slice 1a -->
