# Migration

## Versioning Policy

- **Compatibility promise:** Metric names stable; Prometheus format compatible.
- **Deprecation window:** 2 minor releases for metric name changes.

## Breaking Changes

None until metrics crate or export is implemented. Future changes:

### Example: Metric Name Rename

- **Old behavior:** `actions_executed_total`
- **New behavior:** `nebula_action_executions_total`
- **Migration steps:**
  1. Add new metric name; keep old for 2 minors.
  2. Update dashboards/alerting to use new name.
  3. Remove old metric.

## Rollout Plan

1. **Preparation:** Implement export; document metric names.
2. **Dual-run:** Optional; run old and new export in parallel.
3. **Cutover:** Enable scrape; verify Grafana.
4. **Cleanup:** Remove deprecated metrics.

## Rollback Plan

- **Trigger conditions:** Export causes latency; scrape failures.
- **Rollback steps:** Disable export; revert to no-scrape.
- **Data/state reconciliation:** N/A; metrics ephemeral.

## Validation Checklist

- [ ] Prometheus scrape returns valid format
- [ ] Grafana dashboards work
- [ ] No recording latency regression
- [ ] Alert rules evaluated correctly
