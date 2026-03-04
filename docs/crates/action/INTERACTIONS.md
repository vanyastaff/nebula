# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates (action depends on core, parameter, credential, resource)

- **Upstream:** nebula-core (ids, InterfaceVersion), nebula-parameter (ParameterCollection in ActionMetadata), nebula-credential (CredentialRef in ActionComponents), nebula-resource (ResourceRef in ActionComponents); serde, thiserror, tokio, async-trait, chrono, parking_lot, uuid, hmac, sha2, hex, tokio-util.
- **Downstream (depend on nebula-action):** engine, runtime, execution, plugin, ports, sdk, drivers/sandbox-inprocess — engine/runtime execute actions; plugin implements and registers; ports/sdk expose action contract; sandbox enforces capability boundary.

## In-crate structure

- DX/authoring (optional `dx` or `authoring` module in nebula-action):
  - optional authoring helpers (specialized traits/macros/builders)
  - core protocol and DX live in one crate.

## Downstream Consumers

- `runtime`/`engine` consume `ActionResult` and `ActionOutput` semantics.
- UI/editor consumes metadata and port model for graph building.
- plugin/sdk consumers rely on stable action contract APIs.

## Upstream Dependencies

- `nebula-core`: interface version compatibility rules.
- `nebula-parameter`: parameter definition model.
- `nebula-credential`/`nebula-resource`: typed dependency refs.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| action <-> core | in | ids + interface version semantics | sync | fail on incompatible version | compatibility-critical |
| action <-> parameter | out | parameter declaration in metadata | sync | validation error mapping in runtime | pre-execution path |
| action <-> credential | out | dependency declaration via `CredentialRef` | sync declaration, async resolve in runtime | missing/denied -> fatal/sandbox violation | no direct storage logic |
| action <-> resource | out | dependency declaration via `ResourceRef` | sync declaration, async acquire in runtime | unavailable -> runtime mapping | no direct pooling logic |
| action <-> sandbox | both | capability-enforced context access | async execution | `SandboxViolation` on deny | deterministic policy boundary |
| action <-> runtime/engine | out | execution protocol (`ActionResult`, `ActionOutput`, `ActionError`) | async | retry/degrade handled by runtime/resilience | core integration path |
| action <-> resilience | out | retryability hints/errors | async orchestration | resilience policy decides retries | action provides signal, not policy |
| action <-> api/nodes endpoint | out | `ActionMetadata` for `GET /nodes` + `GET /nodes/:type` | sync read | N/A | API reads metadata + port schema; action never called by api |

## Runtime Sequence

1. Runtime validates params and capability envelope.
2. Runtime resolves `ActionComponents` — acquires credentials and resources declared by the action.
3. Runtime builds context and passes it to the action execute method.
   - **Current/stable**: `ActionContext` (for StatelessAction / StatefulAction / ResourceAction) and `TriggerContext` (for TriggerAction) — concrete structs composed of capability modules (`ResourceAccessor`, `CredentialAccessor`, `ActionLogger`, `TriggerScheduler`, `ExecutionEmitter`, etc.).
4. Action executes and returns `ActionResult<ActionOutput<T>>` or `ActionError`. **Cancellation:** The runtime must race `action.execute(...)` against `ctx.cancellation().cancelled()` (e.g. `tokio::select!`). When cancellation wins, the runtime returns `ActionError::Cancelled`; action implementations do not need to check cancellation in every node.
5. Engine applies flow-control semantics: `ActionResult::Success` → pass output; `Branch` activates path; `Wait` persists state and suspends; `Continue` re-enqueues (stateful); `Break` finalizes; `Retry` reschedules; `ActionError::Retryable` / `Fatal` drive resilience policy.
6. Runtime resolves deferred/streaming outputs before passing to downstream nodes.
7. Resilience layer applies retry/backoff on `ActionError::Retryable` or `ActionResult::Retry`.

## Cross-Crate Ownership

- `action`: contract semantics and stable protocol surface.
- `runtime`/`engine`: orchestration, lifecycle, persistence, scheduling.
- `sandbox`: policy enforcement.
- `resource`/`credential`: operational resolution and lifecycle of dependencies.

## Failure Propagation

- action-level deterministic failures -> `ActionError`.
- policy/capability failures -> `SandboxViolation`.
- retry signals:
  - explicit `ActionResult::Retry`
  - transient `ActionError::Retryable`

## Versioning and Compatibility

- any change to serialized meaning of `ActionResult`/`ActionOutput` is protocol-sensitive.
- breaking-change protocol:
  - major version bump
  - migration doc update
  - compatibility tests with runtime/engine.

## Cross-Crate Ownership

- `action` owns: `Action` trait, metadata/port/component contracts, result/output/error semantics, `Context` base trait
- `runtime`/`engine` own: context implementations, execution ordering, scheduling, state persistence, deferred/streaming resolution
- `sandbox` owns: capability enforcement, `SandboxedContext` proxy (Phase 2)
- `nebula-action` owns DX sub-traits and helper macros in same crate (e.g. `dx` module)
- `resource`/`credential` own: operational lifecycle of dependencies; action only *declares* them via `ActionComponents`

## Contract Tests Needed

- serialization compatibility tests for result/output variants.
- metadata/port compatibility tests across interface versions.
- sandbox violation mapping tests in runtime adapters.
- Context contract continuity: `execution_id` / `node_id` / `workflow_id` / `cancellation` remain available in `ActionContext`.
- `ActionComponents` resolution: all declared credentials and resources available before execute is called.
