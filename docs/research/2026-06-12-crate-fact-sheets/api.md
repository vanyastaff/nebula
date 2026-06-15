# nebula-api — fact sheet

## Назначение
HTTP-входная точка Nebula (axum, API Gateway): REST → типизированные вызовы port-трейтов из `AppState`; бизнес-логика делегируется вниз, SQL/схемы хранилища в крейте отсутствуют (API purity, crates/api/src/lib.rs:1-6). Плюс inbound-транспорты: converged webhook и OAuth2 (Plane-A sign-in и Plane-B integration-flow). Чистая библиотека — без `main`; composition root = `apps/server` (crates/api/AGENTS.md:35).

## Публичная поверхность
- `build_app(state, config) -> Router` — сборка OpenApiRouter + middleware-стек (crates/api/src/app.rs:30); `serve` (app.rs:357), `serve_with_shutdown` (app.rs:392)
- `AppState` — держатель портов (crates/api/src/state.rs:211, файл 1344 строк)
- Port-трейты API-уровня: `OrgResolver` (state.rs:36), `WorkspaceResolver` (state.rs:43), `MembershipStore` c org-lockout инвариантом `add/remove_member_guarded` (state.rs:115, исходы state.rs:75/89)
- `ApiError` `#[non_exhaustive]` (crates/api/src/error/mod.rs:35), `ApiResult` (error/mod.rs:513), `ProblemDetails` RFC 9457 (error/problem.rs:15)
- `ApiConfig` (config/mod.rs:67), `JwtSecret` (без Default — fail-hard), суб-конфиги Tls/Cookie/Cors/Versioning/Idempotency/Auth/Webhook/Smtp/Pagination (config/sub.rs)
- `OAuthProvidersConfig`/`OAuthProviderConfig` (config/oauth.rs:56/97)
- `domain::create_routes` (domain/mod.rs:62) — сборка всех роутов; tenant-вложение `/api/v1/orgs/{org}/workspaces/{ws}/…`
- Credential service-layer: `create/get/update/delete/list/test/refresh/revoke/resolve/continue_resolve_credential` (transport/credential.rs:324-703) — всё через `CredentialService` фасад (ADR-0088 D7)
- `try_default_credential_service` — composition-root фабрика фасада, SQLite-бэкенд по `NEBULA_CRED_DB` (ports/credential_service_factory.rs:1-21); `try_default_registry_port` (ports/credential_schema_registry.rs:220)
- Webhook: `WebhookTransport` (transport/webhook/transport.rs:99), `WebhookKey` (key.rs:59), `WebhookRateLimiter` (ratelimit.rs:58), `bootstrap_webhook_activations` (bootstrap.rs:170), `TriggerLifecycleSubscriber`/`Bus` (events.rs:110/354)
- OAuth-транспорт: `OAuthStateSigner`/`SignedOAuthState` (transport/oauth/state.rs:68/28), `fetch_userinfo` (userinfo.rs:123)
- PAT-scopes: `parse_pat_grant`/`validate_new_pat_scopes` (access/scope.rs:91/113)
- `EmailPort`/`EmailMessage`/`EchoSink` (ports/email.rs:111/65/126)
- Телеметрия: `init_api_telemetry`/`TelemetryGuard` (telemetry_init.rs:205/101)
- Re-exports lib.rs:111-119: `build_app`, `ApiConfig`, `ApiError`, `AppState`, `CursorParams`/`PaginatedResponse`, `map_resource_create/update_storage_error`

## Workspace-зависимости
Deps (Cargo.toml:15-42): nebula-env, nebula-error(derive), nebula-storage(credential-in-memory,sqlite), nebula-storage-port, nebula-tenancy (scoping-декораторы = граница мультитенантности, allowlist в deny.toml), nebula-core, nebula-execution, nebula-validator, nebula-metrics, nebula-workflow, nebula-action, nebula-engine, nebula-plugin, nebula-resilience, nebula-credential, nebula-schema(schemars; только для CredentialSchemaPort, DTO остаются `serde_json::Value`), nebula-eventbus. Внешние ключевые: axum, tower(-http), utoipa(+axum+swagger-ui), governor, jsonwebtoken, argon2, totp-rs, secrecy, moka/dashmap, opentelemetry*. Опционально: sqlx (feature `postgres` → PgAuthBackend + PgIdempotencyStore).
Зависят от nebula-api: apps/server (apps/server/Cargo.toml:15), examples (examples/Cargo.toml:24). Никакой crates/* не зависит.
Features: `test-util` (ApiConfig::for_test, обходит JwtSecret-гейт — не для prod), `postgres`; custom cfg `nebula_test_util` (НЕ feature) + compile_error-гард на release (lib.rs:93-109).

## Структура модулей
- `app.rs` — build_app: merge OpenApiRouter, split_for_parts, middleware-стек (порядок load-bearing: auth до csrf), serve
- `state.rs` — AppState + API-tier port-трейты + scoped-обёртки nebula-tenancy
- `config/` — ApiConfig, env-парсинг, JwtSecret, OAuth-провайдеры, суб-конфиги
- `domain/` — по-доменные `{routes,handler,dto}`: auth (Plane A; backend: in_memory/pg/password/pat/mfa/session/oauth/provider), me, org(+membership), workflow, execution, credential(+oauth, schema_projection), catalog, health, resource; `shared` (курсорная пагинация, AckResponse); `workspace`/`internal`/`metrics` — сборочный роутинг
- `error/` — ApiError, ProblemDetails, `classify` (seam legacy StorageError→ApiError)
- `extractors/` — JSON-extractor, валидаторы credential-входа
- `middleware/` — auth(JWT+API-key→AuthContext), tenancy, rbac, csrf, idempotency(key/layer/memory/store), internal_auth, rate_limit, request_id, security_headers, trace_w3c
- `openapi/` — OpenApiDoc (ADR-0047, drift = compile error) + audit.md (таблица статусов хендлеров)
- `ports/` — API-owned порты: credential_builder/schema/schema_registry/service_factory, email, reqwest_transport
- `transport/` — `credential` (service-layer через фасад), `oauth` (discovery/flow/http/state/userinfo), `webhook` (единый inbound: dispatch/key/provider/ratelimit/replay/routing/signature/bootstrap/events)
- `telemetry_init.rs` / `trace_capture.rs` (priv) / `test_support.rs` (cfg nebula_test_util)
- 28 интеграционных тестов в crates/api/tests/ (knife.rs, openapi_spec.rs, credential_e2e.rs, org_e2e.rs …)

## Напряжения
- **Legacy resource-seam**: domain/resource сидит на сохранённом legacy `nebula_storage::repos::ResourceRepo` с legacy `StorageError`; адаптер-классификация в error/mod.rs:171-183, re-export мапперов в lib.rs:113-115 — единственный путь мимо spec-16 портов
- **Двойной auth-контекст**: middleware/auth.rs:4,106 — «legacy» `AuthenticatedUser` вставляется в extensions рядом с `AuthContext`; re-exports для legacy-потребителей в domain/auth/handler.rs:493
- **Deprecated alias**: transport/oauth/flow.rs:198-203 — `#[deprecated]` алиас на `validate_oauth_outbound_url`
- **Честные 501-заглушки остаются** (openapi/audit.md:57,100): `list_my_orgs` (нет principal→orgs enumeration в MembershipStore/storage), `restart_execution` (семантика за engine-командой)
- **Legacy-совместимость в state.rs**: byte-identical Conflict-сообщение (state.rs:613,662), сортировка по RFC3339-строке или legacy i64 (state.rs:694,729) — наследие старых хендлеров
- README.md (823 строки) и AGENTS.md согласованы с кодом; противоречий не найдено; прим.: AGENTS.md:11 говорит «register route in src/app.rs», фактически роуты регистрируются в domain/*/routes.rs + domain/mod.rs:62 (app.rs только собирает)
- PgAuthBackend владеет собственным `sqlx::Pool` для 2 транзакционных флоу — осознанный обход nebula-storage (Cargo.toml:101-113)

## Роль в credential/resource redesign
**Credential — затронут сильно**: это production-потребитель `CredentialService`-фасада (ADR-0088 D7 wiring DONE): один persistence-путь через фасад, honest 503 при `credential_service: None` (transport/credential.rs:6-24); фабрика фасада для apps/server живёт здесь (ports/credential_service_factory.rs). Грядущее слияние runtime→credential и dyn-erasure (P4 плана facade-nongeneric) ударит по импортам transport/credential.rs и ports/credential_*; OAuth2-флоу Plane B (domain/credential/oauth.rs + transport/oauth) — кандидат на переезд в nebula-credential по rewrite-плану.
**Resource — затронут по краю**: domain/resource — CRUD/status API поверх legacy ResourceRepo; bind-population (M12.4) и topology-инверсия (ADR-0093) до api-слоя пока не дошли — при их приземлении legacy-seam (error/mod.rs:171) подлежит замене.
