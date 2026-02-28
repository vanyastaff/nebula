# Security

## Threat Model

- assets:
  - action contract integrity
  - capability boundary correctness
  - safe handling of data references and deferred/streaming flows
- trust boundaries:
  - untrusted user/workflow definitions and plugin-provided action code
  - privileged runtime capabilities (resources, credentials, network)
- attacker capabilities:
  - malicious action code
  - malformed inputs/outputs
  - capability escalation attempts

## Security Controls

- authn/authz:
  - handled by control-plane and runtime; action crate models capability-relevant error paths.
- isolation/sandboxing:
  - capability denials map to `ActionError::SandboxViolation`.
- secret handling:
  - this crate should not own secret persistence; credentials stay external.
- input validation:
  - parameter and metadata validation in upstream crates before execution.

## Abuse Cases

- case: action tries undeclared resource/credential access.
  - prevention: sandbox capability gate.
  - detection: explicit `SandboxViolation` telemetry.
  - response: deny and audit.
- case: action emits unbounded output payload.
  - prevention: runtime output limits.
  - detection: `DataLimitExceeded`.
  - response: stop node execution and alert.
- case: malicious deferred/streaming protocol misuse.
  - prevention: runtime-side resolution policy and allowlists.
  - detection: stalled/invalid handle monitoring.
  - response: timeout/fail and quarantine action runtime.

## Security Requirements

- must-have:
  - deterministic, explicit failure signaling for policy violations.
  - no hidden privileged operations in contract crate.
  - stable, auditable error taxonomy.
- should-have:
  - contract-level tests for sandbox-relevant failure paths.
  - documented secure defaults for output size and resolution timeouts.

## Security Test Plan

- static analysis:
  - enforce `forbid(unsafe_code)` and lint policy.
- dynamic tests:
  - sandbox violation and data limit behavior tests.
- fuzz/property tests:
  - serialization robustness for result/output enums.
