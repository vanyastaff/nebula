# API

## Stable surface (current)

- `Action` — base trait: `metadata()`, `components()`; object-safe (`dyn Action`)
- `ActionMetadata` — key, name, description, version (`InterfaceVersion`), inputs, outputs, parameters
- `ActionComponents` — static dependency declarations: `credential(CredentialRef)`, `resource(ResourceRef)`
- `Context` — base execution context trait (`Send + Sync`); currently bare — see note below
- `ActionError` + helpers
- `ActionResult<T>` — full control-flow enum (see below)
- `ActionOutput<T>` — payload form enum (see below)
- `WaitCondition` — external event types for `ActionResult::Wait`
- `BreakReason` — termination kinds for `ActionResult::Break`
- `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`

## Minimal action skeleton

```rust
use nebula_action::{Action, ActionComponents, ActionMetadata};

struct MyAction {
    meta: ActionMetadata,
}

impl Action for MyAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    fn components(&self) -> ActionComponents {
        ActionComponents::new()
    }
}
```

## Metadata and ports (contract-first)

```rust
use nebula_action::{ActionMetadata, InputPort, OutputPort};

let meta = ActionMetadata::new("http.request", "HTTP Request", "Calls external API")
    .with_inputs(vec![InputPort::flow("in")])
    .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);
```

Rules:
- key is globally unique per action type (`namespace.name` style recommended)
- default ports are acceptable, but explicit port declaration is preferred for stable contracts

## Dependency declaration (resources + credentials)

```rust
use nebula_action::ActionComponents;
use nebula_credential::CredentialRef;
use nebula_resource::ResourceRef;

struct ApiToken;
struct HttpClient;

let components = ActionComponents::new()
    .credential(CredentialRef::of::<ApiToken>())
    .resource(ResourceRef::of::<HttpClient>());
```

## Execution result and output forms

```rust
use nebula_action::{ActionResult, ActionOutput};

let ok = ActionResult::success_output(ActionOutput::Value(42));
let wait = ActionResult::Wait {
    condition: nebula_action::WaitCondition::Duration {
        duration: std::time::Duration::from_secs(30),
    },
    timeout: Some(std::time::Duration::from_secs(300)),
    partial_output: None,
};
```

Guidelines:
- use `ActionResult::Retry` for intentional reschedule signals
- use `ActionError::Retryable` for transient failures
- use `ActionError::Fatal`/`Validation` for hard stops

## Error helpers

```rust
use nebula_action::ActionError;

let retry = ActionError::retryable_with_backoff("rate limited", std::time::Duration::from_secs(5));
let fatal = ActionError::fatal("invalid schema");
```

## `ActionOutput<T>` variants

The data plane — wraps any payload form the engine understands:

| Variant | Description |
|---------|-------------|
| `Value(T)` | Structured value; serialized for downstream nodes |
| `Binary(BinaryData)` | Raw bytes with content-type (images, files, blobs) |
| `Reference(DataReference)` | Pointer to external storage (S3, blob store) — engine retrieves on demand |
| `Deferred(Box<DeferredOutput>)` | Not-yet-available result; engine resolves via `Resolution` (Poll / Await / Callback / SubWorkflow) |
| `Streaming(StreamOutput)` | Live stream (AI token-by-token, log tail); engine handles backpressure |
| `Collection(Vec<ActionOutput<T>>)` | Multiple outputs in one response; fan-out friendly |
| `Empty` | Explicit "no data" (triggers downstream but carries nothing) |

```rust
// Binary output (e.g. image generation)
ActionResult::success_binary(BinaryData {
    content_type: "image/png".into(),
    data: BinaryStorage::Inline(bytes),
    size: bytes.len(),
    metadata: None,
})

// Deferred output (e.g. async AI job)
ActionResult::success_deferred(DeferredOutput {
    handle_id: "job-123".into(),
    resolution: Resolution::Await { channel_id: "ai-result-ch".into() },
    expected: ExpectedOutput::Value { schema: None },
    ..
})
```

## `ActionResult<T>` variants

| Variant | Engine action |
|---------|--------------|
| `Success { output }` | Pass output to dependent nodes |
| `Skip { reason, output }` | Skip all downstream dependents |
| `Continue { output, progress, delay }` | Re-invoke after optional delay (stateful iteration) |
| `Break { output, reason }` | Finalize iteration, pass output downstream |
| `Branch { selected, output, alternatives }` | Activate specific branch path |
| `Route { port, data }` | Send to a named output port |
| `MultiOutput { outputs, main_output }` | Fan-out to multiple ports simultaneously |
| `Wait { condition, timeout, partial_output }` | Pause until external event |
| `Retry { after, reason }` | Intentional reschedule — *not* an error; upstream not ready |

## `WaitCondition` variants

```rust
WaitCondition::Webhook { callback_id }   // Inbound HTTP callback
WaitCondition::Until { datetime }        // Specific UTC time
WaitCondition::Duration { duration }     // Fixed delay
WaitCondition::Approval { approver, message }  // Human approval
WaitCondition::Execution { execution_id }      // Wait for another execution
```

**Human-in-the-Loop vs Human-on-the-Loop** (from design archive):
- `Approval { on_timeout: OnTimeout::Escalate { .. } }` — hard gate, escalates on timeout
- `Approval { on_timeout: OnTimeout::AutoApprove }` — supervisory, auto-proceeds on timeout

## `BreakReason` variants

```rust
BreakReason::Completed        // All work done naturally
BreakReason::MaxIterations    // Configured iteration limit reached
BreakReason::ConditionMet     // User-defined stop condition
BreakReason::Custom(String)   // Custom reason
```

## Context (current state)

`Context` is currently a bare marker trait (`Send + Sync`) — no capability methods yet. This is by design: the full context + capability model is Phase 2 work.

The temporary bridge used by the engine today is `NodeContext` (doc-hidden):
```rust
// internal — not stable public API
pub struct NodeContext {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub workflow_id: WorkflowId,
    pub cancellation: CancellationToken,
}
```

**Phase 2 target:** `Context` will gain `resource()`, `credential()`, and cancellation access. `NodeContext` will be replaced by a stable adapter. See `ROADMAP.md` Phase 2.

## Planned execution sub-traits (Phase 2)

Currently `Action` has no `execute()` method — that goes in typed sub-traits. These are **not yet in the codebase**; the design is stabilized from the archive:

| Sub-trait | Core / DX | Extends | Purpose |
|-----------|-----------|---------|---------|
| `StatelessAction` | Core | `Action` | Pure function: `execute(input, ctx) → ActionResult<Output>` |
| `StatefulAction` | Core | `Action` | Persistent state: `execute(input, &mut state, ctx)` + `Continue`/`Break` |
| `TriggerAction` | Core | `StatefulAction` | Workflow starter: `start(ctx)` / `stop(ctx)` — lives outside execution graph |
| `ResourceAction` | Core | `Action` | Graph-level DI: `configure(ctx)` → Config; `cleanup(instance, ctx)` |
| `InteractiveAction` | DX | `StatefulAction` | Human-in-the-loop: `Wait { Approval }` with declarative UI helpers |
| `TransactionalAction` | DX | `StatefulAction` | Saga pattern: `execute_tx()` + `compensate()` with `SagaStepKind` |
| `WebhookAction` | DX | `TriggerAction` | Incoming HTTP: `register()`, `handle_request()`, `verify_signature()` |
| `PollAction` | DX | `TriggerAction` | Periodic poll: `poll(cursor, ctx) → PollResult<Event, Cursor>` |

DX types will live in `nebula-action-dx` (separate crate) to keep the core protocol lean.

## Production authoring rules

1. Keep metadata and ports backward compatible inside one major interface version.
2. Declare all external dependencies in `ActionComponents`.
3. Return explicit flow intent with `ActionResult`; avoid out-of-band control channels.
4. Ensure output size/type is predictable for downstream compatibility.
5. Distinguish retryable and fatal errors consistently.
