# nebula-plugin-protocol
Wire protocol for process-isolated plugins. Duplex v2 only — legacy v1 removed in slice 1b (2026-04-13).

## Invariants
- Plugin authors use `nebula-plugin-sdk`, not this crate directly. Host (`nebula-sandbox`) imports `duplex` module.
- **Duplex v2**: bidirectional line-delimited JSON, one envelope per `\n`. `DUPLEX_PROTOCOL_VERSION = 2`. Tagged by `kind`. Flat variants.
- Every envelope carries a `u64` correlation `id`.

## Key Decisions
- Slice 1a shipped duplex JSON **zero new deps** (serde + serde_json). Slice 1b migrated ProcessSandbox + deleted v1. Slice 1c adds `prost` (protobuf); slice 1d adds `tonic` + `rustls` + `rcgen` (gRPC over UDS/TCP).
- Flat enum variants — untagged flattening is fragile.
- String error codes, no `ErrorCode` enum dep.

## Traps
- JSON framing is line-delimited — serialized envelopes must never contain raw `\n`. `serde_json::to_string` is single-line by default; `single_line_serialization` test locks this.
- Parser is permissive: malformed lines logged and skipped. Host/plugin stay in sync.
- `Cancel` / `RpcResponseOk` / `RpcResponseError` ignored in slice 1b. Slice 1d routes to pending-call tables.
- Protocol version matching is compile-time only; runtime handshake lands slice 1d.

## Relations
- Consumed by `nebula-plugin-sdk` (plugin authors) and `nebula-sandbox::ProcessSandbox` (host).
- No internal deps.

<!-- reviewed: 2026-04-13 — slice 1b: v1 removed -->
