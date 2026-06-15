# nebula-action — fact sheet

## Назначение
Типизированное семейство трейтов действий (Action Trait Family) + статические метаданные исполнения
(`ActionMetadata`) — контракт между «что делает узел workflow» и «как движок его оркестрирует».
Авторы пишут typed-трейты (`StatelessAction`/`StatefulAction`/`TriggerAction`/`ResourceAction` + DX-надстройки);
движок диспатчит in-process через `ActionFactory` → `ErasedAction` (`dyn Action` невозможен). WASM/process-изоляция — non-goal (ADR-0091).

## Публичная поверхность
- `Action` (Sized, NOT object-safe; `type Input/Output: HasSchema`, статич. `metadata()`/`dependencies()`) — src/action.rs:60
- `StatelessAction` / `StatelessHandler` / `StatelessActionAdapter` — src/stateless.rs:47,74,97
- `StatefulAction` + DX `PaginatedAction`/`BatchAction`, `StatefulHandler` — src/stateful.rs:38,114,253,385
- `TriggerAction`, `TriggerHandler`, `TriggerEvent`, `TriggerSource` — src/trigger/mod.rs
- `ResourceAction` / `ResourceHandler` (graph-scoped DI, configure/cleanup) — src/resource.rs:31,60; `ResourceProduces<R>` — src/resource_produces.rs:52
- `ControlAction` / `ControlOutcome` / `ControlInput` (If/Switch/Router/Stop/Fail) — src/control.rs:393,269,109
- `WebhookAction` + HMAC-примитивы (`verify_hmac_sha256*`, `SignaturePolicy` fail-closed Required) — src/webhook/mod.rs (2431 строк)
- `PollAction`, `PollTriggerAdapter`, `POLL_INTERVAL_FLOOR`, `DeduplicatingCursor` — src/poll/mod.rs
- `ActionHandler` — top-level enum-диспетчер над `Arc<dyn XxxHandler>` — src/handler.rs:41
- `ErasedAction` enum + `ErasedStateless/Stateful/Trigger/Resource/Control` — src/erased.rs:185,40-162
- `ActionFactory` + `Generic{Stateless,Stateful,Trigger,Resource,Control}Factory` — src/factory.rs:53,69-497
- `FromWorkflowNode` (async slot-binding фабрика; тело генерит derive) — src/from_workflow_node.rs:61
- `ActionError` + `RetryHintCode` (retryable vs fatal), `ValidationReason` — src/error.rs:154,31,58
- `ActionMetadata`, `ActionCategory`, `IsolationLevel` — src/metadata.rs:98,41,13
- `ActionResult<T>`, `TerminationReason`, `WaitCondition`, `BranchKey` — src/result.rs:40,195,297
- `ActionOutput<T>`, `OutputEnvelope`, `DeferredOutput`, `StreamOutput` — src/output.rs:506,465,86,206
- Контексты: `ActionContext`/`TriggerContext` трейты + `ActionRuntimeContext`/`TriggerRuntimeContext` — src/context.rs:81,108,144,397
- `#[derive(Action)]` + `#[action_phantom]` (proc-macro, re-export lib.rs:126) — macros/src/lib.rs
- Тестовая обвязка: `TestActionContext`, `Spy*`, `StatefulTestHarness` — src/testing.rs
- Webhook-провайдеры из коробки: Slack/Stripe/Generic + `WebhookActionFactory` — src/webhook/providers/, src/webhook/factory.rs:163

## Workspace-зависимости
Зависит от: nebula-action-macros (path=macros), nebula-core, nebula-credential, nebula-error, nebula-metadata,
nebula-schema, nebula-resource, nebula-workflow; + http/bytes/url (webhook-словарь), hmac/sha2/hex/base64/subtle (подписи), zeroize (пин «1.8.2», не workspace).
Dev: nebula-credential-macros, nebula-expression, trybuild, insta, rstest.
От него зависят: nebula-plugin (crates/plugin/Cargo.toml:22), nebula-sdk (crates/sdk/Cargo.toml:17), nebula-engine (crates/engine/Cargo.toml:28), nebula-api (crates/api/Cargo.toml:28).

## Структура модулей (src/, ~18.4k строк)
- action.rs — базовый трейт `Action` (identity + статич. метаданные)
- stateless.rs / stateful.rs — one-shot и итеративные действия + handler/adapter + DX (paginated, batch)
- trigger/ — базовый `TriggerAction`, transport-agnostic `TriggerEvent`-конверт, `TriggerSource`
- webhook/ — крупнейший домен: WebhookAction, конфиг, HMAC-верификация, Clock, фабрика, providers (slack/stripe/generic)
- poll/ — PollAction поверх Trigger: interval floor, warn throttle, cursor-дедуп
- resource.rs + resource_produces.rs — ResourceAction (graph-DI) и Output-маркер
- control.rs — flow-control узлы, desugar в stateless-поверхность
- erased.rs + factory.rs — engine-side стирание типов и пер-исполнение фабрики
- from_workflow_node.rs — async-резолв slot-bindings из узла workflow
- handler.rs — суммирующий enum `ActionHandler`; домены ре-экспортируются «for backwards compatibility»
- context.rs — capability-трейты контекстов + runtime-реализации
- error.rs / result.rs / output.rs — типизированные ошибки, flow-control результат, выходные данные (inline/blob/stream/deferred)
- metadata.rs / port.rs / idempotency.rs / capability.rs / validation.rs / testing.rs / macros.rs / prelude.rs — метаданные, порты, дедуп-ключ, Noop/default capability, валидация пакета, тест-утилиты, assert-макросы
- macros/ (отдельный крейт nebula-action-macros) — derive `Action`, `#[action_phantom]`, field_slots-парсер `#[resource]`/`#[credential]`

## Напряжения
- **Stale doc в derive**: macros/src/lib.rs:47 «Action structs must be unit structs with no fields» — противоречит реализации (macros/src/action.rs:37-72 + field_slots.rs принимают named-поля со слотами) и AGENTS.md:34 («structs hold only slot fields»).
- **nebula-action-types.md (431 строка, рус.)** — стихийный дизайн-док в корне крейта с устаревшей иерархией: рисует `TriggerAction` как потомка `StatefulAction`, тогда как lib.rs:15 и код держат его отдельным трейтом «outside the execution graph». Кандидат на удаление/перенос.
- **Legacy path-space**: lib.rs:60 / handler.rs — handler-трейты живут в доменных файлах, но ре-экспортируются через `handler::*` «for backwards compatibility».
- **CheckpointPolicy planned-not-wired**: lib.rs:23-25 — заявлен в доках, но не является полем `ActionMetadata` (осознанно зафиксировано, не баг).
- **План-идентификаторы в Cargo.toml**: строки 56 («Phase 9 / Task 9.1») и 63 («Closes Stage-4 review I3») — нарушают правило «no plan IDs in committed code».
- **zeroize не через workspace**: Cargo.toml:33,65 — пин `1.8.2` локально, остальные deps `workspace = true`.

## Роль в credential/resource redesign
Прямой потребитель обоих: re-export `CredentialGuard`/`CredentialRef` (lib.rs:132) и `ResourceRef` (lib.rs:133);
`#[credential]`-слоты держат `CredentialGuard<C::Scheme>`; `FromWorkflowNode` — то самое место, где slot-bindings
резолвятся (consumer-сторона M12.4 bind-population). Коллапс credential-крейтов за sole-public-sdk поменяет его dep-пути
и типы guard'ов; webhook-домен — пример «секрет не утекает через dyn TriggerHandler». Затронут, но как downstream, не как площадка редизайна.
