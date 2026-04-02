# nebula-credential — HLD v1.6

> **Status:** Draft v1.6 — amended after open architecture review (public session).
> **v1.0:** 10-round adversarial design review.
> **v1.1:** Developer challenge (dx-tester, sdk-user).
> **v1.2:** Town hall #1 (plugin, engine, storage devs).
> **v1.3:** Town hall #2 (5 production use cases, tech-lead synthesis).
> **v1.4:** Conference #1 (healthcare, IoT, solo, fintech, skeptic).
> **v1.5:** Conference #2 (gaming/Keiko, Airflow-migration/Rafael, SOC2-auditor/Sonja, dev-tooling/Jordan, Vault-skeptic/LiWei). DecryptedCacheLayer, DatabaseAuth extensions, registry introspection, dyn-compat deferred v1.1.
> **v1.6:** Open architecture review (fintech multi-tenant/Priya, industrial IoT/Marcus, ML platform/Yuki, security vendor/Dmitri, embedded OSS/Lena). Threat model §1.1, StackBuilder typestate, scope containment matrix, AuditLayer failure policy, dual-material OQ, feature flags for core crate.
> **Crates:** `nebula-credential` (implemented), `nebula-credential-storage` (designed), `nebula-credential-macros` (designed)

---

## 0. Implementation Status [AMENDED v1.2]

This HLD describes both the **implemented system** and the **target design**. Items
marked **DESIGNED** are approved architecture not yet built. Do not attempt to import
DESIGNED items — they will produce compile errors.

### Naming: HLD vs Code

Where the HLD and code disagree, the **code is authoritative**:

| HLD Name | Code Name | Status |
|----------|-----------|--------|
| `CredentialBackend` | `CredentialStore` | Code name used everywhere |
| `InMemoryBackend` | `InMemoryStore` | Code name used everywhere |
| `Resolution<S,P>` | `ResolveResult<S,P>` | Code name used everywhere |
| `StoredCredential.id: CredentialId` | `StoredCredential.id: String` | Target: migrate to CredentialId |

### Core Types

| Item | Status | Notes |
|------|--------|-------|
| `Credential` trait | **IMPL** | 7 methods, 5 consts, 3 associated types |
| `AuthScheme` trait | **IMPL** | In `nebula-core::auth` |
| `CredentialStore` trait | **IMPL** | HLD calls it `CredentialBackend` |
| `CredentialRegistry` | **IMPL** | Keys on `state_kind` (target: `Credential::KEY`) |
| `CredentialSnapshot` | **IMPL** | `Box<dyn Any>`, Clone via fn pointer |
| `CredentialHandle<S>` | **IMPL** | `ArcSwap<S>` |
| `CredentialContext` | **IMPL** | Builder pattern, optional resolver |
| `CredentialDescription` | **IMPL** | Has `builder()` method |
| `RefreshCoordinator` | **IMPL** | Winner/Waiter + CircuitBreaker |
| `CredentialResolver` | **IMPL** | Resolve + refresh coordination |
| 5 built-in credentials | **IMPL** | ApiKey, BasicAuth, OAuth2, Header, Database |
| 13 AuthScheme types | **IMPL** | All in `scheme/` module |

### Designed (Not Yet Implemented)

| Item | Section | Blocked On |
|------|---------|------------|
| `CredentialPhase` (7 states) | 3.3 | StoredCredential field expansion |
| `OwnerId` | 3.2, 6 | nebula-core addition |
| `CredentialEvent` (4 variants) | 3.2, 8 | nebula-core addition |
| `StackBuilder` | 4.2 | New file `stack.rs` |
| `StoredCredential` expansion (5 new fields) | 3.2 | CredentialPhase, OwnerId |
| 5 new `CredentialError` variants | 9.1 | CredentialPhase, OwnerId, AuditSink |
| `ProviderError` (replace `Provider(String)`) | 9.1 | Migration of callers |
| `CallerIdentity` trait | 6.1 | OwnerId |
| `CredentialManager` trait | 6.1 | Use/Read split design |
| `test-support` feature | 10.2 | FakeCredentialBackend, factories |
| `nebula-credential-storage` crate | 2.2 | New crate creation |
| `nebula-credential-macros` | 2.3 | May merge into nebula-sdk-macros |
| Registry keyed on `Credential::KEY` | 3.1 | Migration from `state_kind` |

### Re-export Gaps

Plugin authors currently need 3 crate dependencies:
- `nebula-credential` — main crate
- `nebula-parameter` — for `ParameterValues`, `ParameterCollection`, `Parameter`
- `nebula-core` — for `AuthScheme` trait

Target: re-export these from `nebula-credential` for single-crate plugin DX.

### Correction: RetryAdvice

HLD R9 stated "`RetryAdvice` deleted." This is **wrong**. `RetryAdvice` exists
in code (`Never`, `Immediate`, `After(Duration)`) and is used in `RefreshFailed`.
The `Classify` trait is ALSO implemented but does NOT replace `RetryAdvice`.

### Town Hall Decisions [v1.2]

Developer town hall (plugin author, engine integrator, storage backend dev) produced
25 items. Architect responded to each. Key decisions:

**v1 Must-Ship (promoted from target to v1):**

| Change | Rationale | Blast Radius |
|--------|-----------|--------------|
| `Provider(String)` → `ProviderError { message, transient, retry_after, source }` | #1 plugin author pain point — no retryability signal | `CredentialError` breaking change |
| `CredentialEvent` in nebula-core + emission from resolver | Resources never learn about refresh — pools serve stale auth | nebula-core addition + resolver wiring |
| `CredentialHandle::Clone` → share `Arc<ArcSwap<S>>` | Clones invisible to rotation — sharing broken by design | Non-breaking semantics change |
| `CredentialRegistry` returns `CredentialSnapshot` (not `Box<dyn Any>`) | Type gap at bridge — consumers can't construct snapshot from registry output | Registry API breaking change |
| `StoredCredential.credential_key: String` | Engine can't dispatch to correct Credential type without it | StoredCredential field addition |
| `resolve_dynamic()` on `CredentialResolver` | Runtime needs type-erased resolution path for dispatch table | Additive API |
| `health_check()` on `CredentialStore` | First sign of dead backend is a failed credential access, not a probe | Trait addition with default impl |
| `ListFilter`/`ListPage` on `list()` | `Vec<String>` unbounded for production — no pagination | Trait breaking change |
| Re-export `AuthScheme`, `Parameter*`, `ParameterValues` | Plugin authors need 3 crate deps | Additive re-exports |
| `StoredCredential: PartialEq` derive | Test ergonomics | Additive derive |
| Conformance test suite (`conformance_tests(&store)`) | Backend authors re-derive semantics from InMemoryStore | Behind test-support feature |

**Deferred (v1.2):**
- OAuth2Provider trait / pub `oauth2_flow` module
- Workflow credential bindings on `NodeDefinition`
- Composite PK with `OwnerId`
- DB migration framework
- Connection pool ownership spec

**Rejected:**
- `HmacSecret::verify()` — scheme types are data carriers, no crypto logic
- Metadata encryption — by design, needed for indexing without decryption
- Vault double-encryption opt-out — defense-in-depth, negligible overhead

### Production Use-Case Town Hall [v1.3]

5 developers with real production scenarios reviewed the HLD + code. Tech lead
synthesized cross-cutting patterns across all 5 use cases.

**Use cases tested:** Slack 500-workspace OAuth2, WebSocket/HMAC streaming,
SSH jump-host multi-hop, PostgreSQL pool with 15-min IAM tokens, multi-tenant
SaaS with 10K customers.

**TOP 5 Blockers (blocks 2+ developers from shipping):**

| # | Blocker | Devs Blocked | Scope |
|---|---------|-------------|-------|
| 1 | `CredentialEvent` + resolver emission (B11) | 4/5 (Slack, WS, DB, MT) | core type + resolver wiring |
| 2 | `CredentialHandle::Clone` → `Arc<ArcSwap<S>>` | 3/5 (Slack, WS, DB) | handle.rs internal change |
| 3 | CAS retry on refresh (B7) + panic guard (B8) | 3/5 (Slack, DB, SSH) | resolver.rs refresh path |
| 4 | verify_owner (B6) + ScopeLayer (B5) + OwnerId | 2/5 + security vuln | scope.rs + core type |
| 5 | ListFilter/ListPage + batch put/delete | 2/5 (MT hard-blocker) | trait breaking change |

**New findings from cross-analysis (v1.3):**

| Finding | Impact |
|---------|--------|
| No global refresh concurrency limiter — 500 parallel calls → provider 429 → cascading CB opens | Add `Semaphore(max_concurrent_refreshes)` to `RefreshCoordinator` |
| `DatabaseAuth` missing `expires_at()` — framework can't auto-refresh IAM tokens | 1-line fix on scheme type |
| No auth failure feedback loop (401 → credential system) | Defer to v1.1, document gap |
| `ActionDependencies::credential()` is singular — jump hosts need 2+ | Change to `credentials() -> Vec<CredentialSlot>` |
| `ScopeResolver` is `&self` — can't change tenant per-request in async context | Document `tokio::task_local!` pattern |
| `SshAuthMethod` missing `Certificate` variant — enterprise SSH certs unsupported | Add `#[non_exhaustive]` variant |
| `REFRESH_POLICY` is const — can't customize per-credential-instance | Add optional override in `StoredCredential.metadata` |

**Implementation order:**
```
Week 1: #1 CredentialEvent + #2 Handle::Clone + #3 CAS retry  (parallel)
Week 2: #4 verify_owner/ScopeLayer/OwnerId + #5 ListFilter/batch  (parallel)
Week 2: Global refresh semaphore (additive)
```

**Confirmed solid (don't change):**
- Credential trait design (3 associated types, capability consts) — all 5 devs' use cases fit
- Encryption architecture (AES-256-GCM, AAD, EncryptionLayer always)
- RefreshCoordinator winner/waiter pattern (bugs in edges, not design)
- Layered storage composition (Scope → Audit → Encryption → Cache → Backend)
- AuthScheme as bridge type in nebula-core
- CredentialSnapshot with `Box<dyn Any>` + `project::<S>()`

### Open Conference Decisions [v1.4]

5 external developers reviewed the HLD from real production contexts. Architect
responded to each, accepted 6 new v1 items:

**NEW for v1 (from conference):**

| Change | From | Rationale |
|--------|------|-----------|
| `SecretStore` — minimal put/get/delete API below Credential trait | Dana (solo dev) | "The on-ramp is the highway." 20 HMAC secrets don't need 7 methods + 3 assoc types. |
| `ResolveOutcome::Stale` — graceful degradation on refresh failure | Tomasz (IoT edge) | 4000 turbines with intermittent connectivity. Stale > Error. |
| Keyed refresh semaphore per credential-type | Priya (healthcare) | 340 payer APIs with different rate limits. Global semaphore causes cascading CB opens. |
| `StoredCredential.refresh_policy_override: Option<RefreshPolicy>` | Arjun (fintech) | 9000 banks, each with different rotation rules. Const REFRESH_POLICY insufficient. |
| Structured `CertificateAuth` (chain, fingerprint, not_after) | Tomasz (IoT) | Flat PEM string loses cert structure. Auto-refresh needs `expires_at()`. |
| `cache` feature flag — moka optional | Tomasz (IoT) | 512MB ARM can't afford moka memory overhead. |

**NEW traits:**
- `QueryableAuditSink: AuditSink` — optional query extension for compliance (Arjun)
- `ResolveOutcome<S>` enum — `Fresh`/`Stale`/`Unavailable` (Tomasz)

**Rejected (architect held):**
- Collapse 5 error types into one — type boundaries serve different audiences. Added error decision tree docs instead. (Marcus)

**Definitive v1 ship list (25 items):**

| # | Item | Source |
|---|------|--------|
| 1 | `CredentialPhase` state machine (7 states) | R3 |
| 2 | CAS retry with token reuse (B7) | R3, Priya |
| 3 | Panic guard on perform_refresh (B8) | R3 |
| 4 | `CredentialEvent` in nebula-core + resolver emission (B11) | R6, Priya |
| 5 | `CredentialHandle::Clone` shares `Arc<ArcSwap<S>>` | R6 |
| 6 | `verify_owner` fail-closed (B6) | R8 |
| 7 | `ScopeLayer` filtering on list/exists (B5) | R8 |
| 8 | `ListFilter`/`ListPage` pagination | R2, Arjun |
| 9 | `#[derive(Credential)]` + `#[derive(CredentialState)]` | R10, Marcus |
| 10 | `ProviderError { transient, retry_after }` replaces `Provider(String)` | R9 |
| 11 | `StoredCredential.credential_key` field (B12) | Engine dev |
| 12 | `CredentialRegistry` returns `CredentialSnapshot` (B13) | Engine dev |
| 13 | `resolve_dynamic()` on `CredentialResolver` | Engine dev |
| 14 | `health_check()` on `CredentialStore` | Storage dev |
| 15 | CAS on missing row → `NotFound` (B10) | Storage dev |
| 16 | Global refresh concurrency semaphore (B14) | Slack dev |
| 17 | Re-exports (`AuthScheme`, `Parameter*`, `ParameterValues`) | SDK user |
| 18 | `StoredCredential: PartialEq` | Storage dev |
| 19 | Conformance test suite | Storage dev |
| 20 | **`SecretStore` minimal API** | Dana (conference) |
| 21 | **`ResolveOutcome::Stale` fallback** | Tomasz (conference) |
| 22 | **Keyed refresh semaphore** (replaces #16 global) | Priya (conference) |
| 23 | **`StoredCredential.refresh_policy_override`** | Arjun (conference) |
| 24 | **Structured `CertificateAuth`** | Tomasz (conference) |
| 25 | **`cache` feature flag for moka** | Tomasz (conference) |
| 26 | **`DecryptedCacheLayer`** — opt-in plaintext cache above EncryptionLayer | Keiko (conf2) |
| 27 | **`DatabaseAuth.extensions: serde_json::Map`** — vendor-specific fields | Rafael (conf2) |
| 28 | **`CredentialSnapshot::metadata()` getter** | Rafael (conf2) |
| 29 | **`CredentialRegistry::kinds()` + `description(key)`** | Jordan (conf2) |
| 30 | **`effective_scheme_kind` on `CredentialAuditEntry`** | Sonja (conf2) |
| 31 | **B5+B6 atomic delivery constraint** | Sonja (conf2) |
| 32 | **`MockCredentialAccessor` in test-support** (elevated priority) | Jordan (conf2) |
| 33 | **"Resolve once, snapshot many" usage pattern docs** | Keiko (conf2) |

### Conference #2 Decisions [v1.5]

5 new developers (gaming latency, Airflow migration, SOC2 audit, dev tooling, Vault integration):

**Accepted for v1:**

| Change | From | Impact |
|--------|------|--------|
| `DecryptedCacheLayer` — opt-in plaintext cache above EncryptionLayer for hot-path latency | Keiko (gaming) | New layer, opt-in via `StackBuilder::decrypted_cache()` |
| `DatabaseAuth.extensions: Map<String, Value>` — vendor-specific fields (Snowflake warehouse, etc.) | Rafael (Airflow) | [BREAKING] to DatabaseAuth construction |
| `CredentialSnapshot::metadata()` getter — expose metadata after projection | Rafael | Additive method |
| `CredentialRegistry::kinds()` iterator + `description(key)` introspection | Jordan (dev tooling) | Additive methods |
| `effective_scheme_kind` on `CredentialAuditEntry` | Sonja (SOC2) | Additive field |
| B5+B6 ship atomically — scope filtering without fail-closed (or vice versa) creates false security | Sonja | Constraint on ship order |
| `MockCredentialAccessor` exported in test-support — elevated priority (blocks CLI tooling) | Jordan | Scope of test-support feature |
| "Resolve once, snapshot many" usage pattern in Section 5 docs | Keiko | Documentation |

**Deferred to v1.1:**

| Change | From | Rationale |
|--------|------|-----------|
| `CredentialStore` dyn-compatibility (`async_trait`, delete `AnyBackend`) | Li Wei (Vault) | 33-item ship list, touches every layer. Tracking issue before v1 ships. |
| `put_batch()` on `CredentialStore` | Rafael | After ListFilter/ListPage |

**Rejected (held from both conferences):**

| Change | Reason |
|--------|--------|
| EncryptionLayer opt-out / TrustedStore marker | Defense-in-depth non-negotiable. DecryptedCacheLayer addresses latency. |
| Collapse 5 error types | Type boundaries serve different audiences. Error decision tree docs added. |
| HmacSecret::verify() | Scheme types are data carriers. |
| Metadata encryption | Needed for indexing without decryption. |

**SOC2 Audit Grades (Sonja, NordVault Compliance):**

| Criterion | Grade | Condition |
|-----------|-------|-----------|
| CC6.1 Access Control | **CONDITIONAL** | B5+B6 must ship atomically |
| CC6.3 Encryption | **PASS** | Legacy no-AAD fallback needs sunset date |
| CC7.2 Monitoring | **CONDITIONAL** | Fail-closed audit + caller identity |
| CC8.1 Change Management | **NOT ASSESSED** | Rotation audit deferred to rotation v2 |

### Open Architecture Review [v1.6]

Public session: fintech platform engineer (Priya — multi-tenant isolation), industrial IoT lead
(Marcus — certificate rotation), ML platform engineer (Yuki — usage attribution), security vendor
architect (Dmitri — key management threat model), embedded OSS developer (Lena — dependency footprint).

**Changes made:**

| Change | Type | Section |
|--------|------|---------|
| Threat model table: what v1 encryption does/doesn't protect | **[ADDED]** | §1.1 (new) |
| Design principle: "always encrypted at rest" → "encrypted on disk — see §1.1" | **[AMENDED]** | §1 |
| `StackBuilder` split: `single_tenant()` / `multi_tenant()` with typestate | **[BREAKING]** | §4.2 |
| `ScopeLevel::is_contained_in()` containment matrix documented | **[ADDED]** | §6.1 |
| `CredentialAuditEntry.effective_scheme_kind` added to struct | **[ADDED]** | §6.2 |
| `AuditLayer::with_failure_policy()` + `StackBuilder::audit_with_policy()` | **[ADDED]** | §6.2 |
| Rotation §7: single-active-material constraint made explicit | **[ADDED]** | §7 |
| `events`, `cache`, `audit`, `minimal` feature flags for core crate | **[ADDED]** | §2.1, §12.2 |
| Vault backend framing: remove download count, qualify as third-party | **[AMENDED]** | §2.2, §12.2 |
| OQ#33: Dual-material overlap window for certificate/PSK rotation | **[DEFERRED]** | §14 |
| OQ#34: `nebula-resilience` as optional dep for `minimal` feature | **[DEFERRED]** | §14 |

**Confirmed not changing (with reason):**

| Topic | Decision |
|-------|---------|
| `AuditSink` stays append-only, no aggregation | Aggregation is consumer's concern; field attribution (`effective_scheme_kind`) is sufficient |
| `project()` single-scheme return type | Multi-material fix changes `CredentialSnapshot` model across 3 crates — deferred to OQ#33 |
| Vault not promoted to v1 | B5/B6 scope bugs are higher priority for target audience than HSM backends |

---

## 1. Purpose & Scope

`nebula-credential` is the universal credential management system for the Nebula
workflow automation engine. It provides:

- A unified `Credential` trait for implementing credential types (OAuth2, API keys,
  database auth, certificates, custom types via plugins)
- Secure at-rest storage with layered composition (encryption, caching, scoping, audit)
- Runtime resolution with automatic token refresh and thundering herd prevention
- Interactive flow support (OAuth2 authorization code, device code, SAML)
- Type-safe credential injection into actions and resources via `AuthScheme` projection

**Design principles:**

- Credentials encrypted on disk (AES-256-GCM, AAD-bound) — see §1.1 for threat model
- Secrets zeroize on drop (`SecretString` / `Zeroize`); intermediate buffers use `Zeroizing<Vec<u8>>`
- Open type system — new credential types via trait implementation, not enum extension
- Layered storage — concerns separated, composed at initialization
- credential↔resource communicate via `EventBus<CredentialEvent>` — never direct imports

### 1.1 Threat Model — v1 Key Management [ADDED v1.6]

"Encrypted at rest" is ambiguous. This section replaces that phrase with precision.

**Protected against:**
- Stolen SQLite / PostgreSQL database file (without key access)
- Backup file exfiltration at rest, disk-level forensics
- Version rollback attacks (AAD binds ciphertext to `credential_id:version`)
- Record-swapping attacks (AAD binds ciphertext to identity)

**NOT protected against:**
- A compromised application process — KEK and plaintext both exist in process memory
- Memory dumps of the running process
- A malicious dependency in the same process (`NEBULA_MASTER_KEY` is process-global)
- Key leakage via logs if the environment is captured (log scrubbing is caller's responsibility)
- Side-channel attacks on AES-GCM — mitigated by system crypto library, not by this crate

**`DecryptedCacheLayer` [v1.5] expands the exposure window:** when enabled, decrypted
`StoredCredential` data is held in process memory for the TTL duration. This is a deliberate
opt-in tradeoff for hot-path latency; the security note in §4.2.1 applies.

**What v2 envelope encryption changes:** per-credential DEK wrapped by an external KEK (KMS,
HSM) — moves the key out of the application trust boundary. Until then, "encrypted on disk"
means "decryptable by the running process."

For regulated environments requiring keys to never touch application memory, implement
`CredentialBackend` against your HSM's API or use a v3 external backend crate (see §4.3).

---

## 2. Crate Boundaries

### 2.1 nebula-credential (core)

**Owns:** All abstractions, traits, types, encryption, injection model.

| Category | Contents |
|----------|----------|
| Traits | `Credential`, `CredentialState`, `PendingState`, `CredentialBackend`, `CallerIdentity`, `AuditSink`, `CredentialAccessor` (re-export from action) |
| Types | `CredentialPhase`, `CredentialStatus`, `OwnerId`, `StoredCredential`, `CredentialSnapshot`, `CredentialHandle<S>`, `CredentialContext`, `CredentialRegistry` |
| Resolution | `CredentialResolver`, `RefreshCoordinator`, executor functions |
| Layers | `EncryptionLayer`, `CacheLayer`, `AuditLayer`, `ScopeLayer`, `StackBuilder` |
| Errors | `CredentialError` (14 variants), `StoreError`, `SnapshotError`, `RegistryError`, `ResolveError`, `ExecutorError` |
| Built-in credentials | `ApiKeyCredential`, `BasicAuthCredential`, `OAuth2Credential`, `HeaderAuthCredential`, `DatabaseCredential` |
| Schemes | 13 `AuthScheme` types: `BearerToken`, `BasicAuth`, `OAuth2Token`, `HeaderAuth`, `DatabaseAuth`, `ApiKeyAuth`, `HmacSecret`, `AwsAuth`, `SshAuth`, `CertificateAuth`, `KerberosAuth`, `LdapAuth`, `SamlAuth` |
| Testing | `InMemoryBackend`, `InMemoryPendingStore` (always available); `FakeCredentialBackend`, `CredentialScenario`, factories, `assert_secret_eq!` (behind `test-support`) |
| Crypto | `EncryptionKey`, `EncryptedData`, `encrypt`/`decrypt`, `SecretString` |

**Dependencies:** `nebula-core`, `nebula-parameter`, `nebula-eventbus` (optional, `events` feature), `nebula-resilience`, `nebula-log`, `nebula-error`, `nebula-validator`. External: `aes-gcm`, `argon2`, `hkdf`, `zeroize`, `secrecy`, `moka` (optional, `cache` feature), `chrono`, `serde`, `serde_json`.

**Feature flags [ADDED v1.6]:**

```toml
# nebula-credential/Cargo.toml
[features]
default = ["events", "cache", "audit"]
events  = ["dep:nebula-eventbus"]   # CredentialEvent emission from resolver + handle
cache   = ["dep:moka"]              # CacheLayer and DecryptedCacheLayer
audit   = []                        # AuditLayer + AuditSink (no extra dep)
minimal = []                        # Disables events + cache + audit. Retains:
                                    #   Credential trait, EncryptionLayer,
                                    #   InMemoryStore, SqliteBackend.
                                    #   WARNING: no hot-reload, no audit trail.
                                    #   Not suitable for multi-tenant production.
```

`minimal` is documented as "embedded use only — disables hot-reload, caching, and audit
trail. Do not use in multi-tenant production deployments." `nebula-resilience` (circuit
breaker in `RefreshCoordinator`) remains mandatory pending OQ#34.

### 2.2 nebula-credential-storage

**Owns:** Storage backend implementations only. No cross-cutting concerns.

| Phase | Backends |
|-------|----------|
| v1 | `SqliteBackend` (desktop/dev, behind `sqlite` feature) |
| v2 | `PostgresBackend` (production, behind `postgres` feature) |
| v3 | Vault, AWS SM — **separate crates** (`nebula-credential-vault`, `nebula-credential-aws`) [AMENDED v1.6] |

> **v1.6 note:** v3 backend crates are third-party and not officially maintained by Nebula.
> Evaluate independently before adoption. The `CredentialBackend` extension point is stable;
> implement it against your preferred secrets service.

Also provides: `AnyBackend` enum dispatch, `ResilientBackend<B>` wrapper (via `nebula-resilience`), `BackendConfig` for runtime selection.

**Dependencies:** `nebula-credential` (for `CredentialBackend` trait), `rusqlite` (optional), `sqlx` (optional), `nebula-resilience` (for `ResilientBackend`).

### 2.3 nebula-credential-macros

**Owns:** Proc-macros for credential type boilerplate reduction.

| Macro | v1? | Purpose |
|-------|-----|---------|
| `#[derive(Credential)]` | Yes | Generates `resolve()`, `project()`, `description()` from attributes for static credentials |
| `#[derive(CredentialState)]` | Yes | Generates `CredentialState` impl (KIND, VERSION) + proper serde with `serde_secret` |
| `#[derive(AuthScheme)]` | Deferred | Too simple — manual impl is 5 lines |
| `#[secret]` field attribute | Deferred | Auto-wrap in SecretString + redact Debug — interesting but complex |

**Dependencies:** `syn`, `quote`, `proc-macro2`.

### 2.4 What belongs in infra, not in these crates

- **KMS integration** (AWS KMS, GCP CMEK, Azure Key Vault) — v2 alongside envelope encryption
- **RBAC policy engine** — `nebula-engine` provides `CallerIdentity` implementation
- **Workflow-level credential binding** — engine wires credentials to actions via workflow config
- **Credential UI** — desktop/web app concern; credential crate provides `parameters()` and `InteractionRequest`
- **HTTP/OAuth2 provider communication** — `reqwest` calls in credential impls, not abstracted
- **Certificate issuance/renewal** — infrastructure concern

---

## 3. Core Abstractions

### 3.1 Traits

#### Credential (the main trait)

```rust
pub trait Credential: Send + Sync + 'static {
    /// Consumer-facing auth material (e.g., BearerToken, DatabaseAuth).
    type Scheme: AuthScheme;
    /// Storable/encryptable credential state (may include refresh internals).
    type State: CredentialState;
    /// Ephemeral state for interactive flows (NoPendingState if non-interactive).
    type Pending: PendingState;

    /// Unique key for this credential type. Enforced unique by CredentialRegistry.
    /// Convention: "vendor.credential_name" (e.g., "acme.slack_bot").
    const KEY: &'static str;

    const INTERACTIVE: bool = false;
    const REFRESHABLE: bool = false;
    const REVOCABLE: bool = false;
    const TESTABLE: bool = false;
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    // Required
    fn description() -> CredentialDescription where Self: Sized;
    fn parameters() -> ParameterCollection where Self: Sized;
    fn project(state: &Self::State) -> Self::Scheme where Self: Sized;
    fn resolve(
        values: &ParameterValues, ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>> + Send
    where Self: Sized;

    // Default impls (override when capability const is true)
    fn continue_resolve(/* ... */) -> impl Future<...> + Send { async { Err(NotInteractive) } }
    fn refresh(/* ... */) -> impl Future<...> + Send { async { Ok(RefreshOutcome::NotSupported) } }
    fn test(/* ... */) -> impl Future<...> + Send { async { Ok(TestResult::Untestable) } }
    fn revoke(/* ... */) -> impl Future<...> + Send { async { Ok(()) } }
}
```

Three associated types are all load-bearing:
- **Scheme** (what consumers see): No refresh internals. Actions and resources receive this.
- **State** (what gets stored): Includes refresh_token, client_secret. Consumers never see.
- **Pending** (interactive flow ephemera): Minutes lifetime, single-use, separate security properties.

#### AuthScheme (in nebula-core)

```rust
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    const KIND: &'static str;
    fn expires_at(&self) -> Option<DateTime<Utc>> { None }
}
```

Bridge type between credential (producer) and resource/action (consumer).

#### CredentialStore (storage interface) [AMENDED v1.1]

> **Naming:** HLD v1.0 used `CredentialBackend`. The implemented trait is `CredentialStore`.

**IMPLEMENTED** — current code signature:

```rust
pub trait CredentialStore: Send + Sync {
    fn get(&self, id: &str)
        -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;
    fn put(&self, credential: StoredCredential, mode: PutMode)
        -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;
    fn delete(&self, id: &str)
        -> impl Future<Output = Result<(), StoreError>> + Send;
    fn list(&self, state_kind: Option<&str>)
        -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;
    fn exists(&self, id: &str)
        -> impl Future<Output = Result<bool, StoreError>> + Send;
}
```

**[TARGET DESIGN] Enhancements:**
- Migrate `&str` IDs to `&CredentialId` newtype
- Add `ListFilter`/`ListPage` for pagination (replace `Vec<String>`)
- Add `health_check()` method + `StoreError::Unavailable` variant
- Rename to `CredentialBackend` for clarity (store = composed stack, backend = raw persistence)

RPITIT (not `async_trait`) — not object-safe by design; enum dispatch via `AnyBackend` in `nebula-credential-storage`.

#### CredentialRegistry

Keys on `Credential::KEY` (not `State::KIND`). Returns `Result<(), RegistryError::DuplicateKey>` on collision. Namespacing convention: `"vendor.credential_name"`.

### 3.2 Key types and enums

#### OwnerId (nebula-core) [TARGET DESIGN] [AMENDED v1.1]

> **Status:** DESIGNED — not yet in `nebula-core` or `nebula-credential`.

```rust
pub struct OwnerId {
    scope: ScopeLevel,
    principal: String,
}
```

Replaces three disconnected representations (owner_id: String, caller_scope: Option<ScopeLevel>, owner_scope: Option<ScopeLevel>). Implementation blocked on nebula-core addition.

#### CredentialEvent (nebula-core) [TARGET DESIGN] [AMENDED v1.1]

> **Status:** DESIGNED — not yet in `nebula-core`. Old `CredentialRotationEvent`
> (which leaks credential state) still exists in `rotation/events.rs` behind feature gate.

```rust
#[non_exhaustive]
pub enum CredentialEvent {
    Rotated { credential_id: CredentialId, generation: u64 },
    Refreshed { credential_id: CredentialId, generation: u64 },
    Revoked { credential_id: CredentialId, generation: u64 },
    ExpiringSoon { credential_id: CredentialId, generation: u64, expires_in: Duration },
}
```

Lives in nebula-core (not credential) so both emitter and consumer can use it without peer imports. Carries `credential_id + generation` only — **NO credential state, NO secrets** (R3 binding).

#### StoredCredential [AMENDED v1.1]

**IMPLEMENTED** — current 8-field struct:

```rust
pub struct StoredCredential {
    pub id: String,                                 // target: CredentialId newtype
    pub data: Vec<u8>,                              // encrypted
    pub state_kind: String,
    pub state_version: u32,
    pub version: u64,                               // CAS (every write)
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Map<String, Value>,
}
```

**[TARGET DESIGN] Fields to add:**

| Field | Type | Purpose | Blocked on |
|-------|------|---------|------------|
| `owner` | `OwnerId` | Scoped ownership | `OwnerId` type in nebula-core |
| `phase` | `CredentialPhase` | State machine position | `CredentialPhase` enum |
| `generation` | `u64` | Rotation-only counter | Rotation v2 integration |
| `grace_expires_at` | `Option<DateTime<Utc>>` | Grace period tracking | Rotation v2 integration |
| `id` migration | `String` → `CredentialId` | Type safety | Mechanical refactor |

#### CredentialSnapshot

```rust
pub struct CredentialSnapshot {
    kind: String,
    scheme_kind: String,
    metadata: CredentialMetadata,
    projected: Box<dyn Any + Send + Sync>,
    clone_fn: fn(&(dyn Any + Send + Sync)) -> Box<dyn Any + Send + Sync>,
}
```

Type-erased projected `AuthScheme`. NOT Serialize. IS Clone (via captured clone_fn). Debug redacts projected value.

### 3.4 SecretStore — minimal encrypted key-value API [v1.4]

> **Status:** NEW from conference. Addresses the "on-ramp is the highway" feedback.

The `Credential` trait is designed for credentials with lifecycle (refresh, rotation,
interactive flows). For static secrets that need only encrypt-store-retrieve, the full
trait is excessive. `SecretStore` is the floor below the `Credential` trait.

```rust
/// Minimal encrypted key-value store for static secrets.
/// No Credential trait, no registry, no resolution, no layers beyond encryption.
///
/// # Examples
/// ```rust
/// let store = SecretStore::new(backend, encryption_key);
/// store.put("stripe_key", b"sk_live_abc123").await?;
/// let secret: SecretBytes = store.get("stripe_key").await?;
/// store.delete("stripe_key").await?;
/// ```
pub struct SecretStore<B: CredentialStore> {
    backend: EncryptionLayer<B>,
}

impl<B: CredentialStore> SecretStore<B> {
    pub fn new(backend: B, key: Arc<EncryptionKey>) -> Self;
    pub async fn put(&self, id: &str, plaintext: &[u8]) -> Result<(), StoreError>;
    pub async fn get(&self, id: &str) -> Result<Zeroizing<Vec<u8>>, StoreError>;
    pub async fn delete(&self, id: &str) -> Result<(), StoreError>;
    pub async fn list(&self) -> Result<Vec<String>, StoreError>;
}
```

**On-ramp hierarchy:**
1. `SecretStore` — 3 lines, put/get/delete (solo devs, simple secrets)
2. `#[derive(Credential)]` + `StaticProtocol` — 15 lines (most plugin authors)
3. `impl Credential` manually — full control (OAuth2, interactive flows, custom refresh)

### 3.3 Credential state machine [TARGET DESIGN] [AMENDED v1.1]

> **Status:** DESIGNED — `CredentialPhase` does not exist in code. No `phase.rs` file.
> `StoredCredential` has no `phase` field. Implementation blocked on StoredCredential expansion.

```
                    ┌──────────────────────────┐
                    │                          │
  Created ──→ Pending ──→ Active ──→ Refreshing ──→ Active
    │                       │    ↑       │
    │                       │    │       ↓
    │                       │    └── GracePeriod
    │                       │           │
    │                       ↓           ↓
    │                    Expired     Expired
    │                       │           │
    ↓                       ↓           ↓
  Revoked ←──────────── Revoked ←── Revoked
```

```rust
// TARGET — not yet in code
#[non_exhaustive]
pub enum CredentialPhase {
    Created,       // Initial state
    Pending,       // Interactive flow in progress
    Active,        // Resolved and usable
    Refreshing,    // Refresh in progress (old credential still usable)
    GracePeriod,   // New credential committed, old still valid
    Expired,       // Cannot be refreshed, needs re-auth
    Revoked,       // Explicitly invalidated (terminal)
}

impl CredentialPhase {
    pub fn is_usable(&self) -> bool {
        matches!(self, Active | Refreshing | GracePeriod)
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, Revoked)
    }
    pub fn can_transition_to(self, target: Self) -> bool { /* 21 valid transitions */ }
}
```

**version vs generation:**
- `version: u64` — increments on every store write (CAS for optimistic concurrency)
- `generation: u64` — increments only on rotation (new key material). Refresh does NOT increment.

---

## 4. Storage Architecture

### 4.1 CredentialBackend trait

Defined in `nebula-credential`. Uses `CredentialId` (not String), `ListFilter`/`ListPage` for pagination, `health_check()` method, `StoreError::Unavailable` variant.

### 4.2 Encryption ownership and invariants

**Encryption always owned by `nebula-credential` via `EncryptionLayer`.** Backends receive ciphertext only. No opt-out for v1 — even for Vault/AWS SM backends (defense-in-depth: provider misconfiguration doesn't expose plaintext).

```
Request → ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend
```

**[TARGET DESIGN] StackBuilder** [AMENDED v1.6] — enforces encryption-mandatory composition
with compile-time tenant isolation tier:

> **Status:** DESIGNED — `StackBuilder` and `CredentialStack<B, T>` do not exist.
> No `stack.rs` file. Current workaround: compose layers manually.

**v1.6 BREAKING CHANGE:** `StackBuilder::new()` is replaced by two constructors with typestate
that encodes the isolation model at the type level. `ScopeLayer` is mandatory for multi-tenant
stacks and structurally absent from single-tenant stacks — it cannot be silently omitted.

```rust
// TARGET — not yet in code

pub struct SingleTenant;   // marker: no scope enforcement
pub struct MultiTenant;    // marker: ScopeLayer mandatory

// ── Single-tenant ───────────────────────────────────────────────────────────
// Admin-only access model. .scope() method does NOT exist.
impl<B: CredentialBackend> StackBuilder<B, SingleTenant> {
    pub fn new(backend: B, key: Arc<EncryptionKey>) -> Self;
    pub fn cache(self, config: CacheConfig) -> Self;
    pub fn decrypted_cache(self, config: DecryptedCacheConfig) -> Self;
    pub fn audit(self, sink: Arc<dyn AuditSink>) -> Self;
    pub fn audit_with_policy(self, sink: Arc<dyn AuditSink>, policy: AuditFailurePolicy) -> Self; // [ADDED v1.6]
    pub fn build(self) -> CredentialStack<B, SingleTenant>;
}

// ── Multi-tenant ────────────────────────────────────────────────────────────
// CallerIdentity required at construction. build() only callable once provided.
impl<B: CredentialBackend> StackBuilder<B, MultiTenant> {
    pub fn new(backend: B, key: Arc<EncryptionKey>, identity: Arc<dyn CallerIdentity>) -> Self;
    pub fn cache(self, config: CacheConfig) -> Self;
    pub fn decrypted_cache(self, config: DecryptedCacheConfig) -> Self;
    pub fn audit(self, sink: Arc<dyn AuditSink>) -> Self;
    pub fn audit_with_policy(self, sink: Arc<dyn AuditSink>, policy: AuditFailurePolicy) -> Self; // [ADDED v1.6]
    pub fn build(self) -> CredentialStack<B, MultiTenant>;
}
```

`CredentialStack<B, SingleTenant>` and `CredentialStack<B, MultiTenant>` are different types.
Engine configuration specifies which path it takes. Plugin authors cannot silently omit tenant
isolation by forgetting `.scope()`.

**What this breaks vs v1.2 target:** previous `StackBuilder::new(backend, key)` with optional
`.scope()` is removed. Any code targeting that signature must migrate to one of the two constructors.

**Current workaround** (manual layer composition — until `stack.rs` is implemented):
```rust
let store = InMemoryStore::new();
let encrypted = EncryptionLayer::new(store, key);
let cached = CacheLayer::new(encrypted, CacheConfig::default());
// No enforcement that EncryptionLayer is present
// No enforcement that ScopeLayer is present for multi-tenant use
```

**Invariants (IMPLEMENTED in EncryptionLayer):**
- AES-256-GCM, random 96-bit nonce via `OsRng`
- AAD = `credential_id:version` (prevents record-swapping AND version rollback)
- Three-tier AAD fallback for migration: `id:version` → `id` → no-AAD
- All intermediate plaintext buffers use `Zeroizing<Vec<u8>>`
- `EncryptionKey` implements `Zeroize + ZeroizeOnDrop` + explicit `Debug` with `[REDACTED]`
- `EncryptedData.key_id: Option<String>` — `None` for v1 (direct KEK), reserved for v2 envelope encryption
- `CacheLayer` caches **ciphertext only** — no plaintext in cache (default, safe)
- `DecryptedCacheLayer` [v1.5] caches **plaintext** — opt-in for hot-path latency (see below)

### 4.2.1 DecryptedCacheLayer [v1.5]

CacheLayer (below EncryptionLayer) caches ciphertext → every cache hit still pays AES-256-GCM
decrypt cost. For high-frequency resolution (gaming at 14 regions, IoT polling), this is
measurable overhead.

`DecryptedCacheLayer` sits **above** EncryptionLayer, caching decrypted `StoredCredential` data
in process memory with TTL-based eviction:

```
Request → ScopeLayer → AuditLayer → DecryptedCacheLayer → EncryptionLayer → CacheLayer → Backend
                                     ↑ plaintext cache      ↑ ciphertext cache
```

```rust
/// Opt-in plaintext cache for hot-path credential resolution.
/// Trades memory exposure for latency: decrypted data held in process memory.
/// Use only when decrypt latency dominates and the process memory trust
/// boundary is acceptable.
pub struct DecryptedCacheLayer<S> {
    inner: S,
    cache: moka::future::Cache<String, StoredCredential>,
}
```

**StackBuilder integration:**
```rust
StackBuilder::new(backend, key)
    .decrypted_cache(DecryptedCacheConfig { ttl: Duration::from_secs(60) }) // opt-in
    .cache(CacheConfig::default())  // ciphertext cache still useful for backend round-trips
    .build()
```

**Security note:** Plaintext credential data in process memory. NOT the default.
Document as: "Use for gaming, real-time, and high-throughput paths where
sub-millisecond resolution is required."

**Key management (v1, layered priority):**
1. Environment variable (`NEBULA_MASTER_KEY`, 64 hex chars)
2. OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service)
3. Argon2id passphrase derivation (19 MiB memory, 2 iterations)
4. KMS — deferred to v2 alongside envelope encryption

### 4.3 v1 backends vs Phase 2 backends

| Phase | Backend | Crate | Feature |
|-------|---------|-------|---------|
| v1 | `InMemoryBackend` | `nebula-credential` | always |
| v1 | `SqliteBackend` | `nebula-credential-storage` | `sqlite` (default) |
| v2 | `PostgresBackend` | `nebula-credential-storage` | `postgres` |
| v3 | `VaultBackend` | `nebula-credential-vault` (third-party, evaluate independently) | — |
| v3 | `AwsSmBackend` | `nebula-credential-aws` (third-party, evaluate independently) | — |

Runtime backend selection via `BackendConfig` (serde-tagged enum). Enum dispatch via `AnyBackend` (monomorphized, no `dyn`).

---

## 5. Injection Model

**Pull-based injection** via `ctx.credential_typed::<S>(id)`.

```
Parameters (UI) → resolve() → CredentialState (encrypted store)
                                    ↓ project()
                              AuthScheme (consumer-facing)
                                    ↓ Box<dyn Any>
                              CredentialSnapshot
                                    ↓ into_project::<S>()
                        ┌───────────┴──────────┐
                   ActionContext            ResourceManager
                   .credential_typed()     .acquire(&auth)
```

- `CredentialSnapshot` in `nebula-credential` — type-erased `Box<dyn Any + Send + Sync>`
- `CredentialAccessor` in `nebula-action` — `#[async_trait]` for object safety (`Arc<dyn CredentialAccessor>`)
- Bridge built by `nebula-runtime`: `RuntimeCredentialAccessor` + `CredentialDispatchTable`
- `Box<dyn Any>` erasure is the necessary cost of type-erased `dyn` accessor — downcast is `TypeId` comparison (effectively free)
- No `serde_json::Value` at injection boundary
- `dynosaur` not needed — capability traits are `dyn`-only

**`CredentialHandle::Clone` fix [v1.2]:** Wrap inner `ArcSwap` in `Arc` so clones share the same swap point:

```rust
// BEFORE (broken): Clone creates independent ArcSwap
pub struct CredentialHandle<S: AuthScheme> {
    scheme: ArcSwap<S>,         // each clone gets its own ArcSwap
    credential_id: String,
}

// AFTER (v1.2): Clone shares the ArcSwap via Arc
pub struct CredentialHandle<S: AuthScheme> {
    inner: Arc<ArcSwap<S>>,     // all clones see rotation updates
    credential_id: String,
}
```

### 5.0.1 Usage Patterns [v1.5]

**"Resolve once, snapshot many"** is the canonical hot-path pattern:

```rust
// COLD PATH (startup or first access): resolve from store + decrypt + deserialize
let handle: CredentialHandle<BearerToken> = resolver
    .resolve::<ApiKeyCredential>("prod-api-key").await?;

// HOT PATH (per-request): atomic load + Arc refcount bump
// Zero allocations, zero locks, 5-15ns on x86-64
let token: Arc<BearerToken> = handle.snapshot();

// Use token for the entire request/match/message
let header = format!("Bearer {}", token.expose().expose_secret(|s| s));
```

**Performance characteristics:**
- `handle.snapshot()` — 5-15ns, zero heap alloc, wait-free (ArcSwap atomic load)
- `resolver.resolve()` — 1-5μs cache hit (moka + AES-256-GCM decrypt), 10-100μs cache miss
- `resolver.resolve()` with `DecryptedCacheLayer` — ~50ns cache hit (no decrypt)

**For gaming/streaming:** resolve at session start, hold handle for session lifetime,
snapshot per frame/message. Refresh happens asynchronously via `RefreshCoordinator`
and is visible on next `snapshot()` call.

### 5.1 Runtime Bridge [v1.2]

The engine needs type-erased credential resolution (string ID → `CredentialSnapshot`
without compile-time knowledge of `Credential` type). Two mechanisms:

**1. `CredentialRegistry` returns `CredentialSnapshot` directly (not `Box<dyn Any>`):**

```rust
// Registry captures clone_fn and scheme_kind at register() time
pub fn project(&self, credential_key: &str, data: &[u8], metadata: CredentialMetadata)
    -> Result<CredentialSnapshot, RegistryError>;
```

**2. `CredentialResolver::resolve_dynamic()` for type-erased resolution:**

```rust
impl<S: CredentialStore> CredentialResolver<S> {
    /// Type-erased resolution for runtime dispatch.
    /// Uses CredentialRegistry to deserialize + project without generic parameter.
    /// Does NOT support refresh (refresh requires concrete Credential type).
    pub async fn resolve_dynamic(
        &self, credential_id: &str,
    ) -> Result<CredentialSnapshot, ResolveError> {
        let stored = self.store.get(credential_id).await?;
        self.registry.project(&stored.credential_key, &stored.data, metadata)
            .map_err(|e| ResolveError::Deserialize { ... })
    }
}
```

**3. `StoredCredential.credential_key` field** enables dispatch:

```
Workflow config: node X uses credential "prod-slack-oauth2"
StoredCredential: credential_key = "slack_bot_token"
Registry: "slack_bot_token" → monomorphized project closure
```

---

## 6. Scoping & Access Control

### 6.1 Type-level isolation

**`OwnerId`** in `nebula-core`: typed ownership replacing three disconnected string representations.

```rust
pub struct OwnerId {
    scope: ScopeLevel,     // Organization, Project, Workflow, etc.
    principal: String,     // scope-specific identity
}

impl OwnerId {
    pub fn can_access(&self, resource_owner: &OwnerId) -> bool {
        self.principal == resource_owner.principal
            || resource_owner.scope.is_contained_in(&self.scope)
    }
}
```

**`ScopeLevel::is_contained_in()` containment matrix [ADDED v1.6]:**

`caller.can_access(resource)` — outcome matrix:

| Caller scope → | Organization | Project | Workflow |
|----------------|:-----------:|:-------:|:--------:|
| **Resource: Organization** | ✓ same principal | ✗ | ✗ |
| **Resource: Project** | ✓ org contains project | ✓ same principal | ✗ |
| **Resource: Workflow** | ✓ org contains workflow | ✓ project contains workflow | ✓ same principal |

Rules:
- A caller can access resources at **the same or lower** scope within the same principal chain.
- A `Project` caller cannot access a **sibling** `Project`'s credentials — the `principal` must
  match at the organization level for that to hold.
- **Ownerless credentials** (admin-only): accessible only when `CallerIdentity::caller()` returns
  `None`. Fail-closed — scoped callers are rejected, not given access.

**Use/Read split** — type-level enforcement:
- **Actions** get `CredentialAccessor` → can only `use_credential()` (projected `AuthScheme`)
- **Framework/admin** gets `CredentialManager` → can `read_credential()` (raw state), `write_credential()`, `delete_credential()`
- Type system prevents actions from calling `read_credential`

**`CallerIdentity` trait** (renamed from `ScopeResolver` in credential to avoid collision with `nebula_core::ScopeResolver`):

```rust
pub trait CallerIdentity: Send + Sync {
    fn caller(&self) -> Option<&OwnerId>;
}
```

### 6.2 Multi-tenancy model

**v1 runtime isolation:**
- `CallerIdentity` struct validated at construction via `ScopeResolver`
- `ScopeLayer` MUST filter `list()` and `exists()` — unfiltered = cross-tenant enumeration (CRITICAL fix)
- Unscoped credentials accessible to admin only (fail-closed, not fail-open)
- `StoredCredential.owner: OwnerId` as first-class field
- Storage: shared table with scope columns, composite PK `(owner_scope, owner_principal, id)`

**`AuditEntry`** with `cross_tenant_attempt: bool` from day 1:

```rust
pub struct CredentialAuditEntry {
    pub entry_id: Uuid,
    pub caller_id: String,
    pub caller_scope: Option<ScopeLevel>,
    pub credential_id: String,
    pub credential_owner: Option<String>,
    pub operation: AuditOperation,
    pub timestamp: DateTime<Utc>,
    pub duration: Duration,
    pub outcome: AuditOutcome,
    pub scope_check: ScopeCheckResult,
    // execution context
    pub workflow_id: Option<WorkflowId>,
    pub execution_id: Option<ExecutionId>,
    pub trace_id: Uuid,
    /// [ADDED v1.6] AuthScheme::KIND of the projected scheme when projection occurred.
    /// None for operations that don't project (delete, list, exists).
    /// Non-None for get + resolve operations that returned auth material to the caller.
    /// Required for per-scheme attribution (e.g. OAuth2 vs ApiKey burn rate per team).
    pub effective_scheme_kind: Option<&'static str>,
}
```

**`AuditSink` fail-closed by default, configurable per-instance [AMENDED v1.6]:**

```rust
pub trait AuditSink: Send + Sync {
    fn log(&self, entry: CredentialAuditEntry);
    fn on_audit_failure(&self, error: &dyn std::error::Error) -> AuditFailurePolicy {
        AuditFailurePolicy::RejectOperation // fail-closed default
    }
}
```

`AuditLayer` accepts a per-instance failure policy without requiring a custom `AuditSink`:

```rust
// [ADDED v1.6] — TARGET, not yet in code
impl<B: CredentialBackend> AuditLayer<B> {
    pub fn new(inner: B, sink: Arc<dyn AuditSink>) -> Self;
    /// Override failure policy at construction without subclassing AuditSink.
    /// Use WarnAndContinue for non-critical credentials (ML training keys, etc.)
    /// where an audit hiccup should not halt execution.
    /// Use RejectOperation (default) for financial or multi-tenant credentials
    /// where an audit gap is a compliance violation.
    pub fn with_failure_policy(mut self, policy: AuditFailurePolicy) -> Self;
}
```

`StackBuilder` exposes this through the builder chain (both `SingleTenant` and `MultiTenant`):

```rust
// [ADDED v1.6]
pub fn audit_with_policy(
    self,
    sink: Arc<dyn AuditSink>,
    policy: AuditFailurePolicy,
) -> Self;
```

**Deferred to v2:** Per-tenant encryption key derivation (HKDF), explicit sharing grants, per-workflow credential restriction, compile-time tenant proof types, `list_by_owner` pushed to backend trait.

---

## 7. Rotation Protocol [AMENDED v1.1]

> **Feature gate:** The rotation module (`RotatableCredential`, `TestableCredential`,
> `RotationScheduler`, grace period tracking) is behind `#[cfg(feature = "rotation")]`
> and is **not** enabled by default. It is disconnected from the v2 `Credential` trait.
>
> The `RefreshCoordinator` (thundering herd prevention for token refresh) is
> **always available** — NOT feature-gated. Only key rotation (new material,
> generation increment, grace period) is gated.
>
> The CAS retry fix (R3), scopeguard fix (R3), and expired-non-refreshable
> error (R3) are TARGET DESIGN fixes that need implementation.

### Refresh (same key material, extended lifetime)

```
Active → Refreshing → Active  (success)
                    → Expired  (failure, reauth required)
```

- `RefreshCoordinator`: Winner/Waiter pattern via `Notify` + `scopeguard`
- Winner performs `Credential::refresh(&mut state, ctx)`
- CAS write with **1-2 retries** on `VersionConflict` (R3 fix: prevents token loss)
- `scopeguard` must call both `notify_waiters()` AND `complete()` (R3 fix: prevents poisoned in-flight map on panic)
- Circuit breaker: 5 failures → 300s open → half-open probe
- `RefreshPolicy`: early_refresh (5min), min_retry_backoff (5s), jitter (30s random)
- Expired non-refreshable credentials return `ResolveError::Expired`, not stale data (R3 fix)
- **CAS retry reuses obtained token** — does NOT call `refresh()` again (Priya, conference)

### 7.1 Refresh Concurrency [v1.4]

**Keyed semaphore** replaces global semaphore (Priya: 340 APIs with different rate limits):

```rust
pub struct RefreshConcurrencyConfig {
    pub default_max_concurrent: usize,       // default: 32
    pub per_key_limits: HashMap<String, usize>, // key = Credential::KEY or partition
    pub global_ceiling: usize,               // absolute max across all keys
}
```

### 7.2 RefreshPolicy Resolution [v1.4]

Per-instance override (Arjun: 9000 banks, each with different rotation rules):

1. `StoredCredential.refresh_policy_override` (per-instance, runtime)
2. `Credential::REFRESH_POLICY` (per-type, const)
3. `RefreshPolicy::DEFAULT` (system)

### 7.3 Stale Credential Fallback [v1.4]

Graceful degradation for intermittent connectivity (Tomasz: edge devices):

```rust
pub enum ResolveOutcome<S: AuthScheme> {
    /// Fresh credential, normal operation.
    Fresh(CredentialHandle<S>),
    /// Expired but last-known-good available. Consumer decides.
    Stale {
        handle: CredentialHandle<S>,
        expired_since: DateTime<Utc>,
        last_refresh_error: Option<String>,
    },
    /// No credential available (never resolved, or revoked).
    Unavailable(ResolveError),
}
```

Resolver tries to refresh, fails, returns `Stale` with last-known-good instead of
`Err(RefreshFailed)`. Consumer (action) decides whether stale is acceptable for
their use case. IoT turbine keeps reporting; high-security fintech rejects stale.

### Rotation (new key material)

```
Active → Refreshing → GracePeriod → Active  (grace period ends)
```

- Increments `generation`. Grace period keeps old material in `StoredCredential`.
- `ArcSwap` in `CredentialHandle` hot-swaps new scheme; in-flight `Arc<S>` snapshots remain valid.
- Grace period configurable with validated min/max. Persisted in `StoredCredential.grace_expires_at`.

> **Design constraint [ADDED v1.6] — single-active-material:** `project(state: &Self::State) -> Self::Scheme`
> returns exactly one `AuthScheme`. There is no mechanism to serve both old and new credential
> material simultaneously during a rotation handoff.
>
> The grace period preserves old material *in storage* for rollback, but the injection model
> projects only one scheme to consumers at any given time. Use cases requiring dual-material
> overlap windows — X.509 certificate rotation where both certs must authenticate while a device
> restarts, PSK handoff with firmware restart timers — are **not expressible** with the current
> `project()` contract. See OQ#33 for the design path forward.

### 4 Rotation triggers

```rust
pub enum RefreshTrigger {
    ProactiveExpiry,      // framework detects approaching expiry
    AuthFailure,          // action reports 401/403 via EventBus
    ScheduledRotation,    // RotationScheduler policy
    ManualRotation,       // user/admin action
}
```

---

## 8. Hot Reload Integration

### 8.1 CredentialEvent (in nebula-core)

4 variants: `Rotated`, `Refreshed`, `Revoked`, `ExpiringSoon`. Carry `credential_id + generation` only. **No credential state in events** (R3 binding). Delete existing `CredentialRotationEvent` (leaks state).

### 8.2 nebula-resource subscriber pattern

```rust
pub struct CredentialBinding {
    credential_id: CredentialId,
    last_seen_generation: AtomicU64,
}
```

- `Manager::with_credential_events(config, &credential_bus)` spawns listener task
- `Rotated` → bump pool fingerprint, evict stale on next maintenance
- `Revoked` → immediate pool drain, fail-fast
- `Refreshed` → lazy re-auth on next checkout
- `ExpiringSoon` → advisory log warning

**Watchdog fallback** for EventBus overflow: periodically compare `CredentialBinding.last_seen_generation` against credential system's current generation.

---

## 9. Error Taxonomy [AMENDED v1.1]

### 9.1 CredentialError enum

**IMPLEMENTED** — current 10-variant enum:

```rust
// CURRENT CODE (error.rs)
pub enum CredentialError {
    Crypto { source: CryptoError },
    Validation { source: ValidationError },
    NotInteractive,
    Provider(String),                    // TARGET: replace with ProviderError (see below)
    InvalidInput(String),                // TARGET: absorb into Validation
    RefreshFailed { kind: RefreshErrorKind, retry: RetryAdvice, source: Box<dyn Error + Send + 'static> },
    RevokeFailed { source: Box<dyn Error + Send + 'static> },
    CompositionNotAvailable,
    CompositionFailed { source: Box<dyn Error + Send + 'static> },
    SchemeMismatch { expected: &'static str, actual: String },
}
```

> **Correction from v1.0:** `RetryAdvice` is NOT deleted. It exists (`Never`, `Immediate`,
> `After(Duration)`) and is used in `RefreshFailed`. The `Classify` trait is also
> implemented but does not replace `RetryAdvice`.

**[TARGET DESIGN] 14-variant enum (5 new variants + 1 replacement):**

```rust
// TARGET — not yet in code
pub enum CredentialError {
    // Wrapping (IMPL)
    Crypto { source: CryptoError },
    Validation { source: ValidationError },

    // Provider (TARGET: replaces Provider(String))
    ProviderError { message: String, transient: bool, retry_after: Option<Duration>,
                    source: Option<Box<dyn Error + Send + Sync>> },

    // Refresh/Revoke (IMPL, target: remove RetryAdvice, add Sync)
    RefreshFailed { kind: RefreshErrorKind, source: Box<dyn Error + Send + Sync> },
    RevokeFailed { source: Box<dyn Error + Send + Sync> },

    // Lifecycle (TARGET — needs CredentialPhase)
    Expired { credential_id: String },
    RotationInProgress { credential_id: String },

    // Access control (TARGET — needs OwnerId/ScopeLayer enforcement)
    AccessDenied { reason: String },
    AuditFailed { source: Box<dyn Error + Send + Sync> },

    // Composition (IMPL except CircularDependency)
    CompositionNotAvailable,
    CompositionFailed { source: Box<dyn Error + Send + Sync> },
    CircularDependency { cycle: String },  // TARGET

    // Type / protocol (IMPL)
    SchemeMismatch { expected: &'static str, actual: String },
    NotInteractive,
}
```

### 9.2 Retry policy per variant

| Variant | Retryable | Classify Category |
|---------|-----------|-------------------|
| `Crypto` | Never | Internal |
| `Validation` | Never | Validation |
| `ProviderError(transient=true)` | Yes | External |
| `ProviderError(transient=false)` | Never | Internal |
| `RefreshFailed(TransientNetwork)` | Yes | External |
| `RefreshFailed(ProviderUnavailable)` | Yes | External |
| `RefreshFailed(TokenExpired/Revoked)` | Never | Authentication |
| `RefreshFailed(ProtocolError)` | Never | Internal |
| `Expired` | Never | Authentication |
| `RotationInProgress` | Yes (500ms, max 10) | Conflict |
| `AccessDenied` | Never | Authorization |
| `AuditFailed` | Never | Internal |
| `CircularDependency` | Never | Validation |
| `SchemeMismatch` | Never | Validation |
| `NotInteractive` | Never | Unsupported |

**Boundary mapping to `ActionError`:** Retryable → `ActionError::Retryable`. Auth/validation → `ActionError::Fatal` with safe message. Crypto/audit internals → `ActionError::Fatal` with error code only (redacted).

---

## 10. Testing Strategy

### 10.1 FakeCredentialBackend surface

```rust
pub struct FakeCredentialBackend {
    inner: InMemoryBackend,
    fault: Arc<Mutex<FaultMode>>,
    get_count: AtomicU32,
    put_count: AtomicU32,
    cas_attempt_count: AtomicU32,
}
```

Capabilities: configurable failure injection (`fail_next(n, factory)`), latency injection (`set_latency(dur)` — use with `tokio::time::pause()`), call counters, CAS tracking. Wraps `InMemoryBackend` for storage semantics.

### 10.2 test-support feature flag contents

```toml
[features]
test-support = ["tokio/test-util"]
```

**Behind `test-support`:** `FakeCredentialBackend`, `CredentialScenario` builder, factory functions (`test_bearer_token`, `test_context`, `make_stored_credential`), `assert_secret_eq!` macro.

**Always available:** `InMemoryBackend`, `InMemoryPendingStore`, `CredentialContext::new()`, all credential/scheme types.

### 10.3 External plugin test harness

Plugin author testing WITHOUT full runtime:

```rust
#[tokio::test]
async fn resolve_my_credential() {
    let mut values = ParameterValues::new();
    values.set("api_key", json!("test-key-abc"));
    let ctx = CredentialContext::new("test-user");

    let result = MyCredential::resolve(&values, &ctx).await.unwrap();
    match result {
        StaticResolveResult::Complete(state) => { /* assert */ }
        _ => panic!("expected Complete"),
    }
}
```

Full pipeline test with `CredentialScenario`:

```rust
#[tokio::test]
async fn full_pipeline() {
    let scenario = CredentialScenario::new();
    scenario.seed(make_stored_credential("1", "test-token")).await;
    let handle = scenario.resolver()
        .resolve::<MyCredential>("test-cred-1").await.unwrap();
    handle.snapshot().expose().expose_secret(|s| assert_eq!(s, "test-token"));
}
```

**Secret assertion safety:** `SecretString` has no `PartialEq` — compiler rejects `assert_eq!(secret_a, secret_b)`. Use `expose_secret(|s| assert_eq!(s, expected))` or `assert_secret_eq!` macro (shows lengths on mismatch, not values).

---

## 11. nebula-credential-macros

### 11.1 v1 macro surface

**`#[derive(Credential)]`** — for static credentials (State = Scheme):

```rust
#[derive(Credential)]
#[credential(key = "jira_api", name = "Jira API Token", scheme = BearerToken)]
pub struct JiraCredential;

impl StaticProtocol for JiraCredential {
    type Scheme = BearerToken;
    fn parameters() -> ParameterCollection { /* ... */ }
    fn build(values: &ParameterValues) -> Result<BearerToken, CredentialError> { /* ... */ }
}
```

Generates: `Credential` impl with `resolve()` delegating to `StaticProtocol::build()`, `project()` as identity clone, `description()` from attributes.

**`#[derive(CredentialState)]`** — for custom state types:

```rust
#[derive(CredentialState, Serialize, Deserialize)]
#[credential_state(kind = "my_oauth2", version = 1)]
pub struct MyOAuth2State {
    #[serde(with = "serde_secret")]
    access_token: SecretString,
    #[serde(with = "serde_secret")]
    refresh_token: SecretString,
    expires_at: DateTime<Utc>,
}
```

Generates: `CredentialState` impl with `KIND` and `VERSION` from attributes.

### 11.2 Deferred macros

- `#[derive(AuthScheme)]` — too simple (5 lines manual impl)
- `#[secret]` field attribute — interesting but complex interaction with serde/Debug

---

## 12. Extension Points

### 12.1 CredentialBackend plugin API

Third-party storage backend: implement `CredentialBackend` trait, wrap with `StackBuilder`:

```rust
pub struct MyCustomBackend { /* ... */ }

impl CredentialBackend for MyCustomBackend {
    async fn get(&self, id: &CredentialId) -> Result<StoredCredential, StoreError> { /* ... */ }
    // ... put, delete, list, exists, health_check
}

// Usage:
let stack = StackBuilder::new(MyCustomBackend::new(), key)
    .cache(CacheConfig::default())
    .audit(sink)
    .build();
```

No registration mechanism needed — the engine wraps it directly.

### 12.2 Feature flag strategy for optional backends

**`nebula-credential-storage` feature flags:**

```toml
# nebula-credential-storage/Cargo.toml
[features]
default = ["sqlite"]
sqlite = ["dep:rusqlite"]
postgres = ["dep:sqlx"]
```

**`nebula-credential` core feature flags [ADDED v1.6]:**

```toml
# nebula-credential/Cargo.toml
[features]
default = ["events", "cache", "audit"]
events  = ["dep:nebula-eventbus"]   # CredentialEvent emission; disable for embedded use
cache   = ["dep:moka"]              # CacheLayer + DecryptedCacheLayer
audit   = []                        # AuditLayer + AuditSink (no extra dep, gates inclusion)
minimal = []                        # Alias disabling events + cache + audit.
                                    # Retains: Credential, EncryptionLayer, InMemoryStore.
                                    # NOT for multi-tenant production.
```

Phase 3 backends in separate crates to avoid heavy SDK deps:
- `nebula-credential-vault` — third-party community crate, evaluate independently before adoption
- `nebula-credential-aws` — `aws-sdk-secretsmanager` (120-150 transitive crates)

CI matrix: `default`, `--no-default-features`, `--features postgres`, `--all-features`.

### 12.3 Custom AuthScheme as CredentialState [AMENDED v1.1]

When a credential's `State` and `Scheme` are the same type (static credentials),
the type must implement both `AuthScheme` and `CredentialState`. The
`identity_state!` macro generates the `CredentialState` impl:

```rust
use nebula_credential::identity_state;
use nebula_core::AuthScheme;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MyCustomAuth { pub token: String }

impl AuthScheme for MyCustomAuth {
    const KIND: &'static str = "my_custom";
}

// Generates CredentialState impl: KIND = "my_custom", VERSION = 1
identity_state!(MyCustomAuth, "my_custom", 1);
```

> `identity_state!` is exported via `#[macro_export]`. Required when using
> `#[derive(Credential)]` with `State = Scheme`. Already called for built-in
> schemes (`BearerToken`, `BasicAuth`, etc.) — plugin authors only need this
> for CUSTOM scheme types.

### 12.4 Structured CertificateAuth [v1.4]

Replaces flat PEM string with structured certificate fields (Tomasz, conference):

```rust
pub struct CertificateAuth {
    pub leaf: String,                         // DER-encoded, base64
    pub chain: Vec<String>,                   // intermediates, leaf-to-root order
    #[serde(with = "serde_secret")]
    pub private_key: SecretString,
    pub not_after: DateTime<Utc>,             // enables expires_at() auto-refresh
    pub fingerprint: String,                  // SHA-256
}

impl AuthScheme for CertificateAuth {
    const KIND: &'static str = "certificate";
    fn expires_at(&self) -> Option<DateTime<Utc>> { Some(self.not_after) }
}
```

### 12.5 QueryableAuditSink [v1.4]

Optional query extension for compliance (Arjun, conference):

```rust
pub trait QueryableAuditSink: AuditSink {
    fn query(&self, filter: AuditFilter)
        -> impl Future<Output = Result<Vec<CredentialAuditEntry>, AuditQueryError>> + Send;
    fn count(&self, filter: AuditFilter)
        -> impl Future<Output = Result<u64, AuditQueryError>> + Send;
}

pub struct AuditFilter {
    pub credential_id: Option<String>,
    pub caller_id: Option<String>,
    pub operation: Option<AuditOperation>,
    pub time_range: Option<Range<DateTime<Utc>>>,
    pub limit: usize,
    pub offset: usize,
}
```

Trait defined in v1; concrete implementations in v2 (`nebula-credential-storage`).
`AuditLayer` requires `AuditSink`; admin tooling downcasts to `QueryableAuditSink`.

### 12.6 Error Decision Tree [v1.4]

Each error type serves a different audience (Marcus, conference):

| Error Type | Audience | "What do I do?" |
|------------|----------|-----------------|
| `CredentialError` | Credential implementors | Which variant to return from `resolve()`/`refresh()` |
| `StoreError` | Backend implementors | The 4 outcomes of storage ops |
| `ResolveError` | Framework consumers (engine) | Retry vs re-auth vs fatal |
| `SnapshotError` | Action authors | Type mismatch on `project::<S>()` — check scheme kind |
| `RegistryError` | Startup code | Duplicate key or unknown key on registration |
| `ExecutorError` | Interactive flow orchestrators | Timeout, pending store failure |

### 12.7 Registry Introspection [v1.5]

`CredentialRegistry` gains introspection methods for dev tooling (Jordan, conf2):

```rust
impl CredentialRegistry {
    /// Iterate registered credential type keys.
    pub fn kinds(&self) -> impl Iterator<Item = &str>;

    /// Get description for a registered credential type.
    pub fn description(&self, key: &str) -> Option<CredentialDescription>;
}
```

Captured at `register::<C>()` time alongside the projection closure. Enables CLI
tooling to enumerate available credential types and render parameter forms without
knowing concrete types.

### 12.8 DatabaseAuth Extensions [v1.5 — BREAKING]

`DatabaseAuth` gains extensible fields for vendor-specific parameters (Rafael, conf2):

```rust
pub struct DatabaseAuth {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    password: SecretString,
    pub ssl_mode: SslMode,
    pub expires_at: Option<DateTime<Utc>>,               // [v1.3]
    #[serde(default)]
    pub extensions: serde_json::Map<String, Value>,       // [v1.5] vendor-specific
}

impl DatabaseAuth {
    pub fn with_extension(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extensions.insert(key.to_owned(), value.into());
        self
    }
    pub fn extension(&self, key: &str) -> Option<&Value> {
        self.extensions.get(key)
    }
}
```

Use for: Snowflake `warehouse`/`role`/`account`, MongoDB `authSource`/`replicaSet`,
Oracle `serviceName`. Enables migration from Airflow/n8n without custom AuthScheme
per database vendor.

---

## 13. Public API Surface

### 13.1 nebula-credential module tree

```
nebula-credential/src/
├── lib.rs                  — module declarations + root re-exports
├── credential.rs           — Credential trait (3 assoc types, 5 consts, 7 methods)
├── any.rs                  — AnyCredential (object-safe for dependency declaration)
├── state.rs                — CredentialState trait + identity_state! macro
├── phase.rs                — [TARGET] CredentialPhase enum + CredentialStatus
├── context.rs              — CredentialContext + CredentialResolverRef
├── snapshot.rs             — CredentialSnapshot + SnapshotError
├── handle.rs               — CredentialHandle<S> (Arc<ArcSwap<S>>)
├── key.rs                  — CredentialKey newtype
├── description.rs          — CredentialDescription + builder
├── metadata.rs             — CredentialMetadata
├── pending.rs              — PendingState + NoPendingState + PendingToken
├── pending_store.rs        — PendingStateStore + PendingStoreError
├── pending_store_memory.rs — InMemoryPendingStore
├── resolve.rs              — ResolveResult<S,P>, RefreshOutcome, RefreshPolicy, TestResult
├── error.rs                — CredentialError (14 variants), CryptoError, ValidationError
├── events.rs               — CredentialEvent re-export from core
├── store.rs                — CredentialStore trait + StoreError + PutMode + StoredCredential
├── store_memory.rs         — InMemoryStore
├── secret_store.rs         — [v1.4] SecretStore minimal put/get/delete API
├── registry.rs             — CredentialRegistry + RegistryError
├── resolver.rs             — CredentialResolver + ResolveError + ResolveOutcome [v1.4]
├── refresh.rs              — RefreshCoordinator + RefreshAttempt + RefreshConcurrencyConfig [v1.4]
├── executor.rs             — begin_flow, continue_flow, ExecutorError
├── static_protocol.rs      — StaticProtocol (doc(hidden), derive macro internal)
├── access.rs               — [TARGET] CredentialAccessor + CredentialManager traits
├── scheme/                 — 13 AuthScheme types (CertificateAuth structured [v1.4])
├── credentials/            — 5 built-in credential impls
├── layer/
│   ├── encryption.rs       — EncryptionLayer (AES-256-GCM, AAD)
│   ├── cache.rs            — CacheLayer (ciphertext, moka, behind `cache` feature [v1.4])
│   ├── decrypted_cache.rs  — [v1.5] DecryptedCacheLayer (plaintext, opt-in, above EncryptionLayer)
│   ├── audit.rs            — AuditLayer + AuditSink + QueryableAuditSink [v1.4]
│   └── scope.rs            — ScopeLayer + CallerIdentity
├── stack.rs                — [TARGET] StackBuilder + CredentialStack<B>
├── crypto.rs               — EncryptionKey, EncryptedData, AES-256-GCM, serde_base64
├── retry.rs                — RetryPolicy, retry_with_policy (delegates to nebula-resilience)
│   (SecretString + serde_secret moved to nebula-core — re-exported here)
└── rotation/               — #[cfg(feature = "rotation")] module
```

Testing module (behind `test-support`):
```
├── testing/
│   ├── mod.rs
│   ├── fake_backend.rs     — FakeCredentialBackend
│   ├── factories.rs        — test_bearer_token, test_context, make_stored_credential
│   ├── scenario.rs         — CredentialScenario
│   └── macros.rs           — assert_secret_eq!
```

### 13.2 nebula-credential-storage module tree

```
nebula-credential-storage/src/
├── lib.rs           — feature-gated re-exports
├── config.rs        — BackendConfig (serde-tagged enum)
├── any_backend.rs   — AnyBackend enum dispatch
├── resilient.rs     — ResilientBackend<B> wrapper
├── sqlite.rs        — SqliteBackend (#[cfg(feature = "sqlite")])
└── postgres.rs      — PostgresBackend (#[cfg(feature = "postgres")])
```

### 13.3 nebula-credential-macros exports

```
nebula-credential-macros/src/
├── lib.rs
├── credential.rs    — #[derive(Credential)]
└── state.rs         — #[derive(CredentialState)]
```

---

## 14. Open Questions (explicitly deferred)

| # | Question | Deferred Because |
|---|----------|------------------|
| 1 | Envelope encryption (per-credential DEK, KEK wraps DEKs) | v1 single-instance; `key_id` field reserved in `EncryptedData` |
| 2 | Per-tenant encryption key derivation via HKDF | Single key acceptable for alpha |
| 3 | `CredentialHandle::Clone` replacement design | Need to audit all call sites; `Arc<CredentialHandle<S>>` is the direction |
| 4 | `Credential` trait FFI layer for dylib plugins | Phase 3 plugin system; RPITIT not dyn-compatible |
| 5 | Explicit credential sharing grants ("share with user X") | No user demand; containment hierarchy sufficient |
| 6 | Per-workflow credential restriction | Requires workflow-credential binding table |
| 7 | `list_by_owner` method on `CredentialBackend` trait | v1 uses N+1 filter in `ScopeLayer`; push to backend in v2 |
| 8 | TOCTOU fix in `ScopeLayer::put()` | Requires backend-level transactional scope checking |
| 9 | Compile-time tenant proof types | Runtime checks correct; session types in v2 |
| 10 | ChaCha20-Poly1305 as alternative AEAD | Feature-flagged, deferred until platform demand |
| 11 | Rotation module integration with v2 `Credential` trait | Feature-gated; current `RotatableCredential` disconnected |
| 12 | `CredentialDescription` → `CredentialTypeInfo` rename | Breaking, medium priority, deferred to naming pass |
| 13 | `Provider(String)` → `ProviderError { transient, retry_after }` | Highest-priority error change; plugin authors cannot signal retryability |
| 14 | `into_project<S>(self)` consumes snapshot on mismatch | Options: return `(Error, Self)` or add `try_project(&self) -> Option<&S>` |
| 15 | `identity_state!` macro documentation for external devs | Required for custom schemes, not obvious from trait docs |
| 16 | `serde_secret` at discoverable re-export path | Currently at `utils::serde_secret`, custom state authors need it |
| 17 | Re-export `ParameterValues`, `ParameterCollection`, `AuthScheme` | Plugin authors currently need 3 crate deps |
| 18 | OAuth2 customization via pub `oauth2_flow` module | v1.2 — make helpers public for custom OAuth2 providers |
| 19 | `NodeDefinition` credential bindings | v1.2 — engine crate concern, not credential crate |
| 20 | Connection pool ownership for DB backends | v1.2 — backend owns pool, documented in credential-storage |
| 21 | `REFRESH_POLICY` per-instance override | Const is per-type; need optional runtime override in StoredCredential.metadata |
| 22 | Auth failure feedback loop (401 → credential system) | Add `report_failure(id, kind)` to resolver. Interacts with CB — careful design needed |
| 23 | `ScopeResolver` per-request context | `&self` returns single owner; async multi-tenant needs task_local or context parameter |
| 24 | Per-tenant AuditSink routing | Current AuditSink is global; SaaS customers need per-tenant log export |
| 25 | OAuth2State extension field for provider metadata | Add `extra: serde_json::Map` for team_id, enterprise_id, etc. |
| 26 | Eager vs lazy pool drain on credential refresh | DB pools need eager (immediate drain); WebSocket needs reactive (reconnect). Both use CredentialEvent but different variants. |
| 27 | `credential_handle::<S>()` on ActionContext | Streaming tasks need live handle, not snapshot. Opt-in alongside credential_typed(). |
| 28 | `CredentialStore` dyn-compatibility (v1.1) | RPITIT prevents `dyn CredentialStore`. Migrate to `async_trait`, delete `AnyBackend`. Li Wei (conf2) wants tracking issue before v1 ships. |
| 29 | `put_batch()` on `CredentialStore` (v1.1) | After ListFilter/ListPage. Rafael (conf2): 2000 sequential puts painful for migration. |
| 30 | CC8.1 rotation audit gap | Document as known limitation until rotation v2 integrates. Sonja (conf2). |
| 31 | Legacy no-AAD decryption fallback sunset date | Sonja (conf2): indefinite backward compat with weaker crypto is a risk. |
| 32 | `MockCredentialAccessor` in smaller test crate? | Jordan (conf2): test-support pulls full dep tree. Consider `nebula-credential-test` crate. |
| 33 | **Dual-material overlap window for certificate/PSK rotation** [v1.6] | `project()` returns a single `AuthScheme`. Supporting simultaneous old+new material during a rotation handoff (e.g. X.509 cert overlap while device restarts) requires `CredentialSnapshot` to carry `Vec<Box<dyn Any>>`. Propagates into `CredentialAccessor` and `ResourceManager` — non-trivial cross-crate change. Deferred until a cert-rotation use case owner can co-design the protocol. |
| 34 | **`nebula-resilience` as optional dep for `minimal` feature** [v1.6] | `RefreshCoordinator` circuit breaker currently depends on `nebula-resilience`. Determining whether the CB can be inlined (removing the dep) or feature-gated (removing it in `minimal` mode, losing thundering-herd protection) requires auditing the refresh path. Resolving this unlocks a fully minimal embedded build. |

---

## 15. Non-Goals

- **Secret management service** — this is a credential client library, not Vault/KMS
- **Certificate issuance/renewal** — infrastructure concern
- **LDAP/Kerberos authentication flows** — scheme types exist for DI, protocol impls in dedicated crates
- **Multi-region replication** — storage backend concern
- **Credential garbage collection** — engine/scheduler responsibility
- **Enum-based credential taxonomy** — open trait system chosen deliberately
- **Credential UI components** — crate provides `parameters()` and `InteractionRequest` for rendering
- **Full RBAC policy engine** — engine provides `CallerIdentity`; credential checks ownership

---

## 16. Decisions Log (R1–R10)

| # | Resolution |
|---|-----------|
| **R1** | Open trait system. Registry keys on `Credential::KEY`, returns `Result` on duplicate. All `SecretString` fields use `serde_secret`. 5 built-in credentials, 13 AuthScheme types. Plugin: 1 struct + 1 impl + 1 register. |
| **R2** | `CredentialBackend` trait in nebula-credential. Encryption always in core. AES-256-GCM, single KEK v1, `key_id` reserved for v2 envelope. AAD = `credential_id:version`. `Zeroizing<Vec<u8>>` for plaintext buffers. Key: env → keyring → Argon2id. `StackBuilder` enforces encryption. Enum dispatch. v1: InMemory + SQLite. |
| **R3** | `CredentialPhase` (7 states), runtime guards. `generation` (rotation) vs `version` (CAS). 4 triggers via `RefreshTrigger`. CAS retry 1-2x. Scopeguard calls both `notify_waiters` + `complete`. Expired non-refreshable → error. Grace period persisted. |
| **R4** | Pull injection correct. `CredentialSnapshot` in credential, `CredentialAccessor` in action, bridge in runtime. `Box<dyn Any>` necessary. `dynosaur` not needed. Missing: `RuntimeCredentialAccessor` + `CredentialDispatchTable` in runtime. |
| **R5** | `OwnerId` in core replaces 3 representations. `CallerIdentity` trait (renamed). Use/Read split: `CredentialAccessor` vs `CredentialManager`. Scope containment via `ScopeLevel`. Deferred: sharing grants, per-workflow restriction. |
| **R6** | `CredentialEvent` in core (4 variants, id+generation only, NO state). Delete old `CredentialRotationEvent`. `CredentialBinding` in resource. Restrict `CredentialHandle::Clone`. Watchdog fallback for EventBus overflow. |
| **R7** | `test-support` feature: `FakeCredentialBackend`, `CredentialScenario`, factories, `assert_secret_eq!`. `InMemoryBackend` always available. Plugin test without runtime via `Credential::resolve()` + `CredentialContext::new()`. |
| **R8** | `CallerIdentity` struct validated at construction. `ScopeLayer` MUST filter `list()`/`exists()`. Unscoped = admin-only. `CredentialAuditEntry` with `cross_tenant_attempt`. `AuditSink` fail-closed. Shared table with scope columns. |
| **R9** | 14-variant `CredentialError` is TARGET (current: 10 variants). `ProviderError` with `transient` flag is TARGET (current: `Provider(String)`). `RetryAdvice` is NOT deleted — it exists and is used. `Classify` is ALSO implemented. New target variants: `Expired`, `RotationInProgress`, `AccessDenied`, `AuditFailed`, `CircularDependency`. Boundary mapping redacts internals. [AMENDED v1.1] |
| **R10** | `#[derive(Credential)]` + `#[derive(CredentialState)]` in v1. Feature flags: `sqlite` (default), `postgres`. Phase 3 backends in separate crates. Plugin backend = implement `CredentialBackend`. |

---

## Pre-existing Bugs (found during review, not design decisions)

| # | Finding | Severity | Location |
|---|---------|----------|----------|
| B1 | `SecretString::Serialize` redacts → round-trip destroys identity-state credentials | HIGH | `secret_string.rs` |
| B2 | `OAuth2State` stores `access_token`/`refresh_token` as plain `String` | HIGH | `credentials/oauth2.rs` |
| B3 | `RefreshCoordinator` circuit breaker map unbounded (no eviction on delete) | MEDIUM | `refresh.rs` |
| B4 | `CacheLayer::put()` invalidation unverified | MEDIUM | `layer/cache.rs` |
| B5 | `ScopeLayer.list()` / `exists()` pass through without scope filtering | HIGH | `layer/scope.rs` |
| B6 | `verify_owner` fails open for ownerless credentials | CRITICAL | `layer/scope.rs` |
| B7 | `perform_refresh` doesn't retry CAS on `VersionConflict` | HIGH | `resolver.rs` |
| B8 | `complete()` not called if `perform_refresh` panics | HIGH | `resolver.rs` |
| B9 | `CredentialRotationEvent.new_state` leaks credential material | HIGH | `rotation/events.rs` |
| B10 | `InMemoryStore` CAS on missing row creates instead of `NotFound` | CRITICAL | `store_memory.rs` |
| B11 | `CredentialResolver` emits NO events after refresh — resources never learn | HIGH | `resolver.rs` |
| B12 | `StoredCredential` missing `credential_key` field — engine can't dispatch | HIGH | `store.rs` |
| B13 | `CredentialRegistry::project()` returns `Box<dyn Any>`, not `CredentialSnapshot` | MEDIUM | `registry.rs` |
| B14 | No global refresh concurrency limiter — cascading CB opens at scale | HIGH | `refresh.rs` |
| B15 | `DatabaseAuth` missing `expires_at()` — framework can't auto-refresh IAM tokens | HIGH | `scheme/database.rs` |
| B16 | `ActionDependencies::credential()` is singular — can't declare 2+ credentials | HIGH | `action/dependency.rs` |
| B17 | `SshAuthMethod` missing `Certificate` variant — enterprise SSH certs unsupported | MEDIUM | `scheme/ssh.rs` |