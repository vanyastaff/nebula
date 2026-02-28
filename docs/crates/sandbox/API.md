# API

## Public Surface

- stable APIs:
  - `nebula_ports::sandbox::SandboxRunner`
  - `nebula_ports::sandbox::SandboxedContext`
  - `nebula_sandbox_inprocess::InProcessSandbox`
- experimental APIs:
  - future capability-gated context helpers and full-isolation driver contracts.
- hidden/internal APIs:
  - runtime-provided executor closure internals.

## Usage Patterns

- runtime composes a concrete sandbox driver and calls `SandboxRunner` through the port.
- in-process backend is used for trusted or low-risk actions.
- policy engine selects backend/isolation level per action metadata.

## Minimal Example

```rust
use nebula_ports::sandbox::{SandboxRunner, SandboxedContext};

// runtime code sketch
let ctx = SandboxedContext::new(node_context);
let result = sandbox_runner.execute(ctx, &metadata, input).await?;
```

## Advanced Example

```rust
use std::sync::Arc;
use nebula_sandbox_inprocess::{ActionExecutor, InProcessSandbox};

let executor: ActionExecutor = Arc::new(|ctx, metadata, input| {
    Box::pin(async move {
        // runtime action lookup + execute
        execute_action(ctx, metadata, input).await
    })
});

let sandbox = InProcessSandbox::new(executor);
```

## Error Semantics

- retryable errors:
  - transient backend execution failures (policy-dependent).
- fatal errors:
  - action fatal errors, policy/sandbox violations, unsupported capability in selected backend.
- validation errors:
  - malformed action metadata or invalid sandbox policy configuration.

## Compatibility Rules

- what changes require major version bump:
  - `SandboxRunner` method signature semantics
  - capability/violation contract behavior
- deprecation policy:
  - adapters for one minor release where feasible when evolving execution contracts
