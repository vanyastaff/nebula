# temporalio-sdk — Structure Summary

## Crate count: 6

| Crate | Description |
|-------|-------------|
| temporalio-sdk-core | Core gRPC state-machine engine; powers all other language SDKs |
| temporalio-sdk | The Rust user-facing SDK built on sdk-core |
| temporalio-client | gRPC client to Temporal server (workflow CRUD, schedules, activities) |
| temporalio-common | Shared types, proto definitions, data converters, telemetry primitives |
| temporalio-macros | Proc-macro crate (#[workflow], #[workflow_methods], #[activities], #[fsm]) |
| temporalio-sdk-core-c-bridge | C FFI bindings to sdk-core (used by TypeScript, Python, .NET, Ruby SDKs) |

## Layers (bottom to top)

```
temporalio-common (protos, FSM trait, data converters, telemetry)
   └── temporalio-client (gRPC to server, retry, TLS, schedules)
       └── temporalio-sdk-core (workflow state machines, task poller, worker)
           └── temporalio-sdk (user API: workflows, activities, macros)
               └── temporalio-macros (proc macros used across sdk + sdk-core)
temporalio-sdk-core-c-bridge (FFI layer for non-Rust SDKs)
```

## LOC (Rust source, non-test, non-target)

Approximate counts from `wc -l` on .rs files:
- sdk-core: ~147 files
- sdk: ~47 files  
- client: ~17 files
- common: ~30 files
- Total all crates: 252 .rs files, ~111,063 total lines including comments/blanks

## Key proto files

- `crates/common/protos/local/temporal/sdk/core/` — SDK-internal core API (activation, commands, tasks)
- `crates/common/protos/api_upstream/` — Temporal server API (subtree from temporalio/api)
- `crates/common/protos/api_cloud_upstream/` — Cloud API

## Top-10 external dependencies

1. tokio (async runtime)
2. tonic + prost (gRPC stack)
3. thiserror (typed errors)
4. opentelemetry (metrics, tracing export)
5. backoff + futures-retry (client-side retry)
6. parking_lot (fast synchronization)
7. bon (builder macros)
8. tracing (structured logging)
9. async-trait (trait object support for async)
10. lru (workflow run cache, LRU eviction)

## Test count

- ~80 test files in tests/ directories
- Integration tests in `crates/sdk-core/tests/integ_tests/`
- Unit tests in `crates/sdk-core/src/core_tests/`
- History replay tests from binary fixtures in `crates/sdk-core/tests/histories/`
