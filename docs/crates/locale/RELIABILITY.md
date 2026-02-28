# Reliability

## SLO Targets

- availability:
  - localization service path should not degrade core workflow execution availability.
- latency:
  - translation lookup/render within user-facing response budgets.
- error budget:
  - fallback usage is acceptable within defined limits; localization hard-fail incidents are low-tolerance.

## Failure Modes

- dependency outage:
  - catalog source unavailable.
- timeout/backpressure:
  - excessive lookup pressure or cold-start catalog loads.
- partial degradation:
  - missing keys forcing fallback chains.
- data corruption:
  - invalid/mismatched catalog bundles.

## Resilience Strategies

- retry policy:
  - retry transient catalog retrieval failures with bounded backoff.
- circuit breaking:
  - resilience wrappers for remote catalog providers (if enabled).
- fallback behavior:
  - fallback to default locale and message key-safe output.
- graceful degradation:
  - preserve canonical errors when localized render unavailable.

## Operational Runbook

- alert conditions:
  - spikes in missing keys, fallback ratio, or catalog load failures.
- dashboards:
  - render latency, fallback counts, missing-key distribution by locale.
- incident triage steps:
  1. identify failing locale/key sets.
  2. verify catalog integrity and deployment version.
  3. assess impact and enable safe fallback policy.
  4. patch catalogs and validate with smoke tests.

## Capacity Planning

- load profile assumptions:
  - high-read, low-write catalog access with bursty API traffic.
- scaling constraints:
  - in-memory catalog footprint and lookup cache efficiency.
