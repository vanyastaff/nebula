# nebula-credential — fact sheet

## Назначение
Типизированный Credential Contract: разделение хранимого `State` (шифруется at rest, ZeroizeOnDrop) и проецируемого auth-материала `Scheme`, который получает action-код. После ADR-0092 крейт также содержит ВЕСЬ runtime (resolver/refresh/lease/rotation-оркестрация, перенесено из nebula-engine) и `CredentialService` фасад (перенесён из удалённого nebula-credential-runtime). Крипто (AES-256-GCM/Argon2id) вынесено в nebula-crypto (ADR-0088). `#![forbid(unsafe_code)]`.

## Публичная поверхность (канон — плоские re-export'ы в lib.rs)
- `Credential` trait (Properties/State/Scheme, `resolve`/`project`) — src/contract/credential.rs:119
- Capability sub-traits: `Interactive` (contract/interactive.rs:75), `Refreshable` (contract/refreshable.rs:69), `Revocable` (contract/revocable.rs:48), `Testable` (contract/testable.rs:54), `Dynamic`; `Capabilities` bitflags + `compute_capabilities` (contract/capability_report.rs:71,155)
- `CredentialRegistry` (KEY-keyed, дубликат фатален) — src/contract/registry.rs:87
- `AuthScheme`/`SensitiveScheme`/`PublicScheme`/`AuthPattern` — НЕ здесь: re-export из `nebula_core::auth` (src/scheme/auth.rs:6); 9+ схем в src/scheme/ (SecretToken, OAuth2Token, KeyPair, ConnectionUri, …)
- `CredentialService` фасад (`resolve_for_slot`, `scheme_factory`) — src/service/facade.rs:160; `ValidatedCredentialBinding`+`TenantFingerprint` (service/binding.rs:21,32)
- `CredentialResolver<S>` (resolve_with_refresh, кэш handle'ов) — src/runtime/resolver.rs:33
- `RefreshCoordinator` (L1 coalescer + L2 claim store через nebula-storage-port) — src/runtime/refresh/coordinator.rs:229
- `ExternalProvider`/`ExternalProviderChain`/`LeasedProvider`/`ProviderResolution` (ADR-0051, Vault/AWS/…) — src/provider/mod.rs:173, provider/chain.rs:54
- Секреты: `SecretString` (secrets/secret_string.rs:25, обёртка secrecy), `CredentialGuard` (secrets/guard.rs:37), `SchemeGuard`/`SchemeFactory` §15.7 (secrets/scheme_guard.rs:71,158), PKCE-хелперы (secrets/crypto.rs)
- `CredentialStore` trait + `StoredCredential`/`PutMode`/`StoreError` (impl'ы живут в nebula-storage) — src/store.rs:169
- Dyn-erasure: `DynCredentialStore`/`ErasedCredentialStore`/`DynPendingStateStore` (ADR-0088 D4) — src/erased.rs:71,132,196
- Ошибки: `CredentialError` + per-variant context structs (Smithy RFC-0022, #588) — src/error.rs:472
- `CredentialMetadata`+builder (metadata.rs:57), `CredentialRecord` (record.rs:30), `CredentialContext` (context.rs:99), `CredentialRef<C>` (credential_ref.rs:54), `CredentialHandle` (handle.rs:31)
- `CredentialPolicy`/`RefreshStrategy`/`RevokeStrategy` — capabilities-as-data, ADR-0088 D2 — src/lifecycle.rs:130
- Builtin-кредентиалы: OAuth2/ApiKey/BasicAuth/BearerToken/SharedKey/SigningKey + `register_builtins` — src/credentials/
- Макросы: `#[credential]` (attribute, ADR-0088 D1) + legacy `#[derive(Credential)]` + `#[derive(AuthScheme)]` — re-export из nebula-credential-macros (lib.rs:177)
- Feature `rotation`: blue-green/transaction/grace-period — src/rotation/ + src/runtime/rotation/

## Workspace-зависимости
Deps: nebula-crypto, nebula-credential-macros, nebula-core, nebula-metadata, nebula-schema, nebula-storage-port, nebula-metrics, nebula-resilience, nebula-eventbus, nebula-error (+ tokio/secrecy/zeroize/subtle/arc-swap/lru/regex/ahash/bitflags/compact_str…).
Зависят от него: sdk, storage (+feature rotation), resource (×2 записи), tenancy, api, engine (+feature rotation), plugin, action (+ action → credential/macros напрямую).

## Структура модулей
- `contract/` — Credential trait, 5 capability sub-traits, registry, resolve-типы, AnyCredential, StaticProtocol
- `scheme/` — re-export AuthScheme-базы из core + 10 файлов схем + coercion/instance_binding
- `secrets/` — SecretString, guard'ы, SchemeGuard/SchemeFactory, serde_secret, PKCE
- `credentials/` — 6 builtin-кредентиалов + oauth2_config (builder'ы grant-флоу)
- `provider/` — ExternalProvider chain/lease/future (ADR-0051)
- `runtime/` — resolver, executor, dispatchers, scoped_accessor, lease/ (scheduler из engine, ADR-0092), refresh/ (coordinator, L1, sentinel, token_refresh, transport), rotation/ (gated)
- `service/` — CredentialService facade, DispatchOps, binding, head, observer, ops, state_source
- Корень: error, event, store, audit, pending_store, erased, lifecycle, metadata, record, context, handle, credential_ref, display, snapshot, no_credential, accessor, ext, metrics
- `macros/` — вложенный proc-macro крейт nebula-credential-macros

## Напряжения
- **README устарел против ADR-0092**: README.md:3,14,26-28 утверждает «runtime orchestration lives in nebula-engine::credential», но runtime живёт ЗДЕСЬ (src/runtime/, lib.rs:120-127 «relocated from nebula-engine»). lib.rs:34 сам содержит ту же устаревшую строку. docs/DESIGN.md:14 прямо признаёт этот дрейф.
- **README указывает на несуществующий файл**: seam `crates/credential/src/crypto.rs` (README.md:167,214) — файла нет, крипто в nebula-crypto; есть только secrets/crypto.rs (PKCE).
- **AGENTS.md:20 заявляет feature `test-util`** — в Cargo.toml [features] только `default`+`rotation` (test-util там — фича tokio в dev-deps, Cargo.toml:104).
- **Двойные refresh-пути (legacy String-id vs typed CredentialId)**: ~10 `#[deprecated]` legacy-методов L1 в runtime/refresh/coordinator.rs:911-991, resolver вызывает их под `#[allow(deprecated)]` (runtime/resolver.rs:197,273,419-424) «until П3 typed-id migration».
- **Deprecated shims**: `credentials::AuthStyle` re-export (credentials/mod.rs:34, oauth2.rs:24, oauth2_config.rs:23); back-compat alias `serde_secret` (lib.rs:237-240); back-compat re-export `resolve` (lib.rs:133-144); legacy `#[derive(Credential)]` рядом с новым `#[credential]` (lib.rs:174-177).
- **Дубль rotation-кода**: src/rotation/ (типы/policy/events, feature-gated) И src/runtime/rotation/ (blue_green/transaction/scheduler) — два модуля одного домена.
- **TestResult — два разных типа**: contract::resolve::TestResult (re-export lib.rs:154) vs rotation/error.rs:404; плюс rotation/error.rs:280-296 имеет собственные TestableCredential/RotatableCredential параллельно contract::Testable.
- `no_credential.rs:1,43` — «legacy no-auth marker», но re-export'ится как актуальный API (lib.rs:181).

## Роль в credential/resource redesign
Это ЦЕНТР redesign'а. docs/DESIGN.md (Draft, spec-first, «No Rust refactor ships until approved») фиксирует: ADR-0092 консолидация сделана физически, но логический rewrite ADR-0088 НЕ закончен — параллельные resolve/refresh пути, монолитный OAuth2-тип, docs-дрейф. План: единый CredentialRuntime-пайплайн, отделение management от runtime, Protocol+provider-config вместо типа-на-SaaS, OAuth Plane Law (ноль HTTP-роутов здесь). Свежие коммиты (hot-swap handles, SchemeFactory для пулов, dedupe rotation-gated imports) — активная работа. Связка с resource: `resolve_for_slot` = production bind-population seam (M12.4); rotation fan-out — на стороне resource/engine (non-goal здесь).
