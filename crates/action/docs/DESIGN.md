# nebula-action — design

| Field | Value |
|-------|-------|
| **Status** | `frontier` — Variant A trait shape shipped (M6 / §M11, 2026-04-29) |
| **Layer** | Contract / authoring (Ports & Adapters) — author programs to traits, engine wires adapters |
| **Redesign role** | **Затронут как downstream-потребитель, НЕ как площадка редизайна.** Прямой потребитель credential/resource: `#[credential]`/`#[resource]` слоты, `CredentialGuard<Scheme>`, `FromWorkflowNode` = consumer-сторона bind-population (M12.4). |
| **Related** | ADR-0081 (M6 binding cascade ⊇ ADR-0042/0043/0044/0045), ADR-0091 (in-process, WASM non-goal), ADR-0092 (credential consolidation), PRODUCT_CANON §3.5/§11.3/§13.4/§13.5 |

---

## 1. Назначение и границы

`nebula-action` определяет **типизированное семейство трейтов действий** (Action Trait Family) плюс
статический дескриптор исполнения (`ActionMetadata`) — контракт между «что делает узел workflow»
и «как движок его оркестрирует». Авторы пишут typed-трейты; движок диспатчит **in-process** через
`ActionFactory` → `ActionHandle`.

**Владеет:**
- Базовым трейтом `Action` (Sized, identity + статич. метаданные) и под-трейтами
  `StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction` / `ControlAction`.
- DX-надстройками: `PaginatedAction` / `BatchAction` (над Stateful), `WebhookAction` / `PollAction` (над Trigger).
- Engine-side стиранием типов: `ActionHandle` enum + per-variant `XxxHandle` + `Generic*Factory`.
- Slot-binding фабрикой `FromWorkflowNode` (тело генерит derive).
- Типизированной моделью результата/ошибки/выхода: `ActionResult` (flow-control intent),
  `ActionError` + `RetryHintCode`, `ActionOutput` (inline/blob/stream/deferred).
- Webhook-доменом: HMAC-примитивы (`verify_hmac_sha256*`), `SignaturePolicy` (fail-closed `Required`),
  встроенные провайдеры Slack/Stripe/Generic.
- Capability-контекстами (`ActionContext` / `TriggerContext` трейты + runtime-реализации) и тест-обвязкой.

**ЯВНО НЕ делает:**
- Не машина состояний исполнения — это `nebula-execution` (`ExecutionStatus`, CAS-переходы).
- Не единственная retry-поверхность — engine-level retry живёт в `NodeDefinition.retry_policy` (ADR-0042,
  срабатывает ПОСЛЕ финального исхода), in-call retry — в `nebula-resilience`. Два слоя компонуются на разных границах.
- Не схемная система — `parameters` держит `ValidSchema` из `nebula-schema`; схема `Input`/`Output`
  читается через `nebula_schema::schema_of::<A::Input>()` (нет метода `schema()` на трейте, ADR-0052 P3).
- Не WASM / не process-изоляция — non-goal (ADR-0091, canon §12.6); единственный драйвер = `InProcessRunner`.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `Action` (Sized, NOT object-safe; `type Input/Output: HasSchema`; static `metadata()`/`dependencies()`) | `src/action.rs:60` |
| `StatelessAction` / `StatelessHandler` / `StatelessActionAdapter` | `src/stateless.rs:47,74,97` |
| `StatefulAction` + DX `PaginatedAction` / `BatchAction`, `StatefulHandler` | `src/stateful.rs:38,114,253,385` |
| `TriggerAction`, `TriggerHandler`, `TriggerEvent`, `TriggerSource` | `src/trigger/mod.rs` |
| `ResourceAction` / `ResourceHandler` (graph-scoped DI, configure/cleanup) | `src/resource.rs:31,60` |
| `ResourceProduces<R>` (Output-маркер, пустая схема + topology-tag) | `src/resource_produces.rs:52` |
| `ControlAction` / `ControlOutcome` / `ControlInput` (If/Switch/Router/Stop/Fail) | `src/control.rs:393,269,109` |
| `WebhookAction` + HMAC (`verify_hmac_sha256*`, `SignaturePolicy` fail-closed `Required`) | `src/webhook/mod.rs` (2431 строк) |
| `PollAction`, `PollTriggerAdapter`, `POLL_INTERVAL_FLOOR`, `DeduplicatingCursor` | `src/poll/mod.rs` |
| `ActionHandle` enum + `StatelessHandle/StatefulHandle/TriggerHandle/ResourceHandle/ControlHandle` | `src/handle.rs:184,40-176` |
| `ActionFactory` + `Generic{Stateless,Stateful,Trigger,Resource,Control}Factory` | `src/factory.rs:53,69-497` |
| `FromWorkflowNode` (async slot-binding фабрика; тело генерит derive) | `src/from_workflow_node.rs:61` |
| `ActionError` + `RetryHintCode` (retryable vs fatal), `ValidationReason` | `src/error.rs:154,31,58` |
| `ActionMetadata`, `ActionKind`, `CheckpointPolicy`, `IsolationLevel` | `src/metadata.rs:131,48,83,13` |
| `ActionResult<T>`, `TerminationReason`, `WaitCondition`, `BranchKey` | `src/result.rs:40,195,297` |
| `ActionOutput<T>`, `OutputEnvelope`, `DeferredOutput` | `src/output.rs` |
| `ActionContext`/`TriggerContext` + `ActionRuntimeContext`/`TriggerRuntimeContext` | `src/context.rs:81,108,144,397` |
| `#[derive(Action)]` + `#[action_phantom]` (proc-macro) | `macros/src/lib.rs` (re-export `lib.rs:126`) |
| `TestActionContext`, `Spy*`, `StatefulTestHarness` | `src/testing.rs` |
| Webhook-провайдеры Slack/Stripe/Generic + `WebhookActionFactory` | `src/webhook/providers/`, `src/webhook/factory.rs:163` |

## 3. Зависимости и зависимые

**Зависит от:** `nebula-action-macros` (path=macros), `nebula-core`, `nebula-credential`, `nebula-error`,
`nebula-metadata`, `nebula-schema`, `nebula-resource`, `nebula-workflow`; + `http`/`bytes`/`url` (webhook-словарь),
`hmac`/`sha2`/`hex`/`base64`/`subtle` (подписи), `zeroize` (пин «1.8.2», **не** через workspace).
Dev: `nebula-credential-macros`, `nebula-expression`, `trybuild`, `insta`, `rstest`.

**От него зависят:** `nebula-plugin` (`plugin/Cargo.toml:22`), `nebula-sdk` (`sdk/Cargo.toml:17`),
`nebula-engine` (`engine/Cargo.toml:28`), `nebula-api` (`api/Cargo.toml:28`).

## 4. Внутренняя архитектура

~18.4k строк в `src/`. Поток данных: **derive → metadata/factory → engine erasure → context-bound execute**.

- `action.rs` — базовый `Action` (identity + статич. метаданные).
- `stateless.rs` / `stateful.rs` — one-shot и итеративные действия + handler/adapter + DX (paginated, batch).
- `trigger/` — `TriggerAction`, transport-agnostic `TriggerEvent`-конверт, `TriggerSource`.
- `webhook/` — крупнейший домен: `WebhookAction`, конфиг, HMAC-верификация, `Clock`, фабрика, providers (slack/stripe/generic).
- `poll/` — `PollAction` поверх Trigger: interval floor, warn-throttle, cursor-дедуп.
- `resource.rs` + `resource_produces.rs` — `ResourceAction` (graph-DI) и Output-маркер.
- `control.rs` — flow-control узлы, desugar в stateless-поверхность.
- `handle.rs` + `factory.rs` — engine-side стирание типов (`ActionHandle` + `XxxHandle`) и per-исполнение фабрики (т.к. `dyn Action` невозможен).
- `from_workflow_node.rs` — async-резолв slot-bindings из узла workflow.
- `context.rs` — capability-трейты контекстов + runtime-реализации.
- `error.rs` / `result.rs` / `output.rs` — типизированные ошибки, flow-control результат, выходные данные.
- `metadata.rs` / `port.rs` / `idempotency.rs` / `capability.rs` / `validation.rs` / `testing.rs` / `macros.rs` / `prelude.rs` — метаданные, порты, дедуп-ключ, Noop/default capability, валидация пакета, тест-утилиты, assert-макросы.
- `macros/` (крейт `nebula-action-macros`) — derive `Action`, `#[action_phantom]`, field_slots-парсер `#[resource]`/`#[credential]`.

## 5. Инварианты и контракты

- **`Action` не object-safe by construction.** `dyn Action` не компилируется; engine-диспатч идёт
  через `Arc<dyn ActionFactory>` + `ActionHandle` над per-variant `Box<dyn XxxHandle>`. Это структурный инвариант, не дисциплина.
- **Slots-only на `Self`, form-data на `Self::Input`** (canon §3.5). Action-структура держит только
  slot-поля (`#[resource]`/`#[credential]`); пользовательские данные — на отдельной `Self::Input: HasSchema`.
  Устраняет `self.text` vs `input.text`-неоднозначность на этапе компиляции.
- **Routing по трейту, не по `ActionKind`** (canon §3.5). `ActionKind` — метаданные node-таксономии
  для UI/валидатора/аудита; движок маршрутизирует по семейству трейта (структурно, по хендлу, который
  производит фабрика). Добавление нового трейта = canon-ревизия (§0.2).
- **Webhook fail-closed** (ADR-0022). `SignaturePolicy::Required` с пустым секретом по умолчанию;
  HTTP-транспорт даёт `401 problem+json` на mismatch, `500` на `Required`-без-секрета. `OptionalAcceptUnsigned`
  — явный opt-out. **Секрет НЕ течёт через dyn `TriggerHandler`** — webhook-конфиг читается из typed-action
  на activation-time и форвардится в `WebhookTransport::activate` явным параметром.
- **Trigger delivery — at-least-once** (canon §13.4). Нет тихого drop; дубль обрабатывается через
  stable event identity + dedup/idempotency. Seam: `TriggerAction::start`, `TriggerEvent`.
- **Idempotency для рискованных эффектов** (canon §11.3 / §13.5). Не-идемпотентные side-effects обязаны
  проходить через engine idempotency-key path до вызова remote-системы.
- **`RetryHintCode` различает retryable vs fatal** (`src/error.rs:31`) — типизированная классификация, не строки.

## 6. Известные напряжения / долг

1. **Stale doc в derive.** `macros/src/lib.rs:47` утверждает «Action structs must be unit structs with no
   fields» — противоречит реализации (`macros/src/action.rs:37-72` + `field_slots.rs` принимают named-поля
   со слотами) и `AGENTS.md:34` («structs hold only slot fields»). Док врёт про текущую модель.
2. **`nebula-action-types.md` (431 строка, рус.)** — стихийный дизайн-док в корне крейта с устаревшей
   иерархией: рисует `TriggerAction` как потомка `StatefulAction`, тогда как `lib.rs:15` и код держат его
   отдельным трейтом «outside the execution graph». Кандидат на удаление/перенос (этот DESIGN.md — замена).
3. **Legacy path-space.** `lib.rs:60` / `handler.rs` — handler-трейты живут в доменных файлах, но
   ре-экспортируются через `handler::*` «for backwards compatibility». Четыре прод-пути сознательно остаются
   на legacy handler-поверхности (webhook routing, plugin discovery, SDK runtime, EventSource adapter).
4. **`CheckpointPolicy` поле-без-enforcement.** Поле `checkpoint_policy: CheckpointPolicy` (default `Inherit`)
   теперь есть в `ActionMetadata`; движок ещё НЕ исполняет non-`Inherit` каденции — это persisted-намерение,
   не runtime-гарантия. Документировать именно так («есть поле, enforcement не провязан»).
5. **План-идентификаторы в Cargo.toml.** Строки 56 («Phase 9 / Task 9.1») и 63 («Closes Stage-4 review I3»)
   нарушают правило «no plan IDs in committed code».
6. **`zeroize` не через workspace.** `Cargo.toml:33,65` — локальный пин `1.8.2`, тогда как остальные deps
   `workspace = true`. Расходится с workspace-дисциплиной версий.

## 7. Роль в пост-0092 credential/resource модели

`nebula-action` — **consumer-сторона** обоих редизайнов, не их площадка. Его контракт авторинга прямо опирается
на типы из credential/resource, поэтому консолидация затрагивает его dep-пути и типы guard'ов.

- **Credential-слоты.** `#[credential(key = "…")]`-поле держит `CredentialGuard<C::Scheme>` —
  **спроецированную auth-схему**, не сам тип credential. Фреймворк проецирует state→scheme до заполнения слота.
  Re-export `CredentialGuard`/`CredentialRef` живёт в `lib.rs:132`. После ADR-0092 credential схлопывается в
  ОДИН крейт (contract+runtime+`CredentialService`+builtin); крейты `credential-runtime`/`builtin`/`testutil`/`vault`
  УДАЛЕНЫ. Для `nebula-action` это означает: те же типы guard'а, но из единого `nebula-credential` — точка
  проекции state→scheme должна уважать correction «policy(&State) driving routing» и `OwnerScopedKey` owner-isolation
  на стороне резолвера (action их не реализует, но получает уже-изолированный guard).
- **Resource-слоты.** `#[resource(key = "…")]`-поле держит `ResourceGuard<R>`; re-export `ResourceRef` в `lib.rs:133`.
  `ResourceAction` ставит `type Output = ResourceProduces<Self::Resource>` (graph-side эффект, пустая data-схема).
  Per-slot rotation FAN-OUT теперь во владении `nebula-resource` (`credential_fanout/`, ex-engine) — action только
  потребляет уже-резолвленный/уже-ротированный guard, не участвует в fan-out-механике.
- **Bind-population seam (M12.4).** `FromWorkflowNode::from_workflow_node` — **то самое место**, где slot-bindings
  резолвятся: derive читает `node.resource_binding(slot)` / `node.credential_binding(slot)` (fallback на
  `default_id` = slot key), зовёт `ctx.acquire_resource_by_id::<R>` / `ctx.resolve_credential_by_id::<C>`,
  собирает `Self`. Это consumer-конец producer-gap'а bind-population — `slot_bindings` отделены от `parameters`,
  привязка к конкретному `CredentialId`/resource-id идёт через ADR-0042 hybrid-механизм (default=slot key,
  override через `node.slot_bindings`). Values-only persistence: схема приходит из зарегистрированных типов
  (`HasSchema` → `nebula-metadata` → API catalog), не из inline-значений.
- **Что меняется:** dep-пути (всё credential — из одного крейта за sole-public-`nebula-sdk`), потенциально
  упрощение re-export-блока в `lib.rs`. **Что остаётся:** сама форма слотов (`#[credential]`/`#[resource]`,
  `CredentialGuard<Scheme>`, `ResourceGuard<R>`), `FromWorkflowNode`-seam, webhook «секрет не через dyn»-инвариант,
  routing-по-трейту. Lease — first-class на стороне credential; action видит его опосредованно через guard, не как
  собственный примитив. Unified `#[property]`-авторинг — Phase-5, NOT-YET-BUILT; текущий derive остаётся актуальным.

## 8. Forward design / открытые вопросы

- **Заменить `nebula-action-types.md` этим DESIGN.md.** Удалить/перенести стихийный док с неверной иерархией
  (Напряжение №2); зафиксировать `TriggerAction` как отдельный трейт «outside the execution graph».
- **Починить stale derive-doc** (`macros/src/lib.rs:47`): привести текст в соответствие с named-slot-полями
  (Напряжение №1) — иначе plugin-авторы читают противоречие между доком и компилятором.
- **Снять долг Cargo.toml:** убрать план-идентификаторы (строки 56/63) и поднять `zeroize` до `workspace = true`
  (Напряжения №5/№6) — в идеале согласовать с workspace-bump.
- **Согласовать re-export-блок credential** (`lib.rs:132`) с пост-0092 топологией: после коллапса credential-крейтов
  проверить, что `CredentialGuard`/`CredentialRef` приходят из единого `nebula-credential`, и нет ли осиротевших путей
  к удалённым `credential-runtime`/`builtin`.
- **`CheckpointPolicy`-решение:** поле введено в `ActionMetadata` (default `Inherit`); остаётся провязать
  non-`Inherit` каденции через движок — до этого держать доки честными («persisted, not yet enforced»).
- **Риск bind-population:** `FromWorkflowNode` готов как consumer-конец, но producer (прод-резолвер
  credential→slot) — frontier на стороне `nebula-resource`/`nebula-credential`. Пока producer не зрелый,
  end-to-end slot-binding нельзя считать закрытым со стороны action.
- **Phase-5 unified `#[property]`-authoring** — следить за решением; если введут, derive-поверхность и slots-only-инвариант
  придётся пересмотреть синхронно, но это NOT-YET-BUILT.
