# Nebula Expression - Benchmarking Plan

> **Цель**: Создать comprehensive benchmark suite для измерения прогресса улучшений

---

## 🎯 Зачем бенчмарки первыми?

### Проблема без бенчмарков

```
❌ "Кажется стало быстрее"
❌ "Наверное меньше аллокаций"
❌ "Вроде работает лучше"
```

### С бенчмарками

```
✅ Template parse: 10.2μs → 2.1μs (4.86x faster)
✅ Allocations: 15 → 3 per eval (5x reduction)
✅ Concurrent throughput: 9,847 → 74,123 ops/sec (7.53x)
```

---

## 📁 Структура бенчмарков

```
crates/nebula-expression/
├── benches/
│   ├── template.rs           # Template parsing & rendering
│   ├── engine.rs             # Expression evaluation
│   ├── lexer.rs              # Tokenization
│   ├── parser.rs             # AST construction
│   ├── eval.rs               # Evaluation
│   ├── concurrent.rs         # Concurrent access
│   ├── memory.rs             # Allocation tracking
│   └── builtins.rs           # Built-in functions
├── Cargo.toml                # criterion dependency
└── README.md
```

---

## 🔧 Setup

### 1. Добавить зависимости

```toml
# Cargo.toml

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
dhat = "0.3"  # Heap allocation profiler

[[bench]]
name = "template"
harness = false

[[bench]]
name = "engine"
harness = false

[[bench]]
name = "concurrent"
harness = false

[[bench]]
name = "memory"
harness = false
```

---

## 📊 Benchmark Suite

### 1. Template Benchmarks

**Файл**: `benches/template.rs`

**Что измеряем**:
- Parse time (simple/complex templates)
- Render time
- Memory allocations
- Clone performance

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use nebula_expression::{Template, ExpressionEngine, EvaluationContext};
use nebula_value::Value;

fn benchmark_template_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_parse");

    // Simple template
    group.bench_function("simple", |b| {
        b.iter(|| {
            Template::new(black_box("Hello {{ $input }}!"))
        })
    });

    // Complex template with multiple expressions
    group.bench_function("complex", |b| {
        let template = r#"
            <html>
                <title>{{ $workflow.name }}</title>
                <body>
                    <h1>{{ $execution.id }}</h1>
                    <p>Result: {{ $input | uppercase() }}</p>
                    <span>{{ $node.data.count * 2 }}</span>
                </body>
            </html>
        "#;
        b.iter(|| {
            Template::new(black_box(template))
        })
    });

    // Real-world template
    group.bench_function("realistic", |b| {
        let template = include_str!("../tests/fixtures/email_template.html");
        b.iter(|| {
            Template::new(black_box(template))
        })
    });

    group.finish();
}

fn benchmark_template_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::text("World"));

    let simple = Template::new("Hello {{ $input }}!").unwrap();
    let complex = Template::new(r#"
        <html>
            <title>{{ $input | uppercase() }}</title>
            <p>{{ $input | length() }}</p>
        </html>
    "#).unwrap();

    group.bench_function("simple", |b| {
        b.iter(|| {
            simple.render(black_box(&engine), black_box(&context))
        })
    });

    group.bench_function("complex", |b| {
        b.iter(|| {
            complex.render(black_box(&engine), black_box(&context))
        })
    });

    group.finish();
}

fn benchmark_template_clone(c: &mut Criterion) {
    let template = Template::new("Hello {{ $input }}!").unwrap();

    c.bench_function("template_clone", |b| {
        b.iter(|| {
            black_box(template.clone())
        })
    });
}

criterion_group!(
    benches,
    benchmark_template_parse,
    benchmark_template_render,
    benchmark_template_clone
);
criterion_main!(benches);
```

---

### 2. Engine Benchmarks

**Файл**: `benches/engine.rs`

**Что измеряем**:
- Evaluation time (cached vs uncached)
- Different expression types
- Cache hit rate

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use nebula_expression::{ExpressionEngine, EvaluationContext};
use nebula_value::Value;

fn benchmark_evaluate_no_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluate_no_cache");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let expressions = vec![
        ("literal", "42"),
        ("arithmetic", "2 + 3 * 4"),
        ("comparison", "10 > 5"),
        ("string_concat", "\"hello\" + \" \" + \"world\""),
        ("function_call", "uppercase('hello')"),
        ("nested", "abs(min(-5, -10)) * 2"),
    ];

    for (name, expr) in expressions {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, expr| {
            b.iter(|| {
                engine.evaluate(black_box(expr), black_box(&context))
            })
        });
    }

    group.finish();
}

fn benchmark_evaluate_with_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluate_with_cache");

    let engine = ExpressionEngine::with_cache_size(1000);
    let context = EvaluationContext::new();

    let expr = "2 + 3 * 4";

    // Warm up cache
    let _ = engine.evaluate(expr, &context);

    group.bench_function("cache_hit", |b| {
        b.iter(|| {
            engine.evaluate(black_box(expr), black_box(&context))
        })
    });

    group.finish();
}

fn benchmark_cache_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_lookup");

    let engine = ExpressionEngine::with_cache_size(1000);
    let context = EvaluationContext::new();

    // Fill cache with different expressions
    for i in 0..100 {
        let _ = engine.evaluate(&format!("{} + {}", i, i + 1), &context);
    }

    group.bench_function("lookup", |b| {
        b.iter(|| {
            engine.evaluate(black_box("50 + 51"), black_box(&context))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_evaluate_no_cache,
    benchmark_evaluate_with_cache,
    benchmark_cache_lookup
);
criterion_main!(benches);
```

---

### 3. Concurrent Benchmarks

**Файл**: `benches/concurrent.rs`

**Что измеряем**:
- Concurrent throughput
- Lock contention
- Scalability

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use nebula_expression::{ExpressionEngine, EvaluationContext};
use std::sync::Arc;
use std::thread;

fn benchmark_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_access");

    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let expr = "2 + 2";

    // Single thread baseline
    group.bench_function("single_thread", |b| {
        let context = EvaluationContext::new();
        b.iter(|| {
            engine.evaluate(black_box(expr), black_box(&context))
        })
    });

    // Multi-threaded
    for num_threads in [2, 4, 8, 16] {
        group.bench_function(format!("{}_threads", num_threads), |b| {
            b.iter(|| {
                let handles: Vec<_> = (0..num_threads)
                    .map(|_| {
                        let engine = Arc::clone(&engine);
                        thread::spawn(move || {
                            let context = EvaluationContext::new();
                            for _ in 0..100 {
                                let _ = engine.evaluate(expr, &context);
                            }
                        })
                    })
                    .collect();

                for handle in handles {
                    handle.join().unwrap();
                }
            })
        });
    }

    group.finish();
}

fn benchmark_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.throughput(Throughput::Elements(1));

    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));

    group.bench_function("ops_per_sec", |b| {
        let context = EvaluationContext::new();
        b.iter(|| {
            engine.evaluate(black_box("2 + 2"), black_box(&context))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_concurrent_access,
    benchmark_throughput
);
criterion_main!(benches);
```

---

### 4. Memory Benchmarks

**Файл**: `benches/memory.rs`

**Что измеряем**:
- Heap allocations
- Memory usage
- Clone costs

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebula_expression::{Template, ExpressionEngine, EvaluationContext};
use nebula_value::Value;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn benchmark_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocations");

    group.bench_function("template_parse", |b| {
        b.iter(|| {
            let _profiler = dhat::Profiler::new_heap();
            Template::new(black_box("Hello {{ $input }}!"))
        })
    });

    group.bench_function("template_render", |b| {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("World"));
        let template = Template::new("Hello {{ $input }}!").unwrap();

        b.iter(|| {
            let _profiler = dhat::Profiler::new_heap();
            template.render(black_box(&engine), black_box(&context))
        })
    });

    group.bench_function("evaluate", |b| {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        b.iter(|| {
            let _profiler = dhat::Profiler::new_heap();
            engine.evaluate(black_box("2 + 2"), black_box(&context))
        })
    });

    group.finish();
}

fn benchmark_clone_costs(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone_costs");

    let mut context = EvaluationContext::new();
    context.set_input(Value::integer(42));
    for i in 0..100 {
        context.set_execution_var(&format!("var_{}", i), Value::integer(i));
    }

    group.bench_function("context_clone", |b| {
        b.iter(|| {
            black_box(context.clone())
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_allocations,
    benchmark_clone_costs
);
criterion_main!(benches);
```

---

## 🎯 Baseline Metrics (Current State)

После запуска бенчмарков создадим baseline:

```bash
# Run all benchmarks
cargo bench --package nebula-expression

# Save baseline
cargo bench --package nebula-expression -- --save-baseline before-p0
```

### Ожидаемые результаты (BEFORE)

```
Template:
  parse/simple:        10.2 μs
  parse/complex:       45.8 μs
  render/simple:       8.5 μs
  clone:              2.1 μs

Engine:
  evaluate_no_cache:  48.3 μs
  evaluate_cached:    12.7 μs
  cache_lookup:       150 ns

Concurrent:
  single_thread:      12.7 μs
  2_threads:          ~6.5 μs/thread (contention)
  8_threads:          ~10 μs/thread (severe contention)
  throughput:         ~10,000 ops/sec

Memory:
  allocations/eval:   ~15
  context_clone:      2.0 μs
```

---

## 📊 Tracking Progress

### После каждой P0 задачи

```bash
# Run benchmarks
cargo bench --package nebula-expression

# Compare with baseline
cargo bench --package nebula-expression -- --baseline before-p0

# Save new baseline
cargo bench --package nebula-expression -- --save-baseline after-p0.X
```

### Expected improvements

| After Task | Metric | Before | Target | Actual |
|------------|--------|--------|--------|--------|
| P0.1 Template | parse/simple | 10.2μs | 2μs | ??? |
| P0.2 Engine | concurrent 8t | 10μs | 1.3μs | ??? |
| P0.3 Context | clone | 2.0μs | 50ns | ??? |
| P0.8 Regex | regex match | 10μs | 0.1μs | ??? |

---

## 🔬 Advanced Profiling

### Flamegraph

```bash
# Install
cargo install flamegraph

# Profile
cargo flamegraph --bench template -- --bench

# Output: flamegraph.svg
```

### DHAT (Heap profiling)

```bash
# Run with DHAT
cargo bench --bench memory

# Analyze dhat-heap.json
```

### Perf (Linux)

```bash
perf record cargo bench --bench engine
perf report
```

---

## 📈 Continuous Benchmarking

### GitHub Actions

```yaml
# .github/workflows/benchmark.yml
name: Benchmark

on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Run benchmarks
        run: cargo bench --package nebula-expression

      - name: Store results
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: 'cargo'
          output-file-path: target/criterion/results.json
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
```

---

## ✅ Checklist

Перед началом P0 работы:

- [ ] Benchmarks написаны
- [ ] Baseline метрики собраны
- [ ] CI настроен
- [ ] Документация обновлена

После каждой P0 задачи:

- [ ] Benchmarks запущены
- [ ] Improvements validated
- [ ] Regression tests pass
- [ ] Metrics documented

---

## 🎯 Success Criteria

Задача считается успешной если:

1. ✅ **Benchmark shows improvement** (согласно target)
2. ✅ **No regressions** (другие метрики не ухудшились)
3. ✅ **CI passes** (automated checks)
4. ✅ **Documented** (результаты в CHANGELOG)

---

**Next Step**: Создать benchmark файлы и запустить baseline
