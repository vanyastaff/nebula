# Migration

## Versioning Policy

- compatibility promise:
  - stable cluster placement/failover contracts within major versions.
- deprecation window:
  - minimum one minor release for non-critical API and policy transitions.

## Breaking Changes

- change:
  - migration from non-cluster-aware runtime assumptions to cluster placement contracts.
  - old behavior:
    - primarily local execution decisions.
  - new behavior:
    - control-plane placement and failover ownership.
  - migration steps:
    1. add adapter layer in runtime.
    2. run shadow placement decisions.
    3. cut over scheduling ownership.
- change:
  - scheduling strategy semantics refinements.
  - old behavior:
    - legacy or fixed strategy assumptions.
  - new behavior:
    - explicit strategy profiles and policy controls.
  - migration steps:
    1. benchmark strategy impact.
    2. canary rollout by workload class.

## Rollout Plan

1. preparation
   - implement cluster MVP contracts and adapters.
2. dual-run / feature-flag stage
   - shadow scheduling/failover decisions and compare outcomes.
3. cutover
   - enable cluster-owned placement paths.
4. cleanup
   - remove legacy local-only decision paths.

## Rollback Plan

- trigger conditions:
  - ownership conflicts, elevated failover errors, or severe latency regressions.
- rollback steps:
  - disable cluster placement flag and revert to prior scheduling path.
- data/state reconciliation:
  - reconcile ownership records and replay pending operations safely.

## Validation Checklist

- API compatibility checks:
  - runtime/execution/api/cli compile and contract test pass.
- integration checks:
  - placement/failover/rebalance behavior under failure injection.
- performance checks:
  - control-plane latency/throughput vs baseline.
