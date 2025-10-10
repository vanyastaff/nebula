---
title: "[HIGH] Complete ResourceInstance test implementation with todo!() macros"
labels: bug, high-priority, nebula-resource, testing
assignees:
milestone: Sprint 5
---

## Problem

The test infrastructure in `nebula-resource` contains `todo!()` macros in critical trait implementations. This means tests will panic if these code paths are executed, making the test suite unreliable.

## Current State

**Location:** `crates/nebula-resource/src/manager/mod.rs:915-921`

```rust
impl ResourceInstance for TestInstance {
    fn instance_id(&self) -> Uuid {
        self.id
    }
    fn resource_id(&self) -> &ResourceId {
        todo!()  // ⚠️ Will panic if called
    }
    fn lifecycle_state(&self) -> LifecycleState {
        LifecycleState::Ready
    }
    fn context(&self) -> &ResourceContext {
        todo!()  // ⚠️ Will panic if called
    }
    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
    // ... more methods
}
```

## Impact

🔴 **HIGH Priority** - Test infrastructure is broken and unreliable

**Consequences:**
- Tests panic if they exercise these code paths
- Incomplete test coverage
- False confidence in test suite
- Integration tests may fail unexpectedly
- Difficult to add new tests

## Action Items

### Immediate Fix (Sprint 5)
- [ ] Implement `resource_id()` method
  - [ ] Store ResourceId in TestInstance struct
  - [ ] Return proper reference
- [ ] Implement `context()` method
  - [ ] Store ResourceContext in TestInstance
  - [ ] Return proper reference
- [ ] Review all other TestInstance methods
- [ ] Add tests that exercise all methods

### Extended Fixes
- [ ] Search for other `todo!()` in test code
  - [ ] `nebula-validator/upgrade/validator_arch.rs:324`
  - [ ] Other test infrastructure files
- [ ] Create proper test fixtures
  - [ ] ResourceId factory
  - [ ] ResourceContext factory
  - [ ] TestInstance builder pattern
- [ ] Add integration tests that cover all code paths

### Prevention
- [ ] Add CI check to fail on `todo!()` in test code
- [ ] Document test infrastructure patterns
- [ ] Create test helper module with complete implementations

## Technical Details

### Current TestInstance
```rust
struct TestInstance {
    id: Uuid,
    // Missing fields:
    // resource_id: ResourceId,
    // context: ResourceContext,
}
```

### Required Implementation
```rust
struct TestInstance {
    id: Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
}

impl TestInstance {
    fn new(resource_id: ResourceId) -> Self {
        Self {
            id: Uuid::new_v4(),
            resource_id,
            context: ResourceContext::new(),
        }
    }
}

impl ResourceInstance for TestInstance {
    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn context(&self) -> &ResourceContext {
        &self.context
    }
    // ... all other methods properly implemented
}
```

## Other todo!() Locations

### nebula-validator
**Location:** `crates/nebula-validator/upgrade/validator_arch.rs:324`
```rust
todo!("Conditional skip handling")
```
**Context:** Validator upgrade documentation
**Action:** Complete conditional skip documentation or remove

### nebula-action README
**Location:** `crates/nebula-action/README.md:49`
```rust
todo!()
```
**Context:** Example code in documentation
**Action:** Complete example or mark as placeholder

### nebula-resource README
**Location:** `crates/nebula-resource/README.md:54`
```rust
todo!()
```
**Context:** Example code in documentation
**Action:** Complete example or mark as placeholder

## Files Affected

**Critical (test infrastructure):**
- `crates/nebula-resource/src/manager/mod.rs`
- `crates/nebula-resource/tests/*.rs` (may need updates)

**Documentation:**
- `crates/nebula-validator/upgrade/validator_arch.rs`
- `crates/nebula-action/README.md`
- `crates/nebula-resource/README.md`
- `crates/nebula-resource/src/lib.rs` (examples)

## Test Coverage

### Before Fix
```rust
#[test]
fn test_instance_methods() {
    let instance = TestInstance::new();
    assert_eq!(instance.lifecycle_state(), LifecycleState::Ready);
    // Cannot test resource_id() or context() - they panic!
}
```

### After Fix
```rust
#[test]
fn test_instance_complete() {
    let resource_id = ResourceId::new("test", "1.0");
    let instance = TestInstance::new(resource_id);

    // All methods now testable
    assert_eq!(instance.resource_id().name, "test");
    assert_eq!(instance.lifecycle_state(), LifecycleState::Ready);
    assert!(instance.context().is_empty());
}
```

## CI Check

Add to CI pipeline:
```bash
# Fail if todo!() found in test code (outside doc examples)
! rg "todo!" crates/*/tests/ crates/*/src/ --type rust \
  --glob '!**/examples/**' \
  --glob '!**/README.md' \
  --glob '!**/upgrade/**'
```

## References

- Technical Debt Tracker: [docs/TECHNICAL_DEBT.md](../TECHNICAL_DEBT.md)
- Test Infrastructure Guidelines: [docs/testing.md](../testing.md)
- Related: Issue #6 (Dead Code Cleanup)

## Acceptance Criteria

- [ ] All `todo!()` removed from test infrastructure
- [ ] TestInstance fully implemented with all required fields
- [ ] Tests added that exercise all TestInstance methods
- [ ] Integration tests pass without panics
- [ ] Documentation examples completed
- [ ] CI check added to prevent future `todo!()` in tests
- [ ] Test helper module created with proper fixtures

## Timeline

**Sprint 5** (same as other HIGH priority items):
- Week 1: Fix TestInstance implementation
- Week 2: Complete documentation examples
- Week 3: Add CI checks and prevention measures
