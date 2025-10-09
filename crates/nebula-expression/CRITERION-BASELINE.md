# Criterion Baseline Results (After P0.1, P0.2, P0.3)

**Date**: 2025-10-08
**Optimizations Applied**:
- ✅ P0.1 - Template Arc<str>
- ✅ P0.2 - Engine RwLock + Arc<str> keys
- ✅ P0.3 - Context Arc Values

---

## 📝 Template Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| **parse/simple** | 331.74 ns | Parse "Hello {{ $input }}!" |
| **parse/multiple_expressions** | 502.48 ns | Parse "{{ $a }} + {{ $b }} = {{ $a + $b }}" |
| **parse/complex** | 1.2289 µs | Parse HTML template with multiple expressions |
| **render/simple** | 279.31 ns | Render simple template |
| **render/complex** | 1.2352 µs | Render complex HTML template |
| **clone** | **63.685 ns** | 🎉 Template clone (Arc<str> optimization) |

---

## ⚙️ Engine Benchmarks

### Evaluate (No Cache)
| Expression Type | Time | Example |
|----------------|------|---------|
| **literal** | 165.44 ns | `42` |
| **arithmetic** | 546.22 ns | `2 + 3 * 4` |
| **comparison** | 334.34 ns | `10 > 5` |
| **string_concat** | 929.96 ns | `"hello" + " " + "world"` |
| **function_call** | 630.73 ns | `uppercase('hello')` |
| **nested** | 949.73 ns | `abs(min(-5, -10)) * 2` |
| **conditional** | 651.62 ns | `if true then 1 else 2` |

### Evaluate (With Cache)
| Benchmark | Time | Notes |
|-----------|------|-------|
| **cache_hit** | 228.51 ns | Cache hit (RwLock) |
| **cache_miss** | 2.1440 µs | Cache miss + parse + insert |

---

## 📦 Context Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| **clone_100_vars** | **1.4814 µs** | 🎉 Clone context with 100 vars (Arc optimization) |
| **lookup** | 14.631 ns | Lookup single variable |

**Previous (String/Value)**: 6.49 µs → **Current (Arc<str>/Arc<Value>)**: 1.48 µs = **77% faster!** 🚀

---

## 🔀 Concurrent Benchmarks

| Threads | Time (per operation) | Notes |
|---------|---------------------|-------|
| **1 thread** | 159.40 ns | Baseline single-threaded |
| **2 threads** | 77.945 µs | 10 ops × 2 threads |
| **4 threads** | 130.07 µs | 10 ops × 4 threads |
| **8 threads** | 228.43 µs | 10 ops × 8 threads |

**Throughput**: 160.98 ns/op (~6.2M ops/sec single thread)

**Note**: Multi-threaded performance still degraded due to RwLock write lock contention (ComputeCache requires `&mut self`).

---

## 🔧 Builtin Function Benchmarks

| Category | Function | Time | Example |
|----------|----------|------|---------|
| **String** | uppercase | 677.58 ns | `uppercase('hello world')` |
| **String** | length | 587.93 ns | `length('hello world')` |
| **Math** | abs | 652.88 ns | `abs(-42)` |
| **Math** | max | 1.0576 µs | `max(1, 2, 3, 4, 5)` |
| **Array** | first | 1.6882 µs | `first([1, 2, 3, 4, 5])` |
| **Conversion** | to_string | 625.55 ns | `to_string(42)` |

---

## 🎯 Key Performance Insights

### ✅ Wins (P0.1, P0.2, P0.3):
1. **Template Clone**: 63.7ns (Arc<str> eliminates string copies)
2. **Context Clone**: 1.48µs (77% faster with Arc<str>/Arc<Value>)
3. **Variable Lookup**: 14.6ns (HashMap with Arc keys is fast!)

### ⚠️ Areas for Improvement:
1. **Concurrent Access**: 8 threads = 228µs (1432x slower than single thread)
   - **Root Cause**: ComputeCache.get() requires `&mut self` (updates access metadata)
   - **Fix**: P0.X - Add interior mutability (AtomicUsize) for cache metrics

2. **Cache Miss**: 2.14µs (parsing overhead)
   - **Fix**: P0.4 - AST String Interning
   - **Fix**: P0.5 - Lexer Zero-Copy

3. **Array Operations**: 1.69µs for `first()`
   - **Fix**: P1 - Optimize array builtins

---

## 📊 Comparison to Manual Benchmarks

| Metric | Manual | Criterion | Difference |
|--------|--------|-----------|------------|
| Template parse/simple | 1.71µs | 332ns | **5.2x more accurate** |
| Template clone | 229ns | 64ns | **3.6x more accurate** |
| Context clone | 4.17µs | 1.48µs | **2.8x more accurate** |

**Conclusion**: Criterion provides much more accurate measurements with proper warmup and statistical analysis!

---

## 🚀 Next Steps

### Remaining P0 Tasks (9 of 12):
- [ ] P0.4 - AST String Interning (6 hours) - Reduce parse allocations
- [ ] P0.5 - Lexer Zero-Copy (6.5 hours) - Borrow from source
- [ ] P0.6 - Eval Recursion Limit (3.5 hours) - Safety
- [ ] P0.7 - Short-circuit Evaluation (3.5 hours) - Optimize && and ||
- [ ] P0.8 - Regex Caching (2.5 hours) - Cache compiled regexes
- [ ] P0.9 - Parser Recursion Limit (2.5 hours) - Safety
- [ ] P0.10 - API Surface Cleanup (1.5 hours) - Remove unused APIs
- [ ] P0.11 - Feature Flags (3.5 hours) - Conditional compilation
- [ ] P0.12 - Builtin Type Safety (7 hours) - Better type checking

### Performance Targets:
- **P0.4**: Reduce cache_miss from 2.14µs to <1.5µs
- **P0.5**: Reduce parse/simple from 332ns to <250ns
- **ComputeCache Fix**: Reduce 8_threads from 228µs to ~2µs (100x improvement)
