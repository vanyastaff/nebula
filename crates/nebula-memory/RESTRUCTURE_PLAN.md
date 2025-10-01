# nebula-memory Restructure Plan

## Текущие проблемы:
1. ❌ Нет папки `core/` - несоответствие с архитектурой других крейтов
2. ❌ `error.rs`, `config.rs` в корне - должны быть в `core/`
3. ❌ `traits/` содержит много специализированных трейтов - нужно разделить на core и integration
4. ❌ Много feature-based папок в корне (`arena/`, `pool/`, `cache/`, `budget/`, `lockfree/`)
5. ❌ `monitoring.rs` в корне - должен быть в integration или observability

## Целевая архитектура:

```
src/
├── core/                           # ✨ НОВОЕ - Базовая функциональность
│   ├── mod.rs
│   ├── error.rs                   # ⬅️ Перенести из src/error.rs
│   ├── config.rs                  # ⬅️ Перенести из src/config.rs
│   ├── traits.rs                  # ✨ НОВОЕ - Базовые трейты (MemoryManager, MemoryUsage)
│   └── types.rs                   # ✨ НОВОЕ - Общие типы и константы
│
├── allocator/                      # ✅ УЖЕ ХОРОШО СТРУКТУРИРОВАНО
│   ├── mod.rs
│   ├── traits.rs                  # Allocator trait
│   ├── error.rs
│   ├── stats.rs
│   ├── bump.rs                    # ✅ Production-ready
│   ├── pool.rs                    # ✅ Production-ready
│   ├── stack.rs                   # ✅ Production-ready with StackConfig
│   ├── system.rs                  # ✅ Optimized
│   ├── manager.rs
│   ├── monitored.rs
│   └── tracked.rs
│
├── syscalls/                       # ✅ УЖЕ СОЗДАНО
│   ├── mod.rs
│   ├── info.rs
│   └── direct.rs
│
├── advanced/                       # ✨ НОВОЕ - Группировка продвинутых возможностей
│   ├── mod.rs
│   ├── arena/                     # ⬅️ Переместить из src/arena/
│   ├── pool/                      # ⬅️ Переместить из src/pool/
│   ├── cache/                     # ⬅️ Переместить из src/cache/
│   ├── budget/                    # ⬅️ Переместить из src/budget/
│   └── lockfree/                  # ⬅️ Переместить из src/lockfree/
│
├── integration/                    # ✨ НОВОЕ - Интеграция с ecosystem
│   ├── mod.rs
│   ├── traits/                    # ⬅️ Реорганизовать src/traits/
│   │   ├── mod.rs
│   │   ├── lifecycle.rs          # ⬅️ Из src/traits/lifecycle.rs
│   │   ├── context.rs            # ⬅️ Из src/traits/context.rs
│   │   ├── factory.rs            # ⬅️ Из src/traits/factory.rs
│   │   ├── observer.rs           # ⬅️ Из src/traits/observer.rs
│   │   ├── isolation.rs          # ⬅️ Из src/traits/isolation.rs
│   │   └── priority.rs           # ⬅️ Из src/traits/priority.rs
│   └── monitoring.rs             # ⬅️ Переместить из src/monitoring.rs
│
├── utils.rs                        # ✅ Оставить как есть
├── macros.rs                       # ✅ Оставить как есть
└── lib.rs                          # 🔧 Обновить imports

## УДАЛИТЬ:
❌ `src/platform/` - уже deprecated в пользу `syscalls/`
❌ `src/traits/advanced_lifecycle.rs` - избыточная сложность
❌ `src/compression/` - не core функциональность
❌ `src/extensions/` - не core функциональность
❌ `src/stats/` - уже есть `allocator/stats.rs`
❌ `src/streaming/` - слишком специфично
```

## План миграции:

### Фаза 1: Создание core/
1. ✅ Создать `src/core/mod.rs`
2. ✅ Переместить `error.rs` в `core/`
3. ✅ Переместить `config.rs` в `core/`
4. ✅ Создать `core/traits.rs` с базовыми трейтами
5. ✅ Создать `core/types.rs`

### Фаза 2: Группировка advanced/
1. ✅ Создать `src/advanced/mod.rs`
2. ✅ Переместить feature-based модули в `advanced/`

### Фаза 3: Реорганизация integration/
1. ✅ Создать `src/integration/mod.rs`
2. ✅ Переместить `traits/` в `integration/traits/`
3. ✅ Переместить `monitoring.rs` в `integration/`

### Фаза 4: Cleanup
1. ✅ Удалить deprecated `platform/`
2. ✅ Удалить ненужные папки
3. ✅ Обновить `lib.rs`
4. ✅ Обновить `prelude`

### Фаза 5: Документация
1. ✅ Обновить README
2. ✅ Обновить rustdoc
3. ✅ Создать migration guide

## Преимущества новой структуры:

✅ **Единообразие** - соответствие архитектуре других крейтов (nebula-value, nebula-resource)
✅ **Чистота** - четкое разделение core/advanced/integration
✅ **Понятность** - легко найти нужную функциональность
✅ **Масштабируемость** - простое добавление новых возможностей
✅ **Backwards compatibility** - можно сохранить re-exports для совместимости
