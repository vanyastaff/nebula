---
title: "[MEDIUM] Re-enable and complete disabled modules across codebase"
labels: feature, medium-priority, technical-debt
assignees:
milestone: Sprint 6
---

## Problem

Multiple modules and features are currently disabled throughout the codebase with `//` comments. These represent incomplete functionality that should either be completed or removed.

## Affected Modules

### nebula-derive (3 modules)
**Location:** `crates/nebula-derive/src/lib.rs:52-54`
```rust
// mod parameter;
// mod action;
// mod resource;
```
**Status:** Derive macros not yet implemented for these types

### nebula-memory (6 modules)
**Compression modules:** `crates/nebula-memory/src/compression/mod.rs:12-14`
```rust
// mod arena;
// mod cache;
```

**NUMA support:** `crates/nebula-memory/src/syscalls/mod.rs:27`
```rust
// mod numa;
```

**Streaming:** `crates/nebula-memory/src/lib.rs:104`
```rust
// pub mod streaming;
```

**LFU cache policy:** `crates/nebula-memory/src/cache/policies/mod.rs:4`
```rust
// pub mod lfu;  // TODO: Fix lfu module after migration
```

### nebula-parameter (2 modules)
**Display system:** `crates/nebula-parameter/src/core/mod.rs:9`
```rust
// mod display;  // TODO: Temporarily disabled, needs rewrite
```

**Credential type:** `crates/nebula-parameter/src/types/mod.rs:56`
```rust
// pub mod credential;
```

## Impact

🟡 **MEDIUM Priority** - Incomplete features reduce framework capabilities

**Consequences:**
- Feature gaps in public API
- Reduced functionality vs. documented capabilities
- Potential confusion for users expecting these features
- Code maintenance burden (commented code bitrot)

## Action Items

### Phase 1: Assessment
- [ ] Evaluate each disabled module's necessity
- [ ] Check if features still align with project goals
- [ ] Document reasons for disabling each module
- [ ] Decide: Complete, Remove, or Keep Disabled

### Phase 2: High-Value Modules (Complete)
- [ ] **nebula-parameter/display** (🔴 HIGH - already tracked in Issue #1)
- [ ] **nebula-memory/lfu** (🔴 HIGH - already tracked in Issue #2)
- [ ] **nebula-derive macros** (parameter, action, resource)
  - [ ] Design macro API
  - [ ] Implement code generation
  - [ ] Add tests and documentation

### Phase 3: Medium-Value Modules (Complete or Document)
- [ ] **nebula-memory/compression** (arena, cache)
  - [ ] Complete compression arena implementation
  - [ ] Add compression cache support
  - [ ] Benchmark compression benefits
- [ ] **nebula-memory/numa** NUMA-aware allocations
  - [ ] Research NUMA API requirements
  - [ ] Implement or document as "won't implement"
- [ ] **nebula-parameter/credential** type
  - [ ] Design credential parameter API
  - [ ] Integrate with nebula-credential

### Phase 4: Low-Priority (Remove or Keep)
- [ ] **nebula-memory/streaming** - Evaluate if needed
  - [ ] Document use case or remove module stub

### Phase 5: Cleanup
- [ ] Remove commented module declarations
- [ ] Add clear feature flags for optional modules
- [ ] Update documentation to reflect available features

## Design Considerations

### Feature Flags
```toml
[features]
default = []
derive-macros = []  # Enable parameter/action/resource derives
compression = ["lz4"]  # Compression support
numa = []  # NUMA-aware allocations
```

### Documentation
- Add feature matrix showing enabled/disabled features
- Document why certain features are optional
- Provide migration guide for users

## Files Affected

```
crates/nebula-derive/src/lib.rs
crates/nebula-derive/src/parameter.rs (create)
crates/nebula-derive/src/action.rs (create)
crates/nebula-derive/src/resource.rs (create)
crates/nebula-memory/src/compression/arena.rs
crates/nebula-memory/src/compression/cache.rs
crates/nebula-memory/src/syscalls/numa.rs
crates/nebula-memory/src/lib.rs
crates/nebula-memory/src/cache/policies/lfu.rs
crates/nebula-parameter/src/core/mod.rs
crates/nebula-parameter/src/core/display.rs
crates/nebula-parameter/src/types/credential.rs (create)
```

## Priority Breakdown

| Module | Priority | Reason |
|--------|----------|--------|
| nebula-parameter/display | 🔴 HIGH | Already tracked (Issue #1) |
| nebula-memory/lfu | 🔴 HIGH | Already tracked (Issue #2) |
| nebula-derive macros | 🟡 MEDIUM | High value, but not blocking |
| nebula-memory/compression | 🟡 MEDIUM | Performance optimization |
| nebula-parameter/credential | 🟡 MEDIUM | Integration with auth system |
| nebula-memory/numa | 🟢 LOW | Niche use case |
| nebula-memory/streaming | 🟢 LOW | Unclear requirements |

## References

- Technical Debt Tracker: [docs/TECHNICAL_DEBT.md](../TECHNICAL_DEBT.md)
- Related Issues: #1 (Display System), #2 (Cache Policies)

## Acceptance Criteria

- [ ] All modules either completed or removed (no commented stubs)
- [ ] Feature flags properly configured
- [ ] Documentation updated to reflect actual features
- [ ] Tests added for enabled features
- [ ] Migration guide written if any features removed
- [ ] Public API stable and documented

## Timeline

- **Sprint 6**: Assessment and high-value modules
- **Sprint 7**: Medium-value modules
- **Sprint 8**: Cleanup and documentation
