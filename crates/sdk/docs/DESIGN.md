# nebula-sdk — design

| Field | Value |
|-------|-------|
| **Status** | Partial — пассивный re-export-фасад (только re-exports + builders + test harness; кода миграции нет) |
| **Layer** | Публичная поверхность workspace (по решению 2026-06-10 — единственный публичный крейт) |
| **Redesign role** | **Затронут** (touched). Сам по фактам пока пассивен — но именно он фиксирует итоговую публичную поверхность; `prelude` жёстко перечисляет credential-типы v2 и `Resource`/`ResourceMetadata`, поэтому rewrite credential и resource-redesign ломают этот список напрямую. |
| **Related** | `prelude.rs` (P11 re-export audit), README.md §«Credential, OAuth, and the SDK», решение sole-public-crate 2026-06-10, ветка resource-redesign `dreamy-kare-8698d4` (не смержена) |

---

## 1. Назначение и границы

`nebula-sdk` — единый фасад для авторов интеграций. Цель: автор узла Nebula не должен
знать, какие из девяти+ workspace-крейтов добавлять в `Cargo.toml` — он подключает один
крейт и получает action-трейты, schema-типы, credential-модель, resource-модель,
workflow-builder и test harness (`README.md:14-20`). По решению 2026-06-10 это
**единственный публичный крейт** workspace — всё остальное приватная impl-detail.

**Владеет (собственный код, не re-export):**
- SDK-уровневый `Error` (`Workflow`/`Action`/`Parameter`/`Serialization`/`Other`) + `Result<T>` — `src/lib.rs:73-114`.
- Декларативные макросы `params!` (`src/lib.rs:143-155`), `workflow!` (`src/lib.rs:179-200`), `simple_action!` (`src/lib.rs:235-277`).
- `ActionBuilder` — программная сборка `ActionMetadata` (`src/action.rs:33-73`) + `action::helpers::{validate_schema, parse_input}` (`src/action.rs:108-139`).
- `WorkflowBuilder` → `WorkflowDefinition` с валидацией рёбер и dedup node-id (`src/workflow.rs:43-274`).
- `TestRuntime`/`RunReport` — in-process прогон четырёх видов action (`src/runtime.rs:58-307`).
- `testing::{is_success, is_failure, assert_success, assert_failure, fixtures}` под feature `testing` (`src/testing.rs:22-75`).
- `prelude` — one-stop импорт (~100 строк re-exports, `src/prelude.rs`).
- Вложенный крейт `nebula-macro-support` (`crates/sdk/macros-support/`) — общие proc-macro утилиты для всех `*-macros`.

**ЯВНО НЕ делает (из non-goals README + фактов):**
- Не движок и не runtime — это крейт для *написания* интеграций, не для прогона исполнений (`README.md:90-91`; см. `nebula-engine`).
- Не вычислитель выражений (см. `nebula-expression`, `README.md:92`).
- Не вводит шестую интеграционную концепцию — пять остаются Action/Credential/Resource/Schema/Plugin (`README.md:77-79`).
- Не re-export `nebula-resilience` — resilience-пайплайны собираются на месте вызова action (`README.md:94-95`).
- Не делает HTTP token-exchange/refresh, storage-encryption, engine `CredentialResolver` — это product/runtime-крейты (`README.md:52`).

## 2. Публичная поверхность

| Item | Где |
|------|-----|
| Re-export целых крейтов: `nebula_action/_core/_credential/_plugin/_resource/_schema/_validator/_workflow` + `serde`, `serde_json`, `thiserror` | `src/lib.rs:47-57` |
| `tokio` re-export под feature `testing` | `src/lib.rs:60` |
| `Error` (5 вариантов) + `Result<T>` | `src/lib.rs:73-114` |
| `pub use serde_json::json` | `src/lib.rs:128` |
| `params!` (`FieldValues` из kv-пар через `try_set_raw` + expect) | `src/lib.rs:143-155` |
| `workflow!` (DSL поверх `WorkflowBuilder`) | `src/lib.rs:179-200` |
| `simple_action!` (unit-struct + impl `Action` + `StatelessAction`) | `src/lib.rs:235-277` |
| `ActionBuilder` (`key`/`name`/`description`/`version` → `ActionMetadata`) | `src/action.rs:33-73` |
| `action::helpers::{validate_schema, parse_input}` | `src/action.rs:108-139` |
| `WorkflowBuilder` (`add_node`/`add_node_with_params`/`connect`/`with_variable`/`build`) | `src/workflow.rs:43-274` |
| `TestRuntime` (`run_stateless`/`run_stateful`/`run_poll`/`run_webhook`; knobs `with_stateful_cap`, `with_trigger_window`) | `src/runtime.rs:87-307` |
| `RunReport` (`kind`/`output`/`iterations`/`duration`/`emitted`/`note`/`health`) | `src/runtime.rs:58-77` |
| `testing::{is_success, is_failure, assert_success, assert_failure, fixtures}` (feature `testing`) | `src/testing.rs:22-75` |
| `prelude`: action-трейты+адаптеры+spy-тестхарнесс | `src/prelude.rs:15-32` |
| `prelude`: credential-типы v2 (`ApiKeyCredential`, `OAuth2Credential`, `OAuth2Token`, `CredentialContext`, `CredentialSnapshot`, …) | `src/prelude.rs:37-58` |
| `prelude`: metadata-словарь, `Resource`/`ResourceMetadata`, schema-поля, validator, workflow-типы | `src/prelude.rs:64-88` |
| `nebula-macro-support`: `attrs`, `credential_ref`, `diag`, `utils`, `validation_codegen` | `macros-support/src/lib.rs:9-18` |

## 3. Зависимости и зависимые

- **Deps** (`Cargo.toml:16-24`): `nebula-core`, `nebula-action`, `nebula-metadata`, `nebula-workflow`,
  `nebula-schema`, `nebula-credential`, `nebula-plugin`, `nebula-resource`, `nebula-validator`;
  плюс `tokio`/`serde`/`serde_json`/`thiserror`/`uuid`/`chrono`.
- **Features**: `default = ["derive", "testing"]`; `derive = []` — **пустой** (ничего не гейтит);
  `testing` гейтит `src/testing.rs` + re-export `tokio` (`Cargo.toml:48` и далее).
- **Обратные зависимости: НОЛЬ.** Ни один `Cargo.toml` workspace не зависит от `nebula-sdk`;
  `nebula_sdk::` не используется нигде вне самого крейта — даже в `examples/`.
- **`nebula-macro-support`** (тот же каталог) — наоборот, потребляется 5 macros-крейтами:
  action/resource/plugin/credential/validator macros (`crates/*/macros/Cargo.toml:19-20`).

## 4. Внутренняя архитектура

Поток данных тривиален — фасад без runtime-логики, агрегирующий чужие типы:

- `src/lib.rs` — re-exports целых крейтов, SDK `Error`, три макроса.
- `src/prelude.rs` — единая точка `use nebula_sdk::prelude::*` (~100 строк re-exports).
- `src/action.rs` — `ActionBuilder` (метаданные) + `helpers` (самодельная required-проверка).
- `src/workflow.rs` — `WorkflowBuilder` собирает узлы/рёбра/переменные → `WorkflowDefinition`,
  валидирует рёбра и дедуплицирует node-id на `build`.
- `src/runtime.rs` — `TestRuntime` гоняет stateless/stateful/poll/webhook action in-process,
  отдаёт `RunReport`.
- `src/testing.rs` — assert-хелперы + fixtures (feature `testing`).
- `macros-support/` — отдельный крейт `nebula-macro-support` (syn/quote утилиты для всех `*-macros`).
- `tests/simple_action_macro.rs` — интеграционный тест макроса.

## 5. Инварианты и контракты

- **[L1-§3.5]** SDK-поверхность покрывает пять интеграционных концепций (Action/Credential/Resource/Schema/Plugin)
  и не вводит новых; шестая требует ревизии канона (`README.md:77-79`).
- **[L1-§4.4 / §7]** DX и публичная стабильность — first-class контракт: ломающие изменения
  `prelude` / `WorkflowBuilder` касаются всех авторов интеграций и требуют явного анонса
  и migration-guide, а не drive-by-коммитов (`README.md:81-86`).
- **P11 re-export audit (by-construction):** SDK не вводит второй OAuth-фасад поверх
  `nebula-credential` и не добавляет параллельных OAuth-алиасов — credential-типы берутся
  либо из полного крейта `nebula_sdk::nebula_credential`, либо из `prelude` (`README.md:43-54`).
- **`WorkflowBuilder` ordering:** `build` валидирует, что обе вершины ребра существуют,
  и дедуплицирует node-id (`src/workflow.rs:43-274`) — выход `WorkflowDefinition` структурно консистентен.

## 6. Известные напряжения / долг (честно)

1. **README vs код — ложь про `anyhow`.** `README.md:104-105` утверждает «`anyhow` is re-exported» —
   но `anyhow` нет ни в `Cargo.toml`, ни в `lib.rs`. Документация противоречит коду.
2. **Устаревшее имя трейта.** `README.md:69` называет `simple_action!` обёрткой над `ProcessAction`;
   макрос реально реализует `StatelessAction` (`src/lib.rs:264`).
3. **Мёртвый feature-флаг.** `derive = []` (`Cargo.toml:48`) ничего не включает, но сидит в `default`.
4. **Асимметрия `nebula_metadata`.** Он есть в deps и в `prelude` (`src/prelude.rs:64`), но НЕ
   re-export целым крейтом в `src/lib.rs:47-56` (в отличие от остальных 8) и не упомянут
   в README-списке re-exports.
5. **Дубль конструирования workflow.** В `prelude` два билдера: свой
   `workflow::WorkflowBuilder` (`src/prelude.rs:98`) и `nebula_workflow::WorkflowBuilder as CoreWorkflowBuilder`
   (`src/prelude.rs:86`) — две точки сборки одного и того же.
6. **Самодельная schema-проверка.** `action::helpers::validate_schema` (`src/action.rs:108-120`)
   — ручная JSON-schema проверка required-полей с коммментом «in production, use jsonschema crate»;
   дублирует/обходит пайплайн `nebula-schema`/`nebula-validator`, ошибки — голые `String`.
7. **Сомнительная грамматика `workflow!`.** Макрос принимает `$action:ty` и тут же `stringify!`-ит его
   (`src/lib.rs:183,192`) — тип используется как строка; doc-пример при этом передаёт строковые
   литералы туда, где ожидается `ty`.
8. **Тихая подмена node-id.** `WorkflowBuilder::build` молча заменяет невалидный node-id на `node_{i}`
   (`src/workflow.rs:177-182`): пользовательские `connect()` по исходному имени работают, но итоговый
   `NodeKey` тихо другой.
9. **Спорная топология `nebula-macro-support`.** Крейт лежит ВНУТРИ `crates/sdk/`, при том что его
   потребляют 5 macros-крейтов из других подсистем — sdk-каталог как хост общей proc-macro утилиты.
10. **Zero-consumer фасад.** `examples/` не используют `nebula_sdk` — «единственный публичный крейт»
    без единого внутреннего/example-потребителя; контракт пока не проверяется реальным использованием.
11. **Шаблонная фраза про eventbus.** `AGENTS.md:25` «Cross-crate calls go through nebula-eventbus»
    бессмысленна для re-export-фасада без runtime-логики.

## 7. Роль в пост-0092 credential/resource модели

`nebula-sdk` — итоговая **публичная поверхность** консолидации, поэтому он затронут не как
носитель логики, а как место, где фиксируется контракт того, что видит автор интеграции.

- **Credential rewrite (одно-крейтная `nebula-credential`).** После ADR-0092 `nebula-credential`
  = contract + runtime (resolver/refresh/lease/rotation-state) + `CredentialService` facade +
  builtin-типы; крейты credential-runtime/builtin/testutil/vault удалены. SDK уже re-export-ит
  `nebula_credential` целиком (`src/lib.rs:47-57`), поэтому ликвидация под-крейтов за фасадом
  *не* меняет внешний путь `nebula_sdk::nebula_credential`. **Что ломается:** `prelude.rs:37-58`
  жёстко перечисляет credential-типы v2 (`OAuth2Credential`, `OAuth2Token`, `CredentialSnapshot`,
  `CredentialContext`, `CredentialRecord`…). Переход на Protocol/scheme-enum и `policy(&State)`-routing
  поменяет/переименует эти типы, и список в `prelude` придётся пересобирать под новый набор.
- **Consumer-binding seam.** Авторы объявляют `#[credential]`-слоты и получают `CredentialGuard<Scheme>`;
  слоты (`slot_bindings`) отделены от параметров, persistence — values-only (schema берётся из
  зарегистрированных типов через `HasSchema → nebula-metadata → API catalog`). SDK — это место,
  где этот seam становится виден автору: prelude должен экспонировать guard-тип и authoring-трейты,
  оставаясь по P11-аудиту единственным OAuth-фасадом (без параллельных алиасов, `README.md:54`).
  Per-tenant изоляция (`OwnerScopedKey`), narrow typed `RefreshTransport` и first-class lease живут
  в `nebula-credential`/`nebula-storage` — SDK их не оборачивает, только переэкспортирует.
- **Resource redesign.** `prelude.rs:67` re-export-ит `Resource`/`ResourceMetadata`. Redesign
  (`Resource` = 2 associated types; per-slot rotation fan-out переехал в `nebula-resource`;
  ветка `dreamy-kare-8698d4` не смержена) изменит сигнатуры этих re-exports — prelude обновляется
  вслед за стабилизацией ветки. Fan-out/SlotCell/Manager/topology остаются внутри `nebula-resource`;
  SDK их не раскрывает как механику, только как authoring-типы.
- **Что остаётся неизменным.** Сам крейт пассивен — кода миграции в нём нет, и коллапс
  crate-топологии за фасадом не начат. Путь «full crate» (`nebula_sdk::nebula_credential` /
  `::nebula_resource`) переживает консолидацию by-construction; меняется только курируемый
  список `prelude`.
- **Не входит в SDK (подтверждено redesign-ground-truth).** Engine-bridges + `default_in_memory_coordinator`
  остаются в `nebula-engine`; durable-stores + Encryption/Cache/Audit-декораторы + `KeyProvider` +
  `RefreshClaimRepo` — в `nebula-storage`. SDK не должен их re-export-ить — это runtime, не authoring.

## 8. Forward design / открытые вопросы

- **Сделать `prelude` единственным курируемым швом.** При переходе credential на Protocol/scheme-enum
  обновить `prelude.rs:37-58` атомарно с rewrite credential и зафиксировать новый список в README
  (P11-аудит) — иначе фасад начнёт расходиться с `nebula-credential` так же, как уже разошёлся
  с README по `anyhow`.
- **Закрыть README-долг до первого внешнего релиза.** Убрать ложь про `anyhow` (#1), переименовать
  `ProcessAction` → `StatelessAction` (#2), добавить `nebula_metadata` в список re-exports или
  убрать асимметрию (#4) — это публичная документация единственного публичного крейта.
- **Решить судьбу самодельной schema-проверки.** `action::helpers::validate_schema` (#6) либо снести
  в пользу `nebula-schema`/`nebula-validator`, либо явно задокументировать как тест-only — голые
  `String`-ошибки несовместимы с типизированным error-контрактом workspace.
- **Снять дубль workflow-билдеров** (#5): оставить один путь конструирования в `prelude`,
  чтобы не было двух семантик `WorkflowBuilder`.
- **Пересмотреть топологию `nebula-macro-support`** (#9): крейт с 5 потребителями из других
  подсистем — кандидат на вынос из `crates/sdk/` в нейтральный support-каталог; это boundary-решение,
  а не косметика.
- **Чинить тихую подмену node-id** (#8): `build` должен возвращать ошибку на невалидный node-id,
  а не молча давать другой `NodeKey` — иначе `connect()`-семантика обманчива.
- **Главный риск.** Zero-consumer (#10): пока ни `examples/`, ни внутренние крейты не используют
  `nebula_sdk`, контракт «единственного публичного крейта» не подтверждён реальным потреблением —
  ломающие изменения за фасадом могут пройти незамеченными. Первый шаг — перевести хотя бы один
  example на `nebula_sdk::prelude::*`.
