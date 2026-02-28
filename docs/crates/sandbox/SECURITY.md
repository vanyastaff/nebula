# Security

## Threat Model

- assets:
  - runtime host integrity
  - credential/resource boundaries during action execution
  - tenant isolation and policy enforcement
- trust boundaries:
  - action code (especially third-party/community)
  - sandbox backend boundary
  - capability declaration and evaluation path
- attacker capabilities:
  - execute malicious action logic
  - attempt unauthorized resource/credential/network access
  - exploit weak isolation backend selection

## Security Controls

- authn/authz:
  - upstream authn is outside sandbox; sandbox enforces execution capability policy.
- isolation/sandboxing:
  - current: in-process isolation boundary with cancellation guardrails.
  - target: capability-gated context + full isolation backend for untrusted code.
- secret handling:
  - credential access must be capability-gated and auditable.
- input validation:
  - action metadata and capability declarations must be validated before execution.

## Abuse Cases

- case: action attempts unauthorized credential/resource access.
  - prevention: deny missing capability and return violation error.
  - detection: violation telemetry + audit logging.
  - response: fail execution, optionally quarantine action.
- case: malicious infinite/expensive execution.
  - prevention: cancellation/time/memory limits by sandbox policy.
  - detection: timeout/fuel/memory-limit metrics.
  - response: terminate execution and escalate.
- case: backend policy mis-selection (untrusted action in in-process mode).
  - prevention: strict policy mapping and startup validation.
  - detection: policy-audit mismatch checks.
  - response: deny or reroute to stronger backend.

## Security Requirements

- must-have:
  - explicit backend selection policy per action trust class.
  - fail-fast capability violation behavior.
  - auditable sandbox decision trail.
- should-have:
  - full-isolation backend for untrusted actions.
  - signed/validated capability declarations.

## Security Test Plan

- static analysis:
  - lint policy and backend selection paths.
- dynamic tests:
  - capability deny tests, timeout/cancellation tests, policy-mismatch tests.
- fuzz/property tests:
  - capability matcher and metadata parser fuzzing.
