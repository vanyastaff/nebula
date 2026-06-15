# nebula-tenancy — fact sheet

## Назначение
Business-tier security boundary мультитенантности: владеет *политикой* изоляции, но не данными.
Две функции: (1) `ScopeResolver` проецирует аутентифицированный `Principal` в портовый `Scope { workspace_id, org_id }`
(тип принадлежит `nebula-storage-port`); (2) scope-substituting декораторы оборачивают каждый `Arc<dyn …Store>` порта
и **подменяют** scope в каждом вызове — engine/api структурно не могут подделать чужой tenant (confused-deputy закрыт by construction, spec §6.1/§6.2).

## Публичная поверхность
- `request_scope(&TenantContext) -> Result<Scope, TenancyError>` — per-request fail-closed проекция для api handlers — src/resolver.rs:42
- `struct Principal { actor: ActorPrincipal, org_id, workspace_id: Option<WorkspaceId> }` + `::workspace()` / `::org()` — src/resolver.rs:62
- `trait ScopeResolver { fn resolve(&Principal) -> Result<Scope, TenancyError> }` — src/resolver.rs:100
- `struct BindingScopeResolver` — дефолтный резолвер, доверяет binding из auth-слоя, fail-closed на отсутствие workspace — src/resolver.rs:123
- `enum TenancyError { MissingWorkspace, Unauthorized }` — намеренно coarse (не раскрывает какая половина не совпала) — src/error.rs:17
- Декораторы (один на port-trait, все `new(Arc<dyn T>, Scope)`):
  - `ScopedExecutionStore` (+ `rebind()` пересборки `TransitionBatch` с bound scope в batch и каждой outbox-строке) — src/decorator/execution.rs:15
  - `ScopedWorkflowStore`, `ScopedWorkflowVersionStore` — src/decorator/workflow.rs
  - `ScopedControlQueue` — src/decorator/control_queue.rs
  - `ScopedIdempotencyStore`, `ScopedIdempotencyGuard` (tenant-namespaced ключи `{scope}:{key}`) — src/decorator/idempotency.rs
  - `ScopedExecutionJournalReader` — src/decorator/journal.rs
  - `ScopedNodeResultStore` — src/decorator/node_result.rs
  - `ScopedResourceStore` — src/decorator/resource.rs
  - `ScopedTriggerStore` — src/decorator/trigger.rs
  - `ScopedWebhookActivationStore` — src/decorator/webhook.rs
- `CredentialScopeLayer` (= `credential_scope::ScopeLayer<S>`, re-home из nebula-storage, spec §8): декоратор `CredentialStore`, ключуется на legacy `metadata["owner_id"]`; `None` owner = admin bypass — src/credential_scope.rs:78
- `CredentialScopeResolver` — re-export `nebula_credential::ScopeResolver` (`current_owner() -> Option<&str>`) — src/lib.rs:47
- `verify_owner()` (private) — fail-closed: нет owner_id в metadata ⇒ NotFound для не-admin — src/credential_scope.rs:225

## Workspace-зависимости
Deps: `nebula-core`, `nebula-storage-port`, `nebula-credential` (+ async-trait, thiserror, tracing, serde_json; dev: tokio, chrono).
Зависят от него: **nebula-storage** (crates/storage/Cargo.toml:119, conformance-тесты) и **nebula-api** (crates/api/Cargo.toml:22 — `state.rs`, `middleware/tenancy.rs`, `transport/credential.rs`, `error/mod.rs` From<TenancyError>). Только composition roots, как декларирует README.

## Структура модулей
- `lib.rs` (48 строк) — re-export фасад; `Credential*`-префиксы во избежание коллизии двух scope-моделей
- `resolver.rs` (186) — Principal / ScopeResolver / BindingScopeResolver / request_scope + unit-тесты
- `error.rs` (29) — TenancyError
- `decorator/` (10 файлов, ~1000 строк) — по декоратору на port-трейт; substitute-not-reject
- `credential_scope.rs` (673) — re-homed credential ScopeLayer + in-memory double + тесты
- `tests/cross_tenant_denial.rs` (891) + `tests/scope_decorator_coverage.rs` (148) — threat-model регрессии

## Напряжения
- **Стейл-док vs lib.rs**: credential_scope.rs:7-9 утверждает «nebula_storage::credential now re-exports these under their historical names so every consumer compiles unchanged» — но lib.rs:36-39 фиксирует, что legacy-поверхность **удалена** (spec-16 CONTRACT, «no back-compat re-export»), и в crates/storage/src никаких re-export ScopeLayer нет (только doc-упоминания). Шапка модуля устарела.
- **Двойная scope-модель** в одном крейте: портовый `Scope{workspace,org}` vs legacy owner-строка `metadata["owner_id"]` (credential_scope.rs:47). Признано в lib.rs:26-39 как сознательная развилка, но это и есть дубль политики.
- `TenancyError::Unauthorized` (error.rs:28) задокументирован в `ScopeResolver::resolve` (resolver.rs:108), но **нигде в крейте не конструируется** — мёртвый вариант «на вырост».
- `ScopeLayer::list` — N+1: `get()` на каждый id для фильтрации по owner (credential_scope.rs:173-190); приемлемо для in-memory, дорого на реальном бэкенде.
- Doctest-пример в credential_scope.rs:61-77 — `rust,ignore` + `[lib] doctest = false`: пример не компилируется CI.
- Catch-all `_` для `#[non_exhaustive] PutMode` дефолтит в строгий путь (credential_scope.rs:140-147) — корректно fail-closed, но новые режимы молча получают owner-stamp семантику.

## Роль в credential/resource redesign
Затронут напрямую. `CredentialScopeLayer`/`CredentialScopeResolver` — это «ScopeLayer sole»-половина rewrite-плана: по project_credential_api_wiring следующий цикл включает **P5 ScopeLayer-delete**, а rewrite-план (project_credential_rewrite_plan) фиксирует scope×3 дубль с целевым единственным scope-слоем. Т.е. credential_scope.rs (673 строки) — кандидат на удаление/замену при переходе credential-стека на owner_id-модель фасада `CredentialService` (facade-level owner scoping уже в runtime, ADR-0066). Портовая половина (`ScopeResolver`/декораторы) — стабильный продукт ADR-0072, redesign её не трогает; resource redesign касается лишь `ScopedResourceStore` как обёртки порта.
