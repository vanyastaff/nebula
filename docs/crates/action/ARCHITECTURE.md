# Architecture

## Architectural role

`nebula-action` defines executable node contracts for a Rust workflow platform (n8n-class).
The crate is a protocol, not a runtime. It must be small, stable, and explicit.

## Current architecture (implemented)

1. Action identity and declaration
- `Action` + `ActionMetadata`
- `ActionComponents` with typed dependencies (`CredentialRef`, `ResourceRef`)

2. Control plane
- `ActionResult<T>` defines execution intent
- variants cover success, branching, waiting, retry signaling, and fan-out

3. Data plane
- `ActionOutput<T>` handles synchronous and asynchronous payload forms
- deferred and streaming outputs are first-class

4. Safety and failure semantics
- `ActionError` distinguishes retryable from fatal failures
- sandbox and data-limit violations are explicit variants

5. Graph topology contracts
- typed input/output/support/dynamic port declarations

6. Context placeholder
- `Context` base trait (bare marker)
- `NodeContext` (doc-hidden) — temporary bridge carrying execution_id, node_id, workflow_id, cancellation
- **Design decision:** target context types are `ActionContext` and `TriggerContext`, both concrete structs composed of capability modules (ResourceAccessor, CredentialAccessor, etc.) — see API.md

## Target architecture (production-complete)

1. Stable contract layer (`nebula-action`)
- frozen core traits, result/output/error/port models
- versioned compatibility policy

2. Authoring DX layer (same crate, e.g. `dx` or `authoring` module)
- optional trait families and helper macros for common action patterns
- core contracts stay in root/action modules; DX in optional submodule

3. Runtime adapter layer (`nebula-runtime`)
- context implementation and orchestration
- adapter from runtime state to `Context` and action capabilities

4. Sandbox adapter layer (`nebula-sandbox-*`)
- capability-checked proxies around context operations
- enforce least-privilege access declared in metadata/components

### Planned type hierarchy

Two-level design: **Core types** (engine treats differently; stable contracts) and **DX types** (convenience wrappers; same crate, e.g. `dx` module).

```
Action (base trait — metadata + components)
├── StatelessAction       — ActionContext — pure function: execute(input, &ctx)
├── StatefulAction        — ActionContext — persistent state: execute(input, &mut state, &ctx)
│   └── TriggerAction     — TriggerContext — workflow starter: start(&ctx) / stop(&ctx)
└── ResourceAction        — ActionContext — graph-level DI: configure(&ctx); cleanup(instance, &ctx)

DX types (same crate, e.g. dx module, over core sub-traits):
StatefulAction
├── InteractiveAction     — ActionContext  — Wait { Approval } with declarative UI
└── TransactionalAction   — ActionContext  — Saga: execute_tx() + compensate() + SagaStepKind

TriggerAction
├── WebhookAction         — TriggerContext — register() + handle_request() + verify_signature()
└── PollAction            — TriggerContext — poll(cursor, &ctx) → PollResult; cursor persistence
```

**Context design — composition over inheritance:**

Each sub-trait receives a concrete struct (`ActionContext` or `TriggerContext`), not a trait object. Contexts grow capabilities by adding fields — no trait extension required:

```
ActionContext:                       TriggerContext:
  execution_id, node_id               workflow_id, trigger_id
  workflow_id                         cancellation
  cancellation                        scheduler (next poll)
  resources: ResourceAccessor         emitter (spawn executions)
  credentials: CredentialAccessor     credentials: CredentialAccessor
  logger: ActionLogger                logger: TriggerLogger
```

`TriggerContext` is distinct from `ActionContext` because triggers live **outside** any execution — they are long-lived workflow-scoped processes that *spawn* executions, never inside one.

**Core vs DX distinction** (Rust analogy: `BufReader` is DX over `Read`):
- Engine speaks only to core types — DX reduces boilerplate without adding engine coupling
- `ResourceAction` is core (not DX) because it changes *execution ordering* — engine executes it before downstream nodes and manages scoped lifecycle

**`ResourceAction` vs `ctx.resource()`:**
```
ctx.resource()      → global resource registry (nebula-resource::Manager)
ResourceAction      → graph-level DI: scoped to downstream branch only
```
A `PostgresPool` `ResourceAction` provides a connection pool visible only to its downstream subtree. A `QueryUsers` action then calls `ctx.resource::<DatabasePool>()` to consume it.

### Target structure

```text
crates/action/
├── src/
│   ├── action.rs
│   ├── metadata.rs
│   ├── components.rs
│   ├── context.rs
│   ├── result.rs
│   ├── output.rs
│   ├── error.rs
│   ├── port.rs
│   ├── prelude.rs
│   └── lib.rs
├── docs/                # rustdoc-facing deep docs and how-to
├── examples/            # canonical action authoring patterns
└── tests/               # contract/compat tests (target expansion)
```

## Design invariants

- `ActionResult` decides control flow; `ActionOutput` decides payload form.
- Action contracts are deterministic and serializable for checkpointing/recovery.
- Dependency declarations are static and type-driven where possible.
- Engine-specific behavior must not leak into action traits.

## Extension model

Proposals from archive are kept as staged evolution:
- specialized execution traits (streaming/stateful/trigger/resource patterns)
- advanced orchestration variants (`Fork`, `Join`, `Delegate`) gated for later phases

These will be introduced only with compatibility policy and migration tooling.

## Comparative Analysis

References: n8n, Node-RED, Activepieces/Activeflow, Temporal/Airflow style orchestration.

- Adopt:
  - n8n/Node-RED style explicit node metadata and port contracts for graph tooling.
  - workflow orchestrator style deterministic execution state contracts (`Wait`, retry signals, resumability-friendly outputs).
- Reject:
  - runtime-implicit action behavior with hidden side channels (hard to replay/debug and unsafe for sandbox policy).
  - weakly-typed action contracts centered on untyped maps only.
- Defer:
  - full advanced orchestration variants (`Fork/Join/Delegate`) until engine persistence protocol is stabilized.
  - a full DX macro layer in same crate (optional dx/authoring module; no separate crate).
