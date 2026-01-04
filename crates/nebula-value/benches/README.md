# nebula-value Benchmarks

This directory contains comprehensive performance benchmarks for nebula-value using [Criterion.rs](https://github.com/bheisler/criterion.rs).

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench --features serde

# Run specific benchmark group
cargo bench --bench nebula_value --features serde -- integer

# Generate HTML reports (in target/criterion/)
cargo bench --features serde
```

## Benchmark Coverage

### 1. Scalar Types

**Integer Operations** (`bench_integer`)
- Creation: `Integer::new()`
- Checked arithmetic: `checked_add()`, `checked_mul()`
- Comparison: `<`, `>`, `==`

**Float Operations** (`bench_float`)
- Creation: `Float::new()`
- Arithmetic: `+`, `*`, `/`
- Total ordering: `total_cmp()`

**Text Operations** (`bench_text`)
- Creation: `Text::new()` at 10, 100, 1000 bytes
- Cloning (Arc-based): O(1) clone via Arc<str>
- Concatenation: `concat()`

**Bytes Operations** (`bench_bytes`)
- Creation: `Bytes::new()` at 64B, 1KB, 64KB
- Cloning (bytes-based): O(1) via reference counting
- Slice operations: `slice()`

### 2. Collection Types

**Array Operations** (`bench_array`)
- Construction: `from_vec()` at 10, 100, 1000 elements
- Cloning: O(1) structural sharing via im::Vector
- Access: `get()` - O(log n)
- Mutation: `push()` - O(log n)
- Concatenation: `concat()` - O(log n)

**Object Operations** (`bench_object`)
- Construction: `from_iter()` at 10, 100, 1000 keys
- Cloning: O(1) structural sharing via im::HashMap
- Access: `get()` - O(log n)
- Insertion: `insert()` - O(log n)
- Merging: `merge()` - O(n log n)

### 3. Value Operations

**Arithmetic** (`bench_value_ops`)
- Integer addition: type-safe checked math
- Float addition: IEEE 754 operations
- Mixed type coercion: automatic Integer → Float promotion
- Text concatenation: efficient string building

**Comparison**
- Equality: `eq()` with proper NaN handling
- Ordering: `<`, `>` with total_cmp for floats

**Logical**
- Boolean operations: `and()`, `or()`, `not()`

**Cloning**
- Text cloning: O(1) Arc reference count increment
- Array cloning (1000 items): O(1) structural sharing
- Object cloning (1000 keys): O(1) structural sharing

### 4. Serialization (with `serde` feature)

**JSON Serialization** (`bench_serde`)
- Simple object serialization
- Array serialization (100 elements)
- Roundtrip: serialize → deserialize

**Special Value Handling**
- NaN → null
- Infinity → string representation
- Bytes → base64 encoding

## Performance Characteristics

### Expected Performance

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| Integer arithmetic | O(1) | Checked overflow |
| Float arithmetic | O(1) | IEEE 754 compliance |
| Text clone | O(1) | Arc reference counting |
| Text concat | O(n) | New allocation |
| Bytes clone | O(1) | bytes::Bytes refcount |
| Array get | O(log n) | im::Vector persistent |
| Array push | O(log n) | Structural sharing |
| Array clone | O(1) | Reference increment |
| Object get | O(log n) | im::HashMap persistent |
| Object insert | O(log n) | Structural sharing |
| Object clone | O(1) | Reference increment |
| JSON serialize | O(n) | Linear in data size |
| JSON deserialize | O(n) | Linear in JSON size |

### Zero-Copy Architecture

- **Text**: Uses `Arc<str>` - cloning increments ref count only
- **Bytes**: Uses `bytes::Bytes` - zero-copy slicing
- **Array**: Uses `im::Vector` - structural sharing on mutations
- **Object**: Uses `im::HashMap` - structural sharing on mutations

### Memory Efficiency

- Small value overhead: 16-24 bytes per Value enum variant
- Large string/bytes: Single allocation with multiple references
- Collections: Shared structure - modifications create minimal new nodes
- No deep cloning: All clones are O(1) reference operations

## Interpreting Results

Criterion generates detailed reports in `target/criterion/`:

- **Timing**: Mean time per operation with confidence intervals
- **Throughput**: Operations per second
- **Comparisons**: Automatic detection of performance regressions
- **HTML Reports**: Visual graphs of performance over time

### Key Metrics

- **Integer/Float ops**: Should be <10ns (CPU-bound)
- **Text/Bytes clone**: Should be <50ns (refcount increment)
- **Array/Object clone**: Should be <50ns (structural sharing)
- **Array/Object get**: Should be <100ns (O(log n) tree traversal)
- **JSON serialize/deserialize**: Dominated by serde_json performance

## Implementation Notes

### Persistent Data Structures

Collections use [im](https://github.com/bodil/im-rs) for persistent data structures:

- **im::Vector**: RRB-tree with O(log n) operations
- **im::HashMap**: HAMT with O(log n) operations
- **Structural Sharing**: Mutations create new versions sharing most data

### IEEE 754 Compliance

Float type strictly follows IEEE 754:

- NaN != NaN (no Eq trait)
- Special value handling in comparisons
- total_cmp() for total ordering including NaN

### Checked Arithmetic

Integer operations use checked math:

- `checked_add()`, `checked_mul()`, etc.
- Returns `Option<Integer>` on overflow
- No silent wraparound errors

## Benchmarking Best Practices

### Running Reliable Benchmarks

```bash
# Disable CPU frequency scaling (Linux)
sudo cpupower frequency-set --governor performance

# Run with consistent environment
cargo bench --features serde -- --warm-up-time 3

# Generate baseline for comparison
cargo bench --features serde -- --save-baseline v2-baseline

# Compare against baseline
cargo bench --features serde -- --baseline v2-baseline
```

### What to Benchmark

✅ **Do benchmark:**
- Hot path operations (get, insert, clone)
- Type conversions
- Serialization round-trips
- Large data structures (to verify O(log n))

❌ **Don't benchmark:**
- One-time initialization
- Error handling paths
- Debug/Display formatting

## Future Optimizations

Potential areas for performance improvement:

1. **Small String Optimization**: SmallString for <23 byte strings
2. **String Interning**: Deduplicate common strings
3. **Memory Pooling**: Reuse allocations for temporary values
4. **SIMD Operations**: Vectorized numeric operations
5. **Zero-Copy JSON**: Direct parsing into persistent structures

## Continuous Performance Monitoring

Integrate benchmarks into CI/CD:

```yaml
# .github/workflows/bench.yml
- name: Run benchmarks
  run: cargo bench --features serde -- --save-baseline main

- name: Compare with PR
  run: cargo bench --features serde -- --baseline main
```

This ensures performance regressions are caught early.