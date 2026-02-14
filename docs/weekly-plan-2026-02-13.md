# Планёрка Nebula -- 2026-02-13

## Участники

| Роль | Статус отчёта |
|------|---------------|
| Product Manager | Готов |
| Tech Director (CTO) | Готов |
| QA Lead | Готов |
| Architect | Готов |

---

## 1. Текущее положение дел

### Масштаб проекта

Целевая архитектура (из `docs/`) описывает **30+ crates**: core, workflow, execution, engine, runtime, action, expression, parameter, validator, telemetry, memory, ports, credential, resource, registry, config, resilience, system, locale, derive, 8 drivers, 4 бинарника, cluster, tenant.

**Реализовано сейчас: 11 crates** из ~30 запланированных. Проект находится на стадии **фундамента**.

### Статус crates

| Crate | Компиляция | Тесты | Оценка |
|-------|-----------|-------|--------|
| **core** | OK | 89 pass, 0 dedicated test files | Работает, но тесты только inline |
| **log** | OK | 43 pass, 1 FAIL (`test_context_snapshot`) | Почти ок |
| **validator** | OK | 449 pass | Отлично |
| **config** | OK | 63 pass | Ок |
| **resilience** | OK | 161 pass | Хорошо |
| **credential** | Частично | 9 compile errors в mock rotation | Сломан после рефакторинга |
| **memory** | **FAIL** | -- | 2 compile errors, блокирует expression |
| **expression** | **FAIL** | -- | Зависит от memory |
| **system** | OK | 13 pass, 1 FAIL doctest | Почти ок |
| **resource** | OK (lib) | Пример сломан | Частично |
| **action** | OK | 0 тестов | Пустой crate |

### Текущая ветка: `009-validator-serde-bridge`

**Статус: ЗАВЕРШЕНА.** Все 6 фаз (66 задач) выполнены, 449 тестов зелёные. Фича добавляет мост между `serde_json::Value` и системой валидации -- позволяет валидировать произвольный JSON теми же комбинаторами, что и типизированные структуры.

Также на ветке коммит `1b4f541` -- переименование директорий `crates/nebula-X/` -> `crates/X/`.

---

## 2. Критические проблемы

### P0: `nebula-memory` не компилируется

- `object_pool.rs:273` -- вызов `drain()` на `RefCell` вместо `RefCell::borrow_mut().drain()`
- `hierarchical.rs:118` -- одновременный mutable + immutable borrow self (Rust 2024 ужесточение)
- **Каскад**: блокирует `nebula-expression`, ломает `cargo test --workspace`

### P0: `nebula-credential` -- 9 compile errors

- `rotation/validation.rs` -- lifetime mismatch в `MockCredential` с `#[async_trait]`
- Вероятно, побочный эффект рефакторинга 008 (serde_json migration)

### P1: Security -- bytes CVE

- `RUSTSEC-2026-0007`: integer overflow в `BytesMut::reserve` (bytes 1.11.0)
- Fix: обновить до bytes >= 1.11.1

### P1: `serde_yaml` deprecated upstream

- Используется в `nebula-resource` (production) и `nebula-log` (dev)

---

## 3. Архитектурные наблюдения

### Architect

- Слоёвая модель из CLAUDE.md не совпадает с реальным графом зависимостей
- `resource` (System) зависит от `credential` (Domain) -- нарушение направления
- 3 разных Validatable trait в трёх crates (core, validator, config) -- дублирование
- `core::Cloneable`, `core::Comparable` и т.д. -- бесполезные обёртки над std traits
- `nebula-action` -- полностью пустой crate, балласт в workspace
- 0 циклических зависимостей -- граф корректный DAG

### Tech Director

- `winapi` 0.3.9 устарел (замена -- `windows` crate)
- `check_output.txt` в корне -- артефакт отладки, не в `.gitignore`
- Feature `serde` в validator включен по умолчанию, хотя spec описывал opt-in `serde-json`
- `nebula-log` не использует workspace dependencies для некоторых crates

---

## 4. QA: тестовое покрытие

| Метрика | Текущее | Оценка |
|---------|---------|--------|
| Crates без тестов | 4 (core, action, config\*, system\*) | Критично |
| Property tests (proptest) | 0 (хотя 4 crates имеют proptest в dev-deps) | Критично |
| Async cancellation tests | 0 (при 90 `tokio::select!` в коде) | Высокий риск |
| Rustdoc warnings | 23+ unresolved links | Плохо |
| Deprecated API в коде | criterion::black_box (134 использования) | Средне |
| cargo audit | 1 CVE (bytes) | Требует fix |

---

## 5. План на неделю (13-19 февраля)

### День 1 (чт 13.02): Разблокировать CI

| # | Задача | Оценка |
|---|--------|--------|
| 1 | Fix `memory/object_pool.rs:273` -- `.borrow_mut()` | 15 мин |
| 2 | Fix `memory/hierarchical.rs:118` -- restructure borrow | 1-2 часа |
| 3 | Fix `credential/rotation/validation.rs` -- 9 async_trait methods | 1-2 часа |
| 4 | `cargo fmt --all` | 1 мин |
| 5 | Update `bytes` to >= 1.11.1 | 5 мин |
| 6 | Verify: `cargo clippy --workspace -- -D warnings` PASS | -- |

**Цель**: зелёный CI к концу дня.

### День 2 (пт 14.02): Merge 009 + cleanup

| # | Задача | Оценка |
|---|--------|--------|
| 7 | Удалить `check_output.txt`, добавить в `.gitignore` | 5 мин |
| 8 | Обновить spec 009 (JsonPath -> RFC 6901, feature naming) | 30 мин |
| 9 | Code review + merge `009-validator-serde-bridge` в main | 1 час |
| 10 | Fix log examples (unused imports, dead code) | 15 мин |
| 11 | Fix config example (unused variable) | 5 мин |
| 12 | Fix deprecated `criterion::black_box` -> `std::hint::black_box` | 30 мин |

### День 3 (пн 17.02): Стабилизация тестов

| # | Задача | Оценка |
|---|--------|--------|
| 13 | Fix `log::test_context_snapshot` | 30 мин |
| 14 | Fix `system::format_duration` doctest | 15 мин |
| 15 | Fix `resource` example (missing `fn main()`) | 10 мин |
| 16 | Fix rustdoc warnings (23+ unresolved links) | 1-2 часа |
| 17 | Rename conflicting example targets (4 пары) | 30 мин |

### День 4 (вт 18.02): Покрытие критических crates

| # | Задача | Оценка |
|---|--------|--------|
| 18 | Написать тесты для `nebula-core` (ID, scope, traits) | 2-3 часа |
| 19 | Начать proptest для validator JSON roundtrip | 1-2 часа |
| 20 | Начать async cancellation test (1 crate) | 1-2 часа |

### День 5 (ср 19.02): Планирование следующей фичи

| # | Задача | Оценка |
|---|--------|--------|
| 21 | Spec для `010-config-validator-migration` (замена schema.rs) | 2-3 часа |
| 22 | Обновить `docs/architecture-overview.md` -- реальная vs целевая архитектура | 1 час |
| 23 | Решить судьбу `nebula-action` (implement or exclude) | Обсуждение |

---

## 6. Открытые вопросы

1. **Merge стратегия для 009**: на ветке коммит переименования директорий -- rebase или merge commit?
2. **Слоёвая модель**: обновить CLAUDE.md под фактический граф зависимостей?
3. **Следующая большая фича**: `010-config-validator-migration` или начинать crates из целевой архитектуры (workflow, execution, engine)?
4. **`nebula-action`**: убрать из workspace до готовности или оставить как placeholder?
5. **`serde_yaml`**: мигрировать сейчас или отложить?

---

## 7. Метрики контроля

| Метрика | Сейчас | Цель (к среде 19.02) |
|---------|--------|----------------------|
| Compile errors | 11 | 0 |
| Crates без тестов | 4 | 2 (action допустимо) |
| Падающие тесты | 3+ | 0 |
| `cargo audit` warnings | 1 CVE | 0 |
| Rustdoc warnings | 23+ | 0 |
| Фича 009 в main | Нет | Да |

---

**Итог**: проект в фазе стабилизации фундамента. Последние рефакторинги (008 serde migration, 007 memory, переименование crates) оставили технический долг. Приоритет #1 -- вернуть CI в зелёное состояние, приоритет #2 -- merge готовой фичи 009, приоритет #3 -- начать движение к следующим crates из целевой архитектуры.
