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

## Target architecture (production-complete)

1. Stable contract layer (`nebula-action`)
- frozen core traits, result/output/error/port models
- versioned compatibility policy

2. Authoring DX layer (`nebula-action-dx`, proposed sibling crate)
- optional trait families and helper macros for common action patterns
- no contamination of core contracts

3. Runtime adapter layer (`nebula-runtime`)
- context implementation and orchestration
- adapter from runtime state to `Context` and action capabilities

4. Sandbox adapter layer (`nebula-sandbox-*`)
- capability-checked proxies around context operations
- enforce least-privilege access declared in metadata/components

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
  - a full DX macro layer inside core crate (prefer optional sibling crate to keep protocol boundary lean).
