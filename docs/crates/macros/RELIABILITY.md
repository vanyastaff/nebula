# Reliability

## SLO Targets

- **Availability:** N/A (compile-time only). Compiler availability is the only dependency; macro expansion is deterministic.
- **Latency:** Expansion should complete in reasonable time (seconds for typical crate); no unbounded loops or heavy work in macro.
- **Error budget:** Compile failures are deterministic (invalid input or incompatible trait); no transient failures.

## Failure Modes

- **Invalid attribute or missing required:** Compile error; author fixes. No runtime failure.
- **Trait/contract change in action/plugin/credential/resource:** Author's crate may fail to compile until macro or author code is updated. Mitigation: compatibility matrix and MIGRATION when we break.
- **syn/quote bug or version mismatch:** Unusual parse or expand failure; mitigate with pinned dep versions and CI.

## Resilience Strategies

- **Retry:** N/A (compile-time). Author fixes and recompiles.
- **Graceful degradation:** N/A. Macro either expands or errors.

## Operational Runbook

- **Alert conditions:** N/A (no service). If authors report "macro no longer compiles," check compatibility matrix and recent changes in macro or trait crates.
- **Incident triage:** Verify macro version and trait crate versions; check for attribute or trait breakage; refer to MIGRATION.md.

## Capacity Planning

- N/A (compile-time; no runtime capacity).
