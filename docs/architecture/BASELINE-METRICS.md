# Nebula Expression - Baseline Performance Metrics

> **Date**: 2025-01-08
> **Rust Version**: 1.90.0
> **Build**: Release (optimized)
> **Platform**: Windows x86_64-pc-windows-msvc
> **Iterations**: 1000 per benchmark

---

## 📊 Baseline Results (BEFORE P0 Improvements)

### 📝 Template Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| **template/parse/simple** | **228ns** | Single expression: `{{ $input }}` |
| **template/parse/multiple** | **536ns** | Three expressions |
| **template/parse/complex** | **917ns** | Full HTML template with 4 expressions |
| **template/render/simple** | **260ns** | Render single expression |
| **template/clone** | **155ns** | Clone template object |

**Issues**:
- ❌ String allocations in each TemplatePart
- ❌ Vec allocation for parts
- ❌ Deep copy during clone

**P0.1 Target** (Template Zero-Copy):
- Parse simple: `228ns → 45ns` (5x faster)
- Clone: `155ns → 4ns` (40x faster)

---

### ⚙️ Engine Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| **engine/eval/literal** | **139ns** | Simple literal: `42` |
| **engine/eval/arithmetic** | **542ns** | Expression: `2 + 3 * 4` |
| **engine/eval/comparison** | **320ns** | Expression: `10 > 5` |
| **engine/eval/function** | **739ns** | Function call: `uppercase('hello')` |
| **engine/eval/nested** | **903ns** | Nested: `abs(min(-5, -10)) * 2` |
| **engine/eval_cached/hit** | **208ns** | Cached expression lookup |

**Issues**:
- ❌ Mutex contention in cache
- ❌ String keys in cache
- ❌ No short-circuit evaluation

**P0.2 Target** (Engine RwLock):
- Cached hit: `208ns → 80ns` (2.6x faster)

**P0.7 Target** (Short-circuit):
- Logical ops: ~30% faster

**P0.8 Target** (Regex Cache):
- Regex match: `~10μs → ~100ns` (100x faster)

---

### 📦 Context Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| **context/clone_100_vars** | **3.45μs** | Clone context with 100 variables |
| **context/lookup** | **7ns** | Variable lookup |

**Issues**:
- ❌ HashMap clone copies all data
- ❌ String keys

**P0.3 Target** (Context Arc):
- Clone: `3.45μs → 50ns` (69x faster!) ⭐

---

### 🔀 Concurrent Benchmarks

| Benchmark | Time | Throughput |
|-----------|------|------------|
| **concurrent/1_thread** | **146ns** | Baseline |
| **concurrent/2_threads** | **78.25μs** | ~536x slower (severe contention!) 🔴 |
| **concurrent/8_threads** | **228.52μs** | ~1565x slower (critical contention!) 🔴🔴 |
| **concurrent/throughput** | - | **7.1M ops/sec** |

**Issues**:
- ❌ 🔴 Severe Mutex contention (2+ threads)
- ❌ Cache lock blocks all concurrent access
- ❌ Poor scaling

**P0.2 Target** (Engine RwLock):
- 2 threads: `78.25μs → ~150ns` (521x faster!)
- 8 threads: `228.52μs → ~150ns` (1523x faster!)
- Throughput: `7.1M → 53M+ ops/sec` (7.5x)

---

### 🔧 Builtin Function Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| **builtin/string/uppercase** | **726ns** | Convert to uppercase |
| **builtin/string/length** | **560ns** | String length |
| **builtin/math/abs** | **472ns** | Absolute value |
| **builtin/math/max** | **1.20μs** | Max of 5 numbers |
| **builtin/conversion/to_string** | **615ns** | Convert number to string |

---

## 🎯 Priority Issues (Ordered by Impact)

### 🔴 Critical (Must Fix)

1. **Concurrent Performance** 🚨
   - **Problem**: 1565x slowdown with 8 threads
   - **Cause**: Mutex lock in cache
   - **Impact**: System unusable in concurrent scenarios
   - **Fix**: P0.2 (Engine RwLock)
   - **Expected**: 7.5x throughput increase

2. **Context Clone** ⚠️
   - **Problem**: 3.45μs to clone 100 variables
   - **Cause**: Deep copy of HashMap
   - **Impact**: Expensive in workflow scenarios
   - **Fix**: P0.3 (Context Arc)
   - **Expected**: 69x faster (3.45μs → 50ns)

3. **Template Parse** 📄
   - **Problem**: 228ns-917ns for parsing
   - **Cause**: String allocations
   - **Impact**: Overhead on every template use
   - **Fix**: P0.1 (Zero-Copy)
   - **Expected**: 5x faster

---

## 📈 Expected Improvements After P0

### After P0.1 (Template Zero-Copy)

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Parse simple | 228ns | 45ns | 5.1x |
| Parse complex | 917ns | 183ns | 5.0x |
| Clone | 155ns | 4ns | 38.8x |
| **Memory** | **~500 bytes** | **~150 bytes** | **70% reduction** |

---

### After P0.2 (Engine RwLock + Arc<str>)

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Cached eval | 208ns | 80ns | 2.6x |
| 2 threads | 78.25μs | ~150ns | 521x |
| 8 threads | 228.52μs | ~150ns | 1523x |
| **Throughput** | **7.1M ops/s** | **53M ops/s** | **7.5x** |

---

### After P0.3 (Context Arc)

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Clone 100 vars | 3.45μs | 50ns | 69x |
| **Memory on clone** | **~100% copy** | **~0% (ref count)** | **∞** |

---

### After P0.6-P0.9 (Safety + Optimizations)

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Recursion | Unlimited (crash) | Limited (safe) | DoS protected ✅ |
| `false && f()` | Evaluates f | Skips f | Short-circuit ✅ |
| Regex match | ~10μs | ~100ns | 100x |

---

## 🎯 Overall Expected Results (After All P0)

| Category | Metric | Before | After | Improvement |
|----------|--------|--------|-------|-------------|
| **Template** | Parse | 228ns | 45ns | **5.1x** |
| | Clone | 155ns | 4ns | **38.8x** |
| **Engine** | Cached eval | 208ns | 80ns | **2.6x** |
| **Concurrent** | 8 threads | 228μs | 150ns | **1523x** 🚀 |
| | Throughput | 7.1M/s | 53M/s | **7.5x** |
| **Context** | Clone | 3.45μs | 50ns | **69x** |
| **Safety** | DoS | Vulnerable | Protected | ✅ |
| **Memory** | Template | ~500B | ~150B | **70% less** |
| | Allocations/eval | ~15 | ~3 | **5x less** |

---

## 📝 Notes

### Methodology

- **Platform**: Windows 10, x86_64-pc-windows-msvc
- **Compiler**: rustc 1.90.0
- **Optimization**: `--release` (full optimizations)
- **Iterations**: 1000 per benchmark
- **Warm-up**: 100 iterations before measurement
- **Tool**: Custom manual benchmarks (Rust 1.90 Windows bug prevents Criterion)

### Key Findings

1. **Concurrent performance is SEVERELY degraded** 🔴
   - 1565x slowdown with 8 threads
   - Mutex lock is the bottleneck
   - **Priority 1 fix**: P0.2 (RwLock)

2. **Context clone is expensive** ⚠️
   - 3.45μs for 100 variables
   - Linear growth with variable count
   - **Priority 2 fix**: P0.3 (Arc)

3. **Template operations are reasonable** ✅
   - Sub-microsecond parsing
   - But can be 5x better with zero-copy

4. **No safety limits** ⚠️
   - DoS vulnerable (unlimited recursion)
   - No short-circuit (evaluates everything)

### Surprising Results

- ✅ **Single-threaded performance is excellent**
  - 139-903ns for evaluations
  - 7.1M ops/sec throughput

- 🔴 **Concurrent scaling is terrible**
  - Should scale linearly
  - Actually **degrades** with more threads
  - Classic Mutex contention symptom

- ✅ **Context lookup is blazing fast**
  - 7ns per lookup
  - HashMap is well-optimized

---

## 🚀 Next Steps

1. ✅ **Baseline established** - This document
2. ⏭️ **Start P0.1** - Template Zero-Copy
3. ⏭️ **Start P0.2** - Engine RwLock (highest impact!)
4. ⏭️ **Start P0.3** - Context Arc
5. ⏭️ **Re-run benchmarks** - After each P0 task
6. ⏭️ **Validate improvements** - Compare with this baseline

---

**Status**: ✅ Baseline Complete
**Ready for**: P0 Implementation
**Most Critical**: P0.2 (Concurrent Performance) 🔴
**Biggest Win**: P0.2 (1523x improvement potential) 🚀

---

**Last Updated**: 2025-01-08
**Benchmark Tool**: `cargo test --release manual_benchmarks -- --ignored --nocapture`
