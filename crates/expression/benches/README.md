# Nebula Expression Benchmarks

Comprehensive benchmark suite для измерения производительности nebula-expression.

## 🚀 Quick Start

### Запустить все бенчмарки

```bash
cd crates/nebula-expression
cargo bench
```

### Запустить конкретную группу

```bash
# Template benchmarks only
cargo bench --bench baseline template

# Engine benchmarks only
cargo bench --bench baseline engine

# Concurrent benchmarks only
cargo bench --bench baseline concurrent
```

### Сохранить baseline

```bash
# Before starting P0 improvements
cargo bench -- --save-baseline before-p0

# After each P0 task
cargo bench -- --save-baseline after-p0.1
cargo bench -- --save-baseline after-p0.2
# etc
```

### Сравнить с baseline

```bash
# Compare current with before-p0
cargo bench -- --baseline before-p0

# Compare two baselines
cargo bench -- --baseline before-p0 --load-baseline after-p0.1
```

---

## 📊 Benchmark Groups

### 1. Template Benchmarks

**Что измеряем**:
- Parse time (простые/сложные шаблоны)
- Render time
- Clone performance

**Группы**:
- `template/parse/simple` - Простой шаблон (1 expression)
- `template/parse/multiple_expressions` - Несколько expressions
- `template/parse/complex` - Сложный HTML шаблон
- `template/render/simple` - Рендеринг простого
- `template/render/complex` - Рендеринг сложного
- `template/clone` - Clone performance

**Ожидаемые результаты (BEFORE P0)**:
```
template/parse/simple:            ~10 μs
template/parse/complex:           ~45 μs
template/render/simple:           ~8 μs
template/clone:                   ~2 μs
```

**Target (AFTER P0.1)**:
```
template/parse/simple:            ~2 μs   (5x faster)
template/parse/complex:           ~9 μs   (5x faster)
template/render/simple:           ~3 μs   (2.7x faster)
template/clone:                   ~50 ns  (40x faster)
```

---

### 2. Engine Benchmarks

**Что измеряем**:
- Evaluation time (cached vs uncached)
- Разные типы expressions
- Cache hit rate

**Группы**:
- `engine/evaluate_no_cache/*` - Без кеша
- `engine/evaluate_with_cache/cache_hit` - Cache hit
- `engine/evaluate_with_cache/cache_miss` - Cache miss

**Ожидаемые результаты (BEFORE P0)**:
```
engine/evaluate_no_cache/literal:       ~15 μs
engine/evaluate_no_cache/arithmetic:    ~48 μs
engine/evaluate_no_cache/function_call: ~55 μs
engine/evaluate_with_cache/cache_hit:   ~13 μs
```

**Target (AFTER P0.2)**:
```
engine/evaluate_no_cache/arithmetic:    ~35 μs  (1.4x faster)
engine/evaluate_with_cache/cache_hit:   ~5 μs   (2.6x faster)
```

---

### 3. Context Benchmarks

**Что измеряем**:
- Clone performance (с разным количеством variables)
- Lookup performance

**Группы**:
- `context/operations/clone_100_vars` - Clone контекста со 100 переменными
- `context/operations/lookup` - Lookup переменной

**Ожидаемые результаты (BEFORE P0)**:
```
context/operations/clone_100_vars:    ~2 μs
context/operations/lookup:            ~10 ns
```

**Target (AFTER P0.3)**:
```
context/operations/clone_100_vars:    ~50 ns   (40x faster)
context/operations/lookup:            ~10 ns   (same)
```

---

### 4. Concurrent Benchmarks

**Что измеряем**:
- Concurrent throughput
- Lock contention
- Scalability с количеством threads

**Группы**:
- `concurrent/access/1_thread` - Baseline (single thread)
- `concurrent/access/2_threads` - 2 threads
- `concurrent/access/4_threads` - 4 threads
- `concurrent/access/8_threads` - 8 threads
- `concurrent/throughput/ops_per_sec` - Операций в секунду

**Ожидаемые результаты (BEFORE P0)**:
```
concurrent/access/1_thread:     ~13 μs
concurrent/access/2_threads:    ~7 μs/thread (некоторый contention)
concurrent/access/8_threads:    ~10 μs/thread (сильный contention)
concurrent/throughput:          ~10,000 ops/sec
```

**Target (AFTER P0.2)**:
```
concurrent/access/1_thread:     ~13 μs    (same)
concurrent/access/2_threads:    ~13 μs    (no contention)
concurrent/access/8_threads:    ~13 μs    (no contention)
concurrent/throughput:          ~75,000 ops/sec (7.5x)
```

---

### 5. Builtin Function Benchmarks

**Что измеряем**:
- Performance различных builtin функций

**Группы**:
- `builtins/string/*` - Строковые функции
- `builtins/math/*` - Математические функции
- `builtins/array/*` - Функции массивов
- `builtins/conversion/*` - Конверсии

**Ожидаемые результаты**:
```
builtins/string/uppercase:    ~20 μs
builtins/math/abs:            ~15 μs
builtins/array/first:         ~18 μs
```

---

## 📈 Tracking Progress

### Workflow

1. **Перед началом P0**:
   ```bash
   cargo bench -- --save-baseline before-p0
   ```

2. **После каждой P0 задачи**:
   ```bash
   # Запустить и сравнить
   cargo bench -- --baseline before-p0

   # Сохранить новый baseline
   cargo bench -- --save-baseline after-p0.X
   ```

3. **Документировать результаты**:
   ```markdown
   ## P0.1 - Template Zero-Copy

   ### Results:
   - template/parse/simple: 10.2μs → 2.1μs (4.86x faster ✅)
   - template/clone: 2.1μs → 48ns (43.75x faster ✅)
   - Allocations: 8 → 0 (100% reduction ✅)
   ```

---

## 🔬 Advanced Profiling

### Flamegraph

```bash
# Install
cargo install flamegraph

# Generate flamegraph
cargo flamegraph --bench baseline -- --bench template
```

### Memory Profiling

```bash
# Install
cargo install cargo-instruments  # macOS only

# Profile memory
cargo instruments -t Allocations --bench baseline
```

### CPU Profiling (Linux)

```bash
# Record
perf record -g cargo bench --bench baseline

# Report
perf report
```

---

## 📊 Results Format

Criterion генерирует отчеты в `target/criterion/`:

```
target/criterion/
├── template/
│   ├── parse/
│   │   ├── simple/
│   │   │   ├── report/
│   │   │   │   └── index.html  ← Открыть в браузере
│   │   │   └── estimates.json
│   │   └── complex/
│   └── render/
├── engine/
└── concurrent/
```

### Открыть HTML отчеты

```bash
# macOS
open target/criterion/template/parse/simple/report/index.html

# Linux
xdg-open target/criterion/template/parse/simple/report/index.html

# Windows
start target/criterion/template/parse/simple/report/index.html
```

---

## ✅ Success Criteria

Задача P0 считается успешной если:

1. ✅ **Target met**: Метрика достигла или превысила target
2. ✅ **No regressions**: Другие бенчмарки не ухудшились
3. ✅ **Consistent**: Результаты стабильны (low variance)
4. ✅ **Documented**: Результаты задокументированы

---

## 🎯 Expected Final Results (After All P0)

| Benchmark | Before | After | Improvement |
|-----------|--------|-------|-------------|
| Template parse (simple) | 10.2μs | 2.1μs | 4.86x |
| Template clone | 2.1μs | 48ns | 43.75x |
| Engine eval (cached) | 13μs | 5μs | 2.6x |
| Concurrent (8 threads) | 10μs | 1.3μs | 7.7x |
| Context clone | 2.0μs | 50ns | 40x |
| Throughput | 10k ops/s | 75k ops/s | 7.5x |

---

## 🐛 Troubleshooting

### Benchmarks не запускаются

```bash
# Проверить что criterion установлен
cargo tree | grep criterion

# Rebuild
cargo clean
cargo bench
```

### Нестабильные результаты

```bash
# Увеличить sample size
cargo bench -- --sample-size 1000

# Warm up CPU
cargo bench -- --warm-up-time 5
```

### Comparison fails

```bash
# Убедиться что baseline существует
ls target/criterion/**/baseline/

# Пересоздать baseline
cargo bench -- --save-baseline my-baseline
```

---

## 📚 References

- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Benchmarking Best Practices](https://www.brendangregg.com/blog/2018-06-30/benchmarking-checklist.html)

---

## 🤝 Contributing

При добавлении новых бенчмарков:

1. Добавить в соответствующую группу
2. Использовать `black_box()` для входных данных
3. Документировать expected results
4. Обновить этот README

---

**Last Updated**: 2025-01-08
**Status**: Ready for baseline collection
