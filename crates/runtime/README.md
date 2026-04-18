---
name: nebula-runtime
role: Action Dispatcher (Scheduler Control Plane)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L2-3.5, L2-11.2, L2-12.6]
related: [nebula-engine, nebula-sandbox, nebula-action, nebula-storage, nebula-resilience]
---

# nebula-runtime

## Purpose

The engine schedules work level-by-level but does not directly invoke actions — it delegates to
a runtime that knows how to resolve handlers from a registry, enforce data-passing limits,
emit telemetry, and call into the appropriate sandbox. Without this layer, the engine would
need to embed action dispatch, data policy, and sandbox selection logic, violating the single-
responsibility split canon §12.1 requires. `nebula-runtime` is that layer: it sits between the
engine (which schedules work) and the sandbox (which provides isolation), and owns the path from
"resolved action key + inputs" to "output or error."

## Role

*Action Dispatcher.* Sits between `nebula-engine` (scheduler) and `nebula-sandbox` (isolation)
and owns the per-action dispatch path: registry lookup, data-passing policy enforcement,
telemetry emission, and checkpoint sink for stateful actions.

## Public API

- `ActionRuntime` — executes a resolved action through the sandbox with data limits.
- `ActionRegistry` — registers and looks up action handlers by key.
- `DataPassingPolicy`, `LargeDataStrategy` — controls output size enforcement between nodes.
- `MemoryQueue`, `TaskQueue`, `QueueError` — in-memory task queueing for local execution.
- `BlobRef`, `BlobStorage` — side-channel trait and reference type for large payloads.
- `StatefulCheckpoint`, `StatefulCheckpointSink` — checkpoint boundaries for `StatefulAction`.
- `BoundedStreamBuffer`, `PushOutcome` — streaming with backpressure between nodes.
- `RuntimeError` — typed error for the runtime layer.

Re-exported from `nebula-sandbox` (via `pub use`):
`ActionExecutor`, `InProcessSandbox`, `SandboxRunner`, `SandboxedContext`.

## Contract

- **[L2-§3.5]** Action dispatch is by trait (`StatelessAction`, `StatefulAction`,
  `TriggerAction`, `ResourceAction`) through `ActionRegistry`. Adding a trait family requires
  canon revision (§0.2).

- **[L2-§11.2]** The runtime dispatches individual action invocations; it does **not** schedule
  engine-level node re-execution from `ActionResult::Retry` with persisted attempt accounting.
  That path is `planned`. The canonical retry surface is `nebula-resilience` pipelines
  composable inside an action.

- **[L2-§12.6]** Isolation is by sandbox delegation — the runtime selects `InProcessSandbox`
  for built-in actions and `ProcessSandbox` for community plugins. It does not implement
  isolation itself; canon §12.6 is the normative statement on what `nebula-sandbox` actually
  provides.

## Non-goals

- Not the engine orchestrator — DAG scheduling and level-by-level parallelism live in
  `nebula-engine`.
- Not the sandbox implementation — see `nebula-sandbox`.
- Not the durable control queue — cancel/dispatch signals live in `execution_control_queue`
  (§12.2) in `nebula-storage`, separate from this crate's `MemoryQueue`.
- Not a retry scheduler — see above (§11.2); canonical path is `nebula-resilience`.

## Maturity

See `docs/MATURITY.md` row for `nebula-runtime`.

- API stability: `partial` — `ActionRuntime` and `ActionRegistry` are in active use;
  known open debts listed in Appendix.
- `MemoryQueue` is in-memory only — not a durable queue; durable control signals live
  in `execution_control_queue` in `nebula-storage`.
- 4 panic sites in dispatch paths — candidates for typed `RuntimeError` variants.
- Engine-level retry from `ActionResult::Retry` is `planned` (§11.2).

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5, §11.2, §12.6.
- Glossary: `docs/GLOSSARY.md` §3 (action model), §4 (sandbox/resource).
- Siblings: `nebula-engine` (drives this crate), `nebula-sandbox` (isolation primitives),
  `nebula-action` (action trait family), `nebula-resilience` (in-action retry).

## Appendix

### Architecture notes

- **`blob.rs` sits between runtime and storage** — `BlobStorage` is a trait defined here.
  If blobs become a first-class side channel, their storage contract may belong in
  `nebula-storage` with a re-export here. Flag when blob usage grows.
- **`MemoryQueue`** — single implementation today. If a durable queue is added, consider
  whether it belongs here or in `nebula-storage`. The canon §12.2 control queue is separate
  from this task queue by design.
- **Sandbox re-exports** — `ActionExecutor`, `SandboxRunner`, `InProcessSandbox`,
  `SandboxedContext` are owned by `nebula-sandbox` and re-exported here via `pub use
  nebula_sandbox::...`. The legacy `nebula-runtime/src/sandbox.rs` shim was deleted in
  commit `eae0b54e` (audit task A3.7, canon §14).
