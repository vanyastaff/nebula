# Neon-Inspired Hardening Plan (Resource)

Related spec: `NEON_HARDENING_SPEC.md`

## Status Summary

- completed:
	- poison guard model integrated for pool mutable state
	- create/recycle circuit breaker integration via `nebula-resilience`
	- breaker-open/closed events and typed error surface
	- create/recycle timeout envelope
	- tests for poison, breaker behavior, timeout behavior
- next:
	- improve shutdown gate semantics for spawned background subtasks
	- convert remaining manual wait/run counters to RAII metric guards
	- add ops-facing dashboard guidance for breaker saturation

## Work Packages

## WP1: Pool Cancel-Safety Baseline

Objective:
- ensure interrupted critical sections cannot silently reuse corrupted state

Acceptance:
- pool state accesses use poison arm/disarm flow
- poisoned state produces deterministic failure with diagnostic context

## WP2: Pool Local Failure Storm Protection

Objective:
- stop repeated expensive failures in create/recycle loops

Acceptance:
- create and recycle are breaker-guarded
- breaker-open returns explicit error and emits explicit event
- half-open success transitions back to closed

## WP3: Timeout Envelope and Cleanup Integrity

Objective:
- cap worst-case stall time per create/recycle operation

Acceptance:
- zero timeout is rejected by config validation
- timeout in create returns timeout error
- timeout in recycle triggers cleanup path (no idle leak)

## WP4: Layered Resilience Contract

Objective:
- keep policy ownership clear across crates

Acceptance:
- docs clearly define:
	- pool local breaker protection in `nebula-resource`
	- action-level retry/backoff/rate-limit in engine/runtime

## WP5: Observability and Operational Guidance

Objective:
- make breaker and poisoning behavior easy to operate in production

Acceptance:
- docs include event/error mapping for alerting
- cookbook includes recommended policy profile usage

## Verification Checklist

- `cargo check -p nebula-resource`
- `cargo test -p nebula-resource`
- `cargo clippy -p nebula-resource -- -D warnings`

## Notes

- This plan intentionally does not move global retry orchestration into `nebula-resource`.
- Breaking changes are allowed in current project stage when they improve long-term API clarity.

