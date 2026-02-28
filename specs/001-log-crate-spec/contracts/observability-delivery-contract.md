# Contract: Observability Delivery and Isolation

## Purpose

Define delivery semantics for multi-destination logging, hook execution safety, and graceful degradation when optional telemetry/export paths are unavailable.

## Delivery Model

- Multi-destination mode performs true fanout to all configured healthy destinations.
- Failure policy options:
  - `FailFast`: surface sink failure immediately.
  - `BestEffort`: continue delivery to healthy sinks while recording failure signal.
  - `PrimaryWithFallback`: prefer primary sink and route to fallback according to policy.

## Policy Examples

- `FailFast`: a file destination failure aborts the current write attempt and returns an error.
- `BestEffort`: if stderr succeeds and file fails, the event is still emitted to stderr.
- `PrimaryWithFallback`: if the primary destination fails, the first healthy fallback receives the event.

## Hook Safety Model

- Hook panics are always isolated and must not crash event emission.
- Hook execution budget is bounded when bounded policy is enabled.
- Slow or failing hooks produce diagnostic signals without terminating core logging.

## Degradation Model

- Optional telemetry/export outages do not disable core log emission.
- Destination-level failure does not affect unrelated healthy sinks under non-fail-fast policies.

## Acceptance Contract

- In fault injection with one failing destination, behavior matches selected policy.
- Panicking hooks do not terminate process or stop other hook execution.
- Under degraded backends, log continuity is maintained.

## Compatibility Rules

- New failure-policy variants are additive.
- Existing policy semantics must remain stable in minor releases.
- Behavior removals or semantic redefinition require major-version migration guidance.
