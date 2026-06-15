# nebula-engine — fact sheet

## Назначение
Composition root / оркестратор исполнения workflow: строит `ExecutionPlan` из DAG,
резолвит входы узлов из выходов предшественников, переводит состояние исполнения
через storage-port (`ExecutionStore::commit(TransitionBatch)`, CAS по `version`),
диспатчит экшены in-process (`ActionRuntime`/`InProcessSandbox`) и владеет durable-консьюмером
`execution_control_queue` (canon §12.2). Бывший крейт nebula-runtime поглощён как `src/runtime/`.

## Публичная поверхность
- `WorkflowEngine` — engine.rs:125; `new` 348, `cancel_execution` 416, `with_action_credentials` 887, `execute_workflow` 1388, `resume_execution` 1719; level-by-level DAG, bounded concurrency, Layer-2 retry-heap, cancel-registry
- `ControlConsumer` / трейт `ControlDispatch` / `ControlDispatchError` — control_consumer.rs:281/125/94; polling+claim/ack очереди управления
- `EngineControlDispatch` — control_dispatch.rs:72; канонический impl (Start/Resume/Restart/Cancel/Terminate, идемпотентность по (execution_id, command), ADR-0008/0016)
- `EngineCredentialAccessor` — credential_accessor.rs:91; deny-by-default allowlist кредов на экшен
- `EngineResourceAccessor` — resource_accessor.rs:24; без allowlist (скоупинг — дело topology-слоя); `slot_identities_for_key` :148
- `credential::*` — credential/mod.rs:19-30 — почти целиком re-export из `nebula_credential::runtime` (ADR-0092): `CredentialResolver`, `RefreshCoordinator`, `LeaseLifecycle`, `execute_resolve`/`execute_continue`…; своё только `default_in_memory_coordinator()` :43 (тянет InMemoryRefreshClaimRepo из nebula-storage)
- `credential::rotation` (feature `rotation`) — rotation.rs:9-34 — чистый re-export shim (state-machine из nebula-credential, fan-out `ResourceFanoutDriver` из nebula-resource)
- `resource::{ResourceActivator, ResourceActivatorRegistry, KindActivator, RegisterRequest, RegistrarError, ResourceRegistrationOutcome}` — resource/registrar.rs:103-418; seam «stored row (kind+JSON) → типизированная регистрация в nebula_resource::Manager», unrecognized kind = typed error
- `runtime::{ActionRuntime, ActionRegistry, SandboxRunner, InProcessSandbox, TaskQueue, MemoryQueue, BlobStorage, DataPassingPolicy, BoundedStreamBuffer, StatefulCheckpoint(-Sink)}` — runtime/runtime.rs:61-116, registry.rs:53, sandbox_runner.rs:25-89, queue.rs:22-132
- `scoped_resources::{BranchId, ScopedResourceMap, DashScopedResourceMap, LayeredResourceAccessor, ScopedResourceGuard, run_cleanup(_with_timeout)}` — scoped_resources.rs:116-698; per-branch хранение, scoped→global precedence, RAII LIFO-cleanup с таймаутом
- `daemon::{Daemon, DaemonRegistry, DaemonRuntime, EventSource, EventSourceRuntime/Adapter, RestartPolicy, DaemonError}` — daemon/mod.rs:37-52, registry.rs:38-273, runtime.rs:37, event_source.rs:32-164
- `store_seam::{ExecutionStores, WorkflowStores, engine_scope, node_output_record…}` — store_seam.rs:44-126; мост к spec-16 storage-port
- `ExecutionResult` result.rs:10 · `EngineError` error.rs:10 · `ExecutionEvent` event.rs:19 (eventbus broadcast) · `NodeOutput` node_output.rs:9
- `resource_status::{ResourceRuntimeStatus, EngineResourceStatus, EngineManagerResourceStatus}` — resource_status.rs:45-91
- Re-export plugin-типов: `Plugin, PluginKey, PluginManifest, PluginRegistry, ResolvedPlugin` — lib.rs:102

## Workspace-зависимости
Deps (14 intra-workspace, заявлено намеренным для composition root): nebula-core, nebula-error,
nebula-action, nebula-expression, nebula-plugin, nebula-workflow, nebula-execution, nebula-schema,
nebula-credential, nebula-eventbus, nebula-resource, nebula-resilience, nebula-storage-port,
nebula-storage (feature `credential-in-memory`), nebula-metrics. Внешние ключевые: tokio, dashmap,
opentelemetry/tracing-opentelemetry, async-trait, zeroize.
Фичи: `rotation` (прокидывает в credential/storage/resource), `chaos-full` (nightly chaos-CI), `test-util` (не в prod, ADR-0023).
Обратные зависимости: **только nebula-api** (crates/api/Cargo.toml:29; комментарий :143 — api получает sandbox-типы через engine). sdk/server на engine не зависят.

## Структура модулей
- `engine.rs` (9826 строк!) — WorkflowEngine: run_frontier, retry-decision, budget, cancel-registry; самый нагруженный модуль
- `control_consumer.rs` / `control_dispatch.rs` / `control_trace.rs` — консьюмер очереди управления + W3C trace-parent restore (ADR-0050)
- `credential/` — Plane B (integration credentials): тонкая обёртка + re-export shim над nebula_credential::runtime (ADR-0092); `rotation.rs` — re-export shim ротации
- `credential_accessor.rs` / `resource_accessor.rs` — cross-layer мосты business-трейтов в engine-типы (README признаёт: архитектурно место им в credential/resource)
- `daemon/` — реестр и runtime долгоживущих фоновых провайдеров + EventSource-адаптеры
- `resource/` — registrar: kind-string → typed activation в Manager
- `runtime/` — поглощённый nebula-runtime: ActionRuntime-диспатч, ActionRegistry, InProcessSandbox, очередь, blob, data-policy, backpressure
- `resolver.rs` (pub(crate)) — ParamResolver: Literal/Expression/Template/Reference → JSON
- `scoped_resources.rs` — M6.1/M6.2 per-branch ресурсы; store_seam.rs — фабрики записей/Scope для storage-port
- `node_output.rs`, `result.rs`, `error.rs`, `event.rs`, `resource_status.rs` — мелкие типы

## Напряжения
- **Самоссылочный typo после поглощения runtime**: lib.rs:9 «delegates action dispatch to `nebula-engine`» и lib.rs:81-83 «absorbed `nebula-engine` public surface» — оба должны читаться «nebula-runtime»
- **README vs код**: README.md:7,30,100,128 ссылаются на `nebula-runtime` как на отдельный sibling-крейт — крейта `crates/runtime` больше не существует (поглощён). README `last-reviewed: 2026-04-17`, status partial
- **Re-export shim-слой ADR-0092**: credential/mod.rs:16-27 и rotation.rs целиком — compat-переэкспорты «чтобы старые пути `nebula_engine::credential::*` резолвились»; при единственном потребителе (nebula-api) кандидат на прямую миграцию путей и удаление (память: feedback_no_shims)
- **Legacy-путь регистрации экшенов**: runtime/registry.rs:406-456 `legacy_register_*_with_metadata` («LEGACY test-only»), плюс fallback legacy `ActionHandler` dispatch в runtime/runtime.rs:312-377 — два пути диспатча (factory + legacy) живут параллельно
- **Legacy ExecutionRepo vs spec-16 store**: engine.rs:1127,1231,1727 — ветвление «store-port если сконфигурирован, иначе legacy ExecutionRepo»; двойной seam ещё не схлопнут
- TODO engine.rs:1086 (warning-лог при failure cleanup'а prior run)
- engine.rs ~9.8k строк — монолит, признан в AGENTS.md «largest, load-bearing»
- README Appendix: открытый долг «ExecutionBudget переехал в nebula-execution — import cleanup pending» (engine.rs); edge-gate уже, чем заявлено (§10, multi-hop conditional)

## Роль в credential/resource redesign
Прямо затронут с обеих сторон. **Credential**: после ADR-0092 engine больше не владеет резолвером/refresh/lease — всё переехало в `nebula_credential::runtime`, в engine остались Plane-B фасад (`default_in_memory_coordinator`), deny-by-default `EngineCredentialAccessor` и shim-переэкспорты; фича `rotation` пробрасывает rotation-фичи в credential/storage/resource. **Resource**: engine — место будущего bind-population (M12.4): `ResourceActivatorRegistry` активирует kinds, `rotation.rs` ре-экспортирует fan-out (`ResourceFanoutDriver`, `Bind`) из nebula-resource; scoped_resources/`LayeredResourceAccessor` — engine-сторона precedence. При коллапсе крейтов за sole-public sdk shim-слои engine — первое, что схлопывается.
