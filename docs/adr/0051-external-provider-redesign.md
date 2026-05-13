# ADR-0051: External Provider redesign — dyn-safe future, resolution envelope, error-discriminated chain

**Status:** Accepted (2026-05-12)
**Tags:** credential, provider, vault, secrets-management, dx, breaking

## Context

`nebula_credential::provider::ExternalProvider` was introduced as a placeholder
contract for delegating credential resolution to external secret managers
(HashiCorp Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault,
Infisical, Doppler, OS keyring). At the time of this ADR the trait has **zero
implementors and zero `dyn`/`Arc<dyn>` usages** across the workspace — it is a
designed-but-not-wired contract, which makes this a free moment to align its
shape with three production references studied 2026-05-12:

- `aws-credential-types` (smithy-rs) — `ProvideCredentials` + `NowOrLater` future
  newtype, `Identity` envelope, error-discriminated `CredentialsProviderChain`.
- `keyring-rs` — sync trait + dyn dispatch via `Arc<dyn Trait + Send + Sync>`.
- `oauth2-rs` — `TokenResponse` trait carrying `expires_in` (no `expires_at`),
  caller-managed refresh-token rotation.

The current shape carried three smells against the rest of the crate:

1. **`#[async_trait]` is the lone holdout.** `CredentialStore`, `Credential`,
   `Refreshable`, and every other async trait in `nebula-credential` already use
   `impl Future + Send` (RTPIT) — see `crates/credential/src/store.rs:137-182`
   and `crates/credential/src/contract/refreshable.rs:87-93`. With the
   workspace pinned to Rust 1.95 (`rust-toolchain.toml`), edition 2024 and
   stable RPITIT, the `async-trait` macro adds no capability the language
   doesn't already provide; it only adds a per-call `Box<dyn Future>`
   allocation, a transitive proc-macro dep, and inconsistency with the rest
   of the crate. Aligning `ExternalProvider` with the established idiom is a
   small consistency win.
2. **Return type is `SecretString`.** No way to express Vault leased secrets
   (lease id + TTL), no way for a downstream cache layer to honour a
   provider-suggested expiry. AWS solved this with the `Identity` envelope.
3. **No chain primitive.** Each downstream would re-roll its own fallback logic
   (env → vault → AWS SM), with the usual risk of masking `Unavailable` errors
   under a blanket "try next on any Err".

## Decision

1. **`ExternalProvider` becomes RTPIT + dyn-safe via `ProviderFuture<'a>`
   newtype** (AWS `NowOrLater` pattern). The trait returns a hand-rolled
   `Pin<Box<dyn Future<Output = …> + Send + 'a>>` envelope with a
   `ProviderFuture::ready(value)` ctor that **skips the box allocation** for
   synchronous providers (env-var, in-memory). Trait stays dyn-safe → `Arc<dyn
   ExternalProvider>` keeps working.

2. **`ProviderResolution` envelope replaces `SecretString` as the return type.**
   Shape (all fields `#[non_exhaustive]`-friendly):

   ```rust
   pub struct ProviderResolution {
       pub secret: SecretString,
       pub lease: Option<LeaseHandle>,
       pub ttl: Option<Duration>,
   }
   ```

   `ProviderResolution::from_secret(SecretString)` shortcut keeps static
   providers terse. Future fields (e.g. `properties: TypeMap` per AWS Identity
   sidecar map) are additive; `#[non_exhaustive]` is enforced.

3. **`ExternalProviderChain` with error-discriminated fallback** (AWS pattern).
   `.first_try(name, p).or_else(name, p)…` builder; the chain itself impls
   `ExternalProvider` (Liskov — composable). Dispatch loop is documented as
   contract:

   - `Ok(_)` → return.
   - `Err(ProviderError::NotFound { .. })` → debug-log + try next.
   - Any other `Err` → **short-circuit** (no masking of `Unavailable` /
     `AccessDenied` / `Backend` by a later provider).

   Tracing wraps each step in
   `debug_span!("provider_chain", provider = %name)`.

4. **Cache layer lives in `nebula-storage`, not here** (ADR-0032). A
   `ProviderCacheLayer` wrapper around `ExternalProviderChain` will land in a
   follow-up, using `tokio::sync::OnceCell` per cache key for single-flight
   (AWS `ExpiringCache` pattern). `ProviderResolution::ttl` is the contract
   that makes that layer possible without further trait changes.

5. **No `fallback_on_interrupt`-style pre-staged fallback secret** (AWS pattern
   explicitly **rejected** for Nebula). Silent fallback to a stale or
   anonymous secret is a compliance violation; surface
   `ProviderError::Unavailable` and rely on the audit-sink subscriber, not a
   silent return path.

6. **`async-trait` workspace dep is dropped from `crates/credential/Cargo.toml`.**
   No remaining user inside the crate after this change.

### Non-decisions

- `LeasedProvider` sub-trait (renew / revoke) is **deferred** until the first
  lease-aware provider implementation lands — the `LeaseHandle` data type
  ships now so resolutions can carry lease metadata without requiring trait
  support for renewal yet.
- `properties: TypeMap` sidecar on `ProviderResolution` is **deferred** until a
  concrete consumer is identified (e.g. tracing IDs from Vault headers).

## Consequences

- **Breaking change**, but with **zero consumers** to migrate today. The
  `ExternalProvider` trait was not yet wired into `nebula-engine`, no
  downstream crate constructed `Arc<dyn ExternalProvider>`, and no test or
  example referenced `ProviderError::resolve`.
- **API surface grows by 4 types** (`ProviderFuture`, `ProviderResolution`,
  `LeaseHandle`, `ExternalProviderChain`) and **shrinks by 1 transitive dep**
  (`async-trait`). Net: surface +4, deps -1.
- The cache-layer follow-up (M-future, `nebula-storage::credential::provider_cache`)
  becomes a pure additive change in another crate — no further changes to this
  trait surface needed.

## References

- AWS reference: `smithy-lang/smithy-rs` `aws/rust-runtime/aws-credential-types/src/provider.rs`
  (`ProvideCredentials` trait + `NowOrLater`) and `aws-config/src/meta/credentials/chain.rs`
  (`CredentialsProviderChain::or_else` error-discrimination loop).
- keyring-rs reference: `open-source-cooperative/keyring-core` `src/api.rs`
  (sync `CredentialApi` trait — informational; Nebula stays async because
  external providers do real I/O).
- oauth2-rs reference: `ramosbugs/oauth2-rs` `src/token.rs`
  (`TokenResponse` trait with `expires_in: Option<Duration>` — caller-side
  expiry semantics, mirrored here in `ProviderResolution::ttl`).
- Cross-crate placement: ADR-0032 (encryption/cache/audit/scope layers live in
  `nebula-storage`, not `nebula-credential`).
- Idiom precedent in this crate: `CredentialStore` (`crates/credential/src/store.rs:137-182`)
  and `Refreshable::refresh` (`crates/credential/src/contract/refreshable.rs:87-93`)
  both return `impl Future + Send` rather than using `async-trait` — this ADR
  brings `ExternalProvider` in line. Rust 1.95 + edition 2024 (per
  `rust-toolchain.toml`) makes `async-trait` redundant on contracts that do
  not require dyn-safe async methods; `ProviderFuture<'a>` covers the
  dyn-safe case without the macro.

## Update — 2026-05-13

The `LeasedProvider` sub-trait deferral noted under **Non-decisions** is
**resolved**. `LeasedProvider: ExternalProvider` (with dyn-safe `renew` /
`revoke` returning `ProviderFuture<'a>`) lands in
`crates/credential/src/provider/leased.rs`, sibling to the existing
`Refreshable` capability sub-trait (Tech Spec §15.4 pattern).

Capability discovery is done through a defaulted
`ExternalProvider::lease_renewal() -> Option<&dyn LeasedProvider>` on the
base trait. Leased backends override it to return `Some(self)`; composed
providers act as **dispatchers** rather than transparently exposing the
first inner — `ExternalProviderChain` and `nebula-storage`'s
`ProviderCacheLayer` (added in PR #664, Phase A of this ADR's follow-up
plan) both impl `LeasedProvider` themselves and route lifecycle calls
through:

1. **`LeaseHandle::provider`** — new `Cow<'static, str>` attribution
   field. The issuing provider stamps its `provider_name()` here at
   resolve time. `LeaseHandle` is `#[non_exhaustive]`; a `LeaseHandle::new`
   constructor is the canonical public path.
2. **`LeasedProvider::handles_lease(&self, lease) -> bool`** — default
   matches `provider_name()` against `lease.provider`; chain / cache
   override to delegate the decision to children / inner. This is the
   single-source-of-truth for renew/revoke routing through composed
   providers.

`ExternalProviderChain::renew` / `revoke` iterate children, picking the
one whose `handles_lease` returns `true` — multi-leased chains route
correctly, where the naïve "first leased child wins" rule would
misdispatch.

`ProviderCacheLayer::revoke` invalidates every cached entry whose stored
resolution carries the matching `lease_id` before forwarding to the
inner — guarantees that the next `resolve` after revoke does not serve a
now-invalid secret from cache. `renew` does the same after a successful
inner renew so the refreshed lease metadata reaches subsequent resolves.

`ProviderResolution::empty()` is the public no-secret marker — recommended
return value for `LeasedProvider::revoke` success and reused by the
default `health_check` impl.

The first concrete `LeasedProvider` implementation is still pending (Vault
dynamic-secret backend is the expected first consumer); shipping the
trait ahead of an implementor lets the cache layer and chain wiring be
reviewed once rather than as a series of churn-y follow-ups.

## Update — 2026-05-13 (Phase C)

The first concrete `LeasedProvider` implementation lands in a new
Business-tier crate `nebula-credential-vault`
(`crates/credential-vault/`). Sibling to `nebula-credential-builtin`;
HTTP stack is `reqwest` + `rustls` to match the workspace TLS policy
(see `crates/api/Cargo.toml`).

The Vault backend covers both canonical read shapes the design
anticipated:

1. **KV v2 static reads** — `GET /v1/{kv_mount}/data/{path}?version=N`.
   `ProviderResolution::from_secret` with no lease metadata; caching
   requires the consumer to set a non-zero `ProviderCacheConfig::default_ttl`
   (the `ZERO` default is bypass per Phase A).
2. **Dynamic secrets** — opt-in via a `dyn/<rest>` prefix on
   `ExternalReference::path`, routed to `GET /v1/<rest>` and decoded
   into a `ProviderResolution::with_lease(secret, LeaseHandle::new("vault",
   …))`. The lease's `ttl` is the Vault-reported `lease_duration`, which
   `ProviderResolution::with_lease` mirrors into the envelope-level
   `ttl` — so cache eviction tracks the upstream lease lifetime without
   further wiring.

`LeasedProvider::renew` and `revoke` hit `/v1/sys/leases/renew` and
`/v1/sys/leases/revoke` respectively; `renew` returns a metadata-only
resolution because Vault's `/renew` endpoint never echoes the secret
payload. Both methods short-circuit with `ProviderError::NotFound` when
the supplied `LeaseHandle::provider` is not `"vault"` — defence against
hand-built dispatchers that bypass the chain / cache routing.

Error mapping matches the Phase B + cache-layer contract:

| Vault outcome           | `ProviderError`                    |
|-------------------------|------------------------------------|
| HTTP 404                | `NotFound`   (chain fall-through)  |
| HTTP 403                | `AccessDenied`                     |
| HTTP 5xx, transport err | `Unavailable`                      |
| Other 4xx, decode err   | `Backend`                          |

Layer placement is enforced through `deny.toml`: an empty `[wrappers]`
allowlist on `nebula-credential-vault` ensures upper-tier crates wire it
through their composition root rather than via direct deps. The
integration test (`crates/credential-vault/tests/cache_integration.rs`)
proves the cache layer + Vault composition end-to-end — KV v2 caching,
dynamic-secret caching via `lease_duration`, and `cache.revoke`
invalidating the cached entry while forwarding to Vault exactly once.

Phase D (engine-side consumption of the `LeasedProvider` capability —
proactive renew, revoke-on-rotation) remains tracked separately.

## Update — 2026-05-13 (Phase D)

Engine-side consumption of the `LeasedProvider` capability lands as a
new `nebula_engine::credential::lease` subsystem
(`crates/engine/src/credential/lease/`). One background tokio task per
engine owns a min-heap-driven scheduler that proactively renews tracked
leases at 70% of TTL and accepts on-demand revoke through an opaque
[`LeaseToken`](nebula_engine::credential::LeaseToken).

Default policy mirrors HashiCorp Vault Agent guidance: 70% renewal
ratio, bounded backoff `[1s, 2s, 4s, 8s, 16s]`, five-attempt budget.
`ProviderError::NotFound` and `AccessDenied` drop the lease immediately
(the upstream grant is gone, retries are useless); `Unavailable` and
`Backend` retry on the backoff schedule, then drop with `Expired
{ reason: RenewalFailed }`.

Cross-crate signalling uses a new `LeaseEvent` enum at
`crates/credential/src/provider/event.rs` (sibling to `LeaseHandle`),
published on `EventBus<LeaseEvent>` — not folded into `CredentialEvent`
because lease events carry richer payload (lease id, provider name,
new TTL, expiry reason) and a sizeable subset of leases will be
unattributed to a nebula `CredentialId`. Variants cover the full state
machine: `LeaseRenewed`, `LeaseRevoked`, `LeaseRenewalFailed`,
`LeaseRevocationFailed`, `LeaseExpired { reason: RenewalFailed |
NotFoundUpstream | Shutdown }`.

Revoke-on-rotation is wired through
`LeaseLifecycle::revoke_for_credential(id)` — a registry scan that
fires `revoke` on every tracked lease attributed to the credential.
Failed revokes are best-effort: logged, emitted as
`LeaseRevocationFailed`, but **do not block** rotation. The integration
point is documented inline on `RotationTransaction::mark_committed` so
future composition-root work has a clear hook. The orchestrator pattern
is exercised end-to-end in
`crates/engine/tests/credential_lease_lifecycle.rs`.

New `CredentialMetrics` constants (engine-side lease lifecycle counters):

| Metric                                          | Type      | Labels             |
|-------------------------------------------------|-----------|--------------------|
| `nebula.credential.dynamic_lease_renewed_total` | counter   | `outcome`,`provider` |
| `nebula.credential.dynamic_lease_revoked_total` | counter   | `outcome`,`provider` |
| `nebula.credential.dynamic_lease_renew_failed_total` | counter | `reason`,`provider` |
| `nebula.credential.dynamic_lease_active`        | gauge     | (none)             |

### Non-goals

Two follow-ups stay explicitly out of scope of Phase D — flagged here
so future cascade waves do not re-litigate them:

- **Wiring `ExternalProvider::resolve` into `CredentialResolver`.** The
  resolver still operates on `CredentialStore` state today. Phase D
  delivers the lease lifecycle mechanism so the bridge work only has
  to call `LeaseLifecycle::track` at resolve time.
- **Cross-replica lease coordination.** Phase D ships single-replica
  semantics — every engine instance that calls `track` will
  independently try to renew. Multi-replica coordination would gate
  renewal through `RefreshCoordinator`'s L2 claim repo; deferred to a
  separate ADR.

Phase D closes the ADR-0051 follow-up cascade. The cascade's PR chain
is #664 (cache) → #665 (sub-trait + routing) → #666 (Vault impl) →
this PR.
