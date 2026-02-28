# Migration

## Versioning Policy

- **compatibility promise:** Patch/minor preserve `StorageProvider`, `CredentialProvider`, `CredentialContext`, error variants, protocol types
- **deprecation window:** Minimum 6 months before removal of deprecated APIs (unless security-critical)

## Breaking Changes

- **currently planned:** None committed
- **potential future breaking candidates:**
  - Provider capability negotiation (P-001) — startup validation changes
  - Strict scope enforcement mode (P-002) — operations without scope may fail
  - Rotation policy versioning (P-003) — schema envelope changes

## Rollout Plan

1. **preparation:** Introduce new APIs additively; document migration path
2. **dual-run / feature-flag stage:** Allow old and new behavior side-by-side where possible (e.g. strict scope mode)
3. **cutover:** Switch defaults only in major release
4. **cleanup:** Remove deprecated path after migration window

## Rollback Plan

- **trigger conditions:** Consumer breakage in provider/manager contracts; rotation state corruption
- **rollback steps:** Revert to previous stable version; restore encryption key if rotated
- **data/state reconciliation:** Ensure persisted credentials remain decryptable; rotation state recoverable

## Validation Checklist

- **API compatibility checks:** Compile-time checks for `StorageProvider`, `CredentialProvider`, `CredentialManager` signatures
- **integration checks:** Action/resource fixtures; provider implementations
- **performance checks:** Benchmark comparison; cache hit rate preserved
