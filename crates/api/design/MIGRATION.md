# Migration

## Versioning Policy

- **Compatibility promise:** /health, /api/v1/status paths and StatusResponse schema stable.
- **Deprecation window:** Minimum 2 minor releases.

## Breaking Changes

### None Planned

Current API is minimal. Phase 2 (workflow/execution routes) is additive.

### Potential Future Breaking Changes

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| StatusResponse schema | workers, webhook | Add fields | Additive; clients ignore unknown |
| Route path change | /api/v1/status | /api/v2/status | Version both; deprecate v1 |
| ApiState extension | webhook, workers | + engine, storage | Add optional fields; default None |
| Auth required | All public | Protected routes | Add auth middleware; 401 for missing |

## Rollout Plan

1. **Preparation:** Add workflow/execution routes; extend ApiState.
2. **Dual-run:** Feature flag for new routes; default off.
3. **Cutover:** Enable; update clients.
4. **Cleanup:** Remove flag.

## Rollback Plan

- **Trigger conditions:** Route regression; auth breaks clients.
- **Rollback steps:** Revert deployment; disable feature flag.
- **Data/state reconciliation:** N/A; API is stateless.

## Validation Checklist

- [ ] /health returns 200
- [ ] /api/v1/status returns valid JSON
- [ ] Webhook routes work when merged
- [ ] run() error handling
- [ ] No new clippy warnings
