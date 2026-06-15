# nebula-env — fact sheet

## Назначение
Типизированный кросс-каттинг ридер переменных окружения: единый парсинг-контракт
(`var`/`parse`/`flag`/`list`) для всего workspace вместо разрозненных
`std::env::var(...).unwrap_or_default().parse()` в крейтах. Зависимости — только
`std` + `thiserror`; ошибки консьюмеры маппят в свои типы на границе (lib.rs:1-10).

## Публичная поверхность
- `var(name) -> Result<String, EnvError>` — обязательная переменная; `Err` если unset/не-Unicode — reader.rs:13
- `var_opt(name) -> Result<Option<String>, EnvError>` — опциональная; `Ok(None)` если unset — reader.rs:27
- `parse<T: FromStr>(name) -> Result<Option<T>, EnvError>` — парсинг с trim; `Ok(None)` если unset — reader.rs:38
- `parse_or<T>(name, default) -> Result<T, EnvError>` — то же с дефолтом — reader.rs:57
- `flag(name) -> Result<Option<bool>, EnvError>` — bool: `true/1/yes/on` | `false/0/no/off` (case-insensitive), иначе `Err(Invalid)` — reader.rs:67
- `flag_or(name, default) -> Result<bool, EnvError>` — reader.rs:83
- `list(name) -> Vec<String>` — сплит по whitespace+запятым, пустые отброшены; пустой Vec если unset — reader.rs:90
- `enum EnvError` (`#[non_exhaustive]`, thiserror): `Missing` / `NotUnicode` / `Parse{message}` / `Invalid{value, expected}` — error.rs:10
- `testing::EnvGuard` (feature `testing` или `cfg(test)`): RAII-гард, process-global Mutex + restore-on-drop — testing.rs:27
  - `EnvGuard::acquire()` testing.rs:34, `set(key, value)` testing.rs:42, `remove(key)` testing.rs:56
- Корневые re-exports: `pub use error::EnvError; pub use reader::{flag, flag_or, list, parse, parse_or, var, var_opt}` — lib.rs:45-46
- `forbid(unsafe_code)` вне test/testing-сборки — lib.rs:39

## Workspace-зависимости
Deps: только `thiserror` (workspace). Feature `testing` (пустая, гейтит `EnvGuard`).
Кто зависит (crates/*/Cargo.toml):
- `nebula-api` — [dependencies] (api/Cargo.toml:15) + dev-deps с `testing` (:136)
- `nebula-storage` — [dependencies] (storage/Cargo.toml:16) + dev-deps с `testing` (:108)
- `nebula-log` — ТОЛЬКО dev-deps с `testing` (log/Cargo.toml:83) — рантайм-парсинг env в log не через nebula-env

## Структура модулей
- `src/lib.rs` — корень: docs, re-exports, гейт `testing`, `forbid(unsafe_code)` для не-test сборок
- `src/reader.rs` — все 7 функций парсинг-контракта
- `src/error.rs` — единственный тип `EnvError` (4 варианта)
- `src/testing.rs` — `EnvGuard`: OnceLock<Mutex<()>> + HashMap saved-values; единственное место с `unsafe` (edition-2024 set_var/remove_var)
- `src/tests.rs` — unit-тесты через `EnvGuard`

## Напряжения
- Контрактная асимметрия `list`: молча глотает NotUnicode (`unwrap_or_default()`, reader.rs:91-92), тогда как `var`/`var_opt`/`parse`/`flag` возвращают `Err(NotUnicode)`. Не задокументировано как отличие.
- lib.rs:7-9 заявляет замену дублей «в nebula-api и nebula-log», но nebula-log подключает крейт только как dev-dep (testing) — runtime bool/int-хелперы log не мигрированы на nebula-env (или у log нет runtime env-чтения; по Cargo.toml не видно).
- AGENTS.md:22 «Cross-crate calls go through nebula-eventbus» — boilerplate из корневых правил, неприменим к листовому крейту без cross-crate вызовов; шум, не баг.
- TODO/deprecated/shims — нет. README и код согласованы (таблица API README.md:15-20 соответствует reader.rs).
- README.md:44 ссылается на ADR-0086 как rationale размещения — за пределами крейта, не проверял.

## Роль в credential/resource redesign
Крейт не затронут redesign'ом напрямую: ни credential-, ни resource-крейты от него
не зависят (только api/storage/log). Косвенная роль возможна как кандидат-инфра для
env-чтения provider/OAuth-переменных (reader.rs:6 явно упоминает «per-provider OAuth
vars» как мотив `&str`-имен), но текущих консьюмеров из credential-стека нет.
