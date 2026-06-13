# nebula-core — design

| Field | Value |
|-------|-------|
| **Status** | Stable — workspace vocabulary leaf (depends on nothing in-workspace except `nebula-error`) |
| **Layer** | Foundation / vocabulary (bottom of the stack; 15 reverse-deps) |
| **Redesign role** | **Partially touched** — `auth.rs` (`AuthScheme`/`AuthPattern`/Sensitive-Public) and `dependencies.rs` (`SlotField`/slot decls) and the `guard.rs` + `accessor.rs` seam are load-bearing for the credential/resource rewrite; the rest (IDs, keys, scope, tenancy, obs, slug) is untouched stable foundation. |
| **Related** | ADR-0088/0092 (credential rewrite — consume `auth.rs`/`accessor.rs`), ADR-0093 (topology/hot-swap — `RefreshCoordinator`), PRODUCT_CANON §15.5 (Sensitive/Public), spec 23 (`dependencies.rs`) |

---

## 1. Назначение и границы

`nebula-core` — это **vocabulary layer** в самом низу workspace: общий словарь типов, на котором стоит всё остальное. Никуда вверх не зависит (единственный workspace-dep — `nebula-error`).

**Владеет:** типизированными prefixed-ULID идентификаторами (`exe_…`, `wf_…`, `cred_…` через `domain-key`), нормализованными доменными ключами (`ParameterKey`/`ActionKey`/…), scope-иерархией, контрактом `Context` + capability-трейтами, auth-схемами (`AuthScheme`/`AuthPattern`), декларациями зависимостей (slots, spec 23), guard-трейтами, tenancy/RBAC-примитивами, валидированными slug-ами и observability-идентичностью (W3C trace context).

**ЯВНО НЕ делает:** не хранит секреты (типа `SecretString` здесь НЕТ — он в credential-слое; README:50 ошибочно ссылается на seam в `keys.rs`), не делает crypto/KDF (это `nebula-crypto`), не содержит runtime credential/resource-логику (resolver/refresh/lease/fan-out — в `nebula-credential`/`nebula-resource`), не знает про конкретные storage-бэкенды. Здесь — только словарь и контракты-трейты, реализации живут выше.

## 2. Публичная поверхность

| Группа | Ключевые типы | Где |
|--------|---------------|-----|
| IDs (`define_ulid`) | `OrgId`, `WorkspaceId`, `WorkflowId`, `WorkflowVersionId`, `ExecutionId`, `AttemptId`, `InstanceId`, `TriggerId`, `TriggerEventId`, `UserId`, `ServiceAccountId`, `ResourceId`, `CredentialId`, `SessionId` | `src/id/types.rs:8-22` |
| ID migration alias | `OrganizationId = OrgId` (`#[deprecated]`) | `src/id/types.rs:24-25` |
| Keys (`define_domain`) | `ParameterKey`, `CredentialKey`, `ActionKey`, `ResourceKey`, `PluginKey`, `NodeKey` + макросы `resource_key!`/`action_key!`/… | `src/keys.rs:28-44,55+` |
| Scope | `ScopeLevel` (Global/Organization/Workspace/Workflow/Execution), `Scope`, `Principal`, `ScopeResolver` | `src/scope.rs:24,191,229,166` |
| Context | `Context` (трейт), `BaseContext` (не-Clone, `Box<dyn Clock>`), `BaseContextBuilder` | `src/context/mod.rs:15,39,78` |
| Capability-трейты | `HasResources`, `HasCredentials`, `HasLogger`, `HasMetrics`, `HasEventBus` | `src/context/capability.rs:6-35` |
| Accessors | `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`, `EventEmitter`, `Clock`, `SystemClock`, `RefreshCoordinator`, `RefreshToken` | `src/accessor.rs:13-119` |
| Auth | `AuthPattern` (11 вариантов, `non_exhaustive`), `AuthScheme`, `SensitiveScheme: AuthScheme + ZeroizeOnDrop`, `PublicScheme`, `impl AuthScheme for ()` = NoAuth | `src/auth.rs:41,83,118,137,140` |
| Dependencies (spec 23) | `Dependencies`, `SlotField`, `SlotKind`, `DeclaresDependencies`, `CredentialRequirement`, `ResourceRequirement`, `CredentialLike`, `ResourceLike`, `DependencyError` | `src/dependencies.rs:9-226` |
| Guards | `Guard`, `TypedGuard`, `debug_redacted`, `debug_typed` | `src/guard.rs:9-40` |
| Observability | `TraceId(u128)`, `SpanId(u64)`, `W3cTraceContext`, `parse_traceparent`, `W3cTraceContextError` | `src/obs.rs:11-172` |
| RBAC / tenancy | `OrgRole`, `WorkspaceRole`, `effective_workspace_role` (`src/role.rs:9-40`); `Permission` (`src/permission.rs:8`); `TenantContext`, `PermissionDenied`, `ResolvedIds` (`src/tenancy.rs:15,94,115`) |
| Slug | `Slug`, `SlugKind`, `SlugError`, `is_reserved`, `is_prefixed_ulid` | `src/slug.rs:8-265` |
| Lifecycle / sync | `LayerLifecycle`, `ShutdownOutcome` (`src/lifecycle.rs:8,46`); `Lazy<X>` async-lazy (`src/sync.rs:52`) |
| Error | `CoreError`, `CoreResult` (`src/error.rs:10,148`); `PluginKeyParseError` alias (`src/lib.rs:102`) |
| Prelude | `prelude` (IDs + keys + scope + макросы) | `src/lib.rs:105-130` |

## 3. Зависимости и зависимые

- **Deps (все workspace):** `chrono`, `domain-key`, `nebula-error`, `serde`, `serde_json`, `thiserror`, `tokio` (macros, time), `tokio-util` (rt), `zeroize`. Единственный in-workspace dep — `nebula-error`.
- **Reverse-deps (15):** `nebula-action`, `api`, `credential`, `engine`, `execution`, `metadata`, `plugin`, `resource`, `resource-macros`, `sdk`, `storage`, `storage-port`, `tenancy`, `workflow` + `examples`. Любое изменение публичной поверхности здесь каскадирует на всю эту цепочку.

## 4. Внутренняя архитектура

Плоский набор leaf-модулей без внутреннего runtime-потока — крейт пассивен, это словарь:

- `id/` — prefixed-ULID типы через `domain_key::define_ulid` + re-export `UlidParseError`.
- `keys.rs` (private mod, `pub use *`) — domain-keys + compile-time макросы.
- `scope.rs` — `ScopeLevel`/`Scope`/`Principal`/`ScopeResolver`.
- `context/` — `Context` трейт + `BaseContext(Builder)`; `capability.rs` — `Has*` трейты.
- `accessor.rs` — capability-инъекция (логгер/метрики/часы/credential/resource доступ, refresh-coordination).
- `auth.rs` — `AuthScheme`/`AuthPattern` + Sensitive/Public дихотомия (§15.5).
- `dependencies.rs` — декларации credential/resource-зависимостей и slot-полей (spec 23).
- `guard.rs` — RAII guard-трейты + redacted `Debug`-хелперы.
- `obs.rs` — `TraceId`/`SpanId`/W3C traceparent парсинг.
- `role.rs` / `permission.rs` / `tenancy.rs` — RBAC enums, `Permission`, `TenantContext`/`ResolvedIds`.
- `slug.rs` — валидированные slug-и + reserved-список + `is_prefixed_ulid`.
- `lifecycle.rs` / `sync.rs` / `serde_helpers.rs` / `error.rs` — вспомогательные leaf-модули.

## 5. Инварианты и контракты

- **Типизированные ID by-construction.** Каждый ID — отдельный prefixed-ULID-тип через `domain-key`; перепутать `ExecutionId` с `WorkflowId` невозможно на уровне типов (`src/id/types.rs:8-22`).
- **Sensitive/Public дихотомия (§15.5).** `SensitiveScheme` требует `ZeroizeOnDrop`; redacted `Debug` (`debug_redacted`) гарантирует, что секреты не утекают в логи by-construction для типов, идущих через guard-хелперы (`src/auth.rs:118`, `src/guard.rs`).
- **Capability-инъекция через трейты.** `Context` несёт возможности (`HasCredentials`/`HasResources`/…) как трейт-границы, а не конкретные типы — нижний слой не знает о реализациях выше (`src/context/capability.rs`).
- **W3C trace-context корректность.** `parse_traceparent` валидирует traceparent по спецификации, отбраковывая некорректные → `W3cTraceContextError` (`src/obs.rs:172`).
- **Slug-валидация и reserved-список.** `Slug` — валидированный newtype; `is_reserved`/`is_prefixed_ulid` отсекают зарезервированные и ULID-подобные значения (`src/slug.rs`).
- **Tenant-изоляция в словаре.** `TenantContext`/`ResolvedIds`/`PermissionDenied` дают типизированный язык tenancy/RBAC, на котором строят owner-scoping выше (`src/tenancy.rs:15,94,115`).

## 6. Известные напряжения / долг (честно)

1. **README ↔ код, сильный дрейф.** README:30 утверждает «`CredentialId` lives in `nebula-credential`» — на деле он определён здесь (`src/id/types.rs:21`). README/lib.rs doc перечисляют `NodeId`, `TenantId`, `ProjectId`, `RoleId` (`lib.rs:16-17`, README:30) — таких типов в коде НЕТ. README:78 говорит про re-export `UuidParseError` — фактически `UlidParseError` (`src/id/mod.rs:9`). README:41 относит `PermissionDenied` к модулю `permission` — фактически он в `tenancy.rs:94`. README scope-иерархия «Global → Organization → Project → Workflow → Execution → Action» (README:32) не совпадает с кодом: `ScopeLevel` без `Project` и `Action` (`src/scope.rs:24-39`). **Долг = переписать README под код.**
2. **`SecretString`-фантом в доках.** README:50 (контракт L2-§12.5) ссылается на `SecretString` с seam в `crates/core/src/keys.rs` — в крейте `SecretString` нет вообще (живёт в credential-слое).
3. **Sensitive/Public только macro-enforced.** Взаимоисключаемость `SensitiveScheme`/`PublicScheme` обеспечивается только макросом; hand-rolled двойной impl компилируется — задокументированная дыра с планом в `docs/tracking/credential-concerns-register.md` (`src/auth.rs:97-117`).
4. **Deprecated alias `OrganizationId`.** `OrganizationId = OrgId` (`src/id/types.rs:24-25`) — миграционный, re-export под `#[allow(deprecated)]` (`lib.rs:82`) и в prelude (`lib.rs:114`); подлежит вычистке после миграции потребителей.
5. **Обрыв доков.** `src/dependencies.rs:34` «per ///.» — предложение оборвано (видимо после удаления plan-ID); чистый doc-fix.

## 7. Роль в пост-0092 credential/resource модели

Частично затронут, но как **словарь, а не как реализация**. Канонический дом, который consume-ит rewrite:

- `auth.rs` — `AuthScheme`/`AuthPattern`/Sensitive-Public дихотомия, на которой стоит scheme-enum/Protocol-модель, `SchemeFactory` и hot-swap из `nebula-credential` (ADR-0088/0092).
- `dependencies.rs` — `SlotField`/`SlotKind` = словарь slot-bindings и bind-population (M12.4, `nebula-resource`).
- `accessor.rs` + `guard.rs` — `CredentialAccessor`/`RefreshCoordinator` + guard-трейты = seam для hot-swap handles и framework-owned acquire loop (ADR-0093/topology).

Сами resolver/refresh/lease/rotation-state (`nebula-credential`) и per-slot fan-out (`nebula-resource`) сюда НЕ переезжают — `nebula-core` остаётся пассивным контрактным словарём.

## 8. Forward design / открытые вопросы

Крейт стабилен; «forward» здесь — это гигиена, а не новая архитектура:
- Закрыть macro-only дыру взаимоисключаемости `Sensitive`/`Public` структурно (sealed-marker или type-state), а не дисциплиной (см. напряжение 3).
- Привести README/`lib.rs`-doc в соответствие с кодом и удалить `SecretString`-фантом (напряжения 1-2).
- Завершить миграцию с `OrganizationId` и удалить deprecated alias.

В остальном `nebula-core` — стабильный фундамент: новые типы добавляются по требованию верхних слоёв, но контрактная поверхность меняется редко и осознанно (15 reverse-deps — высокая цена breaking-change).
