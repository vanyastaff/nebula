# Nebula Expression - Краткая сводка проблем

## 🎯 Quick Reference

### Статистика

- **Всего проблем**: 160+
- **Критических (P0)**: 12
- **Важных (P1)**: 23
- **Желательных (P2)**: 45
- **Nice-to-have (P3)**: 80+

### Распределение по категориям

| Категория | Количество | Критичность |
|-----------|-----------|-------------|
| Performance | 45 (28%) | 🔴 High |
| Memory | 38 (24%) | 🔴 High |
| Architecture | 22 (14%) | 🟡 Medium |
| API Design | 18 (11%) | 🟡 Medium |
| Error Handling | 15 (9%) | 🟡 Medium |
| Testing | 12 (7%) | 🟢 Low |
| Documentation | 10 (6%) | 🟢 Low |

---

## 📁 Проблемы по файлам (краткая версия)

### lib.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| Публичные внутренние модули | P0 | 1.5h |
| Нет feature flags | P0 | 3.5h |
| Экспорт Token | P1 | 1h |

**Impact**: API stability, compilation time

---

### engine.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| `Arc<Mutex<...>>` contention | P0 | 3h |
| String ключи в кеше | P0 | 0.5h |
| Нет метрик | P1 | 2h |
| Нет timeout/limits | P1 | 3h |

**Impact**: 7.5x slower concurrent, unnecessary allocations

---

### template.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| String в TemplatePart | P0 | 4h |
| Vec overhead | P0 | 0.5h |
| Нет lifetime | P0 | - |
| Char iteration | P2 | 2h |

**Impact**: 70% excessive allocations, slow parsing

---

### context/mod.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| HashMap clone дорогой | P0 | 5.5h |
| String ключи | P1 | 0.5h |
| Нет nested scopes | P1 | 2h |
| resolve_variable O(n) | P2 | 3h |

**Impact**: 40x slower clone, no lambda scoping

---

### core/ast.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| String везде | P0 | 6h |
| Box<Expr> | P0 | - |
| Нет span/position | P1 | 5h |
| Нет constant folding | P2 | 8h |

**Impact**: Expensive cloning, poor errors

---

### core/token.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| String в токенах | P1 | 4h |
| precedence() не const | P2 | 1h |
| Нет позиции | P1 | 2h |

**Impact**: Allocations, runtime overhead

---

### lexer/mod.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| Vec<char> allocation | P0 | 6.5h |
| chars().collect() upfront | P0 | - |
| Нет fast path для ASCII | P2 | 3h |

**Impact**: O(n) allocation, 1.5x slower

---

### parser/mod.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| Stack overflow риск | P0 | 2.5h |
| Плохие error messages | P1 | 5h |
| Нет error recovery | P2 | 8h |

**Impact**: DoS vulnerability, UX

---

### eval/mod.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| Stack overflow риск | P0 | 3.5h |
| Нет short-circuit && || | P0 | 3.5h |
| Regex::new() каждый раз | P0 | 2.5h |
| Клонирование Value | P1 | 6h |

**Impact**: DoS, performance, correctness

---

### builtins/mod.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| HashMap<String, Fn> lookup | P1 | 3h |
| Нет type safety | P0 | 7h |
| Нет документации | P2 | 8h |

**Impact**: Allocations, type errors

---

### builtins/*.rs

| Файл | Топ-3 проблемы | Приоритет |
|------|---------------|-----------|
| string.rs | check_arg_count allocates, substring O(n), no ASCII fast path | P1 |
| math.rs | Лишние conversions, no SIMD | P2 |
| array.rs | Lambda не реализована, sort копирует, flatten overflow | P1 |
| datetime.rs | 6 форматов O(n), no timezone, no validation | P1 |
| object.rs | keys/values копируют, нет merge/pick | P2 |
| conversion.rs | parse_json no limits (DoS), to_boolean wrong | P1 |
| util.rs | is_* allocate Value, no type_of | P2 |

---

### maybe.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| String storage всегда | P1 | 3h |
| Нет валидации при deser | P1 | 2h |
| untagged serde | P2 | 1h |

**Impact**: Allocations, runtime errors

---

### error_formatter.rs

| Проблема | Приоритет | Время |
|----------|-----------|-------|
| format() allocates | P2 | 3h |
| Нет color support | P3 | 2h |
| lines() collect | P2 | 1h |

**Impact**: Performance, UX

---

## 🔥 Top 10 Most Critical

1. **Template Zero-Copy** (P0, 4h)
   - Impact: 70% memory reduction
   - Files: template.rs

2. **Engine RwLock** (P0, 3h)
   - Impact: 7.5x concurrent throughput
   - Files: engine.rs

3. **Context Arc Values** (P0, 5.5h)
   - Impact: 40x faster clone
   - Files: context/mod.rs

4. **AST String Interning** (P0, 6h)
   - Impact: 10x faster clone, 50% memory
   - Files: core/ast.rs

5. **Lexer Zero-Copy** (P0, 6.5h)
   - Impact: 1.5x faster, 0 allocations
   - Files: lexer/mod.rs

6. **Eval Recursion Limit** (P0, 3.5h)
   - Impact: DoS protection
   - Files: eval/mod.rs

7. **Short-circuit Evaluation** (P0, 3.5h)
   - Impact: Correctness + performance
   - Files: eval/mod.rs

8. **Regex Caching** (P0, 2.5h)
   - Impact: 100x faster regex
   - Files: eval/mod.rs

9. **Parser Recursion Limit** (P0, 2.5h)
   - Impact: DoS protection
   - Files: parser/mod.rs

10. **API Surface Cleanup** (P0, 1.5h)
    - Impact: Stable API
    - Files: lib.rs

**Total P0 time**: ~39 hours

---

## 📊 Ожидаемые улучшения

### Performance

```
Template parse:     10μs → 2μs     (5x)
Expression eval:    50μs → 15μs    (3.3x)
Context clone:      2μs → 50ns     (40x)
Concurrent ops:     10k → 75k      (7.5x)
Regex (cached):     10μs → 0.1μs   (100x)
```

### Memory

```
Allocations/eval:   ~15 → ~3       (5x)
Template memory:    500B → 150B    (3.3x)
AST clone:          Deep → Arc     (∞)
Context clone:      Full → Arc     (∞)
```

### Safety

- ✅ DoS protected (recursion limits)
- ✅ No stack overflow
- ✅ Proper error context
- ✅ Type-safe builtins

---

## 🗓️ Implementation Timeline

### Week 1: Foundation
- Template Zero-Copy
- Engine RwLock
- Context Arc
- AST Interning

**Result**: 70% fewer allocations, 5x clone speed

---

### Week 2: Safety
- Lexer Zero-Copy
- Recursion Limits (eval + parser)
- Short-circuit
- Regex Caching

**Result**: DoS protected, 1.5x faster lexing

---

### Week 3: API
- API Surface Cleanup
- Feature Flags
- Builtin Type Safety

**Result**: Stable API, optional dependencies

---

### Week 4+: P1 Tasks
- Token lifetimes
- Error context
- Iterator builtins
- etc.

**Result**: Long-term quality improvements

---

## 🎯 Success Criteria

После реализации P0-P1:

✅ **Performance**
- [ ] 5-10x throughput
- [ ] 70-80% fewer allocations
- [ ] 50-60% less memory

✅ **Safety**
- [ ] DoS protected
- [ ] No crashes on deep nesting
- [ ] Type-safe operations

✅ **API**
- [ ] Clean public interface
- [ ] Optional dependencies
- [ ] Zero breaking changes

✅ **Quality**
- [ ] 90%+ test coverage
- [ ] Comprehensive docs
- [ ] Performance benchmarks

---

## 📚 Документация

### Детальный анализ
- `nebula-expression-analysis.md` - Глубокий анализ каждого файла

### Roadmap
- `nebula-expression-improvements-roadmap.md` - Детальный план реализации

### Краткая сводка
- `nebula-expression-issues-summary.md` - Этот файл

---

## 🚀 Quick Start (для разработчиков)

### Начать с P0

1. **Прочитать**:
   - `nebula-expression-improvements-roadmap.md` секцию P0

2. **Выбрать задачу**:
   - Начните с P0.1 (Template Zero-Copy)
   - Или P0.2 (Engine RwLock) если нужен quick win

3. **Реализовать**:
   - Создать feature branch
   - Следовать чеклисту в roadmap
   - Написать тесты
   - Benchmark

4. **Review**:
   - Code review
   - Performance check
   - Merge

### Рекомендуемый порядок

```
P0.1 (Template) → P0.2 (Engine) → P0.3 (Context)
         ↓
    Integration test + benchmarks
         ↓
P0.4 (AST) → P0.5 (Lexer)
         ↓
    Performance regression tests
         ↓
P0.6-P0.9 (Safety)
         ↓
    Security audit + fuzzing
         ↓
P0.10-P0.12 (API)
         ↓
    Migration guide + docs
```

---

## 🔗 Связанные документы

- Architecture Overview: `../README.md`
- Performance Benchmarks: `../../benchmarks/`
- Test Strategy: `../../tests/README.md`

---

**Последнее обновление**: 2025-01-08
**Версия**: 1.0
**Статус**: Active Development
