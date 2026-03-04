# Reliability

## SLO Targets

- **Availability:** N/A (library). Registry is in-memory; no service. Callers (engine/API) depend on registry being populated and get/list not failing.
- **Latency:** Registry get/list are O(1) or O(n) in-memory; no I/O in default use. Dynamic load is one-time or rare.
- **Error budget:** Register fails only on duplicate key or invalid input; deterministic.

## Failure Modes

- **Duplicate key:** Register returns AlreadyExists; caller must use different key or skip. No crash.
- **Dynamic load failure:** PluginLoadError; caller handles (skip plugin, log, retry). No panic in loader.
- **Registry not populated:** Engine or API must populate at startup; plugin crate does not auto-discover. Operational responsibility.

## Resilience Strategies

- **Retry:** Not applicable for register (duplicate is permanent for that key). Load could be retried by caller (e.g. path temporarily missing).
- **Graceful degradation:** If load fails, caller can continue with static plugins only; document in runbook.

## Operational Runbook

- **Alert conditions:** N/A (no service). If engine fails to resolve action, check registry was populated and plugin key is correct.
- **Incident triage:** Verify registry contains expected keys; check loader path and permissions if using dynamic-loading.

## Capacity Planning

- **Load profile:** Registry size = number of plugins; in-memory only. No scaling limit in crate; engine may have limits on total actions.
- **Scaling constraints:** N/A.
