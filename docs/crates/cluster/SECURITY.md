# Security

## Threat Model

- assets:
  - control-plane integrity and scheduling ownership state
  - cluster membership authenticity
  - workflow execution continuity under attack/failure
- trust boundaries:
  - inter-node control traffic
  - operator APIs (CLI/API)
  - storage-backed control state
- attacker capabilities:
  - spoof node membership
  - issue unauthorized control commands
  - induce partitions or stale-state decisions

## Security Controls

- authn/authz:
  - mutual authentication for node-to-node control traffic.
  - role-based authorization for operator commands.
- isolation/sandboxing:
  - separate control-plane from worker data-plane privileges.
- secret handling:
  - secure management and rotation of cluster certificates/keys.
- input validation:
  - strict validation for membership and control commands.

## Abuse Cases

- case: rogue node joins cluster.
  - prevention: mTLS identity verification and join policy checks.
  - detection: membership anomaly alerts.
  - response: deny join and quarantine suspicious endpoint.
- case: unauthorized rebalance/failover command.
  - prevention: authenticated operator API with least-privilege roles.
  - detection: audit trail mismatch detection.
  - response: reject command and trigger incident workflow.
- case: partition-induced split-brain scheduling.
  - prevention: consensus-safe leader election and fencing.
  - detection: leadership conflict alarms.
  - response: fail-safe mode and controlled recovery.

## Security Requirements

- must-have:
  - authenticated membership and encrypted control-plane traffic.
  - auditable, authorized cluster control operations.
  - fencing against split-brain ownership.
- should-have:
  - periodic key rotation and certificate revocation flow.
  - policy-as-code validation gates for critical operations.

## Security Test Plan

- static analysis:
  - config and authz policy linting.
- dynamic tests:
  - membership spoofing and unauthorized command scenarios.
- fuzz/property tests:
  - control command parser and state-transition invariants.
