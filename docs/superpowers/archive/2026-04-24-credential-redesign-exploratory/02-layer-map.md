# Layer responsibility map

**Статус:** working draft. Отражает current consensus + honest about open dependencies.

Этот файл — про **кто чем владеет** в credential-адрес architecture. Не про Shape какой-то конкретной trait.

## Визуальная карта

```
                    ┌─────────────────────────────────┐
                    │     nebula-api (HTTP gateway)   │
                    │  • OAuth callback controller    │
                    │  • Credential CRUD endpoints    │
                    │  • Provider registry admin API  │
                    │  • WebSocket /credentials/events│
                    │  • HMAC state (shared secret)   │
                    └──────────────┬──────────────────┘
                                   │ port traits
                                   ▼
                    ┌─────────────────────────────────┐
                    │    nebula-engine (orchestration)│
                    │  • CredentialResolver (hot path)│
                    │  • CredentialRegistry (TypeId)  │
                    │  • RefreshCoordinator impl      │
                    │    (two-tier: L1 proc, L2 store)│
                    │  • rotation/{scheduler,...}     │
                    │  • rotation/token_refresh       │
                    │  • oauth2/flow (HTTP ceremony)  │
                    │  • ExternalProvider impls       │
                    │  • ExecutionCredentialStore     │
                    │    (ephemeral/DYNAMIC creds)    │
                    │  • Credential health probe      │
                    └──────────────┬──────────────────┘
                                   │ traits
                                   ▼
                    ┌─────────────────────────────────┐
                    │   nebula-storage (persistence)  │
                    │  • CredentialStore impls        │
                    │    (memory/sqlite/postgres)     │
                    │  • EncryptionLayer              │
                    │  • CacheLayer (invalidation)    │
                    │  • AuditLayer (fail-closed +    │
                    │    degraded read-only mode)     │
                    │  • ScopeLayer (workflow_id)     │
                    │  • KeyProvider impls            │
                    │  • PendingStore (+GC sweep)     │
                    │  • RefreshClaimRepo (NEW)       │
                    │  • RotationLeaderClaimRepo (NEW)│
                    │  • ProviderRegistryRepo (NEW)   │
                    └──────────────┬──────────────────┘
                                   │ traits + DTOs
                                   ▼
                    ┌─────────────────────────────────┐
                    │ nebula-credential (contract)    │
                    │  • Credential trait             │
                    │  • CredentialStore trait + DTOs │
                    │  • SchemeInjector trait (NEW)   │
                    │  • Capability markers (NEW)     │
                    │  • ExternalProvider trait       │
                    │    (typed resolve + tenant ctx) │
                    │  • §12.5 crypto primitives      │
                    │  • Zeroize wrappers             │
                    │  • Field descriptors            │
                    │  NO HTTP, NO orchestration      │
                    └──────────────┬──────────────────┘
                                   │ traits + DTOs
                                   ▼
              ┌─────────────────────────────────────────┐
              │ nebula-credential-builtin (NEW crate)   │
              │  • SlackOAuth2Credential + family       │
              │  • GitHubOAuth2 + GitHubPat + ...       │
              │  • AwsSigV4 + AwsStsCredential          │
              │  • PostgresConnection + MySql + ...     │
              │  • MtlsClientCredential                 │
              │  • KafkaSaslPlain + SCRAM + ...         │
              │  • SalesforceJwtCredential (multi-step) │
              │  • DiscordWebhookCredential (no-secret) │
              │  • service traits (BitbucketCredential, │
              │    JiraCredential, ...)                 │
              └──────────────┬──────────────────────────┘
                             │
                             ▼ (used by)
                             
┌─────────────────────────┐  ┌──────────────────────────┐
│   nebula-resource       │  │   nebula-action          │
│  • Resource trait       │  │  • Action trait          │
│    type Auth:           │  │  • #[action(credential)] │
│      SchemeInjector     │  │    macro                 │
│  • AuthenticatedRequest │  │  • ActionContext         │
│    Builder (HTTP)       │  │  • ScopedCredentialAcces.│
│  • connection_bound     │  │  • ctx.credential::<C>() │
│    variant              │  │    + ctx.credential_at() │
└─────────────────────────┘  └──────────────────────────┘
```

## Supporting crates

| Крейт | Роль | Credential-специфика |
|---|---|---|
| **`nebula-core`** | Shared vocabulary | `AuthScheme` trait (classification), `AuthPattern` enum, `CredentialAccessor` trait (dyn-safe), `RefreshCoordinator` trait, `CredentialKey` newtype, `CredentialId` ULID, `Guard`/`TypedGuard`, `TenantContext`, `HasCredentials` capability |
| **`nebula-schema`** | Typed config + field validation | `SecretString`/`SecretBytes`/`SecretValue`/`SecretWire` — secret wrappers; `SecretField` — field type with redaction; `HasSchema` trait; `ValidSchema`/`ValidValues`/`ResolvedValues` proof pipeline |
| **`nebula-metadata`** | Static descriptors | `CredentialMetadata` struct (display name, icon, help text); versioning |
| **`nebula-error`** | Error taxonomy | `NebulaError`, `Classify` trait — credential errors classify as `Transient` (network), `Permanent` (revoked/config), `Capability` (wrong scheme requested — new axis, finding #13) |
| **`nebula-resilience`** | Retry/timeout/circuit-breaker | Used by engine's `token_refresh.rs` for IdP token endpoint resilience |
| **`nebula-eventbus`** | Typed broadcast | `CredentialEvent` fire-and-forget (telemetry, NOT compliance audit); `CacheInvalidation` typed channel (internal) |
| **`nebula-metrics`** | Metric constants + export | `CredentialMetrics` — bounded cardinality counters/histograms |
| **`nebula-log`** | Structured tracing | Redaction filters for credential spans |

## Interaction matrix (dep direction)

| From → To | core | schema | metadata | error | credential | credential-builtin | storage | engine | api | resource | action |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **core** | — | — | — | — | — | — | — | — | — | — | — |
| **schema** | ✓ | — | — | ✓ | — | — | — | — | — | — | — |
| **metadata** | ✓ | ✓ | — | ✓ | — | — | — | — | — | — | — |
| **error** | — | — | — | — | — | — | — | — | — | — | — |
| **credential** | ✓ | ✓ | ✓ | ✓ | — | — | — | — | — | — | — |
| **credential-builtin** | ✓ | ✓ | ✓ | ✓ | ✓ | — | — | — | — | — | — |
| **storage** | ✓ | ✓ | ✓ | ✓ | ✓ | — | — | — | — | — | — |
| **engine** | ✓ | ✓ | ✓ | ✓ | ✓ | ? | ✓ | — | — | — | — |
| **api** | ✓ | ✓ | ✓ | ✓ | ✓ | ? | ✓ | ✓ | — | — | — |
| **resource** | ✓ | ✓ | — | ✓ | ✓ | — | — | ✓ | — | — | — |
| **action** | ✓ | ✓ | — | ✓ | ✓ | — | — | ✓ | — | ✓ | — |

**?** Дискуссионные edges:
- `engine → credential-builtin` — нужен ли? Engine orchestrates via `Box<dyn AnyCredential>` + TypeId lookup. Concrete types populate registry **at engine startup** через callbacks. Engine не должен знать about `SlackOAuth2Credential` specifically. **Предложение:** нет dep. Registry populated via plugin registration.
- `api → credential-builtin` — нужен ли? Api вызывает `engine::oauth2::flow.start(provider_id)` — через engine. Api не trait-bound к concrete types. **Предложение:** нет dep.
- `resource → credential-builtin` — нужен ли? Resource binds `type Auth: AcceptsBearer` (trait from credential). Не нужен concrete type. **Предложение:** нет dep.

## deny.toml enforcement (finding #11 — discipline)

Добавить rules:

```toml
# credential crate — contract only, no heavy runtime deps
[[bans]]
name = "nebula-credential"
forbidden-dependencies = [
    "reqwest",       # HTTP → engine / api
    "hyper",
    "tokio-postgres",
    "sqlx",
    "moka",          # cache → storage
    "nebula-storage",
    "nebula-engine",
    "nebula-api",
    "nebula-resource",
    "nebula-action",
]

# credential-builtin — may depend on crypto libs but not runtime
[[bans]]
name = "nebula-credential-builtin"
forbidden-dependencies = [
    "nebula-storage",
    "nebula-engine",
    "nebula-api",
    "reqwest",  # still no HTTP
]

# credential не может быть direct dep'ом storage-internal crates
[[bans]]
name = "nebula-storage"
allowed-credential-access = "nebula-credential"  # traits only
```

## Что где живёт — ответы на частные вопросы

### Где OAuth HTTP ceremony (authorize URL + callback exchange + refresh)?

**Предложение:** вся в `nebula-engine`.
- `engine/src/credential/oauth2/flow.rs` — authorize URL build, token exchange, refresh
- `engine/src/credential/rotation/token_refresh.rs` — refresh specifically (may merge с flow.rs)
- API только routes HTTP + тонко вызывает engine

**Supersede ADR-0031:** argument "n8n parity" was weak. Single owner = cleaner layer boundaries. Engine уже has reqwest per ADR-0030.

**Trade-off:** engine's surface растёт. Acceptable — engine — outbound HTTP owner per ADR-0025 pattern.

### Где credential material zeroize invariants enforced?

- **Pre-decrypt:** storage reads encrypted bytes → `Zeroizing<Vec<u8>>`
- **Post-decrypt:** `SecretString`/`SecretBytes` (nebula-schema types) с ZeroizeOnDrop
- **Projection:** `CredentialGuard<Scheme>` — RAII wrapper implementing `TypedGuard`
- **Resource boundary:** default pattern — per-request injection, plaintext ~μs scope
- **Audit events:** AuditEvent по construction без plaintext — just credential_id + verb + outcome

### Где Pending state (PKCE, CSRF) живёт?

- **Table:** `nebula-storage` — `pending.rs` repo
- **Encrypted at rest:** YES через EncryptionLayer с same crypto pipeline
- **TTL:** ≤ 10 min
- **Single-use:** get_then_delete transactional
- **GC sweep:** periodic (1 min cadence) — delete WHERE expires_at < NOW() (finding #6 — сейчас не реализован)

### Multi-step flows (Salesforce JWT, session login) — где state живёт?

**Open:** сейчас `PendingStore` только для OAuth2 PKCE verifier + CSRF. Для generic N-step accumulator нужен другой shape. Options:

1. **Extended PendingStore** — добавить `accumulator: serde_json::Value` поле. Per-step read/update/replace. Обязательное encryption.
2. **Separate MultiStepStore** — дedicated repo для credentials с `Capabilities::MULTI_STEP`.
3. **In-memory state machine** — step N output passed через to step N+1 в `CredentialContext` без persistence, credential resolves atomic. Но тогда cancellation mid-flow leaves external IdP in partial state.

Предложение (не final): **Option 1** — extended PendingStore с typed accumulator (enum или JSON). Each credential type declares `Pending::State` shape which serializes into accumulator JSON. Persistence needed for crash-resume.

### Где credential rotation orchestration?

**Per ADR-0030:** `nebula-engine/src/credential/rotation/` — scheduler, grace_period, blue_green, transaction. Stays.

**Multi-replica coordination (finding #17 — open):** rotation scheduler leader election. Options:
1. Postgres advisory lock `rotation_leader:{tenant_id}` + heartbeat
2. Dedicated `RotationLeaderClaimRepo` (like RefreshClaimRepo)
3. External coordinator (etcd/Zookeeper) — rejected (Nebula local-first)

Предложение: **option 2** — storage-backed claim repo, similar pattern к RefreshClaim. Each rotation scheduler replica tries to claim `rotation_leader` with TTL 60s; heartbeat every 20s. Only leader runs scheduler. Reclaim sweep если leader died.

### External secret providers (Vault/AWS SM/GCP SM/Azure KV)

**Где impl'ы:** `nebula-storage/src/external_providers/` — parallel к `credential/` module.
- `vault.rs` — VaultProvider impl of `ExternalProvider`
- `aws_sm.rs`, `gcp_sm.rs`, `azure_kv.rs`
- Каждый implements `ExternalProvider` trait (defined в credential crate)

**Tenant scoping:** см. `01-type-system-draft.md` §7. Provider impl prepends tenant namespace, uses tenant-scoped token.

**SSRF allowlist (finding #7):** each provider declares `endpoint_allowlist(&self) -> &EndpointAllowlist` — literal URL match, per-provider config.

### Где "Provider Registry" (OAuth endpoints catalog)

**NEW infra — placement открытый:**
- Option A: `nebula-storage/src/credential/provider_registry.rs`
- Option B: `nebula-engine/src/credential/provider_registry.rs`
- Option C: otдельный crate `nebula-provider-registry` (rejected — too narrow для own crate)

Предложение: **A** — storage owns persistence (consistent с other registries), engine reads. Admin API в `nebula-api` для operator CRUD.

### Где credential health probe scheduler?

**`nebula-engine`** — параллельно с rotation scheduler. Periodic background task per credential (cadence from CredentialMetadata, default 1 hour), calls `Credential::test()`, emits `CredentialStatus` event.

### Где WebSocket /credentials/events endpoint?

**`nebula-api`** — consume `CredentialEvent` eventbus, push to authorized subscribers.

**Open (finding #34):**
- Authorization per-event (user's own only? tenant-wide?)
- Rate limiting
- Reconnect semantics (missed events)

## Edge cases без clear home

### Trigger ↔ credential (finding #35)

Triggers (webhooks, IMAP polling, queue consumers) имеют credentials. Lifecycle не параллельно с Actions:
- Actions — short-lived, credential resolved at start of each action
- Triggers — long-lived, credential needs to persist through trigger lifetime
- Webhook trigger — credential is для signature verification (HMAC secret для incoming webhooks)
- IMAP trigger — credential is connection (like DB)

**Open:** Trigger trait shape для credential integration. Вероятно similar к Resource с connection-bound variant. Separate spike.

### Encryption key rotation (finding #15)

**Existing mechanism:** `KeyProvider` fingerprint-based version. `with_legacy_keys` decrypt-old-encrypt-new lazy.

**Missing:** `nebula credential rotate-master-key --from=<fp> --to=<fp>` walker CLI. ~10 строк per table — credential_state, pending_state, backup. Iterate rows, decrypt с old, encrypt с new, update row.

**Home:** `nebula-cli` (new crate if doesn't exist, or merge into apps/cli).

## Summary — что новое, что existing

### Новая инфра (нужна implementation):

1. `nebula-credential::SchemeInjector` trait + capability markers
2. `nebula-credential-builtin` crate
3. `nebula-storage::RefreshClaimRepo`
4. `nebula-storage::RotationLeaderClaimRepo`
5. `nebula-storage::ProviderRegistryRepo`
6. `nebula-api::oauth_admin` — admin CRUD for providers
7. `nebula-api::credentials_websocket` — realtime events
8. `nebula-engine::oauth2::flow` — HTTP ceremony (moved from credential)
9. `nebula-engine::ExecutionCredentialStore` — ephemeral DYNAMIC creds
10. `nebula-engine::credential_health_probe` — periodic test scheduler

### Refactor existing:

1. `nebula-credential::ExternalProvider` — typed `resolve<S>` + tenant_ctx
2. `nebula-engine::RefreshCoordinator` impl — two-tier (L1 proc + L2 storage claim)
3. `nebula-storage::AuditLayer` — add degraded read-only mode
4. `nebula-storage::CacheLayer` — explicit invalidation channel
5. `nebula-credential::Credential::CAPS` — bitflags (replace 5 bools)
6. `nebula-resource::Resource` — `type Auth: SchemeInjector`, add `create_with_auth` variant

### Keep as-is:

- `nebula-core::AuthScheme`, `AuthPattern`, `CredentialAccessor`, `Guard`, `RefreshCoordinator` trait
- `nebula-schema::SecretString`, `SecretField`, `HasSchema`
- `nebula-credential::CredentialStore` trait + DTOs (per ADR-0032)
- `nebula-credential::Credential` trait 4 assoc types
- ADR-0028 invariants (§12.5 preservation, zeroize boundaries, audit fail-closed)
- ADR-0033 Plane A/B split
- AES-256-GCM + AAD binding bit-for-bit
