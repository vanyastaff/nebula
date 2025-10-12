# Git Commit Guidelines

## Формат коммит-сообщения

```
<type>(<scope>): <subject>

[optional body]

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
```

## Типы коммитов

### Основные типы
- **feat**: Новая функциональность
  ```
  feat(validator): add Optional combinator for nullable fields
  ```

- **fix**: Исправление бага
  ```
  fix(cache): resolve memory leak in TTL policy
  ```

- **refactor**: Рефакторинг без изменения функциональности
  ```
  refactor(memory): apply Extension Trait pattern to Arena
  ```

- **perf**: Улучшение производительности
  ```
  perf(expression): optimize parser with memoization
  ```

- **test**: Добавление или исправление тестов
  ```
  test(validator): add tests for nested Or combinators
  ```

- **docs**: Изменения в документации
  ```
  docs(readme): add architecture overview section
  ```

- **style**: Форматирование, пробелы (не влияет на код)
  ```
  style(all): run cargo fmt
  ```

- **chore**: Вспомогательные изменения (зависимости, конфиг)
  ```
  chore(deps): update serde to 1.0.200
  ```

- **ci**: Изменения в CI/CD
  ```
  ci(github): add Rust 2024 edition check
  ```

### Специальные маркеры

- **BREAKING CHANGE**: В теле коммита для breaking changes
  ```
  feat(validator)!: change TypedValidator trait signature

  BREAKING CHANGE: TypedValidator now requires Error associated type
  instead of using ValidationError directly.

  Migration guide:
  - Before: impl Validator for MyValidator
  - After: impl TypedValidator for MyValidator { type Error = ValidationError; }
  ```

- **Closes #N**: Автоматическое закрытие issue
  ```
  fix(cache): implement TTL clear() method

  Closes #3
  ```

- **Refs #N**: Ссылка на issue без закрытия
  ```
  refactor(validator): improve type inference in tests

  Refs #53
  ```

## Scope (область)

Указывает, какой модуль/компонент затронут:

- `validator` - nebula-validator crate
- `memory` - nebula-memory crate
- `parameter` - nebula-parameter crate
- `expression` - nebula-expression crate
- `derive` - nebula-derive crate
- `log` - nebula-log crate
- `resilience` - nebula-resilience crate
- `all` - изменения во всех crates
- `ci` - CI/CD конфигурация
- `deps` - зависимости

## Subject (тема)

- Используйте императивное наклонение: "add" не "added" или "adds"
- Не начинайте с заглавной буквы
- Без точки в конце
- Максимум 50 символов для краткости
- Описывайте **что** сделано, не **как**

```
✅ ПРАВИЛЬНО:
fix(cache): resolve TTL expiration race condition

❌ НЕПРАВИЛЬНО:
Fixed a bug in the cache where TTL expiration had race conditions
```

## Body (тело сообщения)

Опционально, но рекомендуется для сложных изменений:

- Объясняйте **зачем**, не **что** (что видно из diff)
- Описывайте архитектурное решение
- Ссылайтесь на issues, документацию
- Разделяйте параграфы пустой строкой
- Переносите строки на ~72 символа

```
refactor(validator): apply Type Erasure Wrapper pattern

The display system needs to work with TypedValidator trait which has
associated types and is not object-safe. Applied Type Erasure Wrapper
pattern to bridge typed validators with dynamic dispatch.

Solution:
- Created DisplayRuleEvaluator trait (object-safe)
- Implemented ValidatorAdapter<V> for TypedValidator types
- Maintains ergonomic API while supporting new validator system

Refs #2
```

## Примеры правильных коммитов

### Простой баг-фикс
```
fix(memory): correct CacheStats field name

Changed memory_estimate to weighted_size to match new API.
```

### Новая функциональность
```
feat(memory): add LFU cache eviction policy

Implemented Least Frequently Used eviction policy with:
- Multiple frequency tracking modes
- Adaptive frequency based on access patterns
- Tie-breaking strategies (LRU, FIFO, size-based)

Closes #3
```

### Рефакторинг с архитектурным решением
```
refactor(validator): fix test helpers for Rust 2024

Changed test validators from Input=str to Input=String for Rust 2024
compatibility. The stricter type inference requires sized types since
Option<str> is invalid (can't have Option of unsized type).

Applied systematically across:
- Optional combinator tests
- Or combinator tests
- And combinator tests

Refs #53
```

### Breaking change
```
feat(validator)!: redesign ValidationContext API

BREAKING CHANGE: Removed ValidationContext::simple() method in favor
of builder pattern with new() + insert().

Migration:
- Before: ValidationContext::simple(value)
- After: ValidationContext::new().insert("key", value)

This provides better type safety and extensibility for future features.
```

## Workflow для коммитов

### 1. Подготовка изменений
```bash
# Просмотр изменений
git status
git diff

# Добавление файлов
git add crates/nebula-memory/src/cache/policies/lfu.rs
git add crates/nebula-memory/src/cache/policies/mod.rs
```

### 2. Создание коммита
```bash
# С редактором для длинного сообщения
git commit

# Или напрямую для короткого
git commit -m "fix(cache): implement clear() method for TTL policy"
```

### 3. Проверка перед push
```bash
# Просмотр последнего коммита
git log -1 --stat

# Внесение изменений в последний коммит (если нужно)
git commit --amend

# ВАЖНО: Проверить authorship перед amend!
git log -1 --format='%an %ae'
```

### 4. Push изменений
```bash
# В feature branch
git push origin feature/fix-cache-policies

# Или в main (с осторожностью)
git push origin main
```

## Частые ошибки

### ❌ Слишком общее описание
```
fix: fix bug
refactor: refactor code
update: update files
```

### ✅ Конкретное описание
```
fix(cache): resolve race condition in TTL expiration
refactor(memory): apply Extension Trait pattern to Arena
feat(validator): add Optional combinator for nullable fields
```

### ❌ Прошедшее время
```
added new feature
fixed the bug
```

### ✅ Императивное наклонение
```
add new feature
fix the bug
```

### ❌ Смешивание изменений
```
feat(validator): add Optional combinator and fix TTL tests and update docs
```

### ✅ Один логический блок на коммит
```
feat(validator): add Optional combinator
test(validator): add tests for Optional with nested validators
docs(validator): document Optional combinator usage
```

## Дополнительные ресурсы

- [Conventional Commits](https://www.conventionalcommits.org/)
- [How to Write a Git Commit Message](https://chris.beams.io/posts/git-commit/)
- [Angular Commit Guidelines](https://github.com/angular/angular/blob/main/CONTRIBUTING.md#commit)
