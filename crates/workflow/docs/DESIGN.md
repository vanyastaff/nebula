# nebula-workflow — design

| Field | Value |
|-------|-------|
| **Status** | Stable — Core-layer authoring + validation primitive |
| **Layer** | Core (depends only on `nebula-core` + `nebula-error`; no upward deps) |
| **Redesign role** | **Touched, but only as data carrier** — `NodeDefinition.slot_bindings` is the author-side hybrid binding surface that the post-0092 bind-population path consumes. No runtime, resolver, or facade lives here; the redesign flows *through* this crate, it is not *built* in it. |
| **Related** | [PRODUCT_CANON](../../../docs/PRODUCT_CANON.md) §10 / §12.2, ADR-0092, ADR-0052 (ValidatedCredentialBinding cascade, lives above this crate), `nebula-execution` / `nebula-engine` consumers |

---

## 1. Назначение и границы

`nebula-workflow` — Core-слой: единое serde-round-trippable представление того,
что предстоит исполнить. Без общего типа определения каждый верхний слой (API,
storage, engine, validation) парсил бы и переинтерпретировал workflow-JSON
самостоятельно, давая молчаливые расхождения между тем, что оператор сохранил, и
тем, что движок запускает.

**Владеет:**
- `WorkflowDefinition` — узлы + соединения + конфиг + UI-метаданные, гарантированно
  переживающие `serde_json` round-trip (`definition.rs:16`).
- `DependencyGraph` на `petgraph` — топосортировка и уровни параллелизма, которые
  питают `ExecutionPlan` в `nebula-execution` (`graph.rs:15`).
- `WorkflowBuilder` — fluent-сборка с fail-fast валидацией в `build()` (`builder.rs:19`).
- `validate_workflow` — shift-left валидатор активации (canon §10/§12.2), собирающий
  **все** ошибки в `Vec<WorkflowError>` (`validate.rs:52`), и newtype-witness
  `ValidatedWorkflow` как единственное доказательство «провалидировано» (`validate.rs:204`).

**ЯВНО НЕ делает:**
- НЕ исполняет DAG — `DependencyGraph` лишь считает топологический порядок;
  планирование — забота `nebula-engine`.
- НЕ резолвит выражения — `ParamValue::Expression`/`Template`/`Reference` несут
  нерезолвленные строки; вычисление — это `nebula-expression`.
- НЕ хранит — JSON-персист и состояние активации живут в `nebula-storage`/`nebula-api`.
- НЕ является execution state machine — `ExecutionStatus`/`ExecutionState` в `nebula-execution`.
- НЕ валидирует существование id-шников слотов/действий — нет доступа к реестрам.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `WorkflowDefinition`; `is_schema_supported()`; `CURRENT_SCHEMA_VERSION = 1` | `definition.rs:16 / :63 / :12` |
| `WorkflowConfig` (timeout, max_parallel_nodes, checkpointing, retry_policy, error_strategy) | `definition.rs:111` |
| `ErrorStrategy` (`FailFast` \| `ContinueOnError` \| `IgnoreErrors`) | `definition.rs:98` |
| `TriggerDefinition` (`Manual` \| `Cron` \| `Webhook` \| `Event`) | `definition.rs:72` |
| `RetryConfig` (`fixed` / `exponential` / `delay_for_attempt`) | `definition.rs:171` |
| `CheckpointingConfig` / `UiMetadata` / `NodePosition` / `Viewport` / `Annotation` | `definition.rs:147 / :220 / :234` |
| `NodeDefinition` (id: `NodeKey`, action_key: `ActionKey`, interface_version: `Option<semver::Version>`, parameters, retry_policy, timeout, enabled, rate_limit) | `node.rs:13` |
| `NodeDefinition.slot_bindings: HashMap<String, SlotBinding>` | `node.rs:55` |
| `SlotBinding` (`ResourceId(String)` \| `CredentialId(String)`) + helpers `with_*_binding` / `*_binding` | `node.rs:219 / :119 / :131 / :147 / :159` |
| `ParamValue` (`Literal` \| `Expression` \| `Template` \| `Reference`) — НЕ резолвится здесь | `node.rs:230` |
| `RateLimit` | `node.rs:71` |
| `Connection` (from_node, to_node, from_port, to_port — «чистый провод»); `effective_from_port()` → `"main"` | `connection.rs:45 / :98` |
| `DependencyGraph`: `from_definition` / `topological_sort` / `compute_levels` (Kahn) / `entry_nodes` / `exit_nodes` / `validate` | `graph.rs:24 / :63 / :75 / :148 / :163 / :203` |
| `WorkflowBuilder`: `connect_via` (port-driven, замена `connect_with_condition`) / `build()` (fail-fast) | `builder.rs:94 / :174` |
| `validate_workflow(&WorkflowDefinition) -> Vec<WorkflowError>` | `validate.rs:52` |
| `ValidatedWorkflow` (witness, единственный конструктор `validate()`) | `validate.rs:204` |
| `WorkflowError` (thiserror + `nebula_error::Classify`, коды `WORKFLOW:*`, 15 вариантов, все `category="validation"`) | `error.rs:9` |
| `NodeState` (`Pending`..`WaitingRetry`, `is_terminal` и пр.) | `state.rs:9` |
| `Version` (собственный semver-подобный тип версии workflow) | `version.rs:7` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-core` (path), `nebula-error` (workspace, feature `derive`);
  внешние — `petgraph`, `semver`, `serde`, `serde_json`, `thiserror`, `chrono`.
  Dev: `insta`, `pretty_assertions`, `rstest`. `[lib] doctest = false`.
- **Dependents (6):** `nebula-engine`, `nebula-action`, `nebula-sdk`,
  `nebula-execution`, `nebula-api`, `nebula-plugin`.

## 4. Внутренняя архитектура

Девять модулей + `lib.rs`, разрезанные по ответственности:
`definition` (данные), `node` (узел + `ParamValue` + `SlotBinding` + `RateLimit`),
`connection` (порт-ориентированный провод + таблица активации `ActionResult → port`
в module doc), `graph` (структура на `petgraph`), `validate` (контракт активации),
`builder` (DX-сборка), `state` (прогресс выполнения узла), `error` (`WorkflowError`),
`version` (собственный `Version`). `lib.rs:47` делает `pub(crate)` re-export
`nebula_core::serde_helpers::duration_opt_ms` как `serde_duration_opt`.

Поток данных: автор/билдер → `WorkflowDefinition` → (а) `validate_workflow`/
`ValidatedWorkflow` как ворота активации; (б) `DependencyGraph::from_definition` →
топосорт + уровни → `ExecutionPlan` (вниз, в `nebula-execution`). Сам крейт —
без I/O, без async, чистые трансформации данных.

## 5. Инварианты и контракты

- **[L2-§10] `validate_workflow` — каноническое окно активации.** API-handler,
  включающий workflow без вызова этой функции, нарушает golden path шаг 2.
  Принуждение к вызову живёт в `nebula-api`; сам крейт владеет функцией
  (`validate.rs:52`).
- **[L2-§12.2] Shift-left.** Структурные ошибки (висячие соединения, дубли id,
  циклы) отвергаются на активации структурированными `WorkflowError` (RFC 9457
  через `Classify`), не откладываются до runtime-dispatch.
- **Witness by-construction.** `ValidatedWorkflow` нельзя сконструировать иначе как
  через `validate()` (`validate.rs:204`) — «провалидировано» доказуемо типом, а не
  дисциплиной вызова.
- **Edges без условий.** `Connection` — чистый провод; условная/error-маршрутизация
  вынесена в явные `ControlAction`-узлы (Spec 28 §2.2). Failed-узлы активируют только
  рёбра с `from_port == "error"`. Pre-Spec-28 trio `EdgeCondition`/`ResultMatcher`/
  `ErrorMatcher` удалён.
- **JSON round-trip как public surface.** `WorkflowDefinition` обязан переживать
  `serde_json` round-trip без потерь; `CURRENT_SCHEMA_VERSION = 1` + `is_schema_supported()`
  — версионный gate (`definition.rs:12/:63`).
- **Все коды — `category="validation"`.** 15 вариантов `WorkflowError`, единый класс —
  крейт не вводит runtime/io-ошибок (`error.rs:9`).

## 6. Известные напряжения / долг (честно)

1. **Стейл-клейм о `panic!`.** `README.md:90-91` и `AGENTS.md:26` утверждают «4
   `panic!` sites в lib-коде как builder-invariant guards / debt» — фактически все 4
   `panic!` в `node.rs` лежат внутри `#[cfg(test)]` (`node.rs:348,359,368,384`);
   lib-код чист. Документацию надо привести в соответствие.
2. **Doc/код расхождение в билдере.** `builder.rs:153` doc говорит «Returns
   `WorkflowError::EmptyName` if owner_id is empty», а код возвращает `InvalidOwnerId`
   (`builder.rs:157`).
3. **Два типа Version.** Собственный `crate::Version` для версии workflow
   (`version.rs:7`) сосуществует с `semver::Version` для `interface_version` узла
   (`node.rs:26`). Гибкий пиннинг (`VersionReq`) отложен — spec
   `2026-04-17-replace-interfaceversion-with-semver`.
4. **Артефакты зачистки plan-ID в доках.** `node.rs:211` («/// , action authors
   declare…», обрыв фразы) и `validate.rs:169-173` (хвост `//.`) — следы вырезанных
   ссылок на план, читаются криво.
5. **Дублированная валидация.** `build()` (fail-fast, первый error) частично
   дублирует `validate_workflow` (multi-error) — осознанный двухуровневый дизайн, но
   retry/trigger/schema-проверки есть **только** в `validate_workflow`; билдер их
   пропускает. Граница «что гарантирует `build()` vs `validate_workflow`» нечёткая.
6. **Нет интеграционных тестов.** `README.md:92`: 0 тестов в `tests/`, только unit —
   DAG-edge-cases и round-trip не покрыты на уровне крейта.
7. **`slot_bindings` не валидируются вовсе.** `validate_workflow` проверяет только
   структуру графа; существование/типы привязок не проверяет (нет доступа к реестрам) —
   см. §7.

## 7. Роль в пост-0092 credential/resource модели

Крейт затронут **узко и точечно** — как носитель author-side binding-данных, не как
место исполнения новой модели. Конкретно:

- **Шов = `NodeDefinition.slot_bindings: HashMap<String, SlotBinding>`** (`node.rs:55`).
  Это авторская сторона hybrid-binding: на узел-действие автор может переопределить
  конкретный `resource_id`/`credential_id` для именованного слота через
  `SlotBinding::{ResourceId, CredentialId}` (`node.rs:219`). Слоты живут **отдельно**
  от параметров узла (`parameters`) — ровно как требует пост-0092 разделение
  «slot_bindings ≠ parameters, values-only persistence».
- **Куда это течёт.** Эти привязки — входные данные для bind-population (M12.4):
  production credential→slot resolver (живёт **выше** — в `nebula-resource`
  fan-out / `nebula-api` / `nebula-credential::CredentialService`) читает именно
  `slot_bindings`, чтобы выдать действию `CredentialGuard<Scheme>`. Конвейер
  `ValidatedCredentialBinding` (ADR-0052), tenant-fingerprint и owner-scoping —
  всё это **не здесь**: этот крейт даёт сырое объявление привязки, проверенный и
  owner-изолированный путь строится выше по стеку.
- **Что меняется в пост-0092 мире — почти ничего в самом крейте.** Консолидация
  credential (одна `nebula-credential` = contract + runtime + facade + builtin;
  удаление `credential-runtime`/`builtin`/`testutil`/`vault`; `nebula-crypto`-порты),
  per-slot rotation fan-out в `nebula-resource`, `OwnerScopedKey`-изоляция, узкий
  `RefreshTransport`-seam и lease-as-first-class — всё это **downstream** от
  `slot_bindings`. `nebula-workflow` остаётся стабильной точкой объявления; пока
  форма `SlotBinding` (плоский id-string на слот) не меняется, верхние слои свободны
  эволюционировать резолвер.
- **Что остаётся by-construction здесь.** Разделение слот/параметр; values-only
  семантика (`SlotBinding` несёт только id, не сам секрет/конфиг — никакого secret
  material в `WorkflowDefinition`, что важно для serde-персиста и round-trip). Schema
  для зарегистрированных типов идёт по линии `HasSchema → nebula-metadata → API
  catalog`, минуя этот крейт целиком.
- **Граница ответственности (важно не сместить вверх).** Соблазн «провалидировать
  `slot_bindings` в `validate_workflow`» нужно отвергнуть: крейт Core-слоя не имеет и
  не должен иметь доступа к реестрам credential/resource; проверка существования и
  tenant-scoping — это api/runtime-валидатор поверх `ValidatedCredentialBinding`, не
  shift-left структурный валидатор workflow.

## 8. Forward design / открытые вопросы

- **Привести доки в соответствие с кодом** (низкий риск, высокий сигнал): убрать
  стейл-клейм о 4 `panic!` (`README.md:90-91`, `AGENTS.md:26`), починить doc-строку
  `builder.rs:153` (`EmptyName` → `InvalidOwnerId`), зачистить обрывки plan-ID в
  `node.rs:211` и `validate.rs:169-173`.
- **`interface_version` → `VersionReq`.** Решить судьбу гибкого пиннинга по spec
  `2026-04-17-replace-interfaceversion-with-semver`; это сужает выбор реализации
  действия и взаимодействует с registry-резолвом выше — координировать с
  `nebula-action`/`nebula-plugin`.
- **Контракт `build()` vs `validate_workflow`.** Зафиксировать, какие классы ошибок
  гарантирует каждый путь; либо свести билдер к делегированию в `validate_workflow`
  на финале, чтобы убрать дрейф двух наборов проверок.
- **Интеграционные тесты крейта.** Добавить `tests/` round-trip + DAG-edge-cases
  (циклы, дубли, висячие порты, multi-error агрегация) — сейчас 0.
- **Возможный typed slot-key.** Открытый вопрос на будущее: должен ли ключ
  `slot_bindings` (`String`) стать типизированным `SlotKey`, чтобы согласоваться с
  registry-стороной bind-population и убрать рассинхрон имён слотов между
  объявлением действия и привязкой в узле. Решать вместе с резолвером M12.4, не
  в одиночку здесь — форма привязки сейчас стабильна намеренно.
