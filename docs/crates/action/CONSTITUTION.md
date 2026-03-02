# nebula-action Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula workflows are DAGs of nodes. Each node is implemented by an action: "Send Slack Message", "Create GitHub Issue", "Run SQL Query". The engine and runtime need a stable contract so they can schedule, execute, and sandbox actions without depending on concrete implementations.

**nebula-action is the contract boundary between action authors and the engine/runtime.**

It answers: *What must an action implement, what can it return, and what dependencies can it declare — so that the platform can execute it safely and deterministically?*

```
workflow definition references action by key (e.g. "slack_send_message")
    ↓
engine loads ActionMetadata + ActionComponents (credentials, resources)
    ↓
runtime builds Context (credentials, resources, scope) and calls action.execute(input, &ctx)
    ↓
action returns Result<ActionResult<Output>, ActionError> — Success, Skip, Continue, Break, Branch, Route, MultiOutput, Wait, Retry (or Retryable/Fatal/Validation/SandboxViolation/Cancelled/DataLimitExceeded)
    ↓
engine interprets result (continue, suspend, branch, fail) without knowing action internals
```

This is the action contract: stable, versioned, and enforceable by the sandbox.

---

## User Stories

### Story 1 — Action Author Implements a Simple Node (P1)

A developer writes a "Send Slack Message" action. They implement the `Action` trait, declare required credentials and resources in `ActionComponents`, and return `ActionResult::Done(output)`. They do not need to know how the engine schedules or how the sandbox isolates.

**Acceptance**:
- Implement `Action`: `metadata()`, `components()`; and one of ProcessAction / StatefulAction / TriggerAction / etc. with `execute(input, &ctx)`
- `ActionComponents` lists `CredentialRef` and `ResourceRef` with keys
- Return `ActionResult::Success { output }` on success; retry/suspend via `ActionResult::Retry` / `Wait` or `ActionError::Retryable` / `Fatal`
- Retryable vs fatal signaled via `ActionError::Retryable` vs `ActionError::Fatal`

### Story 2 — Engine Author Interprets Results Without Coupling (P1)

The engine executes a node and receives `ActionResult`. It must decide: continue to next node, suspend for external event, fork branches, or fail execution. The engine must not depend on concrete action types.

**Acceptance**:
- Match on `ActionResult::Success`, `Skip`, `Continue`, `Break`, `Branch`, `Route`, `MultiOutput`, `Wait`, `Retry` and on `ActionError` without downcast
- Flow-control semantics (WaitCondition, BreakReason) are documented and stable
- New result variants require minor/major version and migration notes

### Story 3 — Sandbox Enforces Declared Capabilities (P2)

The sandbox runs untrusted or third-party actions. It must allow only the credentials and resources declared in `ActionComponents` and expose them through a capability-checked context.

**Acceptance**:
- Context passed to action is a proxy that only exposes declared credential/resource keys
- Violations (e.g. access to undeclared credential) produce explicit ActionError variant
- Sandbox boundary is documented in SECURITY.md and INTERACTIONS.md

### Story 4 — Operator Migrates Action Trait Evolution (P3)

When the action trait gains new optional methods or output forms, existing actions continue to work. Migration path is documented and versioned.

**Acceptance**:
- New optional trait methods have default impls or are behind feature flags
- Breaking changes go through major version and MIGRATION.md
- PROPOSALS documents candidate extensions (StreamingAction, BatchAction, etc.)

---

## Core Principles

### I. Contract First, Implementation Second

**Action is a protocol: metadata, components, execute, and result/output/error types are the contract. Runtime and sandbox implement the other side.**

**Rationale**: If the contract were defined by the engine or sandbox, action authors would be tied to one runtime. A clear contract allows multiple runtimes and sandboxes to coexist.

**Rules**:
- No engine- or sandbox-specific types in the core Action trait signature
- Context is a minimal trait (or bridge) that runtime fills with concrete capabilities
- Result and output types are exhaustive and documented

### II. Deterministic Flow-Control Semantics

**ActionResult and WaitCondition/BreakReason have defined semantics that the engine can interpret without action-specific logic.**

**Rationale**: Non-deterministic or underspecified semantics would make execution order and retries unpredictable. Operators need reproducible behavior.

**Rules**:
- Every result variant has documented engine behavior
- Retryable vs fatal is explicit in ActionError
- New flow-control variants require spec update and engine support

### III. Declared Dependencies Only

**Actions declare credentials and resources in ActionComponents. They cannot access undeclared dependencies.**

**Rationale**: Undeclared access prevents proper sandboxing and makes audit and multi-tenancy impossible. Declared dependencies enable capability-based security.

**Rules**:
- CredentialRef and ResourceRef list all required keys
- Context (or sandbox proxy) only exposes those keys
- Violations are errors, not silent denial

### IV. No Orchestration Logic in This Crate

**nebula-action defines what an action is and what it returns. It does not define how the engine schedules, retries, or routes.**

**Rationale**: Orchestration belongs in engine/runtime. Mixing it here would create circular dependency or blur ownership.

**Rules**:
- No scheduler, executor, or workflow DAG types in nebula-action
- Retry/backoff policy is consumed from context or resilience crate, not defined here

### V. Backward Compatibility and Versioned Evolution

**Contract changes that break existing actions require a major version and a migration path.**

**Rationale**: Workflow ecosystems are long-lived. Breaking the action contract without migration would break every deployed workflow.

**Rules**:
- Additive changes (new optional methods, new output variants with default handling) in minor
- Removals or signature changes in major with MIGRATION.md
- PROPOSALS document breaking and non-breaking extensions

---

## Production Vision

### The action contract in an n8n-class fleet

In a production Nebula deployment, hundreds of action types (built-in and plugin) are loaded by the engine. Each action is identified by key, has metadata and components, and is executed inside a sandbox that enforces declared capabilities. The engine never imports concrete action crates for flow control — only for registration and execution.

```
action.rs    — Action trait (metadata, components); no execute (that’s in ProcessAction/StatelessAction, StatefulAction, TriggerAction (same crate; D009))
metadata.rs  — ActionMetadata (key, name, description, version, inputs, outputs, parameters: ParameterCollection)
components.rs — ActionComponents (credentials: Vec<CredentialRef>, resources: Vec<ResourceRef>)
port.rs      — InputPort, OutputPort, SupportPort, DynamicPort; FlowKind; ConnectionFilter; PortKey
result.rs    — ActionResult<T>: Success, Skip, Continue, Break, Branch, Route, MultiOutput, Wait, Retry
output.rs    — ActionOutput<T>, BinaryData, DataReference, DeferredOutput, StreamOutput, …
error.rs     — ActionError: Retryable, Fatal, Validation, SandboxViolation, Cancelled, DataLimitExceeded
context.rs   — Context, NodeContext (bridge)
```

Engine: registry (plugin_key → ActionMetadata + factory); for each node resolve action → build Context from credential + resource managers → call action.execute(input, &ctx); match ActionResult / ActionError → schedule next / suspend / fail. Action authors implement `Action` and register; runtime provides Context; sandbox wraps context in capability-checked proxy.

### From the archives: specialized execution traits and extended results

The archive (`docs/crates/action/_archive/`: archive-ideas.md, archive-layers-interaction.md, archive-nebula-action-types.md, archive-node-execution.md, archive-crates-dependencies.md, archive-nebula-credential-architecture-2.md) proposed specialized traits and extended result variants:

- **StreamingAction** — `execute_stream` returning a stream of items
- **BatchAction** — vectorized execution
- **StatefulAction** — explicit save/load state hooks
- **Extended ActionResult** — Suspend, Fork, Join, Delegate, Error with recovery

These are not current contract but inform production vision: the core contract stays minimal; advanced patterns (streaming, batch, stateful) can be added as optional traits or result variants with explicit engine support. Production path: keep core stable; introduce extensions via versioned proposals and optional traits.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Frozen contract + versioned compatibility policy | High | Document stable surface vs experimental |
| Capability-checked context proxy (sandbox) | High | Enforce ActionComponents at runtime |
| ActionContext / TriggerContext concrete types | Medium | Replace NodeContext bridge; capability modules |
| StreamingAction / BatchAction optional traits | Low | From archive; engine must support first |
| Extended result variants (Suspend, Fork, Join) | Low | Backlog; requires engine and migration |

---

## Key Decisions

### D-001: ActionResult as Enum, Not Generic Event Stream

**Decision**: Engine receives a closed enum (Success, Skip, Continue, Break, Branch, Route, MultiOutput, Wait, Retry) per execution step; errors are separate `ActionError`. Not an open-ended event stream.

**Rationale**: Deterministic engine behavior and testability. Event streams would require engine to handle arbitrary event types and ordering.

**Rejected**: Generic "action emits events" — too open for initial production contract.

### D-002: Context as Trait (or Bridge), Not Concrete in Action Crate

**Decision**: Action receives a context type that provides credentials and resources; concrete implementation lives in runtime/sandbox.

**Rationale**: Action crate must not depend on runtime or sandbox. Trait (or bridge) keeps dependency direction correct.

**Rejected**: Action depending on nebula-runtime for NodeContext — would create cycle.

### D-003: ActionComponents Declare All Dependencies

**Decision**: Every credential and resource the action needs must be listed in ActionComponents.

**Rationale**: Enables sandbox to grant least-privilege access and operators to audit usage.

**Rejected**: Implicit or lazy resolution — no way to enforce or audit.

### D-004: ActionError Distinguishes Retryable vs Fatal

**Decision**: ActionError has variants (or attributes) that classify retryable vs fatal for the engine.

**Rationale**: Engine and resilience layer need to decide retry vs fail-fast without action-specific logic.

**Rejected**: Single error type with no classification — would push policy into engine heuristics.

---

## Open Proposals

### P-001: ActionContext and TriggerContext Concrete Types

**Problem**: NodeContext is a temporary bridge; target is explicit capability modules.

**Proposal**: Introduce ActionContext and TriggerContext structs composed of ResourceAccessor, CredentialAccessor, etc.; runtime implements these.

**Impact**: Breaking for action authors if NodeContext is removed; migration path required.

### P-002: Optional StreamingAction and BatchAction Traits

**Problem**: Some actions need stream or batch execution; current contract is single execute().

**Proposal**: Add optional traits (StreamingAction, BatchAction) with default no-op or "not supported"; engine checks for implementation.

**Impact**: Additive; engine and runtime must implement stream/batch scheduling.

### P-003: Extended ActionResult Variants (Suspend, Fork, Join)

**Problem**: Advanced workflows need suspend/resume and fan-out/fan-in.

**Proposal**: Add variants with explicit semantics and engine handling; document in contract.

**Impact**: Requires engine changes and migration for any existing use of custom flow control.

---

## Non-Negotiables

1. **Action trait is the single contract** — engine and sandbox depend on this crate, not on concrete action crates for flow control.
2. **ActionResult is exhaustive and documented** — every variant has defined engine behavior.
3. **ActionComponents declare all credentials and resources** — no undeclared access.
4. **Retryable vs fatal is explicit** — ActionError supports engine/resilience policy.
5. **No orchestration in nebula-action** — no scheduler, DAG, or execution loop in this crate.
6. **Context is abstract** — concrete context implementation lives in runtime/sandbox.
7. **Breaking contract = major version + MIGRATION.md** — backward compatibility for workflow ecosystems.

---

## Governance

- **PATCH**: Bug fixes, docs, internal refactors. No change to Action trait, ActionResult, ActionError, or ActionOutput.
- **MINOR**: Additive only (new optional trait methods, new result/output variants with default engine behavior). No removal or change of existing semantics.
- **MAJOR**: Breaking changes to contract (trait signature, result/output/error shape). Requires MIGRATION.md and deprecation cycle where feasible.

Every PR must verify: no new dependency on engine/runtime/sandbox; new result variants have documented engine behavior; ActionComponents remain the single source of declared dependencies.
