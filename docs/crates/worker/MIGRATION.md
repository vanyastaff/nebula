# Migration

## Migration Intent

Establish `worker` as production execution plane without losing historical architectural notes.

## Documentation Migration (Completed in this step)

- moved legacy files into `docs/crates/worker/_archive/`
- created template-compliant production doc set
- linked archive index for traceability

## Implementation Migration Plan

1. Introduce minimal `crates/worker` skeleton with config + lifecycle primitives.
2. Add queue lease contract integration and contract tests.
3. Integrate sandbox/resource/resilience boundaries with feature flags.
4. Enable staged rollout with canary worker group and SLO monitoring.

## Compatibility Notes

- initial implementation should preserve at-least-once semantics from docs baseline.
- any shift toward stronger/alternative delivery guarantees must go through decision + migration process.

## Rollback Strategy

- disable new worker deployment and keep control plane accepting no new claims.
- rely on queue redelivery to stable worker version.
- replay pending completion envelopes after rollback if needed.
