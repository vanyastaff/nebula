# nebula-sdk — fact sheet

## Назначение
Единый фасад для авторов интеграций: re-export всей интеграционной поверхности
(`nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`, `nebula-workflow`,
`nebula-plugin`, `nebula-validator`, `nebula-core`, `nebula-metadata`) + собственные
`prelude`, `ActionBuilder`, `WorkflowBuilder`, `TestRuntime`/`RunReport` и макросы
(`params!`, `workflow!`, `simple_action!`). По решению 2026-06-10 — единственный публичный крейт workspace.

## Публичная поверхность
- Re-export целых крейтов: `pub use nebula_action/_core/_credential/_plugin/_resource/_schema/_validator/_workflow` + `serde`, `serde_json`, `thiserror` — `src/lib.rs:47-57`; `tokio` под feature `testing` — `lib.rs:60`
- `Error` (Workflow/Action/Parameter/Serialization/Other) + `Result<T>` — `src/lib.rs:73-114`
- `params!` (FieldValues из kv-пар, `try_set_raw` + expect) — `src/lib.rs:143-155`
- `workflow!` (декларативный DSL поверх WorkflowBuilder) — `src/lib.rs:179-200`
- `simple_action!` (unit-struct + impl `Action` + `StatelessAction`) — `src/lib.rs:235-277`
- `pub use serde_json::json` — `src/lib.rs:128`
- `ActionBuilder` (key/name/description/version → `ActionMetadata`) — `src/action.rs:33-73`
- `action::helpers::{validate_schema, parse_input}` — ручная "schema"-проверка required-полей — `src/action.rs:108-139`
- `WorkflowBuilder` (`add_node`/`add_node_with_params`/`connect`/`with_variable`/`build` → `WorkflowDefinition` с валидацией рёбер и dedup node-id) — `src/workflow.rs:43-274`
- `TestRuntime` (`run_stateless`/`run_stateful`/`run_poll`/`run_webhook`, knobs `with_stateful_cap`, `with_trigger_window`) — `src/runtime.rs:87-307`
- `RunReport` (kind/output/iterations/duration/emitted/note/health) — `src/runtime.rs:58-77`
- `testing::{is_success, is_failure, assert_success, assert_failure, fixtures}` (feature `testing`) — `src/testing.rs:22-75`
- `prelude` — широкий набор: action-трейты+адаптеры+spy-тестхарнесс (`prelude.rs:15-32`), credential-типы v2 (`ApiKeyCredential`, `OAuth2Credential`, `OAuth2Token`, `CredentialContext`, `CredentialSnapshot`… — `prelude.rs:37-58`), metadata-словарь (`prelude.rs:64`), `Resource`/`ResourceMetadata` (`prelude.rs:67`), schema-поля (`prelude.rs:73-79`), validator (`prelude.rs:80-82`), workflow-типы (`prelude.rs:83-88`)
- Вложенный крейт `nebula-macro-support` (`crates/sdk/macros-support/`): общие утилиты proc-macro — `attrs`, `credential_ref`, `diag`, `utils`, `validation_codegen` — `macros-support/src/lib.rs:9-18`

## Workspace-зависимости
Deps (Cargo.toml:16-24): nebula-core, nebula-action, nebula-metadata, nebula-workflow,
nebula-schema, nebula-credential, nebula-plugin, nebula-resource, nebula-validator;
плюс tokio/serde/serde_json/thiserror/uuid/chrono.
Features: `default = ["derive", "testing"]`; `derive = []` — ПУСТАЯ (ничего не гейтит); `testing` гейтит `src/testing.rs` + re-export tokio.
**Обратные зависимости: НОЛЬ** — ни один Cargo.toml workspace не зависит от nebula-sdk; `nebula_sdk::` не используется нигде вне самого крейта (даже в `examples/`).
`nebula-macro-support` (тот же каталог) — наоборот, используется 5 macros-крейтами: action/resource/plugin/credential/validator macros (`crates/*/macros/Cargo.toml:19-20`).

## Структура модулей
- `src/lib.rs` — re-exports, `Error`, макросы `params!`/`workflow!`/`simple_action!`
- `src/prelude.rs` — one-stop импорт (~100 строк re-exports)
- `src/action.rs` — `ActionBuilder` + `helpers` (validate_schema/parse_input)
- `src/workflow.rs` — `WorkflowBuilder` → `WorkflowDefinition`
- `src/runtime.rs` — `TestRuntime`/`RunReport`, in-process прогон 4 видов action
- `src/testing.rs` — assert-хелперы + fixtures (feature `testing`)
- `macros-support/` — отдельный крейт nebula-macro-support (syn/quote утилиты для всех *-macros)
- `tests/simple_action_macro.rs` — интеграционный тест макроса

## Напряжения
- README.md:104-105 утверждает «`anyhow` is re-exported» — ЛОЖЬ: anyhow нет ни в Cargo.toml, ни в lib.rs. README vs код.
- README.md:69 называет `simple_action!` обёрткой над «`ProcessAction`» — макрос реализует `StatelessAction` (lib.rs:264). Устаревшее имя трейта.
- Feature `derive` (Cargo.toml:48) пустая — ничего не включает, мёртвый флаг в default.
- `nebula_metadata` есть в deps и в prelude (prelude.rs:64), но НЕ re-export как целый крейт в lib.rs:47-56 (асимметрия с остальными 8) и не упомянут в README-списке re-exports.
- Два WorkflowBuilder в prelude: свой `workflow::WorkflowBuilder` (prelude.rs:98) + `nebula_workflow::WorkflowBuilder as CoreWorkflowBuilder` (prelude.rs:86) — дубль конструирования workflow.
- `action::helpers::validate_schema` (action.rs:108-120) — самодельная JSON-schema проверка с комментом «in production, use jsonschema crate»; дублирует/обходит nebula-schema/validator пайплайн, ошибки — голые `String`.
- `workflow!` принимает `$action:ty` и тут же `stringify!` (lib.rs:183,192) — тип используется как строка; doc-пример передаёт строковые литералы как `ty` — сомнительная грамматика макроса.
- `WorkflowBuilder::build` молча подменяет невалидный node-id на `node_{i}` (workflow.rs:177-182) — пользовательские connect() по исходному имени работают, но итоговый NodeKey тихо другой.
- Размещение `nebula-macro-support` ВНУТРИ crates/sdk/ при 5 потребителях-macros-крейтах из других подсистем — спорная топология (sdk-каталог как хост общей proc-macro утилиты).
- Zero-consumer фасад: examples/ не используют nebula_sdk — «единственный публичный крейт» без ни одного внутреннего/example потребителя.
- AGENTS.md:25 «Cross-crate calls go through nebula-eventbus» — шаблонная фраза, бессмысленная для re-export-фасада без runtime-логики.

## Роль в credential/resource redesign
Затронут напрямую как итоговая публичная поверхность (решение sole-public-crate 2026-06-10):
- prelude.rs:37-58 жёстко перечисляет credential-типы v2 (`OAuth2Credential`, `CredentialSnapshot`, `CredentialRecord`…) — rewrite credential (Protocol/scheme-enum, merge runtime→credential) ломает этот список; README §«P11 re-export audit» (README.md:43-54) фиксирует контракт «никаких параллельных OAuth-алиасов».
- prelude.rs:67 re-export `Resource`/`ResourceMetadata` — resource-redesign (Resource=2 assoc types, ветка dreamy-kare-8698d4, не смержена) изменит сигнатуру re-exports.
- Сам крейт пока пассивный (только re-exports) — кода миграции нет; коллапс crate-топологии за фасадом не начат.
