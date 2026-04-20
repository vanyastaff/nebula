---
name: nebula-credential architecture cleanup & cross-crate redistribution
status: proposal
date: 2026-04-20
authors: Claude (with tech-lead + security-lead review rounds)
scope: nebula-credential, nebula-storage, nebula-engine, nebula-api, nebula-core
supersedes: []
related:
  - docs/PRODUCT_CANON.md#35-integration-model
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#132-rotation-refresh-seam
  - docs/PRODUCT_CANON.md#14-anti-patterns
  - docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities
  - docs/STYLE.md#6-secret-handling
  - docs/MATURITY.md
  - docs/adr/0023-keyprovider-trait.md
  - docs/adr/0004-rename-credential-metadata-description.md
  - docs/adr/0021-crate-publication-policy.md
planned-adrs:
  - ADR-0028 — cross-crate credential invariants (umbrella)
  - ADR-0029 — nebula-storage owns credential persistence (supersedes ADR-0023 §KeyProvider location)
  - ADR-0030 — nebula-engine owns credential orchestration + token refresh
  - ADR-0031 — nebula-api owns OAuth flow HTTP ceremony
---

# nebula-credential — architecture cleanup design

## §0 — Context & motivation

`nebula-credential` сегодня — монолит ~34 top-level модулей, который смешивает
несколько ответственностей: credential contract (что такое Credential),
storage-работу (CredentialStore + layers), runtime orchestration (rotation
scheduler, resolver executor), и HTTP flow (OAuth2 reqwest client). Это
нарушает SRP и ломает симметрию с sibling-крейтами `nebula-action` (тонкий,
contract-only) и `nebula-resource` (держит orchestration внутри — тоже
отклонение, исправляется отдельным parallel spec'ом).

MATURITY row (`frontier / stable / stable / partial / n/a`) отражает это:
`Engine integration: partial (rotation in integration tests)` — потому что
rotation orchestration сейчас живёт в credential, а не в engine.

Crate также тянет лишние base deps: `reqwest` (для OAuth HTTP),
`nebula-metrics`+`nebula-telemetry` (observability), `moka`+`lru` (два кэша).

Это spec описывает:

1. Расселение ответственностей по 4 существующим крейтам (credential → credential/storage/engine/api) без создания новых крейтов.
2. Domain-first модульная группировка в target-крейтах (`credential/` submodule
   вместо `credential_store` / `credential_rotation`).
3. Четыре ADR-а (0028-0031) как нормативный фундамент.
4. Phased breakdown в ~11 PR'ах, последовательность landing'а.

## §1 — Goal & definition of done

**Цель.** Привести `nebula-credential` к архитектурной завершённости с чёткими
границами, SOLID/SRP гигиеной, и honest MATURITY с `Engine integration: stable`.

Crate становится **pure credential contract**: Credential trait, scheme DTOs,
built-in credential definitions (без HTTP), §12.5 primitives, secret wrappers,
contract-level DTOs.

**Определение «готово».** Observable success criteria:

1. `crates/credential/src/` — ≤ 15 top-level entries, сгруппированы по 6 submodule-темам (было ~34 plain модуля).
2. Zero duplicate modules — `retry.rs` + `rotation/retry.rs` → один (в resilience); `serde_secret.rs` + `option_serde_secret.rs` → один. `moka` collapses to single impl в storage после переезда `layer/cache.rs`. **`lru` retained in credential base deps** — `crates/credential/src/refresh.rs:51` использует `lru::LruCache` для bounded circuit-breaker tracking в `RefreshCoordinator`; sync `parking_lot::Mutex` guard, не подходит для moka (async-first) или HashMap (unbounded growth).
3. Zero direct deps на `nebula-metrics` / `nebula-telemetry` / `reqwest` в `nebula-credential/Cargo.toml`. Observability эмитится через `nebula-eventbus`.
4. `rotation/{scheduler,grace_period,blue_green,transaction}.rs` + token refresh HTTP живут в `crates/engine/src/credential/rotation/`.
5. `executor.rs` + `resolver.rs` + `registry.rs` merged в `crates/engine/src/credential/`.
6. `pending_store*`, `rotation/backup`, `layer/*`, `KeyProvider` — в `crates/storage/src/credential/`.
7. OAuth flow HTTP — `crates/api/src/credential/` (auth URI + callback + exchange) + `crates/engine/src/credential/rotation/token_refresh.rs` (refresh во время resolve).
8. `AuthPattern`, `AuthScheme`, `CredentialEvent`, `CredentialId` — мигрировали из `nebula-core` в `nebula-credential`.
9. ADR-0028..0031 — landed, accepted.
10. MATURITY row `credential`: `frontier / stable / stable / stable / n/a` (Engine integration улучшилось с `partial` до `stable`). API stability остаётся `frontier` — версии пока не увеличиваем (alpha, workspace path-deps).
11. MATURITY row `nebula-api` не регрессирует. OAuth flow — feature-gated под `credential-oauth` до completion'а `e2e_oauth2_flow` integration test (§13). До green'а E2E feature not включается в default, api MATURITY integration поле честно отражает что capability optional.

## §2 — Final shape of `nebula-credential`

```
crates/credential/src/
├── lib.rs                              # public re-exports only
├── contract/                           # Credential trait + assoc types
│   ├── mod.rs
│   ├── credential.rs                   # Credential trait
│   ├── any.rs                          # AnyCredential object-safe supertrait
│   ├── state.rs                        # CredentialState trait
│   ├── pending.rs                      # PendingState + NoPendingState + PendingToken
│   └── static_protocol.rs              # StaticProtocol pattern
├── metadata/                           # static + runtime descriptors
│   ├── mod.rs
│   ├── metadata.rs                     # CredentialMetadata + Builder
│   ├── record.rs                       # CredentialRecord
│   └── key.rs                          # CredentialKey newtype
├── secrets/                            # §12.5 primitives (L2 invariant)
│   ├── mod.rs
│   ├── crypto.rs                       # AES-256-GCM + Argon2id + PKCE primitives (functions)
│   ├── guard.rs                        # CredentialGuard (Deref + Zeroize)
│   ├── secret_string.rs                # SecretString
│   └── serde_secret.rs                 # serde helpers (option + non-option merged)
├── scheme/                             # 12 auth scheme DTO types (pruning = follow-up spec)
│   ├── mod.rs
│   └── {secret_token,identity_password,oauth2_token,key_pair,certificate,
│         signing_key,federated_assertion,challenge_secret,otp_seed,
│         connection_uri,instance_binding,shared_key,coercion}.rs
├── credentials/                        # built-in implementations (definitions only, no HTTP)
│   ├── mod.rs
│   ├── api_key.rs
│   ├── basic_auth.rs
│   ├── oauth2.rs                       # Credential trait impl + State (tokens) + projection
│   └── oauth2_config.rs                # OAuth2 URL/scope shape (data, not client)
├── accessor/                           # consumer interface
│   ├── mod.rs
│   ├── accessor.rs                     # CredentialAccessor trait + Noop/Scoped impls
│   ├── handle.rs                       # CredentialHandle
│   ├── context.rs                      # CredentialContext
│   └── access_error.rs                 # CredentialAccessError
├── rotation/                           # contract types only — orchestration в engine
│   ├── mod.rs
│   ├── policy.rs
│   ├── state.rs
│   ├── validation.rs
│   └── error.rs
├── refresh.rs                          # RefreshCoordinator (§13.2 seam primitive)
├── error.rs                            # CredentialError, CryptoError, RefreshErrorKind, etc.
├── resolve.rs                          # ResolveResult, InteractionRequest, DisplayData DTOs
├── snapshot.rs                         # CredentialSnapshot DTO
└── event.rs                            # CredentialEvent (moved from nebula-core)
```

**Итог:** 13 top-level entries (из ~34), организованных по 6 semantic submodule'ам.

**Cargo.toml после диеты:**

```toml
[dependencies]
# §12.5 primitives
aes-gcm = "0.10"
argon2 = "0.5"
base64 = { workspace = true }
sha2 = { workspace = true }
zeroize = { version = "1.8.2", features = ["zeroize_derive"] }
subtle = { workspace = true }
rand = { workspace = true }  # for PKCE challenge gen

# Sync-guarded bounded map for RefreshCoordinator circuit-breaker tracking
# (see crates/credential/src/refresh.rs:51). Retained after cleanup.
lru = { workspace = true }

# Workspace siblings
nebula-core = { path = "../core" }
nebula-credential-macros = { path = "macros" }
nebula-metadata = { path = "../metadata" }
nebula-schema = { path = "../schema" }
nebula-eventbus = { path = "../eventbus" }
nebula-resilience = { path = "../resilience" }

# Error taxonomy
nebula-error = { workspace = true }
thiserror = { workspace = true }

# Serde DTOs
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }

# Utilities
chrono = { workspace = true }
uuid = { workspace = true, features = ["v4", "serde"] }
tracing = { workspace = true }
humantime-serde = "1.1"

# Async + sync primitives
async-trait = { workspace = true }
# tokio features minimized; audit in P3 whether `time` / `macros` are load-bearing
# (sync needed for oneshot in RefreshCoordinator); fallback to ["sync"] if audit allows.
tokio = { workspace = true, features = ["time", "sync", "macros"] }
arc-swap = { workspace = true }
parking_lot = { workspace = true }
scopeguard = "1"

[features]
default = []
test-util = []

[lints]
workspace = true
```

**Удалены из base deps:** `reqwest`, `url`, `nebula-metrics`, `nebula-telemetry`, `moka`, `lru`, `tokio-util`.

## §3 — File-by-file migration plan

### Уходит из `nebula-credential`

| Откуда | → | Куда | Комментарий |
|---|---|---|---|
| `store.rs` (CredentialStore trait) | → | `nebula-storage/src/credential/store.rs` | trait в storage; credential reexport'ит для DX (permanent) |
| `store_memory.rs` | → | `nebula-storage/src/credential/memory.rs` | под feature `credential-in-memory` |
| `layer/encryption.rs` | → | `nebula-storage/src/credential/layer/encryption.rs` | §12.5 AAD binding preserved bit-for-bit; вызывает `nebula-credential::secrets::crypto` |
| `layer/key_provider.rs` | → | `nebula-storage/src/credential/key_provider.rs` | KeyProvider + Env/File/Static; **ADR-0023 superseded → ADR-0029** |
| `layer/cache.rs` | → | `nebula-storage/src/credential/layer/cache.rs` | collapsed to moka |
| `layer/audit.rs` | → | `nebula-storage/src/credential/layer/audit.rs` | AuditSink trait; default impl → `storage/repos/audit.rs` |
| `layer/scope.rs` | → | `nebula-storage/src/credential/layer/scope.rs` | |
| `pending_store.rs`, `pending_store_memory.rs` | → | `nebula-storage/src/credential/pending.rs` | Invariants (security-lead): (1) `code_verifier` encrypted at rest через `EncryptionLayer` wrap; (2) TTL ≤ 10 min; (3) single-use — transactional delete at callback consume; (4) request-session binding (correlates к OAuth flow session); (5) API возвращает `SecretString`, не raw `String`; (6) zeroize-on-drop на read |
| `rotation/backup.rs` | → | `nebula-storage/src/credential/backup.rs` | |
| `rotation/scheduler.rs`, `grace_period.rs`, `blue_green.rs`, `transaction.rs` | → | `nebula-engine/src/credential/rotation/` | orchestration рядом с `control_consumer.rs` |
| `rotation/retry.rs` | → | **delete** | replaced by `nebula-resilience`. **Gate before P1 delete:** verify `nebula-resilience` covers jittered exponential backoff на transactional flip failures (text из `rotation/retry.rs` semantics). Если gap — fix в resilience первым или keep local. |
| `rotation/metrics.rs`, `events.rs` | → | **delete** | eventbus emission + subscribers |
| `executor.rs`, `resolver.rs` | → | `nebula-engine/src/credential/resolver.rs` | merged с existing `credential_accessor.rs` + `engine/resolver.rs` |
| `registry.rs` | → | `nebula-engine/src/credential/registry.rs` | type-erased dispatch; симметрично с Y-модель |
| `credentials/oauth2_flow.rs` (split) | → | `nebula-api/src/credential/flow.rs` + `nebula-engine/src/credential/rotation/token_refresh.rs` | auth URI + callback + exchange → api; refresh during resolve → engine |
| `credentials/oauth2_flow.rs` PKCE primitives | → | stays в `nebula-credential/src/secrets/crypto.rs` | pure primitive, no HTTP |

### Остаётся в `nebula-credential`

Всё из §2 финального shape — contract + schemes + built-in credential definitions (без HTTP) + §12.5 primitives + DTOs + rotation contract types + RefreshCoordinator seam + event type.

### Типы, мигрирующие из `nebula-core`

- `AuthPattern`, `AuthScheme` — credential-specific trait + enum
- `CredentialEvent` — эмитируется credential, subscribers import из credential
- `CredentialId` — credential-specific newtype

**Все 4** переезжают из `crates/core/src/` в `crates/credential/src/`. `nebula-core` остаётся чист от credential-терминологии.

### Новые пути в target-крейтах

**`nebula-storage/src/credential/`:**
```
├── mod.rs
├── store.rs              # CredentialStore trait + StoredCredential + PutMode + StoreError
├── memory.rs             # InMemoryStore (feature: credential-in-memory)
├── key_provider.rs       # KeyProvider + Env/File/Static (ADR-0029)
├── pending.rs            # pending state repo (encrypted at rest)
├── backup.rs             # rotation backup repo
└── layer/
    ├── mod.rs
    ├── encryption.rs     # EncryptionLayer (§12.5 AAD preserved)
    ├── cache.rs          # CacheLayer (moka)
    ├── audit.rs          # AuditSink trait + default in-line sink
    └── scope.rs          # ScopeLayer
```

**`nebula-engine/src/credential/`:**
```
├── mod.rs
├── resolver.rs           # merged: existing credential_accessor + credential/executor + credential/resolver
├── registry.rs           # type-erased dispatch
└── rotation/
    ├── mod.rs
    ├── scheduler.rs
    ├── grace_period.rs
    ├── blue_green.rs
    ├── transaction.rs
    └── token_refresh.rs  # HTTP token refresh (reqwest) during resolve
```

**`nebula-api/src/credential/`:**
```
├── mod.rs
├── oauth_controller.rs   # GET /credentials/:id/oauth2/auth, GET/POST /credentials/:id/oauth2/callback
├── flow.rs               # auth URI construct + code exchange HTTP client (reqwest)
└── state.rs              # CSRF token + pending-state correlation
```

## §4 — ADR-0028: cross-crate credential invariants (umbrella)

**Роль.** Единый нормативный anchor для всех 4 переездов. ADR-0029/0030/0031
ссылаются на ADR-0028 как источник инвариантов.

**Что фиксирует:**

1. **§12.5 preservation (encryption at rest).** AES-256-GCM + Argon2id + AAD
   credential-id binding сохраняются bit-for-bit. Primitives
   (`encrypt`/`decrypt` функции, `EncryptionKey`, `EncryptedData`) — в
   `nebula-credential/src/secrets/crypto.rs`. Impl (`EncryptionLayer`) — в
   `nebula-storage/src/credential/layer/encryption.rs`. Layer вызывает
   primitives; AAD binding код не меняется.

2. **§13.2 seam integrity (non-stranding refresh).** `RefreshCoordinator`
   (thundering-herd prevention primitive) — stays в
   `nebula-credential/src/refresh.rs`. Orchestration (когда рефрешить,
   grace period windows, transactional state flip) — в
   `nebula-engine/src/credential/rotation/`. Seam определён в credential,
   invariant enforcer — engine.

3. **§3.5 stored-state vs auth-material split.** `Credential::project()` и
   `State`/`Pending` assoc types — stays в credential. Не двигаются.

4. **§14 no discard-and-log.** Audit — in-line durable (§5 этого spec'а).
   Fire-and-forget eventbus — только для metrics/observability, не заменяет
   audit write.

5. **§4.5 operational honesty.** MATURITY flip `Engine integration` в `stable`
   гейтится фактической реализацией. PR series не позволяет flip'ить row пока
   не landed все phases.

6. **Cross-crate compat invariants:**
   - `CredentialStore` trait переезжает в storage; credential re-exports для
     удобства consumers (permanent, не transitional — не хотим заставлять
     переписывать imports в каждом consumer каждые три месяца).
   - Re-exports **не должны** leak'нуть storage-internal types (impl details
     кэша, backend-specific hints). Credential re-exports **только trait +
     error + DTO shapes** — impl detail скрыт.
   - ADR-0023 `KeyProvider` public API переезжает в storage — ADR-0029
     supersedes ADR-0023 в части location.
   - `CredentialRecord`, `CredentialMetadata`, `CredentialKey`,
     `CredentialEvent` — остаются в credential (контракт), доступны из
     storage/engine/api через deps.

7. **Zeroize-on-drop at crate boundaries.** Любое plaintext secret
   пересекающее crate boundary **обязано** быть обёрнуто в
   `SecretString` / `Zeroizing<T>` / `CredentialGuard`. Нельзя:
   `credential → storage (raw String)` или `storage → engine (Vec<u8>
   plaintext)`. Tests (§13 redaction fuzz) ловят случаи когда нарушено —
   crate boundary без zeroize-контейнера = CI fail. Invariant apply'ится
   на всех 4 home'ах (credential, storage, engine, api).

8. **Versioning discipline (alpha).** Workspace path-deps, без SemVer publish
   в процессе миграции. Post-migration — решение по publishing отдельно.

## §5 — ADR-0029: nebula-storage owns credential persistence

**Supersedes:** ADR-0023 в части location `KeyProvider` / `EncryptionLayer`.

**Scope.** Все persistence-related типы credential уезжают из `nebula-credential`
в `nebula-storage/src/credential/`:

- `CredentialStore` trait (было `credential/src/store.rs`)
- `InMemoryStore` (было `credential/src/store_memory.rs`)
- `EncryptionLayer` (было `credential/src/layer/encryption.rs`)
- `KeyProvider` + `EnvKeyProvider` + `FileKeyProvider` + `StaticKeyProvider`
  (было `credential/src/layer/key_provider.rs`) — ADR-0023 public API moved
- `CacheLayer`, `AuditLayer`, `ScopeLayer` (было `credential/src/layer/`)
- `pending.rs` repo (было `credential/src/pending_store*.rs`)
- `backup.rs` repo (было `credential/src/rotation/backup.rs`)

**Canon adherence:**

- §12.5 AAD binding — preserved bit-for-bit (код переезжает без изменений).
- `nebula-credential::secrets::crypto` функции (encrypt/decrypt) импортируются
  `EncryptionLayer` — primitive layer остаётся в credential.
- Test coverage `crates/credential/src/layer/encryption.rs:257+`
  (round-trip + AAD + rotation + CAS + legacy alias) — переезжает в
  `crates/storage/src/credential/layer/encryption.rs` mod tests, all
  invariants pinned.

**Consumer impact.** Все callers `EncryptionLayer::new(store, key_provider)`
обновляют import path. Сам call site не меняется.

## §6 — ADR-0030: nebula-engine owns credential orchestration

**Scope.** Runtime orchestration переезжает из credential в engine:

- Rotation scheduler + grace period + blue/green + transactional state writes
  (`credential/src/rotation/*.rs` минус contract types) →
  `engine/src/credential/rotation/`.
- Resolver + executor + registry (`credential/src/{resolver,executor,registry}.rs`)
  → `engine/src/credential/{resolver,registry}.rs`. Merged с existing
  `engine/src/credential_accessor.rs` + `engine/src/resolver.rs`.
- Token refresh HTTP (было `credential/src/credentials/oauth2_flow.rs`
  fragment, «refresh» путь) → `engine/src/credential/rotation/token_refresh.rs`.

**Canon adherence:**

- §13.2 seam lives с trait: `RefreshCoordinator` stays в credential; engine
  *uses* its via trait surface. Orchestration может дергать `Credential::refresh()`
  под координацией.
- §12.5: engine никогда не материализует plaintext credential в hot loop
  напрямую; все reads через projected material из `CredentialStore`.

**`RefreshCoordinator` design note.** Coordinator остаётся **concrete
primitive** (не trait) в credential. Аналогия — `tokio::sync::Semaphore`:
thundering-herd prevention — property of the data structure, не pluggable
policy. Trait-ification пригласила бы broken impls. ADR-0030 явно декларирует
«no extension seam desired» для RefreshCoordinator. Если в будущем понадобится
alternative coordination strategy, open новый ADR.

**Token refresh logging.** `token_refresh.rs` **никогда** не логирует access
tokens / refresh tokens / bearer values — даже на tracing level DEBUG. Все
HTTP responses перед логированием проходят через redaction filter. Tracing
spans несут только metadata (duration, status code, credential_id), не body.
CI gate (§13 redaction fuzz) проверяет это per-call.

**`reqwest`** становится engine base dep. Engine теперь имеет outbound HTTP
capability, что consistent с ADR-0025 broker RPC-шаблоном (engine — host-side
для out-of-process верб).

## §7 — ADR-0031: nebula-api owns OAuth flow HTTP ceremony

**Scope.** User-facing OAuth HTTP ceremony переезжает в `nebula-api`:

- `GET /credentials/:id/oauth2/auth` — construct authorization URI с CSRF,
  state, PKCE challenge. Redirect user to IdP.
- `GET /credentials/:id/oauth2/callback` (+ POST для form_post response mode)
  — receive authorization code, validate CSRF/state, exchange code for
  tokens через HTTP POST to token endpoint, encrypt token state, store через
  `CredentialStore`.
- `api/src/credential/flow.rs` — HTTP client (reqwest) для token endpoint
  exchange.
- `api/src/credential/state.rs` — CSRF token generation + pending-state
  correlation (correlates request ↔ callback).

**Pattern parity с n8n:**

- n8n `packages/cli/src/controllers/oauth/oauth2-credential.controller.ts` →
  nebula `api/src/credential/oauth_controller.rs`.
- n8n `packages/cli/src/oauth/oauth.service.ts` → nebula
  `api/src/credential/flow.rs`.
- n8n `packages/core/.../request-helper-functions.ts refreshOrFetchToken` →
  nebula `engine/src/credential/rotation/token_refresh.rs`.

**Canon adherence:**

- §12.5: все credential state reads для token exchange проходят через
  `CredentialStore` (encrypted at rest). Plaintext token material только на
  in-memory request path, `Zeroizing<>` обёртки.
- §4.5: эндпоинты советуются в api MATURITY row только после landing этой
  фазы; до тех пор `api/src/credential/` feature-gated под `credential-oauth`.

### Security invariants (non-negotiable, enforced by CI)

1. **PKCE mandatory S256.** OAuth2 authorization URI generation **обязана**
   использовать PKCE с `code_challenge_method=S256`. `plain` не принимается.
   Без PKCE-challenge — fail-closed error, не fallback.

2. **CSRF token.** ≤ 10 minutes TTL. Single-use (transactional delete at
   callback consume). Comparison с request-provided token — constant-time
   (`subtle::ConstantTimeEq`). Mismatch или expired → `OAuthError::CsrfFailure`
   с 401, **не** leaks token value в error message.

3. **State parameter crypto-bound.** Не plain random hex; HMAC'инится над
   `{csrf_token || credential_id || expires_at}` с server secret. Callback
   recomputes и verify'ит HMAC constant-time до consume CSRF record.

4. **reqwest client shape.** TLS only (`rustls`); redirect policy capped at 5;
   timeout per-call ≤ 30s; response body cap 1 MiB (token endpoints никогда
   не возвращают больше). Partial / truncated responses → fail-closed с
   zeroization всех buffers.

5. **Token endpoint URL allowlist.** Workflow-config bindings декларируют
   `allowed_token_endpoints: Vec<Url>` per credential. Runtime verify'ит
   что IdP URL из credential config в allowlist. **Не** rely на DNS resolve
   — literal URL match. Защита против SSRF через misconfigured credentials.

6. **Zeroize on partial reqwest failure.** Timeout, connection reset,
   partial response → все plaintext buffers (request body, partial response
   body) wrapped в `Zeroizing<>` и scrubbed при panic / early return. CI
   test: mock reqwest failure на mid-response, assert что memory contains
   no token substring post-call.

**`reqwest`** становится api base dep (уже не в credential).

## §8 — Audit path: hybrid durable + eventbus metrics

Решение по §14 anti-pattern «discard-and-log workers». Audit — **in-line durable**,
eventbus — только для metrics fanout.

### Архитектура

```
┌──────────────────────────────────────────────────────┐
│ CredentialStore::put(credential) / get(id)           │
│                                                      │
│ 1. AuditLayer.wrap(inner).put(..)                    │
│    ├── inner.put(credential)             [store]     │
│    └── audit_sink.write(AuditEvent)    [in-line]    │
│         └── storage/repos/audit.rs        [durable] │
│                                                      │
│ 2. credential эмитит CredentialEvent                 │
│    └── nebula-eventbus                [fire-forget]  │
│         ├── subscriber: engine metrics               │
│         └── subscriber: observability exporters      │
└──────────────────────────────────────────────────────┘
```

### Гарантии

- **In-line durable.** `AuditEvent` записан в `storage/repos/audit.rs` **до**
  возврата ответа `put/get`. Если write в audit repo failed — вся операция
  failed (fail-closed). §14 avoided.
- **Eventbus fanout.** Параллельно credential эмитит типизированный
  `CredentialEvent` (resolved, rotated, refresh_failed) в `nebula-eventbus`.
  Fire-and-forget по ADR-0025 §4. Только для metrics/dashboards/alerts, **не
  несёт security-critical данных**.
- **Redaction.** `AuditEvent` содержит только safe fields — credential_id,
  operation kind, outcome, timestamp. Никогда plaintext value, никогда
  ciphertext, никогда key_id хэшей.

### Компоненты

- `AuditSink` trait — `nebula-storage/src/credential/layer/audit.rs`. Default
  impl пишет в `nebula-storage/src/repos/audit.rs` (уже существует).
- `AuditLayer` — middleware перед store write. In-line semantics.
- `CredentialEvent` — в `nebula-credential/src/event.rs` (после миграции из
  `nebula-core`). Эмитится через `nebula-eventbus`.
- `AuditEvent` — redaction-safe struct, в storage. Имеет only `verb`,
  `credential_id`, `outcome`, `started_at`, `latency_ms`.
- **`AuditEvent` Debug/Display — guarded**. Не авто-derive'ится; вручную
  пишется `impl Debug` / `impl Display` которые fmt'ят только whitelisted
  fields. Guard against future field addition silently leaking secrets
  через `{:?}` logging.

### CI gates

1. **Audit durability.** `storage/tests/credential_audit_durable.rs`: mock
   AuditSink fails → assert `put()` returns `StoreError` (не silent success).
2. **Eventbus fire-forget.** `storage/tests/credential_eventbus_fanout.rs`:
   subscriber crash → credential op продолжает без блокировки.
3. **Redaction fuzz.** Расширенный existing `tests/redaction.rs` (STYLE.md
   §6.4 helper): по одному case per credential operation (put/get/rotate/
   refresh/resolve/oauth_exchange). Fuzz инжектит secret-bearing input,
   грепит все outputs (audit rows + eventbus emission + tracing spans) на
   substring. Новая операция → обязательный row в тесте.

### Non-goals

- Background audit workers.
- Async-fire-and-forget для audit (только metrics).
- «Log when audit fails, continue anyway» (fail-closed enforced).

## §9 — Base-dep diet (detail)

**Removed из `nebula-credential/Cargo.toml`** (gone permanently):

- `reqwest` — HTTP уехал в api/engine.
- `url` — вместе с reqwest.
- `nebula-metrics`, `nebula-telemetry` — observability через eventbus.
- `tokio-util` — не используется после cleanup (confirmed grep).

**Retained в `nebula-credential/Cargo.toml`** (load-bearing, cite reason):

- `lru` — используется `crates/credential/src/refresh.rs:51` как
  `parking_lot::Mutex<LruCache<String, Arc<CircuitBreaker>>>` для bounded
  circuit-breaker tracking в `RefreshCoordinator`. Sync primitive; moka
  (async-first) и HashMap (unbounded growth) — не подходят. Stays.
- `tokio` features `["time", "sync", "macros"]` — audit в P3, возможно
  trim к `["sync"]` если `time` и `macros` только для test code.

**Relocated** (не удалены, переехали вместе с кодом):

- `moka` — переезжает из credential в `nebula-storage` вместе с
  `layer/cache.rs`. Scope «collapse» ограничен `layer/cache.rs`; в credential
  кэш отсутствует.

**Добавлено в engine/Cargo.toml (P9):** `reqwest` (для token refresh).

**Добавлено в api/Cargo.toml (P10):** `reqwest` (для token exchange), `url`.

**Workspace root:** `reqwest` уже в `[workspace.dependencies]`, shared.

## §10 — `nebula-core` re-exports migration

**Проблема.** Сегодня `nebula-credential::lib.rs` re-exports:

```rust
pub use nebula_core::{AuthPattern, AuthScheme, CredentialEvent, CredentialId};
```

Все 4 — credential-specific. `nebula-core` (frontier MATURITY) утечка frontier-
инстабильности в credential public surface.

**Решение.** Миграция в credential:

| Тип | Сейчас | После |
|---|---|---|
| `CredentialId` | `nebula-core::CredentialId` | `nebula-credential::CredentialId` (`src/metadata/key.rs` или top-level re-export) |
| `CredentialEvent` | `nebula-core::CredentialEvent` | `nebula-credential::event::CredentialEvent` |
| `AuthPattern` | `nebula-core::AuthPattern` | `nebula-credential::scheme::AuthPattern` |
| `AuthScheme` | `nebula-core::AuthScheme` | `nebula-credential::scheme::AuthScheme` |

**Breaking impact.** Minimal — все consumers уже импортируют через
`nebula-credential::…` (re-export), так что полные пути не меняются. Старый
`nebula-core` path ломается — consumers обновляют import. `nebula-core`
после миграции свободен от credential-терминологии.

**No deprecated shim.** Alpha stage + breaking changes разрешены (per
CLAUDE.md «bold refactors are allowed»). Не добавляем `#[deprecated]`
re-exports в `nebula-core` как transition softener — это ненужная ceremony
для single-line import update в consumers. Clean break в P4.

**Альтернативы отвергнуты:**
- Mirror в credential + оставить в core → создаёт два источника правды.
- Pin subset `nebula-core` → заморозить части frontier-крейта не имея ADR
  для stability commitment.

## §11 — Consumer migration order (leaf-first)

Обновляем consumers в порядке:

1. **`nebula-action`** (~30 мин edits). Updates imports для `CredentialId`,
   `CredentialGuard`, `CredentialAccessor`. Meta-derive refs обновляются через
   `#[derive(Credential)]` macro, который сам знает новые пути.
2. **`nebula-plugin`** (~30 мин). Updates trait bounds, plugin manifest
   credential slots.
3. **`nebula-sandbox`** (~30 мин для credential migration). ADR-0025 slice
   1d broker RPC verbs credential — отдельный spec, out of scope.
4. **`nebula-engine`** (~2-3 часа). Принимает массивные migrations (rotation,
   resolver, registry merges). Новый `engine/src/credential/` module.
5. **`nebula-runtime`** (~30 мин). Обновить credential_accessor если trait
   сдвигается.
6. **`nebula-sdk`** (~30 мин + doc update). Публичные re-exports: обновить
   под новую структуру.

Per-consumer PR reviewable separately.

## §12 — Phases & sequencing (PR breakdown)

Каждая фаза = 1 PR (или 2 tight-related). Итого ~11 PR'ов.

| # | Phase | Scope | Blockers |
|---|---|---|---|
| **P1** | Duplicate collapse | `retry.rs` dup удалить, merge `serde_secret` + `option_serde_secret`, collapse moka+lru → moka | — |
| **P2** | Submodule grouping | Re-organize `credential/src/` per §2 (13 top-level entries, 6 submodule групп) | P1 |
| **P3** | Base-dep diet | Remove metrics/telemetry/lru direct deps; reqwest остаётся до P10 | P2 |
| **P4** | `nebula-core` → `nebula-credential` | Move `AuthPattern`/`AuthScheme`/`CredentialEvent`/`CredentialId` | P3 |
| **P5** | ADR-0028..0031 landing | Write + review + accept all 4 ADRs (ADR-only PR, no code) | P4 |
| **P6** | Storage — credential module | `storage/src/credential/` с store + memory + layer + key_provider — ADR-0029 | P5 |
| **P7** | Storage — pending/backup repos | `storage/src/credential/pending.rs`, `backup.rs` | P6 |
| **P8** | Engine — credential module | `engine/src/credential/{resolver,registry,rotation/}` — ADR-0030; merge c existing | P6 |
| **P9** | Engine — token refresh | `rotation/token_refresh.rs` — reqwest client для workflow execution | P8 |
| **P10** | API — OAuth controller | `api/src/credential/` — auth URI + callback + exchange — ADR-0031 | P8, P9 |
| **P11** | Consumer migrations + MATURITY flip | action → plugin → sandbox → runtime → sdk; MATURITY row update; CHANGELOG entries | P10 |

**Critical path:** P5 (ADR landing) — **hard go/no-go checkpoint** перед
любым code move. На P5 вся цепочка ADR 0028..0031 принимается (или
отклоняется); если хоть один блокер — stop, revisit design, не запускать
P6+.

**Parallelism — narrow, не широкая:**
- `P7 ∥ P8` — pending/backup repos (leaf, storage-self-contained) и engine
  credential module (независимо от repos) можно вести одновременно.
- `P11` consumer PRs между собой — action/plugin/sandbox/runtime/sdk
  обновляются параллельно.
- **Остальное sequential.** P8 depends on P6 (engine needs storage trait),
  P9 depends on P8 (token_refresh lives inside engine/credential/rotation/),
  P10 depends on P6 + P8 (api uses storage trait + engine refresh seam).

**P11** закрывает — + sdk re-export audit (explicit sub-task).

Без version bumps — workspace path-deps в alpha.

## §13 — Testing strategy & CI gates

### Существующий тест-охват (сохранить, адаптировать под новые пути)

- `crates/credential/tests/redaction.rs` — расширяется: по одной case на
  операцию (put/get/rotate/refresh/resolve/oauth_callback_exchange). Fuzz
  инжектит secret-bearing input, грепит все outputs на substring.
- `crates/credential/tests/env_provider.rs` — переезжает в storage (KeyProvider
  там).
- `crates/credential/tests/units/encryption_tests.rs` — переезжает в storage **колокейтно с `EncryptionLayer`** под именем `crates/storage/tests/credential_encryption_invariants.rs`. Один файл пинит AAD round-trip + rotation + CAS + legacy alias + §12.5 AAD binding invariants. **Не scatter'им §12.5 тесты через крейты** — invariants fragment и drift; single source of truth.
- `crates/credential/tests/units/{resolve_snapshot,thundering_herd}_tests.rs`
  — переезжают в engine.
- `crates/credential/tests/units/pending_lifecycle_tests.rs` — переезжает в
  storage.
- `crates/credential/tests/units/{scheme_roundtrip,validation,error}_tests.rs`
  — остаются в credential (contract tests).

### Новые CI gates

1. **Audit durability** (§8): `storage/tests/credential_audit_durable.rs`.
2. **Eventbus fire-forget** (§8): `storage/tests/credential_eventbus_fanout.rs`.
3. **Redaction fuzz** (§8): new verb/op ⇒ mandatory row в
   `tests/redaction.rs`. Helper остаётся colocated с credential в
   `crates/credential/tests/redaction.rs`, но accessible from storage/engine/
   api tests через `[dev-dependencies] nebula-credential = { path = "..." }`.
   Единый fuzz grep pass на all outputs in cross-crate e2e test.
4. **Layer direction** (deny.toml): credential → not depend on storage / engine / api.
   storage → may depend on credential. engine/api → may depend on credential + storage.
   Deny.toml updates land **с** P6/P8/P10 PRs, не после — policy-layer audit
   blocked until deny.toml reflects new edge.
5. **Feature matrix** (P10 landing onwards): CI requires `--all-features`
   and `--no-default-features` matrix legs для nebula-api. Без этого
   `credential-oauth` feature silently bitrots между releases.
6. **MSRV 1.95 pin** сохраняется — все new edits держатся в рамках;
   `cargo check` с MSRV — required job.
7. **P2 pre-close grep** — перед закрытием P2, grep for accidental `Copy`
   derives на `CredentialGuard` / `SecretString` через implicit deref.
   Implicit copy секрета через Deref = zeroize bypass.
8. **Derive macros (`credential/macros/`) path audit** — в P2 sub-task,
   проверить что generated code (Credential / AuthScheme derive) не держит
   hardcoded `::nebula_credential::pending::NoPendingState`-style refs
   которые ломаются под новую submodule раскладку.

### Integration test

`crates/api/tests/e2e_oauth2_flow.rs` — end-to-end full cycle:

1. Register OAuth2 credential (type + config).
2. `GET /credentials/:id/oauth2/auth` → redirect response содержит authorize URL.
3. Mock IdP callback с code.
4. `POST /credentials/:id/oauth2/callback` → code exchange → stored encrypted.
5. `CredentialResolver::resolve()` in engine → projected material.
6. Mock refresh trigger → engine `token_refresh.rs` → stored updated.
7. Rotate (manual trigger) → engine rotation orchestration → transactional flip.
8. Revoke → store delete.

Покрывает все 4 крейта end-to-end.

## §14 — Risks & out-of-scope & follow-ups

### Risks

1. **ADR-0023 supersede confusion.** ADR-0029 supersede'ит части ADR-0023.
   Требует frontmatter cross-ref обновление в ADR-0023 при landing ADR-0029.
2. **Engine hot-path interaction.** P8-P9 трогают engine resolver. Возможная
   интерференция с будущим ADR-0025 slice 1d.0 (dispatch refactor, sandbox
   out of current scope). Coordinate с sandbox roadmap.
3. **OAuth2 API surface новое для nebula-api.** P10 — впервые api получает
   outbound HTTP (reqwest), CSRF обработку, callback state management.
   Требует security-lead review. Security invariants §7 (PKCE S256, CSRF
   TTL, state HMAC, URL allowlist) — hard enforcement в ADR-0031.
4. **Resource asymmetry.** До `resource-followup-spec` resource остаётся
   asymmetric с credential/action shape. Canon despondence видимо, не блокер.
5. **Audit durability gate** — fail-closed на audit write может возиливать
   availability. Operators нуждаются в runbook: что делать когда
   `storage/repos/audit.rs` недоступен. Открытый follow-up doc task.
6. **deny.toml synchronization.** При переезде deps между крейтами (reqwest
   credential→engine/api; credential dep в storage) `deny.toml` layer-
   direction rules должны обновиться **в той же PR**, не в follow-up.
   Иначе policy audit пропустит новое edge. Land with P6/P8/P10.
7. **Feature matrix bitrot.** `credential-oauth` feature в nebula-api
   (§7 feature-gate) silently bitrot'ится если CI не гоняет
   `--no-default-features` и `--all-features` matrix legs. Required-job
   update с landing P10.
8. **nebula-sdk re-export surface churn.** P11 listing sdk ~30min edits,
   но sdk — external integration-author facade. Любой path shuffle = doc
   breakage. P11 **must include explicit sub-task**: `sdk re-export audit`
   + migration note в sdk CHANGELOG. Dx-tester review перед P11 close
   (см. §15).
9. **Long red branch.** 11 PR × 2-3 day review = 6-8 недель с одним
   reviewer. P1-P5 self-contained — landing за неделю, даже если P6+
   буксует, крейт уже улучшен. Mitigation: P5 как hard go/no-go checkpoint,
   не продолжать до acceptance всех 4 ADRs. Если P6+ стаётся — остановиться
   с clean intermediate state.

### Out of scope этого spec'а

- ❌ `nebula-resource` rework (отдельный spec, symmetric shape — follow-up).
- ❌ `scheme/` pruning (`InstanceBinding`, `FederatedAssertion`, `OtpSeed`) —
  follow-up spec после landing, когда станет ясно что unused.
- ❌ KMS/Vault KeyProvider impls (ADR-0029 только trait + existing 3 impls;
  future follow-up ADRs).
- ❌ Full OAuth2 Authorization Server (client + callback receiver только).
- ❌ `credentials.get_value` plaintext fetch escape hatch — ADR-0025 §2
  follow-up.
- ❌ Sandbox broker RPC credentials verb — отдельный spec (ADR-0025 slice 1d.3).
- ❌ Version bumps / SemVer commitment — alpha, workspace path-deps.

### Follow-ups после spec'а

- **resource-followup-spec.md** — mirror pattern для `nebula-resource`
  (Manager/Registry/ReleaseQueue/recovery/runtime → engine). Symmetric с
  credential shape.
- **ADR-0032** — KmsKeyProvider (AWS KMS / GCP KMS).
- **ADR-0033** — VaultKeyProvider (HashiCorp Vault Transit).
- **scheme-pruning-audit.md** — через ~3 месяца после landing, проверить
  usage 12 scheme types.
- **nebula-api OAuth2 enhancements** — refresh token revocation endpoint,
  PKCE mandatory enforcement, OIDC support.
- **Audit runbook** — что делать когда audit repo недоступна; graceful
  degradation vs hard fail-closed trade-offs.

## §15 — Handoffs before implementation

- **security-lead** — explicit nod на §12.5/§13.2 seam boundaries при
  ADR-0028/0029 review. Особенно: pending store с `code_verifier` в
  storage крейте (encrypted at rest через EncryptionLayer wrap + TTL ≤10min
  + single-use + session binding — подтвердить что это достаточно); OAuth
  security invariants §7 (PKCE S256, CSRF HMAC, URL allowlist) — final nod.
- **tech-lead** — final priority call на P-sequencing; confirm P5 как
  hard checkpoint, narrow parallelism P7∥P8 only.
- **rust-senior** — `moka` в `layer/cache.rs` access pattern confirmed
  (async-first correct); `lru` retention в credential base deps confirmed
  (`refresh.rs:51` sync primitive). P2 pre-close audit derive macros
  generated paths + CredentialGuard Copy grep.
- **devops** — deny.toml layer-direction updates в P6/P8/P10 same-PR
  landing; `--all-features` + `--no-default-features` CI matrix update
  at P10.
- **dx-tester** — перед P11 close: sdk re-export surface audit. Новичок-
  integration-author должен суметь с новой sdk `use` линии собрать рабочий
  credential-using action без copy-paste из internal docs. Тест pre-merge.

---

**End of design spec.**
