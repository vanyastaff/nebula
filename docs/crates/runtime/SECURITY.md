# Security

## Threat Model

### Assets

- Action execution (may run user-defined or plugin code)
- Credentials passed to actions (via context)
- Workflow input/output data

### Trust Boundaries

- **Trusted actions:** Built-in, IsolationLevel::None; run in process
- **Untrusted actions:** Plugin code; should run in sandbox (Phase 2)
- **Sandbox:** Capability-gated; limits network, credentials, resources

### Attacker Capabilities

- **Malicious plugin:** May try to access credentials, escape sandbox, exhaust resources
- **Large input:** May cause OOM; data limits mitigate

## Security Controls

### Authn/Authz

- Runtime does not perform authn/authz. Engine/API layer responsible.
- Credentials injected into context by engine; runtime passes context to handler.

### Isolation/Sandboxing

- **Current:** All actions run in process; no isolation. TODO: route untrusted to sandbox.
- **Target:** IsolationLevel::CapabilityGated/Isolated → SandboxedContext; capability checks before resource/credential access.
- **SandboxRunner:** Ports trait; drivers implement (e.g. inprocess, wasm, container).

### Secret Handling

- Runtime does not store secrets. Context carries credentials; actions receive them.
- Events (NodeFailed) may include error strings; sanitize to avoid leaking secrets.

### Input Validation

- Input is `serde_json::Value`; runtime does not validate structure. Action validates.
- DataPassingPolicy limits output size; prevents unbounded allocation.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|-------------|------------|----------|
| Malicious action reads all credentials | Sandbox capability check | Audit log | SandboxViolation |
| Action allocates huge output | DataPassingPolicy max_node_output_bytes | DataLimitExceeded | Reject; emit NodeFailed |
| Action hangs indefinitely | (Future) execution timeout | — | Cancel token in context |
| Registry poisoning | Trusted registration path | — | Only app/engine registers |

## Security Requirements

### Must-Have

- Data limits enforced
- Sandbox for untrusted actions (Phase 2)
- No credential logging in events

### Should-Have

- Execution timeout
- Capability audit logging

## Security Test Plan

- **Static analysis:** cargo audit; no unsafe
- **Dynamic tests:** Data limit enforcement; ActionNotFound for unknown key
- **Fuzz:** Optional; input fuzz for action execution
