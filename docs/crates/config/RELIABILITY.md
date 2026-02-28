# Reliability

## SLO Targets

- availability:
  - config read APIs available with process uptime.
- latency:
  - low-latency typed reads for hot code paths.
- error budget:
  - near-zero tolerance for silent bad-config activation.

## Failure Modes

- dependency outage:
  - file system unavailable, env missing, remote source unavailable.
- timeout/backpressure:
  - slow source loading or heavy validation during reload.
- partial degradation:
  - some optional sources fail while required sources succeed.
- stale config:
  - watcher misses events or reload loop is disabled.

## Resilience Strategies

- retry policy:
  - caller/runtime may retry load/reload on transient source IO failures.
- circuit breaking:
  - applies in remote adapters/runtimes, not in core crate directly.
- fallback behavior:
  - keep last-known-good config on reload failure.
- graceful degradation:
  - optional sources may fail without full startup failure.

## Operational Runbook

- alert conditions:
  - repeated reload failures
  - increased config read/type-conversion failures
  - watcher stopped unexpectedly
- dashboards:
  - source load latency, validation failure counts, reload success ratio.
- incident triage steps:
  1. identify failing source and error class
  2. confirm precedence/merge impact
  3. roll back to last-known-good or disable hot reload
  4. patch source or validation rule and re-enable

## Capacity Planning

- load profile assumptions:
  - frequent reads, infrequent writes/reloads
  - bursty reloads during deployments
- scaling constraints:
  - reload path is CPU/IO bound; ensure validation cost and source count budgets are known.

## Reload Failure Runbook (Contract)

- trigger:
  - reload error or validator rejection on candidate update.
- mandatory behavior:
  - reject candidate atomically.
  - preserve last-known-good active snapshot.
- operator steps:
  1. capture failing source id/path and validator message.
  2. compare candidate delta against precedence layers.
  3. fix source or revert change.
  4. rerun validation and reload.
- diagnostics contract:
  - include source and path context for validator failures.
  - do not include sensitive values in validation diagnostics.
