---
name: nebula-action
role: Action Trait Family + Execution Policy Metadata (Ports & Adapters)
status: frontier
last-reviewed: 2026-04-17
canon-invariants: [L1-3.5, L2-11.2, L2-11.3, L2-13.4, L2-13.5]
related: [nebula-core, nebula-schema, nebula-credential, nebula-resource, nebula-resilience, nebula-sandbox, nebula-plugin]
---

# nebula-action

## Purpose

Workflow nodes need a typed contract between "what this step does" and "how the engine orchestrates it." Without one, every action re-invents credential plumbing, retry folklore, and checkpoint placement — and the engine cannot enforce guarantees across them. `nebula-action` defines that contract: a trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction` and DX specializations) that determines iteration semantics, trigger lifecycle, and graph-scoped resource supply; plus `ActionMetadata` that carries the static descriptor (key, ports, parameters, isolation, category) the engine uses for discovery, validation, and dispatch. Action authors describe what their node does; the engine applies checkpoint, retry, and cancel rules from those contracts.

## Role

**Action Trait Family + Execution Policy Metadata (Ports & Adapters pattern)**. Core types and traits live here; concrete execution environments (in-process, `ProcessSandbox` with capability allowlists and OS-level hardening) are drivers in `nebula-sandbox`. WASM is an explicit non-goal per `docs/PRODUCT_CANON.md` §12.6.

Pattern inspiration: *Ports & Adapters / Hexagonal Architecture* — action authors program to traits; the engine wires driver adapters. Adding a new trait family requires a canon revision (§3.5, §0.2).

## Public API

**Trait family (core)**

- `Action` — base trait: `metadata() -> &ActionMetadata`.
- `StatelessAction` — pure, stateless single-execution. Associated types: `Input`, `Output`.
- `StatefulAction` — iterative with persistent state; `Continue`/`Break` flow control.
- `TriggerAction` — workflow starter (start/stop); lives outside the execution graph.
- `ResourceAction` — graph-level DI; configures/tears down a scoped resource for downstream nodes.

**DX specializations**

- `PaginatedAction` — cursor-driven pagination (DX over `StatefulAction`).
- `BatchAction` — fixed-size chunk processing (DX over `StatefulAction`).
- `WebhookAction` — webhook lifecycle with HMAC verification (DX over `TriggerAction`).
- `PollAction` — periodic polling with deduplication (DX over `TriggerAction`).
- `ControlAction` — flow-control nodes (If, Switch, Router, Filter, NoOp, Stop, Fail).

**Handler dispatch**

- `ActionHandler` — top-level enum dispatcher over all handler variants.
- `StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler` — dyn-safe handler contracts.
- `AgentHandler` — autonomous agent with internal reasoning loop.

**Metadata and policy**

- `ActionMetadata` — key, name, description, version (`semver::Version`), input/output ports, `ValidSchema` parameters, `IsolationLevel`, `ActionCategory`.
- `ActionCategory` — `Data`, `Control`, `Trigger`, `Resource`, `Agent`, `Terminal`; UI/validator grouping only, not runtime dispatch.
- `IsolationLevel` — `None`, `CapabilityGated`, `Isolated`; routes to the appropriate sandbox.
- `MetadataCompatibilityError` — typed errors for version-compatibility validation.

**Result and output**

- `ActionResult` — execution result carrying data and flow-control intent.
- `ActionOutput` — first-class output type: inline value, binary blob, stream, reference.
- `ActionError`, `RetryHintCode` — typed error distinguishing retryable from fatal.

**Context and capabilities**

- `Context`, `ActionContext`, `TriggerContext` — execution context traits.
- `ResourceAccessor`, `ActionLogger`, `ExecutionEmitter`, `TriggerHealth`, `TriggerScheduler` — injected capabilities.
- `CredentialContextExt` — credential resolution from context.
- `ActionDependencies` — declarative dependency declaration per action type.

**Ports**

- `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`, `ConnectionFilter`, `FlowKind` — port definitions for validation and UI.

**Testing utilities**

- `TestContextBuilder`, `StatefulTestHarness`, `TriggerTestHarness`, `SpyEmitter`, `SpyLogger`, `SpyScheduler`.

**Macros**

- `#[derive(Action)]` — generates `Action` impl boilerplate.
- `validate_action_package`, `ActionPackageValidationError` — package-level validation.

## Contract

- **[L1-§3.5]** The action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`) is the typed dispatch surface. Adding a new trait requires a canon revision (§0.2). The engine routes by trait, not by `ActionCategory` — that field is metadata for UI and tooling only.
- **[L2-§11.2]** Engine-level node re-execution from an `ActionResult` retry variant requires persisted attempt accounting. Status: `planned` — no persisted `attempts` row exists yet. The `ActionResult::Retry` variant is hidden behind the **`unstable-retry-scheduler`** feature flag (default-off) so default builds do not expose the type. The **canonical retry surface today** is the `nebula-resilience` pipeline an action uses internally for outbound calls. No public variant may describe engine-level retry as a current capability until this row moves to `implemented` (#290). See canon §11.2 status table.
- **[L2-§11.3]** For non-idempotent or risky side effects (payments, writes without natural upsert), action handlers must guard execution with the engine idempotency key path before calling the remote system. See `crates/execution/src/idempotency.rs`.
- **[L2-§13.4]** For `TriggerAction`-backed workflow starts, tests must cover the declared delivery contract (at-least-once): no silent drop, and duplicate delivery is handled via stable event identity and dedup/idempotency. Seam: `TriggerAction::start`, `TriggerEvent`.
- **[L2-§13.5]** For ordinary `StatelessAction` instances that cause irreversible external effects, integration tests must prove single-effect safety under retry/restart pressure. Seam: `StatelessAction::execute` + idempotency key guard.
- **CheckpointPolicy status** — `ActionMetadata` carries `IsolationLevel` and `ActionCategory` but does NOT currently carry a `CheckpointPolicy` field. `docs/INTEGRATION_MODEL.md` and older canon text reference `CheckpointPolicy` as a planned `ActionMetadata` field. Status: `planned` — not yet in the type. Tracked in `docs/MATURITY.md` row for `nebula-action` and noted in `docs/INTEGRATION_MODEL.md` §`nebula-action` status box. Do not document it as a current capability.

## Non-goals

- Not the execution state machine — see `nebula-execution` (`ExecutionStatus`, `ExecutionPlan`, CAS transitions).
- Not a retry pipeline — retry around outbound calls uses `nebula-resilience` internally to an action, not an action framework concern.
- Not the sandbox driver — process isolation, capability enforcement, OS-level hardening are in `nebula-sandbox`.
- Not a schema system — `ActionMetadata.parameters` holds a `ValidSchema` from `nebula-schema`; field definitions and validation rules live there.
- Not WASM — see canon §12.6.

## Maturity

See `docs/MATURITY.md` row for `nebula-action`.

- API stability: `frontier` — trait family, metadata, result/output types, and DX specializations are actively used by engine and plugin-sdk; `ActionHandler` dispatch is the evolving integration point.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` enforced.
- `CheckpointPolicy`: `planned` — not in `ActionMetadata` yet; engine does not consume it end-to-end.
- Engine-level retry from `ActionResult` variant: `planned` — the `Retry` variant is gated behind the `unstable-retry-scheduler` feature (default-off); see §11.2 debt note above.

## Feature flags

- `unstable-retry-scheduler` (default-off) — exposes the `ActionResult::Retry` variant reserved for the future engine retry scheduler. Enabling the flag does **not** install a scheduler; it only un-hides the type so the crate can be inspected by consumers who are preparing to integrate the feature once it lands. The engine mirrors the flag (`nebula-engine/unstable-retry-scheduler`) and routes `Retry` through a synthetic failure path. Per canon §11.2 / §4.5, do **not** enable this flag in production.
- DX specializations (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) are implemented and tested; cross-action-type integration tests: partial.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (action trait family; adding a trait = canon revision), §11.2 (retry debt), §11.3 (idempotency), §12.6 (WASM non-goal), §13.4 (trigger delivery), §13.5 (non-idempotent side effects).
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-action` (including `CheckpointPolicy` status note).
- Sandbox: `crates/sandbox/README.md` — `ProcessSandbox`, capability allowlists, OS-level hardening.
- Siblings: `nebula-schema` (`ValidSchema` for `ActionMetadata.parameters`), `nebula-credential` (`CredentialGuard` via `CredentialContextExt`), `nebula-resource` (`ResourceAction`, `ResourceAccessor`), `nebula-resilience` (retry/timeout/circuit-breaker inside actions).
