# ADR-0085 — OAuth identity providers from operator secrets

- **Status:** Accepted
- **Date:** 2026-05-27
- **Supersedes:** N/A
- **Superseded by:** N/A
- **Related:** ROADMAP §M3.1; ADR-0081 (m6-resource-credential-integration); ADR-0084 (pre-expiry credential refresh deferred); `openspec/changes/oauth-providers-from-operator-secrets/`

## Context

ROADMAP §M3.1 closure left one residual canon §4.5 honesty gap: the OAuth
identity-login path (`POST /auth/oauth/{provider}/start`,
`GET /auth/oauth/{provider}/callback`) is mounted, advertised in OpenAPI 3.1,
and partially wired through `PgAuthBackend`/`InMemoryAuthBackend`, but
`complete_oauth` at `crates/api/src/domain/auth/backend/pg.rs:1079-1110`
explicitly returns `Err(AuthError::NotImplemented(...))`. The synthetic
`https://nebula.local/...` authorize URL emitted by `start_oauth` is
likewise placeholder.

This ADR resolves the design questions raised by closing that gap. The
full SDD planning trail (explore → proposal → spec → design → tasks plus
three recon waves) lives under
`openspec/changes/oauth-providers-from-operator-secrets/`. The directory
is part of this PR series so the audit chain is reachable from git
history.

**Scope distinction.** Nebula has two non-overlapping OAuth surfaces:

- **Plane A — identity login** (this ADR): operator-supplied OAuth client
  proves "this is Alice" via Google/GitHub/Microsoft/Generic-OIDC, then
  Nebula mints its own session and discards IdP tokens.
- **Plane B — credential OAuth** (out of scope; 1.1): user-stored OAuth
  credential lets a workflow node call Google Drive / GitHub API / etc.
  on the user's behalf. Owned by `nebula-credential` + `Interactive::continue_resolve`.

This ADR governs Plane A only.

## Decision

The 13 decisions below resolve the open questions in the SDD design
phase. Each decision identifier matches its label in
`openspec/changes/oauth-providers-from-operator-secrets/design.md` and
its recon supersede chain (recon-2 / recon-3 / recon-4).

### D-1 — Operator config lives in `ApiConfig::auth.oauth.providers`

Operator IdP-client credentials (`client_id`, `client_secret`, endpoints,
scopes) are **infrastructure config**, not credential rows. They live in
`ApiConfig::auth.oauth.providers` next to `ApiConfig::smtp` and
`ApiConfig::idempotency`, populated from `API_AUTH_OAUTH_<PROVIDER>_*`
env vars. This matches the SMTP precedent (`SmtpEmailConfig` +
`API_SMTP_*`).

**Rejected alternatives:**

- **Name-convention credential lookup** (`CredentialService::get_by_name("oauth2/<provider>")`):
  extends `CredentialService` public surface; magic-string fragility;
  scopes/redirect still need a separate home.
- **New `CredentialKind::OAuth2Provider` variant:** largest blast
  radius; couples M3.1 closure to M12.3 sequencing; requires data
  migration for existing credentials.

Env-managed mode is the only supported config path in 1.0. DB-managed
admin UI (matching n8n's `N8N_SSO_MANAGED_BY_ENV=false` mode) is a 1.1+
extension; Nebula has no admin UI for SSO config yet.

### D-3 (recon-4 revised) — `redirect_uri` auto-derived from `ApiConfig::public_url`

`redirect_uri` is **NOT a configuration field**. It is auto-derived at
runtime as
`format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())`
from the existing `ApiConfig::public_url` (`API_PUBLIC_URL` env, declared
at `crates/api/src/config/mod.rs:107`).

Operators that need multiple callback URIs deploy multiple Nebula
instances (each with its own `API_PUBLIC_URL` and IdP client
registration). Matches n8n's `{instanceBaseUrl}/rest/sso/oidc/callback`
pattern.

**Supersedes the prior design's allow-list `redirect_uris: Vec<String>`
shape.** No redirect-URI allow-list. The original Risk R.6 (single vs.
allow-list) is moot.

### D-5 (recon-4 refined) — `OAuthEndpoints` is a tagged union

```rust
enum OAuthEndpoints {
    /// OIDC provider — endpoints discovered at runtime from
    /// `.well-known/openid-configuration`. Scopes hardcoded
    /// `"openid email profile"`.
    Oidc { discovery_url: String },
    /// OAuth2-only provider (e.g. GitHub) or operator-customized OIDC.
    /// `jwks_url` accepted for forward compat but ignored in 1.0 (see D-16).
    /// `scopes` MUST be non-empty for Manual.
    /// `verified_emails_url` (wave-6 addition for P2 review) is required
    /// for providers like GitHub whose `userinfo_url` returns no
    /// `email_verified` flag — PR-4 fetches it after `userinfo_url` and
    /// picks the primary-and-verified email. For providers that return
    /// `email_verified` in the userinfo response itself (most OIDC),
    /// `verified_emails_url` is `None` and PR-4 reads `email_verified`
    /// from the userinfo body.
    Manual {
        authorize_url: String,
        token_url: String,
        userinfo_url: String,
        verified_emails_url: Option<String>,
        jwks_url: Option<String>,
        scopes: Vec<String>,
    },
}
```

Known-provider defaults ship in `crates/api/src/transport/oauth/known.rs`
for every variant of the live `OAuthProvider` enum at
`crates/api/src/domain/auth/backend/oauth.rs:28-47`. In 1.0 the enum has
three variants:

- `Google` — OIDC default (`discovery_url = "https://accounts.google.com/.well-known/openid-configuration"`). `email_verified` claim is in the userinfo response.
- `Microsoft` — OIDC default (`discovery_url = "https://login.microsoftonline.com/common/v2.0/.well-known/openid-configuration"`). `email_verified` claim is in the userinfo response.
- `GitHub` — Manual default. GitHub.com does NOT publish a `.well-known/openid-configuration` AND its primary userinfo endpoint (`/user`) does NOT return `email_verified`. Per the wave-6 Codex P2 review, the Manual shape must therefore carry a **second userinfo endpoint** for the verified-email lookup. GitHub defaults: `userinfo_url = "https://api.github.com/user"` (returns `sub` = `id` as string), `verified_emails_url = Some("https://api.github.com/user/emails")` (returns `[{ email, primary, verified, ... }]`; PR-4 picks the entry where `primary == true AND verified == true`).

Operator config overrides per-provider (e.g. point Google at a staging
IdP mirror via the `Manual` endpoints arm).

**Extending the enum (Auth0, Okta, generic OIDC, custom OAuth2):**
adding a new known provider requires extending the `OAuthProvider` enum,
its `FromStr`, and its `as_str()`, then registering a default in
`known.rs`. This is a small, mechanical change but **out of 1.0 scope**
for this ADR. A separate 1.1 follow-up plan tracks the enum extension
(allowing `Generic { name: String }` for arbitrary operator-named
providers). The 1.0 chain ships with the existing three variants.

### D-6 — `AuthError::ProviderNotConfigured { provider }` → HTTP 503

A new typed variant for "operator has not configured this provider".
Maps to HTTP 503 `ServiceUnavailable` (the capability is not currently
provisioned). Distinct from `AuthError::OAuthFailed(_)` which maps to
HTTP 502 `UpstreamError` (the IdP failed). Closes a discoverability
gap for API integrators.

The variant lands in the `AuthError → outcome` exhaustive audit added
in #753; a new closed-set `outcome = "provider_not_configured"` label
value joins the `nebula_api_auth_*` metrics family.

### D-7 — IdP tokens discarded after Nebula session is minted

IdP-issued `access_token`, `refresh_token`, and `id_token` are
**not persisted** anywhere — no store, no log, no metric. The OAuth
flow proves identity; Nebula owns the session. Persisting IdP tokens
would tie session lifetime to IdP token lifetime, drag the credential
rotation fan-out (#688/#690) into the session path, and re-open the
"downstream calls using IdP creds" surface (deferred to 1.1).

Rust's borrow checker enforces the discard: `OAuth2Token` (or its local
typed equivalent) is a function-local value in `complete_oauth` that
drops at function exit.

### D-8 — `external_identities` table links `(provider, sub) → user_id`

New PG migration:

```sql
CREATE TABLE external_identities (
    user_id     BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,
    subject     TEXT NOT NULL,
    email       TEXT,
    linked_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX external_identities_user_id_idx ON external_identities (user_id);
```

**Note on `user_id BYTEA`** — matches the existing identity-tables
convention (`0001_users.sql` declares `users.id BYTEA PRIMARY KEY` as a
16-byte ULID; `0002_user_auth.sql`'s `auth_identities`, `sessions`,
`personal_access_tokens`, `verification_tokens` all use `BYTEA` FKs to
`users(id)`). Using `UUID` here would fail FK validation at migration
apply time.

Primary key `(provider, subject)` because the IdP guarantees `sub` is
stable per user inside its tenant. `email` is a snapshot at link-time
for audit; `users.email` is the authoritative source for "user's email".
`ON DELETE CASCADE` so deleting a Nebula user removes the IdP linkage.
No `tenant_scope` column — external identity is global per
`(provider, sub)` because IdPs do not respect Nebula's tenancy.

### D-9-WAVE6 — Anti-SSRF gate applies to ALL server-side OAuth HTTP fetches (🟥 WAVE-6 P1 SECURITY FIX)

**Trigger**: Codex P1 (`6ad880d4` review, 22:51:42Z): the original D-9 + D-12 + D-15 framing only ran `validate_token_endpoint` on the token endpoint URL itself. PR-4's userinfo GET (direct `oauth_token_http_client()` call) and PR-3's OIDC discovery doc GET (`fetch_oidc_discovery`) bypassed the anti-SSRF gate. A hostile or misconfigured discovery document could return `userinfo_url = "http://10.0.0.5/admin"` and Nebula would fetch it server-side. **This is a real SSRF hole that must close before any production OAuth code lands.**

**Decision** (replaces the narrower D-9 framing below):

1. **Rename for clarity**: the existing `flow::validate_token_endpoint` function is renamed (or aliased) to `validate_oauth_outbound_url` to signal that its anti-SSRF policy applies to ANY server-side OAuth-related HTTP call, not just the token endpoint. The implementation is unchanged.

2. **Apply at every server-side fetch site**:
   - **Token endpoint POST** (`flow::exchange_code`) — already validated; no change.
   - **OIDC discovery doc GET** (D-15 `fetch_oidc_discovery`) — validate `discovery_url` BEFORE the GET.
   - **Discovery-returned endpoints** — after parsing the discovery JSON, validate `authorize_url`, `token_url`, `userinfo_url`, and `jwks_url` (if present) BEFORE caching the `OidcDiscovery` value. A hostile discovery doc returning internal-IP URLs fails the cache insert with `DiscoveryError::EndpointSsrfRejected { field, url_host }`.
   - **Userinfo GET** (PR-4 `complete_oauth`) — validate `userinfo_url` BEFORE the GET. For OIDC providers, the URL came from the validated cached discovery (already vetted in step above). For Manual providers, the URL came from operator config (validated at boot per REQ-compose-001).
   - **Verified-emails GET** (wave-6 GitHub addition; see Manual.verified_emails_url) — same gate as userinfo GET.

3. **Test bypass** (wave-5 D-14): `nebula_test_util` cfg gates the bypass for ALL three (now four with verified_emails_url) HTTP call sites uniformly via the `test_support` module helpers.

4. **`oauth_allow_insecure_localhost` flag** (original D-9 narrowed scope; wave-7 refined per Codex F.2): applies ONLY to `authorize_url` (browser-fetched; no server-side SSRF surface). For the implementation, this means **two complementary validator functions**:
   - **Strict**: `validate_oauth_outbound_url(url)` — the generalized rename from `validate_token_endpoint`. HTTPS-only, no localhost/private/loopback. Used for `token_url` / `userinfo_url` / `verified_emails_url` / `jwks_url` / `discovery_url`.
   - **Flag-aware**: `validate_oauth_authorize_url(url, flag, in_release)` (new wave-7 helper in `crates/api/src/transport/oauth/flow.rs`) — wraps the strict gate but accepts `http://localhost(:port)?` when `flag == true AND !in_release`. Used ONLY for `Manual.authorize_url` (and the OIDC-discovered authorize URL when the start_oauth handler emits it).

   Production builds (release profile, no `nebula_test_util` cfg) reject any non-HTTPS or loopback/private URL on server-side fields. Dev / integration env can opt-in to localhost for the redirect step via the flag without weakening the server-side anti-SSRF policy.

**Why this resolves the P1 SSRF**:

- Static analysis of PR-4's flow: every server-side OAuth HTTP call passes through `validate_oauth_outbound_url` before the wire request. No bypass path exists in production.
- Discovery-doc poisoning is closed: even a valid HTTPS discovery URL cannot smuggle internal-IP child URLs through, because the child URLs are re-validated.
- The integration-test bypass surface (`test_support`) is the SAME code path used in production but with the validator stubbed; production builds without the cfg cannot reach the stub.

**Spec impact**: REQ-oauth-003 step 5 (userinfo GET) and REQ-oauth-002 step 3 (resolve OIDC discovery) gain explicit `validate_oauth_outbound_url` calls. REQ-compose-001 validation Invariant 1 carries the same gate for static (config-supplied) URLs.

---

### D-9 (recon-3 narrowed — historical context for D-9-WAVE6 above) — `oauth_allow_insecure_localhost` flag scope

The flag scope is narrower than the original design proposed. Existing
in-tree code at `crates/api/src/transport/oauth/flow.rs::validate_token_endpoint`
strictly rejects `http`, `localhost`, private/loopback IPv4, ULA/link-local
IPv6, and IPv4-mapped equivalents — for **token URLs**. This is an
anti-SSRF defense that MUST stay in 1.0.

The `oauth_allow_insecure_localhost` flag therefore applies ONLY to
`authorize_url` (which the browser fetches; no server-side SSRF surface)
and the now-defunct `redirect_uris` (eliminated by D-3 recon-4). For
`token_url` / `userinfo_url`, the strict policy is non-negotiable. The
flag is rejected at boot when the binary is built with the `release`
feature.

Integration tests against `wiremock` on `127.0.0.1` work via the
`test_support` module gated by `--cfg nebula_test_util` documented in D-14.

### D-11 (recon-3 revised) — Reuse `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo`

`PgAuthBackend::start_oauth` uses the existing in-crate `mint_pkce()`
helper (already generates state + code_verifier + code_challenge),
builds `flow::AuthorizationUriRequest` from the operator config, calls
`flow::build_authorization_uri(req, state, code_challenge)`, and
persists `OAuthStateRow` via the existing `self.oauth_state_repo.create(...)`
(slot already on `PgAuthBackend`).

**This does NOT use `OAuth2Credential::initiate_authorization_code`** —
that helper produces `OAuth2Pending`, which is a Plane B shape persisted
to `pending_credentials` via `PendingStateStore`. Plane A uses the
distinct `OAuthStateRepo` / `plane_a_oauth_states` table.

### D-12 (recon-3 revised) — Use `flow::exchange_code` for token endpoint

`PgAuthBackend::complete_oauth` uses the existing `flow::exchange_code(TokenExchangeRequest)`
helper at `crates/api/src/transport/oauth/flow.rs:79-118`. It wraps
`oauth_token_http_client()` + form-encoded body + `AuthStyle::Header`/`PostBody`
selection + bounded response reading + `validate_token_endpoint`. PR-4
does NOT introduce its own HTTP client.

The userinfo GET reuses `oauth_token_http_client()` directly with a
`Bearer` header.

### D-13 — Plane A does NOT route through `Interactive::continue_resolve`

`Interactive::continue_resolve` on `OAuth2Credential` is designed to
persist the resolved `OAuth2State` as an encrypted credential row.
Plane A (per D-7) discards tokens. Bypassing `continue_resolve` makes
the discard a type-level property: no `OAuth2State` is ever constructed,
encrypted, or persisted.

A future change (1.1 "OAuth-as-stored-credential" surface) will use
`continue_resolve` independently for Plane B. Both paths coexist without
conflict.

### D-14 — `nebula-api` `test_support` module exposes ALL OAuth bypass helpers (🟥 RECON-5 — fixed two architectural defects from PR-757 wave-4 Codex review)

Integration tests against `wiremock` on `127.0.0.1` cannot pass
`validate_token_endpoint`'s strict policy (D-9) and therefore fail on
**all three** server-side IdP HTTP call sites used by Plane A:
1. Token endpoint POST (`flow::exchange_code`).
2. Userinfo endpoint GET (uses `oauth_token_http_client()` directly).
3. OIDC discovery doc GET (D-15 `fetch_oidc_discovery`, also via
   `oauth_token_http_client()`).

**Gate mechanism**: use a **custom `cfg`** named `nebula_test_util` (NOT
a Cargo feature). Tokio-precedent: features are *additive* across the
dep tree — if any crate in a transitive dependency activates `test-util`,
production binaries inherit it silently. Custom `cfg` is process-level
opt-in via `RUSTFLAGS="--cfg nebula_test_util"` and **cannot** be
transitively activated. Matches the `tokio_unstable` pattern (see
[`tokio::main` docs on `tokio_unstable`](https://docs.rs/tokio/latest/tokio/#unstable-features)).

**Module shape**:

```rust
// crates/api/src/lib.rs
#[cfg(nebula_test_util)]
pub mod test_support;

// crates/api/src/test_support.rs
//! Test-only helpers — only compiled with `--cfg nebula_test_util`.
//!
//! Production builds (no `RUSTFLAGS` opt-in) do NOT contain this
//! module, so the helpers cannot leak via transitive feature
//! activation.

pub use crate::transport::oauth::flow::exchange_code_unchecked;

/// Build an OAuth HTTP client without the `validate_token_endpoint`
/// anti-SSRF gate. Test-only — used by wiremock integration tests
/// against `127.0.0.1` listeners.
pub fn oauth_token_http_client_test_unchecked() -> &'static reqwest::Client {
    crate::transport::oauth::http::oauth_token_http_client()
}

/// Fetch OIDC discovery doc without `validate_token_endpoint`.
/// Test-only.
pub async fn fetch_oidc_discovery_unchecked(url: &str)
    -> Result<OidcDiscovery, DiscoveryError> { ... }
```

**Production-build guard**: a compile-time assertion in
`crates/api/src/lib.rs` that fires if the `nebula_test_util` cfg is
active in a release build:

```rust
#[cfg(all(nebula_test_util, not(debug_assertions)))]
compile_error!(
    "nebula_test_util cfg must NOT be active in release builds; \
     remove --cfg nebula_test_util from RUSTFLAGS"
);
```

Note `not(debug_assertions)` is the canonical cfg for release-profile
detection (set by `cargo build --release` and any `[profile.<name>]
debug-assertions = false`). The earlier attempt at `cfg(feature =
"release")` was structurally wrong (release is a Cargo profile, not a
feature). CI parity: `.github/workflows/ci.yml` runs `cargo build
--release --workspace` with empty `RUSTFLAGS` to prove the guard
doesn't fire on the default production build, plus a negative-test job
that runs `RUSTFLAGS="--cfg nebula_test_util" cargo build --release
--workspace` and asserts a non-zero exit with the `compile_error!`
message in stderr.

The `nebula-credential-runtime::test_fixtures` Cargo feature precedent
at `crates/credential-runtime/src/lib.rs:31-35` is **acceptable for
that crate** because its surface is limited; for `nebula-api` the
test-bypass exposes multiple SSRF-sensitive helpers, so custom-cfg
gating is the safer choice.

> **Why this resolves both Codex wave-4 P2 issues**:
> - **D-A.1**: `cfg(feature = "release")` is replaced by
>   `cfg(all(nebula_test_util, not(debug_assertions)))`, which is a
>   real production-detection cfg and structurally cannot be bypassed
>   by Cargo feature unification.
> - **D-A.2**: `test_support` module exposes helpers for all three
>   server-side OAuth HTTP call sites, not just `exchange_code_unchecked`.
>   Integration tests can mount wiremock on `127.0.0.1` and exercise
>   token / userinfo / discovery without bypass code leaking to
>   production.

### D-15-WAVE6 — OIDC discovery doc fetch + process-lifetime cache (🟥 WAVE-6 hardened with post-discovery URL validation)

The original D-15 ran `validate_token_endpoint` only on the `discovery_url` itself (the URL Nebula GETs). The wave-6 P1 SSRF audit (D-9-WAVE6 above) requires that the URLs RETURNED in the discovery JSON (authorize_url / token_url / userinfo_url / jwks_url) are ALSO validated before being cached — a hostile discovery doc could return internal-IP endpoint URLs and bypass the gate.

**Implementation**: `fetch_oidc_discovery(url)` calls `validate_oauth_outbound_url(url)` first (gates the doc fetch itself), then after parsing the JSON response calls `validate_oauth_outbound_url` on each of the returned `authorize_url`, `token_url`, `userinfo_url`, and `jwks_url` (if present). The cache insert is skipped and `DiscoveryError::EndpointSsrfRejected { field: "<token_url|userinfo_url|...>", url_host }` returns to the caller if any child URL fails. No partial cache entries.

Original D-15 text below (still applies to the cache-shape and timing decisions):

### D-15 — OIDC discovery doc fetch + process-lifetime cache

`crates/api/src/transport/oauth/discovery.rs` (new file) exposes
`fetch_oidc_discovery(url) -> Result<OidcDiscovery, DiscoveryError>`
which validates the URL via `validate_token_endpoint`, GETs `.well-known/openid-configuration`
via `oauth_token_http_client()`, parses, and caches the result for the
process lifetime (no TTL; discovery docs are stable per provider;
restart Nebula to refresh).

Discovery failures surface at `start_oauth` call time as
`AuthError::OAuthFailed { cause: "oidc_discovery_failed" }`, not at
boot. The operator may add a provider config that points at an
unreachable discovery URL; boot succeeds, the first OAuth attempt
fails closed with a typed cause.

### D-16 — id_token JWKS signature validation deferred to 1.1

The 1.0 OAuth identity flow does **not** validate the `id_token`
signature against the IdP's JWKS. The userinfo endpoint over TLS is
treated as authoritative for the user's `email` + `sub`. Presence of
`id_token` in the token response is logged (`tracing::debug!(id_token_present=true)`)
but otherwise ignored.

**Why defer:** `jsonwebtoken` is already in workspace deps with the
`rust_crypto` feature, so signature validation alone is ~80 LOC. But
proper handling — JWKS fetch, cache with key rotation, multi-key
selection by `kid`, claim validation (`iss`/`aud`/`exp`/`iat`/`nonce`),
clock-skew tolerance — is ~200 LOC + ongoing maintenance. The
`openidconnect` crate (v4.0.1) is the canonical Rust solution but is
a heavy dep to introduce mid-cycle; 1.1 picks the right shape.

**Why this is safe enough for 1.0:**

- The token endpoint is over TLS, validated by `validate_token_endpoint`
  (anti-SSRF defense in D-9).
- The userinfo endpoint reuses the same HTTP client policy.
- A TLS-chain compromise to the IdP would also break JWKS fetching, so
  signature validation does not provide a defense JWKS-fetch-over-TLS
  doesn't itself rely on.
- The anti-CSRF `state` and PKCE `code_verifier` defenses are intact —
  they're pre-token-endpoint defenses, not id_token concerns.
- The marginal residual risk is a malicious operator-chosen IdP swapping
  the userinfo response; an operator who does not trust their IdP's
  TLS chain has bigger problems.

**Release-notes blurb** (must appear in `crates/api/README.md`
"Known limitations" section per PR-5):

> **OAuth identity login (1.0)**: Nebula ships authorization-code with
> PKCE for the three OAuth providers in the live `OAuthProvider` enum:
> Google + Microsoft (`Oidc`-shaped, via `.well-known/openid-configuration`
> discovery) and GitHub (`Manual`-shaped, with explicit endpoint URLs
> because GitHub.com does not publish a `.well-known/openid-configuration`).
> The IdP's userinfo endpoint over TLS is the authoritative source for
> the user's verified email and stable subject identifier. `id_token`
> signature validation against the IdP's JWKS is **not** performed in 1.0
> — a 1.1 hardening pass will add it via the `openidconnect` crate or
> equivalent. Adding Auth0 / Okta / generic OIDC / custom OAuth2 providers
> requires extending the `OAuthProvider` enum at
> `crates/api/src/domain/auth/backend/oauth.rs:28-47` (small mechanical
> change, 1.1 follow-up). Operators that require strict OIDC compliance
> or non-shipped providers now should track issue #TBD (filed alongside
> the closing PR).

## Superseded sub-decisions (audit trail)

The recon waves invalidated three sub-decisions from the original
design. Documented here so the audit chain is reachable without reading
the recon appendices:

| Sub-decision (historical) | Recon | Reason superseded |
|---|---|---|
| **D-2** — dyn-erase `CredentialServiceErased` for `AppState::credential_service` | recon-2 | `CredentialService` is not consumed by Plane A. Operator IdP-client config is infra config (D-1), not credential rows. No dyn-erase trait introduced. |
| **D-4** — `OAuthProviderCredentialKey` newtype with `pub(crate)` constructor | recon-2 | No credential lookup happens in Plane A. The newtype was scaffolding for a typed-decode seam that is itself superseded (see REQ-cred-001 below). |
| **REQ-cred-001** — `CredentialService::get_for_oauth_provider` typed-decode seam | recon-2 | The operator config carries `client_id`/`client_secret` directly. Decoding from `CredentialSnapshot` is moot when the values are already typed in `ApiConfig`. |
| **D-3 (original)** — `redirect_uris: Vec<String>` allow-list | recon-4 | Replaced by auto-derivation from `ApiConfig::public_url` (D-3 revised above). |
| **D-11 (original)** — Use `OAuth2Credential::initiate_authorization_code` + `pending_state_store` | recon-3 | Wrong store: that helper produces Plane B `OAuth2Pending`. Plane A uses the distinct `OAuthStateRepo` already wired into `PgAuthBackend`. |
| **D-12 (original)** — Generic "use `transport/oauth/http.rs`" | recon-3 | Refined to "use `flow::exchange_code` wrapper specifically" (more precise about the right call site). |

The recon appendices (`recon-2-credential-domain.md`,
`recon-3-flow-and-pending.md`, `recon-4-n8n-and-rust-ecosystem.md`)
preserve the full reasoning. Subsequent ADRs that touch these surfaces
should reference both this ADR and the relevant recon.

## Consequences

**Code paths landing across PR-2 through PR-5:**

- New file `crates/api/src/config/oauth.rs` — `OAuthProvidersConfig`,
  `OAuthProviderConfig`, `OAuthEndpoints` enum (`Oidc` / `Manual`),
  validation, env binding.
- New file `crates/api/src/transport/oauth/discovery.rs` —
  `fetch_oidc_discovery` + process-lifetime cache.
- New file `crates/api/src/transport/oauth/known.rs` — hardcoded
  known-provider defaults.
- New variant `AuthError::ProviderNotConfigured { provider }` and
  `AuthError → ApiError → HTTP 503` mapping.
- New migration `crates/storage/migrations/postgres/00XX_external_identities.sql`
  + `PgExternalIdentityRepo` + `InMemoryExternalIdentityRepo` parallel.
- Rewritten `PgAuthBackend::start_oauth` (real authorize URL via
  `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo`).
- Rewritten `PgAuthBackend::complete_oauth` (real token exchange via
  `flow::exchange_code` + userinfo GET + find-or-create user with
  account-takeover defense + session mint). Removes the `NotImplemented`
  return.
- Symmetric `InMemoryAuthBackend` impl for both methods.
- Trait signature change: `AuthBackend::start_oauth(provider, redirect_uri)`
  (redirect_uri auto-derived by the handler from `ApiConfig::public_url`).
- New `nebula-api` `test_support` module gated by `#[cfg(nebula_test_util)]`
  exposing test-only bypass helpers for all three server-side OAuth
  HTTP call sites (token POST + userinfo GET + OIDC discovery GET);
  custom-cfg opt-in via `RUSTFLAGS="--cfg nebula_test_util"`
  (NOT a Cargo feature, to avoid transitive activation); production
  release builds are guarded by `compile_error!` if the cfg is set.

**Code paths NOT changing:**

- `CredentialService` and `nebula-credential-runtime` public surface —
  no new method, no dyn-erase trait, no new credential type.
- `OAuth2Credential` and its `Interactive`/`Refreshable`/`Revocable`/`Testable`
  trait impls — Plane B unchanged.
- `validate_token_endpoint` — anti-SSRF policy preserved.
- `PendingStateStore` (Plane B store) — untouched by this change.
- Workspace `Cargo.toml` — every required dep (`reqwest`, `nebula-credential`,
  `nebula-credential-runtime`, `wiremock` (dev), `jsonwebtoken`, `hmac`,
  `sha2`, `zeroize`) is already present.

**Strict TDD evidence** (per `openspec/config.yaml`): 23 RED tests
anchored across PR-2 (5 tests), PR-3 (5 tests), PR-4 (13 tests). PR-5
is doc-only; PR-1 (this ADR) is markdown-only.

**Review workload:** 5-PR chain, total ~1,230 LOC, each PR ≤ 800 LOC.
Squash-merge order: 1 → 2 → 3 → 4 → 5. No parallelization. PR-1 ADR
gates the chain.

**1.1 follow-ups carved out:**

- id_token JWKS signature validation (D-16). Tracked via the issue
  filed alongside PR-5.
- Plane B credential-OAuth maturity work (out of scope for this ADR).
- DB-managed admin UI for OAuth provider config (n8n's
  `N8N_SSO_MANAGED_BY_ENV=false` equivalent).
- Wholesale replacement of `flow::build_authorization_uri` +
  `flow::exchange_code` with the `openidconnect` crate (if maintenance
  burden grows).
