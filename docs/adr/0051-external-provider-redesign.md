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
