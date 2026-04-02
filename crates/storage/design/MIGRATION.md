# Migration

## Versioning Policy

- **Compatibility promise:** Storage trait additive-only; new methods optional.
- **Deprecation window:** Minimum 2 minor releases.

## Breaking Changes

### None Planned

Current API is minimal and stable. Phase 2 (list, backends) is additive.

### Potential Future Breaking Changes

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| Add list_prefix to trait | N/A | New method | Default impl returns empty; backends implement |
| Key type change | String | WorkflowId | Consumer migrates key format |
| Value type change | Vec<u8> | Bytes | Newtype or migration |

## Rollout Plan

1. **Preparation:** Implement PostgresStorage; add tests.
2. **Dual-run:** Feature flag for postgres backend; memory default.
3. **Cutover:** Enable postgres for production; configure connection.
4. **Cleanup:** Remove feature flag if redundant.

## Rollback Plan

- **Trigger conditions:** Postgres backend failures; data corruption.
- **Rollback steps:** Switch back to MemoryStorage; or fix Postgres.
- **Data/state reconciliation:** Postgres is source of truth; no rollback of data.

## Validation Checklist

- [ ] MemoryStorage tests pass
- [ ] MemoryStorageTyped tests pass
- [ ] (Future) PostgresStorage integration tests
- [ ] (Future) RedisStorage, S3Storage tests
- [ ] No new clippy warnings
