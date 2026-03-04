# nebula-sandbox Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Workflow actions may be untrusted (community plugins, user-defined). Running them in-process without isolation risks crashes, infinite loops, and data leakage. A sandbox layer runs actions with capability-checked context (only declared credentials and resources), cancellation checks, and optional hard isolation (WASM, process).

**nebula-sandbox is the sandbox execution contract and isolation boundary for actions.**

It answers: *How does the runtime run an action with enforced capabilities (credentials, resources) and optional isolation so that untrusted code cannot escape or abuse the host?*

```
Runtime has action + Context (credentials, resources from ActionComponents)
    ↓
SandboxRunner (port) receives action + capability-checked context
    ↓
In-process driver: run with cancellation checks; capability proxy blocks undeclared access
    ↓
Future: WASM or process driver for hard isolation
```

This is the sandbox contract: port (trait) decouples runtime from backend; capability enforcement at boundary; auditable violations; no business logic in sandbox crate.

---

## User Stories

### Story 1 — Runtime Runs Action Through Sandbox (P1)

Runtime calls SandboxRunner::run(action, context). Sandbox runs the action (in-process or isolated). Context is a proxy that only allows access to credentials and resources declared in ActionComponents. Violation (e.g. access to undeclared credential) returns error and is logged.

**Acceptance**:
- SandboxRunner trait in nebula-runtime (or future sandbox crate)
- Context passed to action is capability-checked; undeclared access → error
- Cancellation is checked during run (cooperative or periodic)
- No raw NodeContext or full credential/resource access inside sandbox by default

### Story 2 — In-Process Driver for Trusted Actions (P1)

For trusted (built-in) actions, in-process driver runs action in same process with cancellation checks and tracing. No hard isolation; low latency. Capability proxy still enforces declared-only access.

**Acceptance**:
- In-process driver implements SandboxRunner
- Cancellation token or periodic check so long-running action can be stopped
- Tracing/spans for observability
- Document that in-process is not safe for untrusted code

### Story 3 — Hard Isolation for Untrusted Actions (P2)

For untrusted or community actions, WASM or process driver runs action in isolated environment. Same SandboxRunner port; different backend. Network/filesystem access denied or gated by capability.

**Acceptance**:
- Second backend (WASM or process) implements SandboxRunner
- Capability model: which capabilities (network, fs, credential, resource) are granted per action
- Violation (e.g. network call when not granted) is blocked and auditable
- Document isolation guarantees and limits

### Story 4 — Operator Sees Sandbox Violations (P2)

When an action tries to access undeclared credential or resource, sandbox returns error and emits event or log. Operator can alert on violation rate.

**Acceptance**:
- Violation is explicit error variant (e.g. SandboxError::CapabilityViolation)
- Log or event includes action key, execution_id, and what was attempted
- No secret material in violation log

---

## Core Principles

### I. Sandbox Is a Port (Trait), Not Single Implementation

**Runtime depends on SandboxRunner trait. In-process, WASM, process are different implementations. Runtime does not depend on concrete backend.**

**Rationale**: Allows trusted vs untrusted path (in-process vs WASM) without changing runtime code. Testing with mock sandbox.

**Rules**:
- SandboxRunner (or equivalent) in nebula-runtime or nebula-sandbox
- Runtime receives SandboxRunner via constructor or config
- At least two backends: inprocess (trusted), wasm/process (untrusted) for production target

### II. Capability Enforcement at Boundary

**Context passed to action inside sandbox only exposes credentials and resources declared in ActionComponents. Access to undeclared capability is denied and auditable.**

**Rationale**: Least privilege. Untrusted action cannot probe or access other tenants' credentials.

**Rules**:
- SandboxedContext or proxy wraps real context; only declared keys allowed
- Undeclared access → error; no silent denial without log
- ActionComponents is the single source of declared capabilities

### III. Cancellation Is Honored

**Long-running action must be cancellable. Sandbox checks cancellation token (or equivalent) so that runtime can abort.**

**Rationale**: Otherwise one stuck action can block shutdown or waste capacity.

**Rules**:
- Context or sandbox run accepts cancellation token
- In-process driver checks periodically or on I/O; document period
- Isolated driver (WASM/process) has kill or timeout mechanism

### IV. No Action Business Logic in Sandbox

**Sandbox runs the action; it does not implement actions or workflow logic.**

**Rationale**: Actions are in action crate and plugins. Sandbox is the isolation layer.

**Rules**:
- Sandbox only runs what runtime gives it; no interpretation of workflow
- No dependency on concrete action types beyond Action trait

### V. Auditable Violations and Policy

**Capability violations and policy decisions (e.g. which backend chosen) are logged or emitted. No secret in logs.**

**Rationale**: Security and ops need to see abuse attempts and policy application.

**Rules**:
- Violation event/log: action key, execution_id, capability type (e.g. credential), key attempted
- No credential value or secret in log
- Optional: metrics for violation count per action or tenant

---

## Production Vision

### The sandbox in an n8n-class fleet

In production, runtime runs every action through SandboxRunner. Trusted actions use in-process driver (low latency, cancellation checks). Untrusted actions use WASM or process driver (hard isolation, capability gating). Capability proxy ensures only declared credentials and resources are accessible. Violations are logged and metered.

```
SandboxRunner (port)
    ├── InProcessDriver: trusted actions, cancellation, tracing
    └── WasmDriver / ProcessDriver: untrusted, capability gating, kill/timeout

SandboxedContext: only ActionComponents-declared credential/resource keys
Violations → SandboxError::CapabilityViolation + log/event
```

From the archives: sandbox runner port, in-process driver, capability/cancellation enforcement. Production vision: stable port contract, at least two backends, explicit capability enforcement, auditable violations.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Full capability enforcement in SandboxedContext | High | Current wrapper may mostly forward; enforce declared-only |
| WASM or process isolation backend | High | For untrusted/community actions |
| Cancellation and timeout in isolated backend | High | Kill or timeout when action hangs |
| Violation event/metric | Medium | For alerting and audit |
| Standalone crates/sandbox crate | Medium | Currently port + driver; may consolidate |

---

## Key Decisions

### D-001: SandboxRunner in nebula-runtime

**Decision**: SandboxRunner trait lives in nebula-runtime; InProcessSandbox is the default implementation.

**Rationale**: Decouples runtime from concrete sandbox. Multiple backends possible.

**Rejected**: Runtime depending on single sandbox impl — would block WASM/process.

### D-002: Capability Proxy, Not Trust

**Decision**: Even for in-process, context is a proxy that only allows declared credentials/resources.

**Rationale**: Consistent enforcement. In-process is a performance choice, not a security exception.

**Rejected**: In-process gets raw context — would allow trusted action to escape declaration.

### D-003: Cancellation Required

**Decision**: Sandbox run accepts cancellation; driver must check so runtime can abort.

**Rationale**: Shutdown and resource fairness require abortability.

**Rejected**: No cancellation — would risk stuck actions.

### D-004: Violations Are Errors and Logged

**Decision**: Undeclared access returns error and is logged (no silent deny).

**Rationale**: Audit and debugging. Silent deny would hide misuse.

**Rejected**: Silent deny — would prevent audit trail.

---

## Non-Negotiables

1. **Sandbox is a port (trait)** — runtime does not depend on single backend.
2. **Capability enforcement at boundary** — only declared credentials/resources; violation = error + log.
3. **Cancellation honored** — token or timeout so runtime can abort.
4. **No action business logic in sandbox** — only run and isolate.
5. **Violations auditable** — no secret in logs; explicit error variant.
6. **Breaking sandbox or capability contract = major + MIGRATION.md** — runtime and actions depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to port or capability semantics.
- **MINOR**: Additive (new backend, new capability type). No removal.
- **MAJOR**: Breaking port or capability semantics. Requires MIGRATION.md.
