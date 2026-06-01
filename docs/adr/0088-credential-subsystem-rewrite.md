---
id: 0088
title: credential-subsystem-rewrite
status: proposed
date: 2026-06-01
supersedes: []
amends:
  - docs/adr/0081-m6-resource-credential-integration.md
  - docs/adr/0087-bind-population-producer-resource-activation.md
superseded_by: []
tags: [credential, protocol, scheme, lifecycle, crate-layering, dx, rewrite, m12]
related:
  - docs/adr/0081-m6-resource-credential-integration.md
  - docs/adr/0084-pre-expiry-credential-refresh-deferred.md
  - docs/adr/0087-bind-population-producer-resource-activation.md
  - docs/adr/0072-nebula-storage-spec16-port-adapter-tenancy.md
  - docs/COMPETITIVE_ANALYSIS.md
  - docs/PRODUCT_CANON.md
  - docs/ROADMAP.md  # M12
---

# 0088. Credential subsystem rewrite — Protocol model, policy-as-data lifecycle, and crate re-layering

## Status note

This is a **type-model + crate-layering rewrite** of the credential subsystem. It
*amends* ADR-0081 (M6 resource↔credential integration) by replacing the credential
**type model** (the `Credential` trait + five capability sub-traits + nine fixed
schemes) while **preserving** the resource-side contract it established (typed slot
fields, `SlotCell`, `&self` refresh/revoke hooks, engine-owned rotation fan-out). It
folds ADR-0087 (bind-population producer) in as the **typed-binding** section here:
bind-population still ships, but the slot↔credential binding becomes type-checked
against the consumer's declared output scheme. Reactive-only refresh (ADR-0084) and
the engine refresh-claim coordinator (ADR-0041) are unchanged.

## Context

The credential subsystem works but its **authoring surface and crate topology are
wrong**. Three pains, each independently confirmed.

### Pain 1 — trait explosion (authoring)

A base `Credential` trait plus five capability sub-traits
(`Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic`), each carrying or
referencing associated types (`Properties`/`Scheme`/`State`/`Pending`). Shipping one
concrete credential needs many hand-written impls, and `#[derive(Credential)]` in
`properties` mode emits a `todo!()` resolver the author must override in a separate
block. Capabilities are declared **twice** (a `capabilities(...)` macro flag *and* the
sub-trait impl, reconciled by a parity assertion).

### Pain 2 — generic soup + four registries (runtime)

`CredentialService<B, PS>` propagates two type parameters and an 11-argument builder
through every composition root. Registering one credential type writes into **four**
tables: `CredentialRegistry` (contract) + `StateProjectionRegistry` (engine) +
`CredentialDispatch` (runtime bool flags) + `DispatchOps` (runtime erased closures) —
where the flags are redundant with closure-presence.

### Pain 3 — duplicated/mis-layered logic across crates (codebase audit, file:line-verified)

- **Tenant-scoping ×3:** `credential-runtime::service` (`load_owned`/`owner_matches`/
  O(N) `list`) + `nebula_tenancy::ScopeLayer` (fuller) + `nebula-api::transport/credential`
  (`CredentialStoreHandle::Layered`). Worse, the owner-id format **diverges** —
  `{org}/{workspace}` (runtime `scope.rs`) vs `{org}:{workspace}` (api
  `owner_id_from_scope`): a latent cross-tenant key-collision bug.
- **Refresh CAS-persist ×2:** engine `resolver::perform_refresh` and runtime
  `ops::refresh`+`service::refresh_inner` both implement "Refreshed → CAS persist /
  Coalesced → re-read / ReauthRequired → flag", because the engine resolver exposes no
  public forced-refresh.
- **Validation pipeline ×2:** runtime `ops::validate` vs api `CredentialSchemaPort`.
- **Capability taxonomy ×2:** api `transport::credential::classify` hardcodes
  oauth2/api_key/basic flags that `RegistryCredentialSchema::flags` already owns.
- **Dead parallel SQL row-model:** `storage::repos/credential.rs` (`CredentialRepo`) +
  `rows/credential.rs` (`CredentialRow` with `encrypted_secret`+`encryption_version`)
  + migrations `0008`/`0017` — **zero `impl`s anywhere**; duplicates the live
  `StoredCredential`+`EncryptionLayer` path; its `encryption_version` column duplicates
  the envelope `key_id`.
- **Split-brain stores in api:** CRUD writes `state_kind="api_managed_credential"`;
  OAuth writes `state_kind=OAuth2State::KIND` via a different store handle, with a
  second error-mapper and a raw-store fallback when `credential_service` is `None`.
- **OAuth2 acquisition wired in api** contradicts the generic `resolve`/`continue`
  honest-503 "engine-owned" story — the one live acquisition path sits in the wrong layer.
- **`ValidationError` duplicated:** `nebula-credential::error::ValidationError`
  duplicates the canonical `nebula_error::ValidationError` (the completed
  error-unification has not reached this crate).
- **Mis-layered types:** engine `rotation::{transaction,blue_green,grace_period}` are
  pure `Serialize` data state-machines with zero engine coupling; crypto
  (AES-256-GCM/Argon2id) lives in the contract crate yet is consumed by
  `nebula-storage`; `SecretString`/`RedactedSecret` force a whole-credential-crate dep
  on any crate that just wants a secret type.

### Research verdict (four verified deep-research workflows, 2026-06-01)

The credential/secret field (AWS, Vault, GCP, Azure, n8n, Airbyte, Kong, OpenAPI,
Crossplane, k8s, SPIFFE) is **unanimous** on the modeling axis:

1. **Capabilities are data, not sub-traits.** A real credential (Vault dynamic secret)
   is leased *and* revocable *and* refreshable *and* rotatable *and* expiring **at
   once** — orthogonal capability traits are structurally wrong.
2. **~10 structural categories, not 35 wire schemes.** Per-provider variation
   (GitHub/Slack/Microsoft) is **configuration data over a shared scheme**, never a new
   type (n8n `extends`, OpenAPI `securitySchemes`, Crossplane `source`).
3. **Expiry is three orthogonal cases:** inline `expires_at` (AWS STS/SPIFFE), an
   external renewable `lease` (Vault), and controller-managed declarative TTL (k8s).
4. **A non-generic facade holding `Arc<dyn>` collaborators** (aws-sdk `Client`,
   object_store) is the idiomatic Rust 1.96 shape; `dyn`-AFIT is still nightly, so
   object-safe seams use explicit `-> Pin<Box<dyn Future + Send>>`.
5. **A separate contract crate is justified only with a distinct external consumer**
   (tower-service / axum-core / sqlx-core rule). Credential + runtime do not meet it.

The full digests live in memory (`reference_credential_redesign_research`,
`project_credential_rewrite_plan`); competitive framing in
[`docs/COMPETITIVE_ANALYSIS.md`](../COMPETITIVE_ANALYSIS.md).

## Decision

Rewrite the credential subsystem around **code-per-protocol, config-per-provider,
policy-as-data**, and re-layer the crates so each responsibility has exactly one home.

### D1 — `#[nebula::credential]` attribute macro: one declaration site, compile-safe capabilities, provider-config-as-data

> **Revised 2026-06-01** (supersedes the original pure runtime-gated `Protocol` trait, kept below for the record). A pure policy-as-data `refresh` with a default body would move the "declared a capability but never implemented it" check from **compile time** (`E0046` — the exact property the capability sub-trait split + the removal of `RefreshOutcome::NotSupported` were built to guarantee) to **runtime**. To keep that guarantee *and* kill the authoring boilerplate, capability **code stays compile-gated**, and an **attribute macro is the single declaration site** that **derives the lifecycle policy from which methods the author wrote** — so the policy data can never disagree with the implemented capabilities.

The author writes ONE `impl` block; the macro reads the methods present and generates the
base `Credential` impl, the capability sub-trait impls for the methods supplied
(`Refreshable` / `Revocable` / `Testable` / `Dynamic` / `Interactive`), and a
`CredentialLifecycle::policy()` whose `RefreshStrategy` / `RevokeStrategy` reflect exactly
those methods. Declaring refresh capability without a `refresh` method is impossible — the
macro only emits `RefreshStrategy::RefreshToken` when a `refresh` method is present, so the
compile-time guarantee and the runtime data agree by construction.

```rust
#[nebula::credential(key = "github_oauth", category = RefreshPair)]
impl GithubOAuth {
    type Config = GithubOAuthConfig;   // HasSchema (was "Properties")
    type State  = OAuth2State;         // encrypted-at-rest material
    type Output = OAuth2Token;         // consumer-facing AuthScheme

    async fn acquire(&self, cfg: Self::Config, cx: &CredentialContext)
        -> Result<Acquisition<Self::State>, CredentialError> { /* … */ }
    fn project(state: &Self::State) -> OAuth2Token { /* … */ }

    // presence ⇒ Refreshable impl + policy().refresh = RefreshToken (compile-checked, ONE site):
    async fn refresh(&self, st: &mut Self::State, cx: &CredentialContext)
        -> Result<RefreshOutcome, CredentialError> { /* … */ }
}
```

The bounded `CredentialCategory` enum (the ~10 lifecycle shapes) and the
`CredentialPolicy` / `RefreshStrategy` / `RevokeStrategy` / `LeaseRef` data live in
`nebula_credential::lifecycle` (already shipped). The macro produces a
`CredentialLifecycle` impl returning a `CredentialPolicy` consistent with the methods
present. Per-provider variation stays configuration data over a shared protocol, never a
new type:

#### Superseded original sketch (pure runtime-gated `Protocol`)

```rust
// REJECTED: refresh() with a default body downgrades the E0046 capability guarantee to a
// runtime error. Kept only to document what the macro-derived design above replaces.
pub trait Protocol: Send + Sync + 'static {
    const CATEGORY: CredentialCategory;
    type Config: HasSchema + DeserializeOwned;
    type State:  CredentialState;
    type Output: AuthScheme;
    fn acquire(&self, cfg: &Self::Config, cx: &Cx)
        -> BoxFuture<'_, Result<Acquisition<Self::State>, CredentialError>>;
    fn project(state: &Self::State) -> Self::Output;
    fn policy(state: &Self::State) -> CredentialPolicy;
    fn refresh(&self, state: &mut Self::State, cx: &Cx) -> BoxFuture<'_, RefreshOutcome>;
}
```

Providers are registered as **data**, never a new type:

```rust
registry.provider("github",    OAuth2Config { auth_url, token_url, scopes });
registry.provider("microsoft", OAuth2Config { /* login.microsoftonline.com */ });
```

This generalizes the existing `protocol = TypePath` / `StaticProtocol::build` derive
mode into the **only** path, deleting the `properties`+`todo!()` mode. It dissolves the
"200 types" problem (one `OAuth2` protocol, hundreds of provider configs).

### D2 — lifecycle SHAPE is `CredentialPolicy` data; capability CODE stays compile-gated

```rust
pub struct CredentialPolicy {
    pub category:   CredentialCategory,     // one of the ~10 lifecycle shapes
    pub expires_at: Option<DateTime<Utc>>,  // inline expiry  (AWS STS / SPIFFE)
    pub lease:      Option<LeaseRef>,       // external renewable lease (Vault)
    pub refresh:    RefreshStrategy,
    pub revoke:     RevokeStrategy,         // None | HandleBased | IssueTimePolicy
}
pub enum RefreshStrategy {
    Static,        // valid until revoked
    RefreshToken,  // engine calls the refresh method — OAuth2 w/ refresh_token, Vault renew
    Lease,         // engine lease scheduler renews at N% TTL (Vault/k8s); LeaseRef on the policy
    ReAcquire,     // full roundtrip — STS AssumeRole, SAML/OIDC, OAuth2 w/o refresh
}
```

The engine reads `policy()` from state and drives the matching path. The lifecycle
**shape** (category, `expires_at`, `lease`) is pure data. The strategy fields
(`RefreshStrategy` / `RevokeStrategy`) are **derived by the macro from which capability
methods the author wrote** (D1) — not hand-declared — so they cannot disagree with the
compile-gated capability impls: declaring `RefreshToken` without a `refresh` method is
unrepresentable, preserving the `E0046` guarantee while still surfacing capability as
runtime data. One `OAuth2` protocol covers both refreshable (`refresh_token` present) and
re-acquire (absent). `RevokeStrategy` distinguishes handle-based revoke (Vault) from
issue-time-policy revoke (AWS STS), so the model stops assuming a uniform revoke endpoint.

### D3 — one registry (collapse four into one)

A single KEY-keyed registry in `nebula-credential` holds, per credential type, the
boxed `Protocol` object (which itself carries `acquire`/`project`/`policy`/`refresh` +
its own state codec `KIND`/`VERSION` + `SCHEME`). Capability = **closure/strategy
presence**, not a parallel flag table. `register::<P>()` is the single registrar;
the parallel capability/projection tables (`StateProjectionRegistry`,
`CredentialDispatch`) are deleted. `DispatchOps`'s **capability role** is deleted
too — but see the implementation note: its async *operation*-closure storage is
retained (it is generic over the store/pending types and cannot fold into the
non-generic Core registry).

> **Implemented 2026-06-01 (registry collapse).** Two of the four tables were
> deleted and capability now reads solely from the `CredentialRegistry`
> `Capabilities` bitflag (computed at `register::<C>()` from sub-trait
> membership):
>
> - `CredentialDispatch` (runtime, three bool flags) — deleted; the flags
>   mirrored the bitflag. `CredentialService` reads
>   `registry.is_refreshable/testable/revocable(key)`.
> - `StateProjectionRegistry` (engine, state-KIND → `project` closure) —
>   deleted. It had **zero production callers**: the resolver is generic over
>   `C` and calls `C::project` directly, so the type-erased lookup was
>   vestigial. Its fatal-duplicate-KIND check was also unsound — state KIND is
>   not unique (API-key and bearer-token both project `SecretToken`), so the
>   N7 supply-chain defense correctly stays at KEY granularity
>   (`CredentialRegistry` dup-KEY fatal, §15.6).
> - The test-only `register_credential_complete` registrar is deleted;
>   registration goes straight through `CredentialRegistry::register`.
>
> `DispatchOps<B,PS>` is **retained** as the type-erased async
> *operation*-closure table — "delete `DispatchOps`" is read as "delete its
> capability role"; the async ops cannot fold into the non-generic Core
> registry because they are generic over the store / pending-store types. Net:
> four tables → `CredentialRegistry` (the one capability+metadata source) +
> `DispatchOps<B,PS>` (operation closures).

### D4 — non-generic facade; merge `credential-runtime` into `nebula-credential`

`CredentialService` becomes a **non-generic concrete struct** holding `Arc<dyn>`
collaborators (backend / pending / cache / crypto / registry), erased at construction,
built by a `bon`-typestate `builder()` (replacing the 11-arg constructor). The backend
seams stay object-safe via explicit `-> Pin<Box<dyn Future + Send>>`.

`nebula-credential-runtime` is **merged into `nebula-credential`** as its
`service`/`facade` module. It is ~80% facade-glue plus a duplicated registry and a
duplicated tenant-scope; a separate crate buys no layering benefit (`nebula-credential`
is already shared infra importable from Exec/Business/API). **The merge must land
together with D3 (registry collapse) and D7 (scope dedup)** — merging the crate as-is
would only relocate the duplication.

### D5 — consumer side preserved; bind to the output scheme

Action and resource **authoring barely changes** — the consumer ergonomics are already
right. The one change: a credential field binds to the **output scheme** (the auth
shape it consumes), not to a concrete credential/protocol type. A consumer that wants a
bearer token does not care whether it came from OAuth2 or a static PAT.

```rust
// Action — eager snapshot, or CredentialRef<_> for lazy/long-running (re-resolves fresh)
#[derive(Action)]
#[action(key = "slack.send", input = SlackInput, output = SlackOutput)]
struct SlackSender {
    #[credential(key = "slack")]
    token: CredentialGuard<BearerToken>,        // was CredentialGuard<SlackBotToken>
}

// Resource — unchanged: SlotCell + engine-hot-swapped guard + reactive hooks
#[derive(Resource)]
#[resource(key = "postgres")]
struct PgPool {
    #[credential(key = "db")]
    db: SlotCell<CredentialGuard<ConnectionSecret>>,
}
impl Resource for PgPool {
    async fn create(&self, cfg: &PoolConfig, ctx: &ResourceContext) -> Result<Pool, PoolError> {
        Pool::connect(self.db_slot().ok_or(PoolError::NoCred)?.as_ref(), cfg).await
    }
    async fn on_credential_refresh(&self, _slot: &str, rt: &Pool) -> Result<(), PoolError> {
        rt.rebuild_from(self.db_slot().unwrap().as_ref()).await    // blue-green
    }
}
```

`CredentialGuard` (Deref + zeroize-on-drop + `!Clone` + `!Serialize`), `CredentialRef`
(lazy resolve), `SlotCell` (lock-free ArcSwap, generation-versioned), the
refresh/revoke hooks, the epoch fold, and the `CredentialAccessor` per-action allowlist
are all **kept**. `Protocol`/`CredentialPolicy`/`RefreshStrategy` are never visible to a
consumer — the lifecycle is fully encapsulated engine-side, so a Vault-leased secret and
a static PAT look identical (`CredentialGuard<BearerToken>`); only the refresh cadence
differs. As a follow-on, the resource derive's restriction to exactly
`SlotCell<CredentialGuard<C>>` (rejecting `Option<…>`/`Lazy<…>`) is lifted.

### D6 — typed bind-population (folds in ADR-0087)

Bind-population still ships (the producer remains in `nebula-engine`, Exec). It becomes
**type-checked**: the workflow node binds `slot_name → credential_id`; at activation the
engine validates the binding (`ValidatedCredentialBinding`, tenant-fingerprint sealed),
runs `Protocol::acquire`/`refresh`/`project`, and **verifies the resolved `Output`
matches the consumer's declared scheme type** before `slot.store(guard)` / making it
resolvable. Today the slot↔credential link is a bare string KEY with no shape check.

```text
declare (consumer):  #[credential(key="db")] db: SlotCell<CredentialGuard<ConnectionSecret>>
bind     (workflow): node.slot_bindings { "db" -> credential_id "pg-prod-7" }
resolve  (engine):   validate_binding(scope,id) → Protocol.acquire/refresh → project
                     → assert Output == ConnectionSecret → slot.store(guard)  (before create)
```

### D7 — crate / layer allocation

| Layer | Crate | Credential responsibility |
|---|---|---|
| Cross-cutting | **`nebula-crypto`** (NEW) | AES-256-GCM + Argon2id + `EncryptedData`/`key_id` envelope + `CryptoError`. Extracted from the contract crate (storage already reaches for it; drops aes-gcm/argon2/subtle off the contract). PKCE/state helpers travel with the OAuth protocol, not generic crypto. |
| Cross-cutting | `nebula-core` (or small secret crate) | `SecretString`/`RedactedSecret`/`SecretFreeMessage` — plain secret types consumed widely. `Guard`/`TypedGuard` base already here. |
| Cross-cutting | `nebula-error` | Delete credential-local `ValidationError`; route onto the canonical type (folds into the completed error-unification). |
| Core | **`nebula-credential`** | `Protocol` + `CredentialScheme` + `CredentialPolicy`/`RefreshStrategy` + `CredentialState` + scheme types + `CredentialError` (domain) + events + guards + **one registry** + rotation **data** types (moved from engine) + executor + capability dispatch + state projection + token transport (reqwest behind a feature) + the **facade module** (merged runtime). |
| Exec | `nebula-storage` | `EncryptionLayer`/`CacheLayer`/`AuditLayer` decorators + `KeyProvider` + `RefreshClaimRepo` port-adapter (the one correctly port-shaped concern — the template). **Delete the dead `CredentialRepo`/`CredentialRow`/migrations 0008+0017.** |
| Exec | `nebula-engine` | Orchestration only: `RefreshCoordinator` (L1/L2 coalesce), reclaim/sentinel, lease scheduler, `ResourceFanout`. **Expose a public forced-refresh** so the facade stops re-implementing CAS. Delete the `#[deprecated]` String-id L1 surface. |
| Business | `nebula-tenancy` | `ScopeLayer` becomes **the** single tenant-scope enforcement point (composed into the layered store). Delete the runtime + api copies; fix the `{org}/{ws}`↔`{org}:{ws}` format drift. |
| API | `nebula-api` | Thin edge only (handlers/dto/extractors/schema-projection/`CredentialSchemaPort` trait). **Move OAuth2 acquisition into `nebula-credential`** (engine-dispatched `acquire`/continue). One persistence path through the typed facade — delete the raw-store fallback, the split-brain second store, and the `classify` taxonomy dup. Relocate shared OAuth ceremony (flow/discovery/userinfo/state-signing) into one module consumed by both planes. |

## Layering (binding rules)

- The `Protocol` registry, scheme types, policy types, and rotation **data** live in
  `nebula-credential` (Core/shared-infra). The producer that *drives* them
  (bind-population, refresh-coordination, lease scheduling, fan-out) lives in
  `nebula-engine` (Exec) — Exec orchestrates, shared-infra resolves, Business reacts.
- The resolver/refresh code must **not** sit in `nebula-resource` (would invert the
  dependency). Resource stays a pure consumer of already-resolved guards (ADR-0081).
- `nebula-crypto` is leaf cross-cutting (depends only on `nebula-error` for `Classify`);
  `nebula-storage` and `nebula-credential` consume it downward.
- the `deny.toml` `wrappers = [...]` allowlists (inside the `[bans].deny` entries —
  there is no `[wrappers]` table) are updated to the new consumer set; the merge of
  `credential-runtime` removes one crate from the graph.

## Alternatives considered

- **Attribute-macro-on-impl with capability inferred from methods present** (the
  earlier proposal): better than five sub-traits, but still *method-centric*. Rejected
  in favor of policy-as-data because real credentials hold several capabilities at once
  and the capability is a property of the *state/strategy*, not the type.
- **Keep five sub-traits but strip their associated types** (thin markers): preserves
  compile-time capability gating, but keeps the double-declaration and does not solve
  the Vault "all-at-once" case; the registry still needs runtime capability data.
- **Type-state capability gating (oauth2 5.x style):** excellent for a builder, but does
  not compose with a runtime/`dyn` registry keyed by string (plugins, dynamic dispatch).
- **Keep `credential-runtime` as a separate crate:** rejected — no distinct external
  consumer per the tower/axum/sqlx rule; it costs a fourth registry and a third scope copy.
- **Revive the SQL `CredentialRow` model:** rejected — it is a dead parallel schema; the
  live `StoredCredential` + `EncryptionLayer` path is the one to implement durable
  backends against.

## Scope / non-goals

- **Out (ADR-0084):** proactive pre-expiry refresh stays deferred to 1.1. This rewrite
  wires only the reactive path.
- **Out:** the first-party external Vault/AWS-SM provider backend (the `Protocol` model
  accommodates it via a `Leased`/`FederatedExchange` protocol, but wiring is separate).
- **Preserved (not redesigned):** resource `SlotCell`/hooks/fan-out (ADR-0067/0081),
  engine refresh-claim coordinator (ADR-0041), storage spec-16 ports (ADR-0072).
- **Not a scope grab:** the in-flight error-unification branch
  (`refactor/error-unify-validation`) lands first or merges cleanly; D7's
  `ValidationError` removal builds on it.

## Consequences

- One trait + one enum + one registry replace one trait + five sub-traits + four
  registries; `#[derive]` never emits `todo!()`; capabilities can no longer be declared
  inconsistently.
- Adding a provider becomes a config record, not a type — the connector-breadth path.
- Tenant-scoping has one enforcement point; the latent `{org}/{ws}` cross-tenant bug is
  fixed; the dead SQL model and the api split-brain/raw-store fallback are deleted.
- The credential subsystem shrinks by one crate (runtime merged) and gains one
  cross-cutting crate (`nebula-crypto`); `nebula-credential` no longer pulls aes/argon2.
- Consumers (actions/resources) get provider-decoupled, type-checked bindings with
  near-zero migration of their own code.
- This is a **hard breaking change** across credential/runtime/engine/storage/api/
  tenancy and the derive macros. It is spec-correct and expand-contract-migratable.

## Migration sequence (expand-contract, whole-workspace-green per commit)

1. Cross-cutting prep: extract `nebula-crypto`; move `SecretString`/`RedactedSecret`;
   land `ValidationError` removal on the error-unification base.
2. `Protocol` + `CredentialScheme` + `CredentialPolicy`/`RefreshStrategy` contract in
   `nebula-credential`; generalize the `protocol` derive mode; keep old `Credential`
   path compiling until consumers migrate.
3. Collapse the four registries into one; migrate registration.
4. Merge the facade (`credential-runtime` → `nebula-credential`); compose
   `nebula_tenancy::ScopeLayer` as the sole scope point; delete the runtime/api scope
   copies.
5. Engine trim: public forced-refresh; move rotation data types down; delete deprecated
   String-id L1.
6. Api thin-edge: route everything through the typed facade; move OAuth2 acquisition into
   the credential crate; delete the split-brain store + raw fallback + `classify` dup.
7. Delete the dead SQL `CredentialRepo`/`CredentialRow`/migrations.
8. Delete the old `Credential` trait + five sub-traits + four-registry remnants last.

## References

- ADR-0081 — M6 resource & credential integration (amended: credential type-model
  replaced, resource contract preserved).
- ADR-0087 — bind-population producer (folded in as typed bind-population, D6).
- ADR-0084 — pre-expiry refresh deferred to 1.1 (reactive-only boundary).
- ADR-0041 — durable refresh-claim coordinator (unchanged).
- ADR-0072 — storage spec-16 port/adapter/tenancy (storage layer unchanged).
- `docs/COMPETITIVE_ANALYSIS.md` — active credential lifecycle as the empty-niche moat.
- PRODUCT_CANON §12.5 (no local persist of externally-resolved secrets), §12.7 (no
  orphan modules), §13.2 (no silent strand).
- Research digests: memory `reference_credential_redesign_research`,
  `project_credential_rewrite_plan` (verified duplication map + version pins).
