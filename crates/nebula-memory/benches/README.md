# nebula-memory Benchmarks

Comprehensive benchmark suite for evaluating allocator performance.

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench -p nebula-memory

# Run specific benchmark suite
cargo bench -p nebula-memory --bench allocator_benchmarks
cargo bench -p nebula-memory --bench real_world_scenarios

# Run specific benchmark
cargo bench -p nebula-memory -- single_allocation

# Generate HTML reports
cargo bench -p nebula-memory -- --save-baseline my_baseline
```

## Benchmark Suites

### allocator_benchmarks.rs

Core allocator performance metrics:

- **single_allocation**: Individual alloc/dealloc cycles
  - Measures latency of single operations
  - Compares Bump, Pool, Stack, and System allocators

- **batch_allocations**: Batch operations (100 allocations)
  - Tests throughput for bulk operations
  - Evaluates batch efficiency

- **allocation_sizes**: Performance across different sizes (16B - 4KB)
  - Shows how allocators scale with size
  - Identifies optimal size ranges

- **allocation_patterns**: Different usage patterns
  - Sequential: Allocate all, then deallocate all
  - Interleaved: Allocate and deallocate in pairs
  - Tests cache locality and memory reuse

- **memory_reuse**: Efficiency of memory recycling
  - Pool allocator reuse performance
  - Bump allocator non-reuse baseline

- **large_allocations**: Performance with large blocks (1MB+)
  - Tests scalability
  - Measures overhead for large allocations

### real_world_scenarios.rs

Realistic usage patterns:

- **request_response**: Web server request handling
  - Bump with reset vs Pool with reuse
  - Simulates typical HTTP request lifecycle

- **temporary_buffers**: Parser/compiler temporary allocations
  - Common in text processing
  - Tests arena pattern efficiency

- **object_lifecycle**: Object creation/destruction
  - Simulates OOP patterns
  - Tests pool allocator with object reuse

- **arena_pattern**: Many small objects, bulk deallocation
  - Classic arena allocator use case
  - Shows reset() performance advantage

- **mixed_sizes**: Realistic mixed workload
  - Small, medium, and large allocations
  - Tests fragmentation resistance

- **high_frequency**: Stress test with 1000 allocations
  - Measures peak throughput
  - Identifies performance bottlenecks

## Expected Results

### BumpAllocator
- **Fastest** for sequential allocations
- **Best** when memory can be reset in bulk
- **Ideal** for request-scoped allocations
- No memory reuse (higher memory usage)

### PoolAllocator
- **Best** for fixed-size allocations
- **Excellent** memory reuse
- **Ideal** for object pools
- Small overhead per block

### StackAllocator
- **Good** for LIFO patterns
- **Fast** marker-based deallocation
- **Ideal** for nested scopes
- Requires strict LIFO discipline

### SystemAllocator
- **Baseline** for comparison
- Good general-purpose performance
- Higher overhead than specialized allocators

## Performance Tips

1. **Use BumpAllocator** for:
   - Request-scoped allocations
   - Temporary computations
   - When you can reset frequently

2. **Use PoolAllocator** for:
   - Long-lived object pools
   - Fixed-size allocations
   - High allocation/deallocation frequency

3. **Use StackAllocator** for:
   - Nested scopes
   - LIFO allocation patterns
   - When you need precise control

## Interpreting Results

- **Lower time** = Better performance
- **Higher throughput** = Better for bulk operations
- Compare against **system** baseline
- Look for **consistent** results (low variance)

## Hardware Impact

Results vary by:
- CPU cache size
- Memory bandwidth
- Core count (for concurrent benchmarks)
- OS memory allocator implementation

Always benchmark on your target hardware!
