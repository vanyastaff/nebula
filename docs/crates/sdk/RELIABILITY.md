# Reliability

## SLO Targets

- **Availability:** N/A (library). Authors expect prelude and builders to be stable and compile.
- **Latency:** N/A; no runtime service.
- **Error budget:** Build and test failures are deterministic (author or compatibility issue).

## Failure Modes

- **Compatibility break:** Action or runtime contract changes; sdk not updated → author compile or test failure. Mitigation: compatibility tests and version matrix.
- **Dep/version conflict:** Author depends on sdk and different version of core/action. Mitigation: document compatible versions; sdk follows platform versioning.

## Resilience Strategies

- **Retry:** N/A.
- **Graceful degradation:** Optional features (builders, testing) can be disabled; minimal prelude still works.

## Operational Runbook

- **Alert conditions:** N/A (no service).
- **Incident:** If authors report breakage, check compatibility matrix and action/runtime changes; release patch or document workaround.

## Capacity Planning

- N/A (library).
