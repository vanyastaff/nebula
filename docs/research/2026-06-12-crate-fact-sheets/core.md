# nebula-core — fact sheet

## Назначение
Общий словарь (vocabulary layer) внизу всего workspace: типизированные prefixed-ULID
идентификаторы (`exe_…`, `wf_…`, `cred_…` через `domain-key`), нормализованные строковые ключи,
scope-иерархия, контракт `Context` + capability-трейты, auth-схемы (`AuthScheme`/`AuthPattern`),
tenancy/RBAC-примитивы и observability-идентичность (W3C trace context). Никуда вверх не зависит.

## Публичная поверхность
- IDs (define_ulid): `OrgId`, `WorkspaceId`, `WorkflowId`, `WorkflowVersionId`, `ExecutionId`, `AttemptId`, `InstanceId`, `TriggerId`, `TriggerEventId`, `UserId`, `ServiceAccountId`, `ResourceId`, `CredentialId`, `SessionId` — src/id/types.rs:8-22
- `OrganizationId = OrgId` `#[deprecated]` (migration alias) — src/id/types.rs:24-25
- Keys (define_domain/key_type): `ParameterKey`, `CredentialKey`, `ActionKey`, `ResourceKey`, `PluginKey`, `NodeKey` + compile-time макросы `resource_key!`/`action_key!`/… — src/keys.rs:28-44,55+
- `ScopeLevel` (Global/Organization/Workspace/Workflow/Execution), `Scope`, `Principal`, `ScopeResolver` — src/scope.rs:24,191,229,166
- `Context` трейт, `BaseContext` (не-Clone, `Box<dyn Clock>`), `BaseContextBuilder` — src/context/mod.rs:15,39,78
- Capability-трейты: `HasResources`, `HasCredentials`, `HasLogger`, `HasMetrics`, `HasEventBus` — src/context/capability.rs:6-35
- Accessors: `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`, `EventEmitter`, `Clock`, `SystemClock`, `RefreshCoordinator`, `RefreshToken` — src/accessor.rs:13-119
- Auth: `AuthPattern` (11 вариантов, non_exhaustive), `AuthScheme` (без Clone/Serialize-супертрейтов), `SensitiveScheme: AuthScheme + ZeroizeOnDrop`, `PublicScheme`; `impl AuthScheme for ()` = NoAuth — src/auth.rs:41,83,118,137,140
- Dependencies (spec 23): `Dependencies`, `SlotField`, `SlotKind`, `DeclaresDependencies`, `CredentialRequirement`, `ResourceRequirement`, `CredentialLike`, `ResourceLike`, `DependencyError` — src/dependencies.rs:9-226
- Guards: `Guard`, `TypedGuard`, `debug_redacted`, `debug_typed` — src/guard.rs:9-40
- Obs: `TraceId(u128)`, `SpanId(u64)`, `W3cTraceContext`, `parse_traceparent`, `W3cTraceContextError` — src/obs.rs:11-172
- RBAC/tenancy: `OrgRole`, `WorkspaceRole`, `effective_workspace_role` (src/role.rs:9-40); `Permission` (src/permission.rs:8); `TenantContext`, `PermissionDenied`, `ResolvedIds` (src/tenancy.rs:15,94,115)
- `Slug`, `SlugKind`, `SlugError`, `is_reserved`, `is_prefixed_ulid` — src/slug.rs:8-265
- `LayerLifecycle`, `ShutdownOutcome` — src/lifecycle.rs:8,46; `Lazy<X>` async-lazy — src/sync.rs:52
- `CoreError`, `CoreResult` — src/error.rs:10,148; `PluginKeyParseError` alias — src/lib.rs:102
- `prelude` модуль (IDs+keys+scope+макросы) — src/lib.rs:105-130

## Workspace-зависимости
Deps (все workspace): `chrono`, `domain-key`, `nebula-error`, `serde`, `serde_json`, `thiserror`, `tokio` (macros,time), `tokio-util` (rt), `zeroize`.
Обратные зависимости (15): nebula-action, api, credential, engine, execution, metadata, plugin, resource, resource-macros, sdk, storage, storage-port, tenancy, workflow + examples.
Bench: `benches/id_parse_serialize.rs`; тесты контрактов: `tests/schema_contracts.rs`.

## Структура модулей
- `id/` — prefixed-ULID типы через `domain_key::define_ulid` (types.rs) + re-export `UlidParseError`
- `keys.rs` (private mod, pub use *) — domain-keys + compile-time макросы
- `scope.rs` — ScopeLevel/Scope/Principal/ScopeResolver
- `context/` — Context трейт + BaseContext(Builder); `capability.rs` — Has* трейты
- `accessor.rs` — capability-инъекция (логгер/метрики/часы/credential/resource доступ)
- `auth.rs` — AuthScheme/AuthPattern + Sensitive/Public дихотомия (§15.5)
- `dependencies.rs` — декларации credential/resource-зависимостей и slot-полей (spec 23)
- `guard.rs` — RAII guard-трейты + redacted Debug-хелперы
- `obs.rs` — TraceId/SpanId/W3C traceparent парсинг
- `role.rs` / `permission.rs` / `tenancy.rs` — RBAC enums, Permission, TenantContext/ResolvedIds
- `slug.rs` — валидированные слаги + reserved-список + is_prefixed_ulid
- `lifecycle.rs` — LayerLifecycle/ShutdownOutcome; `sync.rs` — Lazy; `serde_helpers.rs`; `error.rs` — CoreError

## Напряжения
- **README vs код (сильный дрейф)**: README:30 утверждает «`CredentialId` lives in `nebula-credential`» — на деле определён здесь (src/id/types.rs:21); README/lib.rs doc перечисляют `NodeId`, `TenantId`, `ProjectId`, `RoleId` (lib.rs:16-17, README:30) — таких типов в коде НЕТ.
- README:50 (контракт L2-§12.5) ссылается на `SecretString` с seam в `crates/core/src/keys.rs` — в крейте нет `SecretString` вообще (живёт в credential-слое).
- README:78 говорит re-export `UuidParseError` — фактически `UlidParseError` (src/id/mod.rs:9).
- README:41 относит `PermissionDenied` к модулю `permission` — фактически в `tenancy.rs:94` (re-export из tenancy, lib.rs:95).
- Deprecated alias `OrganizationId = OrgId` (src/id/types.rs:24-25) — миграционный, re-export под `#[allow(deprecated)]` (lib.rs:82) и в prelude (lib.rs:114).
- `SensitiveScheme`/`PublicScheme` взаимоисключаемость только macro-enforced, hand-rolled двойной impl компилируется — задокументированная дыра с планом в `docs/tracking/credential-concerns-register.md` (src/auth.rs:97-117).
- Обрыв доков: src/dependencies.rs:34 «per ///.» — предложение оборвано (видимо после удаления plan-ID).
- README scope-иерархия «Global → Organization → Project → Workflow → Execution → Action» (README:32) не совпадает с кодом: `ScopeLevel` = Global/Organization/Workspace/Workflow/Execution, без Project и Action (src/scope.rs:24-39).

## Роль в credential/resource redesign
Прямо затронут: `auth.rs` — канонический дом `AuthScheme`/`AuthPattern`/Sensitive-Public дихотомии,
на котором стоит credential rewrite (scheme-enum/Protocol-модель, SchemeFactory, hot-swap);
`dependencies.rs` (SlotField/slot_bindings) — словарь bind-population (M12.4);
`guard.rs` + `accessor.rs` (`RefreshCoordinator`, `CredentialAccessor`) — seam для hot-swap handles
и framework-owned acquire loop (ADR-0093/topology). Изменения здесь каскадируют на все 15 зависимых крейтов.
