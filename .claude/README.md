# Claude Code Configuration

Эта папка содержит конфигурацию и документацию для работы с Claude Code в проекте Nebula.

## Структура

```
.claude/
├── README.md                    # Этот файл
├── settings.local.json          # Разрешения для автоматизации
├── .claudeignore                # Файлы для исключения из индексации
├── docs/                        # Документация и guidelines
│   ├── coding-standards.md      # Стандарты кода и паттерны
│   ├── commit-guidelines.md     # Правила для git коммитов
│   ├── issue-workflow.md        # Процесс работы с issues
│   ├── refactoring-patterns.md  # Архитектурные паттерны
│   ├── rust-parallel-execution.md # 🚀 Параллельное выполнение операций
│   └── cargo-optimization.md    # 📦 Оптимизация Cargo и производительности
└── commands/                    # Custom slash commands
    ├── fix-issue.md             # /fix-issue <number>
    ├── review-code.md           # /review-code <file>
    └── test-crate.md            # /test-crate <name>
```

## Быстрый старт

### Основной принцип

> **"продолжаем делать реальный рефакторинг правильный а не просто чтобы ошибка исчезла"**

Мы применяем правильные архитектурные решения, а не быстрые патчи.

### Custom Commands

#### `/fix-issue <number>`
Систематически исправить GitHub issue:
```
/fix-issue 53
```

#### `/review-code <path>`
Провести code review файла или модуля:
```
/review-code crates/nebula-validator/src/combinators/
```

#### `/test-crate <name>`
Запустить полное тестирование crate:
```
/test-crate nebula-memory
```

## Документация

### [Coding Standards](docs/coding-standards.md)
- Архитектурные принципы
- Паттерны проектирования (Extension Trait, Type Erasure, RAII)
- Rust 2024 особенности
- Стиль кода

### [Commit Guidelines](docs/commit-guidelines.md)
- Формат коммитов (Conventional Commits)
- Типы изменений (feat, fix, refactor, etc.)
- Примеры правильных коммитов
- Git workflow

### [Issue Workflow](docs/issue-workflow.md)
- Процесс анализа issues
- Создание todo lists
- Документирование решений
- Закрытие issues

### [Refactoring Patterns](docs/refactoring-patterns.md)
- Extension Trait Pattern
- Type Erasure Wrapper
- Scoped Callback (RAII)
- Type-State Builder
- Newtype Pattern
- Visitor Pattern

### [Rust Parallel Execution](docs/rust-parallel-execution.md) 🚀
- **CRITICAL**: Параллельное выполнение операций
- Batch операции cargo
- Concurrent testing стратегии
- Memory-safe координация
- Performance optimization patterns

### [Cargo Optimization](docs/cargo-optimization.md) 📦
- Release profile настройки
- Compilation speed оптимизация
- Binary size минимизация
- Profile-Guided Optimization
- Testing и benchmarking

## Настройка разрешений

В `settings.local.json` можно настроить автоматические разрешения для команд:

```json
{
  "permissions": {
    "allow": [
      "Bash(cargo test:*)",
      "Bash(cargo check:*)",
      "Bash(git commit:*)"
    ],
    "deny": [
      "Bash(rm -rf /*)"
    ],
    "ask": [
      "Bash(git push:*)"
    ]
  }
}
```

### Категории разрешений

- **`allow`** - Выполняются автоматически без запроса
- **`deny`** - Всегда блокируются
- **`ask`** - Всегда требуют подтверждения

### Примеры разрешений

```json
// Rust commands
"Bash(cargo test:*)"
"Bash(cargo check:*)"
"Bash(cargo build:*)"
"Bash(cargo clippy:*)"

// Git operations
"Bash(git commit:*)"
"Bash(git add:*)"
"Bash(gh issue:*)"

// File operations
"Read(**/*.rs)"
"Edit(src/**)"
"Write(tests/**)"

// Tools
"TodoWrite(*)"
"Glob(**)"
"Grep(*)"
```

## .claudeignore

Файл `.claudeignore` исключает файлы из индексации Claude:

```
# Build artifacts
target/
Cargo.lock

# IDE files
.idea/
.vscode/

# Test artifacts
*.profraw
*.log

# Large files
*.csv
data/
```

Это ускоряет работу и снижает использование токенов.

## Применение паттернов

### Extension Trait Pattern
```rust
pub trait ArenaExt<T> {
    fn with<F, R>(&self, f: F) -> R
    where F: FnOnce(&Arena<T>) -> R;
}
```
**Когда**: Эргономика Arc<Mutex<T>>

### Type Erasure Wrapper
```rust
trait DisplayRuleEvaluator: Send + Sync {
    fn evaluate(&self, ctx: &Context) -> Result<bool>;
}
```
**Когда**: Trait не object-safe (associated types)

### Scoped Callback
```rust
impl<T> Arena<T> {
    pub fn scope<F, R>(&mut self, f: F) -> R
    where F: FnOnce(&mut Guard<T>) -> R
}
```
**Когда**: Нужна автоматическая очистка ресурсов

## Workflow

### 1. Начало работы
```bash
# Проверить статус
gh issue list --state open

# Выбрать issue
/fix-issue 53
```

### 2. Разработка
```rust
// Применить правильный паттерн
// Не патчить - рефакторить!
```

### 3. Тестирование
```bash
/test-crate nebula-validator
```

### 4. Review
```bash
/review-code src/combinators/optional.rs
```

### 5. Commit
```bash
git commit -m "fix(validator): apply Type Erasure pattern

Closes #53"
```

## Полезные команды

```bash
# Проверка всего проекта
cargo check --workspace --all-features

# Запуск всех тестов
cargo test --workspace

# Форматирование
cargo fmt --all

# Clippy
cargo clippy --all-features -- -D warnings

# Документация
cargo doc --no-deps --open

# GitHub issues
gh issue list --label high-priority
gh issue view 53
gh issue close 53 --comment "Fixed!"
```

## Ресурсы

- [Claude Code Docs](https://docs.claude.com/claude-code)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/)
- [Conventional Commits](https://www.conventionalcommits.org/)

## Contributing

При добавлении новых паттернов или команд:

1. Документировать в соответствующем файле
2. Добавить примеры использования
3. Обновить этот README

## License

Следует той же лицензии, что и основной проект Nebula.
