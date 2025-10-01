# nebula-memory Features Analysis

## Current State

### Declared in Cargo.toml
```toml
default = ["std"]
std, alloc
pool, arena, cache, stats, budget, streaming, logging
numa-aware, linux-optimizations, monitoring
full
```

### Used in Code but NOT Declared
- **profiling** - 64 uses (Performance profiling)
- **adaptive** - 59 uses (Adaptive cache policies)
- **compression** - 8 uses (Memory compression)
- **nightly** - 4 uses (Nightly-only features)
- **backtrace** - 1 use (Error backtraces)
- **async** - 2 uses (Async support)
- **zstd**, **snappy**, **lz4** - Compression algorithms
- **custom**, **custom-compression** - Custom extensions

## Problems

1. **Missing feature declarations** causing `unexpected_cfgs` warnings
2. **Inconsistent feature dependencies** (e.g., budget requires stats but not declared)
3. **No feature documentation** - users don't know what features do
4. **Overly granular features** - too many small features
5. **monitoring** feature exists but poorly integrated

## Proposed Feature Structure

### Tier 1: Core (Always Available)
- `alloc` - No-std allocation support
- `std` - Standard library support (default)

### Tier 2: Allocators
- `allocators-all` - All allocator types
  - `allocator-bump` - Bump allocator
  - `allocator-pool` - Pool allocator
  - `allocator-stack` - Stack allocator
  - `allocator-arena` - Arena allocators

### Tier 3: Advanced Features
- `caching` - Multi-level caching system
- `streaming` - Streaming data optimizations
- `compression` - Memory compression
  - `compression-lz4` - LZ4 algorithm
  - `compression-zstd` - Zstandard algorithm
  - `compression-snappy` - Snappy algorithm

### Tier 4: Observability
- `monitoring` - System memory monitoring
- `statistics` - Detailed statistics tracking
- `profiling` - Performance profiling
- `tracing` - Integration with tracing ecosystem

### Tier 5: Platform-Specific
- `numa` - NUMA-aware allocation
- `linux-optimizations` - Linux-specific optimizations
- `nightly` - Nightly-only features

### Tier 6: Integrations
- `logging` - nebula-log integration
- `async` - Async allocator support

### Convenience Features
- `default` - Reasonable defaults for most users
- `full` - Everything except nightly
- `production` - Optimized for production
- `development` - Everything including profiling/tracing

## Recommended Default Configuration

```toml
default = ["std", "allocators-basic", "statistics"]
allocators-basic = ["allocator-bump", "allocator-pool", "allocator-stack"]
production = ["std", "allocators-all", "statistics", "monitoring"]
development = ["full", "profiling", "tracing"]
```

## Migration Plan

1. Add missing features to Cargo.toml
2. Consolidate related features (e.g., all compression into one)
3. Update feature documentation
4. Add feature-specific tests
5. Document feature combinations in README
