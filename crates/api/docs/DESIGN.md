# nebula-api — design

| Field | Value |
|-------|-------|
| **Status** | Frontier — HTTP entry point / API Gateway (pure library, no `main`) |
| **Layer** | Transport boundary (top of the stack; delegates down to engine/storage/credential ports) |
| **Redesign role** | **Credential — touched hard** (production consumer of the `CredentialService` facade, ADR-0088 D7); **Resource — touched at the edge** (legacy `ResourceRepo` seam survives until bind-population / topology land). Not the owner of any credential/resource logic — it is the inbound shell. |
| **Related** | [ADR-0082](../../../docs/adr/0082-api-webhooks-idempotency.md) (API edge — OpenAPI/webhooks/idempotency, absorbs 0047), [ADR-0085](../../../docs/adr/0085-oauth-identity-providers-from-secrets.md) (Plane-A OAuth), [ADR-0088](../../../docs/adr/0088-credential-subsystem-rewrite.md) D7 (facade wiring), [ADR-0092](../../../docs/adr/0092-credential-subsystem-consolidation.md), PRODUCT_CANON §4.5 / §12.2 / §12.3 / §12.4 / §13 |

---

## 1. Назначение и границы

`nebula-api` — HTTP-входная точка Nebula (axum, API Gateway, EIP "Message
Endpoint"). Она переводит REST-запросы в типизированные вызовы port-трейтов,
взятых из `AppState`, и делегирует всю бизнес-логику вниз по стеку
(`crates/api/src/lib.rs:1-6`). Это **чистая библиотека** — без `main`;
composition root живёт в `apps/server` (`crates/api/AGENTS.md:35`).

**Владеет:** сборкой axum-`Router` и middleware-стеком (`build_app`), RFC 9457
error-envelope (`ApiError` / `ProblemDetails`), курсорной пагинацией, API-tier
port-трейтами тенант-резолюции (`OrgResolver` / `WorkspaceResolver` /
`MembershipStore`), inbound-транспортами (converged webhook + OAuth2 Plane-A
sign-in и Plane-B integration-flow), OpenAPI-спекой (drift = ошибка
компиляции), а также API-owned портами (`credential_builder/schema/
schema_registry/service_factory`, `email`, `reqwest_transport`).

**Явно НЕ делает:** не содержит SQL-драйвера и знания о схемах хранилища (API
purity — детали хранения принадлежат `nebula-storage` и инжектятся через порты);
не исполняет workflow (это `nebula-engine`); не вычисляет выражения; не делает
**исходящих** HTTP-вызовов кроме узких серверных OAuth-обменов (webhook
**delivery** outbound живёт в action-плагинах — здесь только inbound-приём);
WebSocket/SSE real-time пока не подключён end-to-end (§4.5 — спрятан до готовности
движка).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `build_app(state, config) -> Router` (merge OpenApiRouter + middleware-стек) | `src/app.rs:30` |
| `serve` / `serve_with_shutdown` | `src/app.rs:357 / 392` |
| `AppState` (держатель портов, 1344 строки) | `src/state.rs:211` |
| `OrgResolver` / `WorkspaceResolver` | `src/state.rs:36 / 43` |
| `MembershipStore` с org-lockout инвариантом `add/remove_member_guarded` (исходы `state.rs:75 / 89`) | `src/state.rs:115` |
| `ApiError` (`#[non_exhaustive]`) / `ApiResult` | `src/error/mod.rs:35 / 513` |
| `ProblemDetails` (RFC 9457) | `src/error/problem.rs:15` |
| `ApiConfig` + `JwtSecret` (без `Default` — fail-hard) + суб-конфиги Tls/Cookie/Cors/Versioning/Idempotency/Auth/Webhook/Smtp/Pagination | `src/config/mod.rs:67`, `src/config/sub.rs` |
| `OAuthProvidersConfig` / `OAuthProviderConfig` | `src/config/oauth.rs:56 / 97` |
| `domain::create_routes` (сборка всех роутов; tenant-вложение `/api/v1/orgs/{org}/workspaces/{ws}/…`) | `src/domain/mod.rs:62` |
| Credential service-layer `create/get/update/delete/list/test/refresh/revoke/resolve/continue_resolve_credential` (всё через `CredentialService` фасад) | `src/transport/credential.rs:324-703` |
| `try_default_credential_service` (composition-root фабрика, SQLite по `NEBULA_CRED_DB`) | `src/ports/credential_service_factory.rs:1-21` |
| `try_default_registry_port` | `src/ports/credential_schema_registry.rs:220` |
| `WebhookTransport` / `WebhookKey` / `WebhookRateLimiter` / `bootstrap_webhook_activations` / `TriggerLifecycleSubscriber` / `Bus` | `transport/webhook/{transport.rs:99,key.rs:59,ratelimit.rs:58,bootstrap.rs:170,events.rs:110/354}` |
| `OAuthStateSigner` / `SignedOAuthState` / `fetch_userinfo` | `transport/oauth/{state.rs:68/28,userinfo.rs:123}` |
| `parse_pat_grant` / `validate_new_pat_scopes` (PAT-scopes) | `src/access/scope.rs:91 / 113` |
| `EmailPort` / `EmailMessage` / `EchoSink` | `src/ports/email.rs:111 / 65 / 126` |
| `init_api_telemetry` / `TelemetryGuard` | `src/telemetry_init.rs:205 / 101` |
| Re-exports: `build_app`, `ApiConfig`, `ApiError`, `AppState`, `CursorParams`/`PaginatedResponse`, `map_resource_create/update_storage_error` | `src/lib.rs:111-119` |

## 3. Зависимости и зависимые

**Workspace deps** (`Cargo.toml:15-42`): `nebula-env`, `nebula-error` (derive),
`nebula-storage` (фичи `credential-in-memory`, `sqlite`), `nebula-storage-port`,
`nebula-tenancy` (scoping-декораторы — граница мультитенантности, allowlist в
`deny.toml`), `nebula-core`, `nebula-execution`, `nebula-validator`,
`nebula-metrics`, `nebula-workflow`, `nebula-action`, `nebula-engine`,
`nebula-plugin`, `nebula-resilience`, `nebula-credential`, `nebula-schema`
(schemars; только для `CredentialSchemaPort`, DTO остаются `serde_json::Value`),
`nebula-eventbus`.

**Внешние ключевые:** axum, tower(-http), utoipa (+axum +swagger-ui), governor,
jsonwebtoken, argon2, totp-rs, secrecy, moka/dashmap, opentelemetry*.
Опционально `sqlx` за фичей `postgres` (→ `PgAuthBackend` + `PgIdempotencyStore`).

**Features:** `test-util` (`ApiConfig::for_test`, обходит JwtSecret-гейт — не для
prod), `postgres`; custom cfg `nebula_test_util` (НЕ feature) + `compile_error`-
гард на release-сборке (`lib.rs:93-109`).

**Зависят от nebula-api:** `apps/server` (`apps/server/Cargo.toml:15`) и
`examples` (`examples/Cargo.toml:24`). **Ни один `crates/*` не зависит** от
api — это вершина стека.

## 4. Внутренняя архитектура

- `app.rs` — `build_app`: merge `OpenApiRouter`, `split_for_parts`, сборка
  middleware-стека (порядок load-bearing: **auth до csrf**), `serve`.
- `state.rs` — `AppState` + API-tier port-трейты + scoped-обёртки `nebula-tenancy`.
- `config/` — `ApiConfig`, env-парсинг, `JwtSecret`, OAuth-провайдеры, суб-конфиги.
- `domain/` — по-доменные `{routes, handler, dto}`: auth (Plane A; backends
  in_memory/pg/password/pat/mfa/session/oauth/provider), me, org (+membership),
  workflow, execution, credential (+oauth, schema_projection), catalog, health,
  resource; `shared` (курсорная пагинация, `AckResponse`); `workspace`/`internal`/
  `metrics` — сборочный роутинг.
- `error/` — `ApiError`, `ProblemDetails`, `classify` (seam legacy
  `StorageError`→`ApiError`).
- `extractors/` — JSON-extractor, валидаторы credential-входа.
- `middleware/` — auth (JWT+API-key→`AuthContext`), tenancy, rbac, csrf,
  idempotency (key/layer/memory/store), internal_auth, rate_limit, request_id,
  security_headers, trace_w3c.
- `openapi/` — `OpenApiDoc` (ADR-0047, drift = ошибка компиляции) + `audit.md`
  (таблица статусов хендлеров).
- `ports/` — API-owned порты: `credential_builder/schema/schema_registry/
  service_factory`, `email`, `reqwest_transport`.
- `transport/` — `credential` (service-layer через фасад), `oauth`
  (discovery/flow/http/state/userinfo), `webhook` (единый inbound:
  dispatch/key/provider/ratelimit/replay/routing/signature/bootstrap/events).

**Поток данных:** REST-запрос → middleware-стек (request_id → trace_w3c → auth →
tenancy → rbac → csrf → idempotency → rate_limit) → доменный handler →
port-трейт из `AppState` → нижний крейт. Ошибки сворачиваются в типизированный
`ApiError` и сериализуются как `application/problem+json`.

## 5. Инварианты и контракты

- **[§12.4] RFC 9457 повсюду.** Каждый путь отказа маппится в типизированный
  `ApiError`-вариант с явным HTTP-статусом; новых ad-hoc 500 для бизнес-логики
  нет. Seam: `error/mod.rs`.
- **[§12.2] Durable outbox.** Cancel/terminate пишут долговечный сигнал в
  control-queue в той же логической операции, что и переход состояния — не голый
  DB-flip; второй несверяемый in-memory канал запрещён.
- **[§4.5 operational honesty] by-construction.** Заглушки возвращают честный
  501/503, а не фейк-успех; `openapi/audit.md` + тесты
  (`openapi_canon_compliance.rs`) держат это в обе стороны — молча отгруженный
  endpoint не пройдёт ревью.
- **OpenAPI drift = ошибка компиляции** (ADR-0047): монтирование только через
  `OpenApiRouter::routes(routes!(handler))`; хендлер без `#[utoipa::path]` не
  проходит. Рантайм-гарды в `tests/openapi_spec.rs`.
- **Middleware order load-bearing:** `auth_middleware` строго до
  `csrf_middleware` — csrf читает `AuthContext`, который ставит auth (чтобы
  пропускать PAT/ApiKey-вызовы без cookie).
- **Credential owner-isolation by-construction:** каждая credential-операция
  проходит через фасад, который owner-чекает по канону
  `Scope::credential_owner_id`; cross-workspace id схлопывается в плоский 404.
  Нет raw-store fallback — нет фасада ⇒ честный 503 (`transport/credential.rs:6-24`).
- **Мультитенантность через `nebula-tenancy`-декораторы** (граница в allowlist
  `deny.toml`): порты приходят в `AppState` уже обёрнутыми scope-enforcing
  декоратором из composition root — никогда не raw legacy repo.
- **Anti-SSRF на OAuth-исходящих:** каждый серверный OAuth-URL проходит
  `validate_oauth_outbound_url` (HTTPS-only; отказ loopback/private/link-local);
  body-caps 256 KiB; токены выбрасываются после userinfo (ADR-0085).

## 6. Известные напряжения / долг

1. **Legacy resource-seam.** `domain/resource` сидит на сохранённом legacy
   `nebula_storage::repos::ResourceRepo` с legacy `StorageError`; адаптер-
   классификация в `error/mod.rs:171-183`, re-export мапперов в `lib.rs:113-115`
   — единственный путь мимо spec-16 портов. Подлежит замене при приземлении
   bind-population / topology.
2. **Двойной auth-контекст.** `middleware/auth.rs:4,106` вставляет «legacy»
   `AuthenticatedUser` в extensions рядом с `AuthContext`; re-exports для legacy-
   потребителей в `domain/auth/handler.rs:493`. Два представления одного факта.
3. **Deprecated alias.** `transport/oauth/flow.rs:198-203` — `#[deprecated]`
   алиас на `validate_oauth_outbound_url` (sunset-долг).
4. **Честные 501-заглушки остаются** (`openapi/audit.md:57,100`): `list_my_orgs`
   (нет principal→orgs enumeration в `MembershipStore`/storage),
   `restart_execution` (семантика за engine-командой), org-record и
   service-account endpoints (нет store / нет `Principal::ServiceAccount` пути).
5. **Legacy-совместимость в `state.rs`:** byte-identical Conflict-сообщение
   (`state.rs:613,662`), сортировка по RFC3339-строке или legacy i64
   (`state.rs:694,729`) — наследие старых хендлеров.
6. **`PgAuthBackend` владеет собственным `sqlx::Pool`** для 2 транзакционных флоу
   (`Cargo.toml:101-113`) — осознанный обход `nebula-storage`.
7. **Документная неточность:** `AGENTS.md:11` говорит «register route in
   src/app.rs», фактически роуты регистрируются в `domain/*/routes.rs` +
   `domain/mod.rs:62` (app.rs только собирает).

## 7. Роль в пост-0092 credential/resource модели

**Credential — затронут сильно.** `nebula-api` — единственный production-
потребитель `CredentialService`-фасада (ADR-0088 D7 wiring DONE). Все десять
операций (`create/get/update/delete/list` + lifecycle `test/refresh/revoke` +
acquisition `resolve/resolve/continue`) идут одним persistence-путём через фасад
(`transport/credential.rs:324-703`); при отсутствии фасада — честный 503
(`transport/credential.rs:6-24`), без raw-store fallback. Фабрика фасада для
`apps/server` живёт здесь (`ports/credential_service_factory.rs`).

Что меняется при пост-0092 консолидации `nebula-credential` (одна crate:
contract + runtime ex-engine + `CredentialService` ex-credential-runtime +
builtin types ex-credential-builtin; крейты runtime/builtin/testutil/vault
**удалены**):

- **Импорты, не контракт.** Слияние runtime→credential и dyn-erasure (P4 плана
  facade-nongeneric) ударят по **путям импорта** в `transport/credential.rs` и
  `ports/credential_*` — фасадный seam (одна точка входа, owner-чек,
  values-only persistence) остаётся стабильным. API видит фасад, а не его
  внутреннюю топологию.
- **Schema-проекция.** `nebula-schema` тянется сюда только ради
  `CredentialSchemaPort`; в пост-0092 модели схема приходит из
  зарегистрированных типов (`HasSchema` → `nebula-metadata` → API-каталог), а
  DTO остаются `serde_json::Value` — это уже текущее состояние и оно сохраняется.
- **OAuth2 Plane-B.** `domain/credential/oauth.rs` + `transport/oauth` —
  кандидат на переезд в `nebula-credential` по rewrite-плану (OAuth2-протокол —
  собственность credential-крейта, не транспортного шелла). Конференц-коррекции
  (policy(&State)-routing, `OwnerScopedKey`, узкий типизированный
  `RefreshTransport` seam, lease как first-class) приземляются **внутри**
  `nebula-credential`; API лишь дёргает фасад и наследует их by-construction.
- **`#[property]` / unified authoring — Phase-5, NOT-YET-BUILT.** API сегодня
  принимает credential-вход через DTO + extractors-валидаторы; слотовая модель
  (`#[credential]`/`#[resource]` slots, `slot_bindings` отдельно от parameters,
  `CredentialGuard<Scheme>`) — это consumer-binding в action/resource, не в
  api-слое. API остаётся values-only write-path.

**Resource — затронут по краю.** `domain/resource` — CRUD/status API поверх
legacy `ResourceRepo` (см. §6.1). Per-slot rotation fan-out, `SlotCell`, Manager
и topology теперь принадлежат `nebula-resource`; bind-population (M12.4) и
topology-инверсия (ADR-0093) до api-слоя **пока не дошли**. Когда дойдут,
legacy-seam (`error/mod.rs:171`, `lib.rs:113-115`) заменяется на spec-16
resource-порт — это запланированная замена, не shim.

## 8. Forward design / открытые вопросы

- **Снять legacy resource-seam** одновременно с приземлением bind-population
  (M12.4): заменить `ResourceRepo`/`StorageError`-классификацию в
  `error/mod.rs:171-183` + `lib.rs:113-115` на scope-обёрнутый spec-16
  resource-порт; до тех пор `resource/list` остаётся честным 501.
- **Поглотить facade dyn-erasure (P4).** При слиянии runtime→credential и
  RPITIT/generic-method bridge переписать импорты `transport/credential.rs` +
  `ports/credential_*`; проверить, что фасадный API остаётся не-generic для
  потребителя.
- **Решить судьбу OAuth2 Plane-B.** Если протокол переезжает в
  `nebula-credential`, api сохраняет только тонкий редирект/callback-транспорт; в
  противном случае узаконить `transport/oauth` как постоянную часть api-границы и
  снять `#[deprecated]` alias-долг (§6.3).
- **Закрыть двойной auth-контекст** (§6.2): один источник истины
  (`AuthContext`), удалить legacy `AuthenticatedUser` после миграции
  потребителей — структурный фикс, не дисциплина.
- **Durability-фронтир.** `me/*`, credential CRUD и org-membership работают
  end-to-end, но единственный wired backend — in-memory (нет storage-backed
  `AuthBackend`/`UserRepo`/`PatRepo`/`SessionRepo`/membership repo). 1.0-closure
  требует durable backend за фасадом и storage-backed auth — это работа
  `nebula-storage`, но api-контракт (честный 503/local-first caveat) уже её
  ожидает.
- **Promote slug-webhook surface в OpenAPI** (1.0 follow-up): требует
  типизированных схем для request-envelope каждого провайдера.
- **Риск дрейфа доков:** держать `AGENTS.md` синхронным с фактической
  регистрацией роутов (`domain/*/routes.rs`, не `app.rs` — §6.7).
