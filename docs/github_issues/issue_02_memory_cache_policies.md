---
title: "[HIGH] Fix nebula-memory cache policy integration issues"
labels: bug, high-priority, nebula-memory
assignees:
milestone: Sprint 5
---

## Problem

Multiple cache policy integration issues exist with type mismatches in fallback policies and disabled LFU module. This reduces cache policy flexibility and may cause runtime errors.

## Current State

**Type Mismatch Issues:**
- `crates/nebula-memory/src/cache/policies/ttl.rs:129` - Fallback policy type mismatch with CacheEntry<()>
- `crates/nebula-memory/src/cache/policies/ttl.rs:149` - Type mismatch in fallback integration
- `crates/nebula-memory/src/cache/policies/ttl.rs:239` - Fallback policy support disabled

**Missing Functionality:**
- `crates/nebula-memory/src/cache/policies/ttl.rs:255` - clear() method not implemented
- `crates/nebula-memory/src/cache/policies/mod.rs:3` - LFU module needs fixing after migration

## Impact

🔴 **HIGH Priority** - Reduced cache policy flexibility and potential runtime errors

## Action Items

- [ ] Fix fallback policy type mismatch with CacheEntry<()>
- [ ] Resolve type issues in fallback integration
- [ ] Fix LFU module after migration
- [ ] Re-enable fallback policy support
- [ ] Implement TTL cache clear() method
- [ ] Add integration tests for all cache policies
- [ ] Test fallback policy chains

## Files Affected

```
crates/nebula-memory/src/cache/policies/ttl.rs
crates/nebula-memory/src/cache/policies/mod.rs
crates/nebula-memory/src/cache/policies/lfu.rs (disabled)
```

## Technical Details

The main issue is a type mismatch between:
- Cache policy expecting `CacheEntry<V>`
- Fallback policy providing `CacheEntry<()>`

This needs generic type parameter alignment or adapter pattern implementation.

## References

- Technical Debt Tracker: [docs/TECHNICAL_DEBT.md](../TECHNICAL_DEBT.md#2-nebula-memory-cache-policy-integration-issues)

## Acceptance Criteria

- [ ] All cache policies compile without warnings
- [ ] Fallback policy integration works correctly
- [ ] LFU module functional and tested
- [ ] clear() method implemented for TTL cache
- [ ] Integration tests pass
- [ ] Type safety maintained throughout policy chain
