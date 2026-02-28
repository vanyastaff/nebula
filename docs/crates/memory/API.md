# API

## Core imports

```rust
use nebula_memory::prelude::*;
```

## Object pool

```rust
use nebula_memory::pool::ObjectPool;

let mut pool = ObjectPool::new(32, String::new);
let value = pool.get().unwrap();
```

`PooledValue` returns to the pool on drop.

## Allocators

```rust
use nebula_memory::allocator::{Allocator, BumpAllocator};
use std::alloc::Layout;

let alloc = BumpAllocator::new(4096)?;
let layout = Layout::from_size_align(64, 8)?;
let ptr = unsafe { alloc.allocate(layout)? };
unsafe { alloc.deallocate(ptr.cast(), layout) };
```

## Cache

```rust
use nebula_memory::cache::{CacheConfig, ComputeCache};

let mut cache = ComputeCache::new(CacheConfig::default());
```

## Budget

```rust
use nebula_memory::budget::{BudgetConfig, MemoryBudget};

let budget = MemoryBudget::new(BudgetConfig::default());
```

## Lifecycle helpers

- `nebula_memory::init()` initializes global allocator manager.
- `nebula_memory::shutdown()` performs crate-level shutdown cleanup.

## Error model

Use `MemoryResult<T>` / `MemoryError` from crate root or `prelude`.
