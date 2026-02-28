# Migration

## Versioning Policy

- **Compatibility promise:** execute_action and ActionRegistry API stable; DataPassingPolicy additive.
- **Deprecation window:** Minimum 2 minor releases.

## Breaking Changes

### None Planned

Current API is stable. Phase 2 (isolation, SpillToBlob) is additive.

### Potential Future Breaking Changes

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| execute_action with ResourceProvider | No resources | Optional resource injection | Add param; default None |
| Isolation level required | All direct | Route by metadata | Add ActionMetadata.isolation_level |
| BlobRef in ActionResult | N/A | Output may be BlobRef | Consumers check output type |

## Rollout Plan

1. **Preparation:** Implement isolation routing; add tests.
2. **Dual-run:** Feature flag for isolation; default off.
3. **Cutover:** Enable by default; remove flag.
4. **Cleanup:** Update docs.

## Rollback Plan

- **Trigger conditions:** Regression in engine tests; isolation breaks plugins.
- **Rollback steps:** Revert to direct execution; disable feature flag.
- **Data/state reconciliation:** N/A; no persistent state.

## Validation Checklist

- [ ] Engine integration tests pass
- [ ] Resource integration tests pass
- [ ] Telemetry events correct
- [ ] Data limit enforcement works
- [ ] No new clippy warnings
