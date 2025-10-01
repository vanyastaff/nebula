# nebula-memory Module Reorganization Plan

## Current Structure Problems

1. **Too many top-level modules** (allocator, arena, budget, cache, pool, stats, etc.)
2. **Inconsistent module sizes** (pool.rs is 800+ lines, bump.rs is 700+ lines)
3. **No clear separation of concerns** (everything at same level)
4. **Duplicate functionality** (stats in multiple places)
5. **Poor discoverability** (hard to find what you need)

## Proposed New Structure

```
src/
├── lib.rs                    # Public API and prelude
│
├── core/                     # Core types and traits (DONE)
│   ├── mod.rs
│   ├── error.rs             # Error types
│   ├── traits.rs            # Core traits
│   ├── types.rs             # Common types
│   └── config.rs            # Configuration types
│
├── allocators/              # All allocator implementations
│   ├── mod.rs               # Re-exports and common code
│   │
│   ├── bump/                # Bump allocator
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   ├── cursor.rs        # AtomicCursor/CellCursor
│   │   └── checkpoint.rs    # BumpCheckpoint/BumpScope
│   │
│   ├── pool/                # Pool allocator
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   ├── block.rs         # Block management
│   │   └── pool_box.rs      # PoolBox type
│   │
│   ├── stack/               # Stack allocator
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   ├── frame.rs         # StackFrame type
│   │   └── marker.rs        # StackMarker
│   │
│   ├── system.rs            # System allocator wrapper
│   │
│   └── common/              # Shared allocator utilities
│       ├── mod.rs
│       ├── config.rs        # Common config traits
│       ├── debug.rs         # Debug fill patterns
│       └── stats.rs         # Common stats types
│
├── specialized/             # Specialized allocators
│   ├── mod.rs
│   │
│   ├── arena/               # Arena allocators
│   │   ├── mod.rs
│   │   ├── arena.rs         # Basic arena
│   │   ├── typed.rs         # TypedArena
│   │   ├── thread_safe.rs   # ThreadSafeArena
│   │   └── streaming.rs     # StreamingArena
│   │
│   └── cache/               # Caching system
│       ├── mod.rs
│       ├── config.rs
│       ├── multi_level.rs
│       ├── partitioned.rs
│       └── policies/
│           ├── mod.rs
│           ├── lru.rs
│           ├── lfu.rs
│           └── arc.rs
│
├── tracking/                # Monitoring and tracking
│   ├── mod.rs
│   ├── monitored.rs         # MonitoredAllocator wrapper
│   ├── tracked.rs           # TrackedAllocator wrapper
│   ├── stats.rs             # Statistics collection
│   └── profiling.rs         # Performance profiling
│
├── management/              # Resource management
│   ├── mod.rs
│   ├── manager.rs           # AllocatorManager
│   ├── budget.rs            # MemoryBudget
│   └── registry.rs          # Global registry
│
├── platform/                # Platform-specific code
│   ├── mod.rs
│   ├── syscalls.rs          # System calls
│   ├── numa.rs              # NUMA support
│   └── monitoring.rs        # System monitoring
│
├── compression/             # Compression support
│   ├── mod.rs
│   ├── lz4.rs
│   ├── zstd.rs
│   └── snappy.rs
│
└── utils/                   # Utilities
    ├── mod.rs
    ├── alignment.rs         # Alignment helpers
    ├── arithmetic.rs        # CheckedArithmetic
    ├── barriers.rs          # Memory barriers
    └── formatting.rs        # Size formatting
```

## Migration Strategy

### Phase 1: Create new structure (non-breaking)
1. Create new module directories
2. Copy files to new locations
3. Update internal imports
4. Keep old structure as re-exports

### Phase 2: Update public API (breaking)
1. Update prelude with new paths
2. Deprecate old paths
3. Update documentation
4. Update examples

### Phase 3: Cleanup
1. Remove old module structure
2. Remove deprecated items
3. Update tests

## Key Principles

1. **Clear hierarchy**: core → allocators → specialized → tracking → management
2. **Consistent sizing**: Keep modules under 300-400 lines
3. **Feature gating**: One module per feature where possible
4. **Discoverability**: Logical grouping and clear names
5. **Backward compatibility**: Use re-exports during migration

## File Size Targets

- **Small modules**: < 200 lines (config, types)
- **Medium modules**: 200-400 lines (most implementations)
- **Large modules**: 400-600 lines (complex allocators)
- **Max size**: 600 lines (needs splitting if larger)

## Benefits

1. **Better organization**: Logical grouping by functionality
2. **Easier navigation**: Clear hierarchy
3. **Better compilation**: Smaller modules compile faster
4. **Easier testing**: Focused unit tests
5. **Clearer dependencies**: Explicit import paths
6. **Better feature gating**: One feature per module group

## Implementation Order

1. ✅ Fix features in Cargo.toml
2. Create `allocators/` directory structure
3. Split large files (bump, pool, stack)
4. Move arena to `specialized/arena/`
5. Move cache to `specialized/cache/`
6. Create `tracking/` for monitoring
7. Create `management/` for manager/budget
8. Update `lib.rs` and prelude
9. Update all examples
10. Update tests
11. Update documentation
