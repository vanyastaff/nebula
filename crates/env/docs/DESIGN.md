# nebula-env — design

| Field | Value |
|-------|-------|
| **Status** | Stable — leaf cross-cutting primitive |
| **Layer** | Cross-cutting (leaf; depends only on `thiserror`) |
| **Redesign role** | **Not touched** by the post-0092 credential/resource rewrite. Stable foundation: no credential- or resource-crate depends on it (only `nebula-api` / `nebula-storage` / `nebula-log`-dev). |
| **Related** | ADR-0086 (placement + workspace env conventions), AGENTS.md → *Layered Dependency Map* (Cross-cutting row) |

---

## 1. Назначение и границы

`nebula-env` — типизированный кросс-каттинг ридер переменных окружения. Один
парсинг-контракт (`var` / `parse` / `flag` / `list`) для всего workspace вместо
разрозненных `std::env::var(...).unwrap_or_default().parse()` с тонко
различающейся семантикой defaults и bool/int (lib.rs:1-10).

**Владеет:** обязательным/опциональным чтением строк, парсингом любого `FromStr`
с trim, bool-семантикой (`true/1/yes/on` ↔ `false/0/no/off`), сплитом списков по
whitespace+запятым, типом ошибки `EnvError`, и тест-гардом `EnvGuard`.

**ЯВНО НЕ делает:** не хранит конфиг и не строит config-структуры; не маппит
ошибки — консьюмеры конвертируют `EnvError` в свои типы (`ApiConfigError`,
`ProviderError`, …) на границе (lib.rs:1-10); не тянет сторонних зависимостей
кроме `thiserror`; не делает горячей перезагрузки/watch.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `var(name) -> Result<String, EnvError>` — обязательная; `Err` если unset/не-Unicode | reader.rs:13 |
| `var_opt(name) -> Result<Option<String>, EnvError>` — опциональная; `Ok(None)` если unset | reader.rs:27 |
| `parse<T: FromStr>(name) -> Result<Option<T>, EnvError>` — парсинг с trim | reader.rs:38 |
| `parse_or<T>(name, default) -> Result<T, EnvError>` — то же с дефолтом | reader.rs:57 |
| `flag(name) -> Result<Option<bool>, EnvError>` — bool, иначе `Err(Invalid)` | reader.rs:67 |
| `flag_or(name, default) -> Result<bool, EnvError>` | reader.rs:83 |
| `list(name) -> Vec<String>` — сплит по whitespace+запятым, пустые отброшены | reader.rs:90 |
| `enum EnvError` (`#[non_exhaustive]`, thiserror): `Missing` / `NotUnicode` / `Parse{message}` / `Invalid{value, expected}` | error.rs:10 |
| `testing::EnvGuard` (feature `testing` / `cfg(test)`): RAII, process-global Mutex + restore-on-drop | testing.rs:27 |
| `EnvGuard::acquire()` / `set(key, value)` / `remove(key)` | testing.rs:34 / :42 / :56 |

Корневые re-exports: `pub use error::EnvError; pub use reader::{flag, flag_or,
list, parse, parse_or, var, var_opt}` (lib.rs:45-46). `forbid(unsafe_code)` вне
test/testing-сборки (lib.rs:39).

## 3. Зависимости и зависимые

- **Deps:** только `thiserror` (workspace). Feature `testing` (пустая, гейтит `EnvGuard`).
- **Dependents:** `nebula-api` (api/Cargo.toml:15; dev-deps с `testing` :136),
  `nebula-storage` (storage/Cargo.toml:16; dev-deps с `testing` :108),
  `nebula-log` (ТОЛЬКО dev-deps с `testing`, log/Cargo.toml:83).

## 4. Внутренняя архитектура

- `src/lib.rs` — корень: docs, re-exports, гейт `testing`, `forbid(unsafe_code)`.
- `src/reader.rs` — все 7 функций парсинг-контракта.
- `src/error.rs` — единственный тип `EnvError` (4 варианта).
- `src/testing.rs` — `EnvGuard`: `OnceLock<Mutex<()>>` + `HashMap` saved-values;
  единственное место с `unsafe` (edition-2024 `set_var`/`remove_var`).
- `src/tests.rs` — unit-тесты через `EnvGuard`.

Поток данных тривиален: функция читает `std::env`, при наличии значения
trim'ит/парсит/мапит, иначе возвращает `None`/`Err(Missing)`. Состояния нет.

## 5. Инварианты и контракты

- **`forbid(unsafe_code)` вне тестов** (lib.rs:39): единственный `unsafe` —
  `set_var`/`remove_var` в `testing.rs`, гейченный фичей/`cfg(test)` →
  продакшен-поверхность безопасна by-construction.
- **Сериализация env-мутаций в тестах** (testing.rs:27): `EnvGuard` берёт
  process-global `Mutex` и восстанавливает прежние значения на drop → гонок
  между тестами на shared process-env нет by-construction.
- **Типизированные ошибки на границе:** все провалы surface как `EnvError`
  (`#[non_exhaustive]`), консьюмеры маппят его в свои типы — нет утечки
  `std::env::VarError` наружу.
- **bool-контракт фиксирован** (reader.rs:67): нераспознанное значение → явный
  `Err(Invalid)`, а не молчаливый `false`.

## 6. Известные напряжения / долг

1. **Контрактная асимметрия `list`:** молча глотает NotUnicode
   (`unwrap_or_default()`, reader.rs:91-92), тогда как `var`/`var_opt`/`parse`/
   `flag` возвращают `Err(NotUnicode)`. Отличие не задокументировано.
2. **Заявка lib.rs:7-9** о замене дублей «в nebula-api и nebula-log», но
   `nebula-log` подключает крейт лишь как dev-dep (testing) — runtime
   bool/int-чтение env в log не мигрировано на nebula-env (либо у log его нет;
   по Cargo.toml не видно).
3. **AGENTS.md:22** «Cross-crate calls go through nebula-eventbus» — boilerplate
   корневых правил, неприменим к листовому крейту без cross-crate вызовов (шум,
   не баг).
4. **README.md:44 → ADR-0086** как rationale размещения — за пределами крейта,
   не верифицировано в рамках этого fact-sheet.
5. TODO/deprecated/shims — нет; README (таблица API :15-20) и `reader.rs`
   согласованы.

## 7. Роль в пост-0092 credential/resource модели

Не затронут. Ни `nebula-credential` (контракт+runtime+facade+builtin после
консолидации), ни `nebula-resource` (per-slot rotation fan-out, SlotCell,
topology), ни `nebula-storage`-декораторы не зависят от `nebula-env` через него
свои env не читают. Стабильный фундамент: кросс-каттинг leaf, нулевая площадь
контакта с переписанной credential/resource-поверхностью.

## 8. Forward design / открытые вопросы

Крейт стабилен; новой работы не требует. Два возможных будущих движения, оба
опциональны:

- **Выровнять контракт `list`** с остальными функциями (вернуть `Err(NotUnicode)`
  вместо молчаливого глота) либо задокументировать различие — закрыть напряжение №1.
- **Кандидат-инфра для credential-стека:** `reader.rs:6` мотивирует `&str`-имена
  «per-provider OAuth vars». Если узкому типизированному `RefreshTransport`-seam'у
  или provider-конфигу понадобится единое env-чтение, `nebula-env` — естественная
  точка; текущих консьюмеров из credential-стека нет.
