# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: ids, interface versioning primitives.
- `parameter`: parameter schema declarations consumed via metadata.
- `credential`: typed credential refs in action dependency declarations.
- `resource`: typed resource refs in action dependency declarations.
- `runtime` / `engine`: orchestration and execution lifecycle.
- `sandbox`: capability enforcement boundary for action execution.
- `resilience`: retry/backoff/circuit decisions using action signals.
- `log` / `metrics` / `telemetry`: observability around action execution and failures.
- `workflow`: graph compilation and node contract compatibility.
- `api` / `cli` / `ui`: control plane surfaces using metadata/contracts.

## Planned crates

- `action-dx` (proposed):
  - optional authoring helpers (specialized traits/macros/builders)
  - keeps `nebula-action` as protocol core.

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
   - **Current**: `NodeContext` (doc-hidden temporary placeholder) carrying `execution_id`, `node_id`, `workflow_id`, `cancellation`.
   - **Target**: `ActionContext` (for StatelessAction / StatefulAction / ResourceAction) and `TriggerContext` (for TriggerAction) — concrete structs composed of capability modules (`ResourceAccessor`, `CredentialAccessor`, etc.). Designed for composition: new capabilities add fields without breaking existing signatures.
4. Action executes and returns `ActionResult<ActionOutput<T>>` or `ActionError`.
5. Engine applies flow-control semantics: `Branch` activates path, `Wait` persists state and suspends, `Continue` re-enqueues, `Retry` reschedules.
6. Runtime resolves deferred/streaming outputs before passing to downstream nodes.
7. Resilience layer applies retry/backoff policy on `ActionError::Retryable` or `ActionResult::Retry`.

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
- `nebula-action-dx` (future): DX sub-traits (StatefulAction, TriggerAction, etc.) and helper macros — keeps `nebula-action` as lean protocol core
- `resource`/`credential` own: operational lifecycle of dependencies; action only *declares* them via `ActionComponents`

## Contract Tests Needed

- serialization compatibility tests for result/output variants.
- metadata/port compatibility tests across interface versions.
- sandbox violation mapping tests in runtime adapters.
- `NodeContext` → stable context migration: same execution_id/node_id/workflow_id available post-Phase 2.
- `ActionComponents` resolution: all declared credentials and resources available before execute is called.
