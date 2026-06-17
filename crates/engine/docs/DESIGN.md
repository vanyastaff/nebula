# nebula-engine — design

| Field | Value |
|-------|-------|
| **Status** | Partial — самый нагруженный, load-bearing крейт (`engine.rs` ~9.8k строк, признан в AGENTS.md «largest, load-bearing») |
| **Layer** | Composition root / оркестратор исполнения workflow (L2 control-plane) |
| **Redesign role** | **Затронут с обеих сторон.** Credential: после ADR-0092 runtime (resolver/refresh/lease/rotation-state) уехал в `nebula-credential`, в engine остались accessor-мосты + `default_in_memory_coordinator` + shim-переэкспорты. Resource: engine — место будущего bind-population (M12.4); `rotation.rs` ре-экспортирует fan-out из `nebula-resource`. |
| **Related** | ADR-0092, ADR-0088, ADR-0008 (control plane), ADR-0016 (cancellation), ADR-0068 (layered retry), ADR-0050, PRODUCT_CANON §10/§11.1/§11.2/§12.2/§12.5 |

---

## 1. Назначение и границы

`nebula-engine` — **composition root**: единственный компонент, который собирает все остальные
крейты и доводит исполнение от «активированный workflow» до терминального состояния. Без него
вызывающая сторона вынуждена самостоятельно сшивать runtime, storage, execution-слой и
plugin-registry — и рискует разойтись с canon §12.2 control-plane.

**Владеет:**
- построением `ExecutionPlan` из DAG workflow и резолвом входов узла из выходов предшественников;
- переводом состояния исполнения через storage-port (`ExecutionStore::commit(TransitionBatch)`,
  CAS по `version`) — никакой in-memory мутации lifecycle (canon §11.1);
- level-by-level обходом DAG с bounded concurrency, Layer-2 retry-heap и cancel-registry
  (`engine.rs:125`, поток `run_frontier`);
- durable-консьюмером `execution_control_queue` (canon §12.2): polling + claim/ack + W3C
  trace-parent restore (ADR-0050);
- in-process диспатчем экшенов (`ActionRuntime` / `InProcessRunner`) — бывший крейт
  `nebula-runtime` поглощён как `src/runtime/`.

**ЯВНО НЕ делает** (Non-goals из README):
- не storage-реализация — это `nebula-storage` (`ExecutionRepo`, backends);
- не исполнитель экшенов как таковой — диспатч живёт в поглощённом `runtime/`, но контракт экшена
  принадлежит `nebula-action`;
- не изолятор плагинов — плагины регистрируются и работают in-process через `nebula-plugin`
  (ADR-0091);
- не вычислитель выражений — это `nebula-expression`;
- не владелец credential-runtime (после ADR-0092 — см. §7) и не владелец per-slot rotation
  fan-out (теперь `nebula-resource`).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `WorkflowEngine` (`new`, `cancel_execution`, `with_action_credentials`, `execute_workflow`, `resume_execution`) | `engine.rs:125 / 348 / 416 / 887 / 1388 / 1719` |
| `ControlConsumer` / `ControlDispatch` / `ControlDispatchError` (polling + claim/ack очереди управления) | `control_consumer.rs:281 / 125 / 94` |
| `EngineControlDispatch` (канонический impl Start/Resume/Restart/Cancel/Terminate, идемпотентность по `(execution_id, command)`, ADR-0008/0016) | `control_dispatch.rs:72` |
| `EngineCredentialAccessor` (deny-by-default allowlist кредов на экшен) | `credential_accessor.rs:91` |
| `EngineResourceAccessor` (без allowlist; скоупинг — дело topology-слоя) + `slot_identities_for_key` | `resource_accessor.rs:24 / 148` |
| `credential::*` — почти целиком re-export из `nebula_credential::runtime` (ADR-0092): `CredentialResolver`, `RefreshCoordinator`, `LeaseLifecycle`, `execute_resolve` / `execute_continue`; своё — только `default_in_memory_coordinator()` | `credential/mod.rs:19-30 / 43` |
| `credential::rotation` (feature `rotation`) — чистый re-export shim (state-machine из credential, fan-out `ResourceFanoutDriver` из resource) | `rotation.rs:9-34` |
| `resource::{ResourceActivator, ResourceActivatorRegistry, KindActivator, RegisterRequest, RegistrarError, ResourceRegistrationOutcome}` (seam «stored row (kind+JSON) → типизированная регистрация в `nebula_resource::Manager`»; unrecognized kind = typed error) | `resource/registrar.rs:103-418` |
| `runtime::{ActionRuntime, ActionRegistry, ActionRunner, InProcessRunner, TaskQueue, MemoryQueue, BlobStorage, DataPassingPolicy, BoundedStreamBuffer, StatefulCheckpoint(-Sink)}` | `runtime/runtime.rs:61-116`, `registry.rs:53`, `runner.rs:25-89`, `queue.rs:22-132` |
| `scoped_resources::{BranchId, ScopedResourceMap, DashScopedResourceMap, LayeredResourceAccessor, ScopedResourceGuard, run_cleanup(_with_timeout)}` (per-branch хранение, scoped→global precedence, RAII LIFO-cleanup с таймаутом) | `scoped_resources.rs:116-698` |
| `daemon::{Daemon, DaemonRegistry, DaemonRuntime, EventSource, EventSourceRuntime/Adapter, RestartPolicy, DaemonError}` | `daemon/mod.rs:37-52`, `registry.rs:38-273`, `runtime.rs:37`, `event_source.rs:32-164` |
| `store_seam::{ExecutionStores, WorkflowStores, node_output_record…}` (мост к spec-16 storage-port; реальный per-message `Scope` протягивается в `resume_execution`, плейсхолдер удалён) | `store_seam.rs:44-126` |
| `ExecutionResult` · `EngineError` · `ExecutionEvent` (eventbus broadcast) · `NodeOutput` | `result.rs:10` · `error.rs:10` · `event.rs:19` · `node_output.rs:9` |
| `resource_status::{ResourceRuntimeStatus, EngineResourceStatus, EngineManagerResourceStatus}` | `resource_status.rs:45-91` |
| Re-export plugin-типов: `Plugin, PluginKey, PluginManifest, PluginRegistry, ResolvedPlugin` | `lib.rs:102` |

## 3. Зависимости и зависимые

- **Deps (14 intra-workspace, намеренно для composition root):** `nebula-core`, `nebula-error`,
  `nebula-action`, `nebula-expression`, `nebula-plugin`, `nebula-workflow`, `nebula-execution`,
  `nebula-schema`, `nebula-credential`, `nebula-eventbus`, `nebula-resource`, `nebula-resilience`,
  `nebula-storage-port`, `nebula-storage` (feature `credential-in-memory`), `nebula-metrics`.
  Внешние ключевые: `tokio`, `dashmap`, `opentelemetry` / `tracing-opentelemetry`, `async-trait`,
  `zeroize`.
- **Фичи:** `rotation` (пробрасывается в credential/storage/resource), `chaos-full` (nightly
  chaos-CI), `test-util` (не в prod, ADR-0023).
- **Зависимые:** **только `nebula-api`** (`crates/api/Cargo.toml:29`; комментарий :143 — api
  получает runner-типы через engine). `sdk` / `server` на engine **не** зависят.

## 4. Внутренняя архитектура

- `engine.rs` (~9.8k строк) — `WorkflowEngine`: `run_frontier`, retry-decision (`compute_retry_decision`,
  `effective_retry_policy`), budget, cancel-registry. Самый нагруженный модуль.
- `control_consumer.rs` / `control_dispatch.rs` / `control_trace.rs` — консьюмер очереди управления
  + восстановление W3C trace-parent на dispatch-span (ADR-0050).
- `credential/` — Plane B (integration credentials): тонкая обёртка + re-export shim над
  `nebula_credential::runtime` (ADR-0092); `rotation.rs` — re-export shim ротации.
- `credential_accessor.rs` / `resource_accessor.rs` — cross-layer мосты business-трейтов в
  engine-типы (README признаёт: архитектурно место им в credential/resource).
- `daemon/` — реестр и runtime долгоживущих фоновых провайдеров + EventSource-адаптеры.
- `resource/` — registrar: kind-string → typed activation в `Manager`.
- `runtime/` — поглощённый `nebula-runtime`: ActionRuntime-диспатч, ActionRegistry,
  InProcessRunner, очередь, blob, data-policy, backpressure.
- `resolver.rs` (`pub(crate)`) — `ParamResolver`: Literal / Expression / Template / Reference → JSON.
- `scoped_resources.rs` — M6.1/M6.2 per-branch ресурсы; `store_seam.rs` — фабрики записей/Scope для
  storage-port.
- Мелкие типы: `node_output.rs`, `result.rs`, `error.rs`, `event.rs`, `resource_status.rs`.

**Поток данных:** workflow DAG → `ExecutionPlan` → level-by-level frontier (bounded concurrency) →
для каждого узла `ParamResolver` собирает входы из выходов предшественников → диспатч экшена через
`ActionRuntime` → переход состояния через storage-port (CAS по `version`) → `ExecutionEvent`
broadcast через eventbus. Control-plane (`ControlConsumer`) идёт параллельно: дренит
`execution_control_queue` и через `EngineControlDispatch` сигналит живой frontier-loop.

## 5. Инварианты и контракты

- **[L2-§11.1] CAS-переходы.** Переходы состояния идут только через storage-port
  (`ExecutionStore::commit(TransitionBatch)`, CAS по `version`); ни один handler внутри engine не
  мутирует lifecycle в памяти и не изобретает параллельный жизненный цикл.
- **[L2-§12.2] Engine владеет control-queue.** Все пять команд (Start/Resume/Restart/Cancel/Terminate)
  сшиты end-to-end через `EngineControlDispatch`. `Cancel` достигает живого frontier-loop через
  per-instance cancel-registry (`WorkflowEngine::cancel_execution`; ADR-0016); `Terminate` делит
  cooperative-cancel body до появления отдельного forced-shutdown пути. Handler, который только
  логирует и отбрасывает строки, нарушает инвариант.
- **Идемпотентность control-команд.** Дисптач идемпотентен по паре `(execution_id, command)`
  (ADR-0008 §5); `Cancel`/`Terminate` идемпотентны через лежащий ниже `CancellationToken`.
- **Deny-by-default credential allowlist** (`credential_accessor.rs:91`). Пустой allowlist отклоняет
  любой запрос (canon §12.5, §4.5). Per-action allowlists заполняются через
  `with_action_credentials`; экшен, чьи креды не объявлены engine, проваливается в deny-baseline.
  **Fail-open escape hatch отсутствует by-construction.**
- **No resource allowlist** (`resource_accessor.rs:24`). В отличие от кредов, allowlist для ресурсов
  нет: скоупинг намеренно принадлежит topology-слою (pool scope, daemon scope), не engine.
- **Два retry-слоя, непересекающиеся по границе триггера** (ADR-0042): Layer 1 (in-call,
  `nebula-resilience::retry_with` внутри экшена — engine видит лишь финальный исход) и Layer 2
  (operator-declared `NodeDefinition.retry_policy` — после `Running → Failed` engine паркует узел в
  `NodeState::WaitingRetry` с `next_attempt_at`; cancel / terminate / budget breach дренят
  parked-retries в `Cancelled` без re-dispatch; глобальный cap — `ExecutionBudget.max_total_retries`,
  canon §11.2).
- **Unrecognized resource kind = typed error.** `ResourceActivatorRegistry` при незнакомом kind
  возвращает типизированный `RegistrarError`, не паникует (`resource/registrar.rs`).

## 6. Известные напряжения / долг (честно)

1. **Самоссылочный typo после поглощения runtime.** `lib.rs:9` («delegates action dispatch to
   `nebula-engine`») и `lib.rs:81-83` («absorbed `nebula-engine` public surface») — оба должны
   читаться `nebula-runtime`.
2. **README рассинхронизирован с кодом.** `README.md:7,30,100,128` ссылаются на `nebula-runtime`
   как на отдельный sibling-крейт, которого больше нет (поглощён). `last-reviewed: 2026-04-17`,
   status `partial`.
3. **Re-export shim-слой ADR-0092.** `credential/mod.rs:16-27` и `rotation.rs` целиком — compat-
   переэкспорты, чтобы старые пути `nebula_engine::credential::*` резолвились. При единственном
   потребителе (`nebula-api`) — кандидат на прямую миграцию путей и удаление (память:
   feedback_no_shims).
4. **Legacy-путь регистрации экшенов.** `runtime/registry.rs:406-456`
   `legacy_register_*_with_metadata` («LEGACY test-only») + fallback legacy `ActionHandler` dispatch
   в `runtime/runtime.rs:312-377` — два пути диспатча (factory + legacy) живут параллельно.
5. **Legacy ExecutionRepo vs spec-16 store.** `engine.rs:1127,1231,1727` — ветвление «store-port если
   сконфигурирован, иначе legacy ExecutionRepo»; двойной seam ещё не схлопнут.
6. **`engine.rs` ~9.8k строк** — монолит, признан в AGENTS.md «largest, load-bearing»; декомпозиция
   откладывается.
7. **TODO `engine.rs:1086`** — warning-лог при failure cleanup'а prior run.
8. **`ExecutionBudget` переехал в `nebula-execution`** — import cleanup pending (`engine.rs`,
   README Appendix).
9. **Edge-gate уже, чем заявлено** (§10): downstream-edge gate блокирует только локальные рёбра, не
   полный граф — multi-hop conditional flows покрыты слабее, чем рекламируется.

## 7. Роль в пост-0092 credential/resource модели

После ADR-0092 credential-подсистема консолидирована в **один** крейт `nebula-credential`
(contract + runtime + `CredentialService` facade + builtin-типы; крейты `credential-runtime` /
`credential-builtin` / `testutil` / `vault` УДАЛЕНЫ). Это напрямую перекраивает engine с двух сторон.

**Credential (Plane B).** Engine **больше не владеет** резолвером / refresh / lease / rotation-state —
всё переехало в `nebula_credential::runtime`. В engine остались ровно три вещи:
- `credential/mod.rs` — re-export shim, чтобы исторические пути `nebula_engine::credential::*`
  (`CredentialResolver`, `execute_resolve`/`execute_continue`, `RefreshCoordinator`, `LeaseLifecycle`)
  продолжали резолвиться у единственного потребителя `nebula-api`;
- `default_in_memory_coordinator()` (`credential/mod.rs:43`) — единственный собственный код модуля;
  конструирует `InMemoryRefreshClaimRepo` из `nebula-storage` для тестов / single-replica desktop;
- `EngineCredentialAccessor` (`credential_accessor.rs:91`) — deny-by-default allowlist-мост.

Резолвер generic по конкретному типу `C` и вызывает `C::project(&state)` напрямую — type-erased
projection-registry не нужен (`StateProjectionRegistry` был vestigial, удалён в ADR-0088 D3;
capability + metadata теперь живут только на `nebula_credential::CredentialRegistry`). Conference-
коррекции (которые движутся **внутри** `nebula-credential`, не здесь): `policy(&State)` должна
определять routing; `OwnerScopedKey` для owner-изоляции; узкий типизированный `RefreshTransport`
seam; lease как first-class; `#[property]`/unified authoring — Phase-5, **ещё не построено**.

**Resource.** Per-slot rotation **fan-out** уехал в `nebula-resource`
(`credential_fanout/`, ex-engine). Engine остаётся:
- местом будущего **bind-population (M12.4)** — производственного credential→slot резолвера ещё нет;
- `resource/registrar.rs` — seam активации kinds в `nebula_resource::Manager` (stored row → typed);
- `rotation.rs` (feature `rotation`) — re-export shim fan-out (`ResourceFanoutDriver`, `Bind`) из
  `nebula-resource`;
- `scoped_resources.rs` / `LayeredResourceAccessor` — engine-сторона scoped→global precedence;
- `EngineResourceAccessor` — без allowlist (скоупинг у topology, не engine).

**Что остаётся истинным.** Consumer-binding модель: action/resource объявляют `#[credential]` /
`#[resource]` слоты и получают `CredentialGuard<Scheme>`; `slot_bindings` отделены от parameters;
персистентность values-only (схема — из зарегистрированных типов через `HasSchema → nebula-metadata
→ API-каталог`). Все эти контракты engine **транзитом передаёт**, не владеет ими.

**Что меняется.** При коллапсе крейтов за sole-public `nebula-sdk` **shim-слои engine — первое, что
схлопывается**: `credential/mod.rs` re-export и `rotation.rs` исчезают, как только `nebula-api`
мигрирует импорты на канонические пути `nebula_credential::*` / `nebula_resource::*`.

## 8. Forward design / открытые вопросы

- **Удалить shim-слой (P1).** При единственном потребителе (`nebula-api`) прямая миграция импортов
  `nebula_engine::credential::*` → `nebula_credential::runtime::*` и удаление `credential/mod.rs`
  re-export + `rotation.rs`. Память feedback_no_shims прямо требует «replace the wrong thing directly».
- **Bind-population producer (M12.4) — единственный реальный gap активного credential-lifecycle.**
  Engine — frontier этой работы: нужно подключить production credential→slot резолвер, чтобы
  `register_and_bind` получил живых вызывающих (сейчас quiesce-контракт есть, callers нет).
- **Схлопнуть двойной store seam.** Убрать legacy `ExecutionRepo`-ветку (`engine.rs:1127/1231/1727`),
  оставив только spec-16 storage-port; это снимает половину «store-port если сконфигурирован, иначе…».
- **Убить legacy action-dispatch путь.** `legacy_register_*_with_metadata` +
  `ActionHandler`-fallback (`runtime/registry.rs`, `runtime/runtime.rs`) — два пути диспатча должны
  стать одним (factory-only).
- **Переселить cross-layer мосты.** `credential_accessor.rs` / `resource_accessor.rs` архитектурно
  принадлежат `nebula-credential` / `nebula-resource` как extension points; перенос — кандидат-
  рефактор после закрытия gap'ов выше (README признаёт прямо).
- **Декомпозиция `engine.rs`.** ~9.8k строк load-bearing монолита — риск для maintainability и для
  per-crate-green инкрементов; выделение control-plane / frontier / retry в подмодули.
- **Обновить README и self-referential docs.** Снять `nebula-runtime`-как-sibling ссылки и
  `lib.rs` typo после поглощения runtime; синхронизировать `last-reviewed`.
- **Открытый вопрос:** после удаления shim-слоя и переноса accessor-мостов — сохраняет ли engine
  отдельный `credential/` модуль вообще, или Plane-B сводится к одному
  `default_in_memory_coordinator` + accessor в корне крейта?
