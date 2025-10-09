# Phase 0: Benchmarking Setup

> **Цель**: Установить baseline метрики ПЕРЕД началом P0 улучшений

---

## ✅ Что уже сделано

1. ✅ Created comprehensive benchmark plan
2. ✅ Created baseline benchmark file (`benches/baseline.rs`)
3. ✅ Updated Cargo.toml with criterion
4. ✅ Created benchmarks README with documentation

---

## 🚧 Что нужно сделать

### Step 1: Fix Rust Toolchain Issue (если нужно)

```bash
# Проблема с Rust 1.90 на Windows
# Временное решение - использовать более стабильную версию:

rustup install 1.85.0
rustup default 1.85.0

# ИЛИ использовать nightly:
rustup default nightly
```

### Step 2: Verify Benchmarks Compile

```bash
cd crates/nebula-expression

# Проверить что компилируется
cargo bench --no-run

# Должно быть успешно без ошибок
```

### Step 3: Run Baseline Benchmarks

```bash
# Запустить все бенчмарки (займет ~5-10 минут)
cargo bench

# Результаты сохранятся в target/criterion/
```

### Step 4: Save Baseline

```bash
# Сохранить baseline для сравнения
cargo bench -- --save-baseline before-p0

# Verify baseline saved
ls target/criterion/**/baseline/
```

### Step 5: Document Results

Создать файл `BASELINE-RESULTS.md` с actual результатами:

```markdown
# Baseline Results (Before P0)

Date: YYYY-MM-DD
Rust Version: 1.85.0
Commit: <git hash>

## Template Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| template/parse/simple | X.X μs | - |
| template/parse/complex | XX.X μs | - |
| template/render/simple | X.X μs | - |
| template/clone | X.X μs | - |

## Engine Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| engine/evaluate_no_cache/literal | XX.X μs | - |
| engine/evaluate_no_cache/arithmetic | XX.X μs | - |
| engine/evaluate_with_cache/cache_hit | XX.X μs | - |

## Context Benchmarks

| Benchmark | Time | Notes |
|-----------|------|-------|
| context/operations/clone_100_vars | X.X μs | - |
| context/operations/lookup | XX ns | - |

## Concurrent Benchmarks

| Benchmark | Time | Throughput |
|-----------|------|------------|
| concurrent/access/1_thread | XX.X μs | - |
| concurrent/access/2_threads | XX.X μs | - |
| concurrent/access/8_threads | XX.X μs | - |
| concurrent/throughput | - | X,XXX ops/sec |

## Summary

- Total allocations per eval: ~XX
- Memory usage per template: ~XXX bytes
- Concurrent scalability: X threads = X.Xx slower per thread
```

---

## 📊 Expected Baseline (Estimates)

Эти значения являются **оценками** на основе анализа кода. Actual результаты могут отличаться.

### Template Benchmarks

```
template/parse/simple:               8-12 μs
template/parse/multiple_expressions: 20-30 μs
template/parse/complex:              40-50 μs
template/render/simple:              6-10 μs
template/render/complex:             15-25 μs
template/clone:                      1-3 μs
```

**Проблемы**:
- String allocations в каждом TemplatePart
- Vec allocation для parts
- Deep copy при clone

---

### Engine Benchmarks

```
engine/evaluate_no_cache/literal:       10-20 μs
engine/evaluate_no_cache/arithmetic:    40-60 μs
engine/evaluate_no_cache/function_call: 50-70 μs
engine/evaluate_with_cache/cache_hit:   10-15 μs
engine/evaluate_with_cache/cache_miss:  40-60 μs
```

**Проблемы**:
- Mutex contention в cache
- String keys в cache
- Parsing overhead

---

### Context Benchmarks

```
context/operations/clone_100_vars:  1-3 μs
context/operations/lookup:          5-15 ns
```

**Проблемы**:
- HashMap clone копирует все данные
- String keys

---

### Concurrent Benchmarks

```
concurrent/access/1_thread:     10-15 μs
concurrent/access/2_threads:    6-8 μs/thread (some contention)
concurrent/access/4_threads:    8-12 μs/thread (more contention)
concurrent/access/8_threads:    10-15 μs/thread (severe contention)
concurrent/throughput:          8,000-12,000 ops/sec
```

**Проблемы**:
- Mutex lock contention
- Poor scaling with threads

---

### Builtin Benchmarks

```
builtins/string/uppercase:      15-25 μs
builtins/string/length:         10-20 μs
builtins/math/abs:              10-20 μs
builtins/math/max:              20-30 μs
builtins/array/first:           15-25 μs
builtins/conversion/to_string:  10-20 μs
```

---

## 🎯 Target Metrics (After P0)

| Category | Metric | Before (est) | After (target) | Improvement |
|----------|--------|--------------|----------------|-------------|
| **Template** | parse/simple | 10μs | 2μs | 5x |
| | clone | 2μs | 50ns | 40x |
| **Engine** | eval cached | 13μs | 5μs | 2.6x |
| **Concurrent** | 8 threads | 12μs | 1.5μs | 8x |
| | throughput | 10k/s | 75k/s | 7.5x |
| **Context** | clone | 2μs | 50ns | 40x |

---

## 📁 Files Created

```
crates/nebula-expression/
├── benches/
│   ├── baseline.rs         ✅ Created
│   └── README.md           ✅ Created
├── Cargo.toml              ✅ Updated (criterion added)
└── docs/
    └── BASELINE-RESULTS.md  ⏳ To be created after running benchmarks
```

---

## ✅ Checklist

**Before running benchmarks**:
- [ ] Rust toolchain working (resolve 1.90 issue if needed)
- [ ] Benchmarks compile (`cargo bench --no-run`)
- [ ] No other processes running (clean environment)
- [ ] Sufficient time allocated (~10 minutes)

**After running benchmarks**:
- [ ] Baseline saved (`--save-baseline before-p0`)
- [ ] Results documented in BASELINE-RESULTS.md
- [ ] HTML reports generated (`target/criterion/**/report/index.html`)
- [ ] Results committed to git

**Before starting P0 work**:
- [ ] Baseline metrics established
- [ ] Targets understood
- [ ] Team reviewed baseline
- [ ] Ready to measure improvements

---

## 🚀 Next Steps

После завершения Phase 0:

1. **Review baseline** - Понять где реальные bottlenecks
2. **Validate estimates** - Сравнить actual vs estimated
3. **Adjust priorities** - Если нужно, перепланировать P0 порядок
4. **Start P0.1** - Начать первую задачу с confidence

---

## 📞 Help

Если возникли проблемы:

1. **Rust toolchain issue**:
   ```bash
   rustup default 1.85.0
   # or
   rustup default nightly
   ```

2. **Criterion not compiling**:
   ```bash
   cargo clean
   cargo update
   cargo bench --no-run
   ```

3. **Benchmarks too slow**:
   ```bash
   # Reduce sample size
   cargo bench -- --sample-size 10

   # Or skip certain groups
   cargo bench --bench baseline template
   ```

4. **Unstable results**:
   ```bash
   # Close other programs
   # Increase warm-up time
   cargo bench -- --warm-up-time 10
   ```

---

## 📚 Resources

- [Benchmark Plan](./nebula-expression-benchmarking-plan.md)
- [Benchmarks README](../crates/nebula-expression/benches/README.md)
- [Criterion.rs Book](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)

---

**Status**: ⏳ Waiting for Rust toolchain fix
**Blocking**: P0 work should not start until baseline established
**Owner**: Development Team
**Last Updated**: 2025-01-08
