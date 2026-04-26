# Raftoral — Structure Summary

## Crate count: 2

- `raftoral` (root) — Core library: Raft consensus, workflow runtime, management runtime, gRPC/HTTP servers, RocksDB storage, WASM bindings. Edition 2024.
- `raftoral-client` — Thin gRPC client SDK for sidecar mode; applications using the sidecar talk to raftoral via this crate.

## Source layout (16,243 total LOC across 49 .rs files)

| Module | Purpose |
|--------|---------|
| `src/raft/generic/` | Reusable Raft infrastructure: `RaftNode`, `StateMachine` trait, `EventBus`, `Transport` trait, `ProposalRouter`, `ClusterRouter`, `RocksDBStorage` |
| `src/workflow/` | Workflow-specific: `WorkflowStateMachine`, `WorkflowRuntime`, `WorkflowRegistry`, `WorkflowContext`, `ReplicatedVar` (checkpoints) |
| `src/management/` | Management cluster: topology tracking, `ManagementStateMachine`, `ManagementRuntime`, `SubClusterRuntime` trait |
| `src/grpc/` | gRPC server/client, Protobuf generated code (`raftoral.proto`) |
| `src/http/` | HTTP/REST alternative transport (CORS-friendly, WASM-compatible HTTP client) |
| `src/sidecar/` | Streaming gRPC server for polyglot sidecar mode (`sidecar.proto`) |
| `src/full_node/` | Convenience compositor — wires all layers 0-7 into a single `FullNode<R>` |
| `src/wasm/` | `wasm-bindgen` entry points for JavaScript environments |
| `src/workflow_proxy_runtime.rs` | Proxy runtime for sidecar deployments — delegates workflow execution to external apps |
| `src/kv/` | Example KV store runtime (demo/testing) |

## Key dependencies

- `raft = "0.7"` (forked tikv/raft-rs — custom prost/rand pins)
- `tokio = "1.47"` (full features, native; sync/macros/time for WASM)
- `rocksdb = "0.24"` (optional `persistent-storage` feature, default on)
- `tonic = "0.14"` + `axum = "0.8"` (gRPC + HTTP servers)
- `serde` + `serde_json` (all serialization)
- `wasm-bindgen`, `web-sys` (WASM target)

## Test count: 104 unit/integration tests

## Notable: no separate test crate; tests inline in source modules.
