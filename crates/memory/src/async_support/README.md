# Async Support

This module provides async/await compatible memory management primitives.

## Current Status

- ✅ AsyncPool - Fully functional with semaphore-based backpressure
- ⚠️  AsyncArena - Single-threaded only (see limitations below)
- ⏳ AsyncCache - Not yet implemented

## Features

### AsyncPool

Thread-safe async object pool with backpressure control:

```rust
use nebula_memory::async_support::AsyncPool;

#[tokio::main]
async fn main() {
    let pool = AsyncPool::new(10, || String::new());
    let obj = pool.acquire().await.unwrap();
    // obj returns to pool when dropped
}
```

### AsyncArena (Single-threaded)

Handle-based arena for async contexts (single-threaded runtime only):

```rust
use nebula_memory::async_support::AsyncArena;

#[tokio::main]
async fn main() {
    let arena = AsyncArena::new();
    let handle = arena.alloc(42).await.unwrap();

    // Read value
    let value = handle.read(|v| *v).await;

    // Modify value
    handle.modify(|v| *v = 100).await;
}
```

## Known Limitations

1. **AsyncArena is !Send**: The underlying Arena type uses `Cell` for interior mutability,
   which is !Send. This means AsyncArena can only be used in single-threaded async runtimes
   or with `LocalSet` in Tokio. For multi-threaded async contexts, use AsyncPool instead.

2. **Handle API**: AsyncArena returns handles with closure-based access to avoid lifetime
   issues with async functions. This is more verbose but safer than raw references.

3. **No AsyncCache**: Cache async support is not yet implemented.

## Testing Notes

- AsyncPool tests pass with multi-threaded runtime
- AsyncArena tests require `#[tokio::test(flavor = "current_thread")]`
- Integration with tokio LocalSet is recommended for AsyncArena

## Next Steps

- [ ] Implement Send-safe AsyncArena using different backend
- [ ] Add AsyncCache
- [ ] Add async examples
- [ ] Performance benchmarks
