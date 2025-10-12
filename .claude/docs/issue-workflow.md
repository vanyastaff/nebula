# GitHub Issues Workflow

## Основной принцип

> **"продолжаем делать реальный рефакторинг правильный а не просто чтобы ошибка исчезла"**

Закрывать issues нужно **правильно архитектурно и системно**, а не просто подавлять ошибки.

## Процесс работы с issue

### 1. Анализ issue

#### Получение информации
```bash
# Просмотр issue
gh issue view <number>

# Со всеми деталями
gh issue view <number> --json title,body,labels,state,comments

# Список открытых issues
gh issue list --state open

# По приоритету
gh issue list --label high-priority --state open
```

#### Вопросы для анализа
- **Что** именно сломано?
- **Почему** это произошло? (root cause)
- **Какая** архитектурная проблема стоит за этим?
- **Какой** паттерн решит это правильно?
- **Сколько** времени потребуется?

#### Создание плана
```markdown
## Plan
1. Analyze root cause in [file:line]
2. Apply [Pattern Name] to fix architecturally
3. Update tests to verify fix
4. Document solution
5. Close issue with full explanation
```

### 2. Создание задач (Todo List)

Используем TodoWrite для отслеживания прогресса:

```json
[
  {
    "content": "Analyze Issue #N root cause",
    "activeForm": "Analyzing Issue #N root cause",
    "status": "in_progress"
  },
  {
    "content": "Read affected files and understand architecture",
    "activeForm": "Reading affected files and understanding architecture",
    "status": "pending"
  },
  {
    "content": "Apply architectural pattern (not patch)",
    "activeForm": "Applying architectural pattern",
    "status": "pending"
  },
  {
    "content": "Test compilation and run tests",
    "activeForm": "Testing compilation and running tests",
    "status": "pending"
  },
  {
    "content": "Document solution in GitHub",
    "activeForm": "Documenting solution in GitHub",
    "status": "pending"
  },
  {
    "content": "Close issue with summary",
    "activeForm": "Closing issue with summary",
    "status": "pending"
  }
]
```

**Важно**:
- Отмечать задачи completed СРАЗУ после выполнения
- Всегда иметь РОВНО ОДНУ задачу in_progress
- Не батчить несколько completed - обновлять после каждой

### 3. Исправление проблемы

#### Архитектурный подход

```rust
// ❌ ПЛОХО - Патч, подавление ошибки
#[allow(dead_code)]
fn broken_function() { /* ... */ }

// ✅ ХОРОШО - Правильное архитектурное решение
// Применяем Extension Trait Pattern
pub trait ArenaExt<T> {
    fn with<F, R>(&self, f: F) -> R
    where F: FnOnce(&Arena<T>) -> R;
}
```

#### Проверка компиляции
```bash
# После каждого изменения
cargo check -p <crate-name>

# Полная проверка
cargo check --workspace --all-features

# Тесты
cargo test -p <crate-name> --lib
```

#### Обновление тестов
Если тесты ломаются:
1. Понять, почему они сломались (правильная причина?)
2. Обновить тесты, чтобы отражать новую архитектуру
3. НЕ удалять тесты без веской причины

### 4. Документирование решения

#### Формат комментария в GitHub

```markdown
## ✅ Issue #N - RESOLVED

Brief one-line summary of what was fixed.

### Changes Made

#### 1. **Component Name** (description)
**Problem**: Detailed description of what was wrong.

**Solution**: Explanation of architectural approach used.

**Files Modified**:
- [file.rs:line1-line2](path/to/file.rs#L1-L2) - what changed
- [file2.rs:line](path/to/file2.rs#L42) - what changed

**Root Cause**: Explanation of why the problem existed.

#### 2. **Another Component** (if applicable)
...

### Test Results

✅ **All tests passing**
```bash
$ cargo test -p crate-name --lib
test result: ok. 36 passed; 0 failed
```

### Compilation Status

✅ **Zero errors**
```bash
$ cargo check -p crate-name
Finished `dev` profile
```

### Summary

Detailed explanation of:
- What pattern was applied
- Why this is the right solution
- Any follow-up work needed

**Status**: Issue can be closed.

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

### 5. Закрытие issue

```bash
# С финальным комментарием
gh issue close <number> --comment "Issue resolved! See resolution details above."

# Или просто закрыть
gh issue close <number>
```

## Категории issues

### High Priority (высокий приоритет)
- Блокируют разработку
- Компиляционные ошибки
- Критические баги
- **Подход**: Решать немедленно и системно

### Medium Priority (средний приоритет)
- Технический долг
- Оптимизации
- Тесты
- **Подход**: Планировать и решать архитектурно

### Low Priority (низкий приоритет)
- Документация
- Nice-to-have features
- **Подход**: Можно отложить, но не забыть

## Примеры правильного закрытия issues

### Пример 1: Issue #3 - Cache Policy Integration

```markdown
## ✅ Issue #3 - RESOLVED

All cache policy integration issues have been **successfully fixed** and the module is fully functional.

### Changes Made

#### 1. **Fixed TTL Policy Test** (test_ttl_fallback)
**Problem**: Test expected `None` when no keys were expired, but TTL policy correctly returns oldest entry as fallback.

**Solution**: Updated test assertion to match correct behavior.

**Files Modified**:
- [ttl.rs:229-250](crates/nebula-memory/src/cache/policies/ttl.rs#L229-L250)

#### 2. **Implemented TTL Policy clear() Method**
**Problem**: `clear()` method was missing from TtlPolicy.

**Solution**: Added `clear()` method to reset custom TTLs and insertion times.

**Files Modified**:
- [ttl.rs:68-75](crates/nebula-memory/src/cache/policies/ttl.rs#L68-L75)

#### 3. **Re-enabled LFU Module** ✨
**Problem**: LFU module was disabled since migration.

**Solution**: Fixed missing struct fields and trait implementations.

**Architectural Changes**:
- Added `AccessPattern` and `FrequencyDistribution` structs
- Implemented `EvictionPolicy` and `VictimSelector` traits
- Fixed efficiency metrics calculation

**Files Modified**:
- [lfu.rs:102-124](crates/nebula-memory/src/cache/policies/lfu.rs#L102-L124)
- [lfu.rs:182-185](crates/nebula-memory/src/cache/policies/lfu.rs#L182-L185)
- [lfu.rs:761-797](crates/nebula-memory/src/cache/policies/lfu.rs#L761-L797)

### Test Results

✅ **All 36 cache policy tests passing**

### Summary

All acceptance criteria met:
- ✅ All cache policies compile without errors
- ✅ LFU module functional and tested
- ✅ clear() method implemented for TTL cache
- ✅ All integration tests pass

**Status**: Issue can be closed.
```

### Пример 2: Progress Update (Issue #53)

Когда issue большой и требует времени, регулярно обновлять прогресс:

```markdown
## 🚧 Progress Update on Issue #53

Significant progress made on fixing nebula-validator test compilation errors.

### ✅ Fixed Issues (11 errors resolved)

#### 1. **Fixed Optional<V> Test Helper** (13 → 5 errors)
**Problem**: Test helper used `Input = str` (unsized type).

**Solution**: Changed to use `Input = String` (sized type).

**Root Cause**: Rust 2024's stricter type checking caught that `Option<str>` is invalid.

### 📊 Status

**Before**: 44 errors
**After**: 33 errors
**Fixed**: 11 errors (25%)

### 📝 Next Steps

Remaining errors require:
1. Trait bound fixes for nested combinators
2. Type annotations for complex generics
3. API compatibility updates

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

## Шаблоны для разных типов issues

### Bug Fix
```markdown
## Problem
What is broken and how to reproduce.

## Root Cause
Why this happened (architectural explanation).

## Solution
What pattern/approach was applied.

## Testing
How was the fix verified.
```

### Refactoring
```markdown
## Current State
What the code looks like now and why it's problematic.

## Architectural Problem
What design issue needs fixing.

## Pattern Applied
What pattern solves this properly.

## Benefits
Why this is better architecture.
```

### Feature Implementation
```markdown
## Feature Description
What functionality is being added.

## Design
How it fits into existing architecture.

## Implementation
Key components and their interactions.

## Testing
How the feature is tested.
```

## Anti-patterns (чего НЕ делать)

### ❌ Закрывать без объяснения
```
Fixed #42
```

### ❌ Патчить без понимания
```
Added #[allow(dead_code)] to fix warning.
Closes #42
```

### ❌ Закрывать с "работает у меня"
```
Can't reproduce, closing.
```

### ✅ Правильный подход
```
## Analyzed Issue #42

Root cause identified: Lifetime annotation missing in trait bound.

Applied proper lifetime constraints instead of suppressing warning.

Tests verify correctness.

Closes #42
```

## Метрики качества

Хороший issue closure должен содержать:

- ✅ **Root cause analysis** - понимание, почему проблема возникла
- ✅ **Architectural solution** - правильный паттерн, не патч
- ✅ **Test verification** - доказательство, что работает
- ✅ **Clear documentation** - объяснение для future maintainers
- ✅ **Follow-up tracking** - если есть связанные задачи

## Связь с коммитами

```bash
# Issue закрывается коммитом
git commit -m "fix(cache): implement clear() method

Closes #3"

# Или вручную через gh
gh issue close 3 --comment "Fixed in commit abc123"
```

## Дополнительные команды

```bash
# Создать issue
gh issue create --title "Title" --body "Description"

# Редактировать
gh issue edit <number> --add-label "bug"

# Переоткрыть
gh issue reopen <number>

# Список моих issues
gh issue list --assignee @me

# Искать по тексту
gh issue list --search "cache policy"
```
