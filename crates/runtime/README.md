# nebula-runtime

Action execution orchestration for the Nebula workflow engine.

**Layer:** Exec
**Canon:** §3.5 (action dispatch by trait), §11.2 (retry surface for actions), §12.6 (isolation honesty)

## Status

**Overall:** `implemented` for the default in-process execution path with data-passing limits and a task queue.

**Works today:**

- `ActionRuntime` — executes actions through the sandbox with data limits
- `ActionRegistry` — registers and looks up action handlers by key
- `DataPassingPolicy` / `LargeDataStrategy` — enforces output size limits between nodes
- `MemoryQueue` + `TaskQueue` — in-memory task queueing
- `BlobRef` + `BlobStorage` — side-channel for large payloads that shouldn't thread through the queue
- `StatefulCheckpoint` + `StatefulCheckpointSink` — checkpoint boundaries for `StatefulAction` types
- `BoundedStreamBuffer` + `PushOutcome` — backpressure-aware streaming between nodes
- Sandbox delegation to `nebula-sandbox` via `SandboxRunner` / `ActionExecutor` / `InProcessSandbox`
- 6 unit test markers, 1 integration test binary

**Known gaps / deferred:**

- **Engine-level retry from `ActionResult::Retry`** — `planned` per canon §11.2. Runtime dispatches actions; it does not schedule re-execution across persisted attempts.
- **Durable queue** — `MemoryQueue` is in-memory only. Persistent queueing is not in scope; durable control signals live in `execution_control_queue` (canon §12.2), not here.
- **4 panic sites** in runtime dispatch paths — review for whether they should be typed `RuntimeError` variants instead.

## Architecture notes

**Smells tracked as open debt:**

- **Module `blob.rs` sits awkwardly between runtime and storage** — `BlobStorage` is a trait defined here. If blobs become a first-class side channel (large-payload path), their storage contract may belong in `nebula-storage` with a trait re-export here. Flag when blob usage grows.
- **Module `queue.rs`** — `MemoryQueue` is the only implementation today. If/when a durable queue is added, consider whether it belongs here or in `nebula-storage`. Canon §12.2 control queue is separate from this task queue by design.

**Not smells — intentional:**

- Runtime depending on `nebula-action`, `nebula-sandbox`, `nebula-metrics`, `nebula-telemetry` is correct layering — the runtime is where action traits meet the execution plane.

## Scope

The runtime sits between the engine (which schedules work level-by-level) and the sandbox (which provides isolation). It resolves actions from the registry, enforces data-passing policies, emits telemetry events, and delegates to the sandbox for the actual call. It does **not** orchestrate the DAG — that is `nebula-engine`.

## What this crate provides

| Type / module | Role |
| --- | --- |
| `ActionRuntime` | Executes a resolved action through the sandbox with data limits. |
| `ActionRegistry` | Registers and looks up action handlers by key. |
| `DataPassingPolicy`, `LargeDataStrategy` | Enforces output size policy between nodes. |
| `MemoryQueue`, `TaskQueue`, `QueueError` | In-memory task queueing for local execution. |
| `BlobRef`, `BlobStorage` | Side-channel for large payloads. |
| `StatefulCheckpoint`, `StatefulCheckpointSink` | Checkpoint sink for `StatefulAction` types. |
| `BoundedStreamBuffer`, `PushOutcome` | Streaming with backpressure. |
| `RuntimeError` | Typed error for the runtime layer. |
| `ActionExecutor`, `InProcessSandbox`, `SandboxRunner`, `SandboxedContext` | Re-exported from `nebula-sandbox` directly via `pub use nebula_sandbox::...` in `lib.rs`. |

## Where the contract lives

- Source: `src/lib.rs`, `src/runtime.rs`, `src/registry.rs`, `src/queue.rs`, `src/data_policy.rs`
- Canon: `docs/PRODUCT_CANON.md` §3.5, §11.2, §12.6
- Glossary: `docs/GLOSSARY.md` §3 (action model), §4 (sandbox/resource)

## See also

- `nebula-engine` — drives this crate
- `nebula-sandbox` — isolation primitives this crate delegates to
- `nebula-action` — action trait family this crate dispatches
