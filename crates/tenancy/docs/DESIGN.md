# nebula-tenancy — design

| Field | Value |
|-------|-------|
| **Status** | Stable (портовая половина) / redesign-кандидат (credential-половина) — Business tier |
| **Layer** | Business tier (security boundary мультитенантности; зависит от Core-tier `nebula-storage-port`) |
| **Redesign role** | **Частично затронут.** Портовая половина (`ScopeResolver` + scope-substituting декораторы) — стабильный продукт ADR-0072, redesign её **не трогает**. Credential-половина (`CredentialScopeLayer` / `CredentialScopeResolver`, `src/credential_scope.rs`) — это «ScopeLayer sole»-кандидат на удаление при переходе credential-стека на owner-модель фасада `CredentialService`. |
| **Related** | ADR-0072 (port/adapter/tenancy), ADR-0066 (CredentialService facade owner scoping), spec §6.1/§6.2 (threat model), spec §8 (credential ScopeLayer re-home), `project_credential_api_wiring` (P5 ScopeLayer-delete), `project_credential_rewrite_plan` (scope×3 дубль) |

---

## 1. Назначение и границы

`nebula-tenancy` — это **business-tier security boundary мультитенантности**: крейт владеет
*политикой* изоляции тенантов, но **не данными**. Он выполняет две функции (fact-sheet §Назначение):

1. **`ScopeResolver`** проецирует аутентифицированный `Principal` в портовый
   `Scope { workspace_id, org_id }`. Сам тип `Scope` принадлежит Core-tier `nebula-storage-port`
   (plain-data value type), чтобы сигнатуры портов могли его требовать без upward-зависимости.
2. **Scope-substituting декораторы** оборачивают каждый `Arc<dyn …Store>` порта и **подменяют**
   (substitute, не reject) scope в каждом вызове. engine/api получают только декорированный handle —
   они структурно не могут подделать чужой tenant (confused-deputy закрыт by construction, spec §6.1/§6.2).

**Владеет:** проекцией `Principal -> Scope`, fail-closed-политикой (отсутствие workspace ⇒ отказ, не
расширение до org-only), по-портовыми декораторами с tenant-namespacing (idempotency-ключи `{scope}:{key}`),
coarse-by-design таксономией `TenancyError`, re-homed credential `ScopeLayer`.

**ЯВНО НЕ делает:** не владеет типом `Scope` (он в `nebula-storage-port`); не содержит sqlx / адаптеров /
upward-зависимостей; не аутентифицирует (доверяет binding из auth-слоя); не раскрывает, *какая* половина
scope не совпала (см. §5). Только composition roots зависят от крейта (README, fact-sheet §Workspace-зависимости).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `request_scope(&TenantContext) -> Result<Scope, TenancyError>` — per-request fail-closed проекция для api handlers | `src/resolver.rs:42` |
| `struct Principal { actor: ActorPrincipal, org_id, workspace_id: Option<WorkspaceId> }` + `::workspace()` / `::org()` | `src/resolver.rs:62` |
| `trait ScopeResolver { fn resolve(&Principal) -> Result<Scope, TenancyError> }` | `src/resolver.rs:100` |
| `struct BindingScopeResolver` — дефолт, доверяет binding из auth, fail-closed на отсутствие workspace | `src/resolver.rs:123` |
| `enum TenancyError { MissingWorkspace, Unauthorized }` — намеренно coarse | `src/error.rs:17` |
| `ScopedExecutionStore` (+ `rebind()` пересборки `TransitionBatch` со связанным scope в batch и каждой outbox-строке) | `src/decorator/execution.rs:15` |
| `ScopedWorkflowStore`, `ScopedWorkflowVersionStore` | `src/decorator/workflow.rs` |
| `ScopedControlQueue` | `src/decorator/control_queue.rs` |
| `ScopedIdempotencyStore`, `ScopedIdempotencyGuard` (tenant-namespaced ключи `{scope}:{key}`) | `src/decorator/idempotency.rs` |
| `ScopedExecutionJournalReader` | `src/decorator/journal.rs` |
| `ScopedNodeResultStore` | `src/decorator/node_result.rs` |
| `ScopedResourceStore` | `src/decorator/resource.rs` |
| `ScopedTriggerStore` | `src/decorator/trigger.rs` |
| `ScopedWebhookActivationStore` | `src/decorator/webhook.rs` |
| `CredentialScopeLayer` (= `credential_scope::ScopeLayer<S>`, re-home из nebula-storage, spec §8) — декоратор `CredentialStore`, ключуется на legacy `metadata["owner_id"]`; `None` owner = admin bypass | `src/credential_scope.rs:78` |
| `CredentialScopeResolver` — re-export `nebula_credential::ScopeResolver` (`current_owner() -> Option<&str>`) | `src/lib.rs:47` |
| `verify_owner()` (private) — fail-closed: нет owner_id в metadata ⇒ NotFound для не-admin | `src/credential_scope.rs:225` |

Все декораторы конструируются единообразно: `new(Arc<dyn T>, Scope)` (fact-sheet §Публичная поверхность).

## 3. Зависимости и зависимые

- **Deps:** `nebula-core`, `nebula-storage-port`, `nebula-credential` (+ `async-trait`, `thiserror`,
  `tracing`, `serde_json`; dev: `tokio`, `chrono`). Никаких sqlx / адаптеров / upward-зависимостей.
- **Dependents (только composition roots):**
  - **nebula-storage** (`crates/storage/Cargo.toml:119`) — conformance-тесты.
  - **nebula-api** (`crates/api/Cargo.toml:22`) — `state.rs`, `middleware/tenancy.rs`,
    `transport/credential.rs`, `error/mod.rs` (`From<TenancyError>`).

## 4. Внутренняя архитектура

- `lib.rs` (~48 строк) — re-export фасад. `Credential*`-префиксы введены, чтобы развести **две**
  scope-модели в одном крейте (см. §6).
- `resolver.rs` (~186 строк) — `Principal` / `ScopeResolver` / `BindingScopeResolver` / `request_scope` +
  unit-тесты. Это «вход»: auth-слой → `Principal` → `Scope`.
- `error.rs` (~29 строк) — `TenancyError`.
- `decorator/` (10 файлов, ~1000 строк) — по декоратору на port-трейт. Контракт **substitute-not-reject**:
  декоратор не отклоняет вызов, а подставляет связанный `Scope` в каждый запрос/строку.
- `credential_scope.rs` (~673 строки) — re-homed credential `ScopeLayer` + in-memory double + тесты.
  Поток данных у credential-половины иной: фильтрация по строке `metadata["owner_id"]`, а не по
  портовому `Scope`.
- `tests/cross_tenant_denial.rs` (~891 строка) + `tests/scope_decorator_coverage.rs` (~148 строк) —
  threat-model регрессии (cross-tenant denial, покрытие декораторов).

Поток данных (portовая половина): `Principal` → `ScopeResolver::resolve` → `Scope` → инъекция `Scope` в
конструктор декоратора → каждый вызов store идёт уже с привязанным scope. engine/api никогда не держат
«сырой» `Arc<dyn …Store>`.

## 5. Инварианты и контракты

- **Confused-deputy закрыт by construction (spec §6.1/§6.2).** Поскольку декораторы *подменяют* scope, а
  не валидируют переданный, вызывающий код не имеет канала, через который подделать чужой tenant.
- **Fail-closed проекция.** `BindingScopeResolver` (`src/resolver.rs:123`) и `request_scope`
  (`src/resolver.rs:42`) отклоняют отсутствие workspace, а не «расширяют» до org-only.
- **Coarse-by-design ошибки.** `TenancyError` (`src/error.rs:17`) намеренно не раскрывает, какая половина
  scope не совпала — чтобы не давать существенно-различимый сигнал (existence/denied leak).
- **Tenant-namespacing идемпотентности.** `ScopedIdempotencyStore` / `ScopedIdempotencyGuard` ключуются
  как `{scope}:{key}` — tenant A не может зондировать/отравить dedup-запись tenant B.
- **`rebind()` для batch-транзакций.** `ScopedExecutionStore::rebind()` (`src/decorator/execution.rs:15`)
  пересобирает `TransitionBatch`, проставляя связанный scope и в сам batch, и в каждую outbox-строку.
- **Credential fail-closed.** `verify_owner()` (`src/credential_scope.rs:225`): отсутствие `owner_id` в
  metadata ⇒ `NotFound` для не-admin; `None` owner = admin bypass (явный, документированный).

## 6. Известные напряжения / долг

1. **Стейл-док vs lib.rs (`src/credential_scope.rs:7-9` vs `src/lib.rs:36-39`).** Шапка модуля утверждает
   «nebula_storage::credential now re-exports these under their historical names so every consumer compiles
   unchanged», но `lib.rs:36-39` фиксирует, что legacy-поверхность **удалена** (spec-16 CONTRACT, «no
   back-compat re-export»), и в `crates/storage/src` никаких re-export `ScopeLayer` нет. Комментарий устарел.
2. **Двойная scope-модель в одном крейте (`src/credential_scope.rs:47`).** Портовый
   `Scope { workspace, org }` vs legacy owner-строка `metadata["owner_id"]`. Развилка признана сознательной
   в `lib.rs:26-39`, но это дубль *политики* изоляции внутри одного крейта.
3. **Мёртвый вариант `TenancyError::Unauthorized` (`src/error.rs:28`).** Задокументирован в
   `ScopeResolver::resolve` (`src/resolver.rs:108`), но **нигде в крейте не конструируется** — вариант «на вырост».
4. **N+1 в `ScopeLayer::list` (`src/credential_scope.rs:173-190`).** `get()` на каждый id ради фильтрации по
   owner; приемлемо для in-memory, дорого на реальном бэкенде.
5. **Doctest выключен (`src/credential_scope.rs:61-77`).** Пример помечен `rust,ignore` + `[lib] doctest = false`
   — он не компилируется в CI.
6. **Catch-all для `#[non_exhaustive] PutMode` (`src/credential_scope.rs:140-147`).** `_` дефолтит в строгий
   путь — корректно fail-closed, но новые режимы молча получают owner-stamp семантику без явного решения.

## 7. Роль в пост-0092 credential/resource модели

Крейт затронут **асимметрично** — две его половины живут в разных режимах относительно redesign.

**Портовая половина — стабильна, redesign её не трогает.** `ScopeResolver` / `BindingScopeResolver` /
`request_scope` и весь `decorator/`-набор — продукт ADR-0072 (port/adapter/tenancy). Они изолируют
*любой* портовый store, включая `ScopedResourceStore` (`src/decorator/resource.rs`). В пост-0092 модели,
где `nebula-resource` владеет per-slot rotation fan-out, SlotCell, Manager и топологией, tenancy остаётся
лишь scope-обёрткой портового resource-store — resource redesign касается этой обёртки только как порта,
её контракт не меняется. Аналогично credential durable-store-декораторы здесь не дублируются: durable
stores + Encryption/Cache/Audit decorators + KeyProvider + RefreshClaimRepo живут в `nebula-storage`.

**Credential-половина — «ScopeLayer sole»-кандидат на удаление.** `CredentialScopeLayer` /
`CredentialScopeResolver` (`src/credential_scope.rs`, ~673 строки) — re-homed legacy-слой, ключующийся на
строке `metadata["owner_id"]`. Это та самая половина из rewrite-плана:

- `project_credential_api_wiring` помечает следующий цикл как включающий **P5 ScopeLayer-delete**;
- `project_credential_rewrite_plan` фиксирует **scope×3 дубль** с целевым *единственным* scope-слоем.

В пост-0092 модели credential = один крейт `nebula-credential` (contract + runtime: resolver/refresh/lease/
rotation-state + `CredentialService` facade + builtin types). **Facade-level owner scoping уже есть в runtime**
(ADR-0066): `OwnerScopedKey` обеспечивает owner-изоляцию на уровне фасада. Это означает, что owner-проекция,
которую сегодня делает `CredentialScopeLayer` строкой `owner_id` в metadata, в целевой модели обеспечивается
фасадом `CredentialService` по-конструкции — и legacy-слой в tenancy становится дублем, подлежащим удалению
(а не «миграции через shim»).

**Швы, релевантные пост-0092 (для контекста, не реализуются здесь):**

- **Consumer binding** — action/resource объявляют `#[credential]` / `#[resource]` слоты и получают
  `CredentialGuard<Scheme>`; `slot_bindings` отделены от parameters; persistence values-only (schema из
  зарегистрированных типов через `HasSchema -> nebula-metadata -> API catalog`). Эта проекция «кто что
  видит» — задача consumer-binding и фасада, не legacy `ScopeLayer`.
- **policy(&State) драйвит routing**, narrow typed `RefreshTransport` seam, lease first-class —
  conference-corrections для credential-стека; они укрепляют именно фасадную модель, в которую переходит
  owner-scoping.

**Итого по redesign-роли:** портовая половина = «not-touched» стабильный продукт; credential-половина =
«touched», явный кандидат на удаление в P5, без shim/bridge — owner-scoping переезжает в фасад
`CredentialService`, а не дублируется в tenancy.

## 8. Forward design / открытые вопросы

1. **Исполнить P5 ScopeLayer-delete как hard removal.** При переходе credential-стека на фасадный
   owner-scoping (ADR-0066) удалить `src/credential_scope.rs` целиком вместе с `CredentialScopeLayer` /
   `CredentialScopeResolver` re-export, а не оставлять как параллельную модель. Это снимает напряжения §6.1
   (стейл-док), §6.2 (двойная scope-модель), §6.4 (N+1), §6.5 (выключенный doctest), §6.6 (catch-all
   PutMode) одним удалением.
2. **Решить судьбу `TenancyError::Unauthorized` (§6.3).** Либо начать конструировать вариант на портовом
   пути (если появится сценарий «аутентифицирован, но не авторизован для scope»), либо удалить мёртвый
   вариант — держать coarse-by-design таксономию честной.
3. **Зафиксировать инвариант coarse-ошибок тестом.** Сейчас «не раскрывать, какая половина scope не совпала»
   держится дисциплиной; стоит закрепить регрессией в `tests/cross_tenant_denial.rs`, чтобы будущие
   декораторы не вернули существенно-различимый сигнал.
4. **Риск переходного периода.** Пока `CredentialScopeLayer` сосуществует с фасадным owner-scoping, есть две
   точки решения «кто владелец credential». До удаления P5 любой новый credential-путь должен идти через
   фасад, чтобы legacy-слой не получал новых вызовов (иначе удаление снова откладывается).
5. **`ScopedResourceStore` под resource redesign.** Подтвердить, что открытая топология / per-slot fan-out в
   `nebula-resource` не требует от tenancy ничего сверх портовой scope-обёртки; если resource-store обретёт
   новые портовые методы, добавить их в декоратор по тому же substitute-not-reject контракту.
