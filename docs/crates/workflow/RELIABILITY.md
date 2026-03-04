# Reliability

## SLO Targets

- **Availability:** N/A (library; no service). Callers (engine, API) depend on workflow types and validation being deterministic and fast.
- **Latency:** Validation and graph build should be O(nodes + edges); no I/O in crate.
- **Error budget:** Validation failures are deterministic (invalid input); no transient failures inside crate.

## Failure Modes

- **Invalid input:** Caller passes invalid definition → validate_workflow returns errors; no panic. Graph build may fail if validation was skipped; document "validate first."
- **Dependency:** petgraph and core are required; no fallback. Version pinning in Cargo.toml.

## Resilience Strategies

- **Retry:** Not applicable; validation is pure and deterministic.
- **Graceful degradation:** N/A; crate has no runtime.

## Operational Runbook

- **Alert conditions:** N/A (no service).
- **Dashboards:** N/A.
- **Incident triage:** If engine or API misbehaves on workflow load, check validation and graph build; ensure validate_workflow was called and errors handled.

## Capacity Planning

- **Load profile:** Validation and graph construction scale with definition size. API/engine should enforce max definition size to avoid CPU/memory spikes.
- **Scaling constraints:** Single-threaded validation; no internal parallelism.
