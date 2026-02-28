# Migration

## Versioning Policy

- **Compatibility promise:** Key format `{execution_id}:{node_id}:{attempt}` stable; DuplicateIdempotencyKey preserved.
- **Deprecation window:** 2 minor releases for type moves or renames.

## Breaking Changes

None until idempotency crate extraction. Future changes:

### Example: Extract to nebula-idempotency

- **Old behavior:** `nebula_execution::{IdempotencyKey, IdempotencyManager}`
- **New behavior:** `nebula_idempotency::{IdempotencyKey, IdempotencyManager}`; execution re-exports or depends.
- **Migration steps:**
  1. Add nebula-idempotency crate; move types.
  2. Execution re-exports from idempotency for compatibility.
  3. Update consumers to use nebula-idempotency directly (optional).
  4. Deprecate execution re-exports; remove in next major.

### Example: Key Format Change

- **Old behavior:** `{execution_id}:{node_id}:{attempt}`
- **New behavior:** `v2:{execution_id}:{node_id}:{attempt}` (versioned)
- **Migration steps:**
  1. Support both formats in storage (read old, write new).
  2. Migrate existing keys or let TTL expire.
  3. Remove old format support.

## Rollout Plan

1. **Preparation:** Implement storage; document key format.
2. **Dual-run:** In-memory and storage both active; compare.
3. **Cutover:** Storage primary; in-memory fallback.
4. **Cleanup:** Remove in-memory-only path (optional).

## Rollback Plan

- **Trigger conditions:** Storage corruption; duplicate executions.
- **Rollback steps:** Disable persistent idempotency; in-memory only.
- **Data/state reconciliation:** Keys may be lost; accept duplicate risk during rollback.

## Validation Checklist

- [ ] Key format unchanged
- [ ] check_and_mark semantics preserved
- [ ] NodeAttempt serialization roundtrip
- [ ] DuplicateIdempotencyKey error handling
