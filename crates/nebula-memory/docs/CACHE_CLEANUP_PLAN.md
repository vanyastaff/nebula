# Cache Module Cleanup Plan

**Goal**: Reduce complexity, remove duplicates, keep only what's needed for n8n alternative

## Current State

**Total Lines**: 10,250
**Files**: 18 files

### File Sizes
| File | Lines | Status | Action |
|------|-------|--------|--------|
| policies/lru.rs | 1248 | BLOATED | ⚠️ Simplify (remove 5 strategies, keep Classic only) |
| multi_level.rs | 1180 | COMPLEX | ❓ Evaluate if needed |
| stats.rs | 1107 | BLOATED | ⚠️ Simplify (too many metrics) |
| policies/arc.rs | 1092 | ADVANCED | ❌ Remove (complex, rarely used) |
| scheduled.rs | 1022 | MEDIUM | ❓ Keep minimal version |
| async_compute.rs | 1020 | GOD OBJECT | ✅ Already replaced by simple.rs |
| partitioned.rs | 957 | COMPLEX | ❌ Remove (premature optimization) |
| policies/lfu.rs | 909 | ADVANCED | ❌ Remove (LRU is enough) |
| compute.rs | 795 | CORE | ✅ Keep (core logic) |
| policies/adaptive.rs | 605 | COMPLEX | ❌ Remove (300% overhead) |
| config.rs | 575 | CORE | ⚠️ Simplify |
| **simple.rs** | 484 | NEW | ✅ Keep (our new simple cache) |
| policies/fifo.rs | 337 | SIMPLE | ✅ Keep |
| policies/ttl.rs | 269 | USEFUL | ✅ Keep |
| concurrent.rs | 257 | UTILITY | ✅ Keep |
| policies/random.rs | 238 | SIMPLE | ✅ Keep (good for testing) |
| policies/mod.rs | 105 | CORE | ✅ Keep |
| mod.rs | 52 | CORE | ✅ Keep |

## Analysis by Use Case (n8n Alternative)

### ✅ **Essential** (Keep)
1. **simple.rs** (484 lines) - Our new lightweight cache
2. **compute.rs** (795 lines) - Core get-or-compute logic
3. **config.rs** (575 lines) - Configuration (needs simplification)
4. **policies/fifo.rs** (337 lines) - Simple eviction
5. **policies/random.rs** (238 lines) - Useful for testing
6. **policies/ttl.rs** (269 lines) - Time-based expiration (useful for workflows)
7. **concurrent.rs** (257 lines) - Thread-safe access
8. **policies/mod.rs** (105 lines) - Policy traits
9. **mod.rs** (52 lines) - Module exports

**Subtotal**: ~3,112 lines

### ⚠️ **Simplify** (Reduce complexity)
10. **policies/lru.rs** (1248 → ~300 lines)
    - Remove: Segmented, Clock, Adaptive, MultiQueue strategies
    - Keep: Classic LRU only
    - Remove: aging, protection_ratio, temporal locality detection
    - **Reduction**: ~950 lines

11. **stats.rs** (1107 → ~400 lines)
    - Remove: Complex profiling, histograms, percentiles
    - Keep: Basic counters (hits, misses, size)
    - **Reduction**: ~700 lines

12. **scheduled.rs** (1022 → ~300 lines)
    - Remove: Complex scheduling logic
    - Keep: Simple TTL cleanup task
    - **Reduction**: ~700 lines

**Subtotal after simplification**: ~1,000 lines (saved ~2,350)

### ❌ **Remove** (Not needed)
13. **async_compute.rs** (1020 lines) - God Object, replaced by simple.rs
14. **policies/arc.rs** (1092 lines) - Too complex, LRU is sufficient
15. **policies/lfu.rs** (909 lines) - LRU is enough for most cases
16. **policies/adaptive.rs** (605 lines) - 300% overhead, not worth it
17. **partitioned.rs** (957 lines) - Premature optimization
18. **multi_level.rs** (1180 lines) - Over-engineered

**Subtotal**: 5,763 lines removed

### ❓ **Evaluate** (Decide later)
- None - we know what we need

## Target Architecture

```
cache/
├── mod.rs                    (52 lines) ✅
├── simple.rs                 (484 lines) ✅ NEW - Simple async cache
├── compute.rs                (795 lines) ✅ Core logic
├── config.rs                 (~400 lines) ⚠️ Simplified
├── stats.rs                  (~400 lines) ⚠️ Simplified
├── scheduled.rs              (~300 lines) ⚠️ Simplified
├── concurrent.rs             (257 lines) ✅
└── policies/
    ├── mod.rs                (105 lines) ✅
    ├── lru.rs                (~300 lines) ⚠️ Classic only
    ├── fifo.rs               (337 lines) ✅
    ├── random.rs             (238 lines) ✅
    └── ttl.rs                (269 lines) ✅
```

**Target Total**: ~3,937 lines (from 10,250)
**Reduction**: 61% smaller!

## Implementation Plan

### Phase 1: Remove Unnecessary Files ❌
- [ ] Delete `async_compute.rs` (replaced by simple.rs)
- [ ] Delete `policies/arc.rs`
- [ ] Delete `policies/lfu.rs`
- [ ] Delete `policies/adaptive.rs`
- [ ] Delete `partitioned.rs`
- [ ] Delete `multi_level.rs`
- [ ] Update exports in `mod.rs`
- [ ] Update lib.rs exports

**Impact**: -5,763 lines immediately

### Phase 2: Simplify LRU ⚠️
- [ ] Remove `LruStrategy` enum (keep Classic only)
- [ ] Remove `LruConfig` complex fields
- [ ] Remove segmented/clock/adaptive implementations
- [ ] Keep simple doubly-linked list
- [ ] Update tests

**Impact**: -950 lines

### Phase 3: Simplify Stats ⚠️
- [ ] Remove profiling infrastructure
- [ ] Remove histograms and percentiles
- [ ] Keep basic AtomicU64 counters
- [ ] Remove `AccessPattern`, `SizeDistribution`

**Impact**: -700 lines

### Phase 4: Simplify Scheduled ⚠️
- [ ] Remove complex task scheduling
- [ ] Keep simple TTL cleanup
- [ ] Remove background refresh logic

**Impact**: -700 lines

### Phase 5: Simplify Config ⚠️
- [ ] Remove unused config options
- [ ] Merge related configs
- [ ] Simplify validation

**Impact**: -175 lines

## Expected Results

### Before
```
cache/: 10,250 lines
- 6 eviction policies (LRU, LFU, ARC, FIFO, Random, TTL, Adaptive)
- 5 LRU strategies
- Complex async compute with God Object
- Multi-level caching
- Partitioned caching
- Advanced stats with profiling
```

### After
```
cache/: ~3,937 lines (61% reduction)
- 4 eviction policies (LRU Classic, FIFO, Random, TTL)
- Simple async cache (simple.rs)
- Core compute cache
- Basic stats
- Simple TTL scheduling
```

## Benefits for n8n Alternative

1. **✅ Easier to Understand**
   - 61% less code to read
   - Clear, simple implementations
   - No over-engineering

2. **✅ Faster Compilation**
   - Less template instantiation
   - Fewer dependencies
   - Smaller binary

3. **✅ Better Performance**
   - No overhead from unused features
   - Simpler code = better optimization
   - Less indirection

4. **✅ Easier Maintenance**
   - Less code to maintain
   - Fewer bugs
   - Clearer intent

5. **✅ Still Feature-Complete**
   - simple.rs for basic caching
   - compute.rs for get-or-compute
   - LRU/FIFO/Random/TTL policies
   - TTL scheduling for expiration
   - Concurrent access support

## What We're NOT Losing

- ❌ Complex LRU strategies → Don't need them
- ❌ LFU, ARC policies → LRU is sufficient
- ❌ Adaptive policy → 300% overhead not worth it
- ❌ Multi-level caching → Can build if needed later
- ❌ Partitioned caching → Premature optimization
- ❌ Advanced profiling → Basic stats are enough

## Validation

After cleanup, ensure:
- [ ] `cargo check --all-features` passes
- [ ] Core tests pass
- [ ] simple.rs works as expected
- [ ] Eviction policies work correctly
- [ ] No broken imports in other crates

## Timeline

- Phase 1 (Remove): 30 minutes
- Phase 2 (LRU): 1 hour
- Phase 3 (Stats): 1 hour
- Phase 4 (Scheduled): 30 minutes
- Phase 5 (Config): 30 minutes
- Testing: 1 hour

**Total**: ~5 hours

## Success Metrics

| Metric | Before | Target | Improvement |
|--------|--------|--------|-------------|
| Total lines | 10,250 | 3,937 | -61% |
| Policy count | 7 | 4 | -43% |
| LRU strategies | 5 | 1 | -80% |
| Complexity (files) | 18 | 13 | -28% |
| Compilation time | ? | Faster | TBD |

---

**Status**: Ready to execute
**Priority**: HIGH - Simplification is critical for maintainability
