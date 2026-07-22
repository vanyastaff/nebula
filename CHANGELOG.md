# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once a stable release ships. While the workspace is `frontier`, breaking
changes are expected between minor releases — call them out here.

## [Unreleased]

### Fixed

- **Library-first: doc-output collision** — the `nebula-worker` binary target
  and the `nebula-worker` library both emitted `doc/nebula_worker/index.html`,
  so one silently clobbered the other's published documentation. The binary now
  sets `doc = false` (no public API surface; the library carries the docs).
- **Library-first: 72 broken intra-doc links** repaired across the workspace
  (private-item docs), so `cargo doc --document-private-items` is warning-clean.

- **Plane-A OAuth `redirect_uri` missing `/api/v1` prefix (P2,
  surfaced by PR-5 wave-1 Codex review)** — the
  `derive_oauth_redirect_uri` helper in PRs #758 / #759 / #761
  produced `{public_url}/auth/oauth/{provider}/callback` while the
  actual Plane-A router is nested under `/api/v1/` in
  `crates/api/src/domain/mod.rs:170`. In production the IdP would
  have redirected to a 404 (no handler mounted at the un-prefixed
  path). Fixed in PR-5 by adding the `/api/v1` prefix to the
  derivation formula AND updating the spec.md / ADR-0085 / README
  documented examples so the operator-visible redirect_uri matches
  what they need to register with the IdP. No state-row data
  migration needed because no real Plane-A OAuth flow ran on
  affected commits (PR-1..4 implementation gated by
  `ProviderNotConfigured` for any non-configured provider; the
  fix lands before any operator wires real env vars).

### Added

- **Plane-A OAuth composition seam** — `OAuthIdentityRuntime` and the opaque,
  secret-free `OAuthRuntimeBuildError` are re-exported from `nebula-api` for
  composition roots. `OAuthIdentityRuntime::from_config` returns
  `Result<Option<_>, _>`: an empty provider set creates no HTTP client, while a
  declared set creates one runtime for the selected Memory/Postgres backend.
  These are technical server-wiring exports; `nebula-sdk` remains the sole
  supported, branded Rust surface.
- **Library-first hardening pass** — `[package.metadata.docs.rs]
  all-features = true` on 15 feature-gated crates so docs.rs renders the
  complete API; a CI `feature-hygiene` job (`cargo hack --each-feature`, wired
  into the required-jobs gate) plus a `task features` target enforcing that
  each optional feature builds in isolation — per-feature + `--no-default-features`
  + all-default across every workspace member (the standalone-crate / modularity
  promise); and runnable crate-level Quick Start examples on `nebula-credential`
  (zeroizing-secret invariant) and `nebula-resource` (typed retry-classified
  errors).
- **Plane-A OAuth identity providers from operator secrets (ROADMAP
  §M3.1)** — the 1.0 surface contains exactly two reviewed profiles:
  canonical Google OIDC and GitHub.com. Operators supply only
  `API_AUTH_OAUTH_{GOOGLE,GITHUB}_{CLIENT_ID,CLIENT_SECRET}`; endpoints,
  scopes, token-auth policy, and JWKS are runtime-owned and cannot be
  overridden by environment. Microsoft, generic OIDC, GitHub Enterprise
  Server, and operator-supplied JWKS remain parked and fail boot through a
  secret-free configuration error. PostgreSQL and Memory both implement the
  staged callback: atomically consume state, perform provider egress without
  database locks, then atomically finalize local identity state. Migration
  `0029_external_identities.sql` adds the authoritative
  `(provider, subject) -> user_id` link with `ON DELETE CASCADE`.
- **OAuth MFA completion is challenge-based.** An existing linked user with
  MFA enabled receives `202 Accepted` plus an opaque, single-use challenge;
  the callback creates neither a session nor session/CSRF cookies. The
  finalizer records the MFA-required outcome and challenge atomically with its
  identity decision, and `POST /api/v1/auth/login/mfa` consumes the challenge
  to complete login and mint the session.

### Security

- **(breaking) Plane-A session and TOTP authorities are hardened at rest.**
  PostgreSQL sessions now store only a domain-separated SHA-256 digest of the
  256-bit cookie token; migration `0038` intentionally invalidates existing
  sessions. Active and pending TOTP seeds use versioned AES-256-GCM envelopes
  with distinct user/purpose-bound AAD, and promotion decrypts/re-seals rather
  than copying ciphertext. Credential and identity encryption consume one
  atomic `KeyProvider` snapshot, preventing key-id/key-generation races.
  Startup performs bounded, advisory-lock-serialized, crash-resumable live-row
  conversion and fails closed on tamper, unknown keys, or malformed legacy
  seeds. This conversion is not historical erasure: operators must quarantine
  or expire pre-migration backups/WAL/snapshots/replicas, retain old keys until
  every dependent backup expires, or invalidate and re-enroll MFA in strict
  deployments.

- **MFA re-enrollment no longer weakens an active factor.** Starting enrollment
  now writes a separate, ten-minute candidate and leaves the active secret
  envelope / `mfa_enabled` untouched. Confirmation verifies that candidate and promotes it
  through a storage-owned atomic consume-and-install operation; expiry, replay,
  replacement, and concurrent confirmation fail closed. Both enrollment routes
  require a CSRF-protected host-bound session created by primary authentication
  within the previous ten minutes; PAT, JWT, and API-key authority is denied.
- **Secret-bearing HTTP responses have a route-level no-store boundary.** Every
  response, including errors, from the auth and MFA routers, PAT and service-
  account creation, webhook registration, and interactive credential
  resolution now overwrites weaker inner cache policy with
  `Cache-Control: no-store`, `Pragma: no-cache`, and
  `Referrer-Policy: no-referrer`. This defense is independent of the
  idempotency replay allow-list, so adding a handler branch cannot silently
  make one-time authority cacheable.
- **(breaking, security) Webhook provider configuration is no longer an
  authority side channel.** The selected trusted factory now validates its
  complete provider configuration before registration mints a credential or
  writes a trigger/activation row; the default is fail-closed. The built-in
  Generic, Slack, and Stripe factories reject every unsupported non-empty
  `provider_config`, so arbitrary JSON is neither silently ignored nor retained
  in a soft-deleted failure tombstone, and legacy Generic `challenge_token`
  JSON fails closed. Trusted Rust composition can still set a Generic challenge
  through `GenericWebhookAction::with_challenge_token`; its authority now uses
  one shared zeroizing allocation with redacted diagnostics rather than
  cloneable plaintext strings.
- **Plane-A OAuth egress is fixed and connect-time guarded.** One opaque runtime
  now owns the fixed provider profiles, a rustls HTTPS-only client, DNS
  admission, redirects/retries/proxy prohibition, outbound concurrency, and a
  30-second per-operation network deadline; every callback egress stage reuses
  its one original deadline. Google discovery uses a singleflight/cache.
  Literal IPs and all
  DNS answers must be globally routable, and reqwest receives only the exact
  validated addresses. Provider bodies are capped at 256 KiB in zeroizing
  buffers; access tokens remain inside a one-shot opaque capability. Raw
  provider errors cannot cross the fixed RFC 9457 boundary.
- **Plane-A token-endpoint authentication is explicit and singular.** GitHub.com
  uses its fixed `client_secret_post` profile. Google prefers discovered
  `client_secret_basic`, falls back to `client_secret_post`, applies the OIDC
  Basic default when metadata omits the field, and rejects unsupported-only
  metadata. Basic authentication form-encodes each credential component before
  joining with `:` and Base64 encoding. A token request never carries client
  credentials in both the Authorization header and form body.
- **Google ID-token claims are validated on the direct-TLS path.** Google
  requires an ID token and validates its compact shape, RS256 header, pinned
  issuer, exact audience/`azp`, bounded `exp`/`iat`, nonce, `at_hash`, and subject
  equality with userinfo. Local cryptographic signature verification against
  provider JWKS remains deferred: the discovered JWKS URL is policy-validated
  but not fetched, and signature bytes receive syntax/size validation only.
- **OAuth callback traces are query-free.** HTTP request spans record method and
  the matched route template (or fixed `<unmatched>` marker), preserve inbound
  W3C parent context, and never record the raw URI containing one-time `code`
  and `state` values.
- **(breaking, security) Plane-A state is browser-bound.** OAuth start now sets
  a per-flow `Secure; HttpOnly; SameSite=Lax; Path=/` `__Host-` transaction
  cookie, and callback requires its exact version/provider/state binding before
  backend state consumption or provider egress. Accepted bindings are cleared
  on every terminal backend outcome; missing, duplicate, or swapped cookies
  return a fixed 401 without consuming the flow. A request carrying eight
  Nebula OAuth transaction-cookie names is rejected with 429 before state
  creation; this is a request-local cookie bound, not a globally atomic browser
  quota. Independently, each process or PostgreSQL deployment admits at most
  10,000 live OAuth state rows globally. A full or contended admission gate
  fails closed with 429 and does not mint state. Start and
  callback must use the `API_PUBLIC_URL` authority, so reverse proxies must
  preserve the public `Host`. Non-browser clients must migrate to a cookie jar
  that carries the matching start `Set-Cookie` into callback.
- **Provider-error callbacks are terminal without egress.** A bounded callback
  with exactly one `error` (and no `code`) must still pass authority, state, and
  browser-cookie binding. The backend consumes the matching state atomically,
  clears the accepted transaction cookie, performs no token/userinfo request,
  and returns a fixed 401 without surfacing provider text.
- **Verified-email absence is distinct from upstream failure.** After a valid
  provider identity is established, a first-link flow with no policy-acceptable
  verified email returns `EmailNotVerified` (403) and writes no link/session.
  Network failures, non-success provider responses, and malformed identity
  payloads remain the fixed upstream-failure lane (502).
- **OAuth email possession never auto-links accounts.** An existing
  `(provider, subject)` link is authoritative. A first login may create a new
  account only for an unused verified email; collision with an existing local
  account rolls back with `AccountLinkRequired` (409), creates no session, and
  requires a separate authenticated linking flow.
- **(breaking, security) Credential test contracts are payload-free.** Provider
  adapters must replace `TestResult::Failed { reason: String }` with
  `TestResult::Failed { code: TestFailureCode }`; raw provider text must be
  discarded locally. SDK consumers import both types from
  `nebula_sdk::integration::credential::{TestFailureCode, TestResult}`.
  `CredentialService::test` now returns `TestResult` directly and `TestReport`
  is removed. HTTP v1 clients must migrate from the former boolean response to
  the tagged `status` response: `success` carries `message`/`tested_at`, while
  `failed` additionally requires the frozen `CredentialTestFailureCodeV1`.
  Platform-owned messages never interpolate adapter errors; future core
  classifications map to wire code `other`.

### Changed

- **Breaking Plane-A OAuth Rust migration.** `AuthBackend` implementors must
  add `cancel_oauth(provider, state, redirect_uri)`. `OAuthCompletion` is now a
  non-exhaustive enum (`SessionCreated` or `MfaRequired`) rather than a cloneable
  struct, and callback query construction must account for the provider-error
  lane; `OAuthCallbackParams` is now non-exhaustive so future standard callback
  fields can be added without repeating this break. Raw Axum handlers remain a
  technical boundary; supported integrations should consume the HTTP contract
  or `nebula-sdk`, not construct handler DTOs directly.
- **Breaking auth diagnostic hardening.** `Debug` for login/reset/verify/MFA
  DTOs, session records, freshly minted PATs, token-creation responses, and
  service-account key responses and email envelopes/messages now preserves
  type/shape diagnostics while
  redacting passwords, TOTP values, reset/verification/challenge tokens,
  session/CSRF authority, PAT plaintext/hash material, MFA seeds, recipients,
  and message bodies. Secret-bearing authority values are no longer `Clone`:
  the password wrapper, live session, password/MFA outcome, MFA enrollment,
  OAuth start, freshly minted PAT, and one-time token/key responses must be
  moved through their single-owner path.
- **(breaking, security) Configuration and generated-client secret safety.**
  `ApiConfig`, its OAuth credential containers, and `SmtpEmailConfig` are now
  move-only so JWT, API-key, OAuth, and SMTP authority cannot be multiplied by
  a broad configuration clone. Serializing `ApiConfig` also omits static
  `api_keys` entirely. OpenAPI marks freshly generated PATs and service-account
  keys as response-only (`readOnly`) rather than request-only (`writeOnly`), so
  generated clients retain the one-time credential in creation responses
  without offering it as request input.
- **(breaking) Plane-A OAuth transport internals are private runtime state.**
  The former public `transport::oauth::{discovery,flow,http,userinfo}` modules,
  endpoint/config override types, raw HTTP helpers, PKCE internals, and custom-
  cfg bypass surface are removed. Composition roots receive only the opaque
  `OAuthIdentityRuntime` plus its secret-free build error; HTTP integrations
  use the versioned API and supported Rust integrations use `nebula-sdk`.
- **(breaking) Identity persistence contracts carry no reusable plaintext
  authority.** `UserRow::mfa_secret` becomes `mfa_secret_envelope`; storage-port
  user reads return `Arc<UserRow>` and the identity row is move-only with
  redacted diagnostics. Storage `SessionRow::id` becomes `token_digest`, new
  writes use `SessionDraft` plus a separately presented token, and
  `SessionRepo::{create,get,touch,revoke}` accept the presented token at the
  repository boundary. `OAuthStateRepo::create` becomes atomic `admit` with
  closed `Created | AtCapacity | Contended` outcomes; secret-bearing
  `UserRow`, `SessionRow`, `SessionDraft`, and `OAuthStateRow` values are
  move-only.
- **(breaking) Encryption-key providers return atomic generations.**
  `KeyProvider::{current_key,version}` is replaced by one `current() ->
  KeySnapshot`, whose validated key id and `Arc<EncryptionKey>` come from the
  same observation. External providers must synchronize rotation and return a
  new key id whenever key bytes change.
- **(breaking, security) Session bearers are cookie-only.** Successful
  password, MFA, and OAuth login responses no longer serialize `session_id`.
  The bearer exists only in the `Secure; HttpOnly` session cookie, preserving
  its XSS-containment boundary; JSON retains the non-bearer CSRF token needed
  by clients for the double-submit contract.
- **(breaking) Fixed browser-session protocol and Rust auth contract.** The
  former `nebula_session` / `nebula_csrf` cookies become
  `__Host-nebula-session` / `__Host-nebula-csrf`; `CookieConfig` and
  `ApiConfig::cookies` are removed because the runtime now fixes
  `Secure; Path=/; SameSite=Lax`, no `Domain`, a 14-day TTL, and `HttpOnly`
  only on the session cookie. `AuthMethod::Session` becomes
  `Session { authenticated_at }`, and `AuthBackend::get_principal_by_session`
  returns the session-authentication metadata required by fresh-session
  policy. Migration `0038` intentionally invalidates existing sessions while
  replacing persisted raw bearers with lookup digests. Operators must perform
  a coordinated cutover (mixed old/new nodes are unsupported) and users must
  authenticate again; browser clients must discard both legacy cookie names.
- **Webhook signing secrets are one-owner diagnostics.** The one-time
  registration response now redacts its `signing_secret` in `Debug`, is
  non-`Clone`, and describes the generated value as response-only in OpenAPI.
  `HmacSecret`, `WebhookActivationSpec`, and `GenericWebhookAction` are likewise
  move-only; webhook activation handles and endpoint providers redact the
  nonce-bearing capability URL, and the concrete endpoint provider is
  non-`Clone`. Integrations must move these authority-bearing values into the
  trusted factory/runtime rather than retaining broad clones.
- **(breaking) Webhook factory failures are a closed, secret-free contract.**
  `FactoryError::InvalidSpec.reason` and `FactoryError::UnknownKind` now accept
  only `&'static str`, preventing implementations from forwarding operator
  JSON or secret material into logs and problem responses. The unused
  `SecretResolution` variant is removed: authority resolution belongs before
  factory admission, and integrations must map failures to a static provider-
  owned classification.
- **(breaking, security) Idempotency replay is explicit and secret-free.**
  `IdempotencyLayer` now defaults to no replay-safe routes and requires an
  explicit matched-route allow-list. First-party composition opts in only
  authenticated POST contracts without one-time authority; auth/session, PAT,
  service-account, webhook activation, and interactive credential responses
  bypass cache lookup and storage. `Set-Cookie` and `Cache-Control: no-store`
  provide a second response-side veto, and cached-record `Debug` output redacts
  headers, bodies, and fingerprints across API, storage, and storage-port.
- **Breaking metrics vocabulary.** The parked Microsoft Plane-A profile and
  public `auth_oauth_provider::MICROSOFT` label were removed; the closed metric
  provider vocabulary is exactly `google|github` until another authority-bound
  profile is reviewed and implemented.
- **Closed provider wire tokens.** `OAuthProvider` now pins both serde and
  OpenAPI spellings explicitly to `google|github`; generated clients no longer
  receive the mechanical but invalid `git_hub` spelling for GitHub.
- **Release train:** these changes follow the released `0.1.0` frontier and
  contain intentional semver-major findings for pre-1.0 crates. The Unreleased
  train must be versioned as at least `0.2.0` by the release workflow; it must
  not be published again as `0.1.x`.
- **(breaking, security) Plane-A backend injection is runtime-based.**
  `InMemoryAuthBackend::with_oauth_providers` and
  `PgAuthBackend::with_oauth_providers` are replaced by
  `with_oauth_runtime(Arc<OAuthIdentityRuntime>)`. `AuthError` no longer carries
  attacker/provider-controlled OAuth payloads:
  `ProviderNotConfigured { provider }` and `OAuthFailed(String)` are now
  payload-free unit variants with fixed public messages. `AuthError` and
  `OAuthProvider` are now `#[non_exhaustive]`; downstream exhaustive matches
  must add a wildcard arm.
- **(breaking) `Topology<R>` async hooks are RPITIT, not `#[async_trait]`** —
  the five async hooks (`create_entry`, `accept`, `prepare`, `on_release`,
  `dispatch_credential_hook`) now return `impl Future<Output = …> + Send`
  instead of going through `async_trait`'s `Box<dyn Future>` shim. Custom
  topology authors: drop `#[async_trait]` from your `impl Topology<R>` block;
  keep plain `async fn` bodies. The same applies to the provider-hook traits
  the built-in topologies drive — `PoolProvider::recycle` /
  `PoolProvider::prepare` and `BoundedProvider::reset` are also RPITIT; drop
  `#[async_trait]` from those overrides too if you had one.
  `ResidentProvider` has no async hooks (nothing to migrate there).
  `Provider` is unchanged and still uses `#[async_trait]`. Drops three heap
  allocations per cache-hit acquire+release round trip on the default hook path.
- **(breaking) Public error and open taxonomy enums are now `#[non_exhaustive]`**
  for additive semver evolution — 10 error enums (credential, core, schema,
  resource, plugin, metadata) and 22 open outcome/event/state enums (credential
  rotation, resource, action). Closed sets stay exhaustive on purpose:
  protocol-bounded OAuth `GrantType`/`PkceMethod`/`AuthStyle`, security-critical
  `SignaturePolicy`, codegen-driving `SlotKind`, and metric-label-pinned
  `SlotDispatchOutcome`/`RecycleOutcome`.
- **Library-first: docs are now compile-checked** — converted 176 `ignore`d
  doctests to runnable / `no_run` examples across 16 crates (fixing API drift
  the `ignore` had masked); the workspace now has **zero `ignore` Rust doctest
  fences** (proc-macro derive examples are honest `text` pointing at the parent
  crate's runnable example). Hardened the CI doc gate to
  `--document-private-items`.
- **Library-first: tighter public surface** — 31 accidental `pub` items lowered
  to `pub(crate)` with `#![warn(unreachable_pub)]` guards on six crates; README
  crate map synced (orchestrator/worker/plugin-core; SQLite adapter status).
- **`nebula-resource`:** crate documentation scrubbed of plan-IDs, ADR
  numbers, internal issue/PR references, and stale temp-file links.
  Rewrote `docs/README.md` and `docs/topology-reference.md` to the v4
  three-topology surface (`Pooled`, `Resident`, `Bounded` with sealed
  `Cap` typestate). Added `# Errors` / `# Cancellation` / `# Drop` /
  `# Panics` sections to the `Resource` trait lifecycle methods, the
  `ResourceGuard` type, and the `Manager::register` / `acquire_*`
  entry points.
- Renamed runnable examples from `m6_*` to `resource_*`
  (`m6_postgres_pool` → `resource_postgres_pool`,
  `m6_resident_http` → `resource_resident_http`,
  `m6_telegram_multi_workflow` → `resource_telegram_multi_workflow`).
  Workspace `cargo run -p nebula-examples --example …` invocations
  updated accordingly.

### Removed

- **(breaking, security) Raw Plane-A OAuth internals are no longer public.**
  `transport::oauth` and its former `discovery`, `flow`, `http`, and `userinfo`
  modules are crate-private. This removes the raw singleton client, standalone
  URL validators, discovery/userinfo wire values and errors, and the
  test-only discovery bypass from downstream reach. OAuth state/PKCE helpers
  are also private. The in-memory backend encodes replay protection through
  atomic remove-on-consume, while Postgres retains its durable,
  provider-aware atomic consume contract.
- **(breaking) Raw credential persistence access is no longer public.**
  Integrations using `CredentialService::credential_store_handle()` must use
  scoped facade methods instead; `CredentialHead::last_validated_at` exposes
  lifecycle metadata needed by supported callers without granting raw-store
  or write-authority access.
- **(breaking, security) Raw Plane-B OAuth ceremony routes were removed.**
  Clients must start and continue credential acquisition through the universal
  workspace-scoped `/credentials/resolve` and `/credentials/resolve/continue`
  endpoints. Plane-A identity OAuth routes are unchanged. The default
  credential catalog no longer advertises the unfinished `oauth2` adapter.

- `nebula-resource::docs/recovery.md` `WatchdogHandle` /
  `WatchdogConfig` section — these types are not in the public surface.
  Drive `Resource::check()` directly or compose `nebula-resilience`'s
  health-probe layer.

## How to read this file

- **Added** — new public API or capability.
- **Changed** — non-breaking behavior changes, refactors, or documentation
  improvements that may change reader expectations.
- **Deprecated** — public API still present but slated for removal.
- **Removed** — public API gone in this release.
- **Fixed** — bug fixes.
- **Security** — security-relevant fixes.

Per-crate changelogs may appear under `crates/<name>/CHANGELOG.md` once a
crate stabilises. Until then, this workspace-level changelog is the single
source of truth.
