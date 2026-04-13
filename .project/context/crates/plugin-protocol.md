# nebula-plugin-protocol
Wire protocol for process-isolated plugins. Two versions coexist during Phase 1 migration.

## Invariants
- Plugin authors depend only on this crate. No `nebula-action` / `nebula-core` leakage.
- **v1 (one-shot, legacy)**: `PluginRequest` in, `PluginResponse` out, exit. Used by current `ProcessSandbox`. Deleted in slice 1b.
- **v2 (duplex, `duplex` module)**: bidirectional line-delimited JSON, one envelope per `\n`. `DUPLEX_PROTOCOL_VERSION = 2`. Tagged by `kind`. Flat variants (`ActionResultOk` / `ActionResultError`), no nested flattening.

## Key Decisions
- Slice 1a shipped duplex JSON with **zero new deps** — only pre-existing `serde` + `serde_json`. Slice 1b adds `prost` for protobuf, 1c adds `tonic` + `rustls` + `rcgen` for gRPC over UDS/TCP.
- Flat enum variants — serde untagged flattening is fragile, flat variants round-trip cleanly.
- String error codes; no `ErrorCode` enum dep.

## Traps
- v1 and v2 are **incompatible on the wire**. Only `ProcessSandbox` (v1) is currently wired.
- v1 `run()` panics on broken stdio — intentional.
- v2 parser is permissive: malformed lines logged via `tracing::warn!` and skipped.
- v2 ignores `Cancel` / `RpcResponseOk` / `RpcResponseError` in slice 1a. Slice 1b must route them to pending-call tables or they leak.
- Protocol version matching happens at spawn handshake (slice 1d), not runtime.

## Relations
- Used by community plugin binaries via `nebula-plugin-sdk`.
- `nebula-sandbox::ProcessSandbox` consumes v1 today. v2 host consumer lands slice 1b.
- No internal deps.

<!-- reviewed: 2026-04-13 — Phase 1 slice 1a -->
