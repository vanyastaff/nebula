# Async Support (WIP)

⚠️ **Work in Progress** - This module is under active development.

## Current Status

- ✅ Module structure created
- ✅ AsyncPool implemented with semaphore-based backpressure
- ⏳ AsyncArena has lifetime issues with returned references
- ⏳ AsyncCache not yet implemented

## Known Issues

1. **Lifetime Problems**: AsyncArena cannot safely return references from async functions
   due to Rust lifetime restrictions. Possible solutions:
   - Return owned values instead of references
   - Use Pin/Box for stable addresses
   - Redesign API to use indices instead of references

2. **Testing**: Requires tokio for testing (add to dev-dependencies)

## Next Steps

- [ ] Resolve AsyncArena lifetime issues
- [ ] Add AsyncCache
- [ ] Add comprehensive async tests
- [ ] Add async examples
- [ ] Performance benchmarks

## Usage (when complete)

```rust
use nebula_memory::async_support::AsyncPool;

#[tokio::main]
async fn main() {
    let pool = AsyncPool::new(10, || String::new());
    let obj = pool.acquire().await.unwrap();
    // obj returns to pool when dropped
}
```
