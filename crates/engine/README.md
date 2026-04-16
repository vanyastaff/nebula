# nebula-engine

Workflow execution orchestrator for the Nebula workflow engine.

**Layer:** Exec
**Canon:** §10 (golden path — orchestrator schedules activated workflows), §11.1 (execution authority via `ExecutionRepo`), §12.2 (durable control plane)

## Status

**Overall:** `implemented` for the default level-by-level DAG execution path, with **known open debts** listed below. The engine is the real consumer of the `execution_control_queue` in production deployment modes — canon §12.2.

**Works today:**

- `WorkflowEngine` — level-by-level DAG execution with bounded concurrency
- `ExecutionResult` — post-run summary flowing back to the API layer
- `EngineCredentialAccessor` / `EngineResourceAccessor` — scoped accessors injected into action contexts
- `ExecutionEvent` — broadcast events via `nebula-eventbus`
- Integration tests (`tests/`) exercise the control-queue cancel path end-to-end
- Re-export of `nebula-plugin` registry types used during dispatch

**Known gaps (tracked as in-source `TODO`s — treat as canon debt):**

| Gap | Location | Canon impact |
| --- | --- | --- |
| `ExecutionBudget` is not persisted in `ExecutionState` — re-read on resume loses the original budget | `src/engine.rs:796` | §11.5 durability matrix: budget is **ephemeral**, not authoritative on restart |
| Original workflow input is not persisted in `ExecutionState` — resume cannot replay from input | `src/engine.rs:809` | §11.5 + §11.2 retry / resume story is narrower than it could be |
| Per-node credential `allowed_keys` is not populated from declared credential dependencies — gate is weaker than the §12.5 promise | `src/engine.rs:1312`, `:1601` | §12.5 secrets boundary partially enforced until this lands |
| Downstream-edge gate only blocks **local** edges, not the full graph | `src/engine.rs:1808` | §10 golden path narrower than advertised for multi-hop conditional flows |
| `ExecutionBudget` is a historical type moved to `nebula-execution` — cleanup pending | `src/engine.rs:20` | documentation / import hygiene |

**Not implemented here (by design):**

- **Retry scheduling** from `ActionResult::Retry` with persisted attempt accounting — canon §11.2 `planned`. Action-level retry lives in `nebula-resilience`.
- **Storage implementation** — engine drives `ExecutionRepo`, does not own it. See `nebula-storage`.
- **Action execution** — delegated to `nebula-runtime`.
- **Plugin loading / isolation** — see `nebula-sandbox`.

## Architecture notes

**Smells tracked as open debt:**

- **Fail-open credential allowlist** (`credential_accessor.rs`): an empty allowlist means **all credentials are permitted** ("open / passthrough mode"). Canon §12.5 implies a fail-closed secrets boundary, but today the default is the opposite until `TODO: populate allowed_keys from node_def's declared credential dependencies` (`src/engine.rs:1312`) is implemented. **Before that lands, per-node credential dependency enforcement is a `false capability` (§4.5).**
- **No resource allowlist at all** (`resource_accessor.rs`): unlike credentials, there is no allowlist for resources — any registered key may be acquired by any action. If the `nebula-resource` model needs scoped access, this bridge is the place to enforce it.
- **Cross-layer bridges in engine.** `credential_accessor.rs` and `resource_accessor.rs` bridge business-layer traits (`CredentialAccessor`, `ResourceAccessor`) into engine concrete types. Architecturally these belong to `nebula-credential` / `nebula-resource` as extension points — engine should depend on a trait, not own the bridge. Move is a candidate refactor when the two gaps above get fixed.
- **Fourteen intra-workspace dependencies** — the engine reaches into most of the workspace (`nebula-core`, `-error`, `-action`, `-expression`, `-plugin`, `-workflow`, `-execution`, `-credential`, `-resource`, `-runtime`, `-resilience`, `-storage`, `-metrics`, `-telemetry`). This is the natural centre of gravity, but every new dep should be questioned against the layer rules in `CLAUDE.md`.

**Not smells — intentional:**

- The engine is the single consumer of `execution_control_queue` (canon §12.2). That is by design and not a violation of its focused scope.
- The event/result modules look similar to types in `nebula-execution` but serve a different role (outbound engine events vs. persistent state); this is intentional, not DRY violation.

## Scope

The engine sits between the user-facing API and the runtime. It builds an execution plan from a workflow graph, resolves node inputs from predecessor outputs, transitions execution state through `ExecutionRepo` (CAS on `version`), and delegates action execution to the runtime. It is the component that canon §12.2 names as the **real consumer** of `execution_control_queue` — a demo handler that logs and discards commands does **not** satisfy the canon.

## What this crate provides

| Type | Role |
| --- | --- |
| `WorkflowEngine` | Entry point — executes workflows level-by-level with bounded concurrency. |
| `ExecutionResult` | Final result of an execution run. |
| `EngineError` | Typed engine-layer error. |
| `ExecutionEvent` | Broadcast event type for the eventbus. |
| `EngineCredentialAccessor` | Scoped credential accessor passed into action contexts. |
| `EngineResourceAccessor` | Scoped resource accessor passed into action contexts. |
| `NodeOutput` | Per-node output threaded between levels. |
| `DEFAULT_EVENT_CHANNEL_CAPACITY` | Default backpressure bound for the event channel. |

## Where the contract lives

- Source: `src/lib.rs`, `src/engine.rs` (orchestrator), `src/credential_accessor.rs`, `src/resource_accessor.rs`
- Integration tests: `tests/` — exercise the control-queue cancel path (canon §13 step 5)
- Canon: `docs/PRODUCT_CANON.md` §10, §11.1, §12.2, §13
- Glossary: `docs/GLOSSARY.md` §2

## See also

- `nebula-execution` — state types the engine drives
- `nebula-storage` — `ExecutionRepo` the engine transitions through
- `nebula-runtime` — action dispatcher the engine delegates to
- `nebula-workflow` — DAG definition the engine builds `ExecutionPlan` from
