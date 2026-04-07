# API

## Public Surface

- stable APIs:
  - `MemoryError`, `MemoryResult`, `init()`, `shutdown()`, `prelude`
  - allocator traits and common allocators
  - `pool::ObjectPool`, `cache::ComputeCache`, `budget::MemoryBudget`
- experimental APIs:
  - some `monitoring`, `profiling`, and async support surfaces are still evolving.
- hidden/internal APIs:
  - sealed internals and low-level implementation details in internal modules.

## Usage Patterns

- reuse-first: combine pools/caches with bounded budgets.
- short-lived allocations: arena and bump-based workflows.
- high-throughput shared paths: thread-safe pool/cache variants.

## Minimal Example

```rust
use nebula_memory::prelude::*;

fn main() -> MemoryResult<()> {
    nebula_memory::init()?;

    let mut pool = ObjectPool::new(32, String::new);
    let _item = pool.get().ok_or_else(|| MemoryError::pool_exhausted("strings", 32))?;

    nebula_memory::shutdown()?;
    Ok(())
}
```

## Advanced Example

```rust
use nebula_memory::budget::{BudgetConfig, MemoryBudget};
use nebula_memory::cache::{CacheConfig, ComputeCache};
use nebula_memory::pool::ThreadSafePool;

let budget = MemoryBudget::new(BudgetConfig::new("workflow-a", 64 * 1024 * 1024));
let cache: ComputeCache<String, Vec<u8>> = ComputeCache::new(CacheConfig::default());
let pool = ThreadSafePool::new(128, || Vec::<u8>::with_capacity(4096));

let _ = (budget, cache, pool);
```

## Error Semantics

- retryable errors:
  - `PoolExhausted`, `ArenaExhausted`, `CacheOverflow`, `BudgetExceeded`, `CacheMiss`.
- fatal errors:
  - `Corruption`, invalid operation/state classes, initialization failures.
- validation errors:
  - layout/config/alignment/size errors should fail fast.

## Compatibility Rules

- what changes require major version bump:
  - meaning of `MemoryError` variants
  - behavior guarantees of core allocator/pool/cache/budget contracts
  - default feature behavior that changes runtime semantics
- deprecation policy:
  - one minor release minimum for non-critical removals.
