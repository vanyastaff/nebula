# Migration

## Versioning Policy

- compatibility promise:
  - keep `SandboxRunner` contract stable within major versions.
- deprecation window:
  - at least one minor release for non-critical backend/API transitions.

## Breaking Changes

- change:
  - evolution of `SandboxedContext` toward explicit capability-gated API.
  - old behavior:
    - thin wrapper over `NodeContext`.
  - new behavior:
    - enforced capability checks on sensitive operations.
  - migration steps:
    1. add compatibility wrappers.
    2. migrate runtime/action call sites to gated APIs.
    3. remove legacy passthrough paths.
- change:
  - introduction of full-isolation backend policy defaults.
  - old behavior:
    - in-process default for most actions.
  - new behavior:
    - trust-class based backend selection with stricter defaults.
  - migration steps:
    1. classify actions by trust level.
    2. run dual-mode validation in staging.
    3. cut over policy defaults.

## Rollout Plan

1. preparation
   - define contracts, feature flags, and adapter layer.
2. dual-run / feature-flag stage
   - run new backend in shadow/canary mode.
3. cutover
   - switch default backend policy for target action classes.
4. cleanup
   - remove deprecated legacy behavior.

## Rollback Plan

- trigger conditions:
  - elevated failure rates, policy mismatch incidents, significant latency regression.
- rollback steps:
  - revert backend policy to prior stable configuration.
- data/state reconciliation:
  - reconcile failed executions and audit events after rollback.

## Validation Checklist

- API compatibility checks:
  - compile/runtime tests across `ports`, `runtime`, and drivers.
- integration checks:
  - cancellation, violation, and backend-selection flows.
- performance checks:
  - compare sandbox overhead before/after cutover.
