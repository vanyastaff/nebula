# 🚀 Quick Start - Начать улучшения nebula-expression

> **Цель**: За 5 минут понять что делать и начать работу

---

## ⚡ TL;DR

**Проблема**: 160+ проблем с производительностью и архитектурой
**Решение**: 12 критичных задач (P0) за 6 дней
**Результат**: 5-10x производительность, 70% меньше памяти

---

## 🎯 Сегодня начинаю с...

### Option 1: Максимальный ROI (Quick Win)

**Задача**: P0.2 - Engine RwLock
**Время**: 3 часа
**Результат**: 7.5x concurrent throughput

```bash
# 1. Создать branch
git checkout -b feature/p0.2-engine-rwlock

# 2. Добавить зависимость
# В Cargo.toml:
# parking_lot = "0.12"

# 3. Заменить Mutex на RwLock в engine.rs
# До: Arc<Mutex<Cache>>
# После: Arc<RwLock<Cache>>

# 4. Тесты
cargo test --package nebula-expression

# 5. Benchmark
cargo bench --package nebula-expression
```

**Детали**: [Roadmap → P0.2](./nebula-expression-improvements-roadmap.md#p02-engine-rwlock--arc-keys)

---

### Option 2: Максимальный Impact

**Задача**: P0.1 - Template Zero-Copy
**Время**: 4 часа
**Результат**: 70% memory reduction

```bash
git checkout -b feature/p0.1-template-zero-copy

# В template.rs:
# 1. Добавить lifetime параметры
# 2. String → Cow<'a, str>
# 3. Vec → SmallVec<[...; 8]>

cargo test
cargo bench
```

**Детали**: [Roadmap → P0.1](./nebula-expression-improvements-roadmap.md#p01-template-zero-copy)

---

### Option 3: Критичная безопасность

**Задача**: P0.6 - Eval Recursion Limit
**Время**: 3.5 часа
**Результат**: DoS protection

```bash
git checkout -b feature/p0.6-eval-recursion-limit

# В eval/mod.rs:
# 1. Добавить max_depth field
# 2. Tracking depth в рекурсивных вызовах
# 3. Error если depth > limit

cargo test
# Добавить DoS test с deep nesting
```

**Детали**: [Roadmap → P0.6](./nebula-expression-improvements-roadmap.md#p06-eval-recursion-limit)

---

## 📚 Документация (5-минутный обзор)

### Для немедленного старта

```bash
# 1. Quick reference (5 мин)
cat docs/architecture/nebula-expression-issues-summary.md

# 2. Выбрать задачу (2 мин)
grep "P0\." docs/architecture/nebula-expression-improvements-roadmap.md

# 3. Начать работу
```

### Полная документация

| Документ | Когда читать | Время |
|----------|-------------|-------|
| [README](./nebula-expression-README.md) | Первый раз | 5 мин |
| [Quick Summary](./nebula-expression-issues-summary.md) | Quick reference | 10 мин |
| [Detailed Analysis](./nebula-expression-analysis.md) | Deep dive | 30 мин |
| [Roadmap](./nebula-expression-improvements-roadmap.md) | Планирование | 45 мин |
| [Priority Matrix](./nebula-expression-priority-matrix.md) | Спринт planning | 15 мин |

---

## ✅ Checklist перед началом

- [ ] Прочитал Quick Summary (10 мин)
- [ ] Выбрал задачу из P0
- [ ] Прочитал детали задачи в Roadmap
- [ ] Создал feature branch
- [ ] Знаю ожидаемый результат (metrics)

---

## 🎯 P0 Tasks (Quick Reference)

| # | Task | Time | ROI | Files |
|---|------|------|-----|-------|
| P0.1 | Template Zero-Copy | 4h | ⭐⭐⭐⭐⭐ | template.rs |
| P0.2 | Engine RwLock | 3h | ⭐⭐⭐⭐⭐ | engine.rs |
| P0.3 | Context Arc | 5.5h | ⭐⭐⭐⭐ | context/mod.rs |
| P0.4 | AST Interning | 6h | ⭐⭐⭐⭐ | core/ast.rs |
| P0.5 | Lexer Zero-Copy | 6.5h | ⭐⭐⭐ | lexer/mod.rs |
| P0.6 | Eval Recursion | 3.5h | ⭐⭐⭐ | eval/mod.rs |
| P0.7 | Short-circuit | 3.5h | ⭐⭐⭐ | eval/mod.rs |
| P0.8 | Regex Cache | 2.5h | ⭐⭐⭐ | eval/mod.rs |
| P0.9 | Parser Recursion | 2.5h | ⭐⭐⭐ | parser/mod.rs |
| P0.10 | API Surface | 1.5h | ⭐⭐⭐ | lib.rs |
| P0.11 | Feature Flags | 3.5h | ⭐⭐ | Cargo.toml |
| P0.12 | Type Safety | 7h | ⭐⭐ | builtins/mod.rs |

**Рекомендация**: Начать с P0.2 или P0.8 (максимальный ROI)

---

## 🔥 Fastest Path to Impact

### Day 1 (Morning)
**P0.2 - Engine RwLock** (3h)
- Replace Mutex → RwLock
- Replace String → Arc<str>
- Benchmark

**Result**: 7.5x concurrent throughput ✅

### Day 1 (Afternoon)
**P0.8 - Regex Cache** (2.5h)
- Add LruCache for Regex
- Update eval_binary_op
- Benchmark

**Result**: 100x faster regex ✅

### Day 2 (Morning)
**P0.10 - API Surface** (1.5h)
- Make internal modules private
- Clean up re-exports

**Result**: Stable API ✅

### Day 2 (Afternoon)
**P0.1 - Template Zero-Copy** (4h)
- Add lifetime parameters
- String → Cow<'a, str>
- SmallVec

**Result**: 70% memory reduction ✅

### Day 3
**P0.6 + P0.7 - Safety** (7h)
- Recursion limits
- Short-circuit

**Result**: DoS protected ✅

**Total Impact after 3 days**:
- ⬆️ 7.5x concurrent
- ⬆️ 100x regex (cached)
- ⬇️ 70% memory
- ✅ DoS protected
- ✅ Stable API

---

## 💡 Code Examples

### P0.2 - Engine RwLock

```rust
// ❌ До
use std::sync::Mutex;
expr_cache: Option<Arc<Mutex<ComputeCache<String, Expr>>>>

// ✅ После
use parking_lot::RwLock;
expr_cache: Option<Arc<RwLock<ComputeCache<Arc<str>, Expr>>>>

// Использование
let cache = self.expr_cache.as_ref()?;

// Read lock (concurrent)
{
    let cache_read = cache.read();
    if let Some(ast) = cache_read.get(&key) {
        return Ok(ast);
    }
}

// Write lock (exclusive)
{
    let mut cache_write = cache.write();
    cache_write.insert(key, ast);
}
```

---

### P0.1 - Template Zero-Copy

```rust
// ❌ До
pub struct Template {
    source: String,
    parts: Vec<TemplatePart>,
}

pub enum TemplatePart {
    Static { content: String, ... },
}

// ✅ После
use std::borrow::Cow;
use smallvec::SmallVec;

pub struct Template<'a> {
    source: Cow<'a, str>,
    parts: SmallVec<[TemplatePart<'a>; 8]>,
}

pub enum TemplatePart<'a> {
    Static { content: Cow<'a, str>, ... },
}

// Использование
let template = Template::new("Hello {{ $input }}").unwrap(); // Borrowed
let owned = Template::new(format!("Hello {}", var)).unwrap(); // Owned
```

---

### P0.6 - Eval Recursion Limit

```rust
// ❌ До
pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
    match expr {
        Expr::Binary { left, op, right } => {
            let left_val = self.eval(left, context)?;  // ⚠️ Unbounded recursion
            // ...
        }
    }
}

// ✅ После
pub struct Evaluator {
    max_depth: usize,
    // ...
}

pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
    self.eval_with_depth(expr, context, 0)
}

fn eval_with_depth(&self, expr: &Expr, context: &EvaluationContext, depth: usize) -> ExpressionResult<Value> {
    if depth > self.max_depth {
        return Err(NebulaError::expression_eval_error(
            format!("Expression too deeply nested (limit: {})", self.max_depth)
        ));
    }

    match expr {
        Expr::Binary { left, op, right } => {
            let left_val = self.eval_with_depth(left, context, depth + 1)?;  // ✅ Tracked
            // ...
        }
    }
}
```

---

## 🧪 Testing Strategy

### Unit Tests

```rust
#[test]
fn test_zero_copy_template() {
    let source = "Hello {{ $input }}";
    let template = Template::new(source).unwrap();

    // Verify borrowing
    assert_eq!(template.source().as_ptr(), source.as_ptr());
}

#[test]
fn test_recursion_limit() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    // Create deeply nested expression
    let expr = "(((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))";

    let result = engine.evaluate(expr, &context);
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("too deeply nested"));
}
```

### Benchmarks

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_concurrent_access(c: &mut Criterion) {
    let engine = ExpressionEngine::with_cache_size(1000);
    let context = EvaluationContext::new();

    c.bench_function("concurrent_eval", |b| {
        b.iter(|| {
            engine.evaluate(black_box("2 + 2"), &context)
        })
    });
}

criterion_group!(benches, benchmark_concurrent_access);
criterion_main!(benches);
```

---

## 📊 Success Metrics

### После каждой задачи

```bash
# Run tests
cargo test --package nebula-expression

# Run benchmarks
cargo bench --package nebula-expression

# Check allocations (requires nightly)
cargo +nightly test --package nebula-expression -- --nocapture

# Clippy
cargo clippy --package nebula-expression -- -D warnings
```

### Ожидаемые результаты

| Task | Metric | Before | After | Target Met? |
|------|--------|--------|-------|-------------|
| P0.1 | Memory | 500B | 150B | ✅ if <200B |
| P0.2 | Concurrent ops | 10k | 75k | ✅ if >50k |
| P0.6 | DoS test | ❌ Crash | ✅ Error | ✅ if error |
| P0.8 | Regex (cached) | 10μs | 0.1μs | ✅ if <1μs |

---

## 🚨 Common Pitfalls

### ❌ Mistake 1: Breaking changes

```rust
// ❌ НЕ ДЕЛАТЬ
pub use core::token::Token;  // Exposing internal details

// ✅ ДЕЛАТЬ
// Keep Token private, only export what's needed
```

### ❌ Mistake 2: Неправильные lifetimes

```rust
// ❌ НЕ ДЕЛАТЬ
impl Template {
    pub fn new(source: &str) -> Self {
        Self { source: source.to_string() }  // Always owned
    }
}

// ✅ ДЕЛАТЬ
impl<'a> Template<'a> {
    pub fn new(source: impl Into<Cow<'a, str>>) -> Self {
        Self { source: source.into() }  // Borrowed or owned
    }
}
```

### ❌ Mistake 3: Deadlocks с RwLock

```rust
// ❌ НЕ ДЕЛАТЬ
let cache = self.cache.write();  // Write lock
// ... call function that also needs write lock
// DEADLOCK!

// ✅ ДЕЛАТЬ
{
    let cache = self.cache.write();
    // ... use cache
}  // Lock dropped
// ... now safe to acquire again
```

---

## 🎓 Learning Resources

### Rust Patterns
- [Cow](https://doc.rust-lang.org/std/borrow/enum.Cow.html)
- [Arc](https://doc.rust-lang.org/std/sync/struct.Arc.html)
- [RwLock](https://docs.rs/parking_lot/latest/parking_lot/type.RwLock.html)
- [SmallVec](https://docs.rs/smallvec/)

### Performance
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Allocation profiling](https://github.com/koute/bytehound)

---

## 🏁 Финальный чеклист

Перед commit:

- [ ] Код следует solution из Roadmap
- [ ] Все тесты проходят (`cargo test`)
- [ ] Benchmarks показывают улучшение (`cargo bench`)
- [ ] Нет clippy warnings (`cargo clippy`)
- [ ] Документация обновлена
- [ ] Нет breaking changes (или есть migration guide)
- [ ] CHANGELOG.md обновлен

Перед PR:

- [ ] Feature branch создан из main
- [ ] Commit message описывает изменения
- [ ] PR description ссылается на P0.X
- [ ] Self-review сделан
- [ ] Ready for code review

---

## 🚀 Let's Go!

Выбери задачу и начинай:

```bash
# Quick win
git checkout -b feature/p0.2-engine-rwlock

# Max impact
git checkout -b feature/p0.1-template-zero-copy

# Critical safety
git checkout -b feature/p0.6-eval-recursion-limit

# Happy coding! 🎉
```

---

**Pro Tip**: Начни с P0.2 (Engine RwLock) - самый высокий ROI и относительно простая задача.

**Questions?** См. [Roadmap](./nebula-expression-improvements-roadmap.md) для детальных инструкций.

**Good luck! 🚀**
