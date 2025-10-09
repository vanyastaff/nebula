# Rust 1.90 Windows Bug - Workaround для Benchmarking

## 🐛 Проблема

Rust 1.90.0 имеет критический bug на Windows, который блокирует компиляцию:

```
error[E0080]: scalar size mismatch: expected X bytes but got Y bytes instead
```

Это известная проблема:
- https://github.com/rust-lang/rust/issues/XXXXX
- Affects: windows, windows-core, zerocopy crates
- Platform: Windows only
- Version: Rust 1.90.0

**Impact**: Невозможно использовать Criterion и другие benchmark libraries

---

## ✅ Solution 1: Downgrade Rust (Рекомендуется)

### Option A: Rust 1.85.0

```bash
# Install older version
rustup install 1.85.0

# Set as default
rustup default 1.85.0

# Verify
rustc --version
# Should show: rustc 1.85.0

# Now benchmarks should work
cd crates/nebula-expression
cargo bench
```

### Option B: Nightly (если 1.85 не помогает)

```bash
rustup default nightly

# Verify
rustc --version
# Should show: rustc 1.XX.0-nightly

# Try benchmarks
cargo bench
```

---

## ✅ Solution 2: Manual Benchmarking (Текущий подход)

Так как downgrade может не быть опцией (CI/CD constraints), используем **manual timing**:

### Создать manual benchmark helper

```rust
// crates/nebula-expression/tests/manual_benchmarks.rs

use std::time::{Duration, Instant};

const ITERATIONS: usize = 1000;

fn bench<F>(name: &str, mut f: F)
where
    F: FnMut(),
{
    // Warmup
    for _ in 0..100 {
        f();
    }

    // Measure
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        f();
    }
    let duration = start.elapsed();

    let avg = duration / ITERATIONS as u32;
    println!("{:40} {:>10.2?}", name, avg);
}

#[test]
#[ignore]
fn baseline_benchmarks() {
    use nebula_expression::*;
    use nebula_value::Value;

    println!("\n{:=^52}", " BASELINE ");

    // Template parse
    bench("template/parse/simple", || {
        let _ = Template::new("Hello {{ $input }}!");
    });

    // Engine eval
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();
    bench("engine/eval/arithmetic", || {
        let _ = engine.evaluate("2 + 3 * 4", &context);
    });

    println!("{:=^52}\n", "");
}
```

### Запуск

```bash
# Run manual benchmarks
cargo test --release manual_benchmarks -- --ignored --nocapture --test-threads=1

# Redirect to file
cargo test --release manual_benchmarks -- --ignored --nocapture --test-threads=1 > baseline.txt
```

---

## ✅ Solution 3: Linux/macOS для Benchmarks

Если есть доступ к Linux/macOS:

```bash
# На Linux/macOS Rust 1.90 работает нормально
cargo bench

# Or use WSL on Windows
wsl
cd /mnt/c/Users/vanya/RustroverProjects/nebula
cargo bench
```

---

## 📊 Manual Benchmarking Guide

### 1. Baseline Template

Создать `crates/nebula-expression/tests/manual_benchmarks.rs`:

```rust
use nebula_expression::{Template, ExpressionEngine, EvaluationContext};
use nebula_value::Value;
use std::time::Instant;

const ITERS: usize = 1000;

fn avg_time<F: FnMut()>(mut f: F) -> std::time::Duration {
    // Warmup
    for _ in 0..100 { f(); }

    // Measure
    let start = Instant::now();
    for _ in 0..ITERS { f(); }
    start.elapsed() / ITERS as u32
}

#[test]
#[ignore]
fn benchmark_baseline() {
    println!("\n=== BASELINE BENCHMARKS ===\n");

    // Template benchmarks
    println!("📝 Template:");
    println!("  parse/simple:  {:?}", avg_time(|| {
        Template::new("Hello {{ $input }}!").unwrap();
    }));

    let engine = ExpressionEngine::new();
    let mut ctx = EvaluationContext::new();
    ctx.set_input(Value::text("World"));
    let tmpl = Template::new("Hello {{ $input }}!").unwrap();

    println!("  render/simple: {:?}", avg_time(|| {
        tmpl.render(&engine, &ctx).unwrap();
    }));

    println!("  clone:         {:?}", avg_time(|| {
        let _ = tmpl.clone();
    }));

    // Engine benchmarks
    println!("\n⚙️  Engine:");
    let ctx2 = EvaluationContext::new();

    println!("  eval/literal:    {:?}", avg_time(|| {
        engine.evaluate("42", &ctx2).unwrap();
    }));

    println!("  eval/arithmetic: {:?}", avg_time(|| {
        engine.evaluate("2 + 3 * 4", &ctx2).unwrap();
    }));

    // Context benchmarks
    println!("\n📦 Context:");
    let mut big_ctx = EvaluationContext::new();
    for i in 0..100 {
        big_ctx.set_execution_var(format!("var_{}", i), Value::integer(i));
    }

    println!("  clone_100:   {:?}", avg_time(|| {
        let _ = big_ctx.clone();
    }));

    // Concurrent benchmark
    println!("\n🔀 Concurrent:");
    use std::sync::Arc;
    use std::thread;

    let arc_engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let expr = "2 + 2";

    // Warm cache
    let _ = arc_engine.evaluate(expr, &ctx2);

    println!("  1_thread:  {:?}", avg_time(|| {
        let _ = arc_engine.evaluate(expr, &ctx2);
    }));

    let start = Instant::now();
    for _ in 0..ITERS {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let eng = Arc::clone(&arc_engine);
                thread::spawn(move || {
                    let c = EvaluationContext::new();
                    for _ in 0..10 {
                        let _ = eng.evaluate(expr, &c);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
    println!("  8_threads: {:?}", start.elapsed() / ITERS as u32);

    println!("\n===========================\n");
}
```

### 2. Run and Save Results

```bash
# Run once to see results
cargo test --release benchmark_baseline -- --ignored --nocapture

# Save to file
cargo test --release benchmark_baseline -- --ignored --nocapture > BASELINE-RESULTS.txt 2>&1

# Add to git
git add BASELINE-RESULTS.txt
git commit -m "docs: add baseline benchmark results"
```

### 3. After Each P0 Task

```bash
# Run again
cargo test --release benchmark_baseline -- --ignored --nocapture > AFTER-P0.X-RESULTS.txt

# Compare
diff BASELINE-RESULTS.txt AFTER-P0.1-RESULTS.txt
```

---

## 📈 Example Expected Output

```
=== BASELINE BENCHMARKS ===

📝 Template:
  parse/simple:  10.2μs
  render/simple: 8.5μs
  clone:         2.1μs

⚙️  Engine:
  eval/literal:    15.3μs
  eval/arithmetic: 48.7μs

📦 Context:
  clone_100:   2.0μs

🔀 Concurrent:
  1_thread:  12.7μs
  8_threads: 9.8μs

===========================
```

---

## ✅ Recommended Approach

**For now (Rust 1.90 bug exists)**:

1. ✅ Use manual benchmarks (Solution 2)
2. ✅ Run on release mode: `cargo test --release`
3. ✅ Save results to files
4. ✅ Manual comparison between runs

**Long term (when bug fixed)**:

1. ⏳ Wait for Rust 1.91 or fix in 1.90.1
2. ⏳ Switch to Criterion
3. ⏳ Automated CI benchmarking

---

## 📝 Progress Tracking

### Manual Benchmark Results Format

```markdown
## Baseline (Before P0)
Date: 2025-01-08
Rust: 1.90.0 (manual benchmarks due to Windows bug)

| Benchmark | Time |
|-----------|------|
| template/parse/simple | 10.2μs |
| template/render/simple | 8.5μs |
| template/clone | 2.1μs |
| engine/eval/arithmetic | 48.7μs |
| context/clone_100 | 2.0μs |
| concurrent/1_thread | 12.7μs |
| concurrent/8_threads | 9.8μs |

## After P0.1 (Template Zero-Copy)
Date: 2025-01-09

| Benchmark | Before | After | Improvement |
|-----------|--------|-------|-------------|
| template/parse/simple | 10.2μs | 2.1μs | 4.86x ✅ |
| template/clone | 2.1μs | 48ns | 43.75x ✅ |
```

---

## 🔄 Alternative: Docker with Linux

```dockerfile
# Dockerfile.benchmark
FROM rust:1.90

WORKDIR /app
COPY . .

RUN cargo bench
```

```bash
# Build and run
docker build -f Dockerfile.benchmark -t nebula-bench .
docker run nebula-bench
```

---

## 🎯 Bottom Line

**Текущее решение**:
- ✅ Manual benchmarks работают
- ✅ Достаточно точности для P0 улучшений
- ✅ Не требуют downgrade Rust

**Рекомендация**:
1. Используй manual benchmarks (tests/manual_benchmarks.rs)
2. Запускай с `--release` для точности
3. Сохраняй результаты в файлы
4. Сравнивай вручную (diff или таблицы)

**Good enough для Phase 0!** 🚀

---

**Last Updated**: 2025-01-08
**Status**: Workaround Active
**Tracking Issue**: Rust #XXXXX (Windows scalar size mismatch)
