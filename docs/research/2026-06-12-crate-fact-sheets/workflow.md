# nebula-workflow — fact sheet

## Назначение
Core-слой: единое serde-round-trippable представление workflow — `WorkflowDefinition` (узлы + соединения + конфиг + UI-метаданные), `DependencyGraph` (petgraph: топосортировка + уровни параллелизма для `ExecutionPlan`), `WorkflowBuilder` (fluent сборка) и `validate_workflow` — shift-left валидатор активации (canon §10/§12.2). НЕ исполняет DAG, НЕ резолвит выражения, НЕ хранит. Maturity: stable.

## Публичная поверхность
- `WorkflowDefinition` — src/definition.rs:16; `is_schema_supported()` :63; `CURRENT_SCHEMA_VERSION = 1` :12
- `WorkflowConfig` (timeout, max_parallel_nodes, checkpointing, retry_policy, error_strategy) — definition.rs:111
- `ErrorStrategy` (FailFast | ContinueOnError | IgnoreErrors) — definition.rs:98
- `TriggerDefinition` (Manual | Cron | Webhook | Event) — definition.rs:72
- `RetryConfig` (`fixed`/`exponential`/`delay_for_attempt`) — definition.rs:171
- `CheckpointingConfig` :147, `UiMetadata` :220, `NodePosition` :234, `Viewport`, `Annotation`
- `NodeDefinition` — src/node.rs:13 (id: NodeKey, action_key: ActionKey, interface_version: Option<semver::Version>, parameters, retry_policy, timeout, enabled, rate_limit, **slot_bindings: HashMap<String, SlotBinding>** :55)
- `SlotBinding` enum { ResourceId(String) | CredentialId(String) } — node.rs:219; helpers `with_resource_binding` :119, `with_credential_binding` :131, `resource_binding` :147, `credential_binding` :159
- `ParamValue` (Literal | Expression | Template | Reference) — node.rs:230; выражения НЕ резолвятся здесь
- `RateLimit` — node.rs:71
- `Connection` (from_node, to_node, from_port, to_port; «чистый провод» без условий) — src/connection.rs:45; `effective_from_port()` → "main" :98
- `DependencyGraph` — src/graph.rs:15; `from_definition` :24, `topological_sort` :63, `compute_levels` (Kahn) :75, `entry_nodes` :148, `exit_nodes` :163, `validate` :203
- `WorkflowBuilder` — src/builder.rs:19; `connect_via` (port-driven, замена connect_with_condition) :94, `build()` (fail-fast валидация) :174
- `validate_workflow(&WorkflowDefinition) -> Vec<WorkflowError>` (multi-error) — src/validate.rs:52
- `ValidatedWorkflow` — newtype-witness «provено-валидно», единственный конструктор `validate()` — validate.rs:204
- `WorkflowError` — src/error.rs:9, thiserror + `nebula_error::Classify`, коды `WORKFLOW:*` (15 вариантов, все category="validation")
- `NodeState` (Pending..WaitingRetry, `is_terminal` и пр.) — src/state.rs:9
- `Version` (свой semver-подобный тип для версии workflow) — src/version.rs:7

## Workspace-зависимости
Deps: nebula-core (path), nebula-error (workspace, feature "derive"); внешние: petgraph, semver, serde, serde_json, thiserror, chrono. Dev: insta, pretty_assertions, rstest. `[lib] doctest = false`.
Зависят от него: nebula-engine, nebula-action, nebula-sdk, nebula-execution, nebula-api, nebula-plugin (6 крейтов).

## Структура модулей (9 + lib)
- builder.rs (378) — WorkflowBuilder, fail-fast build()
- connection.rs (167) — Connection + контракт активации портов (таблица ActionResult→port в module doc)
- definition.rs (464) — WorkflowDefinition/Config/Trigger/Retry/Checkpointing/UiMetadata
- error.rs (127) — WorkflowError, Classify-коды
- graph.rs (483) — DependencyGraph на petgraph
- node.rs (450) — NodeDefinition, ParamValue, SlotBinding, RateLimit
- state.rs (228) — NodeState enum (прогресс выполнения узла)
- validate.rs (761) — validate_workflow + ValidatedWorkflow witness + validate_retry_config
- version.rs (140) — собственный Version (major/minor/patch/pre/build)
- lib.rs:47 — pub(crate) re-export `nebula_core::serde_helpers::duration_opt_ms` как `serde_duration_opt`

## Напряжения
- **Стейл-клейм README/AGENTS о panic!**: README.md:90-91 и AGENTS.md:26 утверждают «4 panic! sites в lib-коде как builder-invariant guards / debt» — фактически все 4 panic! в node.rs лежат внутри `#[cfg(test)]` (node.rs:348,359,368,384); lib-код чист. Документация устарела.
- **Doc/код расхождение**: builder.rs:153 doc «Returns WorkflowError::EmptyName if owner_id is empty», код возвращает `InvalidOwnerId` (builder.rs:157).
- **Два типа Version**: собственный `crate::Version` для версии workflow (version.rs:7) + `semver::Version` для `interface_version` узла (node.rs:26). Гибкий пиннинг (`VersionReq`) — отложенная работа, spec `2026-04-17-replace-interfaceversion-with-semver`.
- **Артефакты зачистки plan-ID в доках**: node.rs:211 «///, action authors declare…» (обрыв фразы), validate.rs:172-173 + «//.» (validate.rs:169-173) — следы вырезанных ссылок на план; читается криво.
- **Дублированная валидация**: builder `build()` (fail-fast, первый error) частично дублирует `validate_workflow` (multi-error) — осознанный двухуровневый дизайн, но retry/trigger/schema-проверки есть только в validate_workflow, билдер их пропускает.
- README.md:92 признаёт: 0 интеграционных тестов в `tests/`, только unit.
- TODO/FIXME/deprecated: нет.

## Роль в credential/resource redesign
Прямо затронут как **носитель `slot_bindings`**: `NodeDefinition.slot_bindings: HashMap<String, SlotBinding>` (node.rs:55) — авторская сторона hybrid-binding (override resource_id/credential_id на слот действия). Это входные данные для bind-population (M12.4): production credential→slot resolver должен читать именно эти привязки (ValidatedCredentialBinding-конвейер ADR-0052 живёт выше, в api/runtime). Сам крейт НЕ валидирует существование id-шников (нет доступа к реестрам) — `validate_workflow` проверяет только структуру графа, slot_bindings не проверяются вовсе.
