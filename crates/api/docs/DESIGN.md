# nebula-api — design

| Field | Value |
|-------|-------|
| **Status** | Frontier — HTTP entry point / API Gateway (pure library, no `main`) |
| **Layer** | Transport boundary (top of the stack; delegates down to engine/storage/credential ports) |
| **Redesign role** | **Credential — touched hard** (production consumer of the `CredentialService` facade, ADR-0088 D7); **Resource — touched at the edge** (legacy `ResourceRepo` seam survives until bind-population / topology land). Not the owner of any credential/resource logic — it is the inbound shell. |
| **Related** | ADR-0082 (API edge — OpenAPI/webhooks/idempotency, absorbs 0047), ADR-0085 (Plane-A OAuth), ADR-0088 D7 (facade wiring), ADR-0092, PRODUCT_CANON §4.5 / §12.2 / §12.3 / §12.4 / §13 |

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
sign-in), Plane-B фасадными маршрутами CRUD/lifecycle/`resolve`/`continue`,
OpenAPI-спекой (drift = ошибка компиляции), а также API-owned портами (`credential_builder/schema/
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
| `OAuthIdentityRuntime::from_config` / `OAuthRuntimeBuildError` (узкий technical composition seam; не интеграционный SDK) | `src/transport/oauth/{runtime,error}.rs` + root re-export |
| `OAuthProvider` / `AuthError` (`#[non_exhaustive]`; downstream match обязан иметь wildcard) | `src/domain/auth/backend/{oauth,error}.rs` |
| `domain::create_routes` (сборка всех роутов; tenant-вложение `/api/v1/orgs/{org}/workspaces/{ws}/…`) | `src/domain/mod.rs:62` |
| Credential service-layer `create/get/update/delete/list/test/refresh/revoke/resolve/continue_resolve_credential` (всё через `CredentialService` фасад) | `src/transport/credential.rs:324-703` |
| `try_default_credential_service` (composition-root фабрика, SQLite по `NEBULA_CRED_DB`) | `src/ports/credential_service_factory.rs:1-21` |
| `try_default_registry_port` | `src/ports/credential_schema_registry.rs:220` |
| `WebhookTransport` / `WebhookKey` / `WebhookRateLimiter` / `bootstrap_webhook_activations` / `TriggerLifecycleSubscriber` / `Bus` | `transport/webhook/{transport.rs:99,key.rs:59,ratelimit.rs:58,bootstrap.rs:170,events.rs:110/354}` |
| Private Plane-A OAuth runtime: fixed egress / closed failures / config+cache+deadline owner | `transport/oauth/{egress,error,runtime}.rs` |
| `parse_pat_grant` / `validate_new_pat_scopes` (PAT-scopes) | `src/access/scope.rs:91 / 113` |
| `EmailPort` / `EmailMessage` / `EchoSink` | `src/ports/email.rs:111 / 65 / 126` |
| `init_api_telemetry` / `TelemetryGuard` | `src/telemetry_init.rs:205 / 101` |
| Re-exports: `build_app`, `ApiConfig`, `ApiError`, `AppState`, `CursorParams`/`PaginatedResponse`, `map_resource_create/update_storage_error`, technical `OAuthIdentityRuntime` / `OAuthRuntimeBuildError` | `src/lib.rs` |

`nebula-api` остаётся технической HTTP/composition-границей. Единственная
поддерживаемая и брендированная Rust-поверхность для пользователей и авторов
интеграций — persona-oriented `nebula-sdk`; root re-export OAuth runtime нужен
composition root и не превращает внутреннюю OAuth-механику во второй SDK.

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
prod), `postgres`. У Plane-A OAuth нет custom-cfg или публичного test-support
обхода: hermetic-тесты используют приватный `cfg(test)` TLS-fixture с тем же
fixed-policy client builder, что production.

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
  workflow, execution, credential (+schema_projection), catalog, health,
  resource; `shared` (курсорная пагинация, `AckResponse`); `workspace`/`internal`/
  `metrics` — сборочный роутинг.
- `error/` — `ApiError`, `ProblemDetails`, `classify` (seam legacy
  `StorageError`→`ApiError`).
- `extractors/` — JSON-extractor, валидаторы credential-входа.
- `middleware/` — auth (explicit JWT/PAT/API-key before ambient session
  cookie → `AuthContext`), tenancy, rbac, csrf,
  idempotency (key/layer/memory/store), internal_auth, rate_limit, request_id,
  security_headers, trace_w3c.
- `openapi/` — `OpenApiDoc` (ADR-0047, drift = ошибка компиляции). Инвентарь
  статусов хендлеров (501-заглушки) заархивирован в приватном design-vault.
- `ports/` — API-owned порты: `credential_builder/schema/schema_registry/
  service_factory`, `email`, `reqwest_transport`.
- `transport/` — `credential` (service-layer через фасад), приватный `oauth`
  (Plane-A-only `egress/error/runtime`), `webhook` (единый inbound:
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
  501/503, а не фейк-успех; тесты
  (`openapi_canon_compliance.rs`) держат это в обе стороны — молча отгруженный
  endpoint не пройдёт ревью.
- **OpenAPI drift = ошибка компиляции** (ADR-0047): монтирование только через
  `OpenApiRouter::routes(routes!(handler))`; хендлер без `#[utoipa::path]` не
  проходит. Рантайм-гарды в `tests/openapi_spec.rs`.
- **Middleware order load-bearing:** `auth_middleware` строго до
  `csrf_middleware` — csrf читает `AuthContext`, который ставит auth (чтобы
  проверять double-submit только для session cookie и пропускать явные
  JWT/PAT/API-key credentials).
- **Credential owner-isolation by-construction:** каждая credential-операция
  проходит через фасад, который owner-чекает по канону
  `Scope::credential_owner_id`; cross-workspace id схлопывается в плоский 404.
  Нет raw-store fallback — нет фасада ⇒ честный 503 (`transport/credential.rs:6-24`).
- **Мультитенантность через `nebula-tenancy`-декораторы** (граница в allowlist
  `deny.toml`): порты приходят в `AppState` уже обёрнутыми scope-enforcing
  декоратором из composition root — никогда не raw legacy repo.
- **Plane-A OAuth имеет два фиксированных профиля и одного долгоживущего
  владельца секретов:** 1.0 допускает только canonical Google OIDC и GitHub.com;
  operator задаёт только пары `CLIENT_ID`/`CLIENT_SECRET`. Microsoft, generic
  OIDC, GitHub Enterprise Server, endpoint/scope/auth overrides и operator JWKS
  припаркованы и fail boot. `ApiConfig` временно владеет `SecretString` при
  load-validation, затем `apps/server` перемещает map в
  `OAuthIdentityRuntime`; router config остаётся с пустым map, а backend получает
  только тот же opaque `Arc`, без raw client, копии secret config или
  replaceable policy.
- **Connect-time anti-SSRF:** server-fetched URL — только rustls HTTPS; redirects,
  retries и proxies отключены. Literal IP и все DNS-ответы проходят один IANA-
  ориентированный global-routability classifier; пустой, >32 или смешанный
  public/private набор отвергается, а reqwest получает только точные уже
  проверенные `SocketAddr`. Response body с начала хранится в zeroizing-буфере с
  cap 256 KiB. Bearer token остаётся внутри opaque one-shot capability.
- **Token endpoint auth однозначен:** GitHub.com фиксирован на
  `client_secret_post`; Google предпочитает discovered `client_secret_basic`,
  затем Post и применяет OIDC-default Basic при отсутствии поля. Basic-путь
  сначала form-urlencoded кодирует каждый credential component, затем соединяет
  их `:` и Base64-кодирует. Header и form credentials никогда не отправляются
  одновременно.
- **State привязан к browser transaction:** одного глобального state + PKCE
  недостаточно против login-CSRF/session swapping. Start после успешного
  сохранения state ставит отдельную `__Host-` cookie (`Secure`, `HttpOnly`,
  `SameSite=Lax`, `Path=/`, TTL 10 минут); callback до backend проверяет ровно
  одно exact provider/state binding и очищает принятую cookie при любом
  terminal outcome. Request с восемью распознанными transaction-cookie names
  получает 429 до создания state; это request-local browser bound, а не
  глобально-атомарная browser quota. Независимый hard admission cap — 10 000
  live state на Memory process или общий PostgreSQL deployment: capacity check
  + insert атомарны, full/contended возвращает 429 без state/PKCE/cookie.
  Start/callback принимаются только на authority из `API_PUBLIC_URL`; proxy
  обязан сохранить public `Host`.
- **Три разные границы:** после browser/authority проверки matching state
  потребляется атомарно и не восстанавливается при последующем upstream failure;
  provider egress идёт без DB locks; short finalizer атомарно фиксирует только
  local user/link/session или MFA outcome. Existing subject может завершиться
  после primary identity, отсутствующий subject даёт rollback-only
  `VerifiedEmailRequired`, после чего optional email egress использует тот же
  исходный deadline и вызывается финальный transaction. Весь callback не является
  одной транзакцией.
- **Один callback-network budget, один Google cache:** исходный 30-секундный
  deadline покрывает Google discovery/permit/DNS, token, userinfo и optional
  verified-email, но не finalizer. Google cache локален runtime: TTL 1 час,
  failure cooldown 5 секунд; GitHub.com discovery не выполняет.
- **Identity authority:** existing `(provider, subject)` link авторитетен. Email
  collision без него возвращает 409 `AccountLinkRequired`, откатывает finalizer,
  не создаёт session и никогда не auto-link. Для MFA-enabled linked user
  finalizer атомарно сохраняет opaque challenge + MFA-required outcome; callback
  возвращает 202 без session/CSRF, а login завершает только `/auth/login/mfa`.
- **Verified-email outcome:** валидная provider identity без подходящего
  policy-verified email для first link возвращает 403 `EmailNotVerified` и не
  пишет link/session. Network/non-success response или malformed provider
  identity остаются fixed 502; эти semantic и upstream lanes не смешиваются.
- **Provider error:** bounded callback с ровно одним `error` проходит ту же
  browser/authority binding, атомарно отменяет state, очищает принятую cookie,
  не делает egress и возвращает fixed 401 без provider-controlled текста.
- **Google direct-TLS ID-token claims:** обязательный ID token проверяется по
  shape/RS256, pinned issuer, exact audience/`azp`, `exp`/`iat`, nonce, `at_hash`
  и equality subject с userinfo. Отложена только локальная криптографическая
  signature verification через JWKS: URL валидируется, но не fetch-ится, а
  signature bytes проверяются только синтаксически и по размеру.
- **Secret-free edge:** внутренние OAuth-ошибки имеют закрытые low-cardinality
  коды и наружу превращаются только в фиксированные RFC 9457 ответы. Request span
  пишет method + matched route template (или `<unmatched>`), никогда raw URI или
  query с callback `code`/`state`, и сохраняет inbound W3C parent.

## 6. Известные напряжения / долг

1. **Legacy resource-seam.** `domain/resource` сидит на сохранённом legacy
   `nebula_storage::repos::ResourceRepo` с legacy `StorageError`; адаптер-
   классификация в `error/mod.rs:171-183`, re-export мапперов в `lib.rs:113-115`
   — единственный путь мимо spec-16 портов. Подлежит замене при приземлении
   bind-population / topology.
2. **Двойной auth-контекст.** `middleware/auth.rs:4,106` вставляет «legacy»
   `AuthenticatedUser` в extensions рядом с `AuthContext`; re-exports для legacy-
   потребителей в `domain/auth/handler.rs:493`. Два представления одного факта.
3. **Plane-A OIDC frontier.** Google discovery issuer и ID-token claims уже
   валидируются на direct-TLS path. Не реализована только локальная
   криптографическая проверка ID-token signature по provider JWKS; Microsoft,
   generic OIDC и GitHub Enterprise Server также остаются parked. Plane-B
   browser ceremony не входит в этот scope (§7).
4. **OAuth browser quota.** Cap из восьми transaction cookies проверяется по
   Cookie header до создания state и жёстко ограничивает последовательные
   start-запросы. Одновременный burst может пройти несколько request-local
   проверок до того, как браузер применит ответы; это availability/cookie-
   eviction риск, не auth bypass. Абсолютная browser-wide quota потребует
   стабильного анонимного browser handle и server-side счётчика.
5. **Честные 501-заглушки остаются**: `restart_execution` (семантика за
   engine-командой), org-record и
   service-account endpoints (нет store / нет `Principal::ServiceAccount` пути).
6. **Legacy-совместимость в `state.rs`:** byte-identical Conflict-сообщение
   (`state.rs:613,662`), сортировка по RFC3339-строке или legacy i64
   (`state.rs:694,729`) — наследие старых хендлеров.
7. **`PgAuthBackend` владеет собственным `sqlx::Pool`** для трёх транзакционных
   флоу (`register_user`, `verify_email`, `complete_password_reset`) — осознанный
   обход pool-bound repo API; composition пока создаёт отдельный auth pool.

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
- **OAuth2 Plane-B.** Сырой provider-specific HTTP ceremony удалён и припаркован:
  `credentials/{id}/oauth2/{auth,callback}` не публикуется и отвечает 404.
  Единственный поддерживаемый acquisition-контракт — универсальные фасадные
  `resolve` / `resolve/continue`. `transport/oauth` теперь обслуживает только
  Plane-A identity login. До интеграции типизированного pending-flow дефолтные
  registry/catalog/dispatch не регистрируют и не рекламируют `oauth2`.
  Возврат браузерного Plane-B flow возможен лишь как
  типизированная pending-interaction универсального протокола, без второго store
  и без raw client-secret/query DTO.
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
- **Спроектировать Plane-B browser interaction поверх universal acquisition.**
  Сырой redirect/callback API остаётся припаркованным, пока credential runtime не
  выдаёт типизированную pending interaction, которая проходит через те же
  owner/admission/persistence-инварианты, что `resolve` / `resolve/continue`.
- **Закрыть двойной auth-контекст** (§6.2): один источник истины
  (`AuthContext`), удалить legacy `AuthenticatedUser` после миграции
  потребителей — структурный фикс, не дисциплина.
- **Durability-фронтир.** Credential CRUD уже идёт через файловый SQLite
  (`NEBULA_CRED_DB`) и переживает restart; только pending-acquisition пока
  process-local. Plane-A identity и `me/*` имеют selectable Memory/Postgres
  `AuthBackend`: PG сохраняет users/sessions/PATs/verification/OAuth state и
  external identity links и делится ими между replicas одной БД. Но default
  composition всё ещё не провиженит `MembershipStore`; org-membership требует
  явного совместимого wiring. Multi-replica credentials дополнительно требуют
  shared backend и durable pending-store.
- **Promote slug-webhook surface в OpenAPI** (1.0 follow-up): требует
  типизированных схем для request-envelope каждого провайдера.
- **Риск дрейфа доков:** держать `AGENTS.md` синхронным с фактической
  регистрацией роутов (`domain/*/routes.rs`, не `app.rs`).
