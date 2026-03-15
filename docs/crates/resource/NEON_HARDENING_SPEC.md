# Neon-Inspired Hardening Spec (Resource)

## Scope

This spec defines reliability hardening for `nebula-resource` pool internals using patterns inspired by Neon.

Goals:
- prevent inconsistent pool state reuse after interrupted critical sections
- avoid create/recycle failure storms under degraded dependencies
- keep action-level resilience policy centralized in engine/runtime

## Non-Goals

- moving all retries into `nebula-resource`
- replacing existing scope/health/quarantine model
- introducing cross-crate dependency violations

## Invariants

1. Pool mutable state is never reused silently after unsafe interruption.
2. Create/recycle expensive paths are guarded by circuit breakers.
3. Breaker-open state is observable via explicit events and classified errors.
4. Action-level retry/backoff/rate-limit remains outside `nebula-resource`.
5. Shutdown and cleanup remain deterministic under concurrent load.

## Architecture Split With `nebula-resilience`

### Inside `nebula-resource`

- Use `nebula-resilience` circuit breaker on pool `create` and `recycle` paths.
- Map breaker-open to resource-level errors/events.
- Keep pool self-protection local to pool operation boundary.

### Outside `nebula-resource` (engine/runtime/action loop)

- Keep retry/backoff/rate-limit orchestration at action execution boundary.
- Consume error category/retry hints from resource layer.

This layered split avoids duplicated policy and keeps pool internals safe under pressure.

## Error and Event Contract

Expected signals:
- `Error::CircuitBreakerOpen { operation, retry_after }`
- `ResourceEvent::CircuitBreakerOpen { operation, retry_after }`
- `ResourceEvent::CircuitBreakerClosed { operation }`
- Poisoned-state failures mapped to internal errors with clear diagnostics.

## Timeout Envelope

Pool supports optional operation timeouts:
- `create_timeout`
- `recycle_timeout`

Timeouts must be `> 0` when configured.

## Test Matrix (minimum)

1. Poison arm/disarm happy path.
2. Poison drop-without-disarm path.
3. Acquire fails after deliberate pool poison.
4. Create breaker opens after threshold failures.
5. Half-open probe closes breaker on success.
6. Recycle breaker-open path skips recycle call.
7. Create timeout returns timeout error.
8. Recycle timeout triggers cleanup path.

## Documentation Requirements

The following docs must stay in sync:
- `ARCHITECTURE.md`
- `VISION.md`
- `docs/cookbook.md`
- `docs/adapters.md`
- `TASKS.md`
